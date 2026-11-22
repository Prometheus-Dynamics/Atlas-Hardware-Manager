<script lang="ts">
  // The shell: the sidebar and the current screen. Screens appear at once; nothing animates in.
  import Sidebar from "./lib/shell/Sidebar.svelte";
  import Devices from "./lib/devices/Devices.svelte";
  import Board from "./lib/board/Board.svelte";
  import Flash from "./lib/flash/Flash.svelte";
  import Disk from "./lib/flash/Disk.svelte";
  import NotBuilt from "./lib/shell/NotBuilt.svelte";
  import { route } from "./lib/stores/route.svelte";
</script>

<div class="app">
  <Sidebar />
  {#if route.page === "devices"}
    <Devices />
  {:else if route.page === "board"}
    {#key route.arg}<Board id={route.arg} />{/key}
  {:else if route.page === "flash"}
    <Flash id={route.arg} />
  {:else if route.page === "disk"}
    <Disk id={route.arg} />
  {:else if route.page === "robot" || route.page === "systemcore"}
    <NotBuilt title="Robot" crumb="Devices /" item="F6" />
  {:else}
    <NotBuilt title="Settings" crumb="Atlas /" item="F7" />
  {/if}
</div>
