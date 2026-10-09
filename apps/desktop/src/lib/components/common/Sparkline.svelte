<script lang="ts">
  // A trend line. Points sit at fixed spacing (`capacity` across), newest at
  // the right; each reading redraws it once, with no animation. Hovering
  // shows the nearest point's exact value and, with `times`, when it was read.

  let {
    values,
    color = "var(--accent)",
    height = 32,
    capacity = 60,
    min,
    max,
    times = [],
    format = (value: number) => value.toLocaleString(undefined, { maximumFractionDigits: 3 }),
  }: {
    values: number[];
    /** When each value was read (ms since the epoch), oldest first, aligned to the newest value. */
    times?: number[];
    /** A value as text with its unit, for the hover label. */
    format?: (value: number) => string;
    color?: string;
    height?: number;
    /** Samples across the full width. */
    capacity?: number;
    min?: number;
    max?: number;
  } = $props();

  const W = 100;
  const id = `spark-${Math.random().toString(36).slice(2, 9)}`;
  const step = $derived(W / (capacity - 1));

  // Range with a little headroom so the line never touches the edges.
  const range = $derived.by(() => {
    const low = min ?? Math.min(...values);
    const high = max ?? Math.max(...values);
    const pad = (high - low || Math.abs(high) || 1) * 0.15;
    return { low: low - (min === undefined ? pad : 0), high: high + (max === undefined ? pad : 0) };
  });

  // Right-aligned; one extra point off the right edge is scrolled into view.
  const points = $derived(
    values.map((v, i) => {
      const x = W - (values.length - 1 - i) * step;
      const y = height - ((v - range.low) / (range.high - range.low || 1)) * height;
      return [x, y] as const;
    }),
  );

  const line = $derived.by(() => {
    if (points.length < 2) return "";
    let d = `M${points[0][0]},${points[0][1]}`;
    for (let i = 1; i < points.length; i++) {
      const [x0, y0] = points[i - 1];
      const [x1, y1] = points[i];
      const mx = (x0 + x1) / 2;
      d += ` C${mx},${y0} ${mx},${y1} ${x1},${y1}`;
    }
    return d;
  });
  const area = $derived(line ? `${line} L${W},${height} L${points[0][0]},${height} Z` : "");
  const last = $derived(points.at(-1));

  let wrap = $state<HTMLDivElement>();
  /** The hovered point: its index and where it is on screen. */
  let hover = $state<{ index: number; x: number; left: number; top: number } | null>(null);

  function move(event: PointerEvent) {
    if (!wrap || values.length === 0) return;
    const box = wrap.getBoundingClientRect();
    const x = ((event.clientX - box.left) / box.width) * W;
    const index = Math.min(values.length - 1, Math.round(values.length - 1 - (W - x) / step));
    if (index < 0) {
      hover = null;
      return;
    }
    const [px, py] = points[index];
    hover = { index, x: px, left: box.left + (px / W) * box.width, top: box.top + py };
  }

  const hoverTime = $derived.by(() => {
    if (!hover) return null;
    const t = times[times.length - (values.length - hover.index)];
    return t === undefined ? null : new Date(t).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  });
</script>

<div class="wrap" style="height: {height}px" bind:this={wrap} onpointermove={move} onpointerleave={() => (hover = null)} role="presentation">
  <!-- A faint baseline, so a fresh trend reads as "filling in", not broken. -->
  <span class="base" style="background: {color}"></span>
  <div>
    <svg viewBox="0 0 {W} {height}" preserveAspectRatio="none" style="height: {height}px" class="w-full overflow-visible" aria-hidden="true">
      {#if line}
        <defs>
          <linearGradient id={id} x1="0" x2="0" y1="0" y2="1">
            <stop offset="0" style="stop-color: {color}; stop-opacity: 0.26" />
            <stop offset="1" style="stop-color: {color}; stop-opacity: 0" />
          </linearGradient>
        </defs>
        <path d={area} fill="url(#{id})" />
        <path d={line} fill="none" style="stroke: {color}" stroke-width="1.6" vector-effect="non-scaling-stroke" stroke-linejoin="round" />
      {/if}
    </svg>
  </div>
  {#if hover}
    <span class="guide" style="left: {(hover.x / W) * 100}%; background: {color}"></span>
    <span class="dot hover-dot" style="left: {(hover.x / W) * 100}%; top: {points[hover.index][1]}px; background: {color}"></span>
    <div class="tip glass-layer" style="left: {hover.left}px; top: {hover.top}px" role="tooltip">
      <span class="tip-value">{format(values[hover.index])}</span>{#if hoverTime}<span class="tip-time">{hoverTime}</span>{/if}
    </div>
  {:else if last}
    <span class="dot" style="top: {last[1]}px; background: {color}; box-shadow: 0 0 0 3px color-mix(in srgb, {color} 25%, transparent)"></span>
  {/if}
</div>

<style>
  .wrap {
    position: relative;
    width: 100%;
    overflow: hidden;
    }
  .base {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 1px;
    opacity: 0.18;
  }
  .guide {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1px;
    opacity: 0.45;
    pointer-events: none;
  }
  .hover-dot {
    right: auto !important;
    margin: -2.5px 0 0 -2.5px !important;
    pointer-events: none;
  }
  /* Fixed, so the tile's clipping doesn't cut it off. */
  .tip {
    position: fixed;
    z-index: 60;
    transform: translate(-50%, calc(-100% - 10px));
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 4px 8px;
    border-radius: 7px;
    white-space: nowrap;
    pointer-events: none;
    font-variant-numeric: tabular-nums;
  }
  .tip-value {
    font-size: 12px;
    font-weight: 600;
    color: var(--fg);
  }
  .tip-time {
    font-size: 11px;
    color: var(--fg-faint);
  }
  .dot {
    position: absolute;
    right: 0;
    width: 5px;
    height: 5px;
    margin: -2.5px -2.5px 0 0;
    border-radius: 50%;
  }
</style>
