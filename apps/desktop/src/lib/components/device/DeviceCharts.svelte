<script lang="ts">
  // The device's live readings, each once, as a chart with its current
  // value: temperature (with the fan under it), CPU, vision rate, memory,
  // then whatever else it reports (voltage, current, …). Uptime is in the
  // header, CPU cores in the CPU's detail. Polled while shown.
  import { keyString, type DeviceRecord, type Metric } from "#lib/api/client.ts";
  import Skeleton from "#lib/components/common/Skeleton.svelte";
  import { isCore, isTextMetric, metricTone, metricValue } from "#lib/metrics.ts";
  import { live, watchLive } from "#lib/stores/live.svelte.ts";

  let { record }: { record: DeviceRecord } = $props();

  $effect(() => watchLive([record]));

  const FIRST = ["temp", "cpu", "fps", "memory"];
  const COLORS = ["var(--series-3)", "var(--series-1)", "var(--series-2)", "var(--series-4)", "var(--series-5)"];
  /** Shown elsewhere: the fan under the temperature, uptime in the header. */
  const ELSEWHERE = new Set(["fan", "uptime"]);
  const W = 400;
  const H = 100;

  const id = $derived(keyString(record.key));
  const all = $derived(live.metrics.get(id) ?? []);
  const fan = $derived(all.find((m) => m.id === "fan") ?? null);
  const charts = $derived(
    all
      .filter((m) => !isCore(m) && !ELSEWHERE.has(m.id) && !isTextMetric(m))
      .sort((a, b) => rank(a.id) - rank(b.id))
      .slice(0, 6)
      .map((metric) => ({ metric, values: live.series(id, metric.id), color: COLORS[Math.min(rank(metric.id), 4)] }))
      .filter((c) => c.values.length > 0),
  );
  const error = $derived(live.errors.get(id));

  function rank(metricId: string): number {
    const i = FIRST.indexOf(metricId);
    return i < 0 ? FIRST.length : i;
  }

  function detail(c: { metric: Metric; values: number[] }): string {
    if (c.metric.id === "temp" && fan) {
      const v = metricValue(fan);
      return `fan ${v.value} ${v.unit}${c.metric.warn_above != null ? ` · warns above ${c.metric.warn_above}` : ""}`;
    }
    if (c.metric.detail) return c.metric.detail;
    const d = c.metric.id === "temp" ? 1 : 0;
    return `low ${Math.min(...c.values).toFixed(d)} · high ${Math.max(...c.values).toFixed(d)}`;
  }

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

{#if charts.length === 0 && error}
  <p class="text-[13px] text-warn-fg">{error}</p>
{:else if charts.length === 0}
  <div class="charts">
    {#each [0, 1, 2, 3] as i (i)}<Skeleton height={130} />{/each}
  </div>
{:else}
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
        <div class="range" title={detail(c)}>{detail(c)}</div>
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
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
