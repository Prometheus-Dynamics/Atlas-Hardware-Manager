<script lang="ts">
  // A SystemCore's USB agent (atlas-usb-agent, which flashes the boards on
  // its USB): the version running, and Install or Update when the agent
  // Atlas carries differs from it. Nothing for other devices.
  import { api, errorText, type DeviceRecord } from "#lib/api/client.ts";
  import { deviceName } from "#lib/format.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";

  let { record }: { record: DeviceRecord } = $props();

  const attrs = $derived(record.identity.attributes ?? {});
  const running = $derived(attrs["usb_agent"] ?? null);
  const offered = $derived(attrs["usb_agent.offered"] ?? null);
  const online = $derived(record.presence === "online");
  let busy = $state(false);

  async function install(event: MouseEvent) {
    event.stopPropagation();
    busy = true;
    try {
      await api.runDeviceAction(record.key, "usb-agent.install");
      toasts.success(`USB agent ${offered} on ${deviceName(record)}.`);
    } catch (error) {
      toasts.error(errorText(error));
    } finally {
      busy = false;
    }
  }
</script>

{#if running || offered}
  <span class="chip" title={running ? `Atlas's USB agent ${running} runs here: it flashes the boards on this device's USB` : "Atlas's USB agent isn't on this device"}>
    <span class="mono">{running ? `USB agent ${running}` : "No USB agent"}</span>
    {#if offered && online}
      <button type="button" class="act" disabled={busy} onclick={install}>{busy ? "…" : running ? "Update" : "Install"}</button>
    {/if}
  </span>
{/if}

<style>
  .chip {
    position: relative;
    z-index: 1;
    pointer-events: auto;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 100%;
    padding: 1px 2px 1px 7px;
    border: 1px solid var(--hairline);
    border-radius: var(--r-chip, 6px);
    font-size: 11px;
    color: var(--fg-muted);
    white-space: nowrap;
  }
  .chip .mono {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .act {
    padding: 0 6px;
    border-radius: 4px;
    background: var(--accent-tint);
    color: var(--accent);
    font-weight: 500;
    cursor: pointer;
  }
  .act:disabled {
    opacity: 0.6;
    cursor: default;
  }
</style>
