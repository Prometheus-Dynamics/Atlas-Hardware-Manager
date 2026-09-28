<script lang="ts">
  import Panel from "$lib/components/common/Panel.svelte";
  import { releases } from "$lib/stores/releases.svelte";
  import ReleaseRow from "./ReleaseRow.svelte";

  let { family }: { family: string } = $props();

  const entries = $derived(releases.forFamily(family));
  // Fixed widths keep columns aligned across family groups.
  const columns = [
    ["Version", "w-32"],
    ["Channel", "w-24"],
    ["Signature", "w-28"],
    ["Artifact", ""],
    ["Size", "w-20"],
    ["File", "w-40"],
    ["", "w-20"],
    ["", "w-12"],
  ];
</script>

<Panel eyebrow="Family" title={family} class="overflow-hidden">
  <div class="-m-4 overflow-x-auto">
    <table class="w-full min-w-[52rem] table-fixed border-collapse">
      <thead>
        <tr class="border-b border-surface-800 text-left">
          {#each columns as [column, width], i (i)}
            <th class="micro-label px-2 py-1.5 font-medium first:px-3 {width} {column === 'Size' ? 'text-right' : ''}">{column}</th>
          {/each}
        </tr>
      </thead>
      <tbody>
        {#each entries as entry (entry.id)}
          <ReleaseRow {entry} />
        {/each}
      </tbody>
    </table>
  </div>
</Panel>
