<script lang="ts">
  // The board's Orion refuses this PC, so its live readings are polled: it
  // has another key for this PC's operator name (a reinstall, a fresh data
  // folder) or none. Atlas never enrolls by itself: it says what to run on
  // the board, then tries the stream again when asked.
  import { api, type OrionConnection } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import { live } from "#lib/stores/live.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";

  let { id, kind }: { id: string; kind: "other-key" | "unenrolled" } = $props();

  let open = $state(false);
  let orion = $state<OrionConnection | null>(null);
  async function toggle() {
    open = !open;
    if (open && !orion) orion = await api.orionConnection().catch(() => null);
  }
  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      toasts.success("Command copied.");
    } catch {
      toasts.error("Couldn't copy to the clipboard.");
    }
  }
  const remove = $derived(orion ? `orionctl operators remove ${orion.operator_id}` : "");
</script>

<div class="flex flex-col gap-1.5 pt-1.5 text-[12px]">
  <div class="flex items-center gap-2">
    <span class="min-w-0 flex-1 truncate text-warn-fg">Live readings are polled: this PC isn't paired with the board's Orion.</span>
    <Button size="sm" variant="ghost" icon="key" onclick={toggle}>Re-pair this PC</Button>
  </div>
  {#if open}
    <div class="repair flex flex-col gap-1.5">
      {#if orion}
        <p class="text-fg-muted">
          {kind === "other-key"
            ? `The board has a different key for ${orion.operator_id}. On the board (SSH or its console), run both; a key enrolled before for this name stops working.`
            : `The board hasn't enrolled ${orion.operator_id}. On the board (SSH or its console), run:`}
        </p>
        {#if kind === "other-key"}
          <div class="flex items-center gap-2"><code class="mono min-w-0 flex-1 truncate">{remove}</code><Button size="sm" variant="ghost" icon="copy" onclick={() => copy(remove)}>Copy</Button></div>
        {/if}
        <div class="flex items-center gap-2"><code class="mono min-w-0 flex-1 truncate">{orion.enroll_command}</code><Button size="sm" variant="ghost" icon="copy" onclick={() => copy(orion!.enroll_command)}>Copy</Button></div>
        <div><Button size="sm" icon="refresh" onclick={() => live.retryStream(id)}>Try the stream again</Button></div>
      {:else}
        <p class="text-fg-muted">Atlas's Orion identity isn't available; see Settings → Orion.</p>
      {/if}
    </div>
  {/if}
</div>

<style>
  .repair {
    padding: 8px 10px;
    border-radius: var(--r-card);
    background: var(--warn-bg);
  }
</style>
