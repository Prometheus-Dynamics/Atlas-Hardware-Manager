<script lang="ts">
  import type { Snippet } from "svelte";

  let { icon, title, children }: { icon: string; title: string; children?: Snippet } = $props();
</script>

<div
  class="flex flex-col items-center gap-2 rounded-container border border-dashed border-surface-700/60 px-6 py-10 text-center"
>
  <i class="fa-solid {icon} text-2xl text-surface-600" aria-hidden="true"></i>
  <p class="text-sm font-semibold text-surface-200">{title}</p>
  {#if children}<div class="max-w-md text-xs text-surface-400">{@render children()}</div>{/if}
</div>
