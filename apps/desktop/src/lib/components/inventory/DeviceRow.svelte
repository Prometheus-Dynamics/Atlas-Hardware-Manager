<script lang="ts">
  import { keyString, type DeviceRecord } from "$lib/api/client";
  import ProgressBar from "$lib/components/common/ProgressBar.svelte";
  import Tag from "$lib/components/common/Tag.svelte";
  import { deviceName, linkText, overallFraction, primaryVersion, STEP_LABELS, timeAgo } from "$lib/format";
  import { clock } from "$lib/stores/clock.svelte";
  import { devices } from "$lib/stores/devices.svelte";
  import { jobs } from "$lib/stores/jobs.svelte";
  import { ui } from "$lib/stores/ui.svelte";

  let { record }: { record: DeviceRecord } = $props();

  const id = $derived(keyString(record.key));
  const selected = $derived(ui.selection.has(id));
  const focused = $derived(ui.focused === id);
  const open = $derived(ui.panel?.kind === "device" && ui.panel.key === id);
  const online = $derived(record.presence === "online");
  const recovery = $derived(record.identity.mode === "recovery");
  const active = $derived(jobs.active.get(id));
  const fresh = $derived(devices.fresh.has(id));

  let row: HTMLTableRowElement | undefined = $state();
  $effect(() => {
    if (focused) row?.scrollIntoView({ block: "nearest" });
  });

  function onRowClick(event: MouseEvent) {
    if ((event.target as HTMLElement).closest("input, button, a")) return;
    if (event.shiftKey) {
      ui.selectRange(id);
      return;
    }
    if (event.ctrlKey || event.metaKey) {
      ui.toggle(id);
      return;
    }
    ui.openDevice(id);
  }

  function onCheck(event: MouseEvent & { currentTarget: HTMLInputElement }) {
    ui.focused = id;
    if (event.shiftKey) ui.selectRange(id);
    else ui.toggle(id);
    // Keep the box in step with the store (a range may not match the click).
    event.currentTarget.checked = ui.selection.has(id);
  }
</script>

<tr
  bind:this={row}
  class="cursor-default whitespace-nowrap border-b border-surface-800/70 text-xs transition-colors
    {selected ? 'bg-primary-500/10' : 'hover:bg-surface-800/40'}
    {open ? 'bg-surface-800/70' : ''}
    {focused ? 'outline outline-1 -outline-offset-1 outline-primary-400/70' : ''}
    {fresh ? 'row-new' : ''}
    {online ? '' : 'text-surface-500'}"
  onclick={onRowClick}
  aria-selected={selected}
>
  <td class="w-8 px-2 py-1.5">
    <input
      type="checkbox"
      class="checkbox h-3.5 w-3.5 rounded-sm border-surface-600 bg-surface-900"
      checked={selected}
      onclick={onCheck}
      aria-label="Select {deviceName(record)}"
    />
  </td>
  <td class="max-w-56 px-2 py-1.5">
    <button type="button" class="block max-w-full truncate text-left font-medium text-surface-50 hover:underline {online ? '' : 'text-surface-400'}" onclick={() => ui.openDevice(id)}>
      {deviceName(record)}
    </button>
    <span class="block truncate font-mono text-[0.62rem] text-surface-500">{record.key.serial}</span>
  </td>
  <td class="px-2 py-1.5 text-surface-300">{record.key.family}</td>
  <td class="px-2 py-1.5 text-surface-300">{record.identity.model}</td>
  <td class="px-2 py-1.5 font-mono text-[0.7rem] text-surface-100">{primaryVersion(record.identity) ?? "—"}</td>
  <td class="px-2 py-1.5">
    {#if recovery}
      <Tag tone="error" icon="fa-kit-medical" label="recovery" title="Only accepts a recovery write" />
    {:else}
      <span class="text-[0.65rem] uppercase tracking-[0.12em] text-surface-500">normal</span>
    {/if}
  </td>
  <td class="px-2 py-1.5 text-surface-300">{linkText(record.link_kind, devices.nameOf)}</td>
  <td class="px-2 py-1.5 text-surface-300">{record.robot ?? "—"}</td>
  <td class="w-44 px-2 py-1.5">
    {#if active}
      <div class="flex flex-col gap-0.5">
        <span class="text-[0.65rem] text-secondary-200">
          {active.state.status.status === "queued" ? "Queued" : active.state.step ? STEP_LABELS[active.state.step] : "Starting"}
          · job #{active.job}
        </span>
        <ProgressBar value={overallFraction(active.state)} label="Update progress" />
      </div>
    {:else if online}
      <span class="inline-flex items-center gap-1.5 text-success-400">
        <span class="h-1.5 w-1.5 rounded-full bg-success-500"></span>online
      </span>
    {:else}
      <span class="inline-flex items-center gap-1.5 text-error-300" title="Last seen {new Date(record.last_seen_ms).toLocaleString()}">
        <span class="h-1.5 w-1.5 rounded-full bg-error-500"></span>offline · {timeAgo(record.last_seen_ms, clock.now)}
      </span>
    {/if}
  </td>
</tr>
