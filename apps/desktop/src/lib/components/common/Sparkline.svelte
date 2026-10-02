<script lang="ts">
  // A trend line. Points sit at fixed spacing (`capacity` across), newest at
  // the right; each reading redraws it once, with no animation.

  let {
    values,
    color = "var(--accent)",
    height = 32,
    capacity = 60,
    min,
    max,
  }: {
    values: number[];
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
</script>

<div class="wrap" style="height: {height}px">
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
  {#if last}
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
  .dot {
    position: absolute;
    right: 0;
    width: 5px;
    height: 5px;
    margin: -2.5px -2.5px 0 0;
    border-radius: 50%;
  }
</style>
