// Live readings for the devices on screen. Views call `watch` with the keys
// they show; while anything watches a device that reports telemetry, its
// readings come in and a history is kept for charts. Devices without the
// capability are never asked.
//
// Where the board's own Orion node streams its readings, they are pushed:
// the last five minutes at once (a full chart on open), then a row every
// 500 ms. A poll every STREAM_POLL_MS fills in what the stream doesn't carry
// (voltage, fps); when all it brings besides the stream is readings that
// only drift (temperatures, the fan, uptime: a board whose readings come from
// its status, which the board's status view reads anyway), only every
// STREAM_IDLE_MS. Elsewhere the device is polled, only as often as the most
// eager view showing it asks (FAST for its own page, SLOW for lists and the
// sidebar), never twice for two views, not again when a view re-subscribes,
// and not while the window is hidden.

import { SvelteMap } from "svelte/reactivity";
import { api, errorText, keyString, type DeviceRecord, type LiveFrame, type Metric } from "#lib/api/client.ts";
import { devices } from "./devices.svelte";

/** For a device's own page and the Monitor, when polled. */
export const FAST_MS = 1000;
/** For lists, tiles and the sidebar. */
export const SLOW_MS = 2000;
const TICK_MS = 250;
/** Readings taken at FAST_MS before a device drops to the rate its views ask. */
const WARM_UP = 3;
/** Samples kept per metric: 15 minutes of streamed rows (the Monitor's longest view). */
const HISTORY = 1800;
/** What series() gives by default: two minutes at the fast rate. */
const SHOWN = 120;
/** While streamed, how often a device is polled for the rest. */
const STREAM_POLL_MS = 5000;
/** While streamed and the last poll brought only readings that drift. */
const STREAM_IDLE_MS = 60_000;

/** Readings that change slowly enough to read once a minute. */
function drifts(metricId: string): boolean {
  return metricId === "temp" || metricId.startsWith("temp.") || metricId === "fan" || metricId === "fan.rpm" || metricId === "uptime";
}
/** How long before a board without a stream is asked again. */
const STREAM_RETRY_MS = 30_000;
/** A stream outlives its last view this long, so a view re-subscribing keeps it. */
const STREAM_LINGER_MS = 2000;

export function hasTelemetry(record: DeviceRecord | undefined): boolean {
  return !!record && record.presence === "online" && record.capabilities.includes("telemetry");
}

class LiveStore {
  /** Latest readings per device key. */
  metrics = new SvelteMap<string, Metric[]>();
  /** Recent values per device key, then metric id, oldest first. */
  history = new SvelteMap<string, Record<string, number[]>>();
  /** When each poll answered, per device key (ms), oldest first; a series lines up with its end. */
  times = new SvelteMap<string, number[]>();
  errors = new SvelteMap<string, string>();

  /** The intervals the views showing each device ask for. */
  private watchers = new Map<string, number[]>();
  private timer: ReturnType<typeof setInterval> | null = null;
  private inFlight = new Set<string>();
  /** When each device was last asked (ms). */
  private askedAt = new Map<string, number>();
  /** Devices whose readings are pushed: how to stop, and the metric ids the stream carries. */
  private streams = new Map<string, { stop: () => void; carries: Set<string> }>();
  private starting = new Set<string>();
  /** When a stream was last tried per device (ms). */
  private streamTried = new Map<string, number>();
  private lingering = new Map<string, ReturnType<typeof setTimeout>>();
  /** Streamed devices whose last poll brought, besides the stream, only readings that drift. */
  private covered = new Set<string>();

  /** Polls these devices every `every` ms while watched; returns a function that stops. */
  watch(ids: string[], every = FAST_MS): () => void {
    for (const id of ids) {
      this.watchers.set(id, [...(this.watchers.get(id) ?? []), every]);
      clearTimeout(this.lingering.get(id));
      this.lingering.delete(id);
      this.stream(id);
    }
    this.ensureTimer();
    this.tick();
    return () => {
      for (const id of ids) {
        const left = this.watchers.get(id) ?? [];
        const at = left.indexOf(every);
        if (at >= 0) left.splice(at, 1);
        if (left.length === 0) {
          this.watchers.delete(id);
          this.lingering.set(
            id,
            setTimeout(() => {
              this.lingering.delete(id);
              if (!this.watchers.has(id)) this.unstream(id);
            }, STREAM_LINGER_MS),
          );
        }
      }
      if (this.watchers.size === 0 && this.timer) {
        clearInterval(this.timer);
        this.timer = null;
      }
    };
  }

  /**
   * Whether every given device that reports telemetry has answered at least
   * once (or failed), so a view can show all its readings together rather
   * than one by one.
   */
  settled(records: DeviceRecord[]): boolean {
    return records.filter(hasTelemetry).every((r) => {
      const id = keyString(r.key);
      return this.metrics.has(id) || this.errors.has(id);
    });
  }

  metric(id: string, metricId: string): Metric | undefined {
    return this.metrics.get(id)?.find((m) => m.id === metricId);
  }

  /** The last `last` values (SHOWN: what sparklines and device charts draw). */
  series(id: string, metricId: string, last = SHOWN): number[] {
    return (this.history.get(id)?.[metricId] ?? []).slice(-last);
  }

  /** When the device's recent readings were taken; a series' values line up with its end. */
  seriesTimes(id: string, last = SHOWN): number[] {
    return (this.times.get(id) ?? []).slice(-last);
  }

  /** The readings taken since `since` (ms), each with its time. */
  window(id: string, metricId: string, since: number): { t: number; v: number }[] {
    const values = this.history.get(id)?.[metricId] ?? [];
    const times = this.times.get(id) ?? [];
    const offset = times.length - values.length;
    const out: { t: number; v: number }[] = [];
    for (let i = 0; i < values.length; i++) {
      const t = times[i + offset];
      if (t !== undefined && t >= since) out.push({ t, v: values[i] });
    }
    return out;
  }

  /** Whether the device's readings are pushed right now. */
  streamed(id: string): boolean {
    return this.streams.has(id);
  }

  /** Starts the device's stream, unless it has one or had none a moment ago. */
  private stream(id: string) {
    const record = devices.get(id);
    if (!hasTelemetry(record) || this.streams.has(id) || this.starting.has(id)) return;
    if (Date.now() - (this.streamTried.get(id) ?? -Infinity) < STREAM_RETRY_MS) return;
    this.streamTried.set(id, Date.now());
    this.starting.add(id);
    api
      .streamLive(record!.key, (frame) => this.frame(id, frame))
      .then((stop) => {
        if (!stop) return;
        if (this.watchers.has(id)) this.streams.set(id, { stop, carries: this.streams.get(id)?.carries ?? new Set() });
        else stop();
      })
      .catch(() => {})
      .finally(() => this.starting.delete(id));
  }

  private unstream(id: string) {
    this.streams.get(id)?.stop();
    this.streams.delete(id);
    this.covered.delete(id);
  }

  /** Pushed rows: replace (backfill) or extend the history, and refresh the readings. */
  private frame(id: string, frame: LiveFrame) {
    if (frame.kind === "gone") {
      this.unstream(id);
      this.streamTried.set(id, Date.now());
      return;
    }
    const entry = this.streams.get(id);
    const carries = entry?.carries ?? new Set<string>();
    for (const m of frame.latest) carries.add(m.id);
    if (entry) entry.carries = carries;
    else this.streams.set(id, { stop: () => {}, carries });

    const past = frame.backfill ? {} : (this.history.get(id) ?? {});
    const polled = this.metrics.get(id) ?? [];
    const ids = new Set([...Object.keys(past), ...polled.map((m) => m.id), ...frame.rows.flatMap((r) => Object.keys(r.values))]);
    const next: Record<string, number[]> = {};
    for (const metricId of ids) {
      const values = [...(past[metricId] ?? [])];
      // A value the rows don't have carries the last one (a polled metric, a gap).
      let last = polled.find((m) => m.id === metricId)?.value ?? values.at(-1);
      for (const row of frame.rows) {
        const value = row.values[metricId] ?? last;
        if (value === undefined) continue;
        values.push(value);
        last = value;
      }
      next[metricId] = values.slice(-HISTORY);
    }
    this.history.set(id, next);
    const times = frame.backfill ? [] : (this.times.get(id) ?? []);
    this.times.set(id, [...times, ...frame.rows.map((r) => r.t)].slice(-HISTORY));
    if (frame.latest.length > 0) {
      const fresh = new Map(frame.latest.map((m) => [m.id, m]));
      const kept = polled.filter((m) => !fresh.has(m.id));
      this.metrics.set(id, [...frame.latest, ...kept]);
    }
    this.errors.delete(id);
  }

  private ensureTimer() {
    if (this.timer || typeof window === "undefined") return;
    this.timer = setInterval(() => this.tick(), TICK_MS);
  }

  /** Reads the watched devices that are due. */
  private tick() {
    if (typeof document !== "undefined" && document.hidden) return;
    const now = Date.now();
    // The first few readings come fast whatever the view asks, so its
    // charts have a line at once.
    const every = (id: string, asks: number[]) => {
      if (this.streams.has(id)) return this.covered.has(id) ? STREAM_IDLE_MS : STREAM_POLL_MS;
      return (this.times.get(id)?.length ?? 0) < WARM_UP ? FAST_MS : Math.min(...asks);
    };
    for (const id of this.watchers.keys()) this.stream(id);
    const due = [...this.watchers].filter(([id, asks]) => now - (this.askedAt.get(id) ?? 0) >= every(id, asks) - TICK_MS / 2);
    if (due.length > 0) void this.poll(due.map(([id]) => id));
  }

  private async poll(ids: string[]) {
    await Promise.all(
      ids.map(async (id) => {
        const record = devices.get(id);
        if (!hasTelemetry(record) || this.inFlight.has(id)) {
          if (record && !hasTelemetry(record)) this.metrics.delete(id);
          return;
        }
        this.inFlight.add(id);
        this.askedAt.set(id, Date.now());
        try {
          const readings = await api.deviceTelemetry(record!.key);
          const streamed = this.streams.get(id);
          if (streamed) {
            // The stream has its own readings and rows: take only the rest.
            const rest = readings.filter((m) => !streamed.carries.has(m.id));
            // Matching ids isn't enough: a board's status names its fan
            // `fan` in rpm where the stream may say `fan.rpm`.
            if (rest.every((m) => drifts(m.id))) this.covered.add(id);
            else this.covered.delete(id);
            const pushed = (this.metrics.get(id) ?? []).filter((m) => streamed.carries.has(m.id));
            this.metrics.set(id, [...pushed, ...rest]);
            this.errors.delete(id);
            return;
          }
          this.metrics.set(id, readings);
          this.errors.delete(id);
          const past = this.history.get(id) ?? {};
          const next: Record<string, number[]> = {};
          for (const m of readings) next[m.id] = [...(past[m.id] ?? []), m.value].slice(-HISTORY);
          this.history.set(id, next);
          this.times.set(id, [...(this.times.get(id) ?? []), Date.now()].slice(-HISTORY));
        } catch (error) {
          this.errors.set(id, errorText(error));
        } finally {
          this.inFlight.delete(id);
        }
      }),
    );
  }
}

export const live = new LiveStore();

/**
 * Watches the given records' telemetry for the life of the calling effect,
 * every `every` ms (FAST_MS for a device's own page, SLOW_MS for lists).
 */
export function watchLive(records: DeviceRecord[], every = FAST_MS): () => void {
  return live.watch(records.filter(hasTelemetry).map((r) => keyString(r.key)), every);
}
