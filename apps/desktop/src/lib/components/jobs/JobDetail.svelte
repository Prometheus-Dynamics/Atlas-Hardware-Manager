<script lang="ts">
  import { keyString, type JobRecord } from "$lib/api/client";
  import AsyncButton from "$lib/components/common/AsyncButton.svelte";
  import Panel from "$lib/components/common/Panel.svelte";
  import { clockTime, duration } from "$lib/format";
  import { clock } from "$lib/stores/clock.svelte";
  import { jobs } from "$lib/stores/jobs.svelte";
  import JobDeviceRow from "./JobDeviceRow.svelte";

  let { job }: { job: JobRecord } = $props();

  const total = $derived((job.finished_ms ?? clock.now) - job.created_ms);
</script>

<Panel eyebrow="Job #{job.id}" title="{job.devices.length} device{job.devices.length === 1 ? '' : 's'} · {job.state}" subtitle="Started {clockTime(job.created_ms)} · {duration(total)}{job.finished_ms ? '' : ' so far'}">
  {#snippet actions()}
    {#if job.state === "running"}
      <AsyncButton class="btn btn-sm preset-outlined-error-500" action={() => jobs.cancel(job.id)}>
        <i class="fa-solid fa-ban" aria-hidden="true"></i>Cancel job
      </AsyncButton>
    {/if}
  {/snippet}
  {#if job.devices.length === 0}
    <p class="text-xs text-surface-400"><i class="fa-solid fa-circle-notch fa-spin mr-1" aria-hidden="true"></i>Loading job…</p>
  {:else}
    <ul class="flex flex-col gap-2">
      {#each job.devices as state (keyString(state.device))}
        <JobDeviceRow job={state} />
      {/each}
    </ul>
  {/if}
</Panel>
