<script lang="ts">
  // Rows that are still being read (docs/mockups/states.html, the Loading column): the row's
  // own height and columns, with a grey bar where its words will be. Drawn before the core's
  // first answer, so no page says "none" about something it has not read yet.
  let {
    rows = 3,
    columns = "grid-template-columns:minmax(0,1fr)",
    cells = 1,
    row = "tr",
    style = "",
  }: {
    rows?: number;
    /** The row's `grid-template-columns`, as the list it stands in for has them. */
    columns?: string;
    /** How many of the row's cells hold a bar. */
    cells?: number;
    /** The row's classes, as the list's own rows have them. */
    row?: string;
    style?: string;
  } = $props();
  const WIDTHS = [70, 52, 61, 44];
</script>

{#each { length: rows } as _, r (r)}
  <div class={row} style="{columns};{style}" data-loading>
    {#each { length: cells } as _, c (c)}
      <div style="flex:1;min-width:0"><div class="ph-skel" style="width:{WIDTHS[(r + c) % WIDTHS.length]}%"></div></div>
    {/each}
  </div>
{/each}
