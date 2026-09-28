<script lang="ts">
  import { goto } from "$app/navigation";
  import { sameKey, type DeviceRecord } from "$lib/api/client";
  import ProgressBar from "$lib/components/common/ProgressBar.svelte";
  import Tag from "$lib/components/common/Tag.svelte";
  import { clockTime, jobStatusDetail, jobStatusLabel, jobStatusTone, overallFraction } from "$lib/format";
  import { jobs } from "$lib/stores/jobs.svelte";
  import { ui } from "$lib/stores/ui.svelte";

  let { record }: { record: DeviceRecord } = $props();

  const entries = $derived(
    jobs.sorted.flatMap((job) =>
      job.devices.filter((d) => sameKey(d.device, record.key)).map((state) => ({ job, state })),
    ),
  );

  function open(id: number) {
    ui.selectedJob = id;
    void goto("/jobs");
  }
</script>

{#if entries.length === 0}
  <p class="text-xs text-surface-400">No updates recorded for this device in this session.</p>
{:else}
  <ul class="flex flex-col gap-2">
    {#each entries as { job, state } (job.id)}
      {@const detail = jobStatusDetail(state.status)}
      <li>
        <button type="button" class="w-full rounded-base border border-surface-800 px-3 py-2 text-left hover:border-surface-600" onclick={() => open(job.id)}>
          <div class="flex items-center justify-between gap-2 text-xs">
            <span class="text-surface-100">Job #{job.id} · {state.release.version}</span>
            <Tag tone={jobStatusTone(state.status)} label={jobStatusLabel(state.status)} />
          </div>
          <p class="mt-0.5 text-[0.65rem] text-surface-500">{clockTime(job.created_ms)}</p>
          {#if state.status.status === "running"}
            <div class="mt-1"><ProgressBar value={overallFraction(state)} label="Progress" /></div>
          {/if}
          {#if detail}<p class="mt-1 text-[0.7rem] text-surface-300">{detail}</p>{/if}
        </button>
      </li>
    {/each}
  </ul>
{/if}
