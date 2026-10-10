<script lang="ts">
  import { goto } from "$app/navigation";
  import { keyString } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import GlassCard from "#lib/components/common/GlassCard.svelte";
  import ActivityFeed from "#lib/components/overview/ActivityFeed.svelte";
  import { activity } from "#lib/stores/activity.svelte.ts";
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

  // Before this session's jobs: the fleet's updates and flashes so far.
  const earlier = $derived(activity.entries.filter((e) => e.kind === "version-changed" || e.kind === "update-result" || e.kind === "mode-changed"));

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
    <!-- No jobs yet: what would start one on the left, the fleet's earlier
         updates and flashes filling the right (the same split as with jobs). -->
    <div class="split">
      <section class="glass flex flex-col gap-3 self-start px-4 py-4" style="border-radius: var(--r-panel)" aria-label="No jobs yet">
        <h2 class="text-[13px] font-semibold text-fg">No jobs this session</h2>
        <p class="text-[12.5px] text-fg-muted">
          A job is one update or flash: the image is downloaded, written, and checked once the device comes back on the new
          version. Each shows here with its stages and log while it runs.
        </p>
        <div class="flex flex-col gap-1.5">
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
      </section>
      <GlassCard large title="Earlier updates and flashes" icon="history" pad={false} class="self-start">
        <div class="flex flex-col px-3 pb-3 pt-2">
          <ActivityFeed entries={earlier} limit={80} empty="No device has been updated or flashed yet." />
        </div>
      </GlassCard>
    </div>
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
