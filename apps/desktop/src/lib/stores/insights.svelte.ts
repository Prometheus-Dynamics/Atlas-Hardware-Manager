// Facts derived across stores for the inventory: what is outdated, what
// failed, and what is waiting for the user. Read-only; no backend calls.

import { keyString, type DeviceJobState, type DeviceRecord, type JobId, type ReleaseChoice, type UpdateRequestInput } from "#lib/api/client.ts";
import { compareVersionsDesc, primaryVersion } from "#lib/format.ts";
import { isRecovery } from "#lib/present.ts";
import { devices } from "./devices.svelte";
import { jobs } from "./jobs.svelte";
import { releases } from "./releases.svelte";
import { robots } from "./robots.svelte";
import { system } from "./system.svelte";
import { canUpdate } from "./ui.svelte";

const BAD = new Set(["failed", "rolled-back", "needs-recovery"]);

export interface Outcome {
  job: JobId;
  state: DeviceJobState;
}

class Insights {
  /** The most recent finished job state per device. */
  lastOutcome = $derived.by(() => {
    const map = new Map<string, Outcome>();
    for (const job of jobs.sorted) {
      for (const state of job.devices) {
        const id = keyString(state.device);
        const s = state.status.status;
        if (map.has(id) || s === "queued" || s === "running") continue;
        map.set(id, { job: job.id, state });
      }
    }
    return map;
  });

  /** Devices whose last job failed, rolled back, or left them needing recovery. */
  failed = $derived(
    devices.all.filter((d) => {
      const o = this.lastOutcome.get(keyString(d.key));
      return !!o && BAD.has(o.state.status.status) && !jobs.active.has(keyString(d.key));
    }),
  );

  /** Online devices in USB boot or a bootloader, waiting for an image. */
  waiting = $derived(devices.all.filter((d) => d.presence === "online" && isRecovery(d)));

  outdated = $derived(
    devices.all.filter((d) => {
      if (!canUpdate(d) || isRecovery(d) || jobs.active.has(keyString(d.key))) return false;
      const target = this.target(d);
      const current = primaryVersion(d.identity);
      return !!target && !!current && target !== current && (this.robotTarget(d) !== null || compareVersionsDesc(current, target) > 0);
    }),
  );

  outdatedIds = $derived(new Set(this.outdated.map((d) => keyString(d.key))));
  failedIds = $derived(new Set(this.failed.map((d) => keyString(d.key))));

  /** Host checks that Atlas can fix with one click. */
  fixable = $derived(system.health.filter((h) => h.status !== "ok" && h.fix_action));

  needYou = $derived(this.waiting.length + this.failed.length);

  robotTarget(record: DeviceRecord): string | null {
    if (!record.robot) return null;
    return robots.profile(record.robot)?.targets[record.key.family] ?? null;
  }

  /** The version a device should run: its robot's target, else the newest stable release. */
  target(record: DeviceRecord): string | null {
    const fromRobot = this.robotTarget(record);
    if (fromRobot) return fromRobot;
    const stable = releases.forFamily(record.key.family).find((e) => e.channel === "stable");
    return stable?.version ?? null;
  }

  /** One request that brings every outdated device to its target. */
  updateAllRequest(): UpdateRequestInput | null {
    const list = this.outdated;
    if (list.length === 0) return null;
    const choices: Record<string, ReleaseChoice> = {};
    for (const record of list) {
      const family = record.key.family;
      if (choices[family]) continue;
      const version = this.target(record)!;
      const entry = releases.forFamily(family).find((e) => e.version === version);
      choices[family] = { version, release_id: entry?.id ?? null };
    }
    return {
      devices: list.map((d) => d.key),
      releases: choices,
      staged: system.settings?.staged_default ?? "auto",
    };
  }
}

export const insights = new Insights();
