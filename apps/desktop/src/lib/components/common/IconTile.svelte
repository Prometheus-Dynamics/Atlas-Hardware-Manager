<script lang="ts">
  // A rounded square holding an icon: the face of a device or a section.
  import type { IconName } from "#lib/ui/icons.ts";
  import Icon from "./Icon.svelte";

  let {
    icon,
    size = 36,
    tone = "neutral",
  }: { icon: IconName; size?: number; tone?: "neutral" | "accent" | "ok" | "err" | "warn" | "muted" } = $props();
</script>

<span class="tile {tone}" style="--s: {size}px">
  <Icon name={icon} size={Math.round(size * 0.5)} stroke={1.6} />
</span>

<style>
  .tile {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    width: var(--s);
    height: var(--s);
    border-radius: calc(var(--s) * 0.3);
    background: var(--glass-strong);
    border: 1px solid var(--glass-border);
    color: var(--fg);
    transition:
      background var(--t-med),
      color var(--t-med);
  }
  .accent {
    background: var(--accent-tint-strong);
    border-color: var(--accent-tint-strong);
    color: var(--accent-text-strong);
  }
  .ok {
    background: var(--ok-bg);
    color: var(--ok-fg);
  }
  .err {
    background: var(--err-bg);
    color: var(--err-fg);
  }
  .warn {
    background: var(--warn-bg);
    color: var(--warn-fg);
  }
  .muted {
    background: var(--glass);
    color: var(--fg-faint);
  }
</style>
