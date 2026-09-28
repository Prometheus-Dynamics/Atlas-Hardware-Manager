<script lang="ts">
  import "../app.css";
  import HelpPopover from "$lib/components/shell/HelpPopover.svelte";
  import JobsTray from "$lib/components/shell/JobsTray.svelte";
  import NavRail from "$lib/components/shell/NavRail.svelte";
  import SidePanel from "$lib/components/shell/SidePanel.svelte";
  import Toasts from "$lib/components/shell/Toasts.svelte";
  import TopBar from "$lib/components/shell/TopBar.svelte";
  import { handleShortcut } from "$lib/components/shell/shortcuts";
  import { startSync } from "$lib/stores/sync";

  let { children } = $props();

  // One event subscription for the whole app.
  $effect(() => startSync());
</script>

<svelte:window onkeydown={handleShortcut} />

<div class="flex h-full bg-surface-950 text-surface-100" data-theme="helios">
  <NavRail />
  <div class="flex min-w-0 flex-1 flex-col">
    <TopBar />
    <div class="flex min-h-0 flex-1">
      <main class="min-w-0 flex-1 overflow-y-auto bg-surface-900/30 p-4">
        {@render children()}
      </main>
      <SidePanel />
    </div>
    <JobsTray />
  </div>
  <Toasts />
  <HelpPopover />
</div>
