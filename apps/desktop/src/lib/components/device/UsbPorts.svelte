<script lang="ts">
  // The board's USB ports (its power switches, Lemnos power-switch devices):
  // each port's power, a switch and, where the board can, a reset (a second
  // off, then on). Turning power on is always asked first, with what the
  // board draws now: on a bench PC's USB a device that starts up can brown
  // the board out. Nothing here turns a port on by itself.
  import { api, errorText, type DeviceKey, type HardwareDevice } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import { toasts } from "#lib/stores/toasts.svelte.ts";

  let {
    ports,
    deviceKey,
    amps = null,
    volts = null,
    onchanged,
  }: {
    ports: HardwareDevice[];
    /** The board, when its devices take commands. */
    deviceKey: DeviceKey | null;
    /** The board's supply now, for the question before power goes on. */
    amps?: number | null;
    volts?: number | null;
    onchanged?: () => void;
  } = $props();

  /** What a port can supply, by its name (USB-C up to 3 A, USB-A 0.9 A). */
  const budget = (id: string) => (/usb-?c/i.test(id) ? "up to 3 A" : "up to 0.9 A");
  const nice = (id: string) => id.replace(/-power$/, "").replace(/^usb-?/i, "USB-").toUpperCase();
  const reading = (port: HardwareDevice, name: string) => port.readings.find((r) => r.name === name)?.value ?? null;
  const isOn = (port: HardwareDevice) => (drafts[port.id] ?? reading(port, "power.on") ?? port.controls.find((c) => c.name === "power.on")?.value ?? 0) >= 0.5;
  const fault = (port: HardwareDevice) => {
    const v = reading(port, "fault") ?? reading(port, "power.fault");
    return v !== null && v >= 0.5;
  };
  const canReset = (port: HardwareDevice) => port.controls.some((c) => c.name === "power.reset");
  const canSwitch = (port: HardwareDevice) => deviceKey !== null && port.controls.some((c) => c.name === "power.on");

  let drafts = $state<Record<string, number>>({});
  let asking = $state<{ port: HardwareDevice; what: "on" | "off" | "reset" } | null>(null);
  let busy = $state<string | null>(null);

  async function go() {
    if (!asking || !deviceKey) return;
    const { port, what } = asking;
    asking = null;
    busy = port.id;
    try {
      if (what === "reset") {
        await api.hardwareAction(deviceKey, port.id, "power.reset", { off_ms: 1000 });
        toasts.success(`${nice(port.id)}: off for a second, then back on.`);
      } else {
        drafts[port.id] = what === "on" ? 1 : 0;
        await api.controlHardware(deviceKey, port.id, { command: "power", on: what === "on" });
        toasts.success(`${nice(port.id)} power ${what}.`);
      }
      onchanged?.();
    } catch (error) {
      delete drafts[port.id];
      toasts.error(errorText(error));
    } finally {
      busy = null;
    }
  }

  const draw = $derived(amps === null ? "unknown" : `${amps.toFixed(2)} A${volts === null ? "" : ` at ${volts.toFixed(2)} V`}`);
</script>

{#each ports as port (port.id)}
  {@const on = isOn(port)}
  <div class="port">
    <span class="dot" class:on class:fault={fault(port)}></span>
    <span class="name truncate" title="{port.id} · {port.model}">{nice(port.id)}</span>
    <span class="state truncate" title={fault(port) ? "The port reports a fault (over-current)" : on ? "Power on" : "Power off"}>
      {fault(port) ? "fault" : on ? "on" : "off"}
    </span>
    <span class="acts">
      {#if canSwitch(port)}
        <button
          type="button"
          class="switch"
          role="switch"
          aria-checked={on}
          aria-label="{nice(port.id)} power"
          disabled={busy === port.id}
          onclick={() => (asking = { port, what: on ? "off" : "on" })}
        ></button>
      {/if}
      {#if canSwitch(port) && canReset(port) && on}
        <Button size="sm" variant="ghost" icon="refresh" label="Reset {nice(port.id)}" title="Cut the port for a second" onclick={() => (asking = { port, what: "reset" })} />
      {/if}
    </span>
  </div>
  {#if asking?.port.id === port.id}
    <div class="ask" role="alert">
      {#if asking.what === "on"}
        <p>
          Turn {nice(port.id)} power on? The board draws <b class="mono">{draw}</b> now; a device here can add {budget(port.id)}. On a
          computer's USB port that can brown the board out.
        </p>
      {:else if asking.what === "off"}
        <p>Turn {nice(port.id)} power off? Anything plugged in loses power.</p>
      {:else}
        <p>Cut {nice(port.id)} for a second? What's plugged in restarts.</p>
      {/if}
      <div class="flex justify-end gap-1.5">
        <Button size="sm" variant="ghost" onclick={() => (asking = null)}>Cancel</Button>
        <Button size="sm" variant={asking.what === "on" ? "primary" : "danger"} onclick={() => void go()}>
          {asking.what === "on" ? "Turn on" : asking.what === "off" ? "Turn off" : "Reset"}
        </Button>
      </div>
    </div>
  {/if}
{/each}

<style>
  .port {
    display: grid;
    grid-template-columns: 8px minmax(0, 1fr) 44px auto;
    align-items: center;
    gap: 8px;
    min-height: 26px;
    font-size: 12.5px;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #3a414c;
  }
  .dot.on {
    background: var(--ok);
  }
  .dot.fault {
    background: var(--err);
  }
  .state {
    color: var(--fg-muted);
  }
  .acts {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .ask {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 2px 0 6px;
    padding: 8px 10px;
    border: 1px solid #4a3a1f;
    border-radius: var(--r-panel);
    background: #1d1912;
    font-size: 12px;
    color: var(--fg);
  }
  .switch {
    position: relative;
    width: 32px;
    height: 18px;
    flex-shrink: 0;
    border-radius: var(--r-pill);
    background: var(--glass-strong);
    border: 1px solid var(--glass-border);
  }
  .switch::after {
    content: "";
    position: absolute;
    top: 2px;
    left: 2px;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--fg-muted);
    transition: transform var(--t-fast);
  }
  .switch[aria-checked="true"] {
    background: var(--ok);
    border-color: var(--ok);
  }
  .switch[aria-checked="true"]::after {
    transform: translateX(14px);
    background: #0f1115;
  }
</style>
