<script lang="ts">
  import { flip } from "svelte/animate";
  import { keyString } from "#lib/api/client.ts";
  import Checkbox from "#lib/components/common/Checkbox.svelte";
  import Skeleton from "#lib/components/common/Skeleton.svelte";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import { DUR, ease, ms, rise, softFade } from "#lib/ui/motion.ts";
  import DeviceListRow from "./DeviceListRow.svelte";
  import GroupHeader from "./GroupHeader.svelte";
  import { groupByRobot } from "./groups";

  const groups = $derived(groupByRobot(ui.visible, ui.grouped));
  const keys = $derived(ui.visible.map((d) => keyString(d.key)));
  const picked = $derived(keys.filter((k) => ui.selection.has(k)).length);
</script>

<div class="glass list" role="grid" aria-label="Devices">
  <div class="head" role="row">
    <Checkbox
      checked={keys.length > 0 && picked === keys.length}
      indeterminate={picked > 0 && picked < keys.length}
      label="Select all shown devices"
      onclick={() => ui.selectAllVisible()}
    />
    <span>Device</span><span>Model</span><span>Version</span><span>Connection</span><span class="text-right">Status</span>
  </div>
  {#if !devices.loaded}
    {#each [0, 1, 2, 3] as i (i)}
      <div class="flex items-center gap-4 px-3.5 py-3"><Skeleton width="28px" height={28} /><Skeleton width="30%" /></div>
    {/each}
  {:else}
    {#each groups as group (group.robot ?? "")}
      <div class="px-1.5 pb-1" in:softFade>
        {#if ui.grouped}<div class="px-2 pt-2"><GroupHeader robot={group.robot} count={group.records.length} /></div>{/if}
        {#each group.records as record (keyString(record.key))}
          <div animate:flip={{ duration: ms(DUR.enter), easing: ease }} in:rise out:softFade>
            <DeviceListRow {record} />
          </div>
        {/each}
      </div>
    {/each}
  {/if}
</div>

<style>
  .list {
    padding: 0 0 6px;
    border-radius: var(--r-panel);
  }
  .head {
    display: grid;
    grid-template-columns: 20px minmax(180px, 1.4fr) minmax(120px, 1fr) minmax(90px, 0.7fr) minmax(90px, 0.8fr) minmax(150px, 1fr);
    align-items: center;
    gap: 16px;
    padding: 12px 20px 10px;
    margin-bottom: 4px;
    /* Column names stay in view while the rows scroll under them. */
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--layer-solid);
    border-radius: var(--r-panel) var(--r-panel) 0 0;
    border-bottom: 1px solid var(--hairline);
    font-size: 12px;
    font-weight: 500;
    color: var(--fg-faint);
  }
</style>
