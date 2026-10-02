<script lang="ts">
  import Button from "$lib/components/common/Button.svelte";
  import Icon from "$lib/components/common/Icon.svelte";
  import { ui } from "$lib/stores/ui.svelte";

  let { robot, count }: { robot: string | null; count: number } = $props();
</script>

<div class="flex items-center gap-2 pb-2.5 pt-1">
  <Icon name={robot ? "robot" : "circle-dashed"} size={16} class="text-fg-faint" />
  <h2 class="text-[13px] font-semibold text-fg">{robot ?? "Not on a robot"}</h2>
  <span class="text-[12.5px] text-fg-faint">{count}</span>
  {#if robot}
    <Button variant="ghost" size="sm" class="ml-auto" onclick={() => (ui.robot = robot)}>Show only</Button>
  {/if}
</div>
