<script lang="ts">
  import EmptyState from "$lib/components/common/EmptyState.svelte";
  import PageHeader from "$lib/components/common/PageHeader.svelte";
  import JobDetail from "$lib/components/jobs/JobDetail.svelte";
  import JobList from "$lib/components/jobs/JobList.svelte";
  import { jobs } from "$lib/stores/jobs.svelte";
  import { ui } from "$lib/stores/ui.svelte";
  import { softFade } from "$lib/ui/motion";

  const selected = $derived((ui.selectedJob !== null ? jobs.get(ui.selectedJob) : undefined) ?? jobs.sorted[0]);
</script>

<svelte:head><title>Jobs · Atlas</title></svelte:head>

<div class="reveal flex flex-col gap-5">
  <PageHeader
    title="Jobs"
    subtitle={jobs.running.length > 0 ? `${jobs.running.length} running now` : "Every update and flash from this session, newest first."}
  />
  {#if jobs.sorted.length === 0}
    <EmptyState icon="activity" title="Nothing running">
      Select devices and press Update, flash a board waiting in USB boot, or use Make ready on a robot.
    </EmptyState>
  {:else}
    <div class="grid grid-cols-[17rem_1fr] items-start gap-5">
      <JobList selectedId={selected?.id ?? null} />
      {#if selected}
        {#key selected.id}
          <div in:softFade><JobDetail job={selected} /></div>
        {/key}
      {/if}
    </div>
  {/if}
</div>
