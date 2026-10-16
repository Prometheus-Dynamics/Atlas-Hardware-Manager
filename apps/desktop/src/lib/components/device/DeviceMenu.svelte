<script lang="ts">
  // The device header's ⋯ menu: everything the device offers that isn't a
  // header button, the support bundle and the Monitor toggle, then, apart,
  // what takes it offline (restart, power off, USB boot, roll back). Those
  // ask first, naming the device; in ROBOT mode by typing the end of its
  // serial.
  import { api, errorText, type DeviceAction, type DeviceRecord } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import { deviceName } from "#lib/format.ts";
  import { deviceStatus, updateBusy } from "#lib/stores/status.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import { keyString } from "#lib/api/client.ts";
  import { portal } from "#lib/ui/portal.ts";
  import { quickActions } from "./quick.ts";
  import { saveSupportBundle } from "./supportBundle.ts";

  let { record }: { record: DeviceRecord } = $props();

  /** Offline for a while, or for good: asked first. */
  const GUARDED: Record<string, string> = {
    reboot: "It goes offline for about 40 s, then comes back by itself.",
    restart: "It goes offline for a moment, then comes back by itself.",
    "power-off": "It stays off until its power is cycled.",
    "usb-boot": "It waits in USB boot until it's flashed or power-cycled.",
    "update.rollback": "It restarts into its previous version.",
  };
  const DONE: Record<string, string> = {
    reboot: "is restarting.",
    "power-off": "is shutting down. It stays off until its power is cycled.",
    "usb-boot": "is restarting into USB boot.",
    "set-clock": "has this computer's time.",
    "update.cancel": "cancelled its update.",
    "update.rollback": "is restarting into its previous version.",
  };
  // Their own controls elsewhere: the USB route switch (Settings), and
  // Cancel, which the running update's card on the Overview has.
  const ELSEWHERE = new Set(["usb-route-on", "usb-route-off", "update.cancel"]);

  let actions = $state<DeviceAction[]>([]);
  let open = $state(false);
  let asking = $state<DeviceAction | null>(null);
  let typed = $state("");
  let running = $state(false);

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

  const update = $derived(deviceStatus.byDevice.get(keyString(record.key))?.update ?? null);
  function applies(id: string): boolean {
    if (id === "update.cancel") return !!update && ["staging", "staged"].includes(update.state);
    if (id === "update.rollback") return !!update?.version_previous && !updateBusy(update);
    return true;
  }
  const inHeader = $derived(new Set(quickActions(actions).map((a) => a.id)));
  const offered = $derived(actions.filter((a) => !inHeader.has(a.id) && !ELSEWHERE.has(a.id) && applies(a.id)));
  const everyday = $derived(offered.filter((a) => !a.destructive && !(a.id in GUARDED)));
  const guarded = $derived(offered.filter((a) => a.destructive || a.id in GUARDED));
  const bundle = $derived(record.capabilities.includes("status"));
  const tail = $derived(record.key.serial.slice(-4));
  const robot = $derived(ui.mode === "robot");
  const confirmed = $derived(!robot || typed.trim().toLowerCase() === tail.toLowerCase());

  async function run(action: DeviceAction) {
    open = false;
    try {
      await api.runDeviceAction(record.key, action.id);
      const done = DONE[action.id];
      toasts.success(done ? `${deviceName(record)} ${done}` : `${action.label}: sent to ${deviceName(record)}.`);
      void deviceStatus.refresh(record.key);
    } catch (error) {
      toasts.error(errorText(error));
    }
  }

  function ask(action: DeviceAction) {
    open = false;
    typed = "";
    asking = action;
  }

  async function confirm() {
    if (!asking || !confirmed || running) return;
    running = true;
    try {
      await run(asking);
    } finally {
      running = false;
      asking = null;
    }
  }

  function monitor() {
    open = false;
    const key = keyString(record.key);
    ui.toggleMonitored(key);
    toasts.success(`${deviceName(record)} is ${ui.monitored.has(key) ? "on" : "off"} the Monitor page.`);
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && ((open = false), (asking = null))} />

<div class="relative">
  <Button size="sm" icon="dots" label="More" title="More actions" onclick={() => (open = !open)} />
  {#if open}
    <button type="button" class="scrim" aria-label="Close the menu" onclick={() => (open = false)}></button>
    <div class="menu" role="menu">
      {#each everyday as action (action.id)}
        <button type="button" class="item" role="menuitem" onclick={() => void run(action)}>{action.label}</button>
      {/each}
      {#if bundle}
        <button type="button" class="item" role="menuitem" onclick={() => ((open = false), void saveSupportBundle(record))}>Save a support bundle…</button>
      {/if}
      <button type="button" class="item" role="menuitem" onclick={monitor}>
        {ui.monitored.has(keyString(record.key)) ? "Take off the Monitor page" : "Add to the Monitor page"}
      </button>
      {#if guarded.length > 0}
        <div class="sep"></div>
        <div class="group">Takes it offline</div>
        {#each guarded as action (action.id)}
          <button type="button" class="item danger" role="menuitem" onclick={() => ask(action)}>{action.label}…</button>
        {/each}
      {/if}
    </div>
  {/if}
</div>

{#if asking}
  <div class="modal" use:portal>
    <button type="button" class="backdrop" aria-label="Cancel" onclick={() => (asking = null)}></button>
    <div class="dialog panel" role="dialog" aria-modal="true" aria-label="{asking.label} {deviceName(record)}">
      <h2 class="text-[14px] font-semibold text-fg">{asking.label} {deviceName(record)}?</h2>
      <p class="text-[12.5px] text-fg-muted">{GUARDED[asking.id] ?? "It may go offline."}</p>
      {#if robot}
        <label class="flex flex-col gap-1 text-[12px] text-warn-fg">
          ROBOT mode: type {tail} to confirm.
          <!-- svelte-ignore a11y_autofocus -->
          <input class="input mono" bind:value={typed} placeholder={tail} autofocus onkeydown={(e) => e.key === "Enter" && void confirm()} />
        </label>
      {/if}
      <div class="flex justify-end gap-2">
        <Button size="sm" variant="ghost" onclick={() => (asking = null)}>Cancel</Button>
        <Button size="sm" variant="danger" disabled={!confirmed} busy={running} onclick={() => void confirm()}>{asking.label}</Button>
      </div>
    </div>
  </div>
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 40;
    cursor: default;
  }
  .menu {
    position: absolute;
    right: 0;
    top: calc(100% + 4px);
    z-index: 41;
    min-width: 230px;
    max-width: 320px;
    padding: 4px 0;
    border: 1px solid var(--glass-border);
    border-radius: var(--r-panel);
    background: var(--glass);
    box-shadow: 0 8px 22px rgba(0, 0, 0, 0.5);
  }
  .item {
    display: block;
    width: 100%;
    height: 28px;
    padding: 0 12px;
    overflow: hidden;
    text-align: left;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12.5px;
    color: var(--fg);
  }
  .item:hover {
    background: var(--glass-hover);
  }
  .item.danger {
    color: var(--accent-text);
  }
  .sep {
    height: 1px;
    margin: 4px 0;
    background: var(--hairline);
  }
  .group {
    padding: 3px 12px;
    font-size: 10.5px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--fg-faint);
  }
  .modal {
    position: fixed;
    inset: 0;
    z-index: 95;
    display: grid;
    place-items: center;
  }
  .backdrop {
    position: absolute;
    inset: 0;
    background: rgba(5, 6, 8, 0.65);
    cursor: default;
  }
  .dialog {
    position: relative;
    width: min(400px, calc(100vw - 32px));
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px;
  }
</style>
