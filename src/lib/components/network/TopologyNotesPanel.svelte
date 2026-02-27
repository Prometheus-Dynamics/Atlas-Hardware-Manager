<script lang="ts">
  import { Panel } from "$lib/components";
  import type { TopologyNote } from "./types";

  export let notes: TopologyNote[] = [];
</script>

<Panel eyebrow="Topology Notes" subtitle="Highlights from the latest network audit.">
  <ul class="space-y-3">
    {#each notes as note (note.id)}
      <li class="rounded border border-surface-800/60 bg-surface-900/40 p-4">
        <p class="text-sm font-semibold text-surface-100">{note.title}</p>
        <p class="text-xs text-surface-400">{note.detail}</p>
      </li>
    {/each}
  </ul>
</Panel>
