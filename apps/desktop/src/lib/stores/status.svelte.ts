// What a board is doing and how it is (`status`), and its merged history,
// for the device panel. A view calls `watch` while it shows a device: the
// status is polled (every few seconds while an update runs, slower
// otherwise), and the history is re-read when Atlas says the board has new
// events. Devices without the capability are never asked.

import { SvelteMap } from "svelte/reactivity";
import {
  api,
  errorText,
  keyString,
  type DeviceKey,
  type DeviceRecord,
  type DeviceStatus,
  type EventSource,
  type HistoryEntry,
  type UpdateState,
} from "#lib/api/client.ts";

const BUSY_MS = 3000;
const IDLE_MS = 15000;
const HISTORY_LIMIT = 300;

export function hasStatus(record: DeviceRecord | undefined): boolean {
  return !!record && record.presence === "online" && record.capabilities.includes("status");
}

/** An update is under way: staging, restarting into it, or on trial. */
export function updateBusy(update: UpdateState | null | undefined): boolean {
  return !!update && ["staging", "rebooting", "trying"].includes(update.state);
}

/** Who did something, in words. */
export function sourceLabel(source: EventSource | null | undefined): string {
  switch (source) {
    case "atlas":
      return "you";
    case "orion":
      return "Orion";
    case "local":
      return "on the board";
    default:
      return "unknown";
  }
}

class StatusStore {
  byDevice = new SvelteMap<string, DeviceStatus>();
  errors = new SvelteMap<string, string>();
  /** When each device's status last answered (ms, this computer). */
  readAt = new SvelteMap<string, number>();
  history = new SvelteMap<string, HistoryEntry[]>();

  private watched = new Map<string, { key: DeviceKey; status: boolean; count: number }>();
  private timers = new Map<string, ReturnType<typeof setTimeout>>();
  private inFlight = new Set<string>();

  get(record: DeviceRecord): DeviceStatus | undefined {
    return this.byDevice.get(keyString(record.key));
  }

  /** Follows one device while shown; returns a function that stops. */
  watch(record: DeviceRecord): () => void {
    const id = keyString(record.key);
    const status = hasStatus(record);
    const entry = this.watched.get(id);
    if (entry) {
      entry.count += 1;
      entry.status ||= status;
    } else {
      this.watched.set(id, { key: record.key, status, count: 1 });
    }
    void this.loadHistory(record.key);
    if (status) void this.poll(id);
    return () => {
      const left = this.watched.get(id);
      if (!left) return;
      left.count -= 1;
      if (left.count > 0) return;
      this.watched.delete(id);
      const timer = this.timers.get(id);
      if (timer) clearTimeout(timer);
      this.timers.delete(id);
    };
  }

  /** Reads the status now (after an action, say), then keeps polling. */
  async refresh(key: DeviceKey) {
    await this.poll(keyString(key));
  }

  private async poll(id: string) {
    const entry = this.watched.get(id);
    if (!entry?.status || this.inFlight.has(id)) return;
    const timer = this.timers.get(id);
    if (timer) clearTimeout(timer);
    this.inFlight.add(id);
    try {
      const status = await api.deviceStatus(entry.key);
      this.byDevice.set(id, status);
      this.readAt.set(id, Date.now());
      this.errors.delete(id);
    } catch (error) {
      this.errors.set(id, errorText(error));
    } finally {
      this.inFlight.delete(id);
    }
    if (!this.watched.has(id)) return;
    // A staged update is likely to be applied soon: follow it closely too.
    const update = this.byDevice.get(id)?.update;
    const every = updateBusy(update) || update?.state === "staged" ? BUSY_MS : IDLE_MS;
    this.timers.set(
      id,
      setTimeout(() => void this.poll(id), every),
    );
  }

  async loadHistory(key: DeviceKey) {
    const id = keyString(key);
    try {
      this.history.set(id, await api.deviceHistory(key, HISTORY_LIMIT));
    } catch {
      // No history is shown rather than an error.
    }
  }

  /** The board pushed a change: read a watched device's status now. */
  onPushed(key: DeviceKey) {
    if (this.watched.has(keyString(key))) void this.poll(keyString(key));
  }

  /** Atlas says the board has new events. */
  onHistory(key: DeviceKey) {
    if (this.watched.has(keyString(key))) void this.loadHistory(key);
  }

  /** A fleet-history line about a watched device: its history changed too. */
  onActivity(key: DeviceKey | null) {
    if (key) this.onHistory(key);
  }
}

export const deviceStatus = new StatusStore();
