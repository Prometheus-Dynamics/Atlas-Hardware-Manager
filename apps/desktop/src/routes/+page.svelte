<script lang="ts">
  import PageHeader from "$lib/components/common/PageHeader.svelte";
  import DeviceTable from "$lib/components/inventory/DeviceTable.svelte";
  import InventoryEmpty from "$lib/components/inventory/InventoryEmpty.svelte";
  import InventoryFilters from "$lib/components/inventory/InventoryFilters.svelte";
  import { devices } from "$lib/stores/devices.svelte";
</script>

<svelte:head><title>Inventory · Atlas</title></svelte:head>

<div class="flex flex-col gap-3">
  <PageHeader eyebrow="Inventory" title="Devices" subtitle="Everything Atlas can reach right now, and what it has seen before." />
  {#if devices.all.length === 0}
    <InventoryEmpty />
  {:else}
    <InventoryFilters />
    <DeviceTable />
    {#if devices.scanWarnings.length > 0}
      <ul class="rounded-container border border-warning-600/40 bg-warning-500/5 px-3 py-2 text-xs text-warning-200">
        {#each devices.scanWarnings as warning, i (i)}
          <li><i class="fa-solid fa-triangle-exclamation mr-1" aria-hidden="true"></i>{warning}</li>
        {/each}
      </ul>
    {/if}
  {/if}
</div>
