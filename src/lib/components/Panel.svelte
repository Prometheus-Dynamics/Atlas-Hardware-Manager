<script lang="ts">
  type PanelTone = "default" | "subtle" | "contrast";

  export let tone: PanelTone = "default";
  export let eyebrow: string | null = null;
  export let title: string | null = null;
  export let subtitle: string | null = null;
  export let padded = true;

  const toneClasses: Record<PanelTone, string> = {
    default: "border border-surface-800/70 bg-surface-900/40 backdrop-blur",
    subtle: "border border-surface-800/30 bg-surface-900/20 backdrop-blur",
    contrast: "border border-primary-500/60 bg-surface-950/70 shadow-[0_12px_40px_-24px_var(--color-primary-500)]",
  };
</script>

<section class={`rounded-md ${toneClasses[tone]} ${padded ? "p-5 md:p-6" : ""} space-y-4 flex h-full min-w-0 flex-col overflow-x-hidden`}>
  {#if eyebrow || title || subtitle || $$slots.actions}
    <header class="flex min-w-0 flex-col gap-3 md:flex-row md:items-center md:justify-between">
      <div class="min-w-0 space-y-1">
        {#if eyebrow}
          <p class="uppercase text-[0.58rem] tracking-[0.32em] text-surface-500">
            {eyebrow}
          </p>
        {/if}
        {#if title}
          <h2 class="truncate text-lg font-semibold text-surface-50">
            {title}
          </h2>
        {/if}
        {#if subtitle}
          <p class="truncate text-xs text-surface-400">
            {subtitle}
          </p>
        {/if}
      </div>
      {#if $$slots.actions}
        <div class="flex shrink-0 flex-wrap items-center gap-2">
          <slot name="actions" />
        </div>
      {/if}
    </header>
  {/if}

  <div class="min-w-0 space-y-4 overflow-x-hidden">
    <slot />
  </div>
</section>
