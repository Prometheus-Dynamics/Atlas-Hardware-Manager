<script lang="ts">
  import { keyString, type DeviceRecord } from "$lib/api/client";
  import Checkbox from "$lib/components/common/Checkbox.svelte";
  import IconTile from "$lib/components/common/IconTile.svelte";
  import { deviceName, linkText, primaryVersion } from "$lib/format";
  import { deviceIcon, deviceSubline, isRecovery } from "$lib/present";
  import { devices } from "$lib/stores/devices.svelte";
  import { ui } from "$lib/stores/ui.svelte";
  import DeviceStatus from "./DeviceStatus.svelte";
  import { clickCheck, clickDevice } from "./select";

  let { record }: { record: DeviceRecord } = $props();

  const id = $derived(keyString(record.key));
  const name = $derived(deviceName(record));
  const selected = $derived(ui.selection.has(id));
  const focused = $derived(ui.focused === id);
  const open = $derived(ui.panel?.kind === "device" && ui.panel.key === id);
  const online = $derived(record.presence === "online");

  let row: HTMLElement | undefined = $state();
  $effect(() => {
    if (focused) row?.scrollIntoView({ block: "nearest" });
  });
</script>

<div bind:this={row} class="row" class:selected class:focused class:open class:offline={!online} aria-selected={selected} role="row">
  <button type="button" class="hit" aria-label="Open {name}" onclick={(e) => clickDevice(e, id)}></button>
  <span class="relative z-[1] flex"><Checkbox checked={selected} label="Select {name}" onclick={(e) => clickCheck(e, id)} /></span>
  <span class="cell flex min-w-0 items-center gap-3">
    <IconTile icon={deviceIcon(record)} size={28} tone={online && isRecovery(record) ? "accent" : online ? "neutral" : "muted"} />
    <span class="min-w-0">
      <span class="block truncate text-[13px] font-medium text-fg">{name}</span>
      <span class="mono block truncate text-[11.5px] text-fg-faint">{record.key.serial}</span>
    </span>
  </span>
  <span class="cell truncate text-[12.5px] text-fg-muted">{deviceSubline(record)}</span>
  <span class="cell mono truncate text-[12px] text-fg">{primaryVersion(record.identity) ?? "—"}</span>
  <span class="cell truncate text-[12.5px] text-fg-muted">{linkText(record.link_kind, devices.nameOf)}</span>
  <span class="cell flex min-w-0 justify-end"><DeviceStatus {record} compact /></span>
</div>

<style>
  .row {
    position: relative;
    display: grid;
    grid-template-columns: 20px minmax(180px, 1.4fr) minmax(120px, 1fr) minmax(90px, 0.7fr) minmax(90px, 0.8fr) minmax(150px, 1fr);
    align-items: center;
    gap: 16px;
    padding: 8px 14px;
    border-radius: 12px;
    transition: background var(--t-fast);
  }
  .row:hover {
    background: var(--glass);
  }
  .row.open {
    background: var(--glass-hover);
  }
  .row.selected {
    background: var(--accent-tint);
  }
  .row.focused {
    box-shadow: inset 0 0 0 1px var(--glass-border-strong);
  }
  .row.offline .cell {
    opacity: 0.6;
  }
  .cell {
    position: relative;
    pointer-events: none;
  }
  .hit {
    position: absolute;
    inset: 0;
    border-radius: inherit;
    cursor: pointer;
  }
</style>
