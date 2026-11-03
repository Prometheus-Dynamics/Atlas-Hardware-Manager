<script lang="ts">
  // Two-stage inline confirm for destructive actions: no modal wall.
  import type { Snippet } from "svelte";
  import { errorText } from "#lib/api/client.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { rise } from "#lib/ui/motion.ts";
  import type { IconName } from "#lib/ui/icons.ts";
  import Button from "./Button.svelte";

  let {
    action,
    children,
    prompt = "Are you sure?",
    confirmLabel = "Confirm",
    variant = "danger",
    size = "md",
    icon,
    label,
    disabled = false,
  }: {
    action: () => Promise<unknown>;
    children?: Snippet;
    prompt?: string;
    confirmLabel?: string;
    variant?: "danger" | "glass" | "ghost" | "tint";
    size?: "sm" | "md";
    icon?: IconName;
    label?: string;
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

<!-- The question floats over the button, right-aligned to it: asking takes
     no room, so nothing in the row moves. -->
<span class="slot">
  <span class="button" class:hidden-under={asking}><Button {variant} {size} {icon} {label} {disabled} onclick={() => (asking = true)} {children} /></span>
  {#if asking}
    <span class="confirm" in:rise={{ y: 3, duration: 140 }}>
      <span class="text-[12.5px] text-err-fg">{prompt}</span>
      <Button variant="danger" size="sm" {busy} onclick={confirm} icon="alert-triangle">{confirmLabel}</Button>
      <Button variant="ghost" size="sm" disabled={busy} onclick={() => (asking = false)}>Cancel</Button>
    </span>
  {/if}
</span>

<style>
  .slot {
    position: relative;
    display: inline-flex;
  }
  .hidden-under {
    visibility: hidden;
  }
  .confirm {
    position: absolute;
    top: 50%;
    right: 0;
    z-index: 5;
    translate: 0 -50%;
    white-space: nowrap;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 3px 4px 3px 12px;
    border-radius: var(--r-pill);
    background: var(--err-bg);
    border: 1px solid var(--err-bg);
    box-shadow: var(--shadow-layer);
  }
</style>
