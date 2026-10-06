// Live readings for the devices on screen. Views call `watch` with the keys
// they show; while anything watches a device that reports telemetry, it is
// polled and a short history is kept for sparklines. Devices without the
// capability are never asked.

import { SvelteMap } from "svelte/reactivity";
import { api, errorText, keyString, type DeviceRecord, type Metric } from "#lib/api/client.ts";
import { devices } from "./devices.svelte";

const POLL_MS = 2000;
/** Samples kept per metric: about two minutes at the poll rate. */
const HISTORY = 60;

export function hasTelemetry(record: DeviceRecord | undefined): boolean {
  return !!record && record.presence === "online" && record.capabilities.includes("telemetry");
}

class LiveStore {
  /** Latest readings per device key. */
  metrics = new SvelteMap<string, Metric[]>();
  /** Recent values per device key, then metric id, oldest first. */
  history = new SvelteMap<string, Record<string, number[]>>();
  errors = new SvelteMap<string, string>();

  private watchers = new Map<string, number>();
  private timer: ReturnType<typeof setInterval> | null = null;
  private inFlight = new Set<string>();

  /** Starts polling these devices; returns a function that stops. */
  watch(ids: string[]): () => void {
    for (const id of ids) this.watchers.set(id, (this.watchers.get(id) ?? 0) + 1);
    this.ensureTimer();
    void this.poll(ids);
    return () => {
      for (const id of ids) {
        const left = (this.watchers.get(id) ?? 1) - 1;
        if (left <= 0) this.watchers.delete(id);
        else this.watchers.set(id, left);
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

  series(id: string, metricId: string): number[] {
    return this.history.get(id)?.[metricId] ?? [];
  }

  private ensureTimer() {
    if (this.timer || typeof window === "undefined") return;
    this.timer = setInterval(() => void this.poll([...this.watchers.keys()]), POLL_MS);
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
        try {
          const readings = await api.deviceTelemetry(record!.key);
          this.metrics.set(id, readings);
          this.errors.delete(id);
          const past = this.history.get(id) ?? {};
          const next: Record<string, number[]> = {};
          for (const m of readings) next[m.id] = [...(past[m.id] ?? []), m.value].slice(-HISTORY);
          this.history.set(id, next);
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

/** Watches the given records' telemetry for the life of the calling effect. */
export function watchLive(records: DeviceRecord[]): () => void {
  return live.watch(records.filter(hasTelemetry).map((r) => keyString(r.key)));
}
