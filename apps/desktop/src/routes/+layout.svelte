<script lang="ts">
  import "../app.css";
  let { children } = $props();
</script>

<div class="h-full" data-theme="helios">
  {@render children()}
</div>
