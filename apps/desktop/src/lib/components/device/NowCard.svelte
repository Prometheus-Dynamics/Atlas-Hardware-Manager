<script lang="ts">
  // What the board is doing right now, whoever started it: an update
  // staging (with its progress), a restart into it, a trial boot, a
  // rollback, or a failure; with Cancel and Roll back right there.
  import type { DeviceAction, DeviceStatus } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import ConfirmButton from "#lib/components/common/ConfirmButton.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import Pill from "#lib/components/common/Pill.svelte";
  import ProgressBar from "#lib/components/common/ProgressBar.svelte";
  import type { Tone } from "#lib/format.ts";
  import { sourceLabel, updateBusy } from "#lib/stores/status.svelte.ts";
  import type { IconName } from "#lib/ui/icons.ts";

  let {
    status,
    actions,
    run,
  }: {
    status: DeviceStatus;
    actions: DeviceAction[];
    run: (action: DeviceAction) => Promise<void>;
  } = $props();

  const u = $derived(status.update);
  const cancel = $derived(actions.find((a) => a.id === "update.cancel"));
  const rollback = $derived(actions.find((a) => a.id === "update.rollback"));

  const view = $derived.by((): { tone: Tone; icon: IconName; title: string; detail: string | null; spin?: boolean } => {
    if (!u) return { tone: "neutral", icon: "circle-check", title: "Nothing in progress", detail: null };
    const next = u.version_staged ?? "the new version";
    switch (u.state) {
      case "staging":
        return {
          tone: "primary",
          icon: "download",
          title: `Installing an update into slot ${u.slot_staged ?? "?"}`,
          detail: "Downloading and writing the spare slot; the running version isn't touched.",
          spin: false,
        };
      case "staged":
        // An apply whose restart was cut short says so, and is still staged.
        return {
          tone: u.error ? "warning" : "primary",
          icon: "package",
          title: `${next} is staged`,
          detail: u.error ?? "It runs after the next restart into it.",
        };
      case "rebooting":
        return { tone: "info", icon: "refresh", title: `Restarting into ${next}`, detail: null, spin: true };
      case "trying":
        return {
          tone: "info",
          icon: "heartbeat",
          title: `Trial boot of ${next}`,
          detail: "The board keeps it once its services are healthy, or goes back by itself.",
        };
      case "rolled-back":
        // Asked for (a rollback, or going forward again): no error. A trial
        // that failed or a link that stayed down leaves its reason.
        if (!u.error) {
          return {
            tone: "neutral",
            icon: "history",
            title: `Switched to ${u.version_active ?? "the previous version"}${u.slot_active ? ` on slot ${u.slot_active}` : ""}`,
            detail: u.version_previous ? `${u.version_previous} is the one to go back to.` : null,
          };
        }
        return { tone: "warning", icon: "history", title: `Went back to ${u.version_active ?? "the previous version"}`, detail: u.error };
      case "error":
        return { tone: "error", icon: "alert-circle", title: "The last update failed", detail: u.error };
      case "cancelled":
        return { tone: "neutral", icon: "player-stop", title: "The update was cancelled", detail: null };
      default:
        return {
          tone: "neutral",
          icon: "circle-check",
          title: `Running ${u.version_active ?? "its version"}${u.slot_active ? ` on slot ${u.slot_active}` : ""}`,
          detail: null,
        };
    }
  });

  const active = $derived(!!u && !["idle", "confirmed"].includes(u.state));
  const canCancel = $derived(!!cancel && !!u && ["staging", "staged"].includes(u.state));
  const canRollBack = $derived(!!rollback && !!u?.version_previous && !updateBusy(u) && u.state !== "staging");
</script>

<div class="glass now flex flex-col gap-3 px-4 py-3.5" class:active>
  <div class="flex items-start gap-3">
    <span class="tone mt-0.5 shrink-0 {view.tone}"><Icon name={view.icon} size={18} class={view.spin ? "spin" : ""} /></span>
    <div class="min-w-0 flex-1">
      <p class="text-[14px] font-medium text-fg">{view.title}</p>
      {#if view.detail}<p class="mt-0.5 text-[12.5px] text-fg-muted">{view.detail}</p>{/if}
    </div>
    {#if active && u?.started_by}
      <Pill tone="neutral" icon={u.started_by === "orion" ? "router" : u.started_by === "atlas" ? "device-laptop" : "terminal-2"} label="by {sourceLabel(u.started_by)}" />
    {/if}
  </div>
  {#if u?.state === "staging"}
    <div class="flex items-center gap-3">
      <div class="flex-1"><ProgressBar value={u.progress / 1000} label="Update progress" /></div>
      <span class="mono w-10 text-right text-[12px] text-fg-muted">{Math.round(u.progress / 10)}%</span>
    </div>
  {/if}
  {#if canCancel || canRollBack}
    <div class="flex flex-wrap gap-1.5">
      {#if canCancel && cancel}
        <Button size="sm" icon="player-stop" action={() => run(cancel)}>Cancel update</Button>
      {/if}
      {#if canRollBack && rollback}
        <ConfirmButton
          size="sm"
          icon="history"
          variant="glass"
          action={() => run(rollback)}
          prompt="Restart into {u?.version_previous}?"
          confirmLabel="Roll back"
        >
          Roll back to {u?.version_previous}
        </ConfirmButton>
      {/if}
    </div>
  {/if}
</div>

<style>
  .now.active {
    border-color: var(--accent-ring);
  }
  .tone {
    color: var(--fg-muted);
  }
  .tone.primary,
  .tone.info {
    color: var(--accent-text);
  }
  .tone.warning {
    color: var(--warn-fg);
  }
  .tone.error {
    color: var(--err-fg);
  }
</style>
