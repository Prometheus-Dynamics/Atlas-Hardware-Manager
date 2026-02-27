<script lang="ts">
  type UsageTileAccent = "primary" | "secondary" | "tertiary" | "success" | "warning" | "error";

  export let label: string;
  export let value: string | number;
  export let unit: string | null = null;
  export let change: string | null = null;
  export let accent: UsageTileAccent = "primary";

  const accentClasses: Record<UsageTileAccent, string> = {
    primary: "border-primary-500/30 bg-primary-500/10 text-primary-100",
    secondary: "border-secondary-500/30 bg-secondary-500/10 text-secondary-100",
    tertiary: "border-tertiary-500/30 bg-tertiary-500/10 text-tertiary-100",
    success: "border-success-500/30 bg-success-500/10 text-success-100",
    warning: "border-warning-500/30 bg-warning-500/10 text-warning-900",
    error: "border-error-500/30 bg-error-500/10 text-error-100",
  };
</script>

<article class={`space-y-3 rounded-md border p-4 ${accentClasses[accent]}`}>
  <p class="uppercase text-[0.55rem] tracking-[0.34em] text-surface-400">
    {label}
  </p>

  <div class="flex items-end gap-2">
    <span class="text-3xl font-semibold text-surface-50">{value}</span>
    {#if unit}
      <span class="pb-1 text-xs text-surface-400">{unit}</span>
    {/if}
  </div>

  {#if change}
    <p class="text-xs text-surface-400">{change}</p>
  {/if}

  {#if $$slots.default}
    <div>
      <slot />
    </div>
  {/if}
</article>
