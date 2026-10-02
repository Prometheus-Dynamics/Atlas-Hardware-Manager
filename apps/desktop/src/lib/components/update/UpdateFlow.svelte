<script lang="ts">
  // The one update flow, shared by the inventory selection, the device
  // panel, and robot "Make ready": choose releases, preview the plan, start.
  import {
    api,
    errorText,
    keyString,
    type JobPlan,
    type ReleaseChoice,
    type StagedRollout,
    type UpdateRequestInput,
  } from "$lib/api/client";
  import { primaryVersion, sentence } from "$lib/format";
  import { devices } from "$lib/stores/devices.svelte";
  import { releases } from "$lib/stores/releases.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import PlanTable from "./PlanTable.svelte";
  import ReleasePicker from "./ReleasePicker.svelte";

  let { request, onstarted }: { request: UpdateRequestInput; onstarted?: (job: number) => void } = $props();

  // The request is the starting point; edits here stay local to the flow.
  // svelte-ignore state_referenced_locally
  const initial = request;
  const keys = initial.devices;
  const records = $derived(keys.map((k) => devices.get(k)));
  const families = [...new Set(keys.map((k) => k.family))].sort();
  const recover = $derived(records.length > 0 && records.every((r) => r?.identity.mode === "recovery"));
  const verb = $derived(recover ? "Recover" : "Update");

  function defaultChoice(family: string): ReleaseChoice {
    const given = initial.releases[family];
    if (given) {
      const match = releases.forFamily(family).find((e) => e.id === given.release_id || (!given.release_id && e.version === given.version));
      return match ? { version: match.version, release_id: match.id } : { version: given.version, release_id: null };
    }
    const entries = releases.forFamily(family);
    const preferred = entries.find((e) => e.signed && e.channel === "stable") ?? entries[0];
    return preferred ? { version: preferred.version, release_id: preferred.id } : { version: "", release_id: null };
  }

  let choices = $state<Record<string, ReleaseChoice>>(Object.fromEntries(families.map((f) => [f, defaultChoice(f)])));
  let staged = $state<StagedRollout>(initial.staged);
  let plan = $state<JobPlan | null>(null);
  let planError = $state<string | null>(null);
  let planning = $state(false);
  let starting = $state(false);
  /** The last start failed a checksum; offer "download again" or "flash anyway". */
  let checksumFailed = $state(false);

  const missing = $derived(families.filter((f) => !choices[f]?.version.trim()));
  const unsigned = $derived(
    families
      .map((f) => releases.entries.find((e) => e.id === choices[f]?.release_id))
      .filter((e) => e && !e.signed),
  );

  function currentVersions(family: string): string[] {
    return records
      .filter((r) => r?.key.family === family)
      .map((r) => (r ? primaryVersion(r.identity) : null))
      .filter((v): v is string => !!v);
  }

  function buildRequest(ignoreChecksum = false): UpdateRequestInput {
    const picked: Record<string, ReleaseChoice> = {};
    for (const family of families) {
      const c = choices[family];
      picked[family] = {
        version: c.version.trim(),
        release_id: c.release_id ?? null,
        ignore_checksum: ignoreChecksum && !!c.release_id,
      };
    }
    return { devices: keys, releases: picked, staged };
  }

  // Re-plan whenever the choices change, so the preview is always current.
  let generation = 0;
  $effect(() => {
    const next = buildRequest();
    if (missing.length > 0) {
      plan = null;
      planError = null;
      return;
    }
    const mine = ++generation;
    planning = true;
    const timer = setTimeout(async () => {
      try {
        const result = await api.planUpdate(next);
        if (mine === generation) {
          plan = result;
          planError = null;
        }
      } catch (error) {
        if (mine === generation) {
          plan = null;
          planError = sentence(errorText(error));
        }
      } finally {
        if (mine === generation) planning = false;
      }
    }, 200);
    return () => clearTimeout(timer);
  });

  async function start(ignoreChecksum = false) {
    if (starting || !plan) return;
    starting = true;
    checksumFailed = false;
    try {
      const job = await api.startUpdate(buildRequest(ignoreChecksum));
      toasts.info(`Job #${job} started: ${plan.devices.length} device${plan.devices.length === 1 ? "" : "s"}.`);
      onstarted?.(job);
    } catch (error) {
      const text = errorText(error);
      checksumFailed = text.includes("SHA-256");
      planError = sentence(text);
    } finally {
      starting = false;
    }
  }

  const stagedOptions: { value: StagedRollout; label: string; hint: string }[] = [
    { value: "auto", label: "Auto", hint: "One device first when a family has three or more" },
    { value: "on", label: "On", hint: "One device per family first; the rest wait for it to verify" },
    { value: "off", label: "Off", hint: "All devices at once, within resource limits" },
  ];
</script>

<div class="flex flex-col gap-4">
  <section>
    <p class="micro-label mb-1">Devices</p>
    <ul class="flex flex-wrap gap-1">
      {#each keys as key (keyString(key))}
        {@const record = devices.get(key)}
        <li class="rounded-base border border-surface-700 px-1.5 text-[0.7rem] {record?.presence === 'online' ? 'text-surface-200' : 'text-error-300'}">
          {devices.nameOf(key)}{record?.presence === "online" ? "" : " (offline)"}
        </li>
      {/each}
    </ul>
  </section>

  <section class="flex flex-col gap-2">
    <p class="micro-label">1 · Choose {families.length === 1 ? "a release" : "releases"}</p>
    {#each families as family (family)}
      <ReleasePicker
        {family}
        count={keys.filter((k) => k.family === family).length}
        current={currentVersions(family)}
        bind:choice={choices[family]}
      />
    {/each}

    <fieldset class="mt-1">
      <legend class="micro-label mb-1">Staged rollout</legend>
      <div class="inline-flex overflow-hidden rounded-base border border-surface-700" role="radiogroup">
        {#each stagedOptions as option (option.value)}
          <button
            type="button"
            role="radio"
            aria-checked={staged === option.value}
            title={option.hint}
            class="px-3 py-1 text-[0.7rem] uppercase tracking-[0.12em] {staged === option.value ? 'bg-primary-500/30 text-surface-50' : 'text-surface-400 hover:bg-surface-800'}"
            onclick={() => (staged = option.value)}
          >
            {option.label}
          </button>
        {/each}
      </div>
      <p class="mt-1 text-[0.65rem] text-surface-500">{stagedOptions.find((o) => o.value === staged)?.hint}.</p>
    </fieldset>
  </section>

  <section class="flex flex-col gap-2">
    <p class="micro-label flex items-center gap-2">
      2 · Review the plan
      {#if planning}<i class="fa-solid fa-circle-notch fa-spin text-surface-500" aria-hidden="true"></i>{/if}
    </p>
    {#if missing.length > 0}
      <p class="text-xs text-surface-400">Choose a version for {missing.join(" and ")} to see the plan.</p>
    {:else if planError}
      <p class="rounded-base border border-error-500/50 bg-error-500/10 px-3 py-2 text-xs text-error-200" role="alert">
        <i class="fa-solid fa-circle-exclamation mr-1" aria-hidden="true"></i>{planError}
      </p>
      {#if checksumFailed}
        <div class="flex gap-2">
          <button type="button" class="btn btn-sm preset-tonal" disabled={starting} onclick={() => start(false)}>
            <i class="fa-solid fa-rotate" aria-hidden="true"></i>Download again
          </button>
          <button type="button" class="btn btn-sm preset-tonal-warning" disabled={starting} onclick={() => start(true)}>
            <i class="fa-solid fa-triangle-exclamation" aria-hidden="true"></i>Flash anyway
          </button>
        </div>
      {/if}
    {:else if plan}
      <PlanTable {plan} />
    {/if}
    {#if unsigned.length > 0}
      <p class="rounded-base border border-warning-600/50 bg-warning-500/10 px-3 py-2 text-xs text-warning-200">
        <i class="fa-solid fa-triangle-exclamation mr-1" aria-hidden="true"></i>
        Installing an unsigned release. Atlas cannot verify where it came from, but it will not stop you.
      </p>
    {/if}
  </section>

  <section class="flex items-center gap-2 border-t border-surface-800 pt-3">
    <p class="micro-label mr-auto">3 · Confirm</p>
    <button
      type="button"
      class="btn btn-sm preset-filled-primary-500 uppercase tracking-[0.12em]"
      disabled={!plan || planning || starting}
      onclick={() => start(false)}
    >
      {#if starting}<i class="fa-solid fa-circle-notch fa-spin" aria-hidden="true"></i>{/if}
      {verb} {plan ? plan.devices.length : keys.length} device{(plan ? plan.devices.length : keys.length) === 1 ? "" : "s"}
    </button>
  </section>
</div>
