<script lang="ts">
  import EmptyState from "$lib/components/common/EmptyState.svelte";
  import PageHeader from "$lib/components/common/PageHeader.svelte";
  import JobDetail from "$lib/components/jobs/JobDetail.svelte";
  import JobList from "$lib/components/jobs/JobList.svelte";
  import { jobs } from "$lib/stores/jobs.svelte";
  import { ui } from "$lib/stores/ui.svelte";

  const selected = $derived(
    (ui.selectedJob !== null ? jobs.get(ui.selectedJob) : undefined) ?? jobs.sorted[0],
  );
</script>

<svelte:head><title>Jobs · Atlas</title></svelte:head>

<div class="flex flex-col gap-3">
  <PageHeader eyebrow="Jobs" title="Updates and recoveries" subtitle="Every job started in this session, newest first." />
  {#if jobs.sorted.length === 0}
    <EmptyState icon="fa-list-check" title="No jobs yet">
      Select devices on the Inventory page and press Update, or use Make ready on a robot.
    </EmptyState>
  {:else}
    <div class="grid grid-cols-[16rem_1fr] items-start gap-3">
      <JobList selectedId={selected?.id ?? null} />
      {#if selected}<JobDetail job={selected} />{/if}
    </div>
  {/if}
</div>
