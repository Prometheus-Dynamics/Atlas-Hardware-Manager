<script lang="ts">
  // The tree: devices (a robot, its controller, the boards behind it, then boards on this
  // computer), offline, disks, tools. 22 px rows, 14 px per level, as in the mockups.
  import Icon from "../ui/Icon.svelte";
  import { app, inventory } from "../stores/app.svelte";
  import { href, route } from "../stores/route.svelte";
  import { argOf, nameOf, pageOf, stateOf } from "../devices/state";
  import { tree } from "../devices/tree";

  const groups = $derived(tree(inventory()));
  const shortVersion = $derived(app.version.split("+")[0] ?? "");
  // The flash flow is one tool: its row under Tools is the selected one whichever board it is
  // working on, as the mockups draw it.
  const on = (page: string, arg = "") => page !== "flash" && route.page === page && (arg === "" || route.arg === arg);
</script>

<aside class="side">
  <div class="brand"><b>Atlas</b><span class="ell" title={app.version}>{shortVersion}</span></div>

  <div class="sec up">Devices</div>
  <a class="nav" class:on={route.page === "devices"} href={href("devices")} style="--d:0">
    <Icon name="chip" /><span class="t">All devices</span>
  </a>
  {#if inventory().devices.length === 0}
    <a class="nav off" href={href("devices")} style="--d:1"><span class="t">No devices yet</span></a>
  {/if}
  {#each groups.robots as robot (robot.name)}
    <a class="nav" class:on={on("robot", robot.name)} href={href("robot", robot.name)} style="--d:1">
      <Icon name="robot" /><span class="t">{robot.name}</span><span class="meta">robot</span>
    </a>
    {#each robot.entries as entry (entry.device.id)}
      {@const state = stateOf(entry.device)}
      <a
        class="nav"
        class:on={entry.page !== "robot" && on(entry.page, entry.device.id)}
        href={href(entry.page, argOf(entry.device))}
        style="--d:{2 + entry.depth}"
      >
        <span class="dot {state.dot}"></span><span class="t">{nameOf(entry.device)}</span>
      </a>
    {/each}
  {/each}
  {#each groups.boards as device (device.id)}
    {@const state = stateOf(device)}
    <a class="nav" class:on={on(pageOf(device), device.id)} href={href(pageOf(device), device.id)} style="--d:1">
      <span class="dot {state.dot}"></span><span class="t">{nameOf(device)}</span>
    </a>
  {/each}

  {#if groups.offline.length > 0}
    <div class="sec up">Offline</div>
    {#each groups.offline as device (device.id)}
      <a class="nav off" class:on={on(pageOf(device), device.id)} href={href(pageOf(device), device.id)} style="--d:1">
        <span class="dot off"></span><span class="t">{nameOf(device)}</span>
      </a>
    {/each}
  {/if}

  {#if groups.disks.length > 0}
    <div class="sec up">Disks</div>
    {#each groups.disks as device (device.id)}
      <a class="nav" class:on={on("disk", device.id)} href={href("disk", device.id)} style="--d:1">
        <span class="dot info"></span><span class="t">{nameOf(device)}</span>
      </a>
    {/each}
  {/if}

  <div class="sec up">Tools</div>
  <a class="nav" class:on={route.page === "flash"} href={href("flash")} style="--d:0">
    <Icon name="bolt" /><span class="t">Flash a board</span>
  </a>
  <a class="nav" class:on={route.page === "settings"} href={href("settings")} style="--d:0">
    <Icon name="gear" /><span class="t">Settings</span>
  </a>
</aside>
