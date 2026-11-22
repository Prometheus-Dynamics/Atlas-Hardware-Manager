<script lang="ts">
  // The device list (docs/mockups/devices.html, devices-empty.html): every record in every
  // state in one list, grouped as the tree groups them, with what Atlas can do for each now.
  // One record per board; its links are chips in the row.
  import Page from "../ui/Page.svelte";
  import Icon from "../ui/Icon.svelte";
  import { call } from "../api/client";
  import type { ActivityLine, Device } from "../api/types";
  import { app, breadcrumb, inventory, scan } from "../stores/app.svelte";
  import { go, href } from "../stores/route.svelte";
  import { count, eventTime, when } from "../ui/format";
  import { argOf, canFlash, kindOf, nameOf, pageOf, reachedBy, stateOf } from "./state";
  import { tree } from "./tree";

  const COLUMNS = "grid-template-columns: minmax(0,1.5fr) 196px minmax(0,1.3fr) 84px 64px";
  const ONE = "grid-template-columns:minmax(0,1fr)";
  /** The events panel is 150 px: its header, its padding, and six 17 px lines. */
  const EVENT_LINES = 6;

  type Row = { device: Device; child: boolean };
  type Group = { title: string; rows: Row[] };

  const all = $derived(inventory());
  const groups = $derived.by((): Group[] => {
    const t = tree(all);
    const plain = (devices: Device[]): Row[] => devices.map((device) => ({ device, child: false }));
    return [
      ...t.robots.map((robot) => ({
        title: `${robot.name} · robot (its devices are read-only)`,
        rows: robot.entries.map((entry) => ({ device: entry.device, child: entry.depth > 0 })),
      })),
      { title: "Boards on this computer", rows: plain(t.boards) },
      { title: "Removable disks", rows: plain(t.disks) },
      { title: "Offline", rows: plain(t.offline) },
    ].filter((group) => group.rows.length > 0);
  });

  const links = $derived(scan()?.links ?? []);
  const nameById = (id: string) => {
    const device = all.devices.find((d) => d.id === id);
    return device ? nameOf(device) : undefined;
  };
  const paused = $derived(scan()?.paused ?? false);
  const problems = $derived(scan()?.problems ?? []);

  /** Recent events, newest first: a discovery that is failing now, then what happened to devices.
   *  A link's id in a line is written as the link's label. */
  const events = $derived.by(() => {
    const label = (text: string) =>
      links.reduce((out, link) => out.replace(new RegExp(`(^|\\s)${link.id.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}(?=$|\\s)`), `$1${link.label}`), text);
    const failing = problems.map((problem) => ({
      time: "now",
      level: "warn",
      text: `Looking on ${links.find((l) => l.id === problem.link)?.label ?? problem.link} is failing (${problem.error}). Try ${problem.attempt + 1} in ${Math.max(1, Math.round(problem.retry_in_ms / 1000))} s.`,
    }));
    const happened = all.activity.map((line: ActivityLine) => ({
      time: eventTime(line.at_ms, app.now),
      level: line.level,
      text: `${nameById(line.device) ?? line.name}: ${label(line.message)}`,
    }));
    return [...failing, ...happened].slice(0, EVENT_LINES);
  });

  function open(device: Device) {
    go(pageOf(device), argOf(device));
  }

  function flash(event: MouseEvent, device: Device) {
    event.stopPropagation();
    go("flash", device.id);
  }

  async function toggleScan() {
    breadcrumb(paused ? "resume scan pressed" : "pause scan pressed");
    await call(paused ? "scan.resume" : "scan.pause").catch(() => {});
  }
</script>

{#snippet header()}
  <div class="ph">
    <h2>Boards and devices</h2>
    <span class="chip neu">{count(all.devices.length, "record")}</span>
    <span class="grow"></span>
    {#if problems.length > 0}
      <span class="chip warn" title={problems.map((p) => `${p.link}: ${p.error}`).join("\n")}>
        <span class="dot warn"></span>{count(problems.length, "link")} not answering
      </span>
    {/if}
    {#if paused}
      <span class="chip off"><span class="dot off"></span>Scanning is off</span>
    {:else}
      <span class="chip info"><span class="dot spin"></span>Scanning USB and network</span>
    {/if}
    <button class="btn sm quiet" style="width:58px;justify-content:center" onclick={toggleScan}>
      {paused ? "Resume" : "Pause"}
    </button>
  </div>
{/snippet}

{#if all.devices.length === 0}
  <Page title="Devices" crumb="Atlas /" grid="grid-template-columns:minmax(0,1fr); grid-template-rows:minmax(0,1fr);">
    <section class="panel">
      {@render header()}
      <div class="pb" style="display:flex;min-height:0;padding:0">
        <div class="empty-box">
          <Icon name="plug" sm={false} />
          <div style="font-size:14px;font-weight:600;color:var(--fg)">Nothing found yet</div>
          <div style="max-width:420px">
            {#if paused}
              Scanning is off, so nothing new is found. Press Resume to look on USB and the network again.
            {:else}
              Plug a board in over USB, or join the network it is on. A board in boot ROM shows up here as soon as it
              is plugged in, and Atlas keeps looking while you work.
            {/if}
          </div>
          <div style="display:flex;gap:6px;margin-top:4px">
            <a class="btn pri" href={href("flash")}><Icon name="bolt" />Flash a board</a>
          </div>
        </div>
      </div>
    </section>
  </Page>
{:else}
  <Page title="Devices" crumb="Atlas /" grid="grid-template-rows: minmax(0,1fr) 150px; grid-template-columns: minmax(0,1fr);">
    <section class="panel">
      {@render header()}
      <div class="pb flush tbl" style="padding:0;display:flex;flex-direction:column">
        <div class="tr h" style="{COLUMNS};flex:none">
          <div>Board</div>
          <div>State</div>
          <div>Reached by</div>
          <div>Default</div>
          <div>Seen</div>
        </div>
        <div class="rows">
          {#each groups as group (group.title)}
            <div class="tr g" style={ONE}><div>{group.title}</div></div>
            {#each group.rows as { device, child } (device.id)}
              {@const state = stateOf(device)}
              {@const offline = device.presence === "offline"}
              {@const kind = kindOf(device)}
              {@const reached = reachedBy(device, links, nameById)}
              <div
                class="tr dev link"
                class:sub={child}
                class:dim={offline}
                style={COLUMNS}
                role="link"
                tabindex="0"
                data-device={device.id}
                onclick={() => open(device)}
                onkeydown={(event) => event.key === "Enter" && open(device)}
              >
                <div>
                  {#if child}
                    <span class="ell" style="padding-left:14px"><b>{nameOf(device)}</b> <span class="faint">{kind}</span></span>
                  {:else}
                    <b>{nameOf(device)}</b> <span class="faint">{kind}</span>
                  {/if}
                </div>
                <div>
                  <span class="chip {state.tone}"><span class="dot {state.dot}"></span>{state.word}</span>
                  {#if canFlash(device)}
                    <button class="btn sm pri" onclick={(event) => flash(event, device)}>Flash</button>
                  {/if}
                </div>
                <div>
                  {#if offline}
                    <span class="faint">last {reached.join(" · ")}</span>
                  {:else}
                    {#each reached as word, index (index)}
                      {#if index > 0}{" "}{/if}<span class="chip neu">{word}</span>
                    {/each}
                  {/if}
                </div>
                <div>{device.default_version ?? "·"}</div>
                <div>{offline ? when(device.last_seen_ms, app.now) : "now"}</div>
              </div>
            {/each}
          {/each}
        </div>
      </div>
    </section>
    <section class="panel">
      <div class="ph"><h2>Recent events</h2><span class="grow"></span><span class="kicker">newest first</span></div>
      <div class="pb lines">
        {#each events as line, index (index)}
          <div class="ln">
            <span class="t">{line.time}</span><span class="lv-{line.level}">{line.level}</span><span>{line.text}</span>
          </div>
        {/each}
      </div>
    </section>
  </Page>
{/if}
