<script lang="ts">
  import Button from "#lib/components/common/Button.svelte";
  import EmptyState from "#lib/components/common/EmptyState.svelte";
  import PageHeader from "#lib/components/common/PageHeader.svelte";
  import Skeleton from "#lib/components/common/Skeleton.svelte";
  import RobotCard from "#lib/components/robots/RobotCard.svelte";
  import { robots } from "#lib/stores/robots.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import { rise, stagger } from "#lib/ui/motion.ts";

  const ready = $derived(robots.statuses.filter((s) => s.state === "ready").length);
</script>

<svelte:head><title>Robots · Atlas</title></svelte:head>

<div class="reveal flex flex-col gap-5">
  <PageHeader
    title="Robots"
    subtitle={robots.profiles.length ? `${robots.profiles.length} robot${robots.profiles.length === 1 ? "" : "s"} · ${ready} ready` : "Which device fills which role, and the versions each robot should run."}
  >
    {#snippet actions()}
      <Button variant="primary" icon="plus" onclick={() => ui.openRobot(null)}>New robot</Button>
    {/snippet}
  </PageHeader>

  {#if !robots.loaded}
    <div class="grid grid-cols-[repeat(auto-fill,minmax(24rem,1fr))] gap-4">
      {#each [0, 1] as i (i)}<div class="glass flex flex-col gap-3 p-5"><Skeleton width="40%" height={14} /><Skeleton height={60} /></div>{/each}
    </div>
  {:else if robots.profiles.length === 0}
    <EmptyState icon="robot" title="No robots yet">
      A robot groups devices into roles and sets the version each family should run. Then "Make ready" brings every
      device to its target in one job.
    </EmptyState>
  {:else}
    <div class="grid grid-cols-[repeat(auto-fill,minmax(24rem,1fr))] gap-4">
      {#each robots.profiles as profile, i (profile.name)}
        <div in:rise={{ delay: stagger(i) }}><RobotCard {profile} status={robots.status(profile.name)} /></div>
      {/each}
    </div>
  {/if}
</div>
