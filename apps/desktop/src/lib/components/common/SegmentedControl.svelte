<script lang="ts" generics="T extends string">
  // A pill-shaped segmented control; the active thumb slides between options.
  import type { IconName } from "$lib/ui/icons";
  import Icon from "./Icon.svelte";

  let {
    options,
    value = $bindable(),
    label,
    size = "md",
    onchange,
  }: {
    options: { value: T; label: string; icon?: IconName; title?: string }[];
    value: T;
    label: string;
    size?: "sm" | "md";
    onchange?: (value: T) => void;
  } = $props();

  let buttons: HTMLButtonElement[] = $state([]);
  const index = $derived(Math.max(0, options.findIndex((o) => o.value === value)));
  const thumb = $derived.by(() => {
    const el = buttons[index];
    return el ? { left: el.offsetLeft, width: el.offsetWidth } : null;
  });

  function select(next: T) {
    value = next;
    onchange?.(next);
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") return;
    event.preventDefault();
    const step = event.key === "ArrowRight" ? 1 : -1;
    const next = options[(index + step + options.length) % options.length];
    select(next.value);
    buttons[options.indexOf(next)]?.focus();
  }
</script>

<div class="seg {size}" role="radiogroup" aria-label={label} tabindex="-1" {onkeydown}>
  {#if thumb}
    <span class="thumb" style="transform: translateX({thumb.left - 3}px); width: {thumb.width}px"></span>
  {/if}
  {#each options as option, i (option.value)}
    <button
      bind:this={buttons[i]}
      type="button"
      role="radio"
      aria-checked={value === option.value}
      tabindex={value === option.value ? 0 : -1}
      title={option.title}
      class:active={value === option.value}
      onclick={() => select(option.value)}
    >
      {#if option.icon}<Icon name={option.icon} size={size === "sm" ? 14 : 15} />{/if}
      {option.label}
    </button>
  {/each}
</div>

<style>
  .seg {
    position: relative;
    display: inline-flex;
    align-items: center;
    padding: 3px;
    gap: 2px;
    border-radius: var(--r-pill);
    background: var(--inset);
    border: 1px solid var(--glass-border);
    width: fit-content;
    max-width: 100%;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .thumb {
    position: absolute;
    top: 3px;
    bottom: 3px;
    left: 3px;
    border-radius: var(--r-pill);
    background: var(--glass-strong);
    border: 1px solid var(--glass-border-strong);
    transition:
      transform var(--t-med) var(--ease-out),
      width var(--t-med) var(--ease-out);
  }
  button {
    position: relative;
    z-index: 1;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 13px;
    border-radius: var(--r-pill);
    font-size: 13px;
    font-weight: 500;
    color: var(--fg-muted);
    white-space: nowrap;
    transition: color var(--t-fast);
  }
  .sm button {
    height: 24px;
    padding: 0 10px;
    font-size: 12px;
  }
  button:hover {
    color: var(--fg);
  }
  button.active {
    color: var(--fg);
  }
</style>
