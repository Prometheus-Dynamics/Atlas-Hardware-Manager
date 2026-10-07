// Device self-tests: the last result per physical board, loaded when a
// device panel asks and kept current by `self-test` events (a run by hand,
// or the one Atlas starts when a board comes back from a flash or update).

import { SvelteMap, SvelteSet } from "svelte/reactivity";
import { api, keyString, type DeviceRecord, type SelfTestRecord } from "#lib/api/client.ts";

/** Atlas's board serial for a device: the same in USB boot and running. */
export function boardOf(record: DeviceRecord): string {
  const reported = record.identity.attributes?.board_serial;
  if (reported) return reported;
  const serial = record.key.serial.replace(/[\s\0]+$/, "");
  return /^[0-9a-f]{8,}$/i.test(serial) ? serial.slice(-8).toLowerCase() : serial.toLowerCase();
}

export function canSelftest(record: DeviceRecord | undefined): boolean {
  return !!record && record.presence === "online" && record.capabilities.includes("self-test");
}

class SelfTestStore {
  /** The last run per board serial. */
  byBoard = new SvelteMap<string, SelfTestRecord>();
  /** Device keys with a run in progress from this window. */
  running = new SvelteSet<string>();
  private loaded = new Set<string>();

  forDevice(record: DeviceRecord): SelfTestRecord | undefined {
    return this.byBoard.get(boardOf(record));
  }

  /** Fetches the kept result once per device. */
  async load(record: DeviceRecord) {
    const id = keyString(record.key);
    if (this.loaded.has(id)) return;
    this.loaded.add(id);
    try {
      const run = await api.deviceSelftest(record.key);
      if (run) this.apply(run);
    } catch {
      // A missing result just means none is shown.
      this.loaded.delete(id);
    }
  }

  apply(run: SelfTestRecord) {
    const known = this.byBoard.get(run.board_serial);
    if (!known || known.at_ms <= run.at_ms) this.byBoard.set(run.board_serial, run);
  }

  /** Runs it now; rejects with a sentence when it can't start. */
  async run(record: DeviceRecord): Promise<SelfTestRecord> {
    const id = keyString(record.key);
    this.running.add(id);
    try {
      const run = await api.runSelftest(record.key);
      this.apply(run);
      return run;
    } finally {
      this.running.delete(id);
    }
  }
}

export const selftests = new SelfTestStore();
