<script lang="ts">
  // A live number. Readings snap to their new value: no tween, so a slow
  // machine never spends frames redrawing digits.
  let { value, decimals = 0 }: { value: number; decimals?: number } = $props();

  const text = $derived(value.toLocaleString(undefined, { minimumFractionDigits: decimals, maximumFractionDigits: decimals }));
</script>

<span class="tabular-nums">{text}</span>
