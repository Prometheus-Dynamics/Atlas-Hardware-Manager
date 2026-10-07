<script lang="ts">
  import Sheet from "#lib/components/common/Sheet.svelte";
  import DevicePanel from "#lib/components/device/DevicePanel.svelte";
  import RobotEditor from "#lib/components/robots/RobotEditor.svelte";
  import UpdateFlow from "#lib/components/update/UpdateFlow.svelte";
  import { ui } from "#lib/stores/ui.svelte.ts";

  const label = $derived(
    ui.panel?.kind === "device" ? "Device" : ui.panel?.kind === "robot" ? "Robot" : ui.panel?.kind === "update" ? ui.panel.title : "",
  );
  // The device panel grows with the window, like clamp(480px, 38vw, 820px):
  // live readings get room, and past ~700px its tabs lay out in two columns.
  let viewport = $state(1280);
  const deviceWidth = $derived(Math.round(Math.min(820, Math.max(480, viewport * 0.38))));
  const width = $derived(ui.panel?.kind === "device" ? deviceWidth : 460);
</script>

<svelte:window bind:innerWidth={viewport} />

{#if ui.panel}
  <Sheet label="{label} details" {width} onclose={() => ui.close()} scroll={ui.panel.kind !== "device"}>
    {#if ui.panel.kind === "device"}
      {#key ui.panel.key + (ui.panel.tab ?? "")}
        <DevicePanel key={ui.panel.key} initialTab={ui.panel.tab} />
      {/key}
    {:else if ui.panel.kind === "robot"}
      {#key ui.panel.name}
        <RobotEditor name={ui.panel.name} />
      {/key}
    {:else if ui.panel.kind === "update"}
      {#key ui.panel.request}
        <div class="px-6 pb-6 pt-5">
          <h2 class="mb-5 pr-10 text-[18px] font-semibold text-fg">{ui.panel.title}</h2>
          <UpdateFlow request={ui.panel.request} onstarted={() => ui.close()} />
        </div>
      {/key}
    {/if}
  </Sheet>
{/if}
