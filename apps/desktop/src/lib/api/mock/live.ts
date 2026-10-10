// The A/B Raze's live readings for the browser mock, as board-stream sends
// them: a `devices` frame, then batches of samples every 16 ms at each
// device's period (the IMU at 100 Hz), on the board's monotonic clock.

import type { DeviceKey, HardwareFrame, LiveChannel, LiveDevice } from "../types";

type Model = { class: string; model: string; channels: LiveChannel[]; value: (t: number, i: number) => number };

const axes = (prefix: string, unit: string) => ["x", "y", "z"].map((axis) => ({ name: `${prefix}.${axis}`, unit }));
const noise = (scale: number) => (Math.random() - 0.5) * 2 * scale;

const MODELS: Record<string, Model> = {
  imu: {
    class: "imu",
    model: "bmi088",
    channels: [...axes("acceleration", "m/s²"), ...axes("angular_rate", "rad/s")],
    // A board being handled: slow tilt, a bump every few seconds, sensor noise.
    value: (t, i) => {
      const bump = Math.exp(-(((t % 4) - 2) ** 2) * 20);
      if (i < 3) return [0.4 * Math.sin(t * 1.3), 0.3 * Math.cos(t * 0.9), 9.81][i] + bump * [2, -1.5, 3][i] + noise(0.05);
      return [0.2 * Math.cos(t * 1.3), -0.15 * Math.sin(t * 0.9), 0.05 * Math.sin(t * 3)][i - 3] + bump * 1.2 + noise(0.01);
    },
  },
  magnetometer: {
    class: "magnetometer",
    model: "bmm150",
    // Tesla, as lemnosd reports it (about 50 µT here).
    channels: axes("magnetic_field", "T"),
    value: (t, i) => [21.4e-6, -3.2e-6, -44.8e-6][i] + 1e-6 * Math.sin(t / 5 + i) + noise(0.3e-6),
  },
  // lemnosd's fusion: the same slow tilt as the IMU above, and a heading drifting a little.
  orientation: {
    class: "orientation",
    model: "fusion",
    channels: [
      ...["w", "x", "y", "z"].map((axis) => ({ name: `quaternion.${axis}`, unit: "" })),
      { name: "roll", unit: "rad" },
      { name: "pitch", unit: "rad" },
      { name: "yaw", unit: "rad" },
      { name: "confidence.imu", unit: "" },
      { name: "confidence.magnetometer", unit: "" },
      { name: "magnetic_disturbance", unit: "" },
    ],
    value: (t, i) => {
      const roll = 0.04 * Math.sin(t * 1.3);
      const pitch = 0.03 * Math.cos(t * 0.9);
      const yaw = 0.6 + 0.2 * Math.sin(t / 7);
      // Hamilton, scalar first, from intrinsic ZYX (yaw, pitch, roll).
      const [cr, sr, cp, sp, cy, sy] = [Math.cos(roll / 2), Math.sin(roll / 2), Math.cos(pitch / 2), Math.sin(pitch / 2), Math.cos(yaw / 2), Math.sin(yaw / 2)];
      const q = [cr * cp * cy + sr * sp * sy, sr * cp * cy - cr * sp * sy, cr * sp * cy + sr * cp * sy, cr * cp * sy - sr * sp * cy];
      return [...q, roll, pitch, yaw, 0.94, 0.88, 0][i];
    },
  },
  // The status ring as board-stream sends a light: geometry, then each
  // logical LED's colour (0xWWRRGGBB). A green spinner over a dim base.
  "status-ring": {
    class: "light",
    model: "ws2812",
    channels: [
      { name: "offset", unit: "" },
      { name: "clockwise", unit: "" },
      { name: "brightness", unit: "" },
      ...Array.from({ length: 16 }, (_, i) => ({ name: `led.${i}`, unit: "" })),
    ],
    value: (t, i) => {
      if (i < 3) return [5, 1, 1][i];
      const led = i - 3;
      const head = (t * 8) % 16;
      const d = (led - head + 16) % 16;
      const level = Math.max(0.06, 1 - d / 6);
      const g = Math.round(200 * level);
      const r = Math.round(30 * level);
      return (r << 16) | (g << 8) | Math.round(60 * level);
    },
  },
  power: {
    class: "power",
    model: "ina238",
    channels: [
      { name: "voltage", unit: "V" },
      { name: "current", unit: "A" },
      { name: "power", unit: "W" },
    ],
    value: (t, i) => {
      const amps = 1.5 + Math.sin(t / 3) * 0.3 + noise(0.02);
      return [12.02 - amps * 0.02 + noise(0.005), amps, 12 * amps][i];
    },
  },
  "cpu-thermal": {
    class: "temperature",
    model: "bcm2712",
    channels: [{ name: "temperature", unit: "°C" }],
    value: (t) => 52 + Math.sin(t / 20) * 2 + noise(0.1),
  },
  fan: {
    class: "fan",
    model: "pwmfan",
    channels: [{ name: "rpm", unit: "rpm" }],
    value: (t) => 4200 + Math.sin(t / 15) * 300 + noise(20),
  },
};

export function streamHardware(
  key: DeviceKey,
  devices: [string, number][],
  onFrame: (frame: HardwareFrame) => void,
): Promise<(() => void) | null> {
  if (key.serial !== "8f3a1c2d") return Promise.resolve(null);
  const described: LiveDevice[] = devices.map(([id, period]) => {
    const model = MODELS[id];
    return model
      ? { id, class: model.class, model: model.model, status: "available", period_ms: period, channels: model.channels }
      : { id, missing: true, channels: [] };
  });
  // The board's monotonic clock: it has been up a while, and it keeps
  // counting across streams (a new stream mustn't step back in time).
  const start = 0;
  const boot_us = 3_600_000_000;
  const next = new Map(devices.map(([id]) => [id, performance.now()]));
  setTimeout(() => onFrame({ type: "devices", devices: described }), 0);
  const timer = setInterval(() => {
    const now = performance.now() - start;
    for (const [id, period] of devices) {
      const model = MODELS[id];
      if (!model) continue;
      const samples: [number, (number | null)[]][] = [];
      let due = next.get(id) ?? 0;
      while (due <= now) {
        const t = due / 1000;
        samples.push([boot_us + Math.round(due * 1000), model.channels.map((_, i) => model.value(t, i))]);
        due += period;
      }
      next.set(id, due);
      if (samples.length) onFrame({ type: "samples", device: id, samples });
    }
  }, 16);
  return Promise.resolve(() => clearInterval(timer));
}
