<script lang="ts">
  // A device's camera views: one large, or several in a grid, each named;
  // with `single`, one large view and buttons to switch between them (only
  // the one shown streams). A click opens the view full screen, with the
  // others to switch to.
  import { streamLabel, type CameraStream } from "#lib/present.ts";
  import CameraPreview from "./CameraPreview.svelte";
  import CameraViewer from "./CameraViewer.svelte";

  let { streams, device, single = false }: { streams: CameraStream[]; device: string; single?: boolean } = $props();

  let open = $state<number | null>(null);
  let shown = $state(0);
  const current = $derived(streams[Math.min(shown, streams.length - 1)]);
</script>

{#if single && streams.length > 0}
  <div class="single">
    {#key current.url}
      <CameraPreview
        src={current.url}
        name={device}
        label={streams.length > 1 ? streamLabel(current.name) : "Live view"}
        onexpand={() => (open = Math.min(shown, streams.length - 1))}
      />
    {/key}
    {#if streams.length > 1}
      <div class="switch" role="group" aria-label="Camera view">
        {#each streams as stream, i (stream.name)}
          <button type="button" aria-pressed={i === shown} class:on={i === shown} onclick={() => (shown = i)}>{streamLabel(stream.name)}</button>
        {/each}
      </div>
    {/if}
  </div>
{:else}
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
{/if}

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
  .single {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .switch {
    display: flex;
    gap: 6px;
  }
  .switch button {
    flex: 1;
    min-width: 0;
    height: 28px;
    border-radius: var(--r-sm);
    border: 1px solid var(--glass-border);
    background: transparent;
    color: var(--fg-muted);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .switch button:hover {
    color: var(--fg);
    background: var(--glass-hover);
  }
  .switch button.on {
    border-color: var(--accent);
    background: var(--accent-tint);
    color: var(--accent-text-strong);
  }
</style>
