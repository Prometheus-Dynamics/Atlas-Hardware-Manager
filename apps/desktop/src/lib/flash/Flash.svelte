<script lang="ts">
  // Flash a board (docs/mockups/flash.html, flash-failed.html): the steps on the left, and on the
  // right the image, the board in its boot ROM and what was sent, or the failure with its manual
  // off. The page shows what the libraries do today: the image is read and the loader is sent;
  // the flasher image and the write are the flash flow library's (D6), which this build does not
  // have, and the page says so instead of drawing progress that did not happen.
  import Page from "../ui/Page.svelte";
  import Icon from "../ui/Icon.svelte";
  import { call } from "../api/client";
  import { app, breadcrumb, inventory, settings, topic } from "../stores/app.svelte";
  import { go } from "../stores/route.svelte";
  import { clock, size, when } from "../ui/format";
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
    else out.push({ key: "image", state: "cur", title: "Pick image", detail: image ? "reading the image" : "no image chosen yet" });
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
</script>

<Page title="Flash a board" crumb="Atlas /" grid="grid-template-columns:260px minmax(0,1fr); grid-template-rows:minmax(0,1fr);">
  {#snippet chips()}
    {#if board || run}
      <span class="chip acc"><span class="dot acc"></span>{rom?.label ?? run?.family ?? "Boot ROM"}</span>
      <span class="chip neu">{board ? nameOf(board) : run?.name}</span>
    {/if}
    {#if run && failing}
      <span class="chip err" data-flash-chip="failed">{run.status === "stopped" ? "stopped at loader" : "failed at loader"}</span>
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
      {#each steps as step, index (step.key)}
        <div class="step {step.state}" data-step={step.key} data-state={step.state}>
          <div class="mark">{#if step.state === "done"}&#10003;{:else if step.state === "fail"}!{:else}{index + 1}{/if}</div>
          <div style="min-width:0">
            <div class="st">{step.title}</div>
            <div class="sd" title={step.detail}>{step.detail}</div>
          </div>
        </div>
      {/each}
    </div>
    <div class="subhead"><h2 class="up" style="margin:0;color:var(--fg-muted)">Recent flashes here</h2></div>
    <div class="fill" style="padding:2px 10px" data-panel="recent">
      {#each (flash?.recent ?? []).slice(0, 6) as recent, index (recent.at_ms + recent.name)}
        {@const word = recentWord(recent)}
        <div class="tr" class:more-s={index >= 3} style="grid-template-columns:minmax(0,1fr) 88px;min-height:24px;font-size:11px;padding:0">
          <div><b>{recent.name}</b> <span class="faint">{when(recent.at_ms, app.now)}</span></div>
          <div><span class="chip {word.tone}">{#if word.tone !== "off"}<span class="dot {word.tone}"></span>{/if}{word.word}</span></div>
        </div>
      {:else}
        <div class="faint" style="font-size:11px;padding:6px 0">None yet on this computer.</div>
      {/each}
    </div>
  </section>

  {#if run && failing}
    <div style="display:grid;grid-template-rows:auto minmax(0,1fr);gap:8px;min-height:0;min-width:0">
      <section class="panel" data-panel="failure" data-status={run.status}>
        <div class="ph">
          <h2>Loader failed</h2><span class="grow"></span>
          {#if run.status === "retrying"}
            <span class="chip err"><span class="dot err"></span>Retrying</span>
          {:else if run.status === "stopped"}
            <span class="chip off"><span class="dot off"></span>Stopped</span>
          {:else}
            <span class="chip err"><span class="dot err"></span>Failed</span>
          {/if}
        </div>
        <div class="pb" style="display:flex;flex-direction:column;gap:8px">
          <div class="note err ell">
            {#if run.status === "retrying" || run.status === "stopped"}
              The board in boot ROM did not answer the loader. Nothing was written. The board is still safe to flash.
            {:else}
              The loader was refused and will not be tried again on its own. Nothing was written.
            {/if}
          </div>
          <dl class="kv">
            {#if run.status === "retrying" || run.status === "stopped"}
              <dt>Attempts</dt><dd>{run.attempt} so far, never gives up on its own</dd>
            {/if}
            <dt>Last error</dt><dd title={run.error}>{run.error}</dd>
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
          </div>
          <div class="faint ell" style="font-size:11px">Stop retrying is a manual off. Nothing starts again until you press Retry.</div>
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
          {#if !active}
            <button class="btn sm" class:pri={image?.state !== "ready"} onclick={() => ask("flash.pick_image")}><Icon name="folder" />{image ? "Choose another…" : "Choose image…"}</button>
          {/if}
        </div>
        <div class="pb">
          {#if image?.state === "refused"}
            <div class="note err ell" style="margin-bottom:8px" title={image.words}>{image.words}</div>
          {/if}
          <dl class="kv">
            <dt>File</dt><dd title={image?.path}>{image?.file ?? "none chosen yet"}</dd>
            <dt>Size</dt>
            <dd>
              {#if image?.state === "ready"}{size(image.bytes)} file, {size(image.raw_bytes)} written{:else if image}{size(image.bytes)}{image.state === "reading" ? " · reading" : ""}{:else}·{/if}
            </dd>
            <dt>Built for</dt><dd>{image?.state === "ready" ? image.board : "·"}</dd>
            <dt>Signature</dt><dd>{signatureWords(image, settings()?.security ?? false)}</dd>
          </dl>
        </div>
      </section>
      <section class="panel" data-panel="board">
        <div class="ph">
          <h2>Board</h2><span class="grow"></span>
          {#if board}
            <span class="chip acc"><span class="dot acc"></span>found in boot ROM</span>
          {:else}
            <span class="chip info"><span class="dot spin"></span>Looking on USB</span>
          {/if}
        </div>
        {#if board}
          <div class="pb">
            <dl class="kv">
              <dt>Found on</dt><dd>USB {port} · boot ID {board.identity.attributes["usb-id"] ?? "not reported"}</dd>
              <dt>Family</dt><dd>{rom?.label ?? board.identity.model ?? "boot ROM"}{inLoaderMode ? " · now in loader mode" : ""}</dd>
              <dt>Loader</dt><dd title={rom?.loader}>{rom?.loader ?? "none in this build for this family"}</dd>
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
          <div class="pb muted" data-board="none">
            No board is in its boot ROM. Plug one in over USB with its boot button held; it shows up here as soon as it does, and Atlas keeps looking.
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
          {:else}
            <div class="faint" style="font-size:11px">Nothing has been sent yet.</div>
          {/if}
        </div>
      </section>
    </div>
  {/if}
</Page>
