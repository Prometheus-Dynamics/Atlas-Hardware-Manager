<script lang="ts">
  // A circular gauge. The arc eases to new values; the centre is free for
  // a value or an icon.
  import type { Snippet } from "svelte";

  let {
    fraction,
    size = 64,
    stroke = 6,
    color = "var(--accent)",
    children,
  }: { fraction: number; size?: number; stroke?: number; color?: string; children?: Snippet } = $props();

  const r = $derived((size - stroke) / 2);
  const c = $derived(2 * Math.PI * r);
  // Three quarters of a circle, open at the bottom.
  const sweep = 0.75;
  const dash = $derived(`${c * sweep * Math.min(1, Math.max(0, fraction))} ${c}`);
  const track = $derived(`${c * sweep} ${c}`);
</script>

<div class="ring" style="width: {size}px; height: {size}px">
  <svg viewBox="0 0 {size} {size}" aria-hidden="true">
    <g transform="rotate(135 {size / 2} {size / 2})">
      <circle cx={size / 2} cy={size / 2} {r} fill="none" style="stroke: var(--glass-strong)" stroke-width={stroke} stroke-linecap="round" stroke-dasharray={track} />
      <circle
        class="arc"
        cx={size / 2}
        cy={size / 2}
        {r}
        fill="none"
        stroke-width={stroke}
        stroke-linecap="round"
        stroke-dasharray={dash}
        style="stroke: {color}"
      />
    </g>
  </svg>
  {#if children}<div class="centre">{@render children()}</div>{/if}
</div>

<style>
  .ring {
    position: relative;
    flex-shrink: 0;
  }
  svg {
    width: 100%;
    height: 100%;
  }
  .arc {
    transition:
      stroke-dasharray var(--t-data) var(--ease-out),
      stroke var(--t-med);
  }
  .centre {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
  }
</style>
