<script lang="ts">
  // A floating glass sheet that slides in from the right edge of its
  // positioned parent. The page behind stays usable.
  import type { Snippet } from "svelte";
  import { slideIn } from "#lib/ui/motion.ts";
  import Button from "./Button.svelte";

  let {
    label,
    width = 440,
    onclose,
    header,
    scroll = true,
    children,
  }: {
    label: string;
    width?: number;
    onclose: () => void;
    header?: Snippet;
    /** False when the content manages its own scrolling (pinned header). */
    scroll?: boolean;
    children: Snippet;
  } = $props();
</script>

<aside class="sheet glass-layer" style="--w: {width}px" aria-label={label} transition:slideIn>
  <div class="close">
    <Button variant="ghost" size="sm" icon="x" label="Close (Esc)" onclick={onclose} />
  </div>
  {#if header}<div class="shrink-0">{@render header()}</div>{/if}
  <div class="flex min-h-0 flex-1 flex-col" class:overflow-y-auto={scroll}>{@render children()}</div>
</aside>

<style>
  .sheet {
    position: absolute;
    top: 12px;
    right: 12px;
    bottom: 12px;
    z-index: 30;
    display: flex;
    flex-direction: column;
    width: min(var(--w), calc(100% - 24px));
    border-radius: var(--r-panel);
    overflow: hidden;
    transition: width var(--t-med) var(--ease-out);
  }
  .close {
    position: absolute;
    top: 14px;
    right: 14px;
    z-index: 2;
  }
</style>
