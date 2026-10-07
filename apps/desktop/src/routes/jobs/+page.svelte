<script lang="ts">
  import { goto } from "$app/navigation";
  import { keyString } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import EmptyState from "#lib/components/common/EmptyState.svelte";
  import PageHeader from "#lib/components/common/PageHeader.svelte";
  import JobDetail from "#lib/components/jobs/JobDetail.svelte";
  import JobList from "#lib/components/jobs/JobList.svelte";
  import Page from "#lib/components/layout/Page.svelte";
  import Region from "#lib/components/layout/Region.svelte";
  import { insights } from "#lib/stores/insights.svelte.ts";
  import { jobs } from "#lib/stores/jobs.svelte.ts";
  import { robots } from "#lib/stores/robots.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import { softFade } from "#lib/ui/motion.ts";

  const selected = $derived((ui.selectedJob !== null ? jobs.get(ui.selectedJob) : undefined) ?? jobs.sorted[0]);

  // What could start a job right now, for the empty state.
  const waiting = $derived(insights.waiting);
  const outdated = $derived(insights.outdated.length);

  function updateAll() {
    const request = insights.updateAllRequest();
    if (request) ui.openUpdate(request, "Update all");
  }
</script>

<svelte:head><title>Jobs · Atlas</title></svelte:head>

<Page>
  {#snippet header()}
    <PageHeader
      title="Jobs"
      subtitle={jobs.running.length > 0 ? `${jobs.running.length} running now` : "Every update and flash from this session, newest first."}
    />
  {/snippet}

  {#if jobs.sorted.length === 0}
    <EmptyState icon="activity" title="No jobs yet" class="flex-1 justify-center">
      <p>
        A job is one update or flash: Atlas downloads the image, writes it, and checks the device comes back on the new
        version. Each one shows up here with its stages and log while it runs, and stays for this session.
      </p>
      <div class="mt-5 flex flex-wrap justify-center gap-2">
        {#if waiting.length > 0}
          <Button variant="primary" icon="bolt" onclick={() => ui.openDevice(keyString(waiting[0].key), "software")}>
            Flash {waiting.length === 1 ? "the board" : `${waiting.length} boards`} in USB boot
          </Button>
        {/if}
        {#if outdated > 0}
          <Button variant={waiting.length > 0 ? "glass" : "primary"} icon="arrow-up" onclick={updateAll}>
            Update {outdated} device{outdated === 1 ? "" : "s"}
          </Button>
        {/if}
        <Button icon="layout-grid" onclick={() => goto("/devices")}>Choose devices</Button>
        {#if robots.profiles.length > 0}
          <Button icon="robot" onclick={() => goto("/robots")}>Make a robot ready</Button>
        {/if}
      </div>
    </EmptyState>
  {:else}
    <!-- The list on the left scrolls; the chosen job fills the right. -->
    <div class="split">
      <section class="glass flex min-h-0 flex-col" style="border-radius: var(--r-panel)" aria-label="All jobs">
        <h2 class="shrink-0 border-b border-hairline px-4 py-3 text-[12px] font-medium text-fg-faint">
          {jobs.sorted.length} job{jobs.sorted.length === 1 ? "" : "s"} this session
        </h2>
        <Region inner="p-1.5"><JobList selectedId={selected?.id ?? null} /></Region>
      </section>
      {#if selected}
        {#key selected.id}
          <div class="min-h-0 min-w-0" in:softFade><JobDetail job={selected} /></div>
        {/key}
      {/if}
    </div>
  {/if}
</Page>

<style>
  .split {
    flex: 1 1 0;
    min-height: 0;
    display: grid;
    grid-template-columns: clamp(16rem, 24%, 24rem) minmax(0, 1fr);
    gap: 12px;
  }
  @container page (max-width: 620px) {
    .split {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: minmax(0, 0.6fr) minmax(0, 1.4fr);
    }
  }
</style>
