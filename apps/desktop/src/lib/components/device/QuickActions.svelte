<script lang="ts">
  // The everyday controls in the device header: the device's non-destructive
  // actions (Find it, Restart, …) and its web page. Only what it offers.
  import { api, openExternal, type DeviceAction, type DeviceRecord } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import { deviceName } from "#lib/format.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import type { IconName } from "#lib/ui/icons.ts";

  let { record }: { record: DeviceRecord } = $props();

  const ICON: Record<string, IconName> = {
    locate: "focus-2",
    identify: "focus-2",
    reboot: "refresh",
    restart: "refresh",
    "open-as-disk": "device-sd-card",
    "browse-files": "folder-open",
    eject: "player-eject",
    "add-ssh-key": "key",
  };
  const DONE: Record<string, string> = {
    "open-as-disk": "is starting as a USB disk; its eMMC shows up in a few seconds.",
    "browse-files": "is open read-only in your file manager.",
    eject: "is ejected and safe to unplug.",
    "add-ssh-key": "has your SSH key; it's installed for root on the next start.",
  };

  let actions = $state<DeviceAction[]>([]);
  // Update controls belong with the update (Overview's Now, Software).
  const quick = $derived(actions.filter((a) => !a.destructive && !a.id.startsWith("update.")).slice(0, 3));
  const webUi = $derived(record.presence === "online" ? (record.identity.attributes?.manage_url ?? null) : null);

  $effect(() => {
    const key = record.key;
    if (record.presence !== "online" || !record.capabilities.includes("actions")) {
      actions = [];
      return;
    }
    let stale = false;
    api
      .deviceActions(key)
      .then((list) => !stale && (actions = list))
      .catch(() => !stale && (actions = []));
    return () => (stale = true);
  });

  async function run(action: DeviceAction) {
    await api.runDeviceAction(record.key, action.id);
    const done = DONE[action.id];
    toasts.success(
      action.id === "locate"
        ? `${deviceName(record)} is signalling. Look for it.`
        : done
          ? `${deviceName(record)} ${done}`
          : `${action.label}: sent to ${deviceName(record)}.`,
    );
  }
</script>

{#if quick.length > 0 || webUi}
  <div class="flex flex-wrap gap-1.5">
    {#each quick as action (action.id)}
      <Button size="sm" icon={ICON[action.id] ?? "player-play"} action={() => run(action)}>{action.label}</Button>
    {/each}
    {#if webUi}
      <Button size="sm" icon="world-www" iconRight="external-link" onclick={() => openExternal(webUi)} title={webUi}>Web UI</Button>
    {/if}
  </div>
{/if}
