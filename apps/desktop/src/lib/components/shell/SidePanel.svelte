<script lang="ts">
  import DevicePanel from "$lib/components/device/DevicePanel.svelte";
  import RobotEditor from "$lib/components/robots/RobotEditor.svelte";
  import UpdateFlow from "$lib/components/update/UpdateFlow.svelte";
  import { ui } from "$lib/stores/ui.svelte";

  const title = $derived(
    ui.panel?.kind === "device" ? "Device" : ui.panel?.kind === "robot" ? "Robot" : ui.panel?.kind === "update" ? ui.panel.title : "",
  );
</script>

{#if ui.panel}
  <aside
    class="flex w-[25rem] shrink-0 flex-col border-l border-surface-800 bg-surface-900/90"
    aria-label="{title} details"
  >
    <div class="flex h-9 shrink-0 items-center justify-between border-b border-surface-800 px-4">
      <p class="micro-label">{title}</p>
      <button type="button" class="text-surface-400 hover:text-surface-50" onclick={() => ui.close()} aria-label="Close panel (Esc)" title="Close (Esc)">
        <i class="fa-solid fa-xmark" aria-hidden="true"></i>
      </button>
    </div>
    <div class="min-h-0 flex-1 overflow-y-auto">
      {#if ui.panel.kind === "device"}
        {#key ui.panel.key}
          <DevicePanel key={ui.panel.key} initialTab={ui.panel.tab} />
        {/key}
      {:else if ui.panel.kind === "robot"}
        {#key ui.panel.name}
          <RobotEditor name={ui.panel.name} />
        {/key}
      {:else if ui.panel.kind === "update"}
        {#key ui.panel.request}
          <div class="p-4">
            <UpdateFlow request={ui.panel.request} onstarted={() => ui.close()} />
          </div>
        {/key}
      {/if}
    </div>
  </aside>
{/if}
