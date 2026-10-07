<script lang="ts">
  // A glass panel with an optional header (icon, title, subline, actions).
  // `fill`: the card takes the height its parent gives it and its body
  // scrolls inside (or, with `scroll={false}`, the body just gets the room).
  import type { Snippet } from "svelte";
  import type { IconName } from "#lib/ui/icons.ts";
  import Region from "#lib/components/layout/Region.svelte";
  import IconTile from "./IconTile.svelte";

  let {
    title,
    subtitle,
    icon,
    actions,
    children,
    large = false,
    pad = true,
    fill = false,
    scroll = true,
    class: extra = "",
  }: {
    title?: string;
    subtitle?: string;
    icon?: IconName;
    actions?: Snippet;
    children: Snippet;
    large?: boolean;
    pad?: boolean;
    fill?: boolean;
    scroll?: boolean;
    class?: string;
  } = $props();

  const body = $derived(pad ? "px-5 pb-[18px] pt-4" : "");
</script>

<section class="glass card {extra}" class:large class:fill>
  {#if title || actions}
    <header class="head flex shrink-0 items-center gap-3 px-5 py-3">
      {#if icon}<IconTile {icon} size={28} />{/if}
      <div class="min-w-0 flex-1">
        {#if title}<h2 class="truncate text-[14px] font-semibold text-fg" {title}>{title}</h2>{/if}
        {#if subtitle}<p class="truncate text-[12.5px] text-fg-muted" title={subtitle}>{subtitle}</p>{/if}
      </div>
      {#if actions}<div class="flex shrink-0 items-center gap-2">{@render actions()}</div>{/if}
    </header>
  {/if}
  {#if fill && scroll}
    <Region inner={body}>{@render children()}</Region>
  {:else if fill}
    <div class="relative min-h-0 flex-1 {body}">{@render children()}</div>
  {:else}
    <div class={body}>{@render children()}</div>
  {/if}
</section>

<style>
  .card.large {
    border-radius: var(--r-panel);
  }
  .card.fill {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
  /* A defined header strip: every card reads as titled and bounded. */
  .head {
    border-bottom: 1px solid var(--hairline);
  }
</style>
