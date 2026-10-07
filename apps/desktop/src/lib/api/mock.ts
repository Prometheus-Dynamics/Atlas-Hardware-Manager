// In-memory simulated backend for previewing the UI in a plain browser
// (`bun run dev`). Same shape as `api` in commands.ts; see client.ts.

import type { api as tauriApi } from "./commands";
import type { AppInfo, DeviceAction, HealthCheck, OrionConnection, ReleaseEntry, RobotProfile } from "./types";
import { keyString } from "./types";
import { atlasChannel, downloadChannel, emit, latency, resyncChannel, sleep } from "./mock/bus";
import { fleet, inventory, jobs, razeUsbBoot, releases, robots, settings, simDevice, sources } from "./mock/data";
import * as runner from "./mock/runner";
import { activity, logLines, metrics, online, record, restart, seedHistory } from "./mock/observe";
import { listRecords, robotStatuses, robotUpdateRequest, scan, touch } from "./mock/scan";

seedHistory();

type Api = typeof tauriApi;

// JSON round-trip: inputs may be Svelte state proxies, which structuredClone rejects.
const clone = <T>(value: T): T => (value === undefined ? value : JSON.parse(JSON.stringify(value)));

async function reply<T>(work: () => T): Promise<T> {
  await latency();
  try {
    return clone(work());
  } catch (error) {
    throw typeof error === "string" ? error : String(error);
  }
}

function stored(key: { family: string; serial: string }) {
  const entry = inventory.get(keyString(key));
  if (!entry) throw `no device ${keyString(key)} in the inventory`;
  return entry;
}

const ACTIONS: DeviceAction[] = [
  { id: "locate", label: "Find it", destructive: false },
  { id: "reboot", label: "Restart", destructive: false },
  { id: "factory-reset", label: "Factory reset", destructive: true },
];

function validateRobot(profile: RobotProfile) {
  if (!profile.name.trim()) throw "invalid robot profile: the name is empty";
  for (const role of profile.roles) {
    if (!role.role.trim()) throw "invalid robot profile: a role has no name";
    if (role.device && role.device.family !== role.family)
      throw `invalid robot profile: role \`${role.role}\` expects a ${role.family} device`;
  }
}

// Like atlas-core's watch: look once at start, again whenever the fleet
// changes (see __atlasMock.setOnline), and on the slow safety-net timer.
const orion: OrionConnection = {
  url: null,
  operator_id: "operator:atlas-workstation",
  fingerprint: "sha256:9c41b7e2d0f8a3c65e1b2d4f7a8c9e01",
  enroll_command:
    "orionctl operators enroll operator:atlas-workstation --fingerprint sha256:9c41b7e2d0f8a3c65e1b2d4f7a8c9e01 --action 'update' --action 'reboot' --action 'locate'",
  connected: false,
  enrolled: false,
  node_id: null,
  node_fingerprint: null,
  error: null,
};

let watching = false;
let fallbackTimer: ReturnType<typeof setTimeout> | null = null;
let changeTimer: ReturnType<typeof setTimeout> | null = null;

function schedule() {
  if (fallbackTimer) clearTimeout(fallbackTimer);
  fallbackTimer = setTimeout(async () => {
    if (settings.auto_scan) await scan();
    schedule();
  }, settings.scan_interval_ms);
}

function changed() {
  if (!settings.auto_scan) return;
  if (changeTimer) clearTimeout(changeTimer);
  changeTimer = setTimeout(() => void scan().then(schedule), 300);
}

/**
 * The board restarts straight into USB boot: the running record goes
 * offline, and a few seconds later the same board (same board_serial)
 * shows up as a USB boot device.
 */
function intoUsbBoot(serial: string) {
  const running = fleet.find((d) => d.key.serial === serial);
  const board = running?.attributes?.board_serial;
  if (!running || !board) return;
  setTimeout(() => {
    running.online = false;
    void scan();
  }, 600);
  setTimeout(() => {
    const port = `port-1-${board.slice(0, 4)}`;
    const existing = fleet.find((d) => d.key.serial === port);
    if (existing) existing.online = true;
    else fleet.push(razeUsbBoot(port, board));
    void scan();
  }, 4500);
}

function autoScan() {
  if (watching) return;
  watching = true;
  setTimeout(() => void scan().then(schedule), 400);
}

export const mockApi: Api = {
  scan: () => scan().then(clone),
  listDevices: () => reply(listRecords),
  setDeviceLabel: (key, label) =>
    reply(() => {
      stored(key).label = label?.trim() || null;
      touch(key);
    }),
  setDeviceRobot: (key, robot) =>
    reply(() => {
      stored(key).robot = robot;
      touch(key);
      emit({ type: "robots-changed" });
    }),
  forgetDevice: (key) =>
    reply(() => {
      if (stored(key).presence === "online") throw `${keyString(key)} is online; only offline devices can be forgotten`;
      inventory.delete(keyString(key));
      emit({ type: "device-forgotten", key });
    }),
  deviceActions: (key) =>
    reply(() => {
      const sim = simDevice(key);
      if (stored(key).presence !== "online") throw `${keyString(key)} is offline; reconnect it or run a scan`;
      if (sim?.actions) return sim.actions;
      if (!sim || sim.mode === "recovery") throw `${keyString(key)} does not support actions`;
      return ACTIONS;
    }),
  runDeviceAction: (key, action) =>
    reply(() => {
      const found = (simDevice(key)?.actions ?? ACTIONS).find((a) => a.id === action);
      if (!found) throw `${keyString(key)} has no action named \`${action}\``;
      if (action === "reboot") restart(key);
      if (action === "usb-boot") intoUsbBoot(key.serial);
      const entry = stored(key);
      const name = entry.label ?? entry.record?.identity.name ?? keyString(key);
      record("action-run", "info", key, `${found.label} on ${name}`);
    }),
  deviceTelemetry: (key) => reply(() => metrics(online(key))),
  deviceLogs: (key, lines) => reply(() => logLines(online(key), Math.max(1, Math.min(lines, 2000)))),
  listActivity: (limit) => reply(() => activity.slice(-limit).reverse()),
  saveSupportBundle: (key) =>
    reply(() => {
      stored(key);
    }),

  planUpdate: (request) => reply(() => runner.plan(clone(request))),
  startUpdate: (request) => reply(() => runner.start(clone(request))),
  cancelJob: (id) => reply(() => runner.cancel(id)),
  listJobs: () => reply(() => jobs),
  getJob: (id) => reply(() => jobs.find((job) => job.id === id) ?? null),

  listRobots: () => reply(() => robots),
  robotStatuses: () => reply(robotStatuses),
  saveRobot: (profile, previousName = null) =>
    reply(() => {
      validateRobot(profile);
      const index = robots.findIndex((r) => r.name === (previousName ?? profile.name));
      if (previousName && previousName !== profile.name && robots.some((r) => r.name === profile.name))
        throw `invalid robot profile: a robot named \`${profile.name}\` already exists`;
      if (index >= 0) robots[index] = clone(profile);
      else robots.push(clone(profile));
      emit({ type: "robots-changed" });
    }),
  deleteRobot: (name) =>
    reply(() => {
      const index = robots.findIndex((r) => r.name === name);
      if (index < 0) throw `no robot named \`${name}\``;
      robots.splice(index, 1);
      emit({ type: "robots-changed" });
    }),
  robotUpdateRequest: (name, staged = null) => reply(() => robotUpdateRequest(name, staged, settings.staged_default)),

  listReleases: () => reply(() => releases),
  addLocalRelease: (path, family, version) =>
    reply(() => {
      if (!family.trim() || !version.trim()) throw "a local release needs a family and a version";
      const name = path.split(/[\\/]/).pop() ?? path;
      const entry: ReleaseEntry = {
        id: `local/${family}/${version}`,
        family,
        version,
        channel: "local",
        origin: { kind: "local-file" },
        artifact_name: name,
        sha256: Math.random().toString(16).slice(2).padEnd(64, "0"),
        size_bytes: 1_000_000 + Math.floor(Math.random() * 9_000_000),
        path,
        signed: false,
        boards: [],
        notes_url: null,
        added_ms: Date.now(),
        pinned: false,
      };
      const index = releases.findIndex((r) => r.id === entry.id);
      if (index >= 0) releases[index] = entry;
      else releases.push(entry);
      return entry;
    }),
  setReleasePinned: (id, pinned) =>
    reply(() => {
      const entry = releases.find((r) => r.id === id);
      if (!entry) throw `no release ${id} in the catalog`;
      entry.pinned = pinned;
    }),
  removeRelease: (id) =>
    reply(() => {
      const index = releases.findIndex((r) => r.id === id);
      if (index < 0) throw `no release ${id} in the catalog`;
      releases.splice(index, 1);
    }),
  refreshReleases: () =>
    reply(() => (sources.length ? [`${sources[sources.length - 1].name}: manifest signature did not match any key`] : [])),
  downloadRelease: async (id) => {
    const entry = releases.find((r) => r.id === id);
    if (!entry) throw `no release ${id} in the catalog`;
    const total = entry.size_bytes;
    for (let i = 1; i <= 20; i++) {
      await sleep(120);
      downloadChannel.emit({ id, downloaded: Math.round((total * i) / 20), total });
    }
    entry.path = `/home/atlas/.cache/atlas/releases/${entry.artifact_name}`;
    return clone(entry);
  },
  listReleaseSources: () => reply(() => sources),
  setReleaseSource: (source) =>
    reply(() => {
      if (!/^https?:\/\//.test(source.index_url)) throw "the index URL must start with http:// or https://";
      if (source.public_keys.some((k) => !/^ed25519:[0-9a-f]{64}$/i.test(k)))
        throw "public keys look like ed25519:<64 hex characters>";
      const index = sources.findIndex((s) => s.name === source.name);
      if (index >= 0) sources[index] = clone(source);
      else sources.push(clone(source));
    }),
  removeReleaseSource: (name) =>
    reply(() => {
      const index = sources.findIndex((s) => s.name === name);
      if (index >= 0) sources.splice(index, 1);
    }),

  appInfo: () =>
    reply((): AppInfo => ({
      version: "0.1.0-browser",
      platform: "browser",
      arch: "wasm",
      simulated: "demo",
      paths: {
        data_dir: "/home/atlas/.local/share/atlas",
        cache_dir: "/home/atlas/.cache/atlas",
        settings_file: "/home/atlas/.local/share/atlas/settings.json",
        inventory_file: "/home/atlas/.local/share/atlas/inventory.json",
        releases_file: "/home/atlas/.local/share/atlas/releases.json",
        release_cache_dir: "/home/atlas/.cache/atlas/releases",
        log_file: "/home/atlas/.local/share/atlas/atlas.log",
      },
      startup_warnings: ["Running in a browser without Tauri: every device and job is simulated."],
    })),
  healthChecks: () =>
    reply((): HealthCheck[] => [
      { id: "data-dir", label: "Data directory", status: "ok", detail: "Writable.", fix: null, fix_action: null },
      {
        id: "usbboot.udev",
        label: "USB boot permissions",
        status: "warning",
        detail: "No udev rule for Pi boot devices was found, so USB boot would need root.",
        fix: "Press Fix to install it (one password prompt), then replug the board.",
        fix_action: "usbboot.install-access",
      },
      {
        id: "network.mdns",
        label: "Network discovery",
        status: "ok",
        detail: "Listening for PD devices with mDNS; 0 advertised right now.",
        fix: null,
        fix_action: null,
      },
    ]),
  orionConnection: () => reply(() => orion),
  setOrionUrl: (url) =>
    reply(() => {
      if (url && !url.startsWith("orion+tcp://")) throw "Orion addresses look like orion+tcp://host:port";
      settings.orion_url = url;
      orion.url = url;
      orion.connected = !!url;
      orion.enrolled = false;
      orion.node_id = url ? "raze-8f3a1c2d" : null;
      orion.node_fingerprint = url ? "sha256:5b1f0c2e9a7d44e18c3a06b2f9d1e7a4" : null;
      return orion;
    }),
  checkOrion: () => reply(() => orion),
  enrollOrionWithKey: (key) =>
    reply(() => {
      if (!key.trim()) throw "enter the node's enrollment key";
      orion.enrolled = true;
      return orion;
    }),
  discoveryStatus: () =>
    reply(() => ({ live: settings.auto_scan, watching: ["Simulated"], polled: [], fallback_ms: settings.scan_interval_ms })),
  fixHealth: (action) => reply(() => `Fixed ${action} (simulated).`),
  getSettings: () => reply(() => settings),
  saveSettings: (next) =>
    reply(() => {
      const restart = next.simulated !== settings.simulated;
      Object.assign(settings, next);
      return restart;
    }),
  restartApp: async () => {
    await latency();
    location.reload();
  },
};

export const mockEvents = {
  onAtlasEvent: (handler: Parameters<typeof atlasChannel.listen>[0]) => {
    autoScan();
    return atlasChannel.listen(handler);
  },
  onResync: (handler: () => void) => resyncChannel.listen(handler),
  onDownloadProgress: (handler: Parameters<typeof downloadChannel.listen>[0]) => downloadChannel.listen(handler),
};

// Console hook for trying offline flows in the preview:
// `__atlasMock.setOnline("H-1003", false)` then wait for the next scan.
if (typeof window !== "undefined") {
  (window as unknown as { __atlasMock: object }).__atlasMock = {
    setOnline(serial: string, online: boolean) {
      const device = fleet.find((d) => d.key.serial === serial);
      if (device) device.online = online;
      changed();
    },
    /** Holding the boot button while plugging in: `__atlasMock.holdBoot("5c0ffee1")`. */
    holdBoot(serial: string) {
      intoUsbBoot(serial);
    },
    neverConfirms(serial: string, value = true) {
      const device = fleet.find((d) => d.key.serial === serial);
      if (device) device.neverConfirms = value;
    },
  };
}
