<script lang="ts">
  import EmptyState from "$lib/components/common/EmptyState.svelte";
  import PageHeader from "$lib/components/common/PageHeader.svelte";
  import RobotCard from "$lib/components/robots/RobotCard.svelte";
  import { robots } from "$lib/stores/robots.svelte";
  import { ui } from "$lib/stores/ui.svelte";
</script>

<svelte:head><title>Robots · Atlas</title></svelte:head>

<div class="flex flex-col gap-3">
  <PageHeader eyebrow="Robots" title="Robot profiles" subtitle="Which device fills which role, and the versions each robot should run.">
    {#snippet actions()}
      <button type="button" class="btn btn-sm preset-tonal" onclick={() => ui.openRobot(null)}>
        <i class="fa-solid fa-plus" aria-hidden="true"></i>New robot
      </button>
    {/snippet}
  </PageHeader>

  {#if robots.loaded && robots.profiles.length === 0}
    <EmptyState icon="fa-robot" title="No robots yet">
      A robot groups devices into roles and sets the version each family should run. Then "Make ready" brings every
      device to its target in one job.
    </EmptyState>
  {:else}
    <div class="grid grid-cols-[repeat(auto-fill,minmax(22rem,1fr))] gap-3">
      {#each robots.profiles as profile (profile.name)}
        <RobotCard {profile} status={robots.status(profile.name)} />
      {/each}
    </div>
  {/if}
</div>
