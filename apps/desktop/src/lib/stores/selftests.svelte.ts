// Device self-tests: the last result per physical board, loaded when a
// device panel asks and kept current by `self-test` events (a run by hand,
// or the one Atlas starts when a board comes back from a flash or update).

import { SvelteMap, SvelteSet } from "svelte/reactivity";
import { api, keyString, type CheckStatus, type DeviceKey, type DeviceRecord, type SelfTestRecord, type SelfTestStep } from "#lib/api/client.ts";

/** A check of a run in progress: waiting, running, or how it ended. */
export type LiveCheck = { id: string; state: "waiting" | "running" | CheckStatus; message: string };

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
  /** The checks of a run in progress, by device key, as the board tells them. */
  live = new SvelteMap<string, LiveCheck[]>();
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

  /** One progress step of a running test. */
  progress(key: DeviceKey, step: SelfTestStep) {
    const id = keyString(key);
    if (step.step === "planned") {
      this.live.set(id, step.checks.map((check) => ({ id: check, state: "waiting", message: "" })));
      return;
    }
    const checks = this.live.get(id) ?? [];
    const at = checks.findIndex((c) => c.id === step.check);
    const next: LiveCheck =
      step.step === "started" ? { id: step.check, state: "running", message: "" } : { id: step.check, state: step.status, message: step.message };
    this.live.set(id, at >= 0 ? checks.with(at, next) : [...checks, next]);
  }

  apply(run: SelfTestRecord) {
    this.live.delete(keyString(run.device));
    const known = this.byBoard.get(run.board_serial);
    if (!known || known.at_ms <= run.at_ms) this.byBoard.set(run.board_serial, run);
  }

  /** Runs it now; rejects with a sentence when it can't start. */
  async run(record: DeviceRecord): Promise<SelfTestRecord> {
    const id = keyString(record.key);
    this.running.add(id);
    this.live.delete(id);
    try {
      const run = await api.runSelftest(record.key);
      this.apply(run);
      return run;
    } finally {
      this.running.delete(id);
      this.live.delete(id);
    }
  }
}

export const selftests = new SelfTestStore();
