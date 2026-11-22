<script lang="ts">
  // The board page (docs/mockups/board.html): readings across the top, slots on the left, and a
  // dock of logs, hardware, files and terminal on the right. Everything on it is what the board
  // reports through Orion; the page reads the topic `board:<id>` while it is shown.
  import Page from "../ui/Page.svelte";
  import Icon from "../ui/Icon.svelte";
  import Spark from "../ui/Spark.svelte";
  import { call } from "../api/client";
  import { app, breadcrumb, inventory, scan, topic } from "../stores/app.svelte";
  import { clock, size } from "../ui/format";
  import { kindOf, nameOf, reachedBy, stateOf } from "../devices/state";
  import type { BoardReading, BoardView, Slot } from "./types";

  let { id }: { id: string } = $props();

  const device = $derived(inventory().devices.find((d) => d.id === id));
  const board = $derived(topic<BoardView>(`board:${id}`));
  const links = $derived(scan()?.links ?? []);
  const online = $derived(device?.presence === "online");
  const known = $derived(device !== undefined);

  // The core reads the board only while a page asks for it: ask now, and keep asking.
  $effect(() => {
    const open = id;
    if (!known) return;
    const ask = () => void call("board.open", { id: open }).catch(() => {});
    ask();
    const timer = setInterval(ask, 4000);
    return () => {
      clearInterval(timer);
      void call("board.close", { id: open }).catch(() => {});
    };
  });

  // ----- top bar ---------------------------------------------------------------------------
  const word = $derived(device ? stateOf(device) : undefined);
  const reach = $derived(
    device
      ? reachedBy(device, links, (other) => {
          const found = inventory().devices.find((d) => d.id === other);
          return found ? nameOf(found) : undefined;
        })
      : [],
  );
  const list = $derived(board?.slots?.state === "ok" ? board.slots.list : undefined);
  const runningSlot = $derived(list?.slots.find((slot) => slot.running));
  // What the board offers to run: the methods its devices describe. `undefined` until it is read.
  const offered = $derived(online && board?.actions ? board.actions : undefined);
  // The top bar has a place for two controls. Each one stays in its place, and is off when the
  // board does not offer what it runs.
  // Reboot is the restart a device of the board serves (Lemnos `system.reboot/1`).
  const reboot = $derived(offered?.find((a) => a.name === "system.reboot.reboot" && a.direct));
  // Locate: no board offers one yet (Lemnos has no contract for it), so there is nothing to run.
  const locate = undefined as { name: string } | undefined;
  /** Why a control in the top bar is off, in words; empty while the board is not read. */
  const missing = $derived.by(() => {
    if (!offered) return online && board?.actions_error ? "Atlas could not read what this board offers." : "";
    const names = [locate ? "" : "locate", reboot ? "" : "reboot"].filter(Boolean);
    return names.length > 0 ? `This board doesn't offer ${names.join(" or ")}.` : "";
  });
  /** What the last call said, shown for a few seconds. A refusal carries the board's own words. */
  const said = $derived(board?.said && app.now - board.said.at_ms < 8000 ? board.said : undefined);

  let busy = $state("");
  async function run(what: string, name: string, args: Record<string, unknown>) {
    if (busy) return;
    busy = what;
    breadcrumb(`${name} on ${id}`);
    try {
      await call(name, { id, ...args });
    } catch {
      // The refusal is on the page already (the topic's `said`), and in the app's log.
    } finally {
      busy = "";
    }
  }

  // ----- readings --------------------------------------------------------------------------
  type Tile = { reading: BoardReading; from: string };
  const tiles = $derived.by((): Tile[] => {
    const out: Tile[] = [];
    for (const dev of board?.hardware ?? []) {
      if (dev.state === "off") continue;
      for (const reading of dev.readings) {
        if (reading.number !== null && reading.unit !== null && out.length < 5) out.push({ reading, from: dev.shown.replace(/^lemnos\./, "") });
      }
    }
    return out;
  });
  const stale = $derived(board?.connection && !board.connection.live ? board.connection : undefined);
  const number = (value: number) => (Math.abs(value) >= 100 ? value.toFixed(0) : Math.abs(value) >= 10 ? value.toFixed(1) : value.toFixed(2));
  const short = (value: number) => String(Number(value.toPrecision(4)));
  function foot(tile: Tile): string {
    const { low, high, warn_high, warn_low } = tile.reading;
    const parts = [tile.from];
    if (warn_high !== null) parts.push(`limit ${short(warn_high)}`);
    else if (warn_low !== null) parts.push(`limit ${short(warn_low)}`);
    else if (low !== null && high !== null) parts.push(`${short(low)} to ${short(high)}`);
    return parts.join(" · ");
  }
  const over = (r: BoardReading) => r.number !== null && ((r.warn_high !== null && r.number > r.warn_high) || (r.warn_low !== null && r.number < r.warn_low));
  const updated = $derived(board ? Math.max(0, Math.round((app.now - board.read_ms) / 1000)) : 0);

  // ----- slots -----------------------------------------------------------------------------
  let removing = $state("");
  const install = $derived(board?.install);
  const installing = $derived(install !== undefined && !install.final);
  $effect(() => {
    // An install that ended well is in the list now: its line has nothing left to say.
    if (install?.stage === "done") void call("board.slots.install.dismiss", { id }).catch(() => {});
  });
  const STAGE: Record<string, string> = {
    preparing: "Preparing",
    waiting: "Waiting for the board",
    sending: "Writing",
    installing: "Installing",
    unknown: "Checking",
    failed: "Failed",
    cancelled: "Cancelled",
    done: "Installed",
  };
  function installLine(): string {
    if (!install) return "";
    if (install.stage === "sending" && install.sent !== null && install.total !== null)
      return `${Math.round(install.percent ?? 0)}% · ${size(install.sent)} of ${size(install.total)} · ordinary update`;
    if (install.percent !== null) return `${Math.round(install.percent)}% · ${install.detail}`;
    return install.detail;
  }
  function slotDetail(slot: Slot): string {
    if (slot.trial && slot.running) return `Not confirmed. The board confirms it, or falls back to ${list?.default || "the default"}.`;
    if (slot.trial) return "Trial boot asked for. The board restarts into it.";
    if (slot.failed) return `Did not boot (${slot.failures} ${slot.failures === 1 ? "failure" : "failures"}).`;
    if (slot.running && slot.default) return `Running now. ${size(slot.size)} · ${slot.signed}.`;
    if (slot.running) return `Running now, not the default. ${size(slot.size)}.`;
    if (slot.default && list?.trial_slot) return "The default. The board falls back to it if the trial does not hold.";
    if (slot.default) return "Default at the next boot.";
    if (slot.pinned) return "Never removed by keep-last-N.";
    if (slot.previous) return `Previous default. Kept by keep-last-${list?.keep ?? ""}.`;
    return `${size(slot.size)} · ${slot.signed}`;
  }
  /** While a trial runs, the default is what the board falls back to. */
  const fallback = (slot: Slot) => slot.default && !slot.running && !slot.trial && Boolean(list?.trial_slot);
  const removable = (slot: Slot) => !slot.running && !slot.default && !slot.trial && !slot.pinned && (list?.slots.length ?? 0) > 1;
  const keeps = $derived([...new Set([1, 2, 3, 4, 5, 6, 7, 8, 9, list?.keep ?? 3])].filter((n) => n > 0).sort((a, b) => a - b));
  type Boot = { time: string; slot: string; words: string };
  const boots = $derived.by((): Boot[] => {
    const out: Boot[] = [];
    if (list?.trial) {
      const t = list.trial;
      if (t.last) out.push({ time: t.last_time, slot: t.slot, words: `trial: ${t.last}${t.reason ? ` (${t.reason})` : ""}` });
      for (const attempt of [...t.attempts].reverse()) out.push({ time: attempt.time, slot: t.slot, words: `trial boot, attempt ${attempt.n}` });
    }
    if (list?.resolved) {
      const r = list.resolved;
      const how = r.outcome === "confirmed" ? "trial held, confirmed" : r.outcome === "fell-back" ? "trial did not hold, fell back" : r.outcome;
      out.push({ time: r.time, slot: r.slot, words: `${how}${r.by ? ` by ${r.by === "auto" ? "the board" : r.by}` : ""}${r.reason ? `: ${r.reason}` : ""}` });
    }
    return out;
  });

  // ----- dock ------------------------------------------------------------------------------
  type Tab = "logs" | "hardware" | "files" | "term";
  let tab = $state<Tab>("logs");
  const TABS: [Tab, string][] = [
    ["logs", "Logs"],
    ["hardware", "Hardware"],
    ["files", "Files"],
    ["term", "Terminal"],
  ];
  const logs = $derived(
    (board?.logs ?? []).map((line) => {
      const level = /^\s*(warn|warning)\b/i.test(line.text) ? "warn" : /^\s*(err|error|fail)/i.test(line.text) ? "err" : "info";
      // The board stamps a line with its own clock; a board whose clock is not set counts from its start.
      const time = line.time_ms > 946_684_800_000 ? clock(line.time_ms) : `+${Math.round(line.time_ms / 1000)} s`;
      // A line that starts with its level has it in the level column, once.
      return { time, level, text: line.text.replace(/^\s*(info|warn|warning|err|error)\s*:\s*/i, "") };
    }),
  );
  const hardware = $derived(board?.hardware ?? []);
  const HW = "grid-template-columns:minmax(0,1.2fr) 56px 96px minmax(0,1.6fr)";
  const BOOTS = "grid-template-columns:96px 52px minmax(0,1fr);min-height:22px;font-size:11px";

  /** Why there is nothing to read, in one line; empty when the board is read. */
  const unread = $derived.by(() => {
    if (!device) return "";
    if (!online) return `${nameOf(device)} is offline. Nothing is read until it answers again.`;
    if (!board) return "";
    if (board.reach === "none") return "This device is not reached through Orion, so there is nothing to read from it.";
    return "";
  });
</script>

{#if !device}
  <Page title="Board" crumb="Devices /" grid="grid-template-columns:minmax(0,1fr); grid-template-rows:minmax(0,1fr);">
    <section class="panel">
      <div class="pb" style="display:flex;min-height:0;padding:0">
        <div class="empty-box"><div>Atlas has no record of this board.</div><a class="btn" href="#/devices">All devices</a></div>
      </div>
    </section>
  </Page>
{:else}
  <Page title={nameOf(device)} crumb="Devices /" grid="grid-template-rows:auto minmax(0,1fr); grid-template-columns:minmax(0,1fr) minmax(0,1fr);">
    {#snippet chips()}
      {#if word}<span class="chip {word.tone}"><span class="dot {word.dot}"></span>{word.word}</span>{/if}
      {#if online && runningSlot}
        <span class="chip {runningSlot.trial ? 'warn' : 'neu'}">{runningSlot.id}{runningSlot.trial ? " · trial" : ""}</span>
      {/if}
      {#if kindOf(device)}<span class="chip neu">{kindOf(device)}</span>{/if}
      {#each reach as word (word)}
        {#if online}<span class="chip neu">{word}</span>{:else}<span class="faint ell">last {word}</span>{/if}
      {/each}
      {#if said}
        <span class="chip {said.ok ? 'ok' : 'err'} ell" style="flex:0 1 auto" title={said.words} data-said={said.ok ? "ok" : "refused"}>
          {said.what}{said.words ? `: ${said.words}` : said.ok ? ": done" : ""}
        </span>
      {/if}
    {/snippet}
    {#snippet actions()}
      {#if missing}<span class="faint" data-not-offered>{missing}</span>{/if}
      <button class="btn" class:dis={!locate} disabled={!locate} onclick={() => locate && run("locate", "board.action", { action: locate.name })}><Icon name="locate" />Locate</button>
      <button class="btn" class:dis={!reboot} disabled={!reboot} onclick={() => reboot && run("reboot", "board.action", { action: reboot.name })}><Icon name="power" />Reboot</button>
    {/snippet}

    <section class="panel" style="grid-column:1/-1" data-panel="readings">
      <div class="ph">
        <h2>Readings</h2>
        {#if online && tiles.length > 0}<span class="kicker">live · last 60 s</span>{/if}
        <span class="grow"></span>
        {#if stale && online}
          <span class="chip warn ell" style="flex:0 1 auto" title={stale.reason}><span class="dot warn"></span>Not answering · shown as last read</span>
        {:else if online && board?.connection?.live}
          <span class="faint mono">updated {updated} s ago</span>
        {/if}
      </div>
      {#if tiles.length > 0 && online}
        <div class="pb flush tiles">
          {#each tiles as tile, i (tile.from + tile.reading.name)}
            <div class="tile" data-reading={tile.reading.name}>
              <div class="lbl">{tile.reading.name.replaceAll("_", " ")}</div>
              <div class="val" style={over(tile.reading) ? "color:var(--err-fg)" : ""}>{number(tile.reading.number ?? 0)} <small>{tile.reading.unit}</small></div>
              <div class="foot">{foot(tile)}</div>
              <Spark values={tile.reading.history} series={stale ? "offline" : i + 1} />
            </div>
          {/each}
          {#each { length: 5 - tiles.length } as _, i (i)}
            <div class="tile empty"><div class="lbl">·</div><div class="val">·</div><div class="foot">not reported</div><Spark values={[]} /></div>
          {/each}
        </div>
      {:else}
        <div class="pb" style="display:flex;min-height:103px;padding:0">
          <div class="empty-box" style="margin:8px 10px;padding:8px">
            <div>{unread || (board ? "This board reports no readings." : "Reading the board…")}</div>
          </div>
        </div>
      {/if}
    </section>

    <section class="panel fill" style="min-height:0" data-panel="slots">
      <div class="ph">
        <h2>Slots</h2>
        {#if list && online}<span class="chip neu">{list.slots.length} kept</span>{/if}
        <span class="grow"></span>
        {#if list && online && !installing}
          <button class="btn sm pri" onclick={() => run("install", "board.slots.install", {})}><Icon name="plus" />Install…</button>
        {/if}
      </div>
      {#if list && online}
        <div class="fill" style="flex:1;min-height:0">
          <div class="rows" style="flex:0 1 auto">
            {#if install && install.stage !== "done"}
              <div class="slot tall {install.final ? '' : 'write'}" data-install={install.stage}>
                <div style="min-width:0">
                  <div class="l1">
                    <b class="ell">{install.file}</b>
                    <span class="chip {install.stage === 'failed' ? 'err' : install.final ? 'neu' : 'acc'}">
                      {#if !install.final}<span class="dot acc"></span>{:else if install.stage === "failed"}<span class="dot err"></span>{/if}{STAGE[install.stage]}
                    </span>
                  </div>
                  {#if install.final}
                    <div class="l2 {install.stage === 'failed' ? 'err' : ''}" title={install.detail}>{install.detail || "Nothing was installed."}</div>
                  {:else}
                    <div class="l2">
                      <div class="bar"><i style="width:{install.percent ?? 0}%"></i></div>
                      <div class="ell" style="margin-top:3px">{installLine()}</div>
                    </div>
                  {/if}
                </div>
                <div class="acts">
                  {#if install.final}
                    <button class="btn sm" onclick={() => run("dismiss", "board.slots.install.dismiss", {})}>Dismiss</button>
                  {:else}
                    <button class="btn sm quiet" onclick={() => run("cancel", "board.slots.install.cancel", {})}>Cancel</button>
                  {/if}
                </div>
              </div>
            {/if}
            {#each list.slots as slot (slot.id)}
              {#if removing === slot.id}
                <div class="slot tall confirm" data-slot={slot.id}>
                  <div style="min-width:0">
                    <div class="l1">
                      <b>Remove {slot.id}?</b>
                      {#if list.keep > 0 && list.slots.length > list.keep}<span class="chip err"><span class="dot err"></span>Over keep-last-{list.keep}</span>{/if}
                    </div>
                    <div class="l2">{slot.version} · this cannot be undone</div>
                  </div>
                  <div class="acts">
                    <button class="btn sm" onclick={() => (removing = "")}>Cancel</button>
                    <button
                      class="btn sm danger"
                      onclick={async () => {
                        await run("remove", "board.slots.remove", { slot: slot.id });
                        removing = "";
                      }}>Remove slot</button
                    >
                  </div>
                </div>
              {:else}
                <div class="slot tall {slot.trial ? 'trial' : slot.default && !fallback(slot) ? 'dflt' : ''}" data-slot={slot.id}>
                  <div style="min-width:0">
                    <div class="l1">
                      <b class="ell">{slot.id} · {slot.version}</b>
                      {#if slot.trial}
                        <span class="chip warn"><span class="dot warn"></span>{slot.running ? "Trial, running" : "Trial"}</span>
                      {:else if slot.running}
                        <span class="chip ok"><span class="dot ok"></span>Running</span>
                      {/if}
                      {#if fallback(slot)}<span class="chip neu">Fallback</span>
                      {:else if slot.default && !slot.trial}<span class="chip neu">Default</span>{/if}
                      {#if slot.previous && !slot.running && !list.trial_slot}<span class="chip neu">Fallback</span>{/if}
                      {#if slot.pinned}<span class="chip neu">Pinned</span>{/if}
                      {#if slot.failed}<span class="chip err"><span class="dot err"></span>Failed</span>{/if}
                    </div>
                    <div class="l2 {slot.failed ? 'err' : ''}">{slotDetail(slot)}</div>
                  </div>
                  <div class="acts">
                    {#if slot.pinned}
                      <button class="btn sm" onclick={() => run("unpin", "board.slots.unpin", { slot: slot.id })}>Unpin</button>
                    {:else}
                      <button class="btn sm" onclick={() => run("pin", "board.slots.pin", { slot: slot.id })}><Icon name="pin" />Pin</button>
                    {/if}
                    {#if !slot.running && !slot.trial && !list.trial_slot}
                      <button class="btn sm" onclick={() => run("trial", "board.slots.trial", { slot: slot.id })}>Trial</button>
                    {/if}
                    {#if !slot.default && !slot.trial}
                      <button class="btn sm" onclick={() => run("default", "board.slots.default", { slot: slot.id })}>Set default</button>
                    {/if}
                    {#if removable(slot)}
                      <button class="btn sm" onclick={() => (removing = slot.id)}>Remove</button>
                    {/if}
                  </div>
                </div>
              {/if}
            {/each}
          </div>
          <div class="keep">
            Keep last
            <select class="sel" aria-label="Keep last" value={list.keep} onchange={(event) => run("keep", "board.slots.keep", { keep: Number(event.currentTarget.value) })}>
              {#each keeps as n (n)}<option value={n}>{n}</option>{/each}
            </select>
            · pinned and default slots stay
          </div>
          <div class="subhead"><h2 class="up" style="margin:0;color:var(--fg-muted)">Last boots</h2></div>
          <div style="padding:2px 10px 4px" data-boots>
            {#each boots as boot, i (i)}
              <div class="tr dim {i > 1 ? 'more-s' : ''}" style={BOOTS}>
                <div class="faint">{boot.time || "·"}</div>
                <div>{boot.slot}</div>
                <div title={boot.words}>{boot.words}</div>
              </div>
            {:else}
              <div class="tr dim" style="grid-template-columns:minmax(0,1fr);min-height:22px;font-size:11px">
                <div class="faint">The board reports no trial boot yet.</div>
              </div>
            {/each}
          </div>
        </div>
      {:else}
        <div class="pb" style="display:flex;min-height:0;padding:0">
          <div class="empty-box" data-slots-state={board?.slots?.state ?? "unread"}>
            <div>
              {#if unread}{unread}
              {:else if board?.slots?.state === "none"}This board has no slots: it does not offer them. An Orion node without Lemnos is normal.
              {:else if board?.slots?.state === "not-reported"}This board did not report what it offers, so its slots are not known.
              {:else if board?.slots?.state === "error"}The board's slots could not be read: {board.slots.error}
              {:else}Reading the board…{/if}
            </div>
          </div>
        </div>
      {/if}
    </section>

    <section class="panel dock" data-panel="dock">
      <div class="tabbar" role="tablist">
        {#each TABS as [key, label] (key)}
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_no_noninteractive_element_to_interactive_role, a11y_label_has_associated_control -->
          <label
            class:on={tab === key}
            role="tab"
            tabindex="0"
            aria-selected={tab === key}
            onclick={() => (tab = key)}
            onkeydown={(event) => {
              if (event.key === "Enter" || event.key === " ") tab = key;
            }}>{label}</label
          >
        {/each}
        <span class="grow" style="flex:1"></span>
      </div>
      <div class="panes">
        {#if tab === "logs"}
          <div class="pane pane-logs" style="display:flex">
            {#if logs.length > 0 && online}
              <div class="pb lines" style="flex:1">
                {#each logs as line, i (i)}
                  <div class="ln {i >= 10 ? 'more-s' : ''}"><span class="t">{line.time}</span><span class="lv-{line.level}">{line.level}</span><span>{line.text}</span></div>
                {/each}
              </div>
            {:else}
              <div class="empty-box"><div>{unread || (board ? "No log lines since this page opened." : "Reading the board…")}</div></div>
            {/if}
          </div>
        {:else if tab === "hardware"}
          <div class="pane" style="display:flex">
            {#if hardware.length > 0 && online}
              <div class="pb flush tbl" style="padding:0;display:flex;flex-direction:column">
                <div class="tr h" style={HW}><div>Device</div><div>Bus</div><div>State</div><div>Reads</div></div>
                <div class="rows">
                  {#each hardware as dev (dev.name)}
                    <div class="tr" style="{HW};min-height:24px" data-device={dev.name}>
                      <div><b>{dev.shown.replace(/^lemnos\./, "")}</b> <span class="faint">{dev.kind ?? ""}</span></div>
                      <div class="muted">{dev.bus ?? "·"}</div>
                      <div>
                        {#if dev.state === "running"}<span class="chip ok"><span class="dot ok"></span>running</span>
                        {:else if dev.state === "retrying"}<span class="chip warn" title={dev.reason ?? ""}><span class="dot warn"></span>retrying</span>
                        {:else if dev.state === "off"}<span class="chip off" title={dev.reason ?? ""}><span class="dot off"></span>off</span>
                        {:else}<span class="faint">not reported</span>{/if}
                      </div>
                      <div class="faint" title={dev.reason ?? ""}>
                        {dev.state === "running" || dev.state === null
                          ? dev.readings.map((r) => `${r.name.replaceAll("_", " ")} ${r.number !== null ? number(r.number) : (r.text ?? "·")}${r.unit ? ` ${r.unit}` : ""}`).join(" · ")
                          : (dev.reason ?? "")}
                      </div>
                    </div>
                  {/each}
                </div>
              </div>
            {:else}
              <div class="empty-box"><div>{unread || (board ? "This board describes no hardware." : "Reading the board…")}</div></div>
            {/if}
          </div>
        {:else if tab === "files"}
          <div class="pane pane-files" style="display:flex">
            <div class="empty-box" data-not-offered="files">
              <div>This board doesn't offer files yet.</div>
            </div>
          </div>
        {:else}
          <div class="pane pane-term" style="display:flex">
            <div class="empty-box" data-not-built="terminal">
              <div>The terminal is not in this build.</div>
              <div class="faint" style="max-width:420px;font-size:11px">A terminal is served by Orion on the board. Atlas has no way to carry one to this window yet.</div>
            </div>
          </div>
        {/if}
      </div>
    </section>
  </Page>
{/if}
