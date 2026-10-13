<script lang="ts">
  import { flip } from "svelte/animate";
  import { keyString } from "#lib/api/client.ts";
  import Icon from "#lib/components/common/Icon.svelte";
  import Skeleton from "#lib/components/common/Skeleton.svelte";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { SLOW_MS, watchLive } from "#lib/stores/live.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import { DUR, ease, ms, rise, softFade } from "#lib/ui/motion.ts";
  import DeviceCard from "./DeviceCard.svelte";
  import GroupHeader from "./GroupHeader.svelte";
  import { groupByRobot } from "./groups";

  /** fit: the height the grid has (px); cards grow to share it, up to a point. */
  let { fit = 0 }: { fit?: number } = $props();

  const groups = $derived(groupByRobot(ui.visible, ui.grouped));

  // Rows the cards take at the current column count, and the height that
  // leaves each: a short list fills the page instead of stopping halfway.
  let first: HTMLElement | undefined = $state();
  let width = $state(0);
  const cols = $derived.by(() => {
    void width;
    return first ? Math.max(1, getComputedStyle(first).gridTemplateColumns.split(" ").length) : 1;
  });
  const BASE = 176;
  const cardH = $derived.by(() => {
    if (!fit) return BASE;
    const rows = groups.reduce((n, g) => n + Math.ceil(g.records.length / cols), 0);
    const chrome = (ui.grouped ? groups.length * 40 : 0) + (groups.length - 1) * 24 + 48 + 8;
    return Math.round(Math.max(BASE, Math.min(300, (fit - chrome - (rows - groups.length) * 12) / Math.max(1, rows))));
  });
  // The extra height goes to each reading's trend line.
  const trend = $derived(cardH > BASE + 20 ? Math.min(44, Math.round((cardH - BASE) / 2)) : 0);

  $effect(() => watchLive(ui.visible, SLOW_MS));
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
  <div style="--card-h: {cardH}px" bind:clientWidth={width}>
    {#each groups as group, g (group.robot ?? "")}
      <section class="mb-6 last:mb-0" in:softFade>
        {#if ui.grouped}<GroupHeader robot={group.robot} count={group.records.length} />{/if}
        {#if g === 0}
          <div class="grid-cards" bind:this={first}>
            {#each group.records as record (keyString(record.key))}
              <div animate:flip={{ duration: ms(DUR.enter), easing: ease }} in:rise out:softFade>
                <DeviceCard {record} {trend} />
              </div>
            {/each}
          </div>
        {:else}
          <div class="grid-cards">
            {#each group.records as record (keyString(record.key))}
              <div animate:flip={{ duration: ms(DUR.enter), easing: ease }} in:rise out:softFade>
                <DeviceCard {record} {trend} />
              </div>
            {/each}
          </div>
        {/if}
      </section>
    {/each}
  </div>
  <!-- One line, not a card: it never takes a row of its own. -->
  <p class="hint">
    <Icon name="plug-connected" size={15} />
    <span>Plug in a device or join the robot network:</span>
    <span class="text-fg-faint">it shows up here the moment it's connected.</span>
  </p>
{/if}

<style>
  /* Cards grow with the page (cqw/cqh: the page body), so a big window
     gets bigger, roomier cards rather than a sliver of small ones. */
  .grid-cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(clamp(250px, 17cqw, 400px), 1fr));
    gap: 12px;
  }
  .grid-cards > :global(div) {
    min-width: 0;
  }
  .hint {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 12px;
    padding: 8px 12px;
    border: 1px dashed var(--glass-border-strong);
    border-radius: var(--r-card);
    font-size: 12.5px;
    color: var(--fg-muted);
  }
</style>
