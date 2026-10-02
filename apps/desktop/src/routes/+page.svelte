<script lang="ts">
  // Home: the robot (or everything) at a glance. Readiness first, then live
  // numbers, how it's all connected, and what happened lately.
  import { goto } from "$app/navigation";
  import { sameKey } from "$lib/api/client";
  import Button from "$lib/components/common/Button.svelte";
  import GlassCard from "$lib/components/common/GlassCard.svelte";
  import Skeleton from "$lib/components/common/Skeleton.svelte";
  import PageHeader from "$lib/components/common/PageHeader.svelte";
  import RobotMenu from "$lib/components/inventory/RobotMenu.svelte";
  import DiscoveryStatus from "$lib/components/shell/DiscoveryStatus.svelte";
  import ActivityFeed from "$lib/components/overview/ActivityFeed.svelte";
  import Readiness from "$lib/components/overview/Readiness.svelte";
  import SummaryTiles from "$lib/components/overview/SummaryTiles.svelte";
  import TopologyMap from "$lib/components/overview/TopologyMap.svelte";
  import { activity } from "$lib/stores/activity.svelte";
  import { devices } from "$lib/stores/devices.svelte";
  import { robots } from "$lib/stores/robots.svelte";
  import { NO_ROBOT, ui } from "$lib/stores/ui.svelte";

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

<div class="flex flex-col gap-5">
  <PageHeader>
    {#snippet heading()}<RobotMenu />{/snippet}
    {#snippet actions()}
      <DiscoveryStatus />
      <Button icon="layout-grid" onclick={() => goto("/devices")}>All devices</Button>
    {/snippet}
  </PageHeader>

  {#if !ready}
    <div class="flex flex-col gap-5" aria-busy="true" aria-label="Loading the overview">
      <div class="glass flex h-[104px] items-center gap-5 px-6" style="border-radius: var(--r-panel)">
        <Skeleton width="60px" height={60} round />
        <div class="flex flex-1 flex-col gap-2"><Skeleton width="30%" height={20} /><Skeleton width="45%" height={12} /></div>
      </div>
      <div class="grid gap-3" style="grid-template-columns: repeat(auto-fit, minmax(210px, 1fr))">
        {#each [0, 1, 2, 3, 4] as i (i)}<Skeleton height={132} />{/each}
      </div>
      <div class="grid gap-4 lg:grid-cols-[minmax(0,1.7fr)_minmax(320px,1fr)]">
        <Skeleton height={360} />
        <Skeleton height={360} />
      </div>
    </div>
  {:else}
    <div class="reveal flex flex-col gap-5">
      <Readiness {records} {robot} />

      {#if records.length > 0}
        <SummaryTiles {records} />

        <div class="grid items-stretch gap-4 lg:grid-cols-[minmax(0,1.7fr)_minmax(320px,1fr)] min-[1800px]:grid-cols-[minmax(0,2.4fr)_minmax(380px,1fr)]">
          <GlassCard class="min-w-0" title="How it's connected" subtitle="Click a device to open it" icon="sitemap">
            <div class="px-1 pb-2"><TopologyMap {records} /></div>
          </GlassCard>
          <GlassCard class="min-w-0" title="Recent activity" icon="history">
            <div class="flex flex-col pb-2">
              <ActivityFeed entries={feed} limit={9} empty="Nothing has happened here yet." />
            </div>
          </GlassCard>
        </div>
      {/if}
    </div>
  {/if}
</div>
