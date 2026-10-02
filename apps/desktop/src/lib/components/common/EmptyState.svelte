<script lang="ts">
  import type { Snippet } from "svelte";
  import type { IconName } from "$lib/ui/icons";
  import IconTile from "./IconTile.svelte";

  let { icon, title, children }: { icon: IconName; title: string; children?: Snippet } = $props();
</script>

<div class="empty flex flex-col items-center gap-3 px-6 py-12 text-center">
  <IconTile {icon} size={44} tone="muted" />
  <p class="text-[15px] font-semibold text-fg">{title}</p>
  {#if children}<div class="max-w-md text-[13px] leading-relaxed text-fg-muted">{@render children()}</div>{/if}
</div>

<style>
  .empty {
    border: 1px dashed var(--glass-border);
    border-radius: var(--r-panel);
  }
</style>
