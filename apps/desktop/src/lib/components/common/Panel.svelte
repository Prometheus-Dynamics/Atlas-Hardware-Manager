<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    eyebrow,
    title,
    subtitle,
    actions,
    children,
    class: extra = "",
  }: {
    eyebrow?: string;
    title?: string;
    subtitle?: string;
    actions?: Snippet;
    children: Snippet;
    class?: string;
  } = $props();
</script>

<section class="rounded-container border border-surface-800 bg-surface-900/40 {extra}">
  {#if eyebrow || title || actions}
    <header class="flex items-center justify-between gap-3 border-b border-surface-800 px-4 py-2">
      <div class="min-w-0">
        {#if eyebrow}<p class="micro-label">{eyebrow}</p>{/if}
        {#if title}<h2 class="truncate text-sm font-semibold text-surface-100">{title}</h2>{/if}
        {#if subtitle}<p class="text-xs text-surface-400">{subtitle}</p>{/if}
      </div>
      {#if actions}<div class="flex shrink-0 items-center gap-2">{@render actions()}</div>{/if}
    </header>
  {/if}
  <div class="p-4">{@render children()}</div>
</section>
