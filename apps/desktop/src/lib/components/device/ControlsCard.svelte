<script lang="ts">
  // The board's general controls in one place, however Atlas reaches it
  // (SSH, the identity endpoint or Orion: the driver offers the ids it can
  // run). Destructive ones confirm inline.
  import type { DeviceAction, UpdateState } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import ConfirmButton from "#lib/components/common/ConfirmButton.svelte";
  import { updateBusy } from "#lib/stores/status.svelte.ts";
  import type { IconName } from "#lib/ui/icons.ts";

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
    if (update === undefined) return true;
    if (id === "update.cancel") return !!update && ["staging", "staged"].includes(update.state);
    if (id === "update.rollback") return !!update?.version_previous && !updateBusy(update);
    return true;
  }

  /** The general controls, in this order; anything else stays on the Actions tab. */
  const CONTROLS: { id: string; icon: IconName; prompt?: string }[] = [
    { id: "locate", icon: "focus-2" },
    { id: "reboot", icon: "refresh" },
    { id: "set-clock", icon: "clock" },
    { id: "update.cancel", icon: "player-stop" },
    { id: "update.rollback", icon: "history", prompt: "Restart into the previous version?" },
    { id: "power-off", icon: "power", prompt: "Power it off? It stays off until its power is cycled." },
    { id: "usb-boot", icon: "usb", prompt: "Restart into USB boot? It waits there until it's flashed or power-cycled." },
  ];

  const shown = $derived(
    CONTROLS.flatMap((control) => {
      const action = actions.find((a) => a.id === control.id);
      return action && applies(control.id) ? [{ ...control, action }] : [];
    }),
  );
</script>

{#if shown.length > 0}
  <section class="flex flex-col gap-2.5">
    <h3 class="text-[12px] font-medium uppercase tracking-[0.06em] text-fg-faint">Controls</h3>
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
