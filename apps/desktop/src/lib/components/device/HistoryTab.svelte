<script lang="ts">
  import { goto } from "$app/navigation";
  import { keyString, sameKey, type DeviceRecord, type HistoryEntry } from "#lib/api/client.ts";
  import Icon from "#lib/components/common/Icon.svelte";
  import Pill from "#lib/components/common/Pill.svelte";
  import ProgressBar from "#lib/components/common/ProgressBar.svelte";
  import { clockTime, jobStatusDetail, jobStatusLabel, jobStatusTone, overallFraction, sentence } from "#lib/format.ts";
  import { activity } from "#lib/stores/activity.svelte.ts";
  import { jobs } from "#lib/stores/jobs.svelte.ts";
  import { deviceStatus } from "#lib/stores/status.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import { rise } from "#lib/ui/motion.ts";
  import DeviceTimeline from "./DeviceTimeline.svelte";

  let { record }: { record: DeviceRecord } = $props();

  const entries = $derived(
    jobs.sorted.flatMap((job) => job.devices.filter((d) => sameKey(d.device, record.key)).map((state) => ({ job, state }))),
  );

  // The merged history (Atlas's record and the board's event log); until it
  // loads, Atlas's own entries for this device.
  $effect(() => deviceStatus.watch(record));
  const merged = $derived(deviceStatus.history.get(keyString(record.key)));
  const timeline = $derived<HistoryEntry[]>(
    merged ??
      activity.entries
        .filter((e) => sameKey(e.device, record.key))
        .map((e) => ({
          at_ms: e.at_ms,
          level: e.level,
          kind: e.kind,
          source: "atlas",
          origin: "atlas",
          message: e.message,
          boot_id: null,
          data: {},
        })),
  );

  function open(id: number) {
    ui.selectedJob = id;
    ui.close();
    void goto("/jobs");
  }
</script>

{#if entries.length === 0 && timeline.length === 0}
  <div class="flex flex-col items-center gap-2 py-8 text-center">
    <Icon name="history" size={22} class="text-fg-faint" />
    <p class="text-[13px] text-fg-muted">Nothing has happened to this device yet.</p>
  </div>
{:else if entries.length > 0}
  <ul class="flex flex-col gap-2">
    {#each entries as { job, state } (job.id)}
      {@const detail = jobStatusDetail(state.status)}
      <li in:rise>
        <button type="button" class="glass row w-full px-4 py-3 text-left" onclick={() => open(job.id)}>
          <div class="flex items-center justify-between gap-2">
            <span class="text-[13px] font-medium text-fg">Job #{job.id} · <span class="mono">{state.release.version}</span></span>
            <Pill tone={jobStatusTone(state.status)} label={jobStatusLabel(state.status)} />
          </div>
          <p class="mt-0.5 text-[12px] text-fg-faint">{clockTime(job.created_ms)}</p>
          {#if state.status.status === "running"}
            <div class="mt-2"><ProgressBar value={overallFraction(state)} label="Progress" /></div>
          {/if}
          {#if detail}<p class="mt-1.5 text-[12.5px] text-fg-muted">{sentence(detail)}</p>{/if}
        </button>
      </li>
    {/each}
  </ul>
{/if}

{#if timeline.length > 0}
  <section class="mt-6 flex flex-col gap-2">
    <h3 class="text-[12px] font-medium uppercase tracking-[0.06em] text-fg-faint">Timeline</h3>
    <DeviceTimeline entries={timeline} />
  </section>
{/if}

<style>
  .row {
    transition:
      background var(--t-fast),
      transform var(--t-fast);
  }
  .row:hover {
    background: var(--glass-hover);
  }
  .row:active {
    transform: scale(0.99);
  }
</style>
