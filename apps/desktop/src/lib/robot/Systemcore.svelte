<script lang="ts">
  // A robot's controller that does not run Orion yet (docs/mockups/systemcore.html): what the
  // read-only probe knows about it, the install offer, and what waits on Orion being there.
  // Once it runs Orion it is an ordinary Orion node and its screen is the board screen.
  import Page from "../ui/Page.svelte";
  import { call } from "../api/client";
  import { app, inventory, logUiError, scan } from "../stores/app.svelte";
  import { nameOf, stateOf } from "../devices/state";
  import { eventTime, when } from "../ui/format";
  import { behind } from "./facts";

  let { id }: { id: string } = $props();

  const all = $derived(inventory());
  const device = $derived(all.devices.find((d) => d.id === id));
  const boards = $derived(device ? behind(all, device) : []);
  const online = $derived(device?.presence === "online");
  const log = $derived(all.activity.filter((line) => line.device === id));
  /** A link's id in a line is written as the link's label. */
  const worded = (text: string) => (scan()?.links ?? []).reduce((out, link) => out.split(` ${link.id}`).join(` ${link.label}`), text);
  const host = (address: string) => /^[a-z+]+:\/\/([^:/]+)/.exec(address)?.[1] ?? address;

  let said = $state("");
  async function openPage() {
    said = "";
    try {
      await call("robot.open_page", { id });
    } catch (error) {
      said = error instanceof Error ? error.message : String(error);
      logUiError("systemcore.open_page", error);
    }
  }
</script>

{#if device === undefined}
  <Page title="SystemCore" crumb="Devices /" grid="grid-template-columns:minmax(0,1fr); grid-template-rows:minmax(0,1fr);">
    <section class="panel">
      <div class="pb" style="display:flex;min-height:0;padding:0">
        <div class="empty-box"><div>Atlas knows no controller under this name.</div></div>
      </div>
    </section>
  </Page>
{:else}
  {@const state = stateOf(device)}
  <Page
    title="SystemCore"
    crumb={device.robot ? `Devices / ${device.robot} /` : "Devices /"}
    grid="grid-template-columns:minmax(0,1.05fr) minmax(0,1fr); grid-template-rows:minmax(0,1fr);"
  >
    {#snippet chips()}
      <span class="chip neu">{nameOf(device)}</span>
      {#if online}
        <span class="chip warn"><span class="dot warn"></span>Orion not installed</span>
      {:else}
        <span class="chip {state.tone}"><span class="dot {state.dot}"></span>{state.word}</span>
      {/if}
      {#if said}<span class="chip err ell" style="flex:0 1 auto" title={said}>{said}</span>{/if}
    {/snippet}
    {#snippet actions()}
      {#if online && device.shown.includes("open-ui")}
        <button class="btn" onclick={openPage}>Open its own page</button>
      {/if}
    {/snippet}

    <div style="display:grid;grid-template-rows:auto minmax(0,1fr);gap:8px;min-height:0;min-width:0">
      <section class="panel" data-panel="device">
        <div class="ph">
          <h2>Device</h2><span class="grow"></span><span class="chip neu">{online ? "reached over the network" : "not reached now"}</span>
        </div>
        <div class="pb">
          <dl class="kv" style="grid-template-columns:118px minmax(0,1fr) 118px minmax(0,1fr)">
            <dt>Model</dt><dd>{device.model_name ?? device.identity.model ?? "not reported"}</dd>
            <dt>Team</dt><dd>{device.identity.attributes["team"] ?? "not reported"}</dd>
            <dt>Name</dt><dd>{nameOf(device)}</dd>
            <dt>Address</dt><dd>{host(device.identity.address)}</dd>
            <dt>First seen</dt><dd>{when(device.first_seen_ms, app.now)}</dd>
            <dt>Last seen</dt><dd>{online ? "now" : when(device.last_seen_ms, app.now)}</dd>
            <dt>Orion</dt><dd>not installed</dd>
            <dt>Lemnos</dt><dd>not installed</dd>
            <dt>Serial</dt><dd class="faint">needs Orion</dd>
            <dt>Controller OS</dt><dd class="faint">needs Orion</dd>
            <dt>Storage</dt><dd class="faint">needs Orion</dd>
            <dt>Flash jobs</dt><dd class="faint">needs Orion</dd>
          </dl>
        </div>
      </section>
      <section class="panel" style="min-height:0" data-panel="live">
        <div class="ph">
          <h2>Live</h2><span class="grow"></span><span class="kicker">read from the controller</span>
        </div>
        <div class="pb" style="display:flex;min-height:0;padding:0">
          <div class="empty-box">
            <div style="max-width:380px">
              Temperature, load and storage are read through Orion. A controller without it answers only who it is.
            </div>
          </div>
        </div>
      </section>
    </div>

    <div style="display:grid;grid-template-rows:auto auto minmax(0,1fr);gap:8px;min-height:0;min-width:0">
      <section class="panel" data-panel="install">
        <div class="ph">
          <h2>Install on this controller</h2><span class="grow"></span><span class="chip off">no package in this build</span>
        </div>
        <div class="pb flush">
          <div style="padding:8px 10px;border-bottom:1px solid var(--hairline);display:flex;flex-direction:column;gap:4px">
            <div style="display:flex;align-items:center;gap:8px"><b>Orion and Lemnos</b><span class="chip off">not in this build</span><span class="sp"></span></div>
            <div class="muted" style="font-size:11px">
              Lets Atlas reach boards on this controller and run their jobs here. This build of Atlas carries no controller package, so there is nothing to install yet.
            </div>
            <div class="faint" style="font-size:11px">
              The first install uses the controller's own package installer. Every later install is done through Orion.
            </div>
          </div>
          <div style="padding:8px 10px;display:flex;flex-direction:column;gap:4px">
            <div style="display:flex;align-items:center;gap:8px"><b>Product packages</b><span class="chip off">after Orion</span></div>
            <div class="faint" style="font-size:11px">A product for the controller is offered once Orion is on it.</div>
          </div>
        </div>
      </section>
      <section class="panel" data-panel="behind">
        <div class="ph">
          <h2>Boards behind this controller</h2><span class="grow"></span><span class="kicker">seen on USB</span>
        </div>
        <div class="pb flush tbl" style="padding:0">
          {#each boards as board (board.id)}
            {@const s = stateOf(board)}
            <div class="tr" style="grid-template-columns:minmax(0,1fr) 150px;min-height:34px">
              <div><b>{nameOf(board)}</b></div>
              <div><span class="chip {s.tone}"><span class="dot {s.dot}"></span>{s.word}</span></div>
            </div>
          {:else}
            <div class="faint" style="padding:8px 10px;font-size:11px">None seen.</div>
          {/each}
        </div>
        <div class="note" style="margin:8px 10px">
          Their jobs run on the controller and keep going if Atlas closes. Nothing is reached until Orion is installed.
        </div>
      </section>
      <section class="panel" style="min-height:0" data-panel="log">
        <div class="ph">
          <h2>Controller log</h2><span class="grow"></span><span class="kicker">what Atlas saw · newest first</span>
        </div>
        <div class="pb lines">
          {#each log as line, i (i)}
            <div class="ln" class:more-s={i >= 4}>
              <span class="t">{eventTime(line.at_ms, app.now)}</span><span class="lv-{line.level}">{line.level}</span><span>{worded(line.message)}</span>
            </div>
          {/each}
        </div>
      </section>
    </div>
  </Page>
{/if}
