<script lang="ts">
  // A camera view across the whole window: the stream as large as it fits,
  // its device and view named, the device's other views to switch to.
  // Esc, the close button or a click outside closes it.
  import Button from "#lib/components/common/Button.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import { streamLabel, type CameraStream } from "#lib/present.ts";
  import { portal } from "#lib/ui/portal.ts";

  let {
    streams,
    index = $bindable(0),
    device,
    onclose,
  }: { streams: CameraStream[]; index?: number; device: string; onclose: () => void } = $props();

  const stream = $derived(streams[Math.min(index, streams.length - 1)]);
  let failed = $state(false);

  function key(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.stopPropagation();
      onclose();
    } else if (event.key === "ArrowRight" && streams.length > 1) {
      index = (index + 1) % streams.length;
      failed = false;
    } else if (event.key === "ArrowLeft" && streams.length > 1) {
      index = (index - 1 + streams.length) % streams.length;
      failed = false;
    }
  }
</script>

<svelte:window onkeydown={key} />

<div class="viewer" use:portal role="dialog" aria-modal="true" aria-label="{device}: {streamLabel(stream.name)}">
  <button type="button" class="backdrop" aria-label="Close" onclick={onclose}></button>
  <div class="frame">
    {#if failed}
      <div class="fail"><Icon name="photo-off" size={28} />The stream isn't answering.</div>
    {:else}
      <img src={stream.url} alt="{device}: {streamLabel(stream.name)}" onerror={() => (failed = true)} />
    {/if}
  </div>
  <header class="bar">
    <span class="title"><span class="rec"></span>{device} · {streamLabel(stream.name)}</span>
    {#if streams.length > 1}
      <div class="views" role="tablist" aria-label="Camera views">
        {#each streams as other, i (other.name)}
          <button type="button" role="tab" class="view" class:on={i === index} aria-selected={i === index} onclick={() => ((index = i), (failed = false))}>
            {streamLabel(other.name)}
          </button>
        {/each}
      </div>
    {/if}
    <Button size="sm" icon="x" onclick={onclose}>Close</Button>
  </header>
</div>

<style>
  .viewer {
    position: fixed;
    inset: 0;
    z-index: 90;
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    padding: 12px;
    gap: 10px;
  }
  .backdrop {
    position: absolute;
    inset: 0;
    background: rgba(5, 6, 8, 0.92);
    cursor: default;
  }
  .bar {
    position: relative;
    grid-row: 1;
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .frame {
    position: relative;
    grid-row: 2;
    display: grid;
    place-items: center;
    min-height: 0;
  }
  img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
    border-radius: var(--r-md);
    background: #000;
  }
  .fail {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    color: var(--fg-faint);
    font-size: 13px;
  }
  .title {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 14px;
    font-weight: 600;
    color: var(--fg);
  }
  .rec {
    width: 7px;
    height: 7px;
    border-radius: var(--r-round);
    background: var(--err);
  }
  .views {
    display: flex;
    gap: 4px;
  }
  /* The close button at the far end. */
  .bar > :global(:last-child) {
    margin-left: auto;
  }
  .view {
    height: 26px;
    padding: 0 10px;
    font-size: 12.5px;
    color: var(--fg-muted);
    border: 1px solid var(--glass-border);
    border-radius: var(--r-pill);
  }
  .view:hover {
    color: var(--fg);
    background: var(--glass-hover);
  }
  .view.on {
    color: var(--fg);
    border-color: var(--accent-ring);
    background: var(--accent-tint);
  }
</style>
