<script lang="ts">
  // One screen: the 38 px top bar and the body grid. Each screen declares its own grid, as the
  // mockups do. The body never scrolls.
  //
  // While the core cannot be reached at all the body is one panel that says so: no screen can
  // draw anything true then, and an empty list would read as "nothing is plugged in".
  import type { Snippet } from "svelte";
  import { app } from "../stores/app.svelte";

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
  {#if app.unreachable && !app.ready}
    <div class="body" style="grid-template-columns:minmax(0,1fr); grid-template-rows:minmax(0,1fr);">
      <section class="panel" data-panel="unreachable">
        <div class="ph">
          <h2>Not connected</h2><span class="grow"></span><span class="chip err"><span class="dot err"></span>Trying again</span>
        </div>
        <div class="pb">
          <div class="note err">
            Atlas cannot reach its own core, so nothing on this screen can be read: {app.unreachable}. It keeps trying, at most five seconds
            apart, and the screen fills in by itself when the core answers.
          </div>
        </div>
      </section>
    </div>
  {:else}
    <div class="body" style={grid}>
      {@render children()}
    </div>
  {/if}
</div>
