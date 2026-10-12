<script lang="ts">
  // The USB link as a way onto the robot network (a Raze's usb-route): a
  // computer on this board's USB port reaches the networks it is on, as if
  // plugged into the radio. The switch runs usb-route-on/off through Orion;
  // the state shown is the board's own, read with its identity (the switch
  // shows what was asked until the board reports it).
  import { api, type DeviceAction, type DeviceRecord } from "#lib/api/client.ts";
  import Icon from "#lib/components/common/Icon.svelte";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { errorText } from "#lib/api/client.ts";
  import { usbRoute } from "#lib/present.ts";

  let {
    record,
    actions,
    run,
  }: { record: DeviceRecord; actions: DeviceAction[]; run: (action: DeviceAction) => Promise<void> } = $props();

  const route = $derived(usbRoute(record));
  const on = $derived(actions.find((a) => a.id === "usb-route-on"));
  const off = $derived(actions.find((a) => a.id === "usb-route-off"));
  /** What was asked, until the board's identity says so. */
  let asked = $state<boolean | null>(null);
  let busy = $state(false);
  const enabled = $derived(asked ?? (route ? route.state !== "off" : false));
  $effect(() => {
    if (asked !== null && route && (route.state !== "off") === asked) asked = null;
  });

  async function flip() {
    const action = enabled ? off : on;
    if (!action || busy) return;
    busy = true;
    try {
      await run(action);
      asked = !enabled;
      // The board reports the new state with its identity: read it now
      // rather than at the next scan, so the badges follow at once.
      void api.scan().catch(() => {});
    } catch (error) {
      toasts.error(errorText(error));
    } finally {
      busy = false;
    }
  }
</script>

{#if route}
  <section class="glass route" aria-label="USB to the robot network">
    <Icon name="router" size={18} />
    <div class="min-w-0 flex-1">
      <p class="title">
        USB to the robot network
        {#if route.state === "active" && asked === null}<span class="badge">routing</span>{/if}
      </p>
      <p class="text">
        {#if !enabled}
          A computer on this camera's USB port reaches this camera only.
        {:else if route.subnets.length}
          A computer on this camera's USB port reaches
          {#each route.subnets as net, i (net)}<span class="mono">{net}</span>{i < route.subnets.length - 1 ? ", " : ""}{/each}
          through it, as if on the radio.
        {:else}
          On: once this camera is on a network, a computer on its USB port reaches it through the camera.
        {/if}
      </p>
    </div>
    {#if on && off}
      <button
        type="button"
        class="switch"
        role="switch"
        aria-checked={enabled}
        aria-label="Route USB to the robot network"
        disabled={busy}
        onclick={flip}
      ></button>
    {:else}
      <span class="text-[11.5px] text-fg-faint" title="Switching it needs Orion with Atlas enrolled">{enabled ? "on" : "off"}</span>
    {/if}
  </section>
{/if}

<style>
  .route {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    color: var(--fg-muted);
  }
  .title {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    font-weight: 600;
    color: var(--fg);
  }
  .badge {
    padding: 1px 6px;
    font-size: 11px;
    font-weight: 600;
    color: var(--ok-fg, var(--ok));
    background: color-mix(in srgb, var(--ok) 14%, transparent);
    border-radius: var(--r-sm);
  }
  .text {
    font-size: 12px;
    color: var(--fg-muted);
  }
  /* As common/Toggle, as a button: it changes when the board takes it. */
  .switch {
    position: relative;
    width: 36px;
    height: 20px;
    flex-shrink: 0;
    border-radius: var(--r-pill);
    background: var(--glass-strong);
    border: 1px solid var(--glass-border);
    transition: background var(--t-med);
  }
  .switch::after {
    content: "";
    position: absolute;
    top: 2px;
    left: 2px;
    width: 15px;
    height: 15px;
    border-radius: var(--r-sm);
    background: var(--fg);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.4);
    transition: transform var(--t-med) var(--ease-out);
  }
  .switch[aria-checked="true"] {
    background: var(--accent);
    border-color: transparent;
  }
  .switch[aria-checked="true"]::after {
    transform: translateX(16px);
  }
  .switch:disabled {
    opacity: 0.45;
  }
</style>
