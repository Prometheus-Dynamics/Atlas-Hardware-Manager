<script lang="ts">
  // Two-stage inline confirm for destructive actions: no modal wall.
  import type { Snippet } from "svelte";
  import { errorText } from "$lib/api/client";
  import { toasts } from "$lib/stores/toasts.svelte";

  let {
    action,
    children,
    prompt = "Are you sure?",
    confirmLabel = "Confirm",
    class: extra = "btn btn-sm preset-outlined-error-500",
    disabled = false,
  }: {
    action: () => Promise<unknown>;
    children: Snippet;
    prompt?: string;
    confirmLabel?: string;
    class?: string;
    disabled?: boolean;
  } = $props();

  let asking = $state(false);
  let busy = $state(false);

  async function confirm() {
    busy = true;
    try {
      await action();
      asking = false;
    } catch (error) {
      toasts.error(errorText(error));
    } finally {
      busy = false;
    }
  }
</script>

{#if asking}
  <span class="inline-flex flex-wrap items-center gap-2 rounded-base border border-error-500/50 bg-error-500/10 px-2 py-1">
    <span class="text-xs text-error-200">{prompt}</span>
    <button type="button" class="btn btn-sm preset-filled-error-500" disabled={busy} onclick={confirm}>
      {#if busy}<i class="fa-solid fa-circle-notch fa-spin" aria-hidden="true"></i>{/if}{confirmLabel}
    </button>
    <button type="button" class="btn btn-sm preset-tonal" disabled={busy} onclick={() => (asking = false)}>
      Cancel
    </button>
  </span>
{:else}
  <button type="button" class={extra} {disabled} onclick={() => (asking = true)}>{@render children()}</button>
{/if}
