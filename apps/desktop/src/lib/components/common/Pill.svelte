<script lang="ts">
  // A soft status pill: tinted background, light text, optional icon.
  import type { Snippet } from "svelte";
  import type { Tone } from "$lib/format";
  import type { IconName } from "$lib/ui/icons";
  import Icon from "./Icon.svelte";

  let {
    tone = "neutral",
    icon,
    label,
    mono = false,
    title,
    spin = false,
    children,
  }: {
    tone?: Tone;
    icon?: IconName;
    label?: string;
    mono?: boolean;
    title?: string;
    spin?: boolean;
    children?: Snippet;
  } = $props();
</script>

<span class="pill {tone}" class:mono {title}>
  {#if icon}<Icon name={icon} size={13} stroke={2} class={spin ? "spin" : ""} />{/if}
  {#if label}{label}{/if}
  {#if children}{@render children()}{/if}
</span>

<style>
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 9px;
    border-radius: var(--r-pill);
    font-size: 12px;
    font-weight: 500;
    line-height: 1;
    white-space: nowrap;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .pill.mono {
    font-family: var(--font-code);
    font-size: 11.5px;
    font-weight: 450;
  }
  .success {
    background: var(--ok-bg);
    color: var(--ok-fg);
  }
  .warning {
    background: var(--warn-bg);
    color: var(--warn-fg);
  }
  .error {
    background: var(--err-bg);
    color: var(--err-fg);
  }
  .info {
    background: var(--info-bg);
    color: var(--info-fg);
  }
  .primary {
    background: var(--accent-tint-strong);
    color: var(--accent-text-strong);
  }
  .neutral {
    background: var(--neutral-bg);
    color: var(--fg-muted);
  }
</style>
