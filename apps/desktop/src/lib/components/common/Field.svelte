<script lang="ts">
  // A labelled form row: label, the control (children), and an optional hint.
  import type { Snippet } from "svelte";

  let {
    label,
    hint,
    inline = false,
    children,
    class: extra = "",
  }: { label: string; hint?: string; inline?: boolean; children: Snippet; class?: string } = $props();
</script>

{#if inline}
  <label class="grid grid-cols-[1fr_auto] items-center gap-x-4 gap-y-0.5 {extra}">
    <span class="text-[13px] font-medium text-fg">{label}</span>
    <span class="row-span-2 self-center">{@render children()}</span>
    {#if hint}<span class="text-[12px] text-fg-faint">{hint}</span>{/if}
  </label>
{:else}
  <label class="flex flex-col gap-1.5 {extra}">
    <span class="text-[12.5px] font-medium text-fg-muted">{label}</span>
    {@render children()}
    {#if hint}<span class="text-[12px] text-fg-faint">{hint}</span>{/if}
  </label>
{/if}
