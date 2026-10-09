<script lang="ts">
  // While a running board makes its way into USB boot for a fresh install.
  import Button from "#lib/components/common/Button.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import IconTile from "#lib/components/common/IconTile.svelte";
  import { clock } from "#lib/stores/clock.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import { rise } from "#lib/ui/motion.ts";
  import { HANDOFF_TIMEOUT_MS, handoffs, type Handoff } from "./handoff.svelte";
  import UsbBootSteps from "./UsbBootSteps.svelte";

  let { handoff, steps }: { handoff: Handoff; steps: string[] } = $props();

  const elapsed = $derived(Math.max(0, clock.now - handoff.startedMs));
  const late = $derived(handoff.restarted && elapsed > HANDOFF_TIMEOUT_MS);
  const title = $derived(
    handoff.phase === "starting"
      ? "Found it in USB boot"
      : handoff.phase === "failed"
        ? "The install didn't start"
        : handoff.restarted
          ? "Restarting into USB boot…"
          : "Waiting for USB boot",
  );
</script>

<div class="flex flex-col gap-4" in:rise>
  <div class="glass flex items-start gap-3 px-4 py-3.5">
    <IconTile icon={handoff.phase === "failed" ? "alert-circle" : "usb"} size={36} tone={handoff.phase === "failed" ? "err" : "accent"} />
    <div class="min-w-0 flex-1">
      <p class="text-[13.5px] font-semibold text-fg">{title}</p>
      {#if handoff.phase === "starting"}
        <p class="text-[12.5px] text-fg-muted">Starting the install of <span class="mono">{handoff.choice.version}</span>.</p>
      {:else if handoff.phase === "waiting"}
        <p class="text-[12.5px] text-fg-muted">
          {handoff.name} is erased and gets <span class="mono">{handoff.choice.version}</span> as soon as it shows up.
          {handoff.restarted ? "This usually takes under a minute." : "Put it into USB boot as below."}
        </p>
        <p class="hint mt-1">Waiting {Math.round(elapsed / 1000)} s</p>
      {/if}
    </div>
  </div>

  {#if handoff.phase === "failed" && handoff.error}
    <p class="problem" role="alert"><Icon name="alert-circle" size={16} />{handoff.error}</p>
  {/if}

  {#if handoff.phase === "waiting" && (late || !handoff.restarted)}
    <div class="flex flex-col gap-2" in:rise>
      {#if late}
        <p class="warn">
          <Icon name="alert-triangle" size={16} />It hasn't shown up in USB boot yet. Put it there by hand; the install still starts
          on its own.
        </p>
      {/if}
      <UsbBootSteps {steps} />
    </div>
  {/if}

  <div class="flex flex-wrap gap-2">
    {#if handoff.phase === "failed" && handoff.found}
      {@const found = handoff.found}
      <Button size="sm" icon="usb" onclick={() => (handoffs.cancel(handoff.key), ui.openDevice(found, "software"))}>
        Open the board in USB boot
      </Button>
    {/if}
    {#if handoff.phase !== "starting"}
      <Button size="sm" variant="ghost" icon="x" onclick={() => handoffs.cancel(handoff.key)}>
        {handoff.phase === "failed" ? "Dismiss" : "Stop waiting"}
      </Button>
    {/if}
  </div>
</div>

<style>
  .problem,
  .warn {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    padding: 10px 14px;
    border-radius: var(--r-card);
    font-size: 13px;
    line-height: 1.45;
  }
  .problem {
    background: var(--err-bg);
    color: var(--err-fg);
  }
  .warn {
    background: var(--warn-bg);
    color: var(--warn-fg);
  }
</style>
