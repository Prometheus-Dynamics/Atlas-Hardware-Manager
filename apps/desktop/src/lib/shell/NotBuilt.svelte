<script lang="ts">
  // A screen whose item has not landed yet. It says so in the design's empty box and nothing else.
  import Page from "../ui/Page.svelte";
  let { title, crumb, item }: { title: string; crumb: string; item: string } = $props();
</script>

<Page {title} {crumb} grid="grid-template-columns:minmax(0,1fr); grid-template-rows:minmax(0,1fr);">
  <section class="panel">
    <div class="pb" style="display:flex;min-height:0;padding:0">
      <div class="empty-box"><div>This screen arrives with {item}.</div></div>
    </div>
  </section>
</Page>
