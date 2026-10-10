<script lang="ts">
  // A live chart of some of a device's channels (one unit), drawn on a
  // canvas at the display's frame rate from the device's ring buffer, newest
  // at the right. The time axis grows with the data until it spans
  // `windowS` seconds, then scrolls. The value range follows the data,
  // widening at once and narrowing gently, so lines don't jump. Hovering
  // shows the values under the pointer and keeps drawing; a click pauses
  // (and resumes). The legend carries each channel's newest value.
  import { onMount } from "svelte";
  import { clockText, type LiveSeries } from "./live.ts";
  import { formatValue, label } from "./hardware.ts";
  import { portal } from "#lib/ui/portal.ts";

  let {
    series,
    channels,
    unit,
    windowS,
  }: { series: LiveSeries; channels: number[]; unit: string; windowS: number } = $props();

  // One line needs less height than several crossing.
  const HEIGHT = $derived(channels.length > 1 ? 120 : 84);
  const PALETTE = ["--info", "--ok", "--warn", "--accent", "--err", "--fg-muted"];

  let canvas = $state<HTMLCanvasElement>();
  let width = $state(300);
  let colors = $state<string[]>([]);
  let grid = "rgba(128,128,128,0.18)";
  let text = "rgba(128,128,128,0.8)";
  /** While hovered: where (x on the canvas, and on screen). */
  let hover = $state<{ x: number; left: number; top: number } | null>(null);
  /** Paused: the newest time drawn, frozen until a click resumes. */
  let paused = $state<number | null>(null);
  /** The newest time and span (µs) of the last paint, for the hover lookup. */
  let shown = $state({ newest: 0, span: 1 });
  /** The value range drawn, eased toward the data's. */
  let range: { low: number; high: number } | null = null;
  /** Each channel's newest value, for the legend (a few times a second). */
  let current = $state<(number | null)[]>([]);
  let currentAt = 0;
  /** The shortest time axis (s), so the first samples don't stretch across. */
  const MIN_SPAN_S = 1;

  onMount(() => {
    const style = getComputedStyle(document.documentElement);
    colors = PALETTE.map((name) => style.getPropertyValue(name).trim() || "#888");
    grid = style.getPropertyValue("--glass-strong").trim() || grid;
    text = style.getPropertyValue("--fg-faint").trim() || text;
    let frame = 0;
    // Repaint when something changed: a new sample, the size, the window,
    // the pointer, or while the value range is still easing.
    let painted = "";
    const draw = () => {
      const newest = paused ?? series.newestUs();
      const ms = performance.now();
      if (paused === null && series.count > 0 && ms - currentAt > 250) {
        currentAt = ms;
        const at = series.index(series.count - 1);
        current = channels.map((c) => (Number.isNaN(series.v[c][at]) ? null : series.v[c][at]));
      }
      const now = `${newest} ${width} ${windowS} ${hover?.x ?? -1}`;
      if (now !== painted || easing) {
        painted = now;
        paint(newest, hover?.x);
      }
      frame = requestAnimationFrame(draw);
    };
    frame = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(frame);
  });

  let easing = false;

  /** The samples in view, the time span drawn and the value range. */
  function view(newest: number) {
    const oldest = series.count ? series.t[series.index(0)] : newest;
    // Grow with the data until the window is full.
    const span = Math.max(MIN_SPAN_S * 1_000_000, Math.min(windowS * 1_000_000, newest - oldest));
    const from = newest - span;
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
    const want = { low: low - pad, high: high + pad };
    // Widen at once (nothing drawn off the chart); narrow a little each frame.
    if (!range) range = want;
    const next = {
      low: want.low < range.low ? want.low : range.low + (want.low - range.low) * 0.08,
      high: want.high > range.high ? want.high : range.high + (want.high - range.high) * 0.08,
    };
    easing = Math.abs(next.low - want.low) + Math.abs(next.high - want.high) > (want.high - want.low) * 0.005;
    range = easing ? next : want;
    return { from, first, span, low: range.low, high: range.high };
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
    const { from, first, span, low, high } = view(newest);
    shown = { newest, span };
    const x = (t: number) => ((t - from) / span) * width;
    const y = (value: number) => HEIGHT - 14 - ((value - low) / (high - low)) * (HEIGHT - 18);
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
    // Time: how long ago, at a round step that fits.
    const spanS = span / 1_000_000;
    const step = [0.5, 1, 2, 5, 10, 15, 30].find((s) => spanS / s <= 6) ?? 60;
    ctx.textAlign = "center";
    for (let ago = step; ago < spanS; ago += step) {
      const tx = width - (ago / spanS) * width;
      ctx.fillText(`-${ago % 1 ? ago.toFixed(1) : ago}s`, tx, HEIGHT - 2);
    }
    ctx.textAlign = "start";
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

  /** The sample nearest the hovered x, in what is drawn now. */
  const picked = $derived.by(() => {
    if (!hover || series.count === 0) return null;
    const t = shown.newest - shown.span + (hover.x / width) * shown.span;
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
    hover = { x, left: box.left + x, top: box.top };
  }
</script>

<div class="chart" bind:clientWidth={width}>
  <canvas
    bind:this={canvas}
    style="width: 100%; height: {HEIGHT}px"
    onpointermove={move}
    onpointerleave={() => (hover = null)}
    onclick={() => (paused = paused === null ? series.newestUs() : null)}
    title={paused === null ? "Click to pause" : "Paused: click to resume"}
    aria-label="{series.id}: {channels.map((c) => label(series.channels[c].name)).join(', ')}, live"
  ></canvas>
  <div class="legend">
    {#each channels as c, n (c)}
      <span class="item"><span class="swatch" style="background: {colors[n % colors.length]}"></span>{label(series.channels[c].name)}{#if current[n] !== undefined}<span class="now">{formatValue(current[n])}</span>{/if}</span>
    {/each}
    {#if paused !== null}<span class="paused">Paused</span>{/if}
    {#if unit}<span class="unit">{unit}</span>{/if}
  </div>
  {#if hover && picked}
    <div class="tip glass-layer" style="left: {hover.left}px; top: {hover.top}px" role="tooltip" use:portal>
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
  .now {
    min-width: 4.5ch;
    font-weight: 600;
    color: var(--fg);
    font-variant-numeric: tabular-nums;
  }
  .swatch {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    flex-shrink: 0;
  }
  .paused {
    margin-left: auto;
    color: var(--warn-fg);
    font-weight: 600;
  }
  .paused + .unit {
    margin-left: 8px;
  }
  .unit {
    margin-left: auto;
    color: var(--fg-faint);
  }
  .tip {
    position: fixed;
    /* Above everything: it lives on <body> (portal.ts). */
    z-index: 80;
    transform: translate(-50%, calc(-100% - 8px));
    padding: 6px 9px;
    border-radius: var(--r-md);
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
