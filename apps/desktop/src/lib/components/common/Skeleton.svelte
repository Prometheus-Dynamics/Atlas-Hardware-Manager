<script lang="ts">
  // A shimmering placeholder bar while data loads.
  let { width = "100%", height = 12, round = false }: { width?: string; height?: number; round?: boolean } = $props();
</script>

<span
  class="shimmer block"
  style="width: {width}; height: {height}px; {round ? 'border-radius: 999px;' : ''}"
  aria-hidden="true"
></span>
