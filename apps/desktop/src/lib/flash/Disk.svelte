<script lang="ts">
  // Write a disk (docs/mockups/flash-disk.html): the image, the removable disks, and the erase
  // confirmed in place in the footer. Erasing a card cannot be undone, so it is the one action
  // here that asks; the footer keeps its height through every state.
  import Page from "../ui/Page.svelte";
  import Icon from "../ui/Icon.svelte";
  import { call } from "../api/client";
  import { breadcrumb, inventory, scan, topic } from "../stores/app.svelte";
  import { go } from "../stores/route.svelte";
  import { size } from "../ui/format";
  import { kindOf, nameOf } from "../devices/state";
  import type { FlashView } from "./types";

  let { id }: { id: string } = $props();

  const flash = $derived(topic<FlashView>("flash"));
  const image = $derived(flash?.image ?? null);
  const write = $derived(flash?.disk ?? null);
  const disks = $derived(inventory().devices.filter((d) => d.presence === "online" && d.identity.state === "removable-disk"));
  const chosen = $derived(disks.find((d) => d.id === id) ?? (id === "" && disks.length === 1 ? disks[0] : undefined));
  const dev = (d: (typeof disks)[number]) => kindOf(d) || d.identity.address;
  const bus = (d: (typeof disks)[number]) => {
    const link = scan()?.links.find((l) => l.id === d.identity.link)?.label ?? "disk";
    const port = d.identity.attributes["usb-port"];
    return `${link}${port ? ` · USB ${port}` : ""} · removable`;
  };
  const writing = $derived(write?.status === "writing");

  let confirming = $state(false);
  let busy = $state(false);
  async function ask(name: string, args?: unknown) {
    if (busy) return;
    busy = true;
    breadcrumb(name);
    try {
      await call(name, args);
    } catch {
      // Logged by the core; the footer shows the state the core is in.
    } finally {
      busy = false;
    }
  }
  const erase = async () => {
    if (!chosen) return;
    confirming = false;
    await ask("disk.write", { id: chosen.id });
  };
  // A different disk or image is a different question.
  $effect(() => {
    void chosen?.id;
    void image?.path;
    confirming = false;
  });

  const percent = $derived(write && write.input_total > 0 ? Math.min(100, Math.round((100 * write.input_done) / write.input_total)) : 0);
</script>

<Page title="Write a disk" crumb="Flash /" grid="grid-template-rows:auto minmax(0,1fr); grid-template-columns:minmax(0,1fr);">
  {#snippet chips()}
    {#if chosen}
      <span class="chip info"><span class="dot info"></span>{dev(chosen)} selected</span>
    {/if}
  {/snippet}

  <section class="panel" data-panel="image" data-image={image?.state ?? "none"}>
    <div class="ph">
      <h2>Image</h2><span class="grow"></span><span class="chip neu">first-flash image</span>
      {#if !writing}
        <button class="btn sm" class:pri={image?.state !== "ready"} onclick={() => ask("flash.pick_image")}><Icon name="folder" />{image ? "Choose another…" : "Choose image…"}</button>
      {/if}
    </div>
    <div class="pb">
      {#if image?.state === "refused"}
        <div class="note err ell" style="margin-bottom:8px" title={image.words}>{image.words}</div>
      {/if}
      <dl class="kv">
        <dt>File</dt><dd title={image?.path}>{image ? `${image.file} · ${size(image.bytes)}` : "none chosen yet"}{image?.state === "reading" ? " · reading" : ""}</dd>
        <dt>Written as</dt><dd>the whole disk image, same as the first flash on a board</dd>
        <dt>Verify</dt><dd>reads the card back after writing</dd>
      </dl>
    </div>
  </section>

  <section class="panel" style="min-height:0" data-panel="disks">
    <div class="ph"><h2>Disk</h2><span class="grow"></span><span class="kicker">system and internal disks are not listed</span></div>
    {#if disks.length === 0}
      <div class="pb" style="display:flex;min-height:0;padding:0">
        <div class="empty-box"><div>No removable disk found. Insert a card or plug in a reader; it shows up here by itself.</div></div>
      </div>
    {:else}
      <div class="pb flush tbl rows">
        <div class="tr h" style="grid-template-columns:18px minmax(0,1.2fr) 80px minmax(0,1fr) 110px">
          <div></div><div>Disk</div><div>Device</div><div>Bus</div><div>Kind</div>
        </div>
        {#each disks as disk (disk.id)}
          {@const on = disk.id === chosen?.id}
          <div class="tr link" class:sel={on} role="radio" aria-checked={on} tabindex="0" data-disk={disk.id} style="grid-template-columns:18px minmax(0,1.2fr) 80px minmax(0,1fr) 110px" onclick={() => !writing && go("disk", disk.id)} onkeydown={(e) => e.key === "Enter" && !writing && go("disk", disk.id)}>
            <div>
              {#if on}<span class="dot acc"></span>{:else}<span class="dot off" style="background:transparent;border:1.5px solid var(--offline)"></span>{/if}
            </div>
            <div><b>{nameOf(disk)}</b></div>
            <div class="faint">{dev(disk)}</div>
            <div>{bus(disk)}</div>
            <div><span class="chip info"><span class="dot info"></span>Removable</span></div>
          </div>
        {/each}
      </div>
    {/if}
    <div style="margin-top:auto;border-top:1px solid var(--hairline);padding:8px 10px;min-height:52px;min-width:0" data-foot={write ? write.status : confirming ? "confirm" : "idle"} data-phase={write?.phase}>
      <div style="display:flex;align-items:center;min-height:24px;min-width:0">
      {#if write?.status === "writing"}
        <div style="flex:1;min-width:0;display:flex;flex-direction:column;gap:4px">
          <div class="bar" class:info={write.phase !== "writing"} style="height:6px"><i style="width:{percent}%"></i></div>
          <div class="muted ell" style="font-size:11px">
            {#if write.phase === "dropped" || write.phase === "reattaching"}
              {write.name} went away at {size(write.written)}. Waiting for it to come back; the write goes on from there.
            {:else if write.phase === "checking" || write.phase === "preparing"}
              Checking {write.file} and {write.name}
            {:else}
              {percent}% · {size(write.written)} written and read back · {write.file} to {write.name}
            {/if}
          </div>
        </div>
        <button class="btn sm" style="margin-left:8px" onclick={() => ask("disk.cancel")}>Cancel</button>
      {:else if write?.status === "done"}
        <div class="note ok ell" style="flex:1" title={write.report?.sha256}>
          {write.file} is on {write.name}: {size(write.report?.bytes_written ?? 0)} written and read back. It can be removed.
        </div>
        <button class="btn sm" style="margin-left:8px" onclick={() => ask("disk.dismiss")}>Done</button>
      {:else if write}
        <div class="note err ell" style="flex:1" title={write.words}>
          {write.status === "cancelled" ? `Cancelled. ${write.name} is partly written and needs a new write.` : write.words}
        </div>
        <button class="btn sm" style="margin-left:8px" onclick={() => ask("disk.dismiss")}>Dismiss</button>
      {:else if confirming && chosen && image?.state === "ready"}
        <div class="confirm-line" style="flex:1;height:auto;min-height:24px;padding:4px 8px;background:var(--err-bg);border-left:2px solid var(--err);border-radius:2px">
          <span class="q">Erase {dev(chosen)} ({nameOf(chosen)}) and write {image.file}? This cannot be undone.</span>
          <button class="btn sm" onclick={() => (confirming = false)}>Cancel</button>
          <button class="btn sm danger" onclick={erase}>Erase and write</button>
        </div>
      {:else if flash && !flash.can_write_disks}
        <div class="muted ell" style="flex:1">This build cannot write a disk: its helper is not installed beside the app.</div>
      {:else if image?.state !== "ready"}
        <div class="muted ell" style="flex:1">Choose a first-flash image to write.</div>
      {:else if !chosen}
        <div class="muted ell" style="flex:1">{disks.length === 0 ? "Waiting for a removable disk." : "Choose the disk to write."}</div>
      {:else}
        <div class="muted ell" style="flex:1">Writes {image.file} to {nameOf(chosen)} ({dev(chosen)}). Everything on it is erased.</div>
        <button class="btn sm pri" style="margin-left:8px" onclick={() => (confirming = true)}>Erase and write…</button>
      {/if}
      </div>
    </div>
  </section>
</Page>
