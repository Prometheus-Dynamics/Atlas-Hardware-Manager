<script lang="ts">
  import PageHeader from "#lib/components/common/PageHeader.svelte";
  import Page from "#lib/components/layout/Page.svelte";
  import AboutPanel from "#lib/components/settings/AboutPanel.svelte";
  import HealthPanel from "#lib/components/settings/HealthPanel.svelte";
  import OrionPanel from "#lib/components/settings/OrionPanel.svelte";
  import PreferencesForm from "#lib/components/settings/PreferencesForm.svelte";
  import { system } from "#lib/stores/system.svelte.ts";
</script>

<svelte:head><title>Settings · Atlas</title></svelte:head>

<!-- Section cards in columns that fit the window. Each card takes its
     column's height and scrolls inside when its content is longer. -->
<Page>
  {#snippet header()}
    <PageHeader title="Settings" subtitle="This computer's health, your preferences, and where Atlas keeps its files." />
  {/snippet}

  <div class="sections">
    <div class="col health"><HealthPanel /><AboutPanel /></div>
    <div class="col prefs">
      {#if system.settings}<PreferencesForm settings={system.settings} />{/if}
    </div>
    <div class="col orion"><OrionPanel /></div>
  </div>
</Page>

<style>
  .sections {
    flex: 1 1 0;
    min-height: 0;
    display: grid;
    gap: 12px;
    grid-template-columns: minmax(0, 1fr);
    /* Rows as tall as their cards: when the grid scrolls, nothing squeezes. */
    grid-auto-rows: max-content;
    align-content: start;
    overflow-y: auto;
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
    min-height: 0;
  }
  /* Cards keep their natural height and give way when the column is short
     (their bodies scroll); the last one stretches so columns end together. */
  .col > :global(*) {
    flex: 0 1 auto;
    min-height: 140px;
  }
  .col > :global(:last-child) {
    flex-grow: 1;
  }
  /* Two columns: checks and files | preferences over Orion. */
  @container page (min-width: 760px) {
    .sections {
      grid-template-columns: repeat(2, minmax(0, 1fr));
      grid-template-rows: fit-content(62%) minmax(0, 1fr);
      grid-template-areas: "health prefs" "health orion";
      align-content: stretch;
      overflow: visible;
    }
    .health {
      grid-area: health;
    }
    .prefs {
      grid-area: prefs;
    }
    .orion {
      grid-area: orion;
    }
  }
  /* Three columns on a wide window. */
  @container page (min-width: 1400px) {
    .sections {
      grid-template-columns: repeat(3, minmax(0, 1fr));
      grid-template-rows: minmax(0, 1fr);
      grid-template-areas: "health prefs orion";
    }
  }
  /* A short window: no room to split the height, so the cards keep their
     natural height and the section grid scrolls as one region instead of
     four cramped ones. */
  @container page (max-height: 600px) {
    .sections {
      grid-template-rows: none;
      grid-auto-rows: max-content;
      align-content: start;
      overflow-y: auto;
    }
    .col > :global(*) {
      flex: none;
    }
  }
</style>
