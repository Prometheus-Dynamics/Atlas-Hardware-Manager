<script lang="ts">
  // A summary number with a label, a detail line, and an optional trend.
  import type { Snippet } from "svelte";
  import AnimatedNumber from "#lib/components/common/AnimatedNumber.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import Sparkline from "#lib/components/common/Sparkline.svelte";
  import type { IconName } from "#lib/ui/icons.ts";

  let {
    icon,
    label,
    value,
    decimals = 0,
    unit = "",
    sub,
    series = [],
    color = "var(--info)",
    onclick,
    extra,
  }: {
    icon: IconName;
    label: string;
    /** Numbers glide between readings; text is shown as is. */
    value: string | number;
    decimals?: number;
    unit?: string;
    sub?: string;
    series?: number[];
    color?: string;
    onclick?: () => void;
    extra?: Snippet;
  } = $props();
</script>

<svelte:element
  this={onclick ? "button" : "div"}
  type={onclick ? "button" : undefined}
  class="tile glass"
  class:clickable={!!onclick}
  style="--tone: {color}"
  {onclick}
  role={onclick ? undefined : "group"}
  aria-label={onclick ? undefined : label}
>
  <div class="flex items-center gap-2 text-[12px] font-medium text-fg-muted">
    <span class="ico"><Icon name={icon} size={14} stroke={1.9} /></span>
    {label}
  </div>
  <p class="value">{#if typeof value === "number"}<AnimatedNumber {value} {decimals} />{:else}{value}{/if}{#if unit}<span class="unit">{unit}</span>{/if}</p>
  {#if sub}<p class="truncate text-[12px] text-fg-faint" title={sub}>{sub}</p>{/if}
  {#if extra}{@render extra()}{/if}
  {#if series.length > 1}
    <div class="mt-auto pt-2"><Sparkline values={series} {color} height={26} /></div>
  {/if}
</svelte:element>

<style>
  .tile {
    position: relative;
    width: 100%;
    min-width: 0;
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 2px;
    /* Taller tiles when the page has height to spare (cqh: the page body). */
    min-height: clamp(104px, 12cqh, 168px);
    padding: 12px 16px 10px;
    text-align: left;
    overflow: hidden;
    transition:
      background var(--t-fast),
      transform var(--t-fast),
      border-color var(--t-fast);
  }
  .clickable:hover {
    background: var(--glass-hover);
    border-color: var(--glass-border-strong);
  }
  .clickable:active {
    transform: scale(0.99);
  }
  .ico {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border-radius: var(--r-md);
    color: var(--tone);
    background: color-mix(in srgb, var(--tone) 14%, transparent);
  }
  .value {
    margin-top: 4px;
    font-size: clamp(22px, 2.6cqh, 32px);
    font-weight: 600;
    letter-spacing: -0.03em;
    line-height: 1.1;
    color: var(--fg);
    font-variant-numeric: tabular-nums;
  }
  .unit {
    margin-left: 4px;
    font-size: 14px;
    font-weight: 500;
    color: var(--fg-faint);
    letter-spacing: 0;
  }
</style>
