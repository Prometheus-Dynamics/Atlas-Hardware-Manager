<script lang="ts">
  // A device's page (/device?key=…&tab=…): the device across the window,
  // and the pinned devices in a column beside it, to compare.
  import { page } from "$app/state";
  import { keyString } from "#lib/api/client.ts";
  import DeviceMonitor from "#lib/components/device/DeviceMonitor.svelte";
  import DevicePanel from "#lib/components/device/DevicePanel.svelte";
  import Region from "#lib/components/layout/Region.svelte";
  import { deviceName } from "#lib/format.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";

  const key = $derived(page.url.searchParams.get("key") ?? "");
  const tab = $derived(page.url.searchParams.get("tab") ?? undefined);
  const record = $derived(devices.get(key));
  const pinned = $derived([...ui.pinned].filter((k) => k !== key).map((k) => devices.get(k)).filter((r) => !!r));

  $effect(() => {
    ui.viewing = key || null;
    return () => (ui.viewing = null);
  });
</script>

<svelte:head><title>{record ? deviceName(record) : "Device"} · Atlas</title></svelte:head>

<div class="device-page reveal" class:with-pins={pinned.length > 0}>
  <div class="main">
    {#key key + (tab ?? "")}
      <DevicePanel {key} initialTab={tab} />
    {/key}
  </div>
  {#if pinned.length}
    <aside class="pins" aria-label="Pinned devices">
      <p class="pins-title">Pinned</p>
      <Region inner="flex flex-col gap-2 pb-1">
        {#each pinned as other (keyString(other.key))}
          <DeviceMonitor record={other} removeLabel="Unpin" onremove={() => ui.togglePinned(keyString(other.key))} />
        {/each}
      </Region>
    </aside>
  {/if}
</div>

<style>
  .device-page {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 10px;
    height: 100%;
    min-height: 440px;
  }
  .device-page.with-pins {
    grid-template-columns: minmax(0, 1fr) clamp(300px, 26vw, 420px);
  }
  .main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--glass);
    border: 1px solid var(--glass-border);
    border-radius: var(--r-panel);
    overflow: hidden;
  }
  .pins {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-height: 0;
  }
  .pins-title {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--fg-faint);
  }
  @media (max-width: 900px) {
    .device-page.with-pins {
      grid-template-columns: minmax(0, 1fr);
    }
    .pins {
      display: none;
    }
  }
</style>
