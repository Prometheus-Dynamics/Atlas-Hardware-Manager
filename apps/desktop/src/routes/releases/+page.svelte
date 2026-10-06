<script lang="ts">
  import { api } from "$lib/api/client";
  import Button from "$lib/components/common/Button.svelte";
  import EmptyState from "$lib/components/common/EmptyState.svelte";
  import Icon from "$lib/components/common/Icon.svelte";
  import PageHeader from "$lib/components/common/PageHeader.svelte";
  import Skeleton from "$lib/components/common/Skeleton.svelte";
  import AddLocalRelease from "$lib/components/releases/AddLocalRelease.svelte";
  import ReleaseGroup from "$lib/components/releases/ReleaseGroup.svelte";
  import SourcesPanel from "$lib/components/releases/SourcesPanel.svelte";
  import { releases } from "$lib/stores/releases.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { rise, softFade, stagger } from "$lib/ui/motion";

  let adding = $state(false);

  async function refresh() {
    releases.warnings = await api.refreshReleases();
    await releases.load();
    if (releases.warnings.length === 0) toasts.success("Release sources are up to date.");
  }
</script>

<svelte:head><title>Releases · Atlas</title></svelte:head>

<div class="reveal flex flex-col gap-5">
  <PageHeader title="Releases" subtitle="Images and firmware Atlas can install, by device family. Your 5 newest local images stay listed; pin any you want to keep.">
    {#snippet actions()}
      <Button icon="refresh" action={refresh}>Check for new releases</Button>
      <Button variant="primary" icon="file-plus" onclick={() => (adding = true)} disabled={adding}>Add a file</Button>
    {/snippet}
  </PageHeader>

  {#if releases.warnings.length > 0}
    <div class="glass flex items-start gap-3 px-4 py-3" role="status" transition:softFade>
      <Icon name="alert-triangle" size={18} class="mt-0.5 text-warn-fg" />
      <ul class="flex-1 text-[13px] text-warn-fg">
        {#each releases.warnings as warning, i (i)}<li>{warning}</li>{/each}
      </ul>
      <Button variant="ghost" size="sm" icon="x" label="Dismiss" onclick={() => (releases.warnings = [])} />
    </div>
  {/if}

  {#if adding}<div transition:softFade><AddLocalRelease onclose={() => (adding = false)} /></div>{/if}

  {#if !releases.loaded}
    <div class="glass flex flex-col gap-3 p-5"><Skeleton width="30%" height={14} /><Skeleton height={40} /><Skeleton height={40} /></div>
  {:else if releases.entries.length === 0}
    <EmptyState icon="package" title="The catalog is empty">
      Add a source below and check for new releases, or add an image or firmware file from this computer.
    </EmptyState>
  {/if}

  {#each releases.families as family, i (family)}
    <div in:rise={{ delay: stagger(i) }}><ReleaseGroup {family} /></div>
  {/each}

  <SourcesPanel />
</div>
