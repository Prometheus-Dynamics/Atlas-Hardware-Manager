<script lang="ts">
  // One screen: the 38 px top bar and the body grid. Each screen declares its own grid, as the
  // mockups do. The body never scrolls.
  import type { Snippet } from "svelte";

  let {
    title,
    crumb,
    grid,
    chips,
    actions,
    children,
  }: {
    title: string;
    crumb: string;
    /** The body's `grid-template-rows` and `grid-template-columns`, as the mockup has them. */
    grid: string;
    chips?: Snippet;
    actions?: Snippet;
    children: Snippet;
  } = $props();
</script>

<div class="main">
  <header class="top">
    <h1>{title}</h1>
    <span class="crumb">{crumb}</span>
    <div class="grow">{@render chips?.()}</div>
    <div class="right">{@render actions?.()}</div>
  </header>
  <div class="body" style={grid}>
    {@render children()}
  </div>
</div>
