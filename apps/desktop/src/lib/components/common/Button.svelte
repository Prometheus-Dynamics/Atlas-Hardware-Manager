<script lang="ts">
  // The one button. Variants: primary (red, one per view), tint (red-tinted,
  // for "needs you" actions), glass (default), ghost, and danger (rose).
  // With `action`, it runs an async function once at a time, shows it is
  // busy, and turns a rejection into an error toast.
  import type { Snippet } from "svelte";
  import { errorText } from "#lib/api/client.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import type { IconName } from "#lib/ui/icons.ts";
  import Icon from "./Icon.svelte";

  type Variant = "primary" | "tint" | "glass" | "ghost" | "danger";

  let {
    variant = "glass",
    size = "md",
    icon,
    iconRight,
    action,
    onclick,
    busy: busyProp = false,
    disabled = false,
    type = "button",
    title,
    label,
    full = false,
    class: extra = "",
    children,
  }: {
    variant?: Variant;
    size?: "sm" | "md" | "lg";
    icon?: IconName;
    iconRight?: IconName;
    action?: () => Promise<unknown>;
    onclick?: (event: MouseEvent) => void;
    busy?: boolean;
    disabled?: boolean;
    type?: "button" | "submit";
    title?: string;
    /** Accessible name for icon-only buttons. */
    label?: string;
    full?: boolean;
    class?: string;
    children?: Snippet;
  } = $props();

  let running = $state(false);
  const busy = $derived(busyProp || running);
  /** After an action: a check for a moment when it worked, a red edge when it didn't. */
  let outcome = $state<"done" | "failed" | null>(null);
  let outcomeTimer: ReturnType<typeof setTimeout> | null = null;
  function show(next: "done" | "failed") {
    outcome = next;
    if (outcomeTimer) clearTimeout(outcomeTimer);
    outcomeTimer = setTimeout(() => (outcome = null), next === "done" ? 1300 : 1800);
  }

  async function click(event: MouseEvent) {
    onclick?.(event);
    if (!action || running) return;
    running = true;
    try {
      await action();
      show("done");
    } catch (error) {
      show("failed");
      toasts.error(errorText(error));
    } finally {
      running = false;
    }
  }
</script>

<button
  {type}
  class="btn {variant} {size} {extra}"
  class:full
  class:icon-only={!children}
  class:done={outcome === "done"}
  class:failed={outcome === "failed"}
  disabled={disabled || busy}
  aria-busy={busy}
  aria-label={label}
  title={title ?? label}
  onclick={click}
>
  {#if busy}
    <Icon name="loader-2" class="spin" size={size === "sm" ? 14 : 16} />
  {:else if outcome === "done" && (icon || !children)}
    <Icon name="check" size={size === "sm" ? 14 : 16} />
  {:else if icon}
    <Icon name={icon} size={size === "sm" ? 14 : 16} />
  {/if}
  {#if children}<span class="truncate">{@render children()}</span>{/if}
  {#if iconRight}<Icon name={iconRight} size={size === "sm" ? 14 : 16} />{/if}
</button>

<style>
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 7px;
    min-width: 0;
    height: 34px;
    padding: 0 15px;
    border-radius: var(--r-pill);
    border: 1px solid transparent;
    font: inherit;
    font-size: 13px;
    font-weight: 500;
    white-space: nowrap;
    cursor: pointer;
    user-select: none;
    transition:
      transform var(--t-fast) var(--ease-out),
      background var(--t-fast),
      border-color var(--t-fast),
      color var(--t-fast),
      box-shadow var(--t-fast);
  }
  .btn.sm {
    height: 28px;
    padding: 0 11px;
    font-size: 12px;
    gap: 5px;
  }
  .btn.lg {
    height: 42px;
    padding: 0 20px;
    font-size: 14px;
  }
  .btn.icon-only {
    padding: 0;
    width: 34px;
  }
  .btn.sm.icon-only {
    width: 28px;
  }
  .btn.full {
    width: 100%;
  }
  /* No lift on hover: a button stays in line with its neighbours. */
  .btn:not(:disabled):active {
    transform: scale(0.98);
  }
  /* What just happened, for a moment (see `show`). */
  .btn.done {
    border-color: color-mix(in srgb, var(--ok) 55%, transparent);
    color: var(--ok-fg);
  }
  .btn.failed {
    border-color: color-mix(in srgb, var(--err) 65%, transparent);
  }
  .btn:disabled {
    cursor: default;
    opacity: 0.45;
  }

  .primary {
    background: var(--accent);
    color: var(--on-accent);
    box-shadow: 0 4px 14px -4px var(--accent-glow);
  }
  .primary:not(:disabled):hover {
    background: var(--accent-hover);
    box-shadow: 0 8px 22px -6px var(--accent-glow);
  }
  .primary:not(:disabled):active {
    background: var(--accent-press);
  }
  .tint {
    background: var(--accent-tint);
    color: var(--accent-text-strong);
    border-color: var(--accent-tint-strong);
  }
  .tint:not(:disabled):hover {
    background: var(--accent-tint-hover);
  }
  .glass {
    background: var(--glass);
    border-color: var(--glass-border);
    color: var(--fg);
  }
  .glass:not(:disabled):hover {
    background: var(--glass-hover);
    border-color: var(--glass-border-strong);
    box-shadow: var(--shadow-lift);
  }
  .ghost {
    background: transparent;
    color: var(--fg-muted);
  }
  .ghost:not(:disabled):hover {
    background: var(--glass);
    color: var(--fg);
  }
  .danger {
    background: transparent;
    color: var(--err-fg);
  }
  .danger:not(:disabled):hover {
    background: var(--err-bg);
  }
</style>
