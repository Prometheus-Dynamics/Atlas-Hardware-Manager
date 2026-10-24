<script lang="ts">
  import { api, errorText, keyString } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import ConfirmButton from "#lib/components/common/ConfirmButton.svelte";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import SegmentedControl from "#lib/components/common/SegmentedControl.svelte";
  import Toggle from "#lib/components/common/Toggle.svelte";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { ui, type InventoryView } from "#lib/stores/ui.svelte.ts";
  import { pop } from "#lib/ui/motion.ts";

  const views: { value: InventoryView; label: string; icon: "layout-grid" | "layout-list" }[] = [
    { value: "cards", label: "Cards", icon: "layout-grid" },
    { value: "list", label: "List", icon: "layout-list" },
  ];

  /** Selected devices that are offline: the ones Atlas can forget. */
  const forgettable = $derived(ui.selectedRecords.filter((r) => r.presence === "offline"));

  async function forgetSelected() {
    const list = forgettable;
    let failed = 0;
    for (const record of list) {
      try {
        await api.forgetDevice(record.key);
        ui.selection.delete(keyString(record.key));
      } catch (error) {
        failed++;
        toasts.error(errorText(error));
      }
    }
    if (list.length > failed) toasts.success(`Forgot ${list.length - failed} offline device${list.length - failed === 1 ? "" : "s"}.`);
  }

  function toggleFamily(family: string) {
    if (ui.families.has(family)) ui.families.delete(family);
    else ui.families.add(family);
  }
</script>

<!-- One line at every width: the family chips scroll sideways if they must. -->
<div class="flex shrink-0 items-center gap-3 whitespace-nowrap">
  <SegmentedControl options={views} value={ui.view} label="Layout" size="sm" onchange={(v) => ui.setView(v)} />

  {#if devices.families.length > 1}
    <div class="families flex min-w-0 items-center gap-1" role="group" aria-label="Device family">
      {#each devices.families as family (family)}
        <button type="button" class="chip" aria-pressed={ui.families.has(family)} onclick={() => toggleFamily(family)}>{family}</button>
      {/each}
    </div>
  {/if}

  <label class="flex shrink-0 items-center gap-2 text-[12.5px] text-fg-muted">
    <Toggle bind:checked={ui.showOffline} label="Show offline devices" />
    Show offline
  </label>

  {#if ui.filtersActive}
    <Button variant="ghost" size="sm" icon="filter-off" onclick={() => ui.clearFilters()}>Clear filters</Button>
  {/if}

  <div class="ml-auto flex shrink-0 items-center gap-2 text-[12.5px] text-fg-faint">
    {#if ui.selection.size > 0}
      <span class="text-accent-text" in:pop>{ui.selection.size} selected</span>
      {#if forgettable.length > 0}
        <ConfirmButton
          size="sm"
          icon="trash"
          action={forgetSelected}
          prompt="Forget {forgettable.length} offline device{forgettable.length === 1 ? '' : 's'}?"
          confirmLabel="Forget"
        >
          Forget {forgettable.length} offline
        </ConfirmButton>
      {/if}
      <Button variant="ghost" size="sm" onclick={() => ui.selection.clear()}>Clear</Button>
      <span class="text-fg-faint">·</span>
    {/if}
  </div>
</div>

<style>
  .families {
    overflow-x: auto;
    scrollbar-width: none;
  }
  .chip {
    flex-shrink: 0;
    height: 26px;
    padding: 0 11px;
    border-radius: var(--r-pill);
    font-size: 12px;
    font-weight: 500;
    color: var(--fg-muted);
    border: 1px solid var(--glass-border);
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
