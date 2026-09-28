// Simulated discovery: turns the fleet into inventory records and events.

import type { DeviceKey, DeviceRecord, RobotStatus, ScanReport, UpdateRequestInput, StagedRollout } from "../types";
import { keyString, sameKey } from "../types";
import { emit, sleep } from "./bus";
import { capabilities, fleet, identityOf, inventory, linkKindOf, releases, robots, simDevice, type SimDevice } from "./data";

export function recordOf(device: SimDevice): DeviceRecord {
  const stored = inventory.get(keyString(device.key))!;
  const online = stored.presence === "online";
  const record: DeviceRecord = {
    key: device.key,
    identity: identityOf(device),
    link_kind: linkKindOf(device),
    capabilities: online ? capabilities(device) : ["info"],
    presence: stored.presence,
    first_seen_ms: stored.first_seen_ms,
    last_seen_ms: stored.last_seen_ms,
    label: stored.label,
    robot: stored.robot,
  };
  stored.record = record;
  return record;
}

export function listRecords(): DeviceRecord[] {
  return fleet.filter((d) => inventory.has(keyString(d.key))).map(recordOf);
}

/** Re-emits the record after a change (label, robot, version). */
export function touch(key: DeviceKey) {
  const device = simDevice(key);
  if (device && inventory.has(keyString(key))) emit({ type: "device-seen", record: recordOf(device), new: false });
}

let scanning: Promise<ScanReport> | null = null;

export function scan(): Promise<ScanReport> {
  scanning ??= runScan().finally(() => (scanning = null));
  return scanning;
}

async function runScan(): Promise<ScanReport> {
  const started = Date.now();
  emit({ type: "scan-started" });
  const online: DeviceKey[] = [];
  const wentOffline: DeviceKey[] = [];
  // Gateways answer first, then the boards behind them.
  const ordered = [...fleet].sort((a, b) => Number(!!a.parent) - Number(!!b.parent));
  for (const device of ordered) {
    const id = keyString(device.key);
    let stored = inventory.get(id);
    const parentOnline = !device.parent || simDevice(device.parent)?.online;
    if (device.online && parentOnline) {
      await sleep(60 + Math.random() * 90);
      const isNew = !stored;
      const now = Date.now();
      stored ??= { first_seen_ms: now, last_seen_ms: now, label: null, robot: null, presence: "online", record: null };
      stored.last_seen_ms = now;
      stored.presence = "online";
      inventory.set(id, stored);
      online.push(device.key);
      emit({ type: "device-seen", record: recordOf(device), new: isNew });
    } else if (stored && stored.presence === "online") {
      stored.presence = "offline";
      wentOffline.push(device.key);
      emit({ type: "device-offline", key: device.key });
    }
  }
  const report: ScanReport = { online, went_offline: wentOffline, warnings: [], duration_ms: Date.now() - started };
  emit({ type: "scan-finished", report });
  return report;
}

function primaryVersion(record: DeviceRecord | null): string | null {
  if (!record) return null;
  const v = record.identity.versions;
  return v.os ?? v.firmware ?? v.bootloader ?? Object.values(v)[0] ?? null;
}

function displayName(record: DeviceRecord | null, key: DeviceKey): string {
  return record?.label ?? record?.identity.name ?? keyString(key);
}

export function robotStatuses(): RobotStatus[] {
  return robots.map((profile) => {
    const roles = profile.roles.map((role) => {
      const record = role.device ? (inventory.get(keyString(role.device))?.record ?? null) : null;
      const version = primaryVersion(record);
      const target = profile.targets[role.family] ?? null;
      return {
        role: role.role,
        family: role.family,
        device: role.device,
        device_name: role.device ? displayName(record, role.device) : null,
        presence: record?.presence ?? null,
        version,
        target,
        up_to_date: target && version ? version === target : null,
      };
    });
    const unassigned = [...inventory.values()]
      .map((s) => s.record)
      .filter((r): r is DeviceRecord => !!r && r.robot === profile.name)
      .filter((r) => !profile.roles.some((role) => sameKey(role.device, r.key)))
      .map((r) => r.key);
    const state: RobotStatus["state"] = roles.some((r) => !r.device)
      ? "unassigned"
      : roles.some((r) => r.presence !== "online")
        ? "missing-devices"
        : roles.some((r) => r.up_to_date === false)
          ? "needs-update"
          : "ready";
    return { name: profile.name, state, roles, unassigned_devices: unassigned };
  });
}

export function robotUpdateRequest(name: string, staged: StagedRollout | null, fallback: StagedRollout): UpdateRequestInput | null {
  const status = robotStatuses().find((s) => s.name === name);
  if (!status) throw `no robot named \`${name}\``;
  const stale = status.roles.filter((r) => r.device && r.presence === "online" && r.up_to_date === false);
  if (stale.length === 0) return null;
  const choices: UpdateRequestInput["releases"] = {};
  for (const role of stale) {
    const entry = releases.find((r) => r.family === role.family && r.version === role.target);
    choices[role.family] = { version: role.target!, release_id: entry?.id ?? null };
  }
  return { devices: stale.map((r) => r.device!), releases: choices, staged: staged ?? fallback };
}
