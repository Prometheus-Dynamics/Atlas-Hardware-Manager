<script lang="ts">
  // Several devices' live data at once: a grid of compact panels (readings
  // with trends, streaming sensors as charts). Which devices is remembered;
  // pick them here, from a device's page (Monitor), or a whole robot.
  import { keyString } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import PageHeader from "#lib/components/common/PageHeader.svelte";
  import DeviceMonitor from "#lib/components/device/DeviceMonitor.svelte";
  import Page from "#lib/components/layout/Page.svelte";
  import Region from "#lib/components/layout/Region.svelte";
  import { deviceName } from "#lib/format.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { robots } from "#lib/stores/robots.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";

  const shown = $derived([...ui.monitored].map((k) => devices.get(k)).filter((r) => !!r));
  const known = $derived(
    [...devices.all].sort((a, b) =>
      a.presence !== b.presence ? (a.presence === "online" ? -1 : 1) : deviceName(a).localeCompare(deviceName(b), undefined, { numeric: true }),
    ),
  );
  let picking = $state(false);
  // The grid: as many columns as fit at about 360 px each.
  let width = $state(1200);
  const columns = $derived(Math.max(1, Math.min(shown.length || 1, Math.floor(width / 360))));

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

  {#if shown.length === 0}
    <div class="empty">
      <p class="text-[13px] text-fg-muted">No devices on the Monitor yet.</p>
      <div class="flex gap-2">
        <Button size="sm" variant="primary" onclick={addOnline}>Show every online device</Button>
        <Button size="sm" onclick={() => (picking = true)}>Choose devices</Button>
      </div>
    </div>
  {:else}
    <div class="min-h-0 flex-1" bind:clientWidth={width}>
      <Region class="h-full" inner="grid gap-2.5 pb-1" label="Monitored devices">
        <div class="grid" style="grid-template-columns: repeat({columns}, minmax(0, 1fr))">
          {#each shown as record (keyString(record.key))}
            <DeviceMonitor {record} removeLabel="Take off the Monitor" onremove={() => ui.toggleMonitored(keyString(record.key))} />
          {/each}
        </div>
      </Region>
    </div>
  {/if}
</Page>

<style>
  .grid {
    display: grid;
    gap: 10px;
    align-items: start;
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
