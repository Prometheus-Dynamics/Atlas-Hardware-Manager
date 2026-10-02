<script lang="ts">
  import IconTile from "$lib/components/common/IconTile.svelte";
  import Pill from "$lib/components/common/Pill.svelte";
  import SegmentedControl from "$lib/components/common/SegmentedControl.svelte";
  import UpdateFlow from "$lib/components/update/UpdateFlow.svelte";
  import { deviceName, timeAgo } from "$lib/format";
  import { deviceIcon, isRecovery, modelName, storageName } from "$lib/present";
  import { clock } from "$lib/stores/clock.svelte";
  import { devices } from "$lib/stores/devices.svelte";
  import { system } from "$lib/stores/system.svelte";
  import type { IconName } from "$lib/ui/icons";
  import { softFade } from "$lib/ui/motion";
  import ActionsTab from "./ActionsTab.svelte";
  import FlashTab from "./FlashTab.svelte";
  import HistoryTab from "./HistoryTab.svelte";
  import OverviewTab from "./OverviewTab.svelte";

  let {
    key,
    initialTab,
    onwide,
  }: { key: string; initialTab?: string; onwide?: (wide: boolean) => void } = $props();

  const record = $derived(devices.get(key));
  const recovery = $derived(record ? isRecovery(record) : false);

  // Tabs come from what the device can do right now.
  const tabs = $derived.by(() => {
    const caps = record?.capabilities ?? [];
    const list: { value: string; label: string; icon: IconName }[] = [{ value: "overview", label: "Overview", icon: "info-circle" }];
    if (caps.includes("recover") || (recovery && record?.presence === "online")) list.push({ value: "flash", label: "Flash", icon: "bolt" });
    else if (caps.includes("update")) list.push({ value: "update", label: "Update", icon: "arrow-up" });
    if (caps.includes("actions") && !recovery) list.push({ value: "actions", label: "Actions", icon: "tool" });
    list.push({ value: "history", label: "History", icon: "history" });
    return list;
  });

  // svelte-ignore state_referenced_locally
  let tab = $state(initialTab ?? (recovery ? "flash" : "overview"));
  const current = $derived(tabs.some((t) => t.value === tab) ? tab : "overview");

  $effect(() => {
    onwide?.(current === "flash");
  });
  $effect(() => () => onwide?.(false));

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
  <header class="shrink-0 px-6 pb-4 pt-5" in:softFade>
    <div class="flex items-center gap-4 pr-10">
      <IconTile icon={deviceIcon(record)} size={52} tone={recovery && record.presence === "online" ? "accent" : "neutral"} />
      <div class="min-w-0 flex-1">
        <h2 class="truncate text-[18px] font-semibold text-fg">{deviceName(record)}</h2>
        <p class="truncate text-[13px] text-fg-muted">{subline}</p>
      </div>
    </div>
    <div class="mt-3 flex flex-wrap items-center gap-1.5">
      {#if record.presence === "online"}
        <Pill tone="success" icon="circle-check" label="Online" />
      {:else}
        <Pill tone="neutral" icon="plug-connected-x" label="Offline · seen {timeAgo(record.last_seen_ms, clock.now)}" />
      {/if}
      {#if recovery && record.presence === "online"}<Pill tone="primary" icon="usb" label="Waiting for an image" />{/if}
      <Pill tone="neutral" mono label={record.key.serial} title={key} />
    </div>
    {#if tabs.length > 1}
      <div class="mt-4">
        <SegmentedControl options={tabs} bind:value={tab} label="Device sections" />
      </div>
    {/if}
  </header>

  <div class="min-h-0 flex-1 overflow-y-auto border-t border-hairline px-6 pb-6 pt-5" role="tabpanel">
    {#key current}
      <div in:softFade={{ duration: 160 }}>
        {#if current === "overview"}
          <OverviewTab {record} />
        {:else if current === "flash"}
          <FlashTab {record} />
        {:else if current === "update"}
          <UpdateFlow
            request={{ devices: [record.key], releases: {}, staged: system.settings?.staged_default ?? "auto" }}
            onstarted={() => (tab = "history")}
          />
        {:else if current === "actions"}
          <ActionsTab {record} />
        {:else}
          <HistoryTab {record} />
        {/if}
      </div>
    {/key}
  </div>
{/if}
