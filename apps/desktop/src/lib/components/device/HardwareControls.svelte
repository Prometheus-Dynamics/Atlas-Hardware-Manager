<script lang="ts">
  // A board device's controls. With `deviceKey` (the board takes commands
  // and the source gave each control's range), a slider per control sets it
  // when let go; Restore undoes this computer's writes and a fan can be
  // handed back to the board's own cooling. Otherwise the values only.
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

  const anySettable = $derived(device.controls.some(settable));
</script>

{#if device.controls.length > 0}
  <div class="mt-3 flex flex-col gap-2">
    {#each device.controls as control (control.name)}
      {@const value = shown(control)}
      <div class="glass flex min-w-0 items-center gap-3 px-3.5 py-2.5">
        <span class="w-20 shrink-0 text-[12px] text-fg-faint">{label(control.name)}</span>
        {#if settable(control) && control.min !== null && control.max !== null}
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
          {formatValue(value)}{#if control.unit}&nbsp;<span class="text-fg-faint">{control.unit}</span>{/if}
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
</style>
