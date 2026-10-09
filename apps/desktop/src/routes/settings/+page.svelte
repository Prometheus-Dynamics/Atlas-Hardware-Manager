<script lang="ts">
  import PageHeader from "#lib/components/common/PageHeader.svelte";
  import Page from "#lib/components/layout/Page.svelte";
  import Region from "#lib/components/layout/Region.svelte";
  import AboutPanel from "#lib/components/settings/AboutPanel.svelte";
  import HealthPanel from "#lib/components/settings/HealthPanel.svelte";
  import OrionPanel from "#lib/components/settings/OrionPanel.svelte";
  import PreferencesForm from "#lib/components/settings/PreferencesForm.svelte";
  import { system } from "#lib/stores/system.svelte.ts";

  let width = $state(0);
  // Three columns from 1400 px: Orion gets its own.
  const wide = $derived(width >= 1400);
</script>

<svelte:head><title>Settings · Atlas</title></svelte:head>

<!-- Section cards in columns that fit the window. Cards keep their natural
     height; a column that runs out of room scrolls as a whole, so no card is
     ever cut off with its own scrollbar. -->
<Page>
  {#snippet header()}
    <PageHeader title="Settings" subtitle="This computer's health, your preferences, and where files are kept." />
  {/snippet}

  <div class="sections" class:wide bind:clientWidth={width}>
    <Region class="col" inner="stack" label="Checks and files"><HealthPanel /><AboutPanel /></Region>
    <Region class="col" inner="stack" label={wide ? "Preferences" : "Preferences and Orion"}>
      {#if system.settings}<PreferencesForm settings={system.settings} />{/if}
      {#if !wide}<OrionPanel />{/if}
    </Region>
    {#if wide}<Region class="col" inner="stack" label="Orion"><OrionPanel /></Region>{/if}
  </div>
</Page>

<style>
  .sections {
    flex: 1 1 0;
    min-height: 0;
    display: grid;
    gap: 12px;
    grid-template-columns: minmax(0, 1fr);
    grid-template-rows: minmax(0, 1fr);
  }
  /* One column: everything in the first region's order, one scroll. */
  .sections :global(.col) {
    min-height: 0;
  }
  .sections :global(.stack) {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding-bottom: 4px;
  }
  @container page (max-width: 759px) {
    .sections {
      grid-template-rows: none;
      overflow-y: auto;
    }
    .sections :global(.col) {
      display: contents;
    }
  }
  /* Two columns: checks and files | preferences over Orion. */
  @container page (min-width: 760px) {
    .sections {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
  .sections.wide {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }
</style>
