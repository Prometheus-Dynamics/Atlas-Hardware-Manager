// Simulated board status and event logs for the browser mock: what
// /usr/lib/board/status --json and the event log report, and the history
// atlas-core merges from them. The A/B Raze has a history with a manual
// change made on the board, and an update Orion started that runs while you
// watch (staging, restart, trial boot, kept).

import type {
  DeviceEvent,
  DeviceKey,
  DeviceStatus,
  EventSource,
  HistoryEntry,
  UpdateState,
} from "../types";
import { keyString } from "../types";
import { emit } from "./bus";
import { simDevice, type SimDevice } from "./data";
import { activity, online, restart } from "./observe";

interface Board {
  events: DeviceEvent[];
  update: UpdateState;
  bootCount: number;
  bootId: string;
  bootedAt: number;
  previousClean: boolean | null;
  clockOffset: number;
  /** When the simulated update's stage started (ms), while one runs. */
  stageStart: number | null;
}

const boards = new Map<string, Board>();
const S = 1000;
const H = 3_600_000;
/** How long the simulated stage takes, then the restart and the trial. */
const STAGE_MS = 150 * S;
const REBOOT_MS = 12 * S;
const TRIAL_MS = 25 * S;

const sec = (ms: number) => Math.floor(ms / 1000);
const bootId = () => Math.random().toString(16).slice(2, 10) + "-5c2a-4d1e-9b0f-" + Math.random().toString(16).slice(2, 14);

function event(board: Board, at: number, kind: string, source: EventSource, message: string, data: Record<string, string> = {}) {
  board.events.push({ t: sec(at), boot_id: board.bootId, kind, source, message, data });
}

function seed(device: SimDevice): Board {
  const now = Date.now();
  const board: Board = {
    events: [],
    update: {
      state: "confirmed",
      slot_active: "B",
      slot_staged: "B",
      version_active: device.version,
      version_staged: device.version,
      version_previous: "2025.4.2",
      progress: 1000,
      error: null,
      started_by: "atlas",
    },
    bootCount: 38,
    bootId: bootId(),
    bootedAt: now - 3 * 24 * H,
    previousClean: true,
    clockOffset: -2,
    stageStart: null,
  };
  const boot = (at: number, clean: boolean) => {
    board.bootId = bootId();
    board.bootCount += 1;
    board.bootedAt = at;
    board.previousClean = clean;
    event(board, at, "boot", "local", `boot ${board.bootCount}, slot B, kernel 7.2.9-v8${clean ? "" : "; the previous boot didn't shut down cleanly"}`, {
      count: String(board.bootCount),
      slot: "B",
      kernel: "7.2.9-v8",
      previous_clean: String(clean),
    });
  };
  event(board, now - 50 * H, "update.stage", "atlas", "staging an update into slot B", { slot: "B" });
  event(board, now - 50 * H + 140 * S, "update.staged", "atlas", `staged ${device.version} in slot B`, { version: device.version, slot: "B" });
  event(board, now - 50 * H + 150 * S, "update.apply", "atlas", `restarting into ${device.version} on trial`, { version: device.version, slot: "B" });
  event(board, now - 50 * H + 160 * S, "shutdown", "local", "shutting down cleanly");
  boot(now - 50 * H + 190 * S, true);
  event(board, now - 50 * H + 230 * S, "update.confirmed", "atlas", `kept ${device.version} in slot B`, { version: device.version, slot: "B" });
  event(board, now - 26 * H, "ssh.keys", "local", "installed 2 SSH key(s) from the boot partition", { count: "2", added: "1" });
  // A power cut: no shutdown event before the next boot.
  boot(now - 5 * H, false);
  event(board, now - 4.8 * H, "clock.set", "atlas", "clock set from a computer over SSH", { old: String(sec(now - 400 * 24 * H)), new: String(sec(now - 4.8 * H)) });
  // Someone on the board staged an image by hand, then thought better of it.
  event(board, now - 2.2 * H, "update.stage", "local", "staging an update into slot A", { slot: "A" });
  event(board, now - 2.1 * H, "update.cancelled", "local", "cancelled the update", { stopped: "1" });
  event(board, now - 70 * 60 * S, "shutdown", "local", "shutting down cleanly");
  boot(now - 68 * 60 * S, true);
  // Orion's update, running now.
  board.stageStart = now - 0.35 * STAGE_MS;
  board.update = {
    ...board.update,
    state: "staging",
    slot_staged: "A",
    version_staged: null,
    progress: 0,
    started_by: "orion",
  };
  event(board, board.stageStart - 20 * S, "update.download", "orion", "downloading an update", {
    url: "http://192.168.1.20:5898/helios-raze-2026.3.1.img.zst",
    size: "612368384",
  });
  event(board, board.stageStart, "update.stage", "orion", "staging an update into slot A", { slot: "A" });
  return board;
}

function boardFor(device: SimDevice): Board {
  const id = keyString(device.key);
  let board = boards.get(id);
  if (!board) {
    board = seed(device);
    boards.set(id, board);
  }
  return board;
}

function changed(device: SimDevice) {
  emit({ type: "device-history", key: device.key });
}

/** Moves the simulated update along: stage, restart, trial, kept. */
function advance(device: SimDevice, board: Board) {
  const start = board.stageStart;
  if (start === null) return;
  const now = Date.now();
  const target = "2026.3.1";
  const u = board.update;
  if (u.state === "staging") {
    u.progress = Math.min(1000, Math.round(((now - start) / STAGE_MS) * 1000));
    if (now - start < STAGE_MS) return;
    Object.assign(u, { state: "staged", progress: 1000, version_staged: target });
    event(board, start + STAGE_MS, "update.staged", "orion", `staged ${target} in slot A`, { version: target, slot: "A" });
    event(board, start + STAGE_MS + S, "update.apply", "orion", `restarting into ${target} on trial`, { version: target, slot: "A" });
    u.state = "rebooting";
    changed(device);
  }
  if (u.state === "rebooting" && now - start >= STAGE_MS + REBOOT_MS) {
    event(board, start + STAGE_MS + 2 * S, "shutdown", "local", "shutting down cleanly");
    board.bootId = bootId();
    board.bootCount += 1;
    board.bootedAt = start + STAGE_MS + REBOOT_MS;
    board.previousClean = true;
    event(board, board.bootedAt, "boot", "local", `boot ${board.bootCount}, slot A, kernel 7.2.9-v8`, {
      count: String(board.bootCount),
      slot: "A",
      previous_clean: "true",
    });
    Object.assign(u, { state: "trying", slot_active: "A" });
    restart(device.key);
    changed(device);
  }
  if (u.state === "trying" && now - start >= STAGE_MS + REBOOT_MS + TRIAL_MS) {
    event(board, now, "update.confirmed", "orion", `kept ${target} in slot A`, { version: target, slot: "A" });
    Object.assign(u, { state: "confirmed", version_previous: u.version_active, version_active: target, error: null });
    board.stageStart = null;
    device.version = target;
    if (device.attributes) device.attributes.os_version = target;
    changed(device);
  }
}

export function hasStatus(device: SimDevice | undefined): device is SimDevice {
  return !!device?.caps?.includes("status");
}

export function deviceStatus(key: DeviceKey): DeviceStatus {
  const device = online(key);
  if (!hasStatus(device)) throw `${keyString(key)} does not report its status`;
  const board = boardFor(device);
  advance(device, board);
  const now = Date.now();
  const busy = ["staging", "rebooting", "trying"].includes(board.update.state);
  const temp = 51.5 + Math.sin(now / 20_000) * 2.5 + (busy ? 6 : 0);
  return {
    time: sec(now) + board.clockOffset,
    boot: {
      id: board.bootId,
      count: board.bootCount,
      slot: board.update.slot_active,
      kernel: "7.2.9-v8",
      uptime_s: sec(now - board.bootedAt),
      previous_clean: board.previousClean,
    },
    failed_units: board.update.state === "trying" ? ["photonvision.service"] : [],
    temperatures: [{ id: "cpu-thermal", celsius: Math.round(temp * 10) / 10 }],
    fan: { state: busy ? 2 : 1, max_state: 4, pwm: busy ? 212 : 179, rpm: null },
    update: { ...board.update },
    clock_offset_s: board.clockOffset,
    ntp_synchronized: false,
    drift: {
      checked_at: sec(now - 14 * 60 * S),
      slot: board.update.slot_active,
      root_read_only: true,
      baseline: "stage",
      count: 2,
      flags: ["config", "update_env"],
      items: [
        { area: "boot", path: "config.txt", change: "changed", sha256: "9f2c41d07e5b3a8c6d1e0f4a2b7c9d3e5f8a1b2c4d6e8f0a1b3c5d7e9f1a2b3c" },
        { area: "data", path: "/data/board/update.env", change: "present", sha256: "3b7e1a9c5d2f8e4b6a0c1d3e5f7a9b2c4d6e8f0a1b3c5d7e9f1a2b3c4d5e6f7a" },
        { area: "etc", path: "/etc/board/identity.env", change: "present", sha256: "c1d3e5f7a9b2c4d6e8f0a1b3c5d7e9f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7" },
      ],
    },
  };
}

/** Atlas's entries for the device and its board's events, newest first. */
export function deviceHistory(key: DeviceKey, limit: number): HistoryEntry[] {
  const device = simDevice(key);
  const own: HistoryEntry[] = activity
    .filter((entry) => entry.device && keyString(entry.device) === keyString(key))
    .map((entry) => ({
      at_ms: entry.at_ms,
      level: entry.level,
      kind: entry.kind,
      source: "atlas",
      origin: "atlas",
      message: entry.message,
      boot_id: null,
      data: {},
    }));
  const board = hasStatus(device) ? boardFor(device) : null;
  if (board && device) advance(device, board);
  const theirs: HistoryEntry[] = (board?.events ?? []).map((e) => ({
    at_ms: e.t * 1000,
    level: level(e),
    kind: e.kind,
    source: e.source,
    origin: "board",
    message: e.message,
    boot_id: e.boot_id,
    data: e.data,
  }));
  return [...own, ...theirs].sort((a, b) => b.at_ms - a.at_ms).slice(0, limit);
}

function level(e: DeviceEvent): HistoryEntry["level"] {
  if (e.kind.endsWith("failed") || e.kind === "update.link-fallback") return "error";
  if (e.kind === "update.rolled-back" || (e.kind === "boot" && e.data.previous_clean === "false")) return "warning";
  if (e.kind === "update.confirmed" || e.kind === "update.staged") return "success";
  return "info";
}

/** The board's side of an action Atlas ran: events, and the update state. */
export function boardAction(key: DeviceKey, action: string): void {
  const device = simDevice(key);
  if (!hasStatus(device)) return;
  const board = boardFor(device);
  advance(device, board);
  const now = Date.now();
  const u = board.update;
  switch (action) {
    case "update.cancel":
      if (!["staging", "staged"].includes(u.state)) throw `the update is past staging (state: ${u.state}); it can't be cancelled`;
      event(board, now, "update.cancelled", "atlas", `cancelled ${u.version_staged ?? "the update"}`, { stopped: u.state === "staging" ? "1" : "0" });
      Object.assign(u, { state: "cancelled", slot_staged: null, version_staged: null, progress: 0, started_by: "atlas" });
      board.stageStart = null;
      break;
    case "update.rollback": {
      if (!u.version_previous || ["staging", "rebooting", "trying"].includes(u.state))
        throw "there is no previous confirmed slot to go back to";
      const back = u.version_previous;
      const slot = u.slot_active === "A" ? "B" : "A";
      event(board, now, "update.rollback", "atlas", `going back to ${back} in slot ${slot}`, { version: back, slot });
      Object.assign(u, { state: "rolled-back", version_previous: u.version_active, version_active: back, slot_active: slot, started_by: "atlas" });
      device.version = back;
      reboot(device, board, now);
      break;
    }
    case "reboot":
      event(board, now, "reboot", "atlas", "restart requested from Atlas");
      reboot(device, board, now);
      break;
    case "power-off":
      event(board, now, "power-off", "atlas", "power-off requested from Atlas");
      event(board, now + S, "shutdown", "local", "shutting down cleanly");
      setTimeout(() => {
        device.online = false;
      }, 400);
      break;
    case "set-clock":
      event(board, now, "clock.set", "atlas", "clock set from a computer over SSH", { old: String(sec(now) + board.clockOffset), new: String(sec(now)) });
      board.clockOffset = 0;
      break;
    case "locate":
      break;
    default:
      return;
  }
  changed(device);
}

function reboot(device: SimDevice, board: Board, now: number) {
  event(board, now + S, "shutdown", "local", "shutting down cleanly");
  board.bootId = bootId();
  board.bootCount += 1;
  board.bootedAt = now + 8 * S;
  board.previousClean = true;
  event(board, board.bootedAt, "boot", "local", `boot ${board.bootCount}, slot ${board.update.slot_active}, kernel 7.2.9-v8`, {
    count: String(board.bootCount),
    slot: board.update.slot_active ?? "",
    previous_clean: "true",
  });
  restart(device.key);
}
