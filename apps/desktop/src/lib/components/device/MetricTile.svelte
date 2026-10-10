<script lang="ts">
  // One live reading: a gauge when the metric has a range, its value, the
  // raw numbers behind it, and a trend line of the last couple of minutes.
  // With cores, a switch shows a bar per core in the trend's place, so the
  // tile keeps its size.
  import type { Metric } from "#lib/api/client.ts";
  import AnimatedNumber from "#lib/components/common/AnimatedNumber.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import IconTile from "#lib/components/common/IconTile.svelte";
  import Ring from "#lib/components/common/Ring.svelte";
  import Sparkline from "#lib/components/common/Sparkline.svelte";
  import { coreView } from "#lib/stores/coreView.svelte.ts";
  import {
    frameTime,
    isTextMetric,
    metricDecimals,
    metricDetail,
    metricFraction,
    metricIcon,
    metricTone,
    metricValue,
    toneColor,
  } from "#lib/metrics.ts";

  let {
    metric,
    series = [],
    times = [],
    cores = [],
  }: { metric: Metric; series?: number[]; times?: number[]; cores?: Metric[] } = $props();

  /** A past reading exactly (up to 2 decimals), for the trend's hover label. */
  const format = (value: number) => {
    if (isTextMetric(metric)) return metricValue({ ...metric, value }).value;
    const text = value.toLocaleString(undefined, { maximumFractionDigits: 2 });
    const shown = metric.unit ? `${text} ${metric.unit}` : text;
    return metric.id === "fps" && value > 0 ? `${shown} · ${frameTime(value)}` : shown;
  };

  const tone = $derived(metricTone(metric));
  const color = $derived(toneColor(tone));
  const fraction = $derived(metricFraction(metric));
  const shown = $derived(metricValue(metric));
  const detail = $derived(metricDetail(metric));
  const trend = $derived(metric.id !== "uptime" && series.length > 1);
</script>

<div class="tile glass" class:warn={tone === "warn"} class:err={tone === "err"}>
  <div class="flex items-center gap-3">
    {#if fraction !== null}
      <Ring {fraction} size={44} stroke={4.5} {color}>
        <Icon name={metricIcon(metric)} size={17} stroke={1.7} />
      </Ring>
    {:else}
      <IconTile icon={metricIcon(metric)} size={44} tone="muted" />
    {/if}
    <div class="min-w-0 flex-1">
      <p class="truncate text-[12px] text-fg-muted">{metric.label}</p>
      <p class="value">
        {#if isTextMetric(metric)}{shown.value}{:else}<AnimatedNumber value={metric.value} decimals={metricDecimals(metric)} />{/if}{#if shown.unit}<span class="unit">{shown.unit}</span>{/if}
      </p>
    </div>
    {#if cores.length > 0}
      <button
        type="button"
        class="view-switch"
        onclick={() => coreView.toggle()}
        title={coreView.perCore ? "Show the trend" : "Show each core"}
        aria-label={coreView.perCore ? "Show the CPU trend" : "Show each CPU core"}
        aria-pressed={coreView.perCore}
      >
        <Icon name={coreView.perCore ? "chart-line" : "layout-grid"} size={15} />
      </button>
    {/if}
  </div>
  {#if detail}
    <p class="detail mt-1.5 truncate" title={detail}>{detail}</p>
  {/if}
  {#if cores.length > 0 && coreView.perCore}
    <div class="cores mt-auto pt-2" role="list" aria-label="Each core">
      {#each cores as core (core.id)}
        <div class="bar" role="listitem" title="{core.label}: {Math.round(core.value)} %" aria-label="{core.label}: {Math.round(core.value)} %">
          <span style="height: {Math.min(100, Math.max(0, core.value))}%; background: {toneColor(metricTone(core))}"></span>
        </div>
      {/each}
    </div>
  {:else if trend}
    <div class="mt-auto pt-2"><Sparkline values={series} {times} {format} {color} height={26} /></div>
  {/if}
</div>

<style>
  .tile {
    /* Every tile in a row as tall as the tallest. */
    height: 100%;
    display: flex;
    flex-direction: column;
    padding: 12px 14px;
    transition:
      border-color var(--t-med),
      background var(--t-med);
  }
  .tile.warn {
    border-color: color-mix(in srgb, var(--warn) 45%, transparent);
    background: color-mix(in srgb, var(--warn) 6%, var(--glass));
  }
  .tile.err {
    border-color: color-mix(in srgb, var(--err) 50%, transparent);
    background: color-mix(in srgb, var(--err) 7%, var(--glass));
  }
  .value {
    font-size: 20px;
    font-weight: 600;
    letter-spacing: -0.02em;
    color: var(--fg);
    font-variant-numeric: tabular-nums;
    line-height: 1.2;
  }
  .detail {
    font-size: 11.5px;
    color: var(--fg-faint);
    font-variant-numeric: tabular-nums;
  }
  .cores {
    display: flex;
    gap: 3px;
    /* The sparkline's box (26px line, its inline-box line height), so
       switching views doesn't move anything. */
    height: 33px;
  }
  .bar {
    position: relative;
    flex: 1;
    border-radius: 3px;
    background: var(--glass-strong);
    overflow: hidden;
  }
  .bar span {
    position: absolute;
    inset: auto 0 0 0;
    border-radius: 3px;
    transition: height var(--t-data) var(--ease-out);
  }
  .view-switch {
    align-self: flex-start;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: var(--r-md);
    color: var(--fg-faint);
    transition:
      color var(--t-fast),
      background var(--t-fast);
  }
  .view-switch:hover,
  .view-switch[aria-pressed="true"] {
    color: var(--fg);
    background: var(--glass-strong);
  }
  .unit {
    margin-left: 3px;
    font-size: 12px;
    font-weight: 500;
    color: var(--fg-faint);
  }
</style>
