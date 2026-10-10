<script lang="ts">
  // A device's page (/device?key=…&tab=…): the device across the window.
  import { page } from "$app/state";
  import DevicePanel from "#lib/components/device/DevicePanel.svelte";
  import { deviceName } from "#lib/format.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";

  const key = $derived(page.url.searchParams.get("key") ?? "");
  const tab = $derived(page.url.searchParams.get("tab") ?? undefined);
  const record = $derived(devices.get(key));

  $effect(() => {
    ui.viewing = key || null;
    return () => (ui.viewing = null);
  });
</script>

<svelte:head><title>{record ? deviceName(record) : "Device"} · Atlas</title></svelte:head>

<div class="device-page reveal">
  {#key key + (tab ?? "")}
    <DevicePanel {key} initialTab={tab} />
  {/key}
</div>

<style>
  .device-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 440px;
    min-width: 0;
    background: var(--glass);
    border: 1px solid var(--glass-border);
    border-radius: var(--r-panel);
    overflow: hidden;
  }
</style>
