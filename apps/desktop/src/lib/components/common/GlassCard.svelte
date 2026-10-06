<script lang="ts">
  // A glass panel with an optional header (icon, title, subline, actions).
  import type { Snippet } from "svelte";
  import type { IconName } from "#lib/ui/icons.ts";
  import IconTile from "./IconTile.svelte";

  let {
    title,
    subtitle,
    icon,
    actions,
    children,
    large = false,
    pad = true,
    class: extra = "",
  }: {
    title?: string;
    subtitle?: string;
    icon?: IconName;
    actions?: Snippet;
    children: Snippet;
    large?: boolean;
    pad?: boolean;
    class?: string;
  } = $props();
</script>

<section class="glass card {extra}" class:large>
  {#if title || actions}
    <header class="head flex items-center gap-3 px-5 py-3">
      {#if icon}<IconTile {icon} size={28} />{/if}
      <div class="min-w-0 flex-1">
        {#if title}<h2 class="truncate text-[14px] font-semibold text-fg">{title}</h2>{/if}
        {#if subtitle}<p class="truncate text-[12.5px] text-fg-muted">{subtitle}</p>{/if}
      </div>
      {#if actions}<div class="flex shrink-0 items-center gap-2">{@render actions()}</div>{/if}
    </header>
  {/if}
  <div class:body={pad}>{@render children()}</div>
</section>

<style>
  .card.large {
    border-radius: var(--r-panel);
  }
  /* A defined header strip: every card reads as titled and bounded. */
  .head {
    border-bottom: 1px solid var(--hairline);
  }
  .body {
    padding: 16px 20px 18px;
  }
</style>
