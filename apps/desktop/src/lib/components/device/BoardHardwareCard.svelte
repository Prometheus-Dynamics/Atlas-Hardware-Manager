<script lang="ts">
  // The board's own hardware on the Overview: the supply (the power
  // monitor), the status ring as it looks now, and the USB ports with their
  // power. Everything else about them is in Settings.
  import type { DeviceKey, HardwareSnapshot } from "#lib/api/client.ts";
  import { BoardLive } from "./boardLive.svelte.ts";
  import RingView from "./RingView.svelte";
  import UsbPorts from "./UsbPorts.svelte";

  let {
    hardware,
    live,
    deviceKey = null,
    volts = null,
    amps = null,
    onchanged,
  }: {
    hardware: HardwareSnapshot;
    live: BoardLive;
    deviceKey?: DeviceKey | null;
    volts?: number | null;
    amps?: number | null;
    onchanged?: () => void;
  } = $props();

  const ring = $derived(BoardLive.find(hardware, "light"));
  const ports = $derived(hardware.devices.filter((d) => d.class === "power-switch" && d.status !== "missing"));
  const lens = $derived(BoardLive.find(hardware, "lens"));
  const watts = $derived(volts !== null && amps !== null ? volts * amps : null);
</script>

<section class="panel board" aria-label="Board hardware">
  <div class="panel-head"><h2 class="panel-title">Board hardware</h2><span class="text-[11px] text-fg-faint">lemnosd</span></div>
  <div class="body">
    {#if ring}
      <RingView series={live.series.get(ring.id) ?? null} deviceId={ring.id} {deviceKey} tick={live.tick} size={64} />
    {/if}
    <div class="facts">
      <div class="supply">
        <span class="k">Supply</span>
        <span class="mono truncate" title={watts === null ? "The board's supply now" : `The board's supply now: ${watts.toFixed(1)} W`}>
          {volts === null ? "—" : `${volts.toFixed(2)} V`}{amps === null ? "" : ` · ${amps.toFixed(2)} A`}
        </span>
        {#if lens}
          <span class="k">Lens</span>
          <span class="mono truncate" title={lens.readings.map((r) => `${r.name} ${r.value ?? "–"}`).join(" · ") || lens.model}>{lens.readings.map((r) => `${r.name} ${r.value ?? "–"}`).join(" · ") || lens.model}</span>
        {/if}
      </div>
      {#if ports.length > 0}
        <UsbPorts {ports} {deviceKey} {volts} {amps} {onchanged} />
      {/if}
    </div>
  </div>
</section>

<style>
  .board {
    gap: 6px;
  }
  .body {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    min-width: 0;
  }
  .facts {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }
  .supply {
    display: grid;
    grid-template-columns: 50px minmax(0, 1fr);
    gap: 3px 8px;
    min-width: 0;
    padding-bottom: 3px;
    border-bottom: 1px solid var(--hairline);
    font-size: 12.5px;
  }
  .k {
    font-size: 11.5px;
    color: var(--fg-faint);
  }
</style>
