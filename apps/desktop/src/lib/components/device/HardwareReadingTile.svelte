<script lang="ts">
  // One reading: its label, value and unit, and a trend line of the values
  // seen while the tab is open. `bare` drops the tile for use inside a row.
  import type { HardwareReading } from "#lib/api/client.ts";
  import Sparkline from "#lib/components/common/Sparkline.svelte";
  import { formatValue } from "./hardware.ts";

  let {
    label,
    reading,
    series = [],
    bare = false,
  }: { label: string; reading: HardwareReading; series?: number[]; bare?: boolean } = $props();

  const shown = $derived(formatValue(reading.value));
</script>

<div class="min-w-0" class:glass={!bare} class:tile={!bare} class:bare>
  <p class="truncate text-[12px] text-fg-faint" title={label}>{label}</p>
  <p class="value">
    {shown}{#if reading.value !== null && reading.unit}<span class="unit">{reading.unit}</span>{/if}
  </p>
  <div class="mt-1.5"><Sparkline values={series} height={22} /></div>
</div>

<style>
  .tile {
    padding: 12px 14px;
  }
  .bare {
    min-width: 0;
  }
  .value {
    font-size: 18px;
    font-weight: 600;
    letter-spacing: -0.02em;
    color: var(--fg);
    font-variant-numeric: tabular-nums;
    line-height: 1.2;
  }
  .bare .value {
    font-size: 15px;
  }
  .unit {
    margin-left: 3px;
    font-size: 12px;
    font-weight: 500;
    color: var(--fg-faint);
  }
</style>
