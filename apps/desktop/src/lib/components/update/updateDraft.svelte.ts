// The state behind every update or flash: release choices, the live plan
// preview, and starting the job. Shared by UpdateFlow and the Software tab.
// Create it during component initialisation: it re-plans in an $effect.

import {
  api,
  errorText,
  type DeviceKey,
  type JobId,
  type JobPlan,
  type ReleaseChoice,
  type StagedRollout,
  type UpdateRequestInput,
} from "#lib/api/client.ts";
import { sentence } from "#lib/format.ts";
import { releases } from "#lib/stores/releases.svelte.ts";
import { toasts } from "#lib/stores/toasts.svelte.ts";

export class UpdateDraft {
  readonly keys: DeviceKey[];
  readonly families: string[];
  choices = $state<Record<string, ReleaseChoice>>({});
  staged = $state<StagedRollout>("auto");
  plan = $state<JobPlan | null>(null);
  planError = $state<string | null>(null);
  planning = $state(false);
  starting = $state(false);
  /** The last start failed a checksum; offer "download again" or "flash anyway". */
  checksumFailed = $state(false);

  missing = $derived(this.familiesMissing());
  unsigned = $derived(this.unsignedEntries());

  private generation = 0;

  constructor(request: UpdateRequestInput) {
    this.keys = request.devices;
    this.families = [...new Set(this.keys.map((k) => k.family))].sort();
    this.staged = request.staged;
    this.choices = Object.fromEntries(this.families.map((f) => [f, this.defaultChoice(f, request)]));

    // Re-plan whenever the choices change, so the preview is always current.
    $effect(() => {
      const next = this.buildRequest();
      if (this.missing.length > 0) {
        this.plan = null;
        this.planError = null;
        return;
      }
      const mine = ++this.generation;
      this.planning = true;
      const timer = setTimeout(async () => {
        try {
          const result = await api.planUpdate(next);
          if (mine === this.generation) {
            this.plan = result;
            this.planError = null;
          }
        } catch (error) {
          if (mine === this.generation) {
            this.plan = null;
            this.planError = sentence(errorText(error));
          }
        } finally {
          if (mine === this.generation) this.planning = false;
        }
      }, 200);
      return () => clearTimeout(timer);
    });
  }

  private unsignedEntries() {
    return this.families
      .map((f) => releases.entries.find((e) => e.id === this.choices[f]?.release_id))
      .filter((e) => e && !e.signed);
  }

  private familiesMissing(): string[] {
    return this.families.filter((f) => !this.choices[f]?.version.trim());
  }

  private defaultChoice(family: string, request: UpdateRequestInput): ReleaseChoice {
    const entries = releases.forFamily(family);
    const given = request.releases[family];
    if (given) {
      const match = entries.find((e) => e.id === given.release_id || (!given.release_id && e.version === given.version));
      return match ? { version: match.version, release_id: match.id } : { version: given.version, release_id: null };
    }
    const preferred = entries.find((e) => e.signed && e.channel === "stable") ?? entries[0];
    return preferred ? { version: preferred.version, release_id: preferred.id } : { version: "", release_id: null };
  }

  /** Picks a catalog entry for a family. */
  choose(family: string, releaseId: string) {
    const entry = releases.entries.find((e) => e.id === releaseId);
    if (entry) this.choices[family] = { version: entry.version, release_id: entry.id };
  }

  /** Uses a file once, without adding it to the release list. */
  chooseFile(family: string, path: string, version: string) {
    this.choices[family] = { version, release_id: null, path };
  }

  buildRequest(ignoreChecksum = false): UpdateRequestInput {
    const picked: Record<string, ReleaseChoice> = {};
    for (const family of this.families) {
      const c = this.choices[family];
      picked[family] = {
        version: c.version.trim(),
        release_id: c.release_id ?? null,
        ignore_checksum: ignoreChecksum && !!c.release_id,
        path: c.path ?? null,
      };
    }
    return { devices: this.keys, releases: picked, staged: this.staged };
  }

  /** Starts the job; resolves with its id, or null when it did not start. */
  async start(ignoreChecksum = false): Promise<JobId | null> {
    if (this.starting || !this.plan) return null;
    this.starting = true;
    this.checksumFailed = false;
    try {
      const job = await api.startUpdate(this.buildRequest(ignoreChecksum));
      const n = this.plan.devices.length;
      toasts.info(`Job #${job} started: ${n} device${n === 1 ? "" : "s"}.`);
      return job;
    } catch (error) {
      const text = errorText(error);
      this.checksumFailed = text.includes("SHA-256");
      this.planError = sentence(text);
      return null;
    } finally {
      this.starting = false;
    }
  }
}
