<script lang="ts">
  // A sparkline, as the design language draws it: an SVG path in a stretched viewBox, a 14
  // percent area fill and a 1.5 px line that does not scale. It snaps to new data; nothing tweens.
  let { values, series = 1, height = 34 }: { values: number[]; series?: number | "offline"; height?: number } = $props();

  const W = 160;
  const colour = $derived(series === "offline" ? "var(--offline)" : `var(--series-${series})`);
  const line = $derived.by(() => {
    if (values.length < 2) return "";
    const low = Math.min(...values);
    const high = Math.max(...values);
    const span = high - low || 1;
    const top = 3;
    const bottom = height - 3;
    return values
      .map((value, i) => {
        const x = (i * W) / (values.length - 1);
        const y = high === low ? (top + bottom) / 2 : bottom - ((value - low) / span) * (bottom - top);
        return `${i === 0 ? "M" : "L"}${x.toFixed(1)},${y.toFixed(1)}`;
      })
      .join("");
  });
</script>

<svg class="spark" viewBox="0 0 {W} {height}" preserveAspectRatio="none" aria-hidden="true">
  {#if line}
    <path d="{line}L{W},{height}L0,{height}Z" fill={colour} fill-opacity=".14" />
    <path d={line} fill="none" stroke={colour} stroke-width="1.5" vector-effect="non-scaling-stroke" stroke-linejoin="round" />
  {/if}
</svg>
