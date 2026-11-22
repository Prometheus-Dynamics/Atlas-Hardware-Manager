<script lang="ts">
  // The shell: the sidebar and the current screen. Screens appear at once; nothing animates in.
  import Sidebar from "./lib/shell/Sidebar.svelte";
  import Devices from "./lib/devices/Devices.svelte";
  import NotBuilt from "./lib/shell/NotBuilt.svelte";
  import { route } from "./lib/stores/route.svelte";
</script>

<div class="app">
  <Sidebar />
  {#if route.page === "devices"}
    <Devices />
  {:else if route.page === "board"}
    <NotBuilt title="Board" crumb="Devices /" item="F4" />
  {:else if route.page === "flash" || route.page === "disk"}
    <NotBuilt title="Flash a board" crumb="Atlas /" item="F5" />
  {:else if route.page === "robot" || route.page === "systemcore"}
    <NotBuilt title="Robot" crumb="Devices /" item="F6" />
  {:else}
    <NotBuilt title="Settings" crumb="Atlas /" item="F7" />
  {/if}
</div>
