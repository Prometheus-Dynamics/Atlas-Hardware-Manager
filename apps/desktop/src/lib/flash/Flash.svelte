<script lang="ts">
  // Flash a board (docs/mockups/flash.html, flash-failed.html): the steps on the left, and on the
  // right the image, the board in its boot ROM and what was sent, or the failure with its manual
  // off. The page shows what the libraries do today: the image is read and the loader is sent;
  // the flasher image and the write are the flash flow library's (D6), which this build does not
  // have, and the page says so instead of drawing progress that did not happen.
  import Page from "../ui/Page.svelte";
  import Icon from "../ui/Icon.svelte";
  import Skel from "../ui/Skel.svelte";
  import { call } from "../api/client";
  import { app, breadcrumb, inventory, settings, topic } from "../stores/app.svelte";
  import { go } from "../stores/route.svelte";
  import { clock, dayTime, size } from "../ui/format";
  import { nameOf } from "../devices/state";
  import { recentWord, signatureWords, type FlashView } from "./types";

  let { id }: { id: string } = $props();

  const flash = $derived(topic<FlashView>("flash"));
  const image = $derived(flash?.image ?? null);
  const run = $derived(flash?.run ?? null);
  const roms = $derived(inventory().devices.filter((d) => d.presence === "online" && d.identity.state === "boot-rom"));
  /** The board this page is about: the one it was opened for, the one being flashed, or the only
   *  one there is. With several and none named, the person chooses. */
  const board = $derived(roms.find((d) => d.id === id) ?? roms.find((d) => d.id === run?.device) ?? (id === "" && roms.length === 1 ? roms[0] : undefined));
  const rom = $derived(board ? flash?.roms.find((r) => board.id.startsWith(`${r.family}:`)) : undefined);
  // The scan asks a steady device who it is again only once a minute, so right after a loader
  // run the record still says maskrom; the run itself knows better.
  const inLoaderMode = $derived(board?.identity.attributes["rom-mode"] === "loader" || (run?.status === "loaded" && run.device === board?.id));
  const port = $derived(board?.identity.address ?? run?.port ?? "");

  const active = $derived(run !== null && ["queued", "running", "retrying", "stopped"].includes(run.status));
  const failing = $derived(run !== null && ["retrying", "stopped", "failed", "unknown", "interrupted"].includes(run.status));
  const loaded = $derived(run?.status === "loaded");
  const ready = $derived(image?.state === "ready" && board !== undefined && rom !== undefined && !active);

  let busy = $state(false);
  /** The support bundle's file, once one was made from here. */
  let bundled = $state("");
  async function bundle() {
    try {
      const made = await call<{ path: string }>("support.make_bundle");
      bundled = `Support bundle written to ${made.path}`;
    } catch (error) {
      bundled = error instanceof Error ? error.message : String(error);
    }
  }

  async function ask(name: string, args?: unknown) {
    if (busy) return;
    busy = true;
    breadcrumb(name);
    try {
      await call(name, args);
    } catch {
      // The core has logged it; the page shows the state the core is in.
    } finally {
      busy = false;
    }
  }
  const start = async () => {
    if (!board) return;
    if (run) await call("flash.dismiss").catch(() => {});
    await ask("flash.start", { id: board.id });
  };

  // ----- the six steps ----------------------------------------------------------------------
  type Step = { key: string; state: "done" | "cur" | "pend" | "fail"; title: string; detail: string };
  const tries = (n: number) => `${n} ${n === 1 ? "try" : "tries"}`;
  const steps = $derived.by((): Step[] => {
    const out: Step[] = [];
    if (image?.state === "ready") out.push({ key: "image", state: "done", title: "Pick image", detail: `${image.file} · ${size(image.bytes)}` });
    else if (image?.state === "refused") out.push({ key: "image", state: "fail", title: "Pick image", detail: "not a first-flash image" });
    else out.push({ key: "image", state: "cur", title: "Pick image", detail: image ? "reading the slot out of the image" : "no image chosen yet" });
    const imageDone = image?.state === "ready";

    if (board) out.push({ key: "board", state: "done", title: "Board in boot ROM", detail: `${rom?.label ?? "Boot ROM"} · USB ${port}` });
    else if (run && !active) out.push({ key: "board", state: "done", title: "Board in boot ROM", detail: `${run.family} · USB ${run.port}` });
    else
      out.push({
        key: "board",
        state: imageDone ? "cur" : "pend",
        title: "Board in boot ROM",
        detail: roms.length > 1 ? `${roms.length} boards in boot ROM, choose one` : "waiting for a board in its boot ROM",
      });

    const loader = (state: Step["state"], detail: string): Step => ({ key: "loader", state, title: "Loader sent", detail });
    if (!run || run.status === "cancelled") out.push(loader(imageDone && board ? "cur" : "pend", run ? "cancelled, nothing was written" : imageDone && board ? "ready to send" : "not started"));
    else if (run.status === "loaded") out.push(loader("done", run.loaded?.words ?? "the board answers in loader mode"));
    else if (run.status === "retrying") out.push(loader("fail", `no answer from the board after ${tries(run.attempt)}`));
    else if (run.status === "stopped") out.push(loader("fail", `stopped by hand after ${tries(run.attempt)}`));
    else if (failing) out.push(loader("fail", run.error));
    else out.push(loader("cur", `sending to USB ${run.port}`));

    const later = loaded ? "not in this build" : "not started";
    out.push({ key: "flasher", state: "pend", title: "Flasher image running", detail: later });
    out.push({ key: "write", state: "pend", title: "Writing as an ordinary update", detail: later });
    out.push({ key: "boot", state: "pend", title: "First boot", detail: later });
    return out;
  });

  // ----- the failure panel ------------------------------------------------------------------
  const waitLeft = $derived(run?.status === "retrying" ? Math.max(0, run.retry_at_ms - app.now) : 0);
  const waited = $derived(run?.status === "retrying" && run.retry_in_ms > 0 ? Math.min(100, Math.round(100 * (1 - waitLeft / run.retry_in_ms))) : 0);
  const secs = (ms: number) => {
    const s = ms / 1000;
    return `${s >= 10 || Number.isInteger(s) ? Math.round(s) : s.toFixed(1)} s`;
  };
  const lines = $derived(run ? [...run.log].reverse() : []);
  /** The three newest flashes; the block keeps three rows' height however many there are. */
  const recent = $derived((flash?.recent ?? []).slice(0, 3));
  const stopped = $derived(run?.status === "stopped");
  /** An image packed with xz (the one OS image, `.flash`, or an `.img.xz`): its size is the packed one. */
  const compressed = (file: string) => /\.(flash|xz)$/i.test(file);
  /** The folder a chosen file is in, said beside its name. */
  const folder = (path: string) => path.replace(/[\\/][^\\/]*$/, "") || "/";
</script>

<Page title="Flash a board" crumb="Atlas /" grid="grid-template-columns:260px minmax(0,1fr); grid-template-rows:minmax(0,1fr);">
  {#snippet chips()}
    {#if board || run}
      <span class="chip acc"><span class="dot acc"></span>{rom?.label ?? run?.family ?? "Boot ROM"}</span>
      <span class="chip neu">{board ? nameOf(board) : run?.name}</span>
    {/if}
    {#if run && failing}
      <span class="chip {stopped ? 'off' : 'err'}" data-flash-chip="failed">{stopped ? "stopped at loader" : "failed at loader"}</span>
    {:else if loaded}
      <span class="chip warn" data-flash-chip="loaded">stops after the loader</span>
    {/if}
  {/snippet}
  {#snippet actions()}
    {#if active}
      <button class="btn quiet" onclick={() => ask("flash.cancel")}>Cancel</button>
    {:else if ready}
      {#if run}<button class="btn quiet" onclick={() => ask("flash.dismiss")}>Clear</button>{/if}
      <button class="btn pri" onclick={start}><Icon name="bolt" />{run ? "Send loader again" : "Send loader"}</button>
    {:else if run}
      <button class="btn quiet" onclick={() => ask("flash.dismiss")}>Clear</button>
    {/if}
  {/snippet}

  <section class="panel" data-panel="steps">
    <div class="ph"><h2>Steps</h2></div>
    <div class="pb" style="padding:6px 12px">
      {#if !app.ready}<Skel rows={6} row="" style="height:42px;display:flex;align-items:center" />{/if}
      {#each app.ready ? steps : [] as step, index (step.key)}
        <div class="step {step.state}" data-step={step.key} data-state={step.state}>
          <div class="mark">{#if step.state === "done"}&#10003;{:else if step.state === "fail"}!{:else}{index + 1}{/if}</div>
          <div style="min-width:0">
            <div class="st">{step.title}</div>
            <!-- In full: a long detail (a refusal, a file name) wraps instead of being cut. -->
            <div class="sd" style="white-space:normal;overflow-wrap:anywhere">{step.detail}</div>
          </div>
        </div>
      {/each}
    </div>
    <div class="subhead"><h2 class="up" style="margin:0;color:var(--fg-muted)">Recent flashes here</h2></div>
    <div style="padding:2px 10px;flex:none" data-panel="recent">
      {#each recent as done (done.at_ms + done.name)}
        {@const word = recentWord(done)}
        <div class="tr" style="grid-template-columns:minmax(0,1fr) 88px;min-height:24px;font-size:11px">
          <div><b>{done.name}</b> <span class="faint">{dayTime(done.at_ms, app.now)}</span></div>
          <div><span class="chip {word.tone}">{#if word.tone !== "off"}<span class="dot {word.tone}"></span>{/if}{word.word}</span></div>
        </div>
      {:else}
        {#if app.ready}
          <div class="tr faint" style="grid-template-columns:minmax(0,1fr);min-height:24px;font-size:11px;border-bottom:0"><div>None yet on this computer.</div></div>
        {:else}
          <Skel rows={1} style="min-height:24px;border-bottom:0" />
        {/if}
      {/each}
      <!-- Rows kept for flashes not made yet, so the block is as tall with one as with three. -->
      {#each { length: 3 - Math.max(1, recent.length) } as _, i (i)}
        <div class="tr" style="grid-template-columns:minmax(0,1fr) 88px;min-height:24px;font-size:11px;visibility:hidden;border-bottom:0" aria-hidden="true"><div>·</div></div>
      {/each}
    </div>
  </section>

  {#if run && failing}
    <div style="display:grid;grid-template-rows:auto minmax(0,1fr);gap:8px;min-height:0;min-width:0">
      <section class="panel" data-panel="failure" data-status={run.status}>
        <div class="ph">
          <h2>{stopped ? "Loader stopped" : "Loader failed"}</h2><span class="grow"></span>
          {#if run.status === "retrying"}
            <span class="chip err"><span class="dot err"></span>Retrying</span>
          {:else if run.status === "stopped"}
            <span class="chip off"><span class="dot off"></span>Stopped</span>
          {:else}
            <span class="chip err"><span class="dot err"></span>Failed</span>
          {/if}
        </div>
        <div class="pb" style="display:flex;flex-direction:column;gap:8px">
          <div class="note ell" class:err={!stopped}>
            {#if stopped}
              Stopped by hand. Nothing was written. The board is still safe to flash.
            {:else if run.status === "retrying"}
              The board in boot ROM did not answer the loader. Nothing was written. The board is still safe to flash.
            {:else}
              The loader was refused and will not be tried again on its own. Nothing was written.
            {/if}
          </div>
          <dl class="kv">
            {#if run.status === "retrying" || run.status === "stopped"}
              <dt>Attempts</dt><dd>{run.attempt} so far, never gives up on its own</dd>
            {/if}
            <dt>Last error</dt><dd style="white-space:normal;overflow-wrap:anywhere" data-last-error>{run.error}</dd>
            {#if run.status === "retrying"}
              <dt>Backoff</dt><dd>{secs(run.retry_in_ms)}, capped at {secs(flash?.backoff_cap_ms ?? 0)} between tries</dd>
              <dt>Next try</dt><dd class="num">in {Math.ceil(waitLeft / 1000)} s</dd>
            {:else if run.status === "stopped"}
              <dt>Backoff</dt><dd>stopped by hand</dd>
              <dt>Next try</dt><dd>not until you press Retry</dd>
            {/if}
          </dl>
          <div class="bar err" style="height:4px"><i style="width:{run.status === 'retrying' ? waited : 0}%"></i></div>
          <div style="display:flex;gap:6px;align-items:center;flex-wrap:nowrap">
            {#if run.status === "retrying"}
              <button class="btn sm pri" onclick={() => ask("flash.retry")}><Icon name="retry" />Retry now</button>
              <button class="btn sm" onclick={() => ask("flash.stop")}><Icon name="stop" />Stop retrying</button>
            {:else if run.status === "stopped"}
              <button class="btn sm pri" onclick={() => ask("flash.retry")}><Icon name="retry" />Retry</button>
            {:else if board && rom && image?.state === "ready"}
              <button class="btn sm pri" onclick={start}><Icon name="retry" />Start again</button>
            {/if}
            <span class="sp"></span>
            <button class="btn sm quiet" onclick={bundle}><Icon name="bundle" />Make support bundle</button>
          </div>
          <!-- Where the bundle was written, in full. -->
          <div class="faint" style="font-size:11px;white-space:normal;overflow-wrap:anywhere" data-bundled>
            {bundled ||
              (run.status === "retrying"
                ? "Stop retrying is a manual off. Nothing starts again until you press Retry."
                : stopped
                  ? "Nothing starts again until you press Retry."
                  : "Nothing is tried again on its own.")}
          </div>
        </div>
      </section>
      <section class="panel" data-panel="log">
        <div class="ph"><h2>Log</h2><span class="grow"></span><span class="kicker">this flash only</span></div>
        <div class="pb lines">
          {#each lines as line, index (index)}
            <div class="ln"><span class="t">{clock(line.at_ms)}</span><span class="lv-{line.level}">{line.level}</span><span>{line.text}</span></div>
          {/each}
        </div>
      </section>
    </div>
  {:else}
    <div style="display:grid;grid-template-rows:auto auto minmax(0,1fr);gap:8px;min-height:0;min-width:0">
      <section class="panel" data-panel="image" data-image={image?.state ?? "none"}>
        <div class="ph">
          <h2>Image</h2><span class="grow"></span><span class="chip neu">first-flash image</span>
          <!-- Always in its place: it cannot be pressed while a flash is running. -->
          <button class="btn sm" class:pri={app.ready && !active && image?.state !== "ready"} disabled={!app.ready || active} onclick={() => ask("flash.pick_image")}><Icon name="folder" />{image ? "Choose another…" : "Choose image…"}</button>
        </div>
        <div class="pb">
          <!-- The same four rows in every state, so the panels below never move. A refused file's
               reason takes the place of the three rows that have nothing to say about it. -->
          <dl class="kv" style="grid-template-rows:repeat(4,auto)">
            <dt style="grid-area:1/1">File</dt>
            <dd style="grid-area:1/2" data-file>
              {#if !app.ready}<div class="ph-skel" style="width:40%;margin-top:3px"></div>
              {:else if image}{image.file} <span class="faint">in {folder(image.path)}</span>
              {:else}none chosen yet{/if}
            </dd>
            {#if image?.state === "refused"}
              <dt style="grid-area:2/1;visibility:hidden">Size</dt>
              <dt style="grid-area:3/1;visibility:hidden">Built for</dt>
              <dt style="grid-area:4/1;visibility:hidden">Signature</dt>
              <dd class="note err" style="grid-area:2/1/5/-1;white-space:normal;overflow:hidden" data-refused>{image.words}</dd>
            {:else}
              <dt style="grid-area:2/1">Size</dt>
              <dd style="grid-area:2/2">
                {#if image?.state === "ready"}{size(image.bytes)} {compressed(image.file) ? "compressed" : "file"}, {size(image.raw_bytes)} written{:else if image}{size(image.bytes)}{image.state === "reading" ? " · reading" : ""}{:else}·{/if}
              </dd>
              <dt style="grid-area:3/1">Built for</dt><dd style="grid-area:3/2" data-built-for>{image?.state === "ready" ? `${image.board}${image.slots.length ? ` · ${image.slots.length === 1 ? "slot" : "slots"} ${image.slots.join(", ")}` : ""}` : "·"}</dd>
              <dt style="grid-area:4/1">Signature</dt><dd style="grid-area:4/2">{signatureWords(image, settings()?.security ?? false)}</dd>
            {/if}
          </dl>
        </div>
      </section>
      <section class="panel" data-panel="board">
        <div class="ph">
          <h2>Board</h2><span class="grow"></span>
          {#if !app.ready}
            <span class="chip info"><span class="dot info"></span>Scanning</span>
          {:else if board}
            <span class="chip acc"><span class="dot acc"></span>found in boot ROM</span>
          {:else}
            <span class="chip info"><span class="dot spin"></span>Looking on USB</span>
          {/if}
        </div>
        {#if !app.ready}
          <div class="pb"><Skel rows={4} row="" style="height:18px;display:flex;align-items:center" /><div class="note" style="margin-top:8px;visibility:hidden">·</div></div>
        {:else if board}
          <div class="pb">
            <dl class="kv">
              <dt>Found on</dt><dd>USB {port} · boot ID {board.identity.attributes["usb-id"] ?? "not reported"}</dd>
              <dt>Family</dt><dd>{rom?.label ?? board.identity.model ?? "boot ROM"}{inLoaderMode ? " · now in loader mode" : ""}</dd>
              <dt>Loader</dt><dd style="white-space:normal;overflow-wrap:anywhere" data-loader>{rom?.loader ?? "none in this build for this family"}</dd>
              <dt>Flasher image</dt><dd>not in this build</dd>
            </dl>
            <div class="note" class:warn={loaded} style="margin-top:8px" data-note="limit">
              {#if loaded}
                The loader ran and the board answers. Starting the flasher image and writing the image are not in this build yet. Nothing was written to the board.
              {:else}
                This build sends the loader and stops there. The flasher image and the write arrive with the flash flow. Nothing is written to the board.
              {/if}
            </div>
          </div>
        {:else if roms.length > 1}
          <div class="pb flush tbl">
            {#each roms as candidate (candidate.id)}
              <div class="tr link" role="button" tabindex="0" data-choose={candidate.id} style="grid-template-columns:minmax(0,1fr) 150px;min-height:26px" onclick={() => go("flash", candidate.id)} onkeydown={(e) => e.key === "Enter" && go("flash", candidate.id)}>
                <div><b>{nameOf(candidate)}</b> <span class="faint">USB {candidate.identity.address}</span></div>
                <div><span class="chip acc"><span class="dot acc"></span>Boot ROM, waiting</span></div>
              </div>
            {/each}
          </div>
        {:else}
          <!-- The same rows as for a board that was found, so the panel keeps its height and the log below does not move. -->
          <div class="pb" data-board="none">
            <dl class="kv">
              <dt>Found on</dt><dd class="faint">·</dd>
              <dt>Family</dt><dd class="faint">·</dd>
              <dt>Loader</dt><dd class="faint">·</dd>
              <dt>Flasher image</dt><dd class="faint">·</dd>
            </dl>
            <div class="note" style="margin-top:8px">
              No board is in its boot ROM. Plug one in over USB with its boot button held; it shows up here as soon as it does, and Atlas keeps looking.
            </div>
          </div>
        {/if}
      </section>
      <section class="panel" data-panel="sent">
        <div class="ph"><h2>Log</h2><span class="grow"></span><span class="kicker">this flash only</span></div>
        <div class="pb" style="display:flex;flex-direction:column;gap:8px">
          {#if run?.loaded}
            <dl class="kv">
              {#each run.loaded.facts as [label, value] (label)}
                <dt>{label}</dt><dd>{value}</dd>
              {/each}
            </dl>
          {/if}
          {#if run}
            <div class="lines" style="flex:1;min-height:0">
              {#each lines as line, index (index)}
                <div class="ln"><span class="t">{clock(line.at_ms)}</span><span class="lv-{line.level}">{line.level}</span><span>{line.text}</span></div>
              {/each}
            </div>
          {:else if app.ready}
            <div class="faint" style="font-size:11px">Nothing has been sent yet.</div>
          {:else}
            <Skel rows={2} row="" style="height:17px;display:flex;align-items:center" />
          {/if}
        </div>
      </section>
    </div>
  {/if}
</Page>
