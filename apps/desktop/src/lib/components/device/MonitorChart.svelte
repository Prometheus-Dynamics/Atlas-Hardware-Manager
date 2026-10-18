<script lang="ts">
  // One reading across the Monitor's devices: a line each over the chosen
  // window, newest at the right, the warning level dashed, and each
  // device's current value under it in its colour. It fills the cell it is
  // given (the Monitor's grid fits the window).
  import { keyString, type DeviceRecord } from "#lib/api/client.ts";
  import { deviceName } from "#lib/format.ts";
  import { metricValue } from "#lib/metrics.ts";
  import { clock } from "#lib/stores/clock.svelte.ts";
  import { live } from "#lib/stores/live.svelte.ts";

  let {
    metric,
    title,
    unit,
    devices,
    windowMs,
  }: {
    metric: string;
    title: string;
    unit: string;
    /** The devices to draw, each with its colour. */
    devices: { record: DeviceRecord; color: string }[];
    windowMs: number;
  } = $props();

  // Drawn at the plot's own size in pixels: a stretched viewBox with
  // non-scaling strokes made WebKit's painter keep memory on every redraw.
  let W = $state(400);
  let H = $state(200);

  const since = $derived(clock.now - windowMs);
  const lines = $derived(
    devices
      .map(({ record, color }) => {
        const id = keyString(record.key);
        return { id, record, color, now: live.metric(id, metric), points: live.window(id, metric, since) };
      })
      .filter((l) => l.now && l.points.length > 0),
  );
  const warn = $derived(lines.find((l) => l.now?.warn_above != null)?.now?.warn_above ?? null);
  const top = $derived(lines.find((l) => l.now?.max != null)?.now?.max ?? null);

  // Shares and counts from 0 (to their maximum when known); temperatures
  // and voltages float, with room around them.
  const range = $derived.by(() => {
    const all = lines.flatMap((l) => l.points.map((p) => p.v));
    if (all.length === 0) return { low: 0, high: 1 };
    const fromZero = unit === "%" || unit === "fps" || unit === "rpm" || unit === "A" || unit === "W";
    let low = fromZero ? 0 : Math.min(...all);
    let high = Math.max(...all, warn ?? -Infinity, fromZero && top != null ? top : -Infinity);
    if (!fromZero) {
      const pad = Math.max((high - low) * 0.2, Math.abs(high) * 0.02, 0.5);
      low -= pad;
      high += pad;
    }
    return { low, high: high === low ? low + 1 : high };
  });
  // Until the window has filled, the chart spans what there is, so the
  // first readings aren't a sliver at the right.
  const start = $derived(Math.max(since, Math.min(clock.now - 10_000, ...lines.map((l) => l.points[0].t))));
  const span = $derived(Math.max(1, clock.now - start));
  const y = (v: number) => H - ((v - range.low) / (range.high - range.low)) * H;
  const x = (t: number) => ((t - start) / span) * W;
  const ago = (ms: number) => (span < 120_000 ? `-${Math.round(ms / 1000)} s` : `-${Math.round(ms / 60_000)} min`);
  const tick = (v: number) => {
    const span = range.high - range.low;
    return span >= 20 ? Math.round(v).toString() : span >= 2 ? v.toFixed(1) : v.toFixed(2);
  };
  const path = (points: { t: number; v: number }[]) =>
    points.map((p, i) => `${i ? "L" : "M"}${x(p.t).toFixed(1)} ${y(p.v).toFixed(1)}`).join(" ");
  const hot = (l: (typeof lines)[number]) => warn != null && !!l.now && l.now.value > warn;
  const minutes = $derived(Math.max(1, Math.round(windowMs / 60_000)));
</script>

<section class="chart" aria-label={title}>
  <div class="head">
    <h2>{title}</h2>
    <span class="unit">{unit}</span>
    <span class="flex-1"></span>
    {#if lines.some(hot)}
      <span class="note hot">{lines.filter(hot).map((l) => deviceName(l.record)).join(", ")} above {warn}</span>
    {:else if warn != null}
      <span class="note">warns above {warn}</span>
    {/if}
  </div>
  <div class="plot">
    <div class="axis" aria-hidden="true">
      <span>{tick(range.high)}</span><span>{tick((range.high + range.low) / 2)}</span><span>{tick(range.low)}</span>
    </div>
    <div class="area" bind:clientWidth={W} bind:clientHeight={H}>
      <svg viewBox="0 0 {W} {H}" role="img" aria-label="{title} for each device over {minutes} minutes">
        <line x1="0" x2={W} y1="0.5" y2="0.5" class="grid" />
        <line x1="0" x2={W} y1={H / 2} y2={H / 2} class="grid" />
        <line x1="0" x2={W} y1={H - 0.5} y2={H - 0.5} class="grid" />
        {#if warn != null && warn < range.high}<line x1="0" x2={W} y1={y(warn)} y2={y(warn)} class="limit" />{/if}
        {#each lines as l (l.id)}
          <path d={path(l.points)} style:stroke={l.color} class="line" class:bold={hot(l)} />
        {/each}
      </svg>
    </div>
  </div>
  <div class="times" aria-hidden="true">
    <span>{ago(span)}</span><span>{ago((span * 2) / 3)}</span><span>{ago(span / 3)}</span><span>now</span>
  </div>
  <ul class="legend">
    {#each lines as l (l.id)}
      {@const v = l.now ? metricValue(l.now) : null}
      <li title={deviceName(l.record)}>
        <span class="swatch" style:background={l.color}></span><span class="name">{deviceName(l.record)}</span>
        <span class="value" class:hot={hot(l)}>{v?.value}</span>
      </li>
    {/each}
  </ul>
</section>

<style>
  .chart {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
    min-height: 0;
    padding: 10px 12px;
    background: var(--glass);
    border: 1px solid var(--glass-border);
    border-radius: var(--r-card);
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  h2 {
    font-size: 13px;
    font-weight: 600;
  }
  .unit,
  .note {
    font-size: 11.5px;
    color: var(--fg-muted);
    white-space: nowrap;
  }
  .note {
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--fg-faint);
  }
  .note.hot {
    color: var(--warn-fg);
  }
  .plot {
    flex: 1;
    min-height: 60px;
    display: flex;
    gap: 8px;
  }
  .axis {
    width: 30px;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    font-size: 10.5px;
    color: var(--fg-faint);
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .area {
    position: relative;
    flex: 1;
    min-width: 0;
  }
  svg {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    overflow: visible;
  }
  .grid {
    stroke: var(--hairline);
  }
  .limit {
    stroke: var(--warn);
    stroke-dasharray: 5 4;
  }
  .line {
    fill: none;
    stroke-width: 1.4;
    stroke-linejoin: round;
  }
  .line.bold {
    stroke-width: 2.2;
  }
  .times {
    display: flex;
    justify-content: space-between;
    padding-left: 38px;
    font-size: 10.5px;
    color: var(--fg-faint);
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 2px 12px;
    padding-left: 38px;
    font-size: 11.5px;
    color: var(--fg-muted);
  }
  .legend li {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
  }
  .swatch {
    width: 8px;
    height: 3px;
    border-radius: 1px;
    flex-shrink: 0;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 110px;
  }
  .value {
    color: var(--fg);
    font-variant-numeric: tabular-nums;
  }
  .value.hot {
    color: var(--warn-fg);
  }
</style>
