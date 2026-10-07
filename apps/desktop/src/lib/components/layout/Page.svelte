<script lang="ts">
  // A route's frame: a header that stays put and a body that takes the rest
  // of the window. The page itself never scrolls; regions inside the body do
  // (Region.svelte). The body is a size container named `page`, so what's
  // inside can adapt to the space it actually has (`@container page (…)`,
  // Tailwind's `@lg:` and friends, `cqw`/`cqh` units).
  import type { Snippet } from "svelte";

  let {
    header,
    children,
    class: extra = "",
  }: {
    header: Snippet;
    children: Snippet;
    /** Classes for the body; it is a flex column with a gap unless overridden. */
    class?: string;
  } = $props();
</script>

<div class="page reveal">
  <div class="min-w-0">{@render header()}</div>
  <div class="body {extra}">{@render children()}</div>
</div>

<style>
  .page {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    gap: clamp(12px, 1.6vh, 20px);
    height: 100%;
    /* Below this the window is too short to split; the shell scrolls instead. */
    min-height: 440px;
  }
  .body {
    container: page / size;
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
    min-height: 0;
  }
</style>
