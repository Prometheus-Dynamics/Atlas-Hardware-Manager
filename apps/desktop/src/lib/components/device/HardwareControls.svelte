<script lang="ts">
  // A board device's controls. With `deviceKey` (the board takes commands
  // and the source gave each control's range), a slider per control sets it
  // when let go; Restore undoes this computer's writes and a fan can be
  // handed back to the board's own cooling. An on/off control (power.on, a
  // USB port's power) is a switch, and turning a power switch off asks first.
  // Otherwise the values only.
  import { api, errorText, type DeviceKey, type HardwareCommand, type HardwareDevice } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { formatValue, label } from "./hardware.ts";

  let { device, deviceKey }: { device: HardwareDevice; deviceKey: DeviceKey | null } = $props();

  /** What was set here, until the next snapshot shows the device's own value. */
  let drafts = $state<Record<string, number>>({});
  let sending = $state<string | null>(null);

  // A new value from the board replaces what was set here; a snapshot with
  // the same values (every status read) leaves a slider being dragged alone.
  const values = $derived(device.controls.map((c) => `${c.name}=${c.value}`).join(" "));
  let seen = "";
  $effect(() => {
    if (values === seen) return;
    seen = values;
    if (sending === null) drafts = {};
  });

  const settable = (c: HardwareDevice["controls"][number]) => deviceKey !== null && c.min !== null && c.max !== null;
  const step = (min: number, max: number) => (Number.isInteger(min) && Number.isInteger(max) && max - min >= 20 ? 1 : (max - min) / 100);
  const shown = (c: HardwareDevice["controls"][number]) => drafts[c.name] ?? c.value;

  async function send(command: HardwareCommand) {
    if (!deviceKey) return null;
    return api.controlHardware(deviceKey, device.id, command);
  }

  async function set(control: string, value: number) {
    sending = control;
    drafts[control] = value;
    try {
      const applied = await send({ command: "set", control, value });
      if (applied !== null) drafts[control] = applied;
    } catch (error) {
      delete drafts[control];
      toasts.error(errorText(error));
    } finally {
      sending = null;
    }
  }

  async function restore() {
    await send({ command: "restore", control: null });
    toasts.success(`${device.id}: back to the board's values.`);
  }

  async function release() {
    await send({ command: "release" });
    toasts.success(`${device.id}: the board's cooling has it again.`);
  }

  // power.reset (off for N ms) blocks lemnosd for now: not offered.
  const HIDDEN = new Set(["power.reset"]);
  const visible = $derived(device.controls.filter((c) => !HIDDEN.has(c.name)));
  // On/off by name (power.on, a power switch's), not by range: a fan's duty is 0..1 too.
  const isSwitch = (c: HardwareDevice["controls"][number]) => c.min === 0 && c.max === 1 && (c.name === "on" || c.name.endsWith(".on"));
  const isOn = (c: HardwareDevice["controls"][number]) => (shown(c) ?? 0) >= 0.5;
  /** The power switch waiting for "Turn off". */
  let confirming = $state<string | null>(null);

  function flip(c: HardwareDevice["controls"][number]) {
    const on = isOn(c);
    if (on && device.class === "power-switch" && confirming !== c.name) {
      confirming = c.name;
      return;
    }
    confirming = null;
    void set(c.name, on ? 0 : 1);
  }

  const anySettable = $derived(visible.some(settable));
</script>

{#if visible.length > 0}
  <div class="mt-3 flex flex-col gap-2">
    {#each visible as control (control.name)}
      {@const value = shown(control)}
      <div class="glass flex min-w-0 items-center gap-3 px-3.5 py-2.5">
        <span class="w-20 shrink-0 text-[12px] text-fg-faint">{label(control.name)}</span>
        {#if settable(control) && isSwitch(control)}
          {#if confirming === control.name}
            <span class="min-w-0 flex-1 text-[12.5px] text-warn-fg">Turn off {device.id}? Anything plugged in loses power.</span>
            <Button size="sm" variant="danger" onclick={() => flip(control)}>Turn off</Button>
            <Button size="sm" variant="ghost" onclick={() => (confirming = null)}>Cancel</Button>
          {:else}
            <span class="flex-1"></span>
            <button
              type="button"
              class="switch"
              role="switch"
              aria-checked={isOn(control)}
              aria-label="{label(control.name)} on {device.id}"
              disabled={sending === control.name}
              onclick={() => flip(control)}
            ></button>
          {/if}
        {:else if settable(control) && control.min !== null && control.max !== null}
          <input
            class="slider min-w-0 flex-1"
            type="range"
            min={control.min}
            max={control.max}
            step={step(control.min, control.max)}
            value={value ?? control.min}
            disabled={sending === control.name}
            aria-label="Set {label(control.name)} on {device.id}"
            oninput={(e) => (drafts[control.name] = Number(e.currentTarget.value))}
            onchange={(e) => set(control.name, Number(e.currentTarget.value))}
          />
        {:else}
          <span class="flex-1"></span>
        {/if}
        <span class="mono w-24 shrink-0 text-right text-[13px] tabular-nums">
          {#if isSwitch(control)}{isOn(control) ? "on" : "off"}{:else}{formatValue(value)}{#if control.unit}&nbsp;<span class="text-fg-faint">{control.unit}</span>{/if}{/if}
        </span>
      </div>
    {/each}
    {#if anySettable}
      <div class="flex flex-wrap gap-2">
        <Button size="sm" variant="ghost" icon="refresh" action={restore}>Restore</Button>
        {#if device.class === "fan"}
          <Button size="sm" variant="ghost" icon="propeller" action={release}>Hand back to the board</Button>
        {/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .slider {
    accent-color: var(--accent);
  }
  /* As common/Toggle, as a button: a click asks before it changes. */
  .switch {
    position: relative;
    width: 36px;
    height: 20px;
    flex-shrink: 0;
    border-radius: var(--r-pill);
    background: var(--glass-strong);
    border: 1px solid var(--glass-border);
    transition: background var(--t-med);
  }
  .switch::after {
    content: "";
    position: absolute;
    top: 2px;
    left: 2px;
    width: 15px;
    height: 15px;
    border-radius: var(--r-sm);
    background: var(--fg);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.4);
    transition: transform var(--t-med) var(--ease-out);
  }
  .switch[aria-checked="true"] {
    background: var(--accent);
    border-color: transparent;
  }
  .switch[aria-checked="true"]::after {
    transform: translateX(16px);
  }
  .switch:disabled {
    opacity: 0.45;
  }
</style>
