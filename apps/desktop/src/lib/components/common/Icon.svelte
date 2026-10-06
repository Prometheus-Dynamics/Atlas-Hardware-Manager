<script lang="ts">
  // A Tabler outline icon by name, sized to sit on the text baseline.
  import { ICONS, type IconName } from "#lib/ui/icons.ts";

  let {
    name,
    size = 16,
    stroke = 1.75,
    class: extra = "",
    label,
  }: { name: IconName; size?: number; stroke?: number; class?: string; label?: string } = $props();

  const Component = $derived(ICONS[name]);
</script>

<Component
  {size}
  {stroke}
  class="icon shrink-0 {extra}"
  aria-hidden={label ? undefined : "true"}
  aria-label={label}
  role={label ? "img" : undefined}
/>
