// Live readings for the devices on screen. Views call `watch` with the keys
// they show; while anything watches a device that reports telemetry, it is
// polled and a short history is kept for sparklines. Devices without the
// capability are never asked.
//
// Each reading is a request the board answers with a process of its own, so
// a device is read only as often as the most eager view showing it asks
// (FAST for its own page, SLOW for lists and the sidebar), never twice for
// two views, not again when a view re-subscribes, and not while the window
// is hidden.

import { SvelteMap } from "svelte/reactivity";
import { api, errorText, keyString, type DeviceRecord, type Metric } from "#lib/api/client.ts";
import { devices } from "./devices.svelte";

/** For a device's own page. */
export const FAST_MS = 3000;
/** For lists, tiles and the sidebar. */
export const SLOW_MS = 10_000;
const TICK_MS = 1000;
/** Readings taken at FAST_MS before a device drops to the rate its views ask. */
const WARM_UP = 3;
/** Samples kept per metric: 15 minutes at the fast rate (the Monitor's longest view). */
const HISTORY = 300;
/** What series() gives by default: about three minutes at the fast rate. */
const SHOWN = 60;

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

  /** Polls these devices every `every` ms while watched; returns a function that stops. */
  watch(ids: string[], every = FAST_MS): () => void {
    for (const id of ids) this.watchers.set(id, [...(this.watchers.get(id) ?? []), every]);
    this.ensureTimer();
    this.tick();
    return () => {
      for (const id of ids) {
        const left = this.watchers.get(id) ?? [];
        const at = left.indexOf(every);
        if (at >= 0) left.splice(at, 1);
        if (left.length === 0) this.watchers.delete(id);
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
    const every = (id: string, asks: number[]) => ((this.times.get(id)?.length ?? 0) < WARM_UP ? FAST_MS : Math.min(...asks));
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
