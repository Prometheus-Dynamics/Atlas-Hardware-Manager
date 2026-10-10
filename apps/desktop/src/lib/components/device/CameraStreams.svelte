<script lang="ts">
  // A device's camera views: one large, or several in a grid, each named.
  // A click opens the view full screen, with the others to switch to.
  import { streamLabel, type CameraStream } from "#lib/present.ts";
  import CameraPreview from "./CameraPreview.svelte";
  import CameraViewer from "./CameraViewer.svelte";

  let { streams, device }: { streams: CameraStream[]; device: string } = $props();

  let open = $state<number | null>(null);
</script>

<div class="streams" class:several={streams.length > 1}>
  {#each streams as stream, i (stream.name)}
    <CameraPreview
      src={stream.url}
      name={device}
      label={streams.length > 1 ? streamLabel(stream.name) : "Live view"}
      onexpand={() => (open = i)}
    />
  {/each}
</div>

{#if open !== null}
  <CameraViewer {streams} bind:index={open} {device} onclose={() => (open = null)} />
{/if}

<style>
  .streams {
    display: grid;
    gap: 8px;
  }
  .streams.several {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
</style>
