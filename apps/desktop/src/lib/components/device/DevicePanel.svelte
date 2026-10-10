<script lang="ts">
  // A device's page: a compact header (back, name, status, quick actions,
  // the Monitor toggle, the tabs) over the tab, which takes the rest of
  // the window and lays out in columns as it widens.
  import Button from "#lib/components/common/Button.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import IconTile from "#lib/components/common/IconTile.svelte";
  import Pill from "#lib/components/common/Pill.svelte";
  import SegmentedControl from "#lib/components/common/SegmentedControl.svelte";
  import { handoffs } from "#lib/components/software/handoff.svelte.ts";
  import { updateMethods } from "#lib/components/software/software.ts";
  import { deviceName, timeAgo } from "#lib/format.ts";
  import { deviceIcon, isRecovery, modelName, storageName } from "#lib/present.ts";
  import { clock } from "#lib/stores/clock.svelte.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { deviceStatus } from "#lib/stores/status.svelte.ts";
  import { keyString } from "#lib/api/client.ts";
  import type { IconName } from "#lib/ui/icons.ts";
  import { softFade } from "#lib/ui/motion.ts";
  import { jobs } from "#lib/stores/jobs.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import Region from "#lib/components/layout/Region.svelte";
  import ActiveJob from "./ActiveJob.svelte";
  import HardwareTab from "./HardwareTab.svelte";
  import HistoryTab from "./HistoryTab.svelte";
  import LogsTab from "./LogsTab.svelte";
  import OverviewTab from "./OverviewTab.svelte";
  import QuickActions from "./QuickActions.svelte";
  import SoftwareTab from "./SoftwareTab.svelte";

  let { key, initialTab }: { key: string; initialTab?: string } = $props();

  const record = $derived(devices.get(key));
  const recovery = $derived(record ? isRecovery(record) : false);
  /** A job already running on this device: Software shows it instead. */
  const active = $derived(jobs.active.get(key) ?? null);
  /** A fresh install waiting for this board to reach USB boot. */
  const handoff = $derived(handoffs.get(key));
  /** The board's hardware, when it has reported any devices. */
  const hardware = $derived.by(() => {
    if (!record) return null;
    const snap = deviceStatus.byDevice.get(keyString(record.key))?.hardware ?? null;
    return snap && snap.devices.length > 0 ? snap : null;
  });

  // The Hardware tab can open first, so this panel keeps the status read too.
  $effect(() => (record ? deviceStatus.watch(record) : undefined));

  // Tabs come from what the device can do right now.
  const tabs = $derived.by(() => {
    const caps = record?.capabilities ?? [];
    const list: { value: string; label: string; icon: IconName }[] = [{ value: "overview", label: "Overview", icon: "info-circle" }];
    const software =
      caps.includes("update") ||
      caps.includes("recover") ||
      (recovery && record?.presence === "online") ||
      (record ? updateMethods(record).length > 0 : false) ||
      !!handoff;
    if (software) list.push({ value: "software", label: "Software", icon: "package" });
    if (caps.includes("logs")) list.push({ value: "logs", label: "Logs", icon: "file-text" });
    if (hardware) list.push({ value: "hardware", label: "Hardware", icon: "cpu" });
    list.push({ value: "history", label: "History", icon: "history" });
    return list;
  });

  // Older links name the tabs Software replaced.
  // svelte-ignore state_referenced_locally
  const asked = initialTab === "flash" || initialTab === "update" ? "software" : initialTab;
  // svelte-ignore state_referenced_locally
  let tab = $state(asked ?? (recovery ? "software" : "overview"));
  const current = $derived(tabs.some((t) => t.value === tab) ? tab : "overview");

  const subline = $derived.by(() => {
    if (!record) return "";
    const storage = storageName(record.identity.attributes?.storage);
    return [modelName(record), recovery ? "USB boot" : record.presence === "online" ? "Running" : "Offline", storage]
      .filter(Boolean)
      .join(" · ");
  });
</script>

{#if !record}
  <p class="p-6 text-[13px] text-fg-muted">This device is no longer in the inventory.</p>
{:else}
  <header class="head shrink-0" in:softFade>
    <div class="flex flex-wrap items-center gap-x-3 gap-y-2">
      <a href="/devices" class="back" aria-label="Back to devices" title="Back to devices"><Icon name="arrow-left" size={16} /></a>
      <IconTile icon={deviceIcon(record)} size={34} tone={recovery && record.presence === "online" ? "accent" : "neutral"} />
      <div class="min-w-0">
        <h1 class="truncate text-[16px] font-semibold leading-tight text-fg" title={deviceName(record)}>{deviceName(record)}</h1>
        <p class="truncate text-[12px] text-fg-muted" title={subline}>{subline}</p>
      </div>
      <div class="flex flex-wrap items-center gap-1.5">
        {#if record.presence === "online"}
          <Pill tone="success" icon="circle-check" label="Online" />
        {:else}
          <Pill tone="neutral" icon="plug-connected-x" label="Offline · seen {timeAgo(record.last_seen_ms, clock.now)}" />
        {/if}
        {#if recovery && record.presence === "online"}<Pill tone="primary" icon="usb" label="Waiting for an image" />{/if}
        <Pill tone="neutral" mono label={record.key.serial} title={key} />
      </div>
      <div class="ml-auto flex flex-wrap items-center gap-1.5">
        <QuickActions {record} />
        <span class="sep" aria-hidden="true"></span>
        <Button
          size="sm"
          icon="layout-dashboard"
          onclick={() => {
            ui.toggleMonitored(key);
            if (ui.monitored.has(key)) toasts.success(`${record ? deviceName(record) : "It"} is on the Monitor page.`);
            else toasts.info(`${record ? deviceName(record) : "It"} is off the Monitor page.`);
          }}
          title={ui.monitored.has(key) ? "Take it off the Monitor page" : "Add it to the Monitor page"}
        >{ui.monitored.has(key) ? "On Monitor" : "Monitor"}</Button>
      </div>
    </div>
    {#if tabs.length > 1}
      <div class="mt-2.5">
        <SegmentedControl options={tabs} bind:value={tab} label="Device sections" size="sm" />
      </div>
    {/if}
  </header>

  <!-- The tab scrolls under the header. Its width is a container, so a wide
       window lays a tab out in columns. -->
  <div class="flex min-h-0 flex-1 flex-col border-t border-hairline" role="tabpanel">
    <Region inner="device-tab px-4 pb-4 pt-3">
    {#key current}
      <div in:softFade>
        {#if current === "overview"}
          <OverviewTab {record} />
        {:else if current === "software" && active && !handoff}
          <ActiveJob job={active.job} state={active.state} />
        {:else if current === "software"}
          <SoftwareTab {record} />
        {:else if current === "hardware" && hardware}
          <HardwareTab {hardware} boardKey={record.key} deviceKey={record.capabilities.includes("hardware-control") ? record.key : null} />
        {:else if current === "logs"}
          <LogsTab {record} />
        {:else}
          <HistoryTab {record} />
        {/if}
      </div>
    {/key}
    </Region>
  </div>
{/if}

<style>
  :global(.device-tab) {
    container-type: inline-size;
  }
  .head {
    padding: 10px 16px;
  }
  .sep {
    width: 1px;
    height: 18px;
    margin: 0 2px;
    background: var(--glass-border);
  }
  .back {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: var(--r-md);
    color: var(--fg-muted);
    border: 1px solid var(--glass-border);
  }
  .back:hover {
    color: var(--fg);
    background: var(--glass-hover);
  }
</style>
