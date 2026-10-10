<script lang="ts">
  // A live chart of some of a device's channels (one unit), drawn on a
  // canvas at the display's frame rate from the device's ring buffer: the
  // last `windowS` seconds, newest at the right. Hovering pauses it and shows
  // the nearest sample's values and when it was read.
  import { onMount } from "svelte";
  import { clockText, type LiveSeries } from "./live.ts";
  import { formatValue, label } from "./hardware.ts";

  let {
    series,
    channels,
    unit,
    windowS,
  }: { series: LiveSeries; channels: number[]; unit: string; windowS: number } = $props();

  const HEIGHT = 120;
  const PALETTE = ["--info", "--ok", "--warn", "--accent", "--err", "--fg-muted"];

  let canvas = $state<HTMLCanvasElement>();
  let width = $state(300);
  let colors = $state<string[]>([]);
  let grid = "rgba(128,128,128,0.18)";
  let text = "rgba(128,128,128,0.8)";
  /** While hovered: where, and the frozen time range drawn. */
  let hover = $state<{ x: number; newest: number; left: number; top: number } | null>(null);

  onMount(() => {
    const style = getComputedStyle(document.documentElement);
    colors = PALETTE.map((name) => style.getPropertyValue(name).trim() || "#888");
    grid = style.getPropertyValue("--glass-strong").trim() || grid;
    text = style.getPropertyValue("--fg-faint").trim() || text;
    let frame = 0;
    // Repaint only when something changed: a new sample, the size, the window.
    let painted = "";
    const draw = () => {
      const now = `${series.newestUs()} ${width} ${windowS}`;
      if (!hover && now !== painted) {
        painted = now;
        paint(series.newestUs());
      }
      frame = requestAnimationFrame(draw);
    };
    frame = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(frame);
  });

  /** The samples in view and the value range they span. */
  function view(newest: number) {
    const from = newest - windowS * 1_000_000;
    const first = series.firstFrom(from);
    let low = Infinity;
    let high = -Infinity;
    for (let i = first; i < series.count; i++) {
      const at = series.index(i);
      for (const c of channels) {
        const value = series.v[c][at];
        if (Number.isNaN(value)) continue;
        if (value < low) low = value;
        if (value > high) high = value;
      }
    }
    if (!Number.isFinite(low)) {
      low = 0;
      high = 1;
    }
    const pad = (high - low || Math.abs(high) || 1) * 0.12;
    return { from, first, low: low - pad, high: high + pad };
  }

  function paint(newest: number, crosshair?: number) {
    const ctx = canvas?.getContext("2d");
    if (!ctx || !canvas) return;
    const dpr = window.devicePixelRatio || 1;
    if (canvas.width !== Math.round(width * dpr)) {
      canvas.width = Math.round(width * dpr);
      canvas.height = Math.round(HEIGHT * dpr);
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, HEIGHT);
    const { from, first, low, high } = view(newest);
    const span = windowS * 1_000_000;
    const x = (t: number) => ((t - from) / span) * width;
    const y = (value: number) => HEIGHT - 4 - ((value - low) / (high - low)) * (HEIGHT - 8);
    // Three gridlines with their values.
    ctx.font = "10px ui-sans-serif, system-ui";
    ctx.fillStyle = text;
    ctx.strokeStyle = grid;
    ctx.lineWidth = 1;
    for (const f of [0.25, 0.5, 0.75]) {
      const value = low + (high - low) * f;
      const gy = Math.round(y(value)) + 0.5;
      ctx.beginPath();
      ctx.moveTo(0, gy);
      ctx.lineTo(width, gy);
      ctx.stroke();
      ctx.fillText(formatValue(value), 4, gy - 3);
    }
    ctx.lineWidth = 1.5;
    ctx.lineJoin = "round";
    channels.forEach((c, n) => {
      ctx.strokeStyle = colors[n % colors.length] ?? "#888";
      ctx.beginPath();
      let open = false;
      for (let i = first; i < series.count; i++) {
        const at = series.index(i);
        const value = series.v[c][at];
        if (Number.isNaN(value)) {
          open = false;
          continue;
        }
        const px = x(series.t[at]);
        const py = y(value);
        if (open) ctx.lineTo(px, py);
        else ctx.moveTo(px, py);
        open = true;
      }
      ctx.stroke();
    });
    if (crosshair !== undefined) {
      ctx.strokeStyle = text;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(crosshair + 0.5, 0);
      ctx.lineTo(crosshair + 0.5, HEIGHT);
      ctx.stroke();
    }
  }

  /** The sample nearest the hovered x. */
  const picked = $derived.by(() => {
    if (!hover || series.count === 0) return null;
    const t = hover.newest - windowS * 1_000_000 + (hover.x / width) * windowS * 1_000_000;
    let i = series.firstFrom(t);
    if (i >= series.count) i = series.count - 1;
    if (i > 0 && Math.abs(series.t[series.index(i - 1)] - t) < Math.abs(series.t[series.index(i)] - t)) i -= 1;
    const at = series.index(i);
    return {
      when: clockText(series.wallMs(series.t[at])),
      values: channels.map((c) => ({ name: series.channels[c].name, value: series.v[c][at] })),
    };
  });

  function move(event: PointerEvent) {
    if (!canvas) return;
    const box = canvas.getBoundingClientRect();
    const x = Math.max(0, Math.min(box.width, event.clientX - box.left));
    const newest = hover?.newest ?? series.newestUs();
    hover = { x, newest, left: box.left + x, top: box.top };
    paint(newest, x);
  }
</script>

<div class="chart" bind:clientWidth={width}>
  <canvas
    bind:this={canvas}
    style="width: 100%; height: {HEIGHT}px"
    onpointermove={move}
    onpointerleave={() => {
      hover = null;
      paint(series.newestUs());
    }}
    aria-label="{series.id}: {channels.map((c) => label(series.channels[c].name)).join(', ')}, live"
  ></canvas>
  <div class="legend">
    {#each channels as c, n (c)}
      <span class="item"><span class="swatch" style="background: {colors[n % colors.length]}"></span>{label(series.channels[c].name)}</span>
    {/each}
    {#if unit}<span class="unit">{unit}</span>{/if}
  </div>
  {#if hover && picked}
    <div class="tip glass-layer" style="left: {hover.left}px; top: {hover.top}px" role="tooltip">
      <p class="when">{picked.when}</p>
      {#each picked.values as value, n (value.name)}
        <p class="row">
          <span class="swatch" style="background: {colors[n % colors.length]}"></span>
          <span class="name">{label(value.name)}</span>
          <span class="value">{Number.isNaN(value.value) ? "–" : value.value.toLocaleString(undefined, { maximumFractionDigits: 4 })}{unit ? ` ${unit}` : ""}</span>
        </p>
      {/each}
    </div>
  {/if}
</div>

<style>
  .chart {
    position: relative;
    min-width: 0;
  }
  canvas {
    display: block;
    cursor: crosshair;
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    margin-top: 4px;
    font-size: 11.5px;
    color: var(--fg-muted);
  }
  .item {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .swatch {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    flex-shrink: 0;
  }
  .unit {
    margin-left: auto;
    color: var(--fg-faint);
  }
  .tip {
    position: fixed;
    z-index: 60;
    transform: translate(-50%, calc(-100% - 8px));
    padding: 6px 9px;
    border-radius: 8px;
    pointer-events: none;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .when {
    font-size: 11px;
    color: var(--fg-faint);
    margin-bottom: 3px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
  }
  .name {
    color: var(--fg-muted);
    min-width: 7em;
  }
  .value {
    margin-left: auto;
    font-weight: 600;
    color: var(--fg);
  }
</style>
