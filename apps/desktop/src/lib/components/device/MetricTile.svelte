<script lang="ts">
  // One live reading: a gauge when the metric has a range, its value, the
  // raw numbers behind it, a bar per core for the CPU, and a trend line of
  // the last couple of minutes.
  import type { Metric } from "#lib/api/client.ts";
  import AnimatedNumber from "#lib/components/common/AnimatedNumber.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import IconTile from "#lib/components/common/IconTile.svelte";
  import Ring from "#lib/components/common/Ring.svelte";
  import Sparkline from "#lib/components/common/Sparkline.svelte";
  import { isTextMetric, metricDecimals, metricFraction, metricIcon, metricTone, metricValue, toneColor } from "#lib/metrics.ts";

  let { metric, series = [], cores = [] }: { metric: Metric; series?: number[]; cores?: Metric[] } = $props();

  const tone = $derived(metricTone(metric));
  const color = $derived(toneColor(tone));
  const fraction = $derived(metricFraction(metric));
  const shown = $derived(metricValue(metric));
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
    <div class="min-w-0">
      <p class="truncate text-[12px] text-fg-muted">{metric.label}</p>
      <p class="value">
        {#if isTextMetric(metric)}{shown.value}{:else}<AnimatedNumber value={metric.value} decimals={metricDecimals(metric)} />{/if}{#if shown.unit}<span class="unit">{shown.unit}</span>{/if}
      </p>
    </div>
  </div>
  {#if metric.detail}
    <p class="detail mt-1.5 truncate" title={metric.detail}>{metric.detail}</p>
  {/if}
  {#if cores.length > 0}
    <div class="cores mt-2" role="list" aria-label="Each core">
      {#each cores as core (core.id)}
        {@const coreTone = metricTone(core)}
        <div class="core" role="listitem" title="{core.label}: {Math.round(core.value)} %">
          <div class="bar"><span style="height: {Math.min(100, Math.max(0, core.value))}%; background: {toneColor(coreTone)}"></span></div>
          <span class="core-value">{Math.round(core.value)}</span>
        </div>
      {/each}
    </div>
  {/if}
  {#if trend}
    <div class="mt-2"><Sparkline values={series} {color} height={26} /></div>
  {/if}
</div>

<style>
  .tile {
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
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(18px, 1fr));
    gap: 4px;
  }
  .core {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
  }
  .bar {
    position: relative;
    width: 100%;
    height: 22px;
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
  .core-value {
    font-size: 10px;
    color: var(--fg-faint);
    font-variant-numeric: tabular-nums;
  }
  .unit {
    margin-left: 3px;
    font-size: 12px;
    font-weight: 500;
    color: var(--fg-faint);
  }
</style>
