<script lang="ts">
  import { flip } from "svelte/animate";
  import { keyString } from "$lib/api/client";
  import Icon from "$lib/components/common/Icon.svelte";
  import Skeleton from "$lib/components/common/Skeleton.svelte";
  import { devices } from "$lib/stores/devices.svelte";
  import { watchLive } from "$lib/stores/live.svelte";
  import { ui } from "$lib/stores/ui.svelte";
  import { DUR, ease, ms, rise, softFade } from "$lib/ui/motion";
  import DeviceCard from "./DeviceCard.svelte";
  import GroupHeader from "./GroupHeader.svelte";
  import { groupByRobot } from "./groups";

  const groups = $derived(groupByRobot(ui.visible, ui.grouped));

  $effect(() => watchLive(ui.visible));
</script>

{#if !devices.loaded}
  <div class="grid-cards" aria-busy="true" aria-label="Loading devices">
    {#each [0, 1, 2, 3, 4, 5] as i (i)}
      <div class="glass flex h-[168px] flex-col gap-3 p-4">
        <Skeleton width="36px" height={36} />
        <Skeleton width="60%" height={13} />
        <Skeleton width="40%" height={11} />
        <div class="mt-auto"><Skeleton width="45%" height={20} round /></div>
      </div>
    {/each}
  </div>
{:else}
  {#each groups as group (group.robot ?? "")}
    <section class="mb-6 last:mb-0" in:softFade>
      {#if ui.grouped}<GroupHeader robot={group.robot} count={group.records.length} />{/if}
      <div class="grid-cards">
        {#each group.records as record (keyString(record.key))}
          <div animate:flip={{ duration: ms(DUR.enter), easing: ease }} in:rise out:softFade>
            <DeviceCard {record} />
          </div>
        {/each}
        {#if group === groups[groups.length - 1]}
          <div class="hint-card">
            <Icon name="plug-connected" size={20} />
            <p>Plug in a device or join the robot network</p>
            <span class="text-[12px] text-fg-faint">
              It shows up here the moment it's connected
            </span>
          </div>
        {/if}
      </div>
    </section>
  {/each}
{/if}

<style>
  .grid-cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(250px, 1fr));
    gap: 12px;
  }
  .grid-cards > :global(div) {
    min-width: 0;
  }
  .hint-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    min-height: 168px;
    padding: 16px;
    text-align: center;
    border-radius: var(--r-card);
    border: 1px dashed var(--glass-border-strong);
    color: var(--fg-faint);
  }
  .hint-card p {
    font-size: 13px;
    color: var(--fg-muted);
    max-width: 180px;
  }
</style>
