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
  import { deviceName, primaryVersion, timeAgo } from "#lib/format.ts";
  import { uptimeText } from "#lib/metrics.ts";
  import { live } from "#lib/stores/live.svelte.ts";
  import { deviceIcon, isRecovery, modelName, storageName, usbRoute } from "#lib/present.ts";
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
  import HistoryTab from "./HistoryTab.svelte";
  import LogsTab from "./LogsTab.svelte";
  import OverviewTab from "./OverviewTab.svelte";
  import QuickActions from "./QuickActions.svelte";
  import DeviceMenu from "./DeviceMenu.svelte";
  import SettingsTab from "./SettingsTab.svelte";
  import SoftwareTab from "./SoftwareTab.svelte";
  import { canUpdate } from "#lib/stores/ui.svelte.ts";

  let { key, initialTab }: { key: string; initialTab?: string } = $props();

  const record = $derived(devices.get(key));
  const recovery = $derived(record ? isRecovery(record) : false);
  const route = $derived(record ? usbRoute(record) : null);
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

  // Four tabs: Overview, Settings, Logs, History. Software joins them for
  // a board in USB boot, a running install, or an update started here.
  let wantsSoftware = $state(false);
  const tabs = $derived.by(() => {
    const caps = record?.capabilities ?? [];
    const list: { value: string; label: string; icon: IconName }[] = [{ value: "overview", label: "Overview", icon: "info-circle" }];
    list.push({ value: "settings", label: "Settings", icon: "settings" });
    const software = (recovery && record?.presence === "online") || !!handoff || !!active || wantsSoftware || !!record?.capabilities.includes("slots");
    if (software) list.push({ value: "software", label: "Software", icon: "package" });
    if (caps.includes("logs")) list.push({ value: "logs", label: "Logs", icon: "file-text" });
    list.push({ value: "history", label: "History", icon: "history" });
    return list;
  });

  // Older links name the tabs Software and Settings replaced.
  // svelte-ignore state_referenced_locally
  const asked =
    initialTab === "flash" || initialTab === "update" ? "software" : initialTab === "hardware" ? "settings" : initialTab;
  // svelte-ignore state_referenced_locally
  if (asked === "software") wantsSoftware = true;
  // svelte-ignore state_referenced_locally
  let tab = $state(asked ?? (recovery ? "software" : "overview"));
  const current = $derived(tabs.some((t) => t.value === tab) ? tab : "overview");
  const updatable = $derived(!!record && canUpdate(record) && !recovery && (updateMethods(record).length > 0 || record.capabilities.includes("update")));
  const ssh = $derived(
    record && record.presence === "online" && record.capabilities.includes("status") && record.identity.address
      ? `ssh root@${record.identity.address}`
      : null,
  );
  async function copySsh() {
    if (!ssh) return;
    try {
      await navigator.clipboard.writeText(ssh);
      toasts.success(`Copied: ${ssh}`);
    } catch {
      toasts.info(ssh);
    }
  }

  /** When this device's data was last read: its status, else the last scan. */
  const readAt = $derived(record ? (deviceStatus.readAt.get(keyString(record.key)) ?? devices.lastScanAt) : null);
  let refreshing = $state(false);
  /** Looks for it again on every link, then re-reads its status and history. */
  async function refresh() {
    if (!record || refreshing) return;
    refreshing = true;
    try {
      await devices.scanNow();
      const now = devices.get(key);
      if (now) await Promise.all([deviceStatus.refresh(now.key), deviceStatus.loadHistory(now.key)]);
    } finally {
      refreshing = false;
    }
  }

  const subline = $derived.by(() => {
    if (!record) return "";
    const storage = storageName(record.identity.attributes?.storage);
    const up = live.metric(keyString(record.key), "uptime");
    return [
      modelName(record),
      recovery ? "USB boot" : record.presence === "online" ? (up ? `up ${uptimeText(up.value)}` : "Running") : "Offline",
      primaryVersion(record.identity),
      storage,
      record.key.serial,
    ]
      .filter(Boolean)
      .join(" · ");
  });
</script>

{#if !record}
  <div class="flex items-center gap-3 p-6 text-[13px] text-fg-muted">
    <a href="/devices" class="back" aria-label="Back to devices" title="Back to devices"><Icon name="arrow-left" size={16} /></a>
    This device is no longer in the inventory.
  </div>
{:else}
  <header class="head shrink-0" in:softFade>
    <div class="flex items-center gap-x-3">
      <a href="/devices" class="back" aria-label="Back to devices" title="Back to devices"><Icon name="arrow-left" size={16} /></a>
      <IconTile icon={deviceIcon(record)} size={34} tone={recovery && record.presence === "online" ? "accent" : "neutral"} />
      <div class="min-w-0 shrink">
        <h1 class="truncate text-[16px] font-semibold leading-tight text-fg" title={deviceName(record)}>{deviceName(record)}</h1>
        <p class="truncate text-[12px] text-fg-muted" title={subline}>{subline}</p>
      </div>
      <div class="flex shrink-0 items-center gap-1.5">
        {#if record.presence === "online"}
          <Pill tone="success" icon="circle-check" label="Online" />
        {:else}
          <Pill tone="neutral" icon="plug-connected-x" label="Offline · seen {timeAgo(record.last_seen_ms, clock.now)}" />
        {/if}
        {#if recovery && record.presence === "online"}<Pill tone="primary" icon="usb" label="Waiting for an image" />{/if}
      </div>
      {#if record.presence === "online" && !recovery}
        <button
          type="button"
          class="mode"
          class:pit={ui.mode === "pit"}
          title={ui.mode === "robot" ? "ROBOT mode: network, team and security are locked. Click for PIT." : "PIT mode: the bench; updates and checks first. Click for ROBOT."}
          onclick={() => ui.setMode(ui.mode === "robot" ? "pit" : "robot")}>{ui.mode === "robot" ? "ROBOT" : "PIT"}</button
        >
      {/if}
      <div class="ml-auto flex shrink-0 items-center gap-1.5">
        {#if ui.mode === "pit" && updatable}
          <Button size="sm" variant="primary" icon="download" onclick={() => ((wantsSoftware = true), (tab = "software"))}>Update…</Button>
        {/if}
        <QuickActions {record} />
        {#if ssh}<Button size="sm" icon="terminal-2" title="Copy {ssh}" onclick={() => void copySsh()}>SSH</Button>{/if}
        <Button
          size="sm"
          icon="refresh"
          busy={refreshing || devices.scanning}
          onclick={() => void refresh()}
          title="Look for it on every link and re-read what it reports"
          >Refresh <span class="when">{refreshing || devices.scanning ? "reading" : readAt === null ? "not read" : timeAgo(readAt, clock.now)}</span></Button
        >
        <DeviceMenu {record} />
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
      <div class="tab-body" in:softFade>
        {#if current === "overview"}
          <OverviewTab {record} onhistory={() => (tab = "history")} onsettings={() => (tab = "settings")} />
        {:else if current === "software" && active && !handoff}
          <ActiveJob job={active.job} state={active.state} />
        {:else if current === "software"}
          <SoftwareTab {record} />
        {:else if current === "settings"}
          <SettingsTab {record} {hardware} />
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
  /* At least the view's height, so a tab can fill it (the log does). */
  :global(.device-tab) {
    container-type: inline-size;
    display: flex;
    flex-direction: column;
    min-height: 100%;
    /* Its padding inside that height, or a tab that fits overflows by it. */
    box-sizing: border-box;
  }
  .tab-body {
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    min-height: 0;
  }
  .head {
    padding: 10px 16px;
  }
  .when {
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
  }
  .mode {
    height: 20px;
    padding: 0 7px;
    border: 1px solid var(--glass-border);
    border-radius: var(--r-sm);
    background: var(--glass);
    color: var(--fg-muted);
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.08em;
  }
  .mode.pit {
    border-color: #5a4a22;
    background: #231d12;
    color: var(--warn-fg);
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
