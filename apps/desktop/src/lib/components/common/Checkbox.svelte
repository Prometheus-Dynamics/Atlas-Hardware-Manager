<script lang="ts">
  // A round selection check: empty ring, or a red badge with a tick.
  import { pop } from "#lib/ui/motion.ts";
  import Icon from "./Icon.svelte";

  let {
    checked,
    indeterminate = false,
    label,
    onclick,
    class: extra = "",
  }: {
    checked: boolean;
    indeterminate?: boolean;
    label: string;
    onclick: (event: MouseEvent) => void;
    class?: string;
  } = $props();
</script>

<button
  type="button"
  role="checkbox"
  aria-checked={indeterminate ? "mixed" : checked}
  aria-label={label}
  class="check {extra}"
  class:on={checked || indeterminate}
  {onclick}
>
  {#if checked}
    <span class="mark" in:pop><Icon name="check" size={12} stroke={3} /></span>
  {:else if indeterminate}
    <span class="bar"></span>
  {/if}
</button>

<style>
  .check {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    border: 1.5px solid var(--fg-faint);
    background: var(--inset);
    color: var(--on-accent);
    flex-shrink: 0;
    transition:
      background var(--t-fast),
      border-color var(--t-fast),
      transform var(--t-fast);
  }
  .check:hover {
    border-color: var(--fg-muted);
  }
  .check:active {
    transform: scale(0.92);
  }
  .check.on {
    background: var(--accent);
    border-color: var(--accent);
  }
  .mark {
    display: inline-flex;
  }
  .bar {
    width: 8px;
    height: 2px;
    border-radius: 2px;
    background: var(--on-accent);
  }
</style>
