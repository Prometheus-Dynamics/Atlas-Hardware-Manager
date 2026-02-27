<script lang="ts">
  type SegmentAccent =
    | "primary"
    | "secondary"
    | "tertiary"
    | "success"
    | "warning"
    | "error"
    | "surface";

  interface Segment {
    label: string;
    value: number;
    accent?: SegmentAccent;
  }

  export let segments: Segment[] = [];

  const accentClasses: Record<SegmentAccent, string> = {
    primary: "bg-primary-500 text-primary-contrast-light",
    secondary: "bg-secondary-500 text-secondary-contrast-light",
    tertiary: "bg-tertiary-500 text-tertiary-contrast-light",
    success: "bg-success-500 text-success-contrast-light",
    warning: "bg-warning-500 text-warning-contrast-dark",
    error: "bg-error-500 text-error-contrast-light",
    surface: "bg-surface-800 text-surface-contrast-light",
  };

  $: total = segments.reduce((sum, segment) => sum + segment.value, 0) || 1;
</script>

<div class="space-y-3">
  <div class="flex overflow-hidden rounded-sm border border-surface-800/60 bg-surface-900/60">
    {#each segments as segment (segment.label)}
      <div
        class={`relative flex items-center justify-center text-[0.55rem] uppercase tracking-[0.32em] ${accentClasses[segment.accent ?? "primary"]}`}
        style:flex-basis={`${(segment.value / total) * 100}%`}
        style:flex-grow={`${segment.value}`}
      >
        <span class="px-2 py-[0.15rem]">{segment.label}</span>
      </div>
    {/each}
  </div>

  <div class="grid gap-2 text-[0.55rem] uppercase tracking-[0.32em] text-surface-500 md:grid-cols-2">
    {#each segments as segment (segment.label)}
      <div class="flex items-center gap-2">
        <span class={`h-2 w-6 rounded-sm ${accentClasses[segment.accent ?? "primary"]}`}></span>
        <span>{segment.label}</span>
        <span class="ml-auto text-surface-400">{Math.round((segment.value / total) * 100)}%</span>
      </div>
    {/each}
  </div>
</div>
