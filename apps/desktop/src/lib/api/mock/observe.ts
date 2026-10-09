// Simulated telemetry, logs, and fleet history for the browser mock.
// Mirrors crates/atlas-driver-mock/src/observe.rs and atlas-core's activity.

import type { ActivityEntry, ActivityKind, ActivityLevel, DeviceKey, LogLevel, LogLine, Metric } from "../types";
import { keyString } from "../types";
import { emit } from "./bus";
import { inventory, simDevice, type SimDevice } from "./data";

const booted = new Map<string, number>();
const START = Date.now() - (3 * 3600 + 17 * 60) * 1000;

function bootTime(key: DeviceKey): number {
  return booted.get(keyString(key)) ?? START;
}

export function restart(key: DeviceKey) {
  booted.set(keyString(key), Date.now());
}

function phase(device: SimDevice): number {
  let sum = 0;
  for (const c of device.key.serial) sum += c.charCodeAt(0);
  return (sum % 97) / 97;
}

function wave(device: SimDevice, period: number, low: number, high: number): number {
  const t = Date.now() / 1000;
  const x = (t / period + phase(device)) * Math.PI * 2;
  const jitter = Math.sin(t * 7.3 + phase(device) * 13) * 0.06;
  const unit = Math.min(1, Math.max(0, (Math.sin(x) + 1) / 2 + jitter));
  return low + (high - low) * unit;
}

const round = (value: number, places = 0) => Math.round(value * 10 ** places) / 10 ** places;

function metric(id: string, label: string, value: number, unit: string | null, max: number | null = null, warn: number | null = null): Metric {
  return { id, label, value, unit, max, warn_above: warn };
}

export function isVision(device: SimDevice): boolean {
  return device.key.family === "sim-helios" || device.key.family === "raze";
}

export function metrics(device: SimDevice): Metric[] {
  const uptime = Math.round((Date.now() - bootTime(device.key)) / 1000);
  if (isVision(device)) {
    const hot = device.key.serial === "H-1003" ? 9 : 0;
    const cpu = wave(device, 23, 18, 71);
    const temp = 42 + cpu * 0.32 + wave(device, 61, 0, 3) + hot;
    const cores = [0, 1, 2, 3].map((n) => Math.min(100, Math.max(0, cpu + wave(device, 5 + n * 3, -25, 25))));
    const memory = round(wave(device, 90, 31, 44));
    return [
      { ...metric("cpu", "CPU", round(cpu), "%", 100, 90), detail: `4 cores · load ${[1, 0.9, 0.8].map((f) => ((cpu / 25) * f).toFixed(2)).join(" · ")}` },
      ...cores.map((value, n) => metric(`cpu.core.${n}`, `Core ${n}`, round(value), "%", 100, 90)),
      metric("temp", "Temperature", round(temp, 1), "°C", 85, 75),
      metric("fan", "Fan", round(1800 + (temp - 40) * 95, -1), "rpm", 6000),
      { ...metric("memory", "Memory", memory, "%", 100, 90), detail: `${((memory / 100) * 4).toFixed(1)} GiB of 4.0 GiB` },
      metric("fps", "Vision", round(wave(device, 9, 52, 60)), "fps", 60),
      metric("uptime", "Uptime", uptime, "s"),
    ];
  }
  const current = wave(device, 7, 0.4, 3.2);
  return [
    metric("voltage", "Bus voltage", round(12.6 - current * 0.18, 2), "V", 14),
    metric("current", "Current", round(current, 2), "A", 20, 15),
    metric("temp", "Temperature", round(34 + current * 2.1, 1), "°C", 85, 70),
    metric("uptime", "Uptime", uptime, "s"),
  ];
}

const CAMERA_LINES: [LogLevel, string, string][] = [
  ["info", "photonvision", 'Pipeline "apriltag" running at 58 fps'],
  ["info", "networktables", "Connected to robot at 10.0.0.2"],
  ["debug", "camera", "Exposure adjusted to 12 ms"],
  ["info", "photonvision", "Target 7 acquired"],
  ["info", "systemd", "Started PhotonVision service"],
  ["warning", "thermal", "SoC above 70 °C; fan raised to 80%"],
  ["info", "photonvision", "Target 7 lost"],
  ["debug", "camera", "Frame queue depth 2"],
];

const BOARD_LINES: [LogLevel, string, string][] = [
  ["info", "can", "Heartbeat ok"],
  ["info", "motor", "Closed loop enabled"],
  ["debug", "adc", "Bus voltage sample 12.4 V"],
  ["warning", "can", "One frame retried"],
  ["info", "motor", "Setpoint reached"],
];

export function logLines(device: SimDevice, count: number): LogLine[] {
  const templates = isVision(device) ? CAMERA_LINES : BOARD_LINES;
  const step = 2000;
  const now = Math.floor(Date.now() / step) * step;
  const since = bootTime(device.key);
  const lines: LogLine[] = [];
  for (let back = count - 1; back >= 0; back--) {
    const at = now - back * step;
    if (at < since) continue;
    const [level, source, message] = templates[Math.floor(at / step) % templates.length];
    lines.push({ at_ms: at, level, source, message });
  }
  return lines;
}

/** Fleet history, oldest first, like atlas-core keeps it. */
export const activity: ActivityEntry[] = [];

export function record(kind: ActivityKind, level: ActivityLevel, key: DeviceKey | null, message: string, at = Date.now()) {
  const robot = key ? (inventory.get(keyString(key))?.robot ?? null) : null;
  const entry: ActivityEntry = { at_ms: at, kind, level, device: key, robot, message };
  activity.push(entry);
  if (activity.length > 300) activity.shift();
  emit({ type: "activity", entry });
}

/** A little history from "earlier", so the feed is not empty on first open. */
export function seedHistory() {
  if (activity.length > 0) return;
  const now = Date.now();
  const h = 3_600_000;
  const past: [number, ActivityKind, ActivityLevel, string, string][] = [
    [26 * h, "update-result", "success", "sim-helios:H-1001", "cam-front updated to 2026.2.4"],
    [26 * h, "update-result", "success", "sim-helios:H-1002", "cam-left updated to 2026.2.4"],
    [25.5 * h, "update-result", "warning", "sim-helios:H-1003", "cam-rear rolled back its update: no boot confirmation within the timeout"],
    [5 * h, "device-offline", "warning", "sim-mcu:M-2002", "drive-mcu-2 went offline"],
    [4.8 * h, "device-online", "info", "sim-mcu:M-2002", "drive-mcu-2 came online"],
    [3.3 * h, "action-run", "info", "sim-helios:H-1001", "Restart on cam-front"],
  ];
  for (const [ago, kind, level, id, message] of past) {
    const [family, serial] = id.split(":");
    activity.push({ at_ms: now - ago, kind, level, device: { family, serial }, robot: null, message });
  }
}

export function online(key: DeviceKey): SimDevice {
  const device = simDevice(key);
  const stored = inventory.get(keyString(key));
  if (!device || stored?.presence !== "online" || device.mode === "recovery")
    throw `device did not respond: ${keyString(key)}`;
  return device;
}
