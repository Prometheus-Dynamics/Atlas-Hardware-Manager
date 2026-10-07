<script lang="ts">
  // A bounded area whose content scrolls inside it: device grids, release
  // lists, feeds, logs. It takes whatever height its parent leaves
  // (flex-1, min-h-0) and shows a soft edge where there is more above or
  // below, so a cut-off list reads as scrollable.
  import type { Snippet } from "svelte";

  let {
    children,
    class: extra = "",
    inner = "",
    label,
    viewport = $bindable(),
  }: {
    children: Snippet;
    /** Classes for the region box (sizing, borders). */
    class?: string;
    /** Classes for the content wrapper (padding, grid). */
    inner?: string;
    /** Names the region for assistive tech. */
    label?: string;
    /** The scrolling element, for callers that follow new content. */
    viewport?: HTMLDivElement;
  } = $props();

  let content: HTMLDivElement | undefined = $state();
  let above = $state(false);
  let below = $state(false);

  function measure() {
    const el = viewport;
    if (!el) return;
    above = el.scrollTop > 1;
    below = el.scrollTop + el.clientHeight < el.scrollHeight - 1;
  }

  // Re-check when the box or its content changes size; scrolling checks too.
  $effect(() => {
    if (!viewport || !content) return;
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(viewport);
    observer.observe(content);
    return () => observer.disconnect();
  });
</script>

<div class="region {extra}" class:above class:below>
  <div class="scroller" bind:this={viewport} onscroll={measure} role={label ? "region" : undefined} aria-label={label}>
    <div class={inner} bind:this={content}>{@render children()}</div>
  </div>
</div>

<style>
  .region {
    position: relative;
    display: flex;
    flex-direction: column;
    flex: 1 1 auto;
    min-width: 0;
    min-height: 0;
  }
  .scroller {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  .region::before,
  .region::after {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    z-index: 2;
    height: 12px;
    pointer-events: none;
    opacity: 0;
    transition: opacity var(--t-fast);
  }
  .region::before {
    top: 0;
    border-top: 1px solid var(--hairline);
    background: linear-gradient(to bottom, var(--scroll-edge), transparent);
  }
  .region::after {
    bottom: 0;
    border-bottom: 1px solid var(--hairline);
    background: linear-gradient(to top, var(--scroll-edge), transparent);
  }
  .above::before,
  .below::after {
    opacity: 1;
  }
</style>
