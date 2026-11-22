<script lang="ts">
  // Settings (docs/mockups/settings.html): the trusted keys and who owns them, the one security
  // switch, slots and updates, the app's log and the support bundle, and the robots Atlas looks
  // for. Every change is saved at once; nothing here has a Save button.
  import Page from "../ui/Page.svelte";
  import Icon from "../ui/Icon.svelte";
  import Skel from "../ui/Skel.svelte";
  import { call } from "../api/client";
  import type { Settings } from "../api/types";
  import { app, breadcrumb, inventory, logUiError, settings, topic } from "../stores/app.svelte";
  import { eventTime, size, when } from "../ui/format";
  import { purposeWord, shortFingerprint, type Bundle, type SupportInfo, type Trust, type TrustedKey, type Update } from "./types";

  const KEY_COLUMNS = "grid-template-columns:minmax(0,1fr) 150px 112px";
  const CHECK_COLUMNS = "grid-template-columns:70px 96px minmax(0,1fr) minmax(0,1fr)";
  const KEEP = [1, 2, 3, 4, 5, 6, 7, 8, 9];
  const CHANNELS = ["dev", "beta", "stable"];
  const THEMES = ["system", "light", "dark"];

  const current = $derived(settings());
  const trust = $derived(topic<Trust>("trust") ?? { keys: [], problems: [], checks: [] });
  /** The app's own update. */
  const update = $derived(topic<Update>("update"));
  /** "checked just now", as the mockup says it, then the time. */
  const checked = $derived(
    !update?.source
      ? ""
      : update.checked_ms === null
        ? "not checked yet"
        : app.now - update.checked_ms < 60_000
          ? "checked just now"
          : `checked ${when(update.checked_ms, app.now)}`,
  );
  const TONES = { faint: "var(--fg-faint)", ok: "var(--ok-fg)", acc: "var(--accent-text)", err: "var(--err-fg)" };
  const part = $derived(update?.size ? Math.min(100, Math.round((update.downloaded / update.size) * 100)) : 0);
  let updateSaid = $state("");
  async function ask(name: string) {
    updateSaid = "";
    breadcrumb(name);
    try {
      await call(name);
    } catch (error) {
      updateSaid = words(error);
    }
  }
  const robots = $derived(inventory().robots);

  /** What the last thing asked for answered, by the place it is shown. */
  let said = $state<{ where: "keys" | "robots" | "support"; words: string; ok?: boolean } | null>(null);
  const words = (error: unknown) => (error instanceof Error ? error.message : String(error));

  // Changes are sent one after another, each on top of what the core answered to the one before:
  // two controls changed quickly must not send the second on top of a value that is already old.
  let answered: Settings | null = null;
  let sending: Promise<void> = Promise.resolve();
  function set(change: Partial<Settings>) {
    breadcrumb(`setting changed: ${Object.keys(change).join(", ")}`);
    sending = sending.then(async () => {
      const base = answered ?? current;
      if (!base) return;
      try {
        answered = await call<Settings>("settings.set", { ...base, ...change });
      } catch (error) {
        logUiError("settings.set", error);
      }
    });
    return sending;
  }

  // ----- keys --------------------------------------------------------------------------------
  let openKey = $state<string | null>(null);
  let removing = $state(false);
  const shown = $derived(trust.keys.find((key) => key.key_id === openKey) ?? null);

  function details(key: TrustedKey) {
    removing = false;
    openKey = openKey === key.key_id ? null : key.key_id;
  }

  async function addKey() {
    said = null;
    try {
      const added = await call<string[] | false>("trust.add");
      if (added) said = { where: "keys", words: `Added ${added.join(", ")}.`, ok: true };
    } catch (error) {
      said = { where: "keys", words: words(error) };
    }
  }

  async function removeKey(key: TrustedKey) {
    try {
      await call("trust.remove", { key_id: key.key_id });
      openKey = null;
      said = null;
    } catch (error) {
      said = { where: "keys", words: words(error) };
    }
    removing = false;
  }

  // ----- log and support ---------------------------------------------------------------------
  let info = $state<SupportInfo | null>(null);
  let bundle = $state<Bundle | null>(null);
  const readInfo = () =>
    call<SupportInfo>("support.info")
      .then((answer) => (info = answer))
      .catch((error) => logUiError("support.info", error));
  $effect(() => {
    void readInfo();
  });

  async function makeBundle() {
    said = null;
    try {
      bundle = await call<Bundle>("support.make_bundle");
      void readInfo();
    } catch (error) {
      said = { where: "support", words: words(error) };
    }
  }

  async function open(name: string) {
    said = null;
    try {
      await call(name);
    } catch (error) {
      said = { where: "support", words: words(error) };
    }
  }

  // ----- robots ------------------------------------------------------------------------------
  let robotName = $state("");
  let robotTeam = $state("");

  async function addRobot(event: SubmitEvent) {
    event.preventDefault();
    said = null;
    try {
      await call("robot.add", { name: robotName, team: robotTeam });
      robotName = "";
      robotTeam = "";
    } catch (error) {
      said = { where: "robots", words: words(error) };
    }
  }

  async function removeRobot(name: string) {
    said = null;
    try {
      await call("robot.remove", { name });
    } catch (error) {
      said = { where: "robots", words: words(error) };
    }
  }
</script>

<Page
  title="Settings"
  crumb="Atlas /"
  grid="grid-template-columns:minmax(0,1.25fr) minmax(0,1fr); grid-template-rows:minmax(0,1.1fr) minmax(0,1fr) auto;"
>
  <section class="panel" style="grid-row:1/3" data-panel="keys">
    <div class="ph">
      <h2>Trusted keys</h2>
      <span class="grow"></span>
      <button class="btn sm" onclick={addKey}><Icon name="plus" />Add key…</button>
    </div>
    <div class="pb flush tbl" style="padding:0;flex:none">
      <div class="tr h" style={KEY_COLUMNS}>
        <div>Key and owner</div>
        <div>Fingerprint</div>
        <div></div>
      </div>
      {#if !app.ready}<Skel rows={2} columns={KEY_COLUMNS} cells={2} style="min-height:50px" />{/if}
      {#each trust.keys as key (key.key_id)}
        <!-- The selected look, written out: the stylesheet's `.sel` is also its select box, whose border and padding would move the row. -->
        <div
          class="tr"
          style="{KEY_COLUMNS};min-height:50px;{openKey === key.key_id ? 'background:var(--accent-tint);box-shadow:inset 2px 0 0 var(--accent)' : ''}"
          data-key={key.key_id}
          data-open={openKey === key.key_id}
        >
          <div style="white-space:normal">
            <b>{key.name}</b>
            <div class="faint ell" style="font-size:10.5px">{key.owner} · {key.contact}</div>
            <div class="faint ell" style="font-size:10.5px">signs: {purposeWord(key)}{key.yours ? " · added by you" : " · built into Atlas"}</div>
          </div>
          <div class="mono faint">{shortFingerprint(key)}</div>
          <div class="acts"><button class="btn sm quiet" onclick={() => details(key)}>{openKey === key.key_id ? "Close" : "Details"}</button></div>
        </div>
      {/each}
    </div>
    {#if said?.where === "keys"}
      <!-- What adding or removing a key answered, in full, where the note about the keys is. -->
      <div class="note {said.ok ? 'ok' : 'err'}" style="margin:8px 10px" data-said>{said.words}</div>
    {:else}
      <div class="note" style="margin:8px 10px">
        Atlas checks an image you choose against these keys. Owner details come from the key file. A board checks an image with its own keys when its security is on.
      </div>
    {/if}
    {#if shown}
      <div class="ph" style="border-top:1px solid var(--hairline)">
        <h2>Key details</h2><span class="grow"></span><span class="kicker">{shown.yours ? "added by you" : "built into Atlas"}</span>
      </div>
      <div class="pb" style="display:flex;flex-direction:column;gap:8px" data-panel="key-details">
        <dl class="kv">
          <dt>Name</dt><dd>{shown.name}</dd>
          <dt>Owner</dt><dd>{shown.owner}</dd>
          <dt>Contact</dt><dd>{shown.contact}</dd>
          <dt>Signs</dt><dd>{purposeWord(shown)}</dd>
          <dt>Fingerprint</dt><dd class="mono">{shown.fingerprint}</dd>
          <dt>Created</dt><dd>{shown.created}</dd>
          <dt>Expires</dt><dd>{shown.expires ?? "never"}</dd>
          <dt>From</dt><dd style="white-space:normal;overflow-wrap:anywhere">{shown.source}</dd>
          <dt class="more-s">Note</dt><dd class="more-s" style="white-space:normal;overflow-wrap:anywhere">{shown.note || "·"}</dd>
        </dl>
        {#if shown.yours}
          {#if removing}
            <div class="confirm-line" style="padding:0 8px;background:var(--err-bg);border-left:2px solid var(--err);border-radius:2px">
              <span class="q">Remove {shown.name} from the trusted keys? Files it signed stop being trusted.</span>
              <button class="btn sm" onclick={() => (removing = false)}>Cancel</button>
              <button class="btn sm danger" onclick={() => removeKey(shown)}>Remove key</button>
            </div>
          {:else}
            <div class="confirm-line"><span class="sp"></span><button class="btn sm" onclick={() => (removing = true)}>Remove…</button></div>
          {/if}
        {/if}
      </div>
    {:else}
      <div class="ph" style="border-top:1px solid var(--hairline)">
        <h2>Recent signature checks</h2><span class="grow"></span><span class="kicker">on this computer · newest first</span>
      </div>
      <div class="pb flush" style="padding:0 10px" data-panel="checks">
        {#each trust.checks.slice(0, 12) as check, i (i)}
          <div class="tr" class:more-s={i >= 4} style="{CHECK_COLUMNS};min-height:24px;font-size:11px;padding:0">
            <div class="faint">{eventTime(check.at_ms, app.now)}</div>
            <div>{check.what}</div>
            <div style="white-space:normal;overflow-wrap:anywhere">{check.subject}</div>
            <div class="muted" style="white-space:normal;overflow-wrap:anywhere;{check.refused ? 'color:var(--err-fg)' : ''}">
              {check.result}{check.refused ? " · refused" : current?.security ? "" : " · used: security is off"}
            </div>
          </div>
        {:else}
          {#if app.ready}
            <div class="faint" style="padding:8px 0;font-size:11px">
              Nothing has been checked yet. Atlas checks an image's signature when you choose it to flash or to write to a card.
            </div>
          {:else}
            <Skel rows={2} row="" style="height:24px;display:flex;align-items:center" />
          {/if}
        {/each}
        {#each trust.problems as problem}
          <div class="note warn" style="margin-top:6px">{problem}</div>
        {/each}
      </div>
    {/if}
  </section>

  <section class="panel" data-panel="security">
    <div class="ph"><h2>Security</h2></div>
    <div class="pb" style="display:flex;flex-direction:column;gap:8px">
      <div class="swrap" style="justify-content:space-between">
        <span><b>Check signatures</b></span>
        <div style="display:flex;align-items:center;gap:8px">
          <span class="faint">{!current ? "·" : current.security ? "on" : "off"}</span>
          <!-- Until the settings are read the switch is the disabled one: it has no value to show or change. -->
          <button
            class="sw"
            class:on={current?.security}
            class:dis={!current}
            disabled={!current}
            role="switch"
            aria-checked={current?.security ?? false}
            aria-label="Check signatures"
            onclick={() => set({ security: !current?.security })}
          ></button>
        </div>
      </div>
      <div class="muted" style="font-size:11px">
        Off by default. Off: an unsigned image is used, as FRC setups expect, and what its signature says is recorded. On: Atlas refuses an image that is not signed by a trusted key.
      </div>
      <div class="more-s">
        <div class="subhead" style="margin:0"><h2 class="up" style="margin:0;color:var(--fg-muted)">What is checked when on</h2></div>
        <dl class="kv" style="grid-template-columns:130px minmax(0,1fr);margin-top:8px">
          <dt>An image you choose</dt><dd>its .minisig, against the trusted keys</dd>
          <dt>Failed check</dt><dd>the file is refused before anything is sent</dd>
          <dt>Expired key</dt><dd>refused, like an unknown key</dd>
          <dt>On a board</dt><dd>the board's own switch decides; Atlas does not set it</dd>
        </dl>
      </div>
    </div>
  </section>

  <section class="panel" data-panel="updates">
    <div class="ph"><h2>Slots and updates</h2></div>
    <div class="pb" style="display:flex;flex-direction:column;gap:8px">
      <div class="swrap" style="justify-content:space-between">
        <span><b>Keep last N slots</b> by default</span>
        <select class="sel" aria-label="Keep last N slots" disabled={!current} value={current?.keep_last ?? 3} onchange={(e) => set({ keep_last: Number(e.currentTarget.value) })}>
          {#each KEEP as n}<option value={n}>{n}</option>{/each}
        </select>
      </div>
      <div class="muted" style="font-size:11px">Each board can set its own number. Pinned, running and default slots are never removed.</div>
      <div class="swrap" style="justify-content:space-between;border-top:1px solid var(--hairline);padding-top:8px">
        <span class="ell"><b>Atlas</b> <span class="faint mono">{app.version}</span></span>
        <span class="acts" data-update-controls>
          {#if !app.ready}
            <button class="btn sm" disabled><Icon name="retry" />Check for updates</button>
          {:else if !update?.source}
            <span class="faint" style="font-size:11px;white-space:nowrap">no update source configured</span>
          {:else if update.phase === "checking"}
            <span class="chip info"><span class="dot spin"></span>Checking</span>
          {:else if update.phase === "retrying"}
            <span class="chip err"><span class="dot err"></span>Retrying</span>
            <button class="btn sm" onclick={() => ask("update.stop")}><Icon name="stop" />Stop</button>
          {:else if update.phase === "available"}
            <button class="btn sm" onclick={() => ask("update.check")}><Icon name="retry" />Check for updates</button>
            <button class="btn sm pri" onclick={() => ask("update.download")}><Icon name="download" />Download</button>
          {:else if update.phase === "downloading"}
            <button class="btn sm" onclick={() => ask("update.cancel")}>Cancel</button>
          {:else if update.phase === "ready" && update.apply}
            <button class="btn sm pri" onclick={() => ask("update.apply")}>{update.apply}</button>
          {:else}
            {#if update.phase === "failed" && update.version}
              <button class="btn sm" onclick={() => ask("update.download")}><Icon name="download" />Download again</button>
            {/if}
            <button class="btn sm" onclick={() => ask("update.check")}><Icon name="retry" />Check for updates</button>
          {/if}
        </span>
      </div>
      <div class="swrap">
        <span class="faint">Channel</span>
        <select class="sel" aria-label="Channel" disabled={!current} value={current?.channel ?? "dev"} onchange={(e) => set({ channel: e.currentTarget.value })}>
          {#each CHANNELS as channel}<option>{channel}</option>{/each}
        </select>
        <span class="faint">Theme</span>
        <select class="sel" aria-label="Theme" disabled={!current} value={current?.theme ?? "system"} onchange={(e) => set({ theme: e.currentTarget.value as Settings["theme"] })}>
          {#each THEMES as theme}<option>{theme}</option>{/each}
        </select>
        <span class="faint ell" style="font-size:11px" data-update-checked>{checked}</span>
      </div>
      <!-- One line, always there, so nothing below it moves when an update comes and goes. -->
      <div class="swrap" style="min-height:17px;font-size:11px" data-update>
        {#if updateSaid}
          <span style="color:var(--err-fg)">{updateSaid}</span>
        {:else if update?.source && update.phase === "downloading"}
          <span style="white-space:nowrap;color:var(--accent-text)">{update.words}</span>
          <div class="bar" style="flex:1"><i style="width:{part}%"></i></div>
          <span class="faint num" style="white-space:nowrap">{part}% · {size(update.downloaded)} of {size(update.size ?? 0)}</span>
        {:else if update?.source && update.words}
          <!-- In full: a reason for a failed check wraps instead of being cut. -->
          <span style="color:{TONES[update.tone]}">{update.words}{update.phase === "available" && update.size ? ` · ${size(update.size)}` : ""}</span>
        {/if}
      </div>
      <div class="more-s">
        <div class="subhead" style="margin:0"><h2 class="up" style="margin:0;color:var(--fg-muted)">What lands on boards</h2></div>
        <dl class="kv" style="grid-template-columns:150px minmax(0,1fr);margin-top:8px">
          <dt>Flasher image</dt><dd class="faint">none in this build</dd>
          <dt>Orion and Lemnos</dt><dd class="faint">no controller package in this build</dd>
          <dt>Atlas app</dt><dd>{info?.app_bytes ? `${size(info.app_bytes)}, this program's own file` : "not known"}</dd>
        </dl>
      </div>
    </div>
  </section>

  <section class="panel" data-panel="support">
    <div class="ph"><h2>Logs and support</h2></div>
    <div class="pb" style="display:flex;flex-direction:column;gap:8px">
      <dl class="kv">
        <dt>App log</dt><dd class="mono" style="white-space:normal;overflow-wrap:anywhere">{#if info}{info.log_path ?? "this run keeps no log"}{:else}<div class="ph-skel" style="width:60%;margin-top:3px"></div>{/if}</dd>
        <dt>Size</dt><dd>{info ? size(info.log_bytes) : "·"} · UI errors, failed calls, breadcrumbs</dd>
      </dl>
      <div class="swrap">
        <button class="btn sm" onclick={() => open("support.open_log_folder")}><Icon name="folder" />Open log folder</button>
        <button class="btn sm" onclick={makeBundle}><Icon name="bundle" />Make support bundle</button>
        {#if bundle}<button class="btn sm quiet" onclick={() => open("support.open_bundle_folder")}>Show the bundle</button>{/if}
      </div>
      {#if said?.where === "support"}
        <div style="font-size:11px;color:var(--err-fg)">{said.words}</div>
      {:else if bundle}
        <!-- Where the bundle is, in full, and what is in it. -->
        <div class="muted" style="font-size:11px;white-space:normal;overflow-wrap:anywhere" data-bundle>
          {bundle.path} · {size(bundle.bytes)} · holds: {bundle.parts.map((part) => part.name).join(", ")}
        </div>
      {:else}
        <div class="muted" style="font-size:11px">The bundle is a file on this computer. You see what is in it before you send it anywhere.</div>
      {/if}
    </div>
  </section>

  <section class="panel" data-panel="robots">
    <div class="ph">
      <h2>Robots</h2><span class="grow"></span><span class="kicker">Atlas looks on each team's robot network</span>
    </div>
    <div class="pb flush tbl" style="padding:0;display:flex;flex-direction:column">
      <div class="rows" style="flex:none;max-height:52px">
        {#if !app.ready}<Skel rows={1} />{/if}
        {#each robots as robot (robot.name)}
          <div class="tr" style="grid-template-columns:minmax(0,1fr) 90px auto" data-robot={robot.name}>
            <div><b>{robot.name}</b></div>
            <div class="muted">team {robot.team ?? "·"}</div>
            <div class="acts"><button class="btn sm quiet" onclick={() => removeRobot(robot.name)}>Remove</button></div>
          </div>
        {/each}
      </div>
      <form class="swrap" style="padding:8px 10px" onsubmit={addRobot}>
        <input class="inp" style="flex:1" placeholder="Robot name" aria-label="Robot name" bind:value={robotName} />
        <input class="inp" style="width:90px;flex:none" placeholder="Team" aria-label="Team number" inputmode="numeric" bind:value={robotTeam} />
        <button class="btn sm" type="submit"><Icon name="plus" />Add robot</button>
      </form>
      <div style="padding:0 10px 8px;font-size:11px;{said?.where === 'robots' ? 'color:var(--err-fg)' : 'color:var(--fg-muted)'}" data-robot-said>
        {said?.where === "robots" ? said.words : "Removing a robot forgets its name and team, not its devices."}
      </div>
    </div>
  </section>
</Page>
