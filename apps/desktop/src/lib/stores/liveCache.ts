// The last readings, kept across restarts so the app opens on them at once
// (marked stale) instead of on placeholders: the live values replace them in
// place as each device answers. Per viewer, in the web view's own storage;
// a missing or broken cache just means placeholders, as before.

import type { Metric } from "#lib/api/client.ts";

const KEY = "atlas.live.v1";
/** Samples kept per metric: enough for a sparkline's worth. */
const KEEP = 60;
/** At most this many devices, the most recently read first. */
const DEVICES = 48;
/** Older than this, a cache says nothing worth showing. */
const MAX_AGE_MS = 24 * 3600_000;

export interface CachedDevice {
  metrics: Metric[];
  history: Record<string, number[]>;
  times: number[];
  /** When the newest reading was taken (ms). */
  at: number;
}

export function loadLiveCache(now = Date.now()): Map<string, CachedDevice> {
  const out = new Map<string, CachedDevice>();
  try {
    const raw = globalThis.localStorage?.getItem(KEY);
    if (!raw) return out;
    const parsed = JSON.parse(raw) as { v: number; devices: Record<string, CachedDevice> };
    if (parsed?.v !== 1 || typeof parsed.devices !== "object") return out;
    for (const [id, d] of Object.entries(parsed.devices)) {
      if (!d || !Array.isArray(d.metrics) || typeof d.at !== "number" || now - d.at > MAX_AGE_MS) continue;
      out.set(id, { metrics: d.metrics, history: d.history ?? {}, times: Array.isArray(d.times) ? d.times : [], at: d.at });
    }
  } catch {
    // Unreadable or blocked storage: start without it.
  }
  return out;
}

export function saveLiveCache(devices: Iterable<[string, CachedDevice]>) {
  try {
    const kept = [...devices]
      .filter(([, d]) => d.metrics.length > 0)
      .sort((a, b) => b[1].at - a[1].at)
      .slice(0, DEVICES)
      .map(([id, d]) => {
        const history: Record<string, number[]> = {};
        for (const [metric, values] of Object.entries(d.history)) history[metric] = values.slice(-KEEP);
        return [id, { metrics: d.metrics, history, times: d.times.slice(-KEEP), at: d.at }] as const;
      });
    globalThis.localStorage?.setItem(KEY, JSON.stringify({ v: 1, devices: Object.fromEntries(kept) }));
  } catch {
    // Full or blocked storage: the next launch shows placeholders.
  }
}
