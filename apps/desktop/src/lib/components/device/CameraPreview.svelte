<script lang="ts">
  // What the camera sees, when the device offers a stream. MJPEG streams
  // and still images both work in an <img>.
  import Icon from "#lib/components/common/Icon.svelte";

  let { src, name }: { src: string; name: string } = $props();

  let failed = $state(false);
  let loaded = $state(false);
</script>

<figure class="frame">
  {#if failed}
    <div class="flex h-full flex-col items-center justify-center gap-2 text-[12.5px] text-fg-faint">
      <Icon name="photo-off" size={22} />
      The camera stream isn't answering.
      <button type="button" class="link text-[12.5px]" onclick={() => ((failed = false), (loaded = false))}>Try again</button>
    </div>
  {:else}
    <img {src} alt="Live view from {name}" class:loaded onload={() => (loaded = true)} onerror={() => (failed = true)} />
    {#if !loaded}<div class="shimmer absolute inset-0"></div>{/if}
    <figcaption><span class="rec"></span>Live view</figcaption>
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
  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    opacity: 0;
    transition: opacity var(--t-med);
  }
  img.loaded {
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
    background: var(--err);
  }
</style>
