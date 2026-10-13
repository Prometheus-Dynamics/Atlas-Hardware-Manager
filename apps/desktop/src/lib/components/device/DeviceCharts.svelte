<script lang="ts">
  // The device's main readings as charts, filling the space under the live
  // tiles: temperature, CPU, vision rate, memory (whichever it reports), each
  // with its current value, its range since the page opened and the warning
  // level when there is one. Fed by the same live polling as the tiles.
  import { keyString, type DeviceRecord, type Metric } from "#lib/api/client.ts";
  import { metricTone, metricValue } from "#lib/metrics.ts";
  import { live } from "#lib/stores/live.svelte.ts";

  let { record }: { record: DeviceRecord } = $props();

  const ORDER: [string, string][] = [
    ["temp", "var(--series-3)"],
    ["cpu", "var(--series-1)"],
    ["fps", "var(--series-2)"],
    ["memory", "var(--series-4)"],
  ];
  const W = 400;
  const H = 100;

  const id = $derived(keyString(record.key));
  const charts = $derived(
    ORDER.map(([metricId, color]) => ({ metric: live.metric(id, metricId), values: live.series(id, metricId), color })).filter(
      (c): c is { metric: Metric; values: number[]; color: string } => !!c.metric && c.values.length > 1,
    ),
  );

  function scale(c: { metric: Metric; values: number[] }) {
    // Temperatures float well above 0; the rest are counts and shares from 0.
    const low = c.metric.id === "temp" ? Math.floor(Math.min(...c.values) - 5) : 0;
    const high = Math.max(c.metric.max ?? -Infinity, ...c.values, c.metric.warn_above ?? -Infinity);
    const span = high - low || 1;
    return (v: number) => H - ((v - low) / span) * (H - 6) - 3;
  }

  function line(c: { metric: Metric; values: number[] }): string {
    const y = scale(c);
    const step = W / (Math.max(30, c.values.length) - 1);
    return c.values.map((v, i) => `${i ? "L" : "M"}${(W - (c.values.length - 1 - i) * step).toFixed(1)} ${y(v).toFixed(1)}`).join(" ");
  }
</script>

{#if charts.length > 0}
  <div class="charts" class:one={charts.length === 1}>
    {#each charts as c (c.metric.id)}
      {@const v = metricValue(c.metric)}
      {@const tone = metricTone(c.metric)}
      {@const d = line(c)}
      <section class="chart" aria-label={c.metric.label}>
        <div class="head">
          <span class="label">{c.metric.label}</span>
          <span class="value" class:hot={tone === "warn" || tone === "err"}>{v.value}</span>
          <span class="unit">{v.unit}</span>
        </div>
        <div class="range">
          low {Math.min(...c.values).toFixed(c.metric.id === "temp" ? 1 : 0)} · high {Math.max(...c.values).toFixed(c.metric.id === "temp" ? 1 : 0)}
          {#if c.metric.warn_above != null}· warns above {c.metric.warn_above}{/if}
        </div>
        <svg viewBox="0 0 {W} {H}" preserveAspectRatio="none" aria-hidden="true">
          {#if c.metric.warn_above != null}
            <line x1="0" x2={W} y1={scale(c)(c.metric.warn_above)} y2={scale(c)(c.metric.warn_above)} class="limit" />
          {/if}
          <path d="{d} L{W} {H} L{W - (c.values.length - 1) * (W / (Math.max(30, c.values.length) - 1))} {H} Z" fill={c.color} class="area" />
          <path {d} stroke={c.color} class="line" />
        </svg>
      </section>
    {/each}
  </div>
{/if}

<style>
  .charts {
    flex: 1;
    min-height: 220px;
    display: grid;
    gap: 10px;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    grid-auto-rows: minmax(110px, 1fr);
  }
  .charts.one {
    grid-template-columns: minmax(0, 1fr);
  }
  .chart {
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding: 10px 12px 0;
    border-radius: var(--r-card);
    background: var(--glass);
    border: 1px solid var(--glass-border);
    overflow: hidden;
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: 6px;
  }
  .label {
    flex: 1;
    font-size: 12px;
    color: var(--fg-muted);
  }
  .value {
    font-size: 18px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .value.hot {
    color: var(--warn-fg);
  }
  .unit {
    font-size: 12px;
    color: var(--fg-muted);
  }
  .range {
    font-size: 11px;
    color: var(--fg-faint);
    font-variant-numeric: tabular-nums;
  }
  svg {
    flex: 1;
    min-height: 50px;
    width: calc(100% + 24px);
    margin: 6px -12px 0;
  }
  .line {
    fill: none;
    stroke-width: 1.6;
    vector-effect: non-scaling-stroke;
  }
  .area {
    opacity: 0.14;
  }
  .limit {
    stroke: var(--warn);
    stroke-dasharray: 5 4;
    vector-effect: non-scaling-stroke;
  }
</style>
