<script lang="ts">
  import Button from "#lib/components/common/Button.svelte";
  import EmptyState from "#lib/components/common/EmptyState.svelte";
  import PageHeader from "#lib/components/common/PageHeader.svelte";
  import Page from "#lib/components/layout/Page.svelte";
  import Region from "#lib/components/layout/Region.svelte";
  import RobotDetail from "#lib/components/robots/RobotDetail.svelte";
  import Skeleton from "#lib/components/common/Skeleton.svelte";
  import RobotCard from "#lib/components/robots/RobotCard.svelte";
  import { robots } from "#lib/stores/robots.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import { rise, softFade, stagger } from "#lib/ui/motion.ts";

  let width = $state(1200);
  const wide = $derived(width >= 840);
  let chosen = $state<string | null>(null);
  /** The robot in the detail view: the one picked, else the robot filter, else the first. */
  const selected = $derived(
    robots.profiles.find((p) => p.name === chosen) ?? robots.profiles.find((p) => p.name === ui.robot) ?? robots.profiles[0],
  );
  const ready = $derived(robots.statuses.filter((s) => s.state === "ready").length);
</script>

<svelte:head><title>Robots · Atlas</title></svelte:head>

<Page>
  {#snippet header()}
    <PageHeader
      title="Robots"
      subtitle={robots.profiles.length ? `${robots.profiles.length} robot${robots.profiles.length === 1 ? "" : "s"} · ${ready} ready` : "Which device fills which role, and the versions each robot should run."}
    >
      {#snippet actions()}
        <Button variant="primary" icon="plus" onclick={() => ui.openRobot(null)}>New robot</Button>
      {/snippet}
    </PageHeader>
  {/snippet}

  <!-- Wide: the robots on the left, the chosen one's devices filling the
       right. Narrow: just the cards, as many across as fit. -->
  <div class="layout" class:wide bind:clientWidth={width}>
    {#if !robots.loaded}
      <div class="auto-grid" style="--min: 22rem">
        {#each [0, 1] as i (i)}<div class="glass flex flex-col gap-3 p-5"><Skeleton width="40%" height={14} /><Skeleton height={60} /></div>{/each}
      </div>
    {:else if robots.profiles.length === 0}
      <EmptyState icon="robot" title="No robots yet" class="flex-1 justify-center">
        A robot groups devices into roles and sets the version each family should run. Then "Make ready" brings every
        device to its target in one job.
        <div class="mt-4 flex justify-center">
          <Button variant="primary" icon="plus" onclick={() => ui.openRobot(null)}>New robot</Button>
        </div>
      </EmptyState>
    {:else}
      <Region class="-mx-1" inner={wide ? "flex flex-col gap-3 p-1" : "auto-grid p-1"} label="Robots">
        {#each robots.profiles as profile, i (profile.name)}
          <div in:rise={{ delay: stagger(i) }}>
            <RobotCard
              {profile}
              status={robots.status(profile.name)}
              selected={wide && profile.name === selected?.name}
              onselect={wide ? () => (chosen = profile.name) : undefined}
            />
          </div>
        {/each}
      </Region>
      {#if wide && selected}
        {#key selected.name}
          <div class="min-h-0" in:softFade><RobotDetail profile={selected} status={robots.status(selected.name)} /></div>
        {/key}
      {/if}
    {/if}
  </div>
</Page>

<style>
  .layout {
    flex: 1 1 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .layout.wide {
    display: grid;
    grid-template-columns: clamp(22rem, 28%, 30rem) minmax(0, 1fr);
    gap: 12px;
  }
  .layout :global(.auto-grid) {
    --min: 24rem;
  }
</style>
