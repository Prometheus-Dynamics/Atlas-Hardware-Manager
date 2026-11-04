<script lang="ts">
  // One reading across every device that reports it, a line each over the
  // last five minutes (or what there is of them), newest at the right, with
  // the warning level as a dashed line and each device's current value in the
  // legend. Lines come from the live store's history (the overview watches
  // these devices), placed by time: a device read twice a second and one read
  // every 15 s line up.
  import { keyString, type DeviceRecord } from "#lib/api/client.ts";
  import { deviceName } from "#lib/format.ts";
  import { metricTone } from "#lib/metrics.ts";
  import { clock } from "#lib/stores/clock.svelte.ts";
  import { live } from "#lib/stores/live.svelte.ts";

  let {
    records,
    metric,
    title,
    unit,
    decimals = 1,
  }: { records: DeviceRecord[]; metric: string; title: string; unit: string; decimals?: number } = $props();

  const COLORS = ["var(--series-1)", "var(--series-2)", "var(--series-3)", "var(--series-4)", "var(--series-5)"];
  const WINDOW_MS = 300_000;
  // Drawn at the plot's own size in pixels: a stretched viewBox with
  // non-scaling strokes made WebKit's painter keep memory on every redraw
  // (~5 MB a minute on the Overview, measured).
  let W = $state(600);
  let H = $state(200);

  const online = $derived(records.filter((r) => r.presence === "online"));
  const start = $derived(live.chartStart(online.map((r) => keyString(r.key)), clock.now - WINDOW_MS, clock.now));
  const span = $derived(Math.max(1, clock.now - start));
  const lines = $derived(
    online
      .map((r) => ({ record: r, id: keyString(r.key), now: live.metric(keyString(r.key), metric), points: live.window(keyString(r.key), metric, start) }))
      .filter((l) => l.now && l.points.length > 0)
      .map((l, i) => ({ ...l, color: COLORS[i % COLORS.length] })),
  );
  const warn = $derived(lines.find((l) => l.now?.warn_above != null)?.now?.warn_above ?? null);

  // A range with room above the warning level, rounded to whole tens.
  const range = $derived.by(() => {
    const all = lines.flatMap((l) => l.points.map((p) => p.v));
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

  const x = (t: number) => ((t - start) / span) * W;
  const path = (points: { t: number; v: number }[]) =>
    points.map((p, i) => `${i ? "L" : "M"}${x(p.t).toFixed(1)} ${y(p.v).toFixed(1)}`).join(" ");
</script>

<section class="chart" aria-label={title}>
  <div class="head">
    <h2>{title}</h2>
    <span class="sub">every device · last {span < 120_000 ? `${Math.round(span / 1000)} s` : `${Math.round(span / 60_000)} min`}</span>
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
        <svg viewBox="0 0 {W} {H}" data-live role="img" aria-label="{title} of {lines.length} devices">
          {#each ticks as t, i (i)}<line x1="0" x2={W} y1={y(t)} y2={y(t)} class="grid" />{/each}
          {#if warn !== null}<line x1="0" x2={W} y1={y(warn)} y2={y(warn)} class="limit" />{/if}
          {#each lines as l (l.id)}
            <path d={path(l.points)} stroke={l.color} class="line" class:hot={l.now && metricTone(l.now) === "warn"} />
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
