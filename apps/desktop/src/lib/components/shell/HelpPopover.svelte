<script lang="ts">
  import Button from "$lib/components/common/Button.svelte";
  import { ui } from "$lib/stores/ui.svelte";
  import { popover } from "$lib/ui/motion";

  const groups = [
    {
      title: "Anywhere",
      keys: [
        ["1 – 6", "Overview, Devices, Robots, Jobs, Releases, Settings"],
        ["S", "Scan now"],
        ["Esc", "Close the panel"],
        ["?", "Show or hide this list"],
      ],
    },
    {
      title: "Devices",
      keys: [
        ["/", "Search"],
        ["↑ ↓", "Move between devices"],
        ["Space", "Select or deselect"],
        ["Shift + click", "Select a range"],
        ["Ctrl + click", "Add to the selection"],
        ["Enter", "Open the device"],
        ["Ctrl + A", "Select all shown"],
        ["U", "Update the selection"],
      ],
    },
  ];
</script>

{#if ui.helpOpen}
  <div class="pop glass-layer" role="dialog" aria-label="Keyboard shortcuts" transition:popover>
    <div class="mb-1 flex items-center justify-between">
      <p class="text-[14px] font-semibold text-fg">Keyboard shortcuts</p>
      <Button variant="ghost" size="sm" icon="x" label="Close" onclick={() => (ui.helpOpen = false)} />
    </div>
    {#each groups as group (group.title)}
      <p class="mb-2 mt-3 text-[12px] font-medium text-fg-faint">{group.title}</p>
      <dl class="grid grid-cols-[8rem_1fr] gap-x-3 gap-y-2 text-[13px]">
        {#each group.keys as [key, text] (key)}
          <dt><kbd>{key}</kbd></dt>
          <dd class="text-fg-muted">{text}</dd>
        {/each}
      </dl>
    {/each}
  </div>
{/if}

<style>
  .pop {
    position: fixed;
    left: 64px;
    bottom: 12px;
    z-index: 45;
    width: 380px;
    padding: 14px 18px 18px;
    border-radius: var(--r-panel);
    transform-origin: bottom left;
  }
</style>
