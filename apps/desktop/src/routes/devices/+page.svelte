<script lang="ts">
  import { keyString } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import PageHeader from "#lib/components/common/PageHeader.svelte";
  import DeviceGrid from "#lib/components/inventory/DeviceGrid.svelte";
  import DeviceList from "#lib/components/inventory/DeviceList.svelte";
  import InventoryEmpty from "#lib/components/inventory/InventoryEmpty.svelte";
  import InventoryToolbar from "#lib/components/inventory/InventoryToolbar.svelte";
  import NeedsYou from "#lib/components/inventory/NeedsYou.svelte";
  import RobotMenu from "#lib/components/inventory/RobotMenu.svelte";
  import SearchPill from "#lib/components/inventory/SearchPill.svelte";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { insights } from "#lib/stores/insights.svelte.ts";
  import { system } from "#lib/stores/system.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import { softFade } from "#lib/ui/motion.ts";

  const shown = $derived(ui.visible);
  const online = $derived(shown.filter((d) => d.presence === "online").length);
  const needYou = $derived(
    shown.filter((d) => {
      const id = keyString(d.key);
      return insights.failedIds.has(id) || insights.waiting.includes(d);
    }).length,
  );
  const subline = $derived(
    devices.loaded
      ? `${shown.length} device${shown.length === 1 ? "" : "s"} · ${online} online${needYou ? ` · ${needYou} need${needYou === 1 ? "s" : ""} you` : ""}`
      : "Looking for devices…",
  );

  const staged = $derived(system.settings?.staged_default ?? "auto");
  const selecting = $derived(ui.selection.size > 0);
  const count = $derived(ui.updatable.length);
  const flashOnly = $derived(count > 0 && ui.updatable.every((d) => d.identity.mode === "recovery"));

  function updateAll() {
    const request = insights.updateAllRequest();
    if (request) ui.openUpdate(request, "Update all");
  }
</script>

<svelte:head><title>Devices · Atlas</title></svelte:head>

<div class="reveal flex flex-col gap-5">
  <PageHeader subtitle={subline}>
    {#snippet heading()}<RobotMenu />{/snippet}
    {#snippet actions()}
      <SearchPill />
      {#if selecting}
        <Button
          variant="primary"
          icon={flashOnly ? "bolt" : "arrow-up"}
          disabled={count === 0}
          title={count === 0 ? "None of the selected devices can be updated right now" : "Update the selection (U)"}
          onclick={() => ui.updateSelection(staged)}
        >
          {flashOnly ? "Flash" : "Update"} ({count})
        </Button>
      {:else if insights.outdated.length > 0}
        <Button variant="primary" icon="arrow-up" onclick={updateAll}>Update all</Button>
      {/if}
    {/snippet}
  </PageHeader>

  {#if devices.loaded && devices.all.length === 0}
    <InventoryEmpty />
  {:else}
    <NeedsYou />
    <InventoryToolbar />

    {#if devices.loaded && shown.length === 0}
      <p class="py-10 text-center text-[13px] text-fg-muted" in:softFade>
        No device matches.
        <button type="button" class="link" onclick={() => ui.clearFilters()}>Clear filters</button>
      </p>
    {:else if ui.view === "cards"}
      <DeviceGrid />
    {:else}
      <DeviceList />
    {/if}

    {#if devices.scanWarnings.length > 0}
      <ul class="flex flex-col gap-1 text-[12.5px] text-warn-fg">
        {#each devices.scanWarnings as warning, i (i)}
          <li class="flex items-center gap-2"><Icon name="alert-triangle" size={14} />{warning}</li>
        {/each}
      </ul>
    {/if}
  {/if}
</div>
