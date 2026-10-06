<script lang="ts">
  // The robot at a glance. Each tile appears only when some device reports
  // the reading behind it, so a fleet without telemetry still looks whole.
  import { goto } from "$app/navigation";
  import { keyString, type DeviceRecord } from "#lib/api/client.ts";
  import StatusDot from "#lib/components/common/StatusDot.svelte";
  import { deviceName } from "#lib/format.ts";
  import { metricTone, metricValue, toneColor } from "#lib/metrics.ts";
  import { insights } from "#lib/stores/insights.svelte.ts";
  import Skeleton from "#lib/components/common/Skeleton.svelte";
  import { hasTelemetry, live, watchLive } from "#lib/stores/live.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import { rise, stagger } from "#lib/ui/motion.ts";
  import StatTile from "./StatTile.svelte";

  let { records }: { records: DeviceRecord[] } = $props();

  $effect(() => watchLive(records));

  // Readings arrive device by device; hold the row until all have answered
  // (or a moment has passed) so the tiles land together instead of popping
  // in one at a time.
  let waited = $state(false);
  $effect(() => {
    const timer = setTimeout(() => (waited = true), 2500);
    return () => clearTimeout(timer);
  });
  const ready = $derived(waited || live.settled(records));
  const placeholders = $derived(records.some(hasTelemetry) ? 5 : 2);

  const online = $derived(records.filter((r) => r.presence === "online"));

  /** Every reading of one metric across the shown devices. */
  function readings(metricId: string) {
    return records
      .map((record) => ({ record, id: keyString(record.key), metric: live.metric(keyString(record.key), metricId) }))
      .filter((r): r is { record: DeviceRecord; id: string; metric: NonNullable<typeof r.metric> } => !!r.metric && r.record.presence === "online");
  }

  /** Point-wise combination of several series, aligned at their newest end. */
  function combine(series: number[][], fold: (values: number[]) => number): number[] {
    const length = Math.min(...series.map((s) => s.length));
    if (!Number.isFinite(length) || length < 2) return [];
    return Array.from({ length }, (_, i) => fold(series.map((s) => s[s.length - length + i])));
  }

  const hottest = $derived.by(() => {
    const temps = readings("temp");
    if (temps.length === 0) return null;
    const top = temps.reduce((a, b) => (b.metric.value > a.metric.value ? b : a));
    const fan = live.metric(top.id, "fan");
    return { ...top, fan, series: live.series(top.id, "temp") };
  });

  const vision = $derived.by(() => {
    const fps = readings("fps");
    if (fps.length === 0) return null;
    const avg = fps.reduce((sum, r) => sum + r.metric.value, 0) / fps.length;
    const slowest = fps.reduce((a, b) => (b.metric.value < a.metric.value ? b : a));
    return {
      avg,
      count: fps.length,
      slowest,
      series: combine(fps.map((r) => live.series(r.id, "fps")), (v) => v.reduce((a, b) => a + b, 0) / v.length),
    };
  });

  const power = $derived.by(() => {
    const volts = readings("voltage");
    if (volts.length === 0) return null;
    const lowest = volts.reduce((a, b) => (b.metric.value < a.metric.value ? b : a));
    const amps = readings("current").reduce((sum, r) => sum + r.metric.value, 0);
    return {
      lowest,
      amps,
      series: combine(volts.map((r) => live.series(r.id, "voltage")), (v) => Math.min(...v)),
    };
  });

  const behind = $derived(records.filter((r) => insights.outdatedIds.has(keyString(r.key))));

  function showDevices() {
    void goto("/devices");
  }
</script>

<div class="grid gap-3" style="grid-template-columns: repeat(auto-fit, minmax(210px, 1fr))">
  {#if !ready}
    {#each Array(placeholders) as _, i (i)}
      <div class="glass flex h-[132px] flex-col gap-3 p-4" aria-hidden="true">
        <Skeleton width="40%" height={12} />
        <Skeleton width="55%" height={26} />
        <div class="mt-auto"><Skeleton height={24} /></div>
      </div>
    {/each}
  {:else}
  <div class="min-w-0" in:rise>
    <StatTile
      icon="plug-connected"
      label="Online"
      value={online.length}
      unit="/ {records.length}"
      sub={online.length === records.length ? "Everything is connected" : `${records.length - online.length} not connected`}
      color={online.length === records.length ? "var(--ok)" : "var(--warn)"}
      onclick={showDevices}
    >
      {#snippet extra()}
        <div class="mt-auto flex flex-wrap gap-1.5 pt-3">
          {#each records as record (keyString(record.key))}
            <span title={deviceName(record)}><StatusDot state={record.presence === "online" ? "online" : "offline"} label={deviceName(record)} /></span>
          {/each}
        </div>
      {/snippet}
    </StatTile>
  </div>

  {#if hottest}
    {@const t = metricValue(hottest.metric)}
    <div class="min-w-0" in:rise={{ delay: stagger(1) }}>
      <StatTile
        icon="temperature"
        label="Hottest"
        value={hottest.metric.value}
        decimals={1}
        unit={t.unit}
        sub="{deviceName(hottest.record)}{hottest.fan ? ` · fan ${metricValue(hottest.fan).value} ${metricValue(hottest.fan).unit}` : ''}"
        series={hottest.series}
        color={toneColor(metricTone(hottest.metric))}
        onclick={() => ui.openDevice(hottest.id)}
      />
    </div>
  {/if}

  {#if vision}
    <div class="min-w-0" in:rise={{ delay: stagger(2) }}>
      <StatTile
        icon="eye"
        label="Vision"
        value={vision.avg}
        unit="fps"
        sub={vision.count > 1 ? `Average of ${vision.count} cameras · slowest ${deviceName(vision.slowest.record)}` : deviceName(vision.slowest.record)}
        series={vision.series}
        color="var(--info)"
        onclick={() => ui.openDevice(vision.slowest.id)}
      />
    </div>
  {/if}

  {#if power}
    {@const v = metricValue(power.lowest.metric)}
    <div class="min-w-0" in:rise={{ delay: stagger(3) }}>
      <StatTile
        icon="battery-charging"
        label="Power"
        value={power.lowest.metric.value}
        decimals={1}
        unit={v.unit}
        sub="Lowest at {deviceName(power.lowest.record)}{power.amps > 0 ? ` · ${power.amps.toFixed(1)} A total` : ''}"
        series={power.series}
        color="var(--warn)"
        onclick={() => ui.openDevice(power.lowest.id)}
      />
    </div>
  {/if}

  <div class="min-w-0" in:rise={{ delay: stagger(4) }}>
    <StatTile
      icon={behind.length ? "arrow-up" : "shield-check"}
      label="Software"
      value={behind.length ? behind.length : "Current"}
      unit={behind.length ? "behind" : ""}
      sub={behind.length ? behind.map(deviceName).join(", ") : "Every device runs its target version"}
      color={behind.length ? "var(--accent)" : "var(--ok)"}
      onclick={showDevices}
    />
  </div>
  {/if}
</div>
