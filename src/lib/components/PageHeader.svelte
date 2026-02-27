<script lang="ts">
  export let eyebrow: string | null = null;
  export let title: string;
  export let subtitle: string | null = null;
</script>

<header class="flex flex-col gap-4 md:flex-row md:items-center md:justify-between">
  <div class="space-y-2">
    {#if eyebrow}
      <p class="uppercase text-[0.6rem] tracking-[0.35em] text-secondary-300">
        {eyebrow}
      </p>
    {/if}

    <h1 class="text-3xl font-semibold text-surface-50">
      {title}
    </h1>

    {#if subtitle}
      <p class="text-sm text-surface-400">
        {subtitle}
      </p>
    {/if}
  </div>

  {#if $$slots.actions}
    <div class="flex flex-wrap items-center gap-2">
      <slot name="actions" />
    </div>
  {/if}
</header>
