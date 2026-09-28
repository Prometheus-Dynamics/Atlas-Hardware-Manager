<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    eyebrow,
    title,
    subtitle,
    actions,
  }: { eyebrow: string; title: string; subtitle?: string; actions?: Snippet } = $props();
</script>

<header class="flex flex-wrap items-end justify-between gap-3">
  <div class="min-w-0">
    <p class="micro-label">{eyebrow}</p>
    <h1 class="text-xl font-semibold text-surface-50">{title}</h1>
    {#if subtitle}<p class="text-xs text-surface-400">{subtitle}</p>{/if}
  </div>
  {#if actions}<div class="flex items-center gap-2">{@render actions()}</div>{/if}
</header>
