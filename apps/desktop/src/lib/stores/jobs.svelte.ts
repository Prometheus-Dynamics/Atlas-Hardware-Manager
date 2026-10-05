// Job projection: full records from listJobs, patched by job-* events.

import {
  api,
  errorText,
  keyString,
  sameKey,
  type AtlasEvent,
  type DeviceJobState,
  type DeviceKey,
  type JobId,
  type JobRecord,
} from "$lib/api/client";
import { toasts } from "./toasts.svelte";
import { isRecoveryPlan } from "$lib/present";

const LOG_LIMIT = 500;

class JobStore {
  list = $state<JobRecord[]>([]);
  loaded = $state(false);

  sorted = $derived([...this.list].sort((a, b) => b.id - a.id));
  running = $derived(this.list.filter((j) => j.state === "running"));

  /** Latest per-device job state, for live progress in the inventory. */
  active = $derived.by(() => {
    const map = new Map<string, { job: JobId; state: DeviceJobState }>();
    for (const job of this.running) {
      for (const state of job.devices) {
        const s = state.status.status;
        if (s === "running" || s === "queued") map.set(keyString(state.device), { job: job.id, state });
      }
    }
    return map;
  });

  get(id: JobId): JobRecord | undefined {
    return this.list.find((j) => j.id === id);
  }

  async load() {
    try {
      this.list = await api.listJobs();
      this.loaded = true;
    } catch (error) {
      toasts.error(`Could not load jobs: ${errorText(error)}`);
    }
  }

  private device(job: JobId, key: DeviceKey): DeviceJobState | undefined {
    return this.get(job)?.devices.find((d) => sameKey(d.device, key));
  }

  apply(event: AtlasEvent) {
    switch (event.type) {
      case "job-started": {
        if (!this.get(event.job)) {
          // Placeholder until getJob fills in names, plans, and releases.
          this.list.push({
            id: event.job,
            state: "running",
            created_ms: Date.now(),
            finished_ms: null,
            summary: null,
            devices: [],
          });
        }
        void this.refresh(event.job);
        break;
      }
      case "job-device": {
        const state = this.device(event.job, event.device);
        if (!state) return void this.refresh(event.job);
        state.status = event.status;
        const s = event.status.status;
        if (s === "running") state.started_ms ??= Date.now();
        else if (s !== "queued") state.finished_ms ??= Date.now();
        break;
      }
      case "job-step": {
        const state = this.device(event.job, event.device);
        if (!state) return;
        state.step = event.step;
        state.fraction = 0;
        break;
      }
      case "job-progress": {
        const state = this.device(event.job, event.device);
        if (!state) return;
        state.step = event.step;
        state.fraction = event.fraction;
        break;
      }
      case "job-log": {
        const state = this.device(event.job, event.device);
        if (!state) return;
        state.log.push(event.message);
        if (state.log.length > LOG_LIMIT) state.log.splice(0, state.log.length - LOG_LIMIT);
        break;
      }
      case "job-finished": {
        const job = this.get(event.job);
        if (job) {
          job.summary = event.summary;
          job.state = event.state;
          job.finished_ms = Date.now();
        }
        void this.refresh(event.job);
        const s = event.summary;
        const bad = s.failed + s.needs_recovery + s.rolled_back;
        const text = `Job #${event.job} finished: ${s.verified} verified${bad ? `, ${bad} with problems` : ""}${s.skipped ? `, ${s.skipped} skipped` : ""}.`;
        if (bad) toasts.warning(text);
        else toasts.success(text);
        // A flashed board only starts its new image after a power-cycle.
        const flashed = job?.devices.filter((d) => isRecoveryPlan(d.plan) && d.status.status === "verified") ?? [];
        if (flashed.length > 0) {
          const who = flashed.length === 1 ? flashed[0].name : `${flashed.length} boards`;
          toasts.push("info", `Now power-cycle ${who}: unplug its power and plug it back in, without holding the boot button.`, 15000);
        }
        break;
      }
    }
  }

  /** Replaces one job with the backend's full record. */
  private async refresh(id: JobId) {
    try {
      const fresh = await api.getJob(id);
      if (!fresh) return;
      const index = this.list.findIndex((j) => j.id === id);
      if (index >= 0) this.list[index] = fresh;
      else this.list.push(fresh);
    } catch {
      // The next event or resync corrects the view.
    }
  }

  async cancel(id: JobId) {
    try {
      await api.cancelJob(id);
      toasts.info(`Cancelling job #${id}; devices already writing will finish.`);
    } catch (error) {
      toasts.error(errorText(error));
    }
  }
}

export const jobs = new JobStore();
