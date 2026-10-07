<script lang="ts">
  import { api } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import EmptyState from "#lib/components/common/EmptyState.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import PageHeader from "#lib/components/common/PageHeader.svelte";
  import Page from "#lib/components/layout/Page.svelte";
  import Region from "#lib/components/layout/Region.svelte";
  import Skeleton from "#lib/components/common/Skeleton.svelte";
  import AddLocalRelease from "#lib/components/releases/AddLocalRelease.svelte";
  import ReleaseGroup from "#lib/components/releases/ReleaseGroup.svelte";
  import SourcesPanel from "#lib/components/releases/SourcesPanel.svelte";
  import { releases } from "#lib/stores/releases.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { rise, softFade, stagger } from "#lib/ui/motion.ts";

  let adding = $state(false);
  let width = $state(1200);
  const wide = $derived(width >= 1100);

  async function refresh() {
    releases.warnings = await api.refreshReleases();
    await releases.load();
    if (releases.warnings.length === 0) toasts.success("Release sources are up to date.");
  }
</script>

<svelte:head><title>Releases · Atlas</title></svelte:head>

<Page>
  {#snippet header()}
    <PageHeader title="Releases" subtitle="Images and firmware Atlas can install, by device family. Your 5 newest local images stay listed; pin any you want to keep.">
      {#snippet actions()}
        <Button icon="refresh" action={refresh}>Check for new releases</Button>
        <Button variant="primary" icon="file-plus" onclick={() => (adding = true)} disabled={adding}>Add a file</Button>
      {/snippet}
    </PageHeader>
  {/snippet}

  {#if releases.warnings.length > 0}
    <div class="glass flex shrink-0 items-start gap-3 px-4 py-3" role="status" transition:softFade>
      <Icon name="alert-triangle" size={18} class="mt-0.5 text-warn-fg" />
      <ul class="flex-1 text-[13px] text-warn-fg">
        {#each releases.warnings as warning, i (i)}<li>{warning}</li>{/each}
      </ul>
      <Button variant="ghost" size="sm" icon="x" label="Dismiss" onclick={() => (releases.warnings = [])} />
    </div>
  {/if}

  {#if adding}<div class="shrink-0" transition:softFade><AddLocalRelease onclose={() => (adding = false)} /></div>{/if}

  <!-- Families in as many columns as fit; sources beside them on a wide
       window, or folded into one row under them on a narrow one. -->
  <div class="layout" class:wide bind:clientWidth={width}>
    {#if !releases.loaded}
      <div class="glass flex flex-col gap-3 p-5"><Skeleton width="30%" height={14} /><Skeleton height={40} /><Skeleton height={40} /></div>
    {:else if releases.entries.length === 0}
      <Region><EmptyState icon="package" title="The catalog is empty">
        Add a source {wide ? "on the right" : "below"} and check for new releases, or add an image or firmware file from this computer.
      </EmptyState></Region>
    {:else}
      <Region class="-mx-1" inner="families auto-grid fit p-1" label="Release families">
        {#each releases.families as family, i (family)}
          <div class="min-h-0" in:rise={{ delay: stagger(i) }}><ReleaseGroup {family} /></div>
        {/each}
      </Region>
    {/if}
    <SourcesPanel compact={!wide} />
  </div>
</Page>

<style>
  .layout {
    flex: 1 1 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .layout.wide {
    display: grid;
    grid-template-columns: minmax(0, 1fr) clamp(300px, 24%, 420px);
  }
  /* Rows share the height: few families get tall cards, many scroll. */
  .layout :global(.families) {
    --min: 400px;
    --max: 4;
    min-height: 100%;
    grid-auto-rows: minmax(240px, 1fr);
  }
</style>
