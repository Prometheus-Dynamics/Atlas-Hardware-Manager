<script lang="ts">
  // The host checks a flash depends on (USB access, boot files, the write
  // helper), re-run when the Flash tab opens. Problems show here with their
  // fix, before anything starts; an error blocks the Flash button.
  import { api, errorText } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import { system } from "#lib/stores/system.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";

  let { blocking = $bindable(false) }: { blocking?: boolean } = $props();

  const RELEVANT = ["usb.", "usbboot.", "blockdev."];
  const relevant = $derived(system.health.filter((check) => RELEVANT.some((prefix) => check.id.startsWith(prefix))));
  const problems = $derived(relevant.filter((check) => check.status !== "ok"));

  $effect(() => {
    blocking = problems.some((check) => check.status === "error");
  });

  // Fresh results every time the tab opens: a cable or driver may have changed.
  $effect(() => {
    void system.checkHealth();
  });

  let fixing = $state<string | null>(null);
  async function fix(action: string) {
    if (fixing) return;
    fixing = action;
    try {
      toasts.success(await api.fixHealth(action));
      await system.checkHealth();
    } catch (error) {
      toasts.error(errorText(error));
    } finally {
      fixing = null;
    }
  }
</script>

{#if problems.length > 0}
  <ul class="flex flex-col gap-2" aria-label="Problems on this computer">
    {#each problems as check (check.id)}
      <li class="problem {check.status}">
        <Icon name={check.status === "error" ? "circle-x" : "alert-triangle"} size={16} class="mt-0.5 shrink-0" />
        <div class="min-w-0 flex-1">
          <p class="text-[13px] font-medium">{check.label}</p>
          <p class="text-[12.5px] opacity-90">{check.detail}{check.fix ? ` ${check.fix}` : ""}</p>
        </div>
        {#if check.fix_action}
          <Button size="sm" icon="wand" busy={fixing === check.fix_action} onclick={() => fix(check.fix_action!)}>Fix</Button>
        {/if}
      </li>
    {/each}
  </ul>
{:else if relevant.length > 0 && !system.checking}
  <p class="flex items-center gap-2 text-[12.5px] text-fg-faint">
    <Icon name="circle-check" size={14} class="text-ok-fg" />This computer is ready: USB access, boot files, and the write helper all check out.
  </p>
{/if}

<style>
  .problem {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    padding: 10px 12px;
    border-radius: var(--r-card);
  }
  .problem.warning {
    background: var(--warn-bg);
    color: var(--warn-fg);
  }
  .problem.error {
    background: var(--err-bg);
    color: var(--err-fg);
  }
</style>
