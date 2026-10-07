<script lang="ts">
  // One way to install: a selectable card, or a dimmed one saying why not.
  import type { Snippet } from "svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import type { IconName } from "#lib/ui/icons.ts";

  let {
    title,
    icon,
    selected,
    disabled = null,
    danger = false,
    onselect,
    children,
  }: {
    title: string;
    icon: IconName;
    selected: boolean;
    /** Why this way is not possible right now; null when it is. */
    disabled?: string | null;
    danger?: boolean;
    onselect: () => void;
    children?: Snippet;
  } = $props();
</script>

<label class="card" class:selected={selected && !disabled} class:disabled={!!disabled} class:danger>
  <input type="radio" name="software-method" checked={selected && !disabled} disabled={!!disabled} onchange={onselect} class="sr-only" />
  <span class="flex items-center gap-2">
    <span class="ic"><Icon name={icon} size={15} stroke={2} /></span>
    <span class="text-[13px] font-semibold text-fg">{title}</span>
  </span>
  {#if disabled}
    <span class="text-[12px] leading-snug text-fg-faint">{disabled}</span>
  {:else if children}
    <span class="flex flex-col gap-1 text-[12px] leading-snug text-fg-muted">{@render children()}</span>
  {/if}
</label>

<style>
  .card {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
    padding: 12px 14px;
    border-radius: var(--r-card);
    background: var(--glass);
    border: 1px solid var(--glass-border);
    cursor: pointer;
    transition:
      background var(--t-fast),
      border-color var(--t-fast),
      box-shadow var(--t-fast);
  }
  .card:hover:not(.disabled) {
    background: var(--glass-hover);
  }
  .card:focus-within {
    box-shadow: 0 0 0 2px var(--accent-ring);
  }
  .card.selected {
    background: var(--accent-tint);
    border-color: var(--accent-ring);
  }
  .card.disabled {
    cursor: default;
    background: transparent;
    border-style: dashed;
  }
  .ic {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border-radius: 7px;
    background: var(--glass-strong);
    color: var(--fg-muted);
    transition:
      background var(--t-fast),
      color var(--t-fast);
  }
  .selected .ic {
    background: var(--accent);
    color: var(--on-accent);
  }
  .selected.danger .ic {
    background: var(--err-bg);
    color: var(--err-fg);
  }
  .disabled .ic {
    color: var(--fg-faint);
  }
</style>
