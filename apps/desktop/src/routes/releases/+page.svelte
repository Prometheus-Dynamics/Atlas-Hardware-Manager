<script lang="ts">
  import { api } from "$lib/api/client";
  import AsyncButton from "$lib/components/common/AsyncButton.svelte";
  import EmptyState from "$lib/components/common/EmptyState.svelte";
  import PageHeader from "$lib/components/common/PageHeader.svelte";
  import AddLocalRelease from "$lib/components/releases/AddLocalRelease.svelte";
  import ReleaseGroup from "$lib/components/releases/ReleaseGroup.svelte";
  import SourcesPanel from "$lib/components/releases/SourcesPanel.svelte";
  import { releases } from "$lib/stores/releases.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";

  let adding = $state(false);

  async function refresh() {
    releases.warnings = await api.refreshReleases();
    await releases.load();
    if (releases.warnings.length === 0) toasts.success("Release sources are up to date.");
  }
</script>

<svelte:head><title>Releases · Atlas</title></svelte:head>

<div class="flex flex-col gap-3">
  <PageHeader eyebrow="Releases" title="Release catalog" subtitle="Firmware and images Atlas can install, grouped by device family.">
    {#snippet actions()}
      <AsyncButton action={refresh}><i class="fa-solid fa-rotate" aria-hidden="true"></i>Refresh</AsyncButton>
      <button type="button" class="btn btn-sm preset-tonal" onclick={() => (adding = true)} disabled={adding}>
        <i class="fa-solid fa-file-circle-plus" aria-hidden="true"></i>Add local file
      </button>
    {/snippet}
  </PageHeader>

  {#if releases.warnings.length > 0}
    <div class="rounded-container border border-warning-600/40 bg-warning-500/5 px-3 py-2 text-xs text-warning-200" role="status">
      <div class="flex items-center justify-between">
        <p class="micro-label text-warning-300">Refresh warnings</p>
        <button type="button" class="text-surface-400 hover:text-surface-100" aria-label="Dismiss warnings" onclick={() => (releases.warnings = [])}>
          <i class="fa-solid fa-xmark" aria-hidden="true"></i>
        </button>
      </div>
      <ul class="mt-1">
        {#each releases.warnings as warning, i (i)}
          <li><i class="fa-solid fa-triangle-exclamation mr-1" aria-hidden="true"></i>{warning}</li>
        {/each}
      </ul>
    </div>
  {/if}

  {#if adding}<AddLocalRelease onclose={() => (adding = false)} />{/if}

  {#if releases.loaded && releases.entries.length === 0}
    <EmptyState icon="fa-box-archive" title="The catalog is empty">
      Add a remote source below and refresh, or add a local image or firmware file.
    </EmptyState>
  {/if}

  {#each releases.families as family (family)}
    <ReleaseGroup {family} />
  {/each}

  <SourcesPanel />
</div>
