<script lang="ts">
  // The board's Orion refuses this PC, so its live readings are polled. Either
  // the board has another key for this PC's operator name (a reinstall, a
  // fresh data folder: the fix is two commands on the board), or it hasn't
  // enrolled the operator at all. Atlas never enrolls or removes by itself:
  // it says what to run on the board, then tries the stream again when asked.
  import { api, type OrionConnection, type OtherKey } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import { live } from "#lib/stores/live.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";

  let { id, kind }: { id: string; kind: "other-key" | "unenrolled" } = $props();

  let open = $state(false);
  let orion = $state<OrionConnection | null>(null);
  // The refusal's own key and commands; only the "not enrolled" case reads the connection.
  const other: OtherKey | null = $derived(kind === "other-key" ? live.otherKey(id) : null);
  const name = $derived((other?.operator_id ?? "").replace(/^operator:/, ""));
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
  const stopped = $derived(live.retriesStopped(id));
  const every = $derived(live.retryEvery(id));
</script>

{#snippet retryControls()}
  <div class="flex flex-wrap items-center gap-2">
    <Button size="sm" icon="refresh" onclick={() => live.retryStream(id)}>Try the stream again</Button>
    {#if stopped}
      <span class="text-fg-muted">Retries are stopped.</span>
    {:else}
      <Button size="sm" variant="ghost" onclick={() => live.stopRetrying(id)}>Stop retrying</Button>
      {#if every}<span class="text-fg-faint">Retrying every {Math.round(every / 1000)} s.</span>{/if}
    {/if}
  </div>
{/snippet}

{#if kind === "other-key"}
  <div class="flex flex-col gap-1.5 pt-1.5 text-[12px]">
    <span class="text-warn-fg">Live readings are polled: the board's Orion has another key for this PC.</span>
    <div class="repair flex flex-col gap-1.5">
      {#if other}
        <p class="text-fg-muted">
          This computer's Orion key ({other.fingerprint}) isn't the one the board enrolled for {name}. On the board (SSH or its console), run both, in order. A key enrolled before for this name stops working.
        </p>
        <div class="flex items-center gap-2"><code class="mono min-w-0 flex-1 break-all">{other.remove_command}</code><Button size="sm" variant="ghost" icon="copy" onclick={() => copy(other!.remove_command)}>Copy</Button></div>
        <div class="flex items-center gap-2"><code class="mono min-w-0 flex-1 break-all">{other.enroll_command}</code><Button size="sm" variant="ghost" icon="copy" onclick={() => copy(other!.enroll_command)}>Copy</Button></div>
      {:else}
        <p class="text-fg-muted">The board refuses this PC's Orion key. See Settings → Orion for the key and the commands.</p>
      {/if}
      {@render retryControls()}
    </div>
  </div>
{:else}
  <div class="flex flex-col gap-1.5 pt-1.5 text-[12px]">
    <div class="flex items-center gap-2">
      <span class="min-w-0 flex-1 truncate text-warn-fg">Live readings are polled: this PC isn't paired with the board's Orion.</span>
      <Button size="sm" variant="ghost" icon="key" onclick={toggle}>Re-pair this PC</Button>
    </div>
    {#if open}
      <div class="repair flex flex-col gap-1.5">
        {#if orion}
          <p class="text-fg-muted">The board hasn't enrolled {orion.operator_id}. On the board (SSH or its console), run:</p>
          <div class="flex items-center gap-2"><code class="mono min-w-0 flex-1 truncate">{orion.enroll_command}</code><Button size="sm" variant="ghost" icon="copy" onclick={() => copy(orion!.enroll_command)}>Copy</Button></div>
          {@render retryControls()}
        {:else}
          <p class="text-fg-muted">Atlas's Orion identity isn't available; see Settings → Orion.</p>
        {/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .repair {
    padding: 8px 10px;
    border-radius: var(--r-card);
    background: var(--warn-bg);
  }
</style>
