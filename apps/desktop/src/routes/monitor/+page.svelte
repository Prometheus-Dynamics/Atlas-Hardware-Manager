<script lang="ts">
  // Several devices at once: their live data as a grid of compact panels
  // (readings with trends, streaming sensors as charts), or their cameras as
  // a wall of views. Which devices, the view and the columns are remembered;
  // pick devices here, from a device's page (Monitor), or a whole robot.
  // With none picked, every online device (and camera) shows.
  import { keyString } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import PageHeader from "#lib/components/common/PageHeader.svelte";
  import SegmentedControl from "#lib/components/common/SegmentedControl.svelte";
  import CameraWall from "#lib/components/device/CameraWall.svelte";
  import DeviceMonitor from "#lib/components/device/DeviceMonitor.svelte";
  import { cameraStreams } from "#lib/present.ts";
  import Page from "#lib/components/layout/Page.svelte";
  import Region from "#lib/components/layout/Region.svelte";
  import { deviceName } from "#lib/format.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { robots } from "#lib/stores/robots.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import { masonry } from "#lib/ui/masonry.ts";

  const shown = $derived([...ui.monitored].map((k) => devices.get(k)).filter((r) => !!r));
  const known = $derived(
    [...devices.all].sort((a, b) =>
      a.presence !== b.presence ? (a.presence === "online" ? -1 : 1) : deviceName(a).localeCompare(deviceName(b), undefined, { numeric: true }),
    ),
  );
  let picking = $state(false);
  const VIEW_KEY = "atlas.monitor.view";
  const COLS_KEY = "atlas.monitor.columns";
  const remembered = (key: string, fallback: string) => {
    try {
      return localStorage.getItem(key) ?? fallback;
    } catch {
      return fallback;
    }
  };
  let view = $state(remembered(VIEW_KEY, "data"));
  let cols = $state(remembered(COLS_KEY, "0"));
  $effect(() => {
    try {
      localStorage.setItem(VIEW_KEY, view);
      localStorage.setItem(COLS_KEY, cols);
    } catch {
      // Private mode: it lasts this session.
    }
  });
  const views = [
    { value: "data", label: "Data", icon: "chart-line" as const },
    { value: "cameras", label: "Cameras", icon: "camera" as const },
  ];
  const colOptions = [
    { value: "0", label: "Auto" },
    { value: "1", label: "1" },
    { value: "2", label: "2" },
    { value: "3", label: "3" },
    { value: "4", label: "4" },
  ];
  // The camera wall: the chosen devices, or every online camera.
  const cameraRecords = $derived(shown.length ? shown : devices.all.filter((d) => d.presence === "online" && cameraStreams(d).length > 0));
  // The data panels: the chosen devices, or every online one.
  const dataRecords = $derived(shown.length ? shown : devices.all.filter((d) => d.presence === "online"));
  // Packed in as many columns as fit at about 360 px each.
  let width = $state(1200);
  const columns = $derived(Math.max(1, Math.min(dataRecords.length || 1, Math.floor(width / 360))));

  function addOnline() {
    ui.setMonitored([...new Set([...ui.monitored, ...devices.all.filter((d) => d.presence === "online").map((d) => keyString(d.key))])]);
  }
  function showRobot(name: string) {
    ui.setMonitored(devices.all.filter((d) => d.robot === name).map((d) => keyString(d.key)));
  }
</script>

<svelte:head><title>Monitor · Atlas</title></svelte:head>

<Page>
  {#snippet header()}
    <PageHeader title="Monitor" subtitle="Live data from several devices side by side.">
      {#snippet actions()}
        <SegmentedControl options={views} bind:value={view} label="Monitor view" size="sm" />
        {#if view === "cameras"}<SegmentedControl options={colOptions} bind:value={cols} label="Columns" size="sm" />{/if}
        {#each robots.names as name (name)}
          <Button size="sm" variant="ghost" icon="robot" onclick={() => showRobot(name)} title="Show the devices of {name}">{name}</Button>
        {/each}
        <Button size="sm" icon={picking ? "check" : "plus"} onclick={() => (picking = !picking)}>{picking ? "Done" : "Choose devices"}</Button>
      {/snippet}
    </PageHeader>
  {/snippet}

  {#if picking}
    <div class="picker" role="group" aria-label="Devices to monitor">
      {#each known as record (keyString(record.key))}
        {@const id = keyString(record.key)}
        <button type="button" class="chip" class:on={ui.monitored.has(id)} aria-pressed={ui.monitored.has(id)} onclick={() => ui.toggleMonitored(id)}>
          <span class="dot" class:online={record.presence === "online"}></span>{deviceName(record)}
        </button>
      {/each}
      <span class="ml-auto flex gap-1.5">
        <Button size="sm" variant="ghost" onclick={addOnline}>Add all online</Button>
        <Button size="sm" variant="ghost" onclick={() => ui.setMonitored([])} disabled={ui.monitored.size === 0}>Clear</Button>
      </span>
    </div>
  {/if}

  {#if view === "cameras"}
    <div class="min-h-0 flex-1">
      <Region class="h-full" inner="pb-1" label="Camera views">
        <CameraWall records={cameraRecords} columns={Number(cols)} />
      </Region>
    </div>
  {:else if dataRecords.length === 0}
    <div class="empty">
      <p class="text-[13px] text-fg-muted">No device is online.</p>
    </div>
  {:else}
    <div class="min-h-0 flex-1" bind:clientWidth={width}>
      <Region class="h-full" inner="grid gap-2.5 pb-1" label="Monitored devices">
        {#if shown.length === 0}
          <p class="text-[12px] text-fg-faint">Every online device. Choose devices to show only some.</p>
        {/if}
        <div class="packed" style="grid-template-columns: repeat({columns}, minmax(0, 1fr))">
          {#each dataRecords as record (keyString(record.key))}
            <div use:masonry={10}>
              <DeviceMonitor
                {record}
                removeLabel="Take off the Monitor"
                onremove={shown.length ? () => ui.toggleMonitored(keyString(record.key)) : undefined}
              />
            </div>
          {/each}
        </div>
      </Region>
    </div>
  {/if}
</Page>

<style>
  /* Panels of different heights packed under each other, in order
     (masonry: each spans rows by its height). */
  .packed {
    display: grid;
    grid-auto-rows: 4px;
    column-gap: 10px;
    row-gap: 0;
  }
  .picker {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 8px;
    background: var(--glass);
    border: 1px solid var(--glass-border);
    border-radius: var(--r-card);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 9px;
    font-size: 12.5px;
    color: var(--fg-muted);
    border: 1px solid var(--glass-border);
    border-radius: var(--r-pill);
  }
  .chip:hover {
    color: var(--fg);
    background: var(--glass-hover);
  }
  .chip.on {
    color: var(--fg);
    border-color: var(--accent-ring);
    background: var(--accent-tint);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: var(--r-round);
    background: var(--offline);
  }
  .dot.online {
    background: var(--ok);
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 10px;
    padding: 16px;
    background: var(--glass);
    border: 1px solid var(--glass-border);
    border-radius: var(--r-card);
  }
</style>
