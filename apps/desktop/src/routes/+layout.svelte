<script lang="ts">
  import "../app.css";
  import HelpPopover from "#lib/components/shell/HelpPopover.svelte";
  import JobsTray from "#lib/components/shell/JobsTray.svelte";
  import NavRail from "#lib/components/shell/NavRail.svelte";
  import SidePanel from "#lib/components/shell/SidePanel.svelte";
  import Toasts from "#lib/components/shell/Toasts.svelte";
  import { handleShortcut } from "#lib/components/shell/shortcuts.ts";
  import { startSync } from "#lib/stores/sync.ts";

  let { children } = $props();

  // One event subscription for the whole app.
  $effect(() => startSync());
</script>

<svelte:window onkeydown={handleShortcut} />

<div class="app flex h-full">
  <NavRail />
  <div class="relative flex min-w-0 flex-1 flex-col">
    <div class="relative min-h-0 flex-1">
      <main class="h-full overflow-y-auto px-6 pb-10 pt-7 xl:px-8 min-[1800px]:px-12">
        <div class="mx-auto max-w-[2200px]">
          {@render children()}
        </div>
      </main>
      <SidePanel />
      <Toasts />
    </div>
    <JobsTray />
  </div>
  <HelpPopover />
</div>

<style>
  .app {
    position: relative;
    background: var(--bg);
    isolation: isolate;
  }
</style>
