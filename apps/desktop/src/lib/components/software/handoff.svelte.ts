// Fresh install of a running board: restart it into USB boot (or wait for
// the user to), watch for the same board to show up in USB boot, then flash
// it with the chosen image. Lives outside the component so the wait
// survives the running board going offline and the panel closing.

import { goto } from "$app/navigation";
import { api, errorText, keyString, type DeviceRecord, type ReleaseChoice, type StagedRollout } from "#lib/api/client.ts";
import { deviceName, sentence } from "#lib/format.ts";
import { isRecovery } from "#lib/present.ts";
import { devices } from "#lib/stores/devices.svelte.ts";
import { toasts } from "#lib/stores/toasts.svelte.ts";
import { ui } from "#lib/stores/ui.svelte.ts";
import { SvelteMap } from "svelte/reactivity";
import { boardSerial, USB_BOOT_ACTION } from "./software";

/** After this long without the board in USB boot, show the manual steps. */
export const HANDOFF_TIMEOUT_MS = 90_000;
const POLL_MS = 500;

export class Handoff {
  readonly key: string;
  readonly name: string;
  readonly model: string;
  readonly board: string | null;
  readonly choice: ReleaseChoice;
  readonly staged: StagedRollout;
  /** Atlas restarted the board itself; otherwise the user does it by hand. */
  readonly restarted: boolean;
  readonly startedMs = Date.now();
  /** Boards already in USB boot, so a board without a serial matches only a new one. */
  readonly known: Set<string>;
  phase = $state<"waiting" | "starting" | "failed">("waiting");
  error = $state<string | null>(null);
  /** The board in USB boot, once found. */
  found = $state<string | null>(null);

  constructor(record: DeviceRecord, choice: ReleaseChoice, staged: StagedRollout, restarted: boolean) {
    this.key = keyString(record.key);
    this.name = deviceName(record);
    this.model = record.identity.model.split(" ")[0];
    this.board = boardSerial(record);
    this.choice = { ...choice };
    this.staged = staged;
    this.restarted = restarted;
    this.known = new Set(devices.all.filter((r) => isRecovery(r) && r.presence === "online").map((r) => keyString(r.key)));
  }

  matches(record: DeviceRecord): boolean {
    if (!isRecovery(record) || record.presence !== "online") return false;
    if (this.board) return boardSerial(record) === this.board;
    return !this.known.has(keyString(record.key)) && record.identity.model.split(" ")[0] === this.model;
  }
}

class Handoffs {
  map = new SvelteMap<string, Handoff>();
  private timer: ReturnType<typeof setInterval> | null = null;

  get(key: string): Handoff | undefined {
    return this.map.get(key);
  }

  /** Restarts the board into USB boot (when `restart`) and starts watching. */
  async begin(record: DeviceRecord, choice: ReleaseChoice, staged: StagedRollout, restart: boolean) {
    const handoff = new Handoff(record, choice, staged, restart);
    if (restart) await api.runDeviceAction(record.key, USB_BOOT_ACTION);
    this.map.set(handoff.key, handoff);
    this.timer ??= setInterval(() => this.check(), POLL_MS);
  }

  cancel(key: string) {
    this.map.delete(key);
    this.idle();
  }

  private idle() {
    if (this.map.size === 0 && this.timer) {
      clearInterval(this.timer);
      this.timer = null;
    }
  }

  private check() {
    for (const handoff of this.map.values()) {
      if (handoff.phase !== "waiting") continue;
      const target = devices.all.find((r) => handoff.matches(r));
      if (target) void this.flash(handoff, target);
    }
  }

  private async flash(handoff: Handoff, target: DeviceRecord) {
    handoff.phase = "starting";
    handoff.found = keyString(target.key);
    try {
      const job = await api.startUpdate({
        devices: [target.key],
        releases: { [target.key.family]: { ...handoff.choice, ignore_checksum: false } },
        staged: handoff.staged,
      });
      toasts.info(`Job #${job} started: installing on ${handoff.name}.`);
      this.cancel(handoff.key);
      ui.selectedJob = job;
      ui.close();
      void goto("/jobs");
    } catch (error) {
      handoff.phase = "failed";
      handoff.error = sentence(errorText(error));
    }
  }
}

export const handoffs = new Handoffs();
