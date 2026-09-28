<script lang="ts">
  import { devices } from "$lib/stores/devices.svelte";
  import { ui } from "$lib/stores/ui.svelte";

  let input: HTMLInputElement | undefined = $state();
  $effect(() => {
    ui.filterInput = input ?? null;
    return () => (ui.filterInput = null);
  });

  function toggleFamily(family: string) {
    if (ui.families.has(family)) ui.families.delete(family);
    else ui.families.add(family);
  }
</script>

<div class="flex flex-wrap items-center gap-2">
  <div class="relative">
    <i class="fa-solid fa-magnifying-glass pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-[0.65rem] text-surface-500" aria-hidden="true"></i>
    <input
      bind:this={input}
      bind:value={ui.filterText}
      class="field w-64 pl-6"
      type="search"
      placeholder="Filter by name, serial, model…"
      aria-label="Filter devices"
      onkeydown={(e) => {
        if (e.key === "ArrowDown") {
          e.preventDefault();
          input?.blur();
          ui.moveFocus(0);
        }
      }}
    />
    <kbd class="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 rounded-base border border-surface-700 px-1 font-mono text-[0.6rem] text-surface-500">/</kbd>
  </div>

  {#if devices.families.length > 1}
    <div class="flex items-center gap-1" role="group" aria-label="Family filter">
      {#each devices.families as family (family)}
        <button type="button" class="chip-toggle" aria-pressed={ui.families.has(family)} onclick={() => toggleFamily(family)}>
          {family}
        </button>
      {/each}
    </div>
  {/if}

  <label class="ml-1 flex items-center gap-1.5 text-xs text-surface-300">
    <input type="checkbox" class="checkbox h-3.5 w-3.5 rounded-sm border-surface-600 bg-surface-900" bind:checked={ui.showOffline} />
    Show offline
  </label>

  {#if ui.filtersActive}
    <button type="button" class="btn btn-sm preset-tonal" onclick={() => ui.clearFilters()}>
      <i class="fa-solid fa-filter-circle-xmark" aria-hidden="true"></i>Clear filters
    </button>
  {/if}

  <span class="ml-auto text-[0.65rem] text-surface-500">
    {ui.visible.length} shown{#if ui.selection.size > 0} · <span class="text-primary-200">{ui.selection.size} selected</span>
      <button type="button" class="ml-1 underline hover:text-surface-200" onclick={() => ui.selection.clear()}>clear</button>{/if}
  </span>
</div>
