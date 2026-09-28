<script lang="ts">
  import Tag from "$lib/components/common/Tag.svelte";
  import UpdateFlow from "$lib/components/update/UpdateFlow.svelte";
  import { deviceName, primaryVersion } from "$lib/format";
  import { devices } from "$lib/stores/devices.svelte";
  import { system } from "$lib/stores/system.svelte";
  import ActionsTab from "./ActionsTab.svelte";
  import HistoryTab from "./HistoryTab.svelte";
  import OverviewTab from "./OverviewTab.svelte";

  let { key, initialTab }: { key: string; initialTab?: string } = $props();

  const record = $derived(devices.get(key));
  const recovery = $derived(record?.identity.mode === "recovery");

  // Tabs come from what the device can do right now.
  const tabs = $derived.by(() => {
    const caps = record?.capabilities ?? [];
    const list = [{ id: "overview", label: "Overview" }];
    if (caps.includes("recover")) list.push({ id: "update", label: "Recover" });
    else if (caps.includes("update")) list.push({ id: "update", label: "Update" });
    if (caps.includes("actions")) list.push({ id: "actions", label: "Actions" });
    list.push({ id: "history", label: "History" });
    return list;
  });

  // svelte-ignore state_referenced_locally
  let tab = $state(initialTab ?? "overview");
  const current = $derived(tabs.some((t) => t.id === tab) ? tab : "overview");
</script>

{#if !record}
  <p class="p-4 text-xs text-surface-400">This device is no longer in the inventory.</p>
{:else}
  <div class="border-b border-surface-800 px-4 pb-0 pt-3">
    <div class="flex items-start justify-between gap-2">
      <div class="min-w-0">
        <h2 class="truncate text-base font-semibold text-surface-50">{deviceName(record)}</h2>
        <p class="font-mono text-[0.65rem] text-surface-500">{key}</p>
      </div>
      <div class="flex shrink-0 flex-col items-end gap-1">
        {#if record.presence === "online"}
          <Tag tone="success" label="online" />
        {:else}
          <Tag tone="error" label="offline" />
        {/if}
        {#if recovery}<Tag tone="error" icon="fa-kit-medical" label="recovery" />{/if}
      </div>
    </div>
    <p class="mt-1 text-xs text-surface-300">
      {record.identity.model} · <span class="font-mono">{primaryVersion(record.identity) ?? "unknown version"}</span>
    </p>
    <div class="mt-3 flex gap-1" role="tablist">
      {#each tabs as t (t.id)}
        <button
          type="button"
          role="tab"
          aria-selected={current === t.id}
          class="-mb-px border-b-2 px-2.5 py-1.5 text-[0.68rem] uppercase tracking-[0.14em]
            {current === t.id ? 'border-primary-500 text-surface-50' : 'border-transparent text-surface-400 hover:text-surface-100'}"
          onclick={() => (tab = t.id)}
        >
          {t.label}
        </button>
      {/each}
    </div>
  </div>

  <div class="p-4" role="tabpanel">
    {#if current === "overview"}
      <OverviewTab {record} />
    {:else if current === "update"}
      {#if recovery}
        <p class="mb-3 rounded-base border border-error-500/40 bg-error-500/10 px-3 py-2 text-xs text-error-100">
          This device is in recovery mode and only accepts a full recovery write. Pick the image to write.
        </p>
      {/if}
      {#key record.identity.mode}
        <UpdateFlow
          request={{ devices: [record.key], releases: {}, staged: system.settings?.staged_default ?? "auto" }}
          onstarted={() => (tab = "history")}
        />
      {/key}
    {:else if current === "actions"}
      <ActionsTab {record} />
    {:else}
      <HistoryTab {record} />
    {/if}
  </div>
{/if}
