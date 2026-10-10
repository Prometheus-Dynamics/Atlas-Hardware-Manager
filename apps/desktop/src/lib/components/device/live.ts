// Live readings for the Hardware tab: a ring buffer per device of what the
// board streams (its monotonic time and a value per channel), sized for the
// longest window at the fastest rate, and drawn from directly (no
// reactivity per sample).

import type { HardwareReading, LiveChannel, LiveDevice } from "#lib/api/client.ts";

/** Samples kept per device: 40 s at 100 Hz. */
export const CAPACITY = 4096;
/** The windows a chart can show, seconds. */
export const WINDOWS = [2, 10, 30] as const;

export class LiveSeries {
  readonly id: string;
  readonly channels: LiveChannel[];
  readonly period_ms: number;
  /** The board's monotonic time of each sample, µs. */
  readonly t = new Float64Array(CAPACITY);
  /** One buffer per channel; NaN where it wasn't read. */
  readonly v: Float64Array[];
  /** Where the next sample goes, and how many there are. */
  head = 0;
  count = 0;
  /** This computer's clock (ms) when the newest sample arrived, to tell when one was read. */
  arrivedMs = 0;

  constructor(device: LiveDevice) {
    this.id = device.id;
    this.channels = device.channels;
    this.period_ms = device.period_ms ?? 0;
    this.v = device.channels.map(() => new Float64Array(CAPACITY).fill(Number.NaN));
  }

  push(t_us: number, values: (number | null)[]) {
    this.t[this.head] = t_us;
    for (let c = 0; c < this.v.length; c++) this.v[c][this.head] = values[c] ?? Number.NaN;
    this.head = (this.head + 1) % CAPACITY;
    this.count = Math.min(this.count + 1, CAPACITY);
    this.arrivedMs = Date.now();
  }

  /** The buffer index of the i-th sample, oldest first. */
  index(i: number): number {
    return (this.head - this.count + i + CAPACITY) % CAPACITY;
  }

  newestUs(): number {
    return this.count ? this.t[this.index(this.count - 1)] : 0;
  }

  /** This computer's time (ms) for a board time, by the newest sample's arrival. */
  wallMs(t_us: number): number {
    return this.arrivedMs - (this.newestUs() - t_us) / 1000;
  }

  /** The first sample at or after `t_us` (binary search; times only grow). */
  firstFrom(t_us: number): number {
    let low = 0;
    let high = this.count;
    while (low < high) {
      const mid = (low + high) >> 1;
      if (this.t[this.index(mid)] < t_us) low = mid + 1;
      else high = mid;
    }
    return low;
  }

  /** The newest value of each channel, as readings. */
  latest(): HardwareReading[] {
    const at = this.count ? this.index(this.count - 1) : -1;
    return this.channels.map((channel, c) => ({
      name: channel.name,
      unit: channel.unit,
      value: at >= 0 && !Number.isNaN(this.v[c][at]) ? this.v[c][at] : null,
    }));
  }

  /** The channels grouped by unit (one chart each), in order. */
  groups(): { unit: string; channels: number[] }[] {
    const groups: { unit: string; channels: number[] }[] = [];
    this.channels.forEach((channel, c) => {
      const group = groups.find((g) => g.unit === channel.unit);
      if (group) group.channels.push(c);
      else groups.push({ unit: channel.unit, channels: [c] });
    });
    return groups;
  }

  /** Samples per second over the last second. */
  rate(): number {
    if (this.count < 2) return 0;
    const newest = this.newestUs();
    return this.count - this.firstFrom(newest - 1_000_000);
  }
}

/** How often to ask for a device: motion sensors at 100 Hz, the rest at 10 Hz. */
export function periodFor(className: string): number {
  return ["imu", "accelerometer", "gyroscope", "magnetometer"].includes(className) ? 10 : 100;
}

/** "14:02:31.284". */
export function clockText(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number, w = 2) => String(n).padStart(w, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`;
}
