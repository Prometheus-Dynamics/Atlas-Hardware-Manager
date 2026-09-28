<script lang="ts">
  import { keyString } from "$lib/api/client";
  import { ui } from "$lib/stores/ui.svelte";
  import DeviceRow from "./DeviceRow.svelte";

  const visibleKeys = $derived(ui.visible.map((d) => keyString(d.key)));
  const selectedVisible = $derived(visibleKeys.filter((k) => ui.selection.has(k)).length);
  const all = $derived(visibleKeys.length > 0 && selectedVisible === visibleKeys.length);
  const some = $derived(selectedVisible > 0 && !all);

  const columns = ["Name", "Family", "Model", "Version", "Mode", "Link", "Robot", "State"];
</script>

<div class="overflow-x-auto rounded-container border border-surface-800 bg-surface-900/40">
  <table class="w-full border-collapse" aria-label="Devices">
    <thead>
      <tr class="border-b border-surface-800 text-left">
        <th class="w-8 px-2 py-1.5">
          <input
            type="checkbox"
            class="checkbox h-3.5 w-3.5 rounded-sm border-surface-600 bg-surface-900"
            checked={all}
            indeterminate={some}
            onchange={() => ui.selectAllVisible()}
            aria-label="Select all shown devices"
          />
        </th>
        {#each columns as column (column)}
          <th class="micro-label px-2 py-1.5 font-medium">{column}</th>
        {/each}
      </tr>
    </thead>
    <tbody>
      {#each ui.visible as record (keyString(record.key))}
        <DeviceRow {record} />
      {/each}
    </tbody>
  </table>
  {#if ui.visible.length === 0}
    <p class="px-4 py-6 text-center text-xs text-surface-500">
      No device matches these filters.
      <button type="button" class="underline hover:text-surface-200" onclick={() => ui.clearFilters()}>Clear filters</button>
    </p>
  {/if}
</div>
