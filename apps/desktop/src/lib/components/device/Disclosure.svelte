<script lang="ts">
  // A quiet collapsible section that opens smoothly.
  import type { Snippet } from "svelte";
  import { slide } from "svelte/transition";
  import Icon from "$lib/components/common/Icon.svelte";
  import { DUR, ease, ms } from "$lib/ui/motion";

  let {
    title,
    count,
    open: initial = false,
    children,
  }: { title: string; count?: number; open?: boolean; children: Snippet } = $props();

  // svelte-ignore state_referenced_locally
  let open = $state(initial);
</script>

<div>
  <button type="button" class="head" aria-expanded={open} onclick={() => (open = !open)}>
    <span class="chev" class:open><Icon name="chevron-right" size={14} /></span>
    {title}
    {#if count !== undefined}<span class="text-fg-faint">{count}</span>{/if}
  </button>
  {#if open}
    <div class="pt-3" transition:slide={{ duration: ms(DUR.med), easing: ease }}>{@render children()}</div>
  {/if}
</div>

<style>
  .head {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
    font-weight: 600;
    color: var(--fg);
  }
  .head:hover {
    color: var(--fg);
  }
  .chev {
    display: inline-flex;
    color: var(--fg-faint);
    transition: transform var(--t-med) var(--ease-out);
  }
  .chev.open {
    transform: rotate(90deg);
  }
</style>
