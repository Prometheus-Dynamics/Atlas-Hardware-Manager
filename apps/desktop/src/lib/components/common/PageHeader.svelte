<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    title,
    subtitle,
    actions,
    heading,
  }: { title?: string; subtitle?: string; actions?: Snippet; heading?: Snippet } = $props();
</script>

<header class="flex flex-wrap items-center justify-between gap-x-4 gap-y-3">
  <div class="min-w-0 flex-1">
    {#if heading}{@render heading()}{:else}<h1 class="truncate text-[22px] font-semibold tracking-[-0.01em] text-fg">{title}</h1>{/if}
    {#if subtitle}<p class="mt-0.5 text-[13px] text-fg-muted">{subtitle}</p>{/if}
  </div>
  {#if actions}<div class="flex items-center gap-2">{@render actions()}</div>{/if}
</header>
