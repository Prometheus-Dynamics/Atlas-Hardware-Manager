<script lang="ts">
  let {
    value,
    tone = "primary",
    label,
  }: { value: number | null; tone?: "primary" | "success" | "warning" | "error"; label?: string } = $props();

  const fill = {
    primary: "bg-secondary-400",
    success: "bg-success-500",
    warning: "bg-warning-500",
    error: "bg-error-500",
  };
</script>

<div
  class="h-1.5 w-full overflow-hidden rounded-base bg-surface-800"
  role="progressbar"
  aria-label={label}
  aria-valuemin="0"
  aria-valuemax="100"
  aria-valuenow={value === null ? undefined : Math.round(value * 100)}
>
  {#if value === null}
    <div class="h-full w-1/3 animate-pulse {fill[tone]}"></div>
  {:else}
    <div class="h-full transition-[width] duration-300 {fill[tone]}" style="width: {Math.round(value * 100)}%"></div>
  {/if}
</div>
