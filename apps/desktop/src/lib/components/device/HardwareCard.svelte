<script lang="ts">
  // One hardware device from the board's snapshot, rendered from its data
  // alone: a status pill, its readings (axis trios in a row), and its controls.
  // With live readings: a chart per unit at device rate, whose legend shows
  // the newest values (no tiles then).
  import type { DeviceKey, HardwareDevice } from "#lib/api/client.ts";
  import GlassCard from "#lib/components/common/GlassCard.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import Pill from "#lib/components/common/Pill.svelte";
  import HardwareControls from "./HardwareControls.svelte";
  import HardwareReadingTile from "./HardwareReadingTile.svelte";
  import LiveChart from "./LiveChart.svelte";
  import ImuView from "./ImuView.svelte";
  import { groupReadings, label, pillFor } from "./hardware.ts";
  import type { LiveSeries } from "./live.ts";

  let {
    device,
    seriesOf,
    deviceKey,
    live = null,
    windowS = 10,
    tick = 0,
    compact = false,
  }: {
    device: HardwareDevice;
    seriesOf: (reading: string) => { values: number[]; times: number[] };
    /** The board, when its devices take commands; null shows the controls' values only. */
    deviceKey: DeviceKey | null;
    /** Its live readings, when the board streams them. */
    live?: LiveSeries | null;
    windowS?: number;
    /** Beside other devices (the Monitor page): the charts only, no tiles or controls. */
    compact?: boolean;
    /** Changes while live, to re-read the newest values. */
    tick?: number;
  } = $props();

  const pill = $derived(pillFor(device.status));
  // Live: the newest streamed values (re-read on each tick), no trend tiles.
  const readings = $derived.by(() => {
    void tick;
    // A reading with no value (a fan without a tachometer) isn't shown.
    return live && live.count > 0 ? live.latest() : device.readings.filter((reading) => reading.value !== null);
  });
  // A reading its control row already shows (a power switch's power.on) isn't repeated.
  const entries = $derived(groupReadings(readings.filter((r) => !device.controls.some((c) => c.name === r.name))));
  const noSeries = { values: [], times: [] };
  const seriesFor = (name: string) => (live ? noSeries : seriesOf(name));
  const rate = $derived.by(() => {
    void tick;
    return live ? live.rate() : 0;
  });
  // The ring buffer isn't reactive: the tick says when to look again.
  const streaming = $derived.by(() => {
    void tick;
    return !!live && live.count > 0;
  });
  const subtitle = $derived([device.class, device.model].filter(Boolean).join(" · "));
</script>

<GlassCard title={device.id} {subtitle} class={live && streaming && device.class === "imu" ? "wide" : ""}>
  {#snippet actions()}
    {#if live && rate > 0}<span class="rate text-[11.5px] text-fg-faint">{rate} Hz</span>{/if}
    <Pill tone={pill.tone} label={pill.label} />
  {/snippet}

  {#if live && streaming}
    <!-- Wide: the 3D view beside the charts; narrow: above them. -->
    <div class="live" class:imu={device.class === "imu"}>
      {#if device.class === "imu"}<div class="view"><ImuView series={live} /></div>{/if}
      <div class="charts">
        {#each live.groups() as group (group.unit)}
          <LiveChart series={live} channels={group.channels} unit={group.unit} {windowS} />
        {/each}
      </div>
    </div>
  {/if}

  {#if device.reason}
    <p class="mb-3 flex items-start gap-2 text-[13px] {device.status === 'faulted' ? 'text-err-fg' : 'text-fg-muted'}">
      {#if device.status === "faulted"}<Icon name="alert-circle" size={15} class="mt-0.5 shrink-0" />{/if}
      <span class="min-w-0">{device.reason}</span>
    </p>
  {/if}

  {#if streaming}
    <!-- Every live reading is charted; the legends carry the newest values. -->
  {:else if entries.length === 0}
    {#if device.controls.length === 0 && !device.reason}<p class="text-[13px] text-fg-muted">No readings.</p>{/if}
  {:else}
    <div class="auto-grid" style="--min: 150px; --gap: 8px">
      {#each entries as entry (entry.key)}
        {#if entry.kind === "axes"}
          <div class="axes glass min-w-0 px-3.5 py-2.5">
            <p class="text-[12px] text-fg-faint">{entry.label}</p>
            <div class="mt-1 grid grid-cols-3 gap-3">
              {#each entry.axes as axis (axis.axis)}
                <HardwareReadingTile label={axis.axis} reading={axis.reading} series={seriesFor(axis.reading.name)} bare />
              {/each}
            </div>
          </div>
        {:else}
          <HardwareReadingTile label={label(entry.reading.name)} reading={entry.reading} series={seriesFor(entry.reading.name)} />
        {/if}
      {/each}
    </div>
  {/if}

  {#if !compact}<HardwareControls {device} {deviceKey} />{/if}
</GlassCard>

<style>
  .rate {
    font-variant-numeric: tabular-nums;
  }
  .axes {
    grid-column: 1 / -1;
  }
  .live {
    display: grid;
    gap: 12px;
  }
  .charts {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
  }
  @container (min-width: 880px) {
    .live.imu {
      grid-template-columns: minmax(300px, 400px) minmax(0, 1fr);
      align-items: start;
    }
  }
</style>
