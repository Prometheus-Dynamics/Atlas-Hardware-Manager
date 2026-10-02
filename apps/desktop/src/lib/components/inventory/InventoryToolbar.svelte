<script lang="ts">
  import Button from "$lib/components/common/Button.svelte";
  import SegmentedControl from "$lib/components/common/SegmentedControl.svelte";
  import Toggle from "$lib/components/common/Toggle.svelte";
  import { timeAgo } from "$lib/format";
  import { clock } from "$lib/stores/clock.svelte";
  import { devices } from "$lib/stores/devices.svelte";
  import { ui, type InventoryView } from "$lib/stores/ui.svelte";
  import { pop } from "$lib/ui/motion";

  const views: { value: InventoryView; label: string; icon: "layout-grid" | "layout-list" }[] = [
    { value: "cards", label: "Cards", icon: "layout-grid" },
    { value: "list", label: "List", icon: "layout-list" },
  ];

  /** Only a scan the user asked for spins the button; background scans are quiet. */
  let manual = $state(false);
  async function scan() {
    manual = true;
    try {
      await devices.scanNow();
    } finally {
      manual = false;
    }
  }

  function toggleFamily(family: string) {
    if (ui.families.has(family)) ui.families.delete(family);
    else ui.families.add(family);
  }
</script>

<div class="flex flex-wrap items-center gap-x-3 gap-y-2">
  <SegmentedControl options={views} value={ui.view} label="Layout" size="sm" onchange={(v) => ui.setView(v)} />

  {#if devices.families.length > 1}
    <div class="flex flex-wrap items-center gap-1" role="group" aria-label="Device family">
      {#each devices.families as family (family)}
        <button type="button" class="chip" aria-pressed={ui.families.has(family)} onclick={() => toggleFamily(family)}>{family}</button>
      {/each}
    </div>
  {/if}

  <label class="flex items-center gap-2 text-[12.5px] text-fg-muted">
    <Toggle bind:checked={ui.showOffline} label="Show offline devices" />
    Show offline
  </label>

  {#if ui.filtersActive}
    <Button variant="ghost" size="sm" icon="filter-off" onclick={() => ui.clearFilters()}>Clear filters</Button>
  {/if}

  <div class="ml-auto flex items-center gap-2 text-[12.5px] text-fg-faint">
    {#if ui.selection.size > 0}
      <span class="text-accent-text" in:pop>{ui.selection.size} selected</span>
      <Button variant="ghost" size="sm" onclick={() => ui.selection.clear()}>Clear</Button>
      <span class="text-fg-faint">·</span>
    {/if}
    <span aria-live="polite">
      {#if devices.lastScanAt}Checked {timeAgo(devices.lastScanAt, clock.now)}{:else if devices.scanning}Looking for devices…{/if}
    </span>
    <Button variant="ghost" size="sm" icon="radar-2" busy={manual} onclick={scan} title="Scan now (S)">Scan</Button>
  </div>
</div>

<style>
  .chip {
    height: 26px;
    padding: 0 11px;
    border-radius: var(--r-pill);
    font-size: 12px;
    font-weight: 500;
    color: var(--fg-muted);
    border: 0.5px solid var(--glass-border);
    background: transparent;
    transition:
      background var(--t-fast),
      color var(--t-fast),
      border-color var(--t-fast);
  }
  .chip:hover {
    color: var(--fg);
    background: var(--glass);
  }
  .chip[aria-pressed="true"] {
    color: var(--fg);
    background: var(--glass-strong);
    border-color: var(--glass-border-strong);
  }
</style>
