<script lang="ts">
  // Two-stage inline confirm for destructive actions: no modal wall.
  import type { Snippet } from "svelte";
  import { errorText } from "$lib/api/client";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { rise } from "$lib/ui/motion";
  import type { IconName } from "$lib/ui/icons";
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

{#if asking}
  <span class="confirm" in:rise={{ y: 3, duration: 140 }}>
    <span class="text-[12.5px] text-err-fg">{prompt}</span>
    <Button variant="danger" size="sm" {busy} onclick={confirm} icon="alert-triangle">{confirmLabel}</Button>
    <Button variant="ghost" size="sm" disabled={busy} onclick={() => (asking = false)}>Cancel</Button>
  </span>
{:else}
  <Button {variant} {size} {icon} {label} {disabled} onclick={() => (asking = true)} {children} />
{/if}

<style>
  .confirm {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 3px 4px 3px 12px;
    border-radius: var(--r-pill);
    background: var(--err-bg);
    border: 1px solid var(--err-bg);
  }
</style>
