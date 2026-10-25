<script lang="ts">
  // One reading across every device that reports it, a line each, newest at
  // the right, with the warning level as a dashed line and each device's
  // current value in the legend. Lines come from the live store's history
  // (the overview watches these devices), so the chart fills as it runs.
  import { keyString, type DeviceRecord } from "#lib/api/client.ts";
  import { deviceName } from "#lib/format.ts";
  import { metricTone } from "#lib/metrics.ts";
  import { live } from "#lib/stores/live.svelte.ts";

  let {
    records,
    metric,
    title,
    unit,
    decimals = 1,
  }: { records: DeviceRecord[]; metric: string; title: string; unit: string; decimals?: number } = $props();

  const COLORS = ["var(--series-1)", "var(--series-2)", "var(--series-3)", "var(--series-4)", "var(--series-5)"];
  // Drawn at the plot's own size in pixels: a stretched viewBox with
  // non-scaling strokes made WebKit's painter keep memory on every redraw
  // (~5 MB a minute on the Overview, measured).
  let W = $state(600);
  let H = $state(200);

  const lines = $derived(
    records
      .filter((r) => r.presence === "online")
      .map((r) => ({ record: r, id: keyString(r.key), now: live.metric(keyString(r.key), metric), values: live.series(keyString(r.key), metric) }))
      .filter((l) => l.now && l.values.length > 0)
      .map((l, i) => ({ ...l, color: COLORS[i % COLORS.length] })),
  );
  const warn = $derived(lines.find((l) => l.now?.warn_above != null)?.now?.warn_above ?? null);
  // Across the whole width from the first readings, not a sliver at the right.
  const capacity = $derived(Math.max(2, ...lines.map((l) => l.values.length)));

  // A range with room above the warning level, rounded to whole tens.
  const range = $derived.by(() => {
    const all = lines.flatMap((l) => l.values);
    if (all.length === 0) return { low: 0, high: 1 };
    let low = Math.min(...all);
    let high = Math.max(...all, warn ?? -Infinity);
    const pad = (high - low || 1) * 0.15;
    low = Math.floor((low - pad) / 10) * 10;
    high = Math.ceil((high + pad) / 10) * 10;
    return { low, high: high === low ? low + 10 : high };
  });
  const y = (v: number) => H - ((v - range.low) / (range.high - range.low)) * H;
  const ticks = $derived([range.high, (range.high * 2 + range.low) / 3, (range.high + range.low * 2) / 3, range.low]);

  function path(values: number[]): string {
    const step = W / (capacity - 1);
    return values.map((v, i) => `${i ? "L" : "M"}${(W - (values.length - 1 - i) * step).toFixed(1)} ${y(v).toFixed(1)}`).join(" ");
  }
</script>

<section class="chart" aria-label={title}>
  <div class="head">
    <h2>{title}</h2>
    <span class="sub">every device · live</span>
    <span class="flex-1"></span>
    {#if warn !== null}<span class="warn"><span class="dash"></span>warns above {warn} {unit}</span>{/if}
  </div>
  {#if lines.length === 0}
    <p class="empty">No device reports this yet.</p>
  {:else}
    <div class="plot">
      <div class="axis" aria-hidden="true">
        {#each ticks as t, i (i)}<span>{Math.round(t)}</span>{/each}
      </div>
      <div class="area" bind:clientWidth={W} bind:clientHeight={H}>
        <svg viewBox="0 0 {W} {H}" role="img" aria-label="{title} of {lines.length} devices">
          {#each ticks as t, i (i)}<line x1="0" x2={W} y1={y(t)} y2={y(t)} class="grid" />{/each}
          {#if warn !== null}<line x1="0" x2={W} y1={y(warn)} y2={y(warn)} class="limit" />{/if}
          {#each lines as l (l.id)}
            <path d={path(l.values)} stroke={l.color} class="line" class:hot={l.now && metricTone(l.now) === "warn"} />
          {/each}
        </svg>
      </div>
    </div>
    <ul class="legend">
      {#each lines as l (l.id)}
        <li>
          <span class="swatch" style:background={l.color}></span>{deviceName(l.record)}
          <span class="value" class:hot={l.now && metricTone(l.now) !== "ok" && metricTone(l.now) !== "neutral"}>{l.now?.value.toFixed(decimals)} {unit}</span>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .chart {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-height: 0;
    padding: 12px 14px;
    border-radius: var(--r-card);
    background: var(--glass);
    border: 1px solid var(--glass-border);
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  h2 {
    font-size: 13px;
    font-weight: 600;
  }
  .sub,
  .warn {
    font-size: 12px;
    color: var(--fg-muted);
  }
  .warn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .dash {
    width: 14px;
    border-top: 1.5px dashed var(--warn);
  }
  .plot {
    flex: 1;
    min-height: 80px;
    display: flex;
    gap: 8px;
  }
  .axis {
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    width: 26px;
    font-size: 10.5px;
    color: var(--fg-faint);
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  /* Clipped: a reading stamped just after the chart's clock draws no
     further than its edge. */
  .area {
    position: relative;
    flex: 1;
    min-width: 0;
    overflow: hidden;
  }
  svg {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }
  .grid {
    stroke: var(--glass-border);
  }
  .limit {
    stroke: var(--warn);
    stroke-dasharray: 5 4;
  }
  .line {
    fill: none;
    stroke-width: 1.5;
    stroke-linejoin: round;
  }
  .line.hot {
    stroke-width: 2.2;
  }
  /* Two lines at most, the rest a scroll away: the plot keeps the room. */
  .legend {
    flex-shrink: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 4px 14px;
    max-height: 38px;
    overflow-y: auto;
    padding-left: 34px;
    font-size: 12px;
  }
  .legend li {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .swatch {
    width: 10px;
    height: 3px;
    border-radius: 1px;
  }
  .value {
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
  }
  .value.hot {
    color: var(--warn-fg);
  }
  .empty {
    color: var(--fg-muted);
    font-size: 12.5px;
  }
</style>
