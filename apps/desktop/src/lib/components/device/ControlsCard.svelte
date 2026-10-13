<script lang="ts">
  // The board's controls that aren't already on screen, and only those that
  // apply now: the header has the everyday ones (quick.ts); when the board
  // reports its update, Now has Cancel and Roll back, and without a report
  // they don't show (nothing says an update is there to cancel). Any other
  // action the device offers follows the known ones. Whichever transport
  // offers an id (SSH, the identity endpoint or Orion) runs it. Destructive
  // ones confirm inline.
  import type { DeviceAction, UpdateState } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import ConfirmButton from "#lib/components/common/ConfirmButton.svelte";
  import { updateBusy } from "#lib/stores/status.svelte.ts";
  import type { IconName } from "#lib/ui/icons.ts";
  import { quickActions } from "./quick.ts";

  let {
    actions,
    run,
    update,
  }: {
    actions: DeviceAction[];
    run: (action: DeviceAction) => Promise<void>;
    /** The board's update state, when it reports one: Cancel and Roll back show only when they apply. */
    update?: UpdateState | null;
  } = $props();

  function applies(id: string): boolean {
    if (update === undefined) return !id.startsWith("update.");
    if (id === "update.cancel") return !!update && ["staging", "staged"].includes(update.state);
    if (id === "update.rollback") return !!update?.version_previous && !updateBusy(update);
    return true;
  }

  /** The general controls, in this order; the device's other actions follow. */
  const CONTROLS: { id: string; icon: IconName; prompt?: string }[] = [
    { id: "locate", icon: "focus-2" },
    { id: "reboot", icon: "refresh" },
    { id: "set-clock", icon: "clock" },
    { id: "update.cancel", icon: "player-stop" },
    { id: "update.rollback", icon: "history", prompt: "Restart into the previous version?" },
    { id: "power-off", icon: "power", prompt: "Power it off? It stays off until its power is cycled." },
    { id: "usb-boot", icon: "usb", prompt: "Restart into USB boot? It waits there until it's flashed or power-cycled." },
  ];

  const inHeader = $derived(new Set(quickActions(actions).map((a) => a.id)));
  // Restarting into USB boot belongs to Software, which continues into the install.
  // Shown elsewhere: USB boot (Software), the USB route (its own card).
  const elsewhere = new Set(["usb-boot", "usb-route-on", "usb-route-off"]);
  const shown = $derived([
    ...CONTROLS.flatMap((control) => {
      const action = actions.find((a) => a.id === control.id);
      if (!action || inHeader.has(control.id) || elsewhere.has(control.id) || !applies(control.id)) return [];
      // Now shows the update's own controls when the board reports its update.
      if (update !== undefined && control.id.startsWith("update.")) return [];
      return [{ ...control, action }];
    }),
    ...actions
      .filter((a) => !CONTROLS.some((c) => c.id === a.id) && !inHeader.has(a.id) && !elsewhere.has(a.id) && applies(a.id))
      .map((action) => ({ id: action.id, icon: "player-play" as IconName, prompt: undefined, action })),
  ]);
</script>

<!-- A row inside Details: a panel of its own for one or two buttons was
     mostly frame. -->
{#if shown.length > 0}
  <section class="controls" aria-label="Controls">
    <span class="label">Controls</span>
    <div class="flex flex-wrap gap-1.5">
      {#each shown as { action, icon, prompt } (action.id)}
        {#if action.destructive}
          <ConfirmButton
            size="sm"
            variant="glass"
            {icon}
            action={() => run(action)}
            prompt={prompt ?? `${action.label}?`}
            confirmLabel={action.label}
          >
            {action.label}
          </ConfirmButton>
        {:else}
          <Button size="sm" {icon} action={() => run(action)}>{action.label}</Button>
        {/if}
      {/each}
    </div>
  </section>
{/if}

<style>
  .controls {
    display: flex;
    align-items: center;
    gap: 10px;
    padding-top: 8px;
    border-top: 1px solid var(--hairline);
  }
  .label {
    width: 88px;
    flex-shrink: 0;
    font-size: 11.5px;
    color: var(--fg-muted);
  }
</style>
