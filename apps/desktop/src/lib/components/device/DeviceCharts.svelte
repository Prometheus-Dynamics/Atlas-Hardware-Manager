<script lang="ts">
  // The live readings the Health rows don't draw, each as a chart with its
  // current value: the vision rate first, then whatever else the device
  // reports (voltage, current, …). Polled while shown, for Health too.
  import { keyString, type DeviceRecord, type Metric } from "#lib/api/client.ts";
  import Skeleton from "#lib/components/common/Skeleton.svelte";
  import { isCore, isTextMetric, metricTone, metricValue } from "#lib/metrics.ts";
  import { live, watchLive } from "#lib/stores/live.svelte.ts";

  let { record }: { record: DeviceRecord } = $props();

  $effect(() => watchLive([record]));

  const COLORS = ["var(--series-2)", "var(--series-4)", "var(--series-5)", "var(--series-1)", "var(--series-3)"];
  /** Rows of their own in Health. */
  const ELSEWHERE = new Set(["temp", "cpu", "memory", "fan", "uptime"]);
  // Each chart is drawn at its own size in pixels: a stretched viewBox with
  // non-scaling strokes made WebKit's painter keep memory on every redraw.
  let sizes = $state<Record<string, { w: number; h: number }>>({});
  const sizeOf = (metricId: string) => sizes[metricId] ?? { w: 400, h: 100 };
  function measure(node: HTMLElement, metricId: string) {
    const observer = new ResizeObserver(() => (sizes[metricId] = { w: Math.max(1, node.clientWidth), h: Math.max(1, node.clientHeight) }));
    observer.observe(node);
    return { destroy: () => observer.disconnect() };
  }

  const id = $derived(keyString(record.key));
  const all = $derived(live.metrics.get(id) ?? []);
  const charts = $derived(
    all
      .filter((m) => !isCore(m) && !ELSEWHERE.has(m.id) && !isTextMetric(m))
      .sort((a, b) => Number(b.id === "fps") - Number(a.id === "fps"))
      .slice(0, 6)
      .map((metric, i) => ({ metric, values: live.series(id, metric.id), color: COLORS[i % COLORS.length] }))
      .filter((c) => c.values.length > 0),
  );
  const error = $derived(live.errors.get(id));

  function detail(c: { metric: Metric; values: number[] }): string {
    if (c.metric.detail) return c.metric.detail;
    return `low ${Math.min(...c.values).toFixed(0)} · high ${Math.max(...c.values).toFixed(0)}`;
  }

  function scale(c: { metric: Metric; values: number[] }, H: number) {
    // Voltages float well above 0; the rest are counts and shares from 0.
    const floats = c.metric.unit === "V";
    const spread = Math.max(...c.values) - Math.min(...c.values);
    const low = floats ? Math.min(...c.values) - Math.max(spread, 0.5) : 0;
    const high = floats ? Math.max(...c.values) + Math.max(spread, 0.5) : Math.max(c.metric.max ?? -Infinity, ...c.values, c.metric.warn_above ?? -Infinity);
    const span = high - low || 1;
    return (v: number) => H - ((v - low) / span) * (H - 6) - 3;
  }

  function line(c: { metric: Metric; values: number[] }, W: number, H: number): string {
    const y = scale(c, H);
    // Across the whole width from the first readings (a sliver otherwise).
    const step = W / (Math.max(2, c.values.length) - 1);
    return c.values.map((v, i) => `${i ? "L" : "M"}${(W - (c.values.length - 1 - i) * step).toFixed(1)} ${y(v).toFixed(1)}`).join(" ");
  }
</script>

{#if charts.length === 0 && error}
  <p class="text-[13px] text-warn-fg">{error}</p>
{:else if all.length === 0}
  <div class="charts">
    {#each [0, 1] as i (i)}<Skeleton height={130} />{/each}
  </div>
{:else if charts.length > 0}
  <div class="charts" class:one={charts.length === 1}>
    {#each charts as c (c.metric.id)}
      {@const v = metricValue(c.metric)}
      {@const tone = metricTone(c.metric)}
      {@const { w: W, h: H } = sizeOf(c.metric.id)}
      {@const d = line(c, W, H)}
      <section class="chart" aria-label={c.metric.label}>
        <div class="head">
          <span class="label">{c.metric.label}</span>
          <span class="value" class:hot={tone === "warn" || tone === "err"}>{v.value}</span>
          <span class="unit">{v.unit}</span>
        </div>
        <div class="range" title={detail(c)}>{detail(c)}</div>
        <div class="plot" use:measure={c.metric.id}>
          <svg viewBox="0 0 {W} {H}" aria-hidden="true">
            {#if c.metric.warn_above != null}
              <line x1="0" x2={W} y1={scale(c, H)(c.metric.warn_above)} y2={scale(c, H)(c.metric.warn_above)} class="limit" />
            {/if}
            <path d="{d} L{W} {H} L{W - (c.values.length - 1) * (W / (Math.max(2, c.values.length) - 1))} {H} Z" fill={c.color} class="area" />
            <path {d} stroke={c.color} class="line" />
          </svg>
        </div>
      </section>
    {/each}
  </div>
{/if}

<style>
  .charts {
    display: grid;
    gap: 10px;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    grid-auto-rows: 140px;
  }
  .charts.one {
    grid-template-columns: minmax(0, 1fr);
  }
  /* An odd one out takes the whole last row rather than leaving a hole. */
  .chart:last-child:nth-child(odd) {
    grid-column: 1 / -1;
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
  .plot {
    position: relative;
    flex: 1;
    min-height: 50px;
    width: calc(100% + 24px);
    margin: 6px -12px 0;
  }
  svg {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }
  .line {
    fill: none;
    stroke-width: 1.6;
  }
  .area {
    opacity: 0.14;
  }
  .limit {
    stroke: var(--warn);
    stroke-dasharray: 5 4;
  }
</style>
