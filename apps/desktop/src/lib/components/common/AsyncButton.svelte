<script lang="ts">
  // A button that runs an async action once at a time, shows it is busy,
  // and turns a rejection into an error toast.
  import type { Snippet } from "svelte";
  import { errorText } from "$lib/api/client";
  import { toasts } from "$lib/stores/toasts.svelte";

  let {
    action,
    children,
    class: extra = "btn btn-sm preset-tonal",
    disabled = false,
    title,
    type = "button",
  }: {
    action: () => Promise<unknown>;
    children: Snippet;
    class?: string;
    disabled?: boolean;
    title?: string;
    type?: "button" | "submit";
  } = $props();

  let busy = $state(false);

  async function run() {
    if (busy) return;
    busy = true;
    try {
      await action();
    } catch (error) {
      toasts.error(errorText(error));
    } finally {
      busy = false;
    }
  }
</script>

<button {type} class={extra} disabled={disabled || busy} aria-busy={busy} {title} onclick={run}>
  {#if busy}<i class="fa-solid fa-circle-notch fa-spin" aria-hidden="true"></i>{/if}
  {@render children()}
</button>
