<script lang="ts">
  // One hardware device from the board's snapshot, rendered from its data
  // alone: a status pill, its readings (axis trios in a row), and its controls.
  import type { HardwareDevice } from "#lib/api/client.ts";
  import GlassCard from "#lib/components/common/GlassCard.svelte";
  import Pill from "#lib/components/common/Pill.svelte";
  import HardwareReadingTile from "./HardwareReadingTile.svelte";
  import { groupReadings, label, pillFor } from "./hardware.ts";

  let { device, seriesOf }: { device: HardwareDevice; seriesOf: (reading: string) => number[] } = $props();

  const pill = $derived(pillFor(device.status));
  const entries = $derived(groupReadings(device.readings));
  const subtitle = $derived([device.class, device.model].filter(Boolean).join(" · "));
</script>

<GlassCard title={device.id} {subtitle}>
  {#snippet actions()}
    <Pill tone={pill.tone} label={pill.label} />
  {/snippet}

  {#if entries.length === 0}
    <p class="text-[13px] text-fg-muted">No readings.</p>
  {:else}
    <div class="auto-grid" style="--min: 150px; --gap: 8px">
      {#each entries as entry (entry.key)}
        {#if entry.kind === "axes"}
          <div class="axes glass min-w-0 px-3.5 py-2.5">
            <p class="text-[12px] text-fg-faint">{entry.label}</p>
            <div class="mt-1 grid grid-cols-3 gap-3">
              {#each entry.axes as axis (axis.axis)}
                <HardwareReadingTile label={axis.axis} reading={axis.reading} series={seriesOf(axis.reading.name)} bare />
              {/each}
            </div>
          </div>
        {:else}
          <HardwareReadingTile label={label(entry.reading.name)} reading={entry.reading} series={seriesOf(entry.reading.name)} />
        {/if}
      {/each}
    </div>
  {/if}

  {#if device.controls.length > 0}
    <p class="mt-3 text-[12px] text-fg-faint">Controls: {device.controls.join(", ")}</p>
  {/if}
</GlassCard>

<style>
  .axes {
    grid-column: 1 / -1;
  }
</style>
