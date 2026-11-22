<script lang="ts">
  // A robot (docs/mockups/robot.html): its own devices, read-only, its controller, and the
  // boards behind the controller. Atlas reads these devices and changes nothing on them.
  import Page from "../ui/Page.svelte";
  import { inventory, scan } from "../stores/app.svelte";
  import { href } from "../stores/route.svelte";
  import { kindOf, nameOf, reachedBy, stateOf } from "../devices/state";
  import { count } from "../ui/format";
  import { behind, controllerOf, ownDevices, reported, whatOf } from "./facts";

  let { name }: { name: string } = $props();

  const COLUMNS = "grid-template-columns:minmax(0,1.3fr) 70px 110px minmax(0,1fr)";

  const all = $derived(inventory());
  const robot = $derived(all.robots.find((r) => r.name === name));
  const devices = $derived(ownDevices(all, name));
  const controller = $derived(controllerOf(all, name));
  const boards = $derived(controller ? behind(all, controller) : []);
  const links = $derived(scan()?.links ?? []);
  const nameById = (id: string) => {
    const device = all.devices.find((d) => d.id === id);
    return device ? nameOf(device) : undefined;
  };
  /** How a robot device is reached, in a word: its link's kind. */
  const bus = (kind: string) => (kind === "ethernet" ? "network" : kind === "gateway" ? "USB" : kind);
  const busOf = (device: (typeof devices)[number]) =>
    bus(device.links.find((sighting) => sighting.link === device.identity.link)?.kind.kind ?? "network");
  const host = (address: string) => /^[a-z+]+:\/\/([^:/]+)/.exec(address)?.[1] ?? address;
</script>

<Page title={name} crumb="Devices /" grid="grid-template-columns:minmax(0,1.25fr) minmax(0,1fr); grid-template-rows:minmax(0,1fr);">
  {#snippet chips()}
    <span class="chip neu">robot</span>
    {#if robot?.team}<span class="chip neu">team {robot.team}</span>{/if}
    {#if controller}<span class="chip neu">{count(boards.length, "board")} behind a SystemCore</span>{/if}
  {/snippet}

  <section class="panel" style="min-height:0" data-panel="robot-devices">
    <div class="ph">
      <h2>Robot devices</h2><span class="chip neu">read-only</span><span class="grow"></span><span class="kicker">read from the robot's network</span>
    </div>
    {#if robot === undefined}
      <div class="pb" style="display:flex;min-height:0;padding:0">
        <div class="empty-box"><div>There is no robot named {name}.</div></div>
      </div>
    {:else if devices.length === 0}
      <div class="pb" style="display:flex;min-height:0;padding:0">
        <div class="empty-box">
          <div style="max-width:420px">
            Nothing has answered on team {robot.team ?? "?"}'s robot network yet. Atlas keeps looking while this computer is on that network.
          </div>
        </div>
      </div>
    {:else}
      <div class="pb flush tbl fill" style="padding:0">
        <div class="tr h" style={COLUMNS}>
          <div>Device</div>
          <div>Bus</div>
          <div>State</div>
          <div>Reported</div>
        </div>
        <div class="rows">
          {#each devices as device (device.id)}
            {@const online = device.presence === "online"}
            <div class="tr" class:dim={!online} style="{COLUMNS};min-height:40px" data-robot-device={device.id}>
              <div>
                <b>{nameOf(device)}</b>
                <div class="faint ell" style="font-size:10.5px">{whatOf(device)}</div>
              </div>
              <div class="muted">{busOf(device)}</div>
              <div>
                {#if online}
                  <span class="chip ok"><span class="dot ok"></span>reporting</span>
                {:else}
                  <span class="chip off"><span class="dot off"></span>not reporting</span>
                {/if}
              </div>
              <div class={online ? "muted" : "faint"} title={reported(device)}>{reported(device) || "·"}</div>
            </div>
          {/each}
        </div>
      </div>
      <div class="note" style="margin:8px 10px">
        Atlas reads these devices and shows them. It does not change them. A robot power cycle is not a health failure.
      </div>
    {/if}
  </section>

  <div style="display:grid;grid-template-rows:auto minmax(0,1fr);gap:8px;min-height:0;min-width:0">
    <section class="panel" data-panel="controller">
      <div class="ph">
        <h2>SystemCore</h2><span class="grow"></span>
        {#if controller}
          {@const state = stateOf(controller)}
          <span class="chip {state.tone}"><span class="dot {state.dot}"></span>{controller.presence === "online" ? "Needs Orion" : state.word}</span>
        {/if}
      </div>
      <div class="pb">
        {#if controller}
          <dl class="kv">
            <dt>Name</dt><dd>{nameOf(controller)}</dd>
            <dt>Network</dt><dd>{host(controller.identity.address)}</dd>
            <dt>Boards behind</dt><dd>{boards.length === 0 ? "none seen" : `${boards.length}, seen over USB`}</dd>
            <dt>Orion</dt><dd>not installed yet</dd>
          </dl>
          <div style="margin-top:8px">
            <a class="btn pri" href={href("systemcore", controller.id)}>Open SystemCore</a>
          </div>
        {:else}
          <div class="muted" style="font-size:11px">
            No SystemCore without Orion has answered on this robot's network. A controller that runs Orion is listed as a board.
          </div>
        {/if}
      </div>
    </section>
    <section class="panel" data-panel="behind">
      <div class="ph">
        <h2>Boards behind {controller ? nameOf(controller) : "the controller"}</h2><span class="grow"></span><span class="kicker">reached once Orion is on the controller</span>
      </div>
      <div class="pb flush tbl" style="padding:0">
        {#each boards as board (board.id)}
          {@const state = stateOf(board)}
          <div class="tr" style="grid-template-columns:minmax(0,1fr) 150px;min-height:34px">
            <div>
              <b>{nameOf(board)}</b> <span class="faint">{kindOf(board)}</span>
              <div class="faint" style="font-size:10.5px">{reachedBy(board, links, nameById).join(" · ")}</div>
            </div>
            <div><span class="chip {state.tone}"><span class="dot {state.dot}"></span>{state.word}</span></div>
          </div>
        {:else}
          <div class="faint" style="padding:8px 10px;font-size:11px">None seen. A controller without Orion does not say what is plugged into it.</div>
        {/each}
      </div>
    </section>
  </div>
</Page>
