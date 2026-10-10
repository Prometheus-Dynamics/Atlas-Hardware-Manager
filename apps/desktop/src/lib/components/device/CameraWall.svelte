<script lang="ts">
  // Many cameras on one screen: every view of the given devices as a tile,
  // named by device and view, with the device's frame rate and temperature.
  // A click opens the view full screen (with its device's other views).
  import { keyString, type DeviceRecord } from "#lib/api/client.ts";
  import { deviceName } from "#lib/format.ts";
  import { metricValue } from "#lib/metrics.ts";
  import { cameraStreams, streamLabel, type CameraStream } from "#lib/present.ts";
  import { live, watchLive } from "#lib/stores/live.svelte.ts";
  import CameraPreview from "./CameraPreview.svelte";
  import CameraViewer from "./CameraViewer.svelte";

  let { records, columns = 0 }: { records: DeviceRecord[]; columns?: number } = $props();

  const tiles = $derived(
    records
      .filter((r) => r.presence === "online")
      .flatMap((record) => cameraStreams(record).map((stream, index) => ({ record, stream, index }))),
  );
  let width = $state(1200);
  let wall = $state<HTMLDivElement>();
  let windowHeight = $state(900);
  // Auto: the columns that make the tiles largest with all of them on the
  // screen (16:9 views, a caption under each), the wall centred; a set
  // number of columns fills the width.
  const GAP = 8;
  const CAPTION = 22;
  const layout = $derived.by(() => {
    const n = tiles.length || 1;
    if (columns) return { cols: columns, tile: 0 };
    const room = Math.max(240, windowHeight - (wall?.getBoundingClientRect().top ?? 160) - 24);
    let best = { cols: 1, tile: 0 };
    for (let c = 1; c <= n; c++) {
      const rows = Math.ceil(n / c);
      const byWidth = (width - GAP * (c - 1)) / c;
      const byHeight = (((room - GAP * (rows - 1)) / rows - CAPTION) * 16) / 9;
      const tile = Math.floor(Math.min(byWidth, byHeight));
      if (tile > best.tile) best = { cols: c, tile };
    }
    // Never tiny: below about 280 px, scroll instead.
    return best.tile >= 280 ? best : { cols: Math.max(1, Math.min(n, Math.floor(width / 360))), tile: 0 };
  });
  let open = $state<{ streams: CameraStream[]; index: number; device: string } | null>(null);

  $effect(() => watchLive(records));

  function readings(record: DeviceRecord): string[] {
    const id = keyString(record.key);
    return ["fps", "temp"]
      .map((m) => live.metric(id, m))
      .filter((m) => !!m)
      .map((m) => {
        const shown = metricValue(m!);
        return `${shown.value} ${shown.unit}`;
      });
  }
</script>

<svelte:window bind:innerHeight={windowHeight} />

<div class="wall" bind:this={wall} bind:clientWidth={width} style="grid-template-columns: repeat({layout.cols}, {layout.tile ? `${layout.tile}px` : 'minmax(0, 1fr)'})">
  {#each tiles as tile (keyString(tile.record.key) + tile.stream.name)}
    <figure class="tile">
      <CameraPreview
        src={tile.stream.url}
        name={deviceName(tile.record)}
        label={streamLabel(tile.stream.name)}
        onexpand={() => (open = { streams: cameraStreams(tile.record), index: tile.index, device: deviceName(tile.record) })}
      />
      <figcaption class="info">
        <span class="dev">{deviceName(tile.record)}</span>
        <span class="view">{streamLabel(tile.stream.name)}</span>
        <span class="vals">{readings(tile.record).join(" · ")}</span>
      </figcaption>
    </figure>
  {:else}
    <p class="empty">None of these devices has a camera view online.</p>
  {/each}
</div>

{#if open}
  <CameraViewer streams={open.streams} bind:index={open.index} device={open.device} onclose={() => (open = null)} />
{/if}

<style>
  .wall {
    display: grid;
    gap: 8px;
    align-content: start;
    justify-content: center;
  }
  .tile {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .info {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 0 2px;
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
  }
  .dev {
    font-weight: 600;
    color: var(--fg);
  }
  .view {
    color: var(--fg-muted);
  }
  .vals {
    margin-left: auto;
    color: var(--fg-faint);
    font-variant-numeric: tabular-nums;
  }
  .empty {
    font-size: 13px;
    color: var(--fg-muted);
  }
</style>
