// Simulated board status and event logs for the browser mock: what
// /usr/lib/board/status --json and the event log report, and the history
// atlas-core merges from them. The A/B Raze has a history with a manual
// change made on the board, and an update Orion started that runs while you
// watch (staging, restart, trial boot, kept).

import type {
  CalibrationRoutine,
  CalibrationStatus,
  CalibrationStep,
  DeviceEvent,
  DeviceKey,
  DeviceStatus,
  EventSource,
  HardwareCommand,
  HardwareSnapshot,
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
  /** How this boot began: "power-on" or "reset". */
  reset: string;
  /** Kernel logs kept (pstore), oldest first, under the boot that found them. */
  kernelLogs: { boot: string; panic: boolean; text: string }[];
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
    reset: "reset",
    kernelLogs: [],
    clockOffset: -2,
    stageStart: null,
  };
  /** A boot; `log`: the kernel log it found of the boot before (a warm restart always leaves its console). */
  const boot = (at: number, clean: boolean, reset = "reset", slot = "B", log?: { panic: boolean; text: string }) => {
    board.bootId = bootId();
    board.bootCount += 1;
    board.bootedAt = at;
    board.previousClean = clean;
    board.reset = reset;
    if (log) board.kernelLogs.push({ boot: board.bootId, ...log });
    let message = `boot ${board.bootCount}, slot ${slot}, kernel 7.2.9-v8`;
    if (!clean) message += "; the previous boot didn't shut down cleanly";
    if (reset === "power-on") message += "; it began at power-on";
    event(board, at, "boot", "local", message, {
      count: String(board.bootCount),
      slot,
      kernel: "7.2.9-v8",
      previous_clean: String(clean),
      reset,
    });
    if (log?.panic || (log && !clean)) {
      event(board, at + 2 * S, "kernel.pstore", "local", `the last boot ended ${log.panic ? "in a kernel panic or oops" : "without a clean shutdown"}; its kernel log is in /data/board/pstore/${board.bootId}`, {
        dir: `/data/board/pstore/${board.bootId}`,
        panics: log.panic ? "1" : "0",
        previous_clean: String(clean),
      });
    }
  };
  const restartConsole = (up: number, command = "") =>
    `[    0.000000] Booting Linux on physical CPU 0x0000000000 [0x414fd0b1]\n[    0.000000] Linux version 7.2.9-v8 (helios@build) #1 SMP PREEMPT\n[${up.toFixed(6).padStart(12)}] systemd-shutdown[1]: Rebooting.\n[${(up + 0.01).toFixed(6).padStart(12)}] reboot: Restarting system${command ? ` with command '${command}'` : ""}\n`;
  event(board, now - 50 * H, "update.stage", "atlas", "staging an update into slot B", { slot: "B" });
  event(board, now - 50 * H + 140 * S, "update.staged", "atlas", `staged ${device.version} in slot B`, { version: device.version, slot: "B" });
  event(board, now - 50 * H + 150 * S, "update.apply", "atlas", `restarting into ${device.version} on trial`, { version: device.version, slot: "B" });
  event(board, now - 50 * H + 160 * S, "shutdown", "local", "shutting down cleanly");
  boot(now - 50 * H + 190 * S, true, "reset", "B", { panic: false, text: `== console-ramoops-0\n${restartConsole(8123.4, "0 tryboot")}` });
  event(board, now - 50 * H + 230 * S, "update.confirmed", "atlas", `kept ${device.version} in slot B`, { version: device.version, slot: "B" });
  event(board, now - 26 * H, "ssh.keys", "local", "installed 2 SSH key(s) from the boot partition", { count: "2", added: "1" });
  // An update that failed its trial and rolled back.
  event(board, now - 12 * H, "update.apply", "atlas", "restarting into 2026.3.0-rc2 on trial", { version: "2026.3.0-rc2", slot: "A" });
  event(board, now - 12 * H + 10 * S, "shutdown", "local", "shutting down cleanly");
  boot(now - 12 * H + 40 * S, true, "reset", "A", { panic: false, text: `== console-ramoops-0\n${restartConsole(32760.2, "0 tryboot")}` });
  event(board, now - 12 * H + 160 * S, "update.trial-failed", "local", "photonvision.service didn't start in the trial", { slot: "A", reason: "photonvision.service failed" });
  event(board, now - 12 * H + 165 * S, "update.rolled-back", "local", "rolled back to slot B: photonvision.service failed in the trial", { slot: "B", from: "A", reason: "photonvision.service failed" });
  event(board, now - 12 * H + 170 * S, "shutdown", "local", "shutting down cleanly");
  boot(now - 12 * H + 200 * S, true, "reset", "B", { panic: false, text: `== console-ramoops-0\n${restartConsole(160.9)}` });
  // A hard lockup: the kernel panicked and the watchdog restarted it.
  boot(now - 8 * H, false, "reset", "B", {
    panic: true,
    text:
      "== dmesg-ramoops-0\nPanic#1 Part1\n<0>[17702.418211] watchdog: Watchdog detected hard LOCKUP on cpu 2\n<4>[17702.418230] CPU: 2 PID: 1834 Comm: photonvision Tainted: G         C         7.2.9-v8 #1\n<4>[17702.418236] Hardware name: Raspberry Pi Compute Module 5 Rev 1.0 (DT)\n<4>[17702.418240] pc : rp1_cfe_isr+0x1a4/0x2f0 [rp1_cfe]\n<4>[17702.418252] lr : __handle_irq_event_percpu+0x5c/0x1e0\n<4>[17702.418260] Call trace:\n<4>[17702.418262]  rp1_cfe_isr+0x1a4/0x2f0 [rp1_cfe]\n<4>[17702.418270]  __handle_irq_event_percpu+0x5c/0x1e0\n<4>[17702.418276]  handle_irq_event+0x4c/0xb0\n<0>[17702.418290] Kernel panic - not syncing: Hard LOCKUP\n<2>[17702.418295] SMP: stopping secondary CPUs\n<0>[17702.418310] Kernel Offset: disabled\n<0>[17702.418312] ---[ end Kernel panic - not syncing: Hard LOCKUP ]---\n\n== console-ramoops-0\n[17690.104512] rp1-cfe 1f00110000.csi: Frame wait timed out\n[17702.418211] watchdog: Watchdog detected hard LOCKUP on cpu 2\n[17702.418290] Kernel panic - not syncing: Hard LOCKUP\n",
  });
  // A power cut: no shutdown event before the next boot, and RAM (pstore) lost.
  boot(now - 5 * H, false, "power-on");
  event(board, now - 4.8 * H, "clock.set", "atlas", "clock set from a computer over SSH", { old: String(sec(now - 400 * 24 * H)), new: String(sec(now - 4.8 * H)) });
  // Someone on the board staged an image by hand, then thought better of it.
  event(board, now - 2.2 * H, "update.stage", "local", "staging an update into slot A", { slot: "A" });
  event(board, now - 2.1 * H, "update.cancelled", "local", "cancelled the update", { stopped: "1" });
  event(board, now - 70 * 60 * S, "shutdown", "local", "shutting down cleanly");
  boot(now - 68 * 60 * S, true, "reset", "B", { panic: false, text: `== console-ramoops-0\n${restartConsole(13790.5)}` });
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
  // As board-stream does: the board says its state changed.
  emit({ type: "device-status", key: device.key });
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
    board.reset = "reset";
    event(board, board.bootedAt, "boot", "local", `boot ${board.bootCount}, slot A, kernel 7.2.9-v8`, {
      count: String(board.bootCount),
      slot: "A",
      previous_clean: "true",
      reset: "reset",
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

/** Controls written from here, by `<device key>/<hardware>/<control>`; the board's own value otherwise. */
const writes = new Map<string, number>();
const writeKey = (key: DeviceKey, hardware: string, control: string) => `${keyString(key)}/${hardware}/${control}`;

/** A command to the A/B Raze's devices, as Lemnos's Orion bridge answers it. */
export function controlHardware(key: DeviceKey, hardware: string, command: HardwareCommand): number | null {
  const device = online(key);
  const snapshot = device ? hardwareFor(device, Date.now(), 50) : null;
  const target = snapshot?.devices.find((d) => d.id === hardware);
  if (!target) throw `Orion doesn't list the board's ${hardware}`;
  const prefix = writeKey(key, hardware, "");
  switch (command.command) {
    case "set": {
      const control = target.controls.find((c) => c.name === command.control);
      if (!control) throw `${hardware}: no such control`;
      if (control.min !== null && control.max !== null && (command.value < control.min || command.value > control.max))
        throw `${hardware}: value out of range`;
      writes.set(writeKey(key, hardware, command.control), command.value);
      return command.value;
    }
    case "restore":
      for (const k of [...writes.keys()])
        if (k.startsWith(prefix) && (command.control === null || k === prefix + command.control)) writes.delete(k);
      return null;
    case "release":
      if (target.class !== "fan") throw `${hardware}: device error: unsupported`;
      for (const k of [...writes.keys()]) if (k.startsWith(prefix)) writes.delete(k);
      return null;
    case "power":
      if (target.class !== "power-switch") throw `${hardware}: unsupported action \`power.set\``;
      writes.set(writeKey(key, hardware, "power.on"), command.on ? 1 : 0);
      return command.on ? 1 : 0;
    case "calibrate":
      calibrate(target.class, hardware, command.step, command.routine);
      return null;
  }
}

/** Simulated calibration by board device: a routine runs ~8 s to a candidate. */
type SimCalibration = { running: CalibrationRoutine | null; started: number; candidate: boolean; revision: number; parts: CalibrationStatus["parts"] };
const calibrations = new Map<string, SimCalibration>();
const ROUTINE_MS = 8000;
const PARTS: Record<string, ("accel" | "gyro" | "mag")[]> = { imu: ["accel", "gyro"], magnetometer: ["mag"] };

function calibrationOf(hardware: string, kind: string): SimCalibration {
  let c = calibrations.get(hardware);
  if (!c) {
    const parts: CalibrationStatus["parts"] = {};
    for (const part of PARTS[kind] ?? []) parts[part] = { samples: 0, confidence: 0, coverage: 0, residual: 0, active: false };
    c = { running: null, started: 0, candidate: false, revision: 0, parts };
    calibrations.set(hardware, c);
  }
  // A routine that has run its time leaves a candidate.
  if (c.running && Date.now() - c.started >= ROUTINE_MS) {
    c.running = null;
    c.candidate = true;
  }
  return c;
}

function calibrate(kind: string, hardware: string, step: CalibrationStep, routine?: CalibrationRoutine) {
  if (!PARTS[kind]) throw `${hardware}: unsupported action \`calibration.${step}\``;
  const c = calibrationOf(hardware, kind);
  switch (step) {
    case "start":
      if (!routine) throw "`routine` (accel-six, mag-rotate or gyro-hold) is required";
      Object.assign(c, { running: routine, started: Date.now(), candidate: false });
      break;
    case "stop":
      c.running = null;
      break;
    case "apply":
      if (!c.candidate) throw `${hardware}: no candidate to apply`;
      for (const part of Object.values(c.parts)) Object.assign(part!, { samples: 600, confidence: 0.92, coverage: 0.88, residual: 0.03, active: true });
      Object.assign(c, { candidate: false, revision: c.revision + 1 });
      break;
    case "discard":
      c.candidate = false;
      break;
    case "reset":
      for (const part of Object.values(c.parts)) Object.assign(part!, { samples: 0, confidence: 0, coverage: 0, residual: 0, active: false });
      Object.assign(c, { running: null, candidate: false, revision: c.revision + 1 });
      break;
  }
}

/** The A/B Raze's ring presets: Lemnos's built-ins, and any saved here. */
const BUILTIN_PRESETS = ["scheme-a", "scheme-b", "scheme-c"];
const ring = {
  presets: new Map<string, string>(BUILTIN_PRESETS.map((name) => [name, `# ${name}: a built-in look preset\n[looks."status.ok"]\nlayers = [{ block = "solid", color = "00ff40" }]\n`])),
  active: "scheme-b",
  brightness: 1,
};

/** A board device's own action (Lemnos over Orion): the ring's looks and brightness, a power switch's reset. */
export function hardwareAction(key: DeviceKey, hardware: string, name: string, args: Record<string, string | number | boolean>): Record<string, unknown> {
  const device = online(key);
  const target = device ? hardwareFor(device, Date.now(), 50)?.devices.find((d) => d.id === hardware) : null;
  if (!target) throw `Orion doesn't list the board's ${hardware}`;
  const unsupported = () => `${hardware}: unsupported action \`${name}\``;
  const text = () => [...ring.presets.keys()].map((n) => `${n === ring.active ? "* " : "  "}${n}`).join("\n");
  if (target.class === "power-switch") {
    if (name !== "power.reset") throw unsupported();
    const offMs = Number(args.off_ms ?? 1000);
    writes.set(writeKey(key, hardware, "power.on"), 0);
    setTimeout(() => writes.set(writeKey(key, hardware, "power.on"), 1), offMs);
    return { control: "power.reset", applied: offMs };
  }
  if (target.class !== "light") throw unsupported();
  const named = () => {
    const n = String(args.name ?? "");
    if (!n) throw "`name` (string) is required";
    return n;
  };
  switch (name) {
    case "looks.preset.list":
      return { text: text() };
    case "looks.preset.show": {
      const body = ring.presets.get(named());
      if (body === undefined) throw `no preset ${args.name}`;
      return { text: body };
    }
    case "looks.preset.apply": {
      const n = named();
      if (!ring.presets.has(n)) throw `no preset ${n}`;
      ring.active = n;
      return { text: text() };
    }
    case "looks.preset.save":
      ring.presets.set(named(), String(args.body ?? ""));
      return { text: text() };
    case "looks.preset.delete": {
      const n = named();
      if (BUILTIN_PRESETS.includes(n)) throw `${n} is built in`;
      ring.presets.delete(n);
      if (ring.active === n) ring.active = "scheme-b";
      return { text: text() };
    }
    case "light.brightness":
      ring.brightness = Math.max(0, Math.min(1, Number(args.value)));
      return { brightness: ring.brightness };
    case "looks.locate":
    case "looks.look":
    case "looks.off":
    case "looks.show_inline":
      return {};
    default:
      throw unsupported();
  }
}

/** A board device's calibration, as Lemnos's calibration.status answers it. */
export function calibrationStatus(key: DeviceKey, hardware: string): CalibrationStatus {
  const device = online(key);
  const target = device ? hardwareFor(device, Date.now(), 50)?.devices.find((d) => d.id === hardware) : null;
  if (!target) throw `Orion doesn't list the board's ${hardware}`;
  const c = calibrationOf(hardware, target.class);
  if (!PARTS[target.class]) throw `${hardware}: unsupported action \`calibration.status\``;
  const progress = c.running ? Math.min(1, (Date.now() - c.started) / ROUTINE_MS) : c.candidate ? 1 : 0;
  return { revision: c.revision, running: c.running, progress, candidate: c.candidate, failed: false, parts: structuredClone(c.parts) };
}

/** The A/B Raze's hardware as lemnosd reports it; the readings drift a little on every read. */
function hardwareFor(device: SimDevice, now: number, celsius: number): HardwareSnapshot | null {
  if (device.key.serial !== "8f3a1c2d") return null;
  const t = now / 1000;
  const jitter = (scale: number) => (Math.random() - 0.5) * 2 * scale;
  const round = (value: number, digits: number) => Math.round(value * 10 ** digits) / 10 ** digits;
  const volts = round(12.02 + Math.sin(t / 30) * 0.04 + jitter(0.01), 2);
  const amps = round(1.5 + Math.sin(t / 11) * 0.1 + jitter(0.02), 3);
  const fanDuty = writes.get(writeKey(device.key, "fan", "duty")) ?? round(0.8 + Math.sin(t / 40) * 0.05, 2);
  return {
    at: sec(now),
    devices: [
      {
        id: "imu",
        class: "imu",
        model: "bmi088",
        status: "available",
        readings: [
          { name: "accel_x", value: round(0.04 * Math.sin(t / 7) + jitter(0.02), 3), unit: "m/s²" },
          { name: "accel_y", value: round(-0.03 * Math.cos(t / 9) + jitter(0.02), 3), unit: "m/s²" },
          // Lying flat, lens up: the IMU (mounted lens-side down) reads -g on z.
          { name: "accel_z", value: round(-9.81 + jitter(0.03), 3), unit: "m/s²" },
          { name: "gyro_x", value: round(jitter(0.01), 4), unit: "rad/s" },
          { name: "gyro_y", value: round(jitter(0.01), 4), unit: "rad/s" },
          { name: "gyro_z", value: round(jitter(0.01), 4), unit: "rad/s" },
        ],
        controls: [],
      },
      {
        id: "power",
        class: "power",
        model: "ina238",
        status: "available",
        readings: [
          { name: "voltage", value: volts, unit: "V" },
          { name: "current", value: amps, unit: "A" },
          { name: "power", value: round(volts * amps, 2), unit: "W" },
        ],
        controls: [],
      },
      {
        id: "magnetometer",
        class: "magnetometer",
        model: "bmm150",
        status: "available",
        readings: [
          { name: "magnetic_field.x", value: (21.4 + jitter(0.4)) * 1e-6, unit: "T" },
          { name: "magnetic_field.y", value: (-3.2 + jitter(0.4)) * 1e-6, unit: "T" },
          { name: "magnetic_field.z", value: (-44.8 + jitter(0.4)) * 1e-6, unit: "T" },
        ],
        controls: [],
      },
      {
        id: "status-ring",
        class: "light",
        model: "ws2812",
        status: "available",
        readings: [],
        controls: [],
      },
      {
        id: "orientation",
        class: "orientation",
        model: "fusion",
        status: "available",
        readings: [],
        controls: [],
      },
      {
        id: "fan",
        class: "fan",
        model: "pwmfan",
        status: "available",
        readings: [{ name: "rpm", value: Math.round(fanDuty * 5200 + jitter(40)), unit: "rpm" }],
        controls: [{ name: "duty", value: fanDuty, min: 0, max: 1, unit: "" }],
      },
      {
        id: "cpu-thermal",
        class: "temperature",
        model: "bcm2712",
        status: "available",
        readings: [{ name: "temperature", value: celsius, unit: "°C" }],
        controls: [],
      },
      ...(["usb-a-power", "usb-c-power"] as const).map((id) => {
        const on = writes.get(writeKey(device.key, id, "power.on")) ?? 1;
        return {
          id,
          class: "power-switch",
          model: "gpio-power-switch",
          status: "available" as const,
          readings: [{ name: "power.on", value: on, unit: "" }],
          controls: [
            { name: "power.on", value: on, min: 0, max: 1, unit: "" },
            { name: "power.reset", value: 0, min: 0, max: 10000, unit: "ms" },
          ],
        };
      }),
    ],
  };
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
      pstore_saved: board.kernelLogs.some((log) => log.boot === board.bootId),
      reset: board.reset,
      rsts: board.reset === "power-on" ? "0x1000" : "0x20",
    },
    pstore: board.kernelLogs
      .slice(-10)
      .reverse()
      .map((log) => ({ boot: log.boot, panic: log.panic, files: log.panic ? ["console-ramoops-0", "dmesg-ramoops-0"] : ["console-ramoops-0"] })),
    failed_units: board.update.state === "trying" ? ["photonvision.service"] : [],
    temperatures: [{ id: "cpu-thermal", celsius: Math.round(temp * 10) / 10 }],
    fan: { state: busy ? 2 : 1, max_state: 4, pwm: busy ? 212 : 179, rpm: null },
    update: { ...board.update },
    clock_offset_s: board.clockOffset,
    ntp_synchronized: false,
    hardware: hardwareFor(device, now, Math.round(temp * 10) / 10),
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

/** `pstore --show <boot>`: a kept kernel log's text. */
export function kernelRecord(key: DeviceKey, boot: string): string {
  const device = online(key);
  if (!hasStatus(device)) throw `${keyString(key)} does not report its status`;
  const log = boardFor(device).kernelLogs.find((l) => l.boot === boot);
  if (!log) throw `no kernel log kept for boot ${boot}`;
  return log.text;
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
  if (e.kind === "update.rolled-back" || e.kind === "kernel.pstore" || (e.kind === "boot" && e.data.previous_clean === "false")) return "warning";
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
    case "usb-route-on":
    case "usb-route-off": {
      const on = action === "usb-route-on";
      if (device.attributes) device.attributes.usb_route = on ? "active" : "off";
      event(board, now, "usb-route", "orion", `USB route ${on ? "on" : "off"}`);
      changed(device);
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
  board.reset = "reset";
  event(board, board.bootedAt, "boot", "local", `boot ${board.bootCount}, slot ${board.update.slot_active}, kernel 7.2.9-v8`, {
    count: String(board.bootCount),
    slot: board.update.slot_active ?? "",
    previous_clean: "true",
    reset: "reset",
  });
  restart(device.key);
}
