<script lang="ts">
  // Several devices at once (Monitor-v2): each reading as one chart with a
  // line per device, so they compare at a glance, over the last 1, 5 or 15
  // minutes; or their cameras as a wall of views. The grid always fits the
  // window. Which devices show is remembered (the chips; a device's page
  // can add itself); with none chosen, every online device that reports.
  import { keyString, type DeviceRecord, type Metric } from "#lib/api/client.ts";
  import PageHeader from "#lib/components/common/PageHeader.svelte";
  import SegmentedControl from "#lib/components/common/SegmentedControl.svelte";
  import CameraWall from "#lib/components/device/CameraWall.svelte";
  import MonitorChart from "#lib/components/device/MonitorChart.svelte";
  import Page from "#lib/components/layout/Page.svelte";
  import Region from "#lib/components/layout/Region.svelte";
  import { deviceName } from "#lib/format.ts";
  import { isCore, isTextMetric, sortMetrics } from "#lib/metrics.ts";
  import { cameraStreams, isRecovery } from "#lib/present.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { hasTelemetry, live, watchLive } from "#lib/stores/live.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";

  const COLORS = Array.from({ length: 8 }, (_, i) => `var(--series-${i + 1})`);
  const VIEW_KEY = "atlas.monitor.view";
  const RANGE_KEY = "atlas.monitor.range";
  const remembered = (key: string, fallback: string) => {
    try {
      return localStorage.getItem(key) ?? fallback;
    } catch {
      return fallback;
    }
  };
  let view = $state(remembered(VIEW_KEY, "data"));
  let range = $state(remembered(RANGE_KEY, "300000"));
  $effect(() => {
    try {
      localStorage.setItem(VIEW_KEY, view);
      localStorage.setItem(RANGE_KEY, range);
    } catch {
      // Private mode: it lasts this session.
    }
  });
  const views = [
    { value: "data", label: "Charts", icon: "chart-line" as const },
    { value: "cameras", label: "Cameras", icon: "camera" as const },
  ];
  const ranges = [
    { value: "60000", label: "1 min" },
    { value: "300000", label: "5 min" },
    { value: "900000", label: "15 min" },
  ];

  const byName = (a: DeviceRecord, b: DeviceRecord) => deviceName(a).localeCompare(deviceName(b), undefined, { numeric: true });
  // Every device that can report, online first: the chips. Colours follow
  // this order, so a device keeps its colour while others come and go.
  const candidates = $derived(
    devices.all.filter((d) => d.capabilities.includes("telemetry") && !isRecovery(d)).sort((a, b) => (a.presence !== b.presence ? (a.presence === "online" ? -1 : 1) : byName(a, b))),
  );
  const colorOf = $derived(new Map(candidates.map((d, i) => [keyString(d.key), COLORS[i % COLORS.length]])));
  const chosen = (d: DeviceRecord) => ui.monitored.size === 0 || ui.monitored.has(keyString(d.key));
  const shown = $derived(candidates.filter((d) => hasTelemetry(d) && chosen(d)));

  function toggle(d: DeviceRecord) {
    const id = keyString(d.key);
    // With none chosen all show: the first click hides just that one.
    if (ui.monitored.size === 0) ui.setMonitored(candidates.map((c) => keyString(c.key)).filter((k) => k !== id));
    else if (ui.monitored.has(id) && ui.monitored.size === 1) ui.setMonitored([]);
    else ui.toggleMonitored(id);
  }

  $effect(() => watchLive(shown));

  // One chart per reading any shown device reports, in the usual order.
  const charts = $derived.by(() => {
    const seen = new Map<string, Metric>();
    for (const d of shown) {
      for (const m of live.metrics.get(keyString(d.key)) ?? []) {
        if (!isCore(m) && !isTextMetric(m) && !seen.has(m.id)) seen.set(m.id, m);
      }
    }
    return sortMetrics([...seen.values()]).slice(0, 9);
  });

  // As many columns as fit at about 420 px; rows share the height.
  let width = $state(1200);
  const cols = $derived(Math.max(1, Math.min(charts.length, 3, Math.floor(width / 420) || 1)));
  // Rows share the height; a short last row stretches across, so no cell
  // stays empty.
  const rows = $derived(
    Array.from({ length: Math.max(1, Math.ceil(charts.length / cols)) }, (_, r) => charts.slice(r * cols, r * cols + cols)),
  );

  const cameraRecords = $derived(
    devices.all.filter((d) => d.presence === "online" && cameraStreams(d).length > 0 && (ui.monitored.size === 0 || ui.monitored.has(keyString(d.key)))),
  );
</script>

<svelte:head><title>Monitor · Atlas</title></svelte:head>

<Page>
  {#snippet header()}
    <PageHeader title="Monitor">
      {#snippet actions()}
        {#if view === "data"}<SegmentedControl options={ranges} bind:value={range} label="Time shown" size="sm" />{/if}
        <SegmentedControl options={views} bind:value={view} label="Monitor view" size="sm" />
      {/snippet}
    </PageHeader>
  {/snippet}

  <div class="chips" role="group" aria-label="Devices shown">
    {#each candidates as d (keyString(d.key))}
      {@const on = chosen(d)}
      {@const online = d.presence === "online"}
      <button
        type="button"
        class="chip"
        class:on
        class:offline={!online}
        aria-pressed={on}
        title={online ? (on ? `Hide ${deviceName(d)}` : `Show ${deviceName(d)}`) : `${deviceName(d)} is offline`}
        onclick={() => toggle(d)}
      >
        <span class="swatch" style:background={on && online ? colorOf.get(keyString(d.key)) : "var(--offline)"}></span>{deviceName(d)}
      </button>
    {/each}
    {#if ui.monitored.size > 0}<button type="button" class="link" onclick={() => ui.setMonitored([])}>Show all</button>{/if}
  </div>

  {#if view === "cameras"}
    <div class="min-h-0 flex-1">
      <Region class="h-full" inner="pb-1" label="Camera views">
        <CameraWall records={cameraRecords} columns={0} />
      </Region>
    </div>
  {:else if shown.length === 0}
    <div class="empty">
      <p class="text-[13px] text-fg-muted">{candidates.length ? "None of the chosen devices is online." : "No device that reports readings is online."}</p>
    </div>
  {:else}
    <div class="grid-area" bind:clientWidth={width}>
      {#each rows as row, r (r)}
        <div class="row">
          {#each row as m (m.id)}
            <MonitorChart
              metric={m.id}
              title={m.label}
              unit={m.unit ?? ""}
              devices={shown.map((record) => ({ record, color: colorOf.get(keyString(record.key)) ?? COLORS[0] }))}
              windowMs={Number(range)}
            />
          {/each}
        </div>
      {/each}
    </div>
  {/if}
</Page>

<style>
  .chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 9px;
    font-size: 12px;
    color: var(--fg-faint);
    border: 1px solid var(--glass-border);
    border-radius: var(--r-pill);
  }
  .chip:hover {
    color: var(--fg);
    background: var(--glass-hover);
  }
  .chip.on {
    color: var(--fg);
    background: var(--glass);
  }
  .chip.offline {
    opacity: 0.6;
  }
  .swatch {
    width: 10px;
    height: 3px;
    border-radius: 1px;
  }
  .link {
    font-size: 12px;
    color: var(--info-fg);
  }
  .link:hover {
    color: var(--fg);
  }
  .grid-area {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .row {
    flex: 1 1 0;
    min-height: 0;
    display: flex;
    gap: 10px;
  }
  .row > :global(*) {
    flex: 1 1 0;
  }
  .empty {
    padding: 16px;
    background: var(--glass);
    border: 1px solid var(--glass-border);
    border-radius: var(--r-card);
  }
</style>
