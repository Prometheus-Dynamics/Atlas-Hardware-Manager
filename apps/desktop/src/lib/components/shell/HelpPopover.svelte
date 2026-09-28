<script lang="ts">
  import { ui } from "$lib/stores/ui.svelte";

  const groups = [
    {
      title: "Anywhere",
      keys: [
        ["1 – 5", "Inventory, Robots, Jobs, Releases, Settings"],
        ["S", "Scan now"],
        ["Esc", "Close the side panel"],
        ["?", "Show or hide this list"],
      ],
    },
    {
      title: "Inventory",
      keys: [
        ["/", "Focus the filter"],
        ["↑ ↓", "Move between rows"],
        ["Space", "Select or deselect the row"],
        ["Shift + click", "Select a range"],
        ["Enter", "Open device details"],
        ["Ctrl + A", "Select all shown"],
        ["U", "Update the selection"],
      ],
    },
  ];
</script>

{#if ui.helpOpen}
  <div
    class="fixed right-4 top-14 z-40 w-[26rem] rounded-container border border-surface-700 bg-surface-900 p-4"
    role="dialog"
    aria-label="Keyboard shortcuts"
  >
    <div class="mb-2 flex items-center justify-between">
      <p class="micro-label">Keyboard shortcuts</p>
      <button type="button" class="text-surface-400 hover:text-surface-50" aria-label="Close" onclick={() => (ui.helpOpen = false)}>
        <i class="fa-solid fa-xmark" aria-hidden="true"></i>
      </button>
    </div>
    {#each groups as group (group.title)}
      <p class="mb-1 mt-3 text-[0.65rem] font-semibold uppercase tracking-[0.14em] text-surface-300">{group.title}</p>
      <dl class="grid grid-cols-[7.5rem_1fr] gap-x-3 gap-y-1 text-xs">
        {#each group.keys as [key, text] (key)}
          <dt><kbd class="rounded-base border border-surface-700 bg-surface-800 px-1 font-mono text-[0.65rem]">{key}</kbd></dt>
          <dd class="text-surface-300">{text}</dd>
        {/each}
      </dl>
    {/each}
  </div>
{/if}
