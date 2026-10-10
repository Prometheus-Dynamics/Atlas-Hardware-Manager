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
  // Auto: as many columns as fit at about 420 px.
  const cols = $derived(columns || Math.max(1, Math.min(tiles.length || 1, Math.floor(width / 420))));
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

<div class="wall" bind:clientWidth={width} style="grid-template-columns: repeat({cols}, minmax(0, 1fr))">
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
