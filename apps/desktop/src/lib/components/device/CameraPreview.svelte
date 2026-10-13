<script lang="ts">
  // What the camera sees, when the device offers a stream (MJPEG or a
  // still image), healing itself when the stream breaks (CameraFeed). A
  // click opens it full screen (`onexpand`), with the device's other views
  // beside it.
  import Icon from "#lib/components/common/Icon.svelte";
  import CameraFeed from "./CameraFeed.svelte";

  let {
    src,
    name,
    label = "Live view",
    caption = true,
    onexpand,
  }: { src: string; name: string; label?: string; caption?: boolean; onexpand?: () => void } = $props();

  let phase = $state<"connecting" | "live" | "reconnecting">("connecting");
</script>

<figure class="frame">
  <CameraFeed {src} alt="Live view from {name}" bind:phase />
  {#if caption}<figcaption><span class="rec" class:lost={phase !== "live"}></span>{label}</figcaption>{/if}
  {#if onexpand}
    <button type="button" class="expand" aria-label="Open {label} full screen" title="Full screen" onclick={onexpand}>
      <Icon name="arrows-maximize" size={15} />
    </button>
  {/if}
</figure>

<style>
  .frame {
    position: relative;
    aspect-ratio: 16 / 9;
    overflow: hidden;
    border-radius: var(--r-card);
    background: var(--inset);
    border: 1px solid var(--glass-border);
  }
  /* The whole preview opens it; the corner icon says so. */
  .expand {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: flex-start;
    justify-content: flex-end;
    padding: 9px;
    color: var(--fg);
    cursor: zoom-in;
  }
  .expand :global(svg) {
    padding: 4px;
    width: 23px;
    height: 23px;
    border-radius: var(--r-sm);
    background: rgba(0, 0, 0, 0.55);
    opacity: 0;
    transition: opacity var(--t-fast);
  }
  .frame:hover .expand :global(svg),
  .expand:focus-visible :global(svg) {
    opacity: 1;
  }
  figcaption {
    position: absolute;
    left: 10px;
    top: 10px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 9px;
    border-radius: var(--r-pill);
    font-size: 11px;
    font-weight: 500;
    color: var(--fg);
    background: rgba(0, 0, 0, 0.6);
  }
  .rec {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--ok);
  }
  .rec.lost {
    background: var(--warn);
  }
</style>
