<script lang="ts">
  // Home: the robot (or everything) at a glance. Readiness first, then live
  // numbers, how it's all connected, and what happened lately.
  import { goto } from "$app/navigation";
  import { sameKey } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import GlassCard from "#lib/components/common/GlassCard.svelte";
  import Skeleton from "#lib/components/common/Skeleton.svelte";
  import PageHeader from "#lib/components/common/PageHeader.svelte";
  import Page from "#lib/components/layout/Page.svelte";
  import RobotMenu from "#lib/components/inventory/RobotMenu.svelte";
  import DiscoveryStatus from "#lib/components/shell/DiscoveryStatus.svelte";
  import ActivityFeed from "#lib/components/overview/ActivityFeed.svelte";
  import Readiness from "#lib/components/overview/Readiness.svelte";
  import SummaryTiles from "#lib/components/overview/SummaryTiles.svelte";
  import TopologyMap from "#lib/components/overview/TopologyMap.svelte";
  import { activity } from "#lib/stores/activity.svelte.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { robots } from "#lib/stores/robots.svelte.ts";
  import { NO_ROBOT, ui } from "#lib/stores/ui.svelte.ts";

  const robot = $derived(ui.robot !== null && ui.robot !== NO_ROBOT ? ui.robot : null);
  // A robot's devices: those assigned to it and those filling its roles.
  const roleKeys = $derived(robot ? (robots.profile(robot)?.roles.map((r) => r.device).filter((k) => !!k) ?? []) : []);
  const records = $derived(
    devices.all.filter((d) => {
      if (ui.robot === null) return true;
      if (ui.robot === NO_ROBOT) return !d.robot;
      return d.robot === ui.robot || roleKeys.some((k) => sameKey(k, d.key));
    }),
  );
  const ids = $derived(records.map((r) => r.key));
  const feed = $derived(
    ui.robot === null ? activity.entries : activity.entries.filter((e) => (robot && e.robot === robot) || ids.some((k) => sameKey(k, e.device))),
  );

  // Hold the dashboard until the first discovery pass so it lands whole,
  // instead of filling in device by device.
  let waited = $state(false);
  $effect(() => {
    const timer = setTimeout(() => (waited = true), 3000);
    return () => clearTimeout(timer);
  });
  const ready = $derived(devices.loaded && (devices.lastScanAt !== null || waited));
</script>

<svelte:head><title>Overview · Atlas</title></svelte:head>

<Page>
  {#snippet header()}
    <PageHeader>
      {#snippet heading()}<RobotMenu />{/snippet}
      {#snippet actions()}
        <DiscoveryStatus />
        <Button icon="layout-grid" onclick={() => goto("/devices")}>All devices</Button>
      {/snippet}
    </PageHeader>
  {/snippet}

  {#if !ready}
    <div class="contents" aria-busy="true" aria-label="Loading the overview">
      <div class="glass flex h-[58px] shrink-0 items-center gap-4 px-5" style="border-radius: var(--r-panel)">
        <Skeleton width="34px" height={34} round />
        <div class="flex flex-1 flex-col gap-2"><Skeleton width="30%" height={14} /><Skeleton width="45%" height={10} /></div>
      </div>
      <div class="auto-grid fit shrink-0" style="--min: 170px">
        {#each [0, 1, 2, 3, 4] as i (i)}<Skeleton height={112} />{/each}
      </div>
      <div class="split">
        <div class="shimmer"></div>
        <div class="shimmer"></div>
      </div>
    </div>
  {:else}
    <Readiness {records} {robot} />

    {#if records.length > 0}
      <SummaryTiles {records} />

      <div class="split">
        <GlassCard fill scroll={false} pad={false} large title="How it's connected" subtitle="Click a device to open it" icon="sitemap">
          <TopologyMap {records} />
        </GlassCard>
        <GlassCard fill large title="Recent activity" icon="history" pad={false}>
          <div class="flex flex-col px-3 pb-3 pt-2">
            <ActivityFeed entries={feed} limit={60} empty="Nothing has happened here yet." />
          </div>
        </GlassCard>
      </div>
    {/if}
  {/if}
</Page>

<style>
  /* Topology and activity share whatever height is left. */
  .split {
    flex: 1 1 0;
    min-height: 0;
    display: grid;
    gap: 12px;
    grid-template-columns: minmax(0, 1.7fr) minmax(300px, 1fr);
  }
  @container page (min-width: 1600px) {
    .split {
      grid-template-columns: minmax(0, 2.4fr) minmax(380px, 1fr);
    }
  }
  /* Narrow: stacked, the map on top. */
  @container page (max-width: 720px) {
    .split {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: minmax(0, 1.4fr) minmax(0, 1fr);
    }
  }
</style>
