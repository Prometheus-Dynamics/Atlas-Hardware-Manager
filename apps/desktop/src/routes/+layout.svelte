<script lang="ts">
  import "../app.css";
  import HelpPopover from "$lib/components/shell/HelpPopover.svelte";
  import JobsTray from "$lib/components/shell/JobsTray.svelte";
  import NavRail from "$lib/components/shell/NavRail.svelte";
  import SidePanel from "$lib/components/shell/SidePanel.svelte";
  import Toasts from "$lib/components/shell/Toasts.svelte";
  import { handleShortcut } from "$lib/components/shell/shortcuts";
  import { startSync } from "$lib/stores/sync";

  let { children } = $props();

  // One event subscription for the whole app.
  $effect(() => startSync());
</script>

<svelte:window onkeydown={handleShortcut} />

<div class="app flex h-full">
  <div class="glow" aria-hidden="true"></div>
  <NavRail />
  <div class="relative flex min-w-0 flex-1 flex-col">
    <div class="relative min-h-0 flex-1">
      <main class="h-full overflow-y-auto px-8 pb-10 pt-7">
        <div class="mx-auto max-w-[1360px]">
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
  /* Faint light behind the glass: warm top right, cool bottom left. */
  .glow {
    position: absolute;
    inset: 0;
    z-index: -1;
    pointer-events: none;
    background:
      radial-gradient(900px 600px at 92% -8%, var(--glow-warm), transparent 70%),
      radial-gradient(800px 600px at 8% 108%, var(--glow-cool), transparent 70%);
  }
</style>
