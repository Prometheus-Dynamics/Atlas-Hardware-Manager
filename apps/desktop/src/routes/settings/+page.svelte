<script lang="ts">
  import PageHeader from "$lib/components/common/PageHeader.svelte";
  import AboutPanel from "$lib/components/settings/AboutPanel.svelte";
  import HealthPanel from "$lib/components/settings/HealthPanel.svelte";
  import PreferencesForm from "$lib/components/settings/PreferencesForm.svelte";
  import { system } from "$lib/stores/system.svelte";
</script>

<svelte:head><title>Settings · Atlas</title></svelte:head>

<div class="flex flex-col gap-5">
  <PageHeader title="Settings" subtitle="This computer's health, your preferences, and where Atlas keeps its files." />
  <div class="grid grid-cols-1 items-start gap-4 xl:grid-cols-2">
    <div class="flex flex-col gap-4">
      <HealthPanel />
      <AboutPanel />
    </div>
    {#if system.settings}
      <PreferencesForm settings={system.settings} />
    {/if}
  </div>
</div>
