<script lang="ts">
  // Everything about what runs on one device: what is installed, which
  // image to install, and how: in place (keeps settings) or fresh (erases
  // it, through USB boot). One button that says what will happen.
  import { goto } from "$app/navigation";
  import { api, errorText, keyString, type DeviceAction, type DeviceRecord, type SlotSet } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import HandoffView from "#lib/components/software/HandoffView.svelte";
  import ImageList from "#lib/components/software/ImageList.svelte";
  import InstalledCard from "#lib/components/software/InstalledCard.svelte";
  import SlotsCard from "#lib/components/software/SlotsCard.svelte";
  import Checkbox from "#lib/components/common/Checkbox.svelte";
  import ProgressBar from "#lib/components/common/ProgressBar.svelte";
  import MethodCard from "#lib/components/software/MethodCard.svelte";
  import UsbBootSteps from "#lib/components/software/UsbBootSteps.svelte";
  import { handoffs } from "#lib/components/software/handoff.svelte.ts";
  import { imagesFor, preferredImage, updateMethods, usbBootSteps, USB_BOOT_ACTION, versionPart } from "#lib/components/software/software.ts";
  import { UpdateDraft } from "#lib/components/update/updateDraft.svelte.ts";
  import { bytes, deviceName, primaryVersion, sentence } from "#lib/format.ts";
  import { deviceStatus } from "#lib/stores/status.svelte.ts";
  import { isRecovery } from "#lib/present.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { releases } from "#lib/stores/releases.svelte.ts";
  import { system } from "#lib/stores/system.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import { rise } from "#lib/ui/motion.ts";
  import BootloaderRow from "./BootloaderRow.svelte";
  import FlashChecks from "./FlashChecks.svelte";
  import HostReadiness from "./HostReadiness.svelte";
  import SshKeyOption from "./SshKeyOption.svelte";

  let { record }: { record: DeviceRecord } = $props();

  /** Update methods that mean the board can be put into USB boot by hand. */
  const USB_METHODS = ["image-write", "usb-boot-reboot", "rpiboot-recovery"];

  // svelte-ignore state_referenced_locally
  const family = record.key.family;
  // svelte-ignore state_referenced_locally
  const draft = new UpdateDraft({ devices: [record.key], releases: {}, staged: system.settings?.staged_default ?? "auto" });

  const handoff = $derived(handoffs.get(keyString(record.key)));
  const recovery = $derived(isRecovery(record));
  const online = $derived(record.presence === "online");
  const methods = $derived(updateMethods(record));
  const ab = $derived(methods.includes("ab-tryboot"));
  /** Dynamic slots: an update installs the image beside the others. */
  const slotted = $derived(record.capabilities.includes("slots"));
  let slotSet = $state<SlotSet | null>(null);
  let slotsReload = $state(0);
  let bootAfter = $state(true);
  let pinIt = $state(false);
  let installing = $state<number | null>(null);
  const entries = $derived(imagesFor(record, releases.entries));
  const choice = $derived(draft.choices[family]);
  const version = $derived(choice?.version.trim() ?? "");
  const entry = $derived(choice?.path ? undefined : releases.entries.find((e) => e.id === choice?.release_id));
  const steps = $derived(usbBootSteps(record, devices.all));
  /** Already past USB boot: the eMMC is exposed and gets written directly. */
  const exposed = $derived(record.identity.attributes?.stage === "storage");
  const summary = $derived(draft.plan?.devices[0]?.plan.summary ?? null);

  // Start from the image that fits best, once the catalog is there. The
  // list spans families, so the draft's own default may be empty or odd.
  let preselected = false;
  $effect(() => {
    if (preselected || entries.length === 0) return;
    preselected = true;
    const preferred = preferredImage(record, entries);
    if (preferred && !draft.choices[family]?.path) draft.choose(family, preferred.id);
  });

  /** The board's "Restart into USB boot" action, when it offers one. */
  let usbBoot = $state<DeviceAction | null>(null);
  $effect(() => {
    if (recovery || !online || !record.capabilities.includes("actions")) {
      usbBoot = null;
      return;
    }
    const key = record.key;
    let stale = false;
    checkingActions = true;
    api
      .deviceActions(key)
      .then((list) => !stale && (usbBoot = list.find((a) => a.id === USB_BOOT_ACTION) ?? null))
      .catch(() => !stale && (usbBoot = null))
      .finally(() => !stale && (checkingActions = false));
    return () => (stale = true);
  });

  const updateBlocked = $derived.by((): string | null => {
    if (recovery) return "The board is in USB boot, so it can only be installed fresh.";
    if (!online) return "The board isn't reachable. Connect it to update in place.";
    if (slotted) return null;
    if (record.capabilities.includes("update")) return null;
    if (methods.length > 0 && !ab) return "This board needs one fresh install to get the A/B layout.";
    return "This device can't update in place.";
  });

  /** How a fresh install gets the board into USB boot; null when it can't. */
  const fresh = $derived.by((): "flash" | "restart" | "manual" | null => {
    if (recovery) return "flash";
    if (usbBoot) return "restart";
    if (methods.some((m) => USB_METHODS.includes(m))) return "manual";
    return null;
  });
  const freshBlocked = $derived(fresh === "flash" && !online ? "Connect the board in USB boot to install it." : null);

  let picked = $state<"update" | "fresh" | null>(null);
  const method = $derived.by(() => {
    if (picked === "update" && !updateBlocked) return "update";
    if (picked === "fresh" && fresh && !freshBlocked) return "fresh";
    if (!updateBlocked) return "update";
    return fresh && !freshBlocked ? "fresh" : null;
  });

  /** A host problem that would make USB boot fail, such as missing boot files. */
  let hostBlocked = $state(false);
  let confirming = $state(false);
  /** The board's actions are still loading, so how it reaches USB boot isn't known yet. */
  let checkingActions = $state(false);
  let handing = $state(false);
  const installed = $derived(primaryVersion(record.identity));
  const reinstall = $derived(method === "update" && !!version && !!installed && versionPart(version) === versionPart(installed));

  /** The plan's error matters only where the plan is what runs. */
  const problem = $derived(method === "update" || fresh === "flash" ? draft.planError : null);

  // What the chosen install does, step by step, for the panel under the button.
  const update = $derived(deviceStatus.get(record)?.update ?? null);
  const active = $derived(update?.slot_active ?? record.identity.attributes?.slot_active ?? null);
  const spare = $derived(active === "A" ? "B" : active === "B" ? "A" : null);
  const plan = $derived.by(() => {
    if (!version) return [];
    const get = entry ? (entry.path ? "Already on this computer" : `Download ${bytes(entry.size_bytes)}`) : "Read the file you chose";
    if (method === "update" && slotted) {
      return [
        { title: entry && !entry.path ? "Download it on the Releases page first" : get, note: "" },
        { title: "Copy it to the board beside the other slots", note: "Every file is checked against its manifest before anything is replaced" },
        ...(bootAfter
          ? [
              { title: "Restart into it once, on trial", note: "If it doesn't come up, the board goes back to the default by itself" },
              { title: "Keep it as the default", note: "Only once it confirms its boot; the old default stays as the way back" },
            ]
          : [{ title: "Leave it installed", note: "Boot it once from Slots when you're ready" }]),
      ];
    }
    if (method === "update" && ab) {
      return [
        { title: get, note: entry?.signed ? "Its signature is checked" : "" },
        { title: `Write it to slot ${spare ?? "the spare"}`, note: `Slot ${active ?? "the running one"} keeps running ${primaryVersion(record.identity) ?? "what it runs"} meanwhile` },
        { title: `Restart into slot ${spare ?? "the spare"} on trial`, note: "Settings and data stay" },
        { title: "Check it came up", note: "It has to confirm its boot, or the board goes back by itself" },
        { title: "Keep it", note: `Slot ${active ?? "the old one"} stays as the way back` },
      ];
    }
    if (method === "update") return [{ title: get, note: "" }, { title: "Install and restart", note: "Settings stay" }];
    return [
      { title: fresh === "flash" ? "It's in USB boot" : "Restart into USB boot", note: "" },
      { title: get, note: "" },
      { title: "Write the whole storage", note: "Everything on it is erased" },
      { title: "Read it back to verify", note: "" },
      { title: "Restart into it", note: "" },
    ];
  });

  /** Exactly what gets written: the file, its size and digest when known. */
  const writes = $derived.by(() => {
    if (!version) return null;
    const name = entry?.artifact_name ?? (choice?.path ?? "").split(/[\\/]/).pop() ?? "";
    if (!name) return null;
    const parts = [name];
    if (entry?.size_bytes) parts.push(bytes(entry.size_bytes));
    parts.push(entry?.sha256 ? `SHA-256 ${entry.sha256.slice(0, 12)}…` : "checked when it starts");
    return parts.join(" · ");
  });

  /** The bundle file for a slot install: chosen, or a release already here. */
  const bundlePath = $derived(choice?.path ?? entry?.path ?? null);
  /** Slots take slot bundles; a whole-disk image is for a fresh install. */
  const notBundle = $derived(!!version && !(entry?.artifact_name ?? bundlePath ?? "").endsWith(".slot.tar"));
  const label = $derived.by(() => {
    if (!version) return "Choose an image";
    if (method === "update" && slotted) return bootAfter ? `Install ${version} and boot it once` : `Install ${version} as a slot`;
    if (method === "update") return `Update to ${version} · keeps settings`;
    if (method === "fresh") return `Erase and install ${version}`;
    return "Not available";
  });
  const disabled = $derived.by(() => {
    if (!version || !method) return true;
    if (method === "update" && slotted) return !bundlePath || notBundle || installing !== null;
    if (method === "update") return !draft.plan || draft.planning;
    if (fresh === "flash") return !draft.plan || draft.planning || hostBlocked;
    return hostBlocked;
  });

  async function flash(ignoreChecksum = false) {
    const job = await draft.start(ignoreChecksum);
    if (job === null || method === "update") return;
    ui.selectedJob = job;
    ui.close();
    void goto("/jobs");
  }

  async function handOff() {
    confirming = false;
    handing = true;
    try {
      await handoffs.begin(record, draft.buildRequest().releases[family], draft.staged, fresh === "restart");
    } catch (error) {
      toasts.error(sentence(errorText(error)));
    } finally {
      handing = false;
    }
  }

  async function installSlot() {
    if (!bundlePath) return;
    installing = 0;
    try {
      await api.installSlot(
        record.key,
        bundlePath,
        pinIt,
        bootAfter,
        (u) => {
          if (u.type === "step-progress") installing = u.fraction;
        },
        // A catalog release is checked against its own digest: a cached file
        // that isn't it is refused before anything is sent.
        entry?.sha256 || null,
      );
      toasts.success(bootAfter ? `${version} is installed; ${deviceName(record)} is restarting into it once.` : `${version} is installed on ${deviceName(record)}.`);
    } catch (error) {
      toasts.error(sentence(errorText(error)));
    } finally {
      installing = null;
      slotsReload++;
    }
  }

  function go() {
    if (method === "update" && slotted) return void installSlot();
    if (method === "fresh" && fresh === "restart") confirming = true;
    else if (method === "fresh" && fresh === "manual") void handOff();
    else void flash(false);
  }
</script>

{#if handoff}
  <HandoffView {handoff} {steps} />
{:else}
  <!-- One column, or two in a wide panel: what's installed and what to
       install | how, its checks, and the button. The button stays in view
       at the bottom either way. -->
  <div class="software">
    <div class="pick flex min-w-0 flex-col gap-6">
    <section class="flex flex-col gap-2">
      <h3 class="section-title">Installed</h3>
      {#if slotted && !recovery}
        <SlotsCard {record} bind:set={slotSet} reload={slotsReload} />
      {:else}
        <InstalledCard {record} />
      {/if}
    </section>

    <section class="flex flex-col gap-2">
      <h3 class="section-title">Install</h3>
      <ImageList {draft} {family} {entries} />
      {#if reinstall}
        <p class="flex items-center gap-1.5 text-[12px] text-fg-faint">
          <Icon name="info-circle" size={14} />Already on {version}; this installs it again.
        </p>
      {/if}
    </section>
    </div>

    <section class="how flex min-w-0 flex-col gap-3">
      <h3 class="section-title">How</h3>
      <div class="auto-grid" style="--min: 220px; --gap: 8px; --max: {fresh ? 2 : 1}">
        <MethodCard
          title={slotted ? "Install as a slot" : "Update in place"}
          icon="arrow-up"
          selected={method === "update"}
          disabled={updateBlocked}
          onselect={() => ((picked = "update"), (confirming = false))}
        >
          <span>
            {#if slotted}
              Goes beside the installed ones; settings and data stay.{slotSet ? ` ${bytes(Math.max(0, slotSet.free_bytes - slotSet.reserve_bytes))} free.` : ""}
            {:else}
              Keeps settings and data.{ab ? " If the new version doesn't start, the board goes back to this one by itself." : ""}
            {/if}
          </span>
          {#if method === "update" && summary && !slotted}<span class="text-fg-faint">{summary}</span>{/if}
        </MethodCard>
        {#if fresh}
          <MethodCard
            title="Fresh install"
            icon="bolt"
            danger
            selected={method === "fresh"}
            disabled={freshBlocked}
            onselect={() => (picked = "fresh")}
          >
            <span>Erases everything on the board, including settings.</span>
            <span class="text-fg-faint">
              {#if fresh === "flash"}
                {exposed ? "Its storage is showing as a disk and gets written directly." : "It's in USB boot and ready."}
              {:else if checkingActions}
                Checking whether it can be restarted into USB boot…
              {:else if fresh === "restart"}
                It restarts into USB boot, then the install starts.
              {:else}
                Put it into USB boot by hand; the install starts when it shows up.
              {/if}
            </span>
          </MethodCard>
        {/if}
      </div>

      {#if method === "update" && slotted}
        <div class="flex flex-wrap gap-x-5 gap-y-2" in:rise>
          <span class="flex items-center gap-2 text-[12.5px] text-fg">
            <Checkbox checked={bootAfter} label="Boot it once when it's in" onclick={() => (bootAfter = !bootAfter)} />Boot it once when it's in
          </span>
          <span class="flex items-center gap-2 text-[12.5px] text-fg">
            <Checkbox checked={pinIt} label="Pin it" onclick={() => (pinIt = !pinIt)} />Pin it <span class="text-fg-faint">(kept from removal)</span>
          </span>
        </div>
      {/if}
      {#if method === "fresh"}
        <div class="flex flex-col gap-3" in:rise>
          {#if fresh === "manual"}
            <div class="glass flex flex-col gap-2 px-4 py-3">
              <p class="text-[12.5px] font-medium text-fg">Getting it into USB boot</p>
              <UsbBootSteps {steps} />
            </div>
          {/if}
          {#if entry}<FlashChecks {entry} />{/if}
          {#if fresh === "flash" && !exposed}<BootloaderRow {record} />{/if}
          <HostReadiness bind:blocking={hostBlocked} />
          <SshKeyOption />
        </div>
      {/if}
    </section>

    <div class="act flex min-w-0 flex-col gap-2">
      {#if problem && version}
        <p class="problem" role="alert" in:rise><Icon name="alert-circle" size={16} />{problem}</p>
        {#if draft.checksumFailed}
          <div class="flex gap-2">
            <Button size="sm" icon="refresh" disabled={draft.starting} onclick={() => flash(false)}>Download again</Button>
            <Button size="sm" variant="danger" icon="alert-triangle" disabled={draft.starting} onclick={() => flash(true)}>
              {method === "update" ? "Update anyway" : "Install anyway"}
            </Button>
          </div>
        {/if}
      {/if}

      {#if confirming}
        <div class="confirm" in:rise {@attach (el) => el.scrollIntoView({ block: "nearest" })}>
          <p class="text-[13px] text-fg">
            {deviceName(record)} stops running and restarts into USB boot. Everything on it is erased, including settings.
          </p>
          <div class="flex gap-2">
            <Button variant="primary" icon="bolt" busy={handing} onclick={handOff}>Erase and install</Button>
            <Button variant="ghost" disabled={handing} onclick={() => (confirming = false)}>Cancel</Button>
          </div>
        </div>
      {:else}
        <Button
          variant="primary"
          size="lg"
          full
          icon={method === "fresh" ? "bolt" : "arrow-up"}
          busy={draft.starting || handing || installing !== null}
          {disabled}
          onclick={go}
        >
          {label}
        </Button>
      {/if}
      {#if writes && !confirming}
        <p class="hint" title={entry?.sha256 ?? ""}>Writes {writes}</p>
      {/if}
      {#if method === "update" && slotted && notBundle}
        <p class="hint">That's a whole-disk image, for a fresh install. A slot installs from a slot bundle (.slot.tar).</p>
      {/if}
      {#if installing !== null}
        <ProgressBar value={installing} label="Copying to the board" />
      {/if}
      {#if entries.length === 0 && !version}
        <p class="hint text-center">No images for this model yet. Choose a file, or add one on the Releases page.</p>
      {/if}
      {#if plan.length && !confirming}
        <section class="panel plan" aria-label="What happens">
          <div class="panel-head">
            <h2 class="panel-title">What happens</h2>
            {#if spare && method === "update" && ab}<span class="panel-note">A/B: the running version is never touched</span>{/if}
          </div>
          {#if spare && method === "update" && ab}
            <div class="slots" aria-hidden="true">
              <span class="slot running"><b>Slot {active}</b>{primaryVersion(record.identity) ?? "—"} · keeps running</span>
              <span class="slot next"><b>Slot {spare}</b>{version} · written</span>
            </div>
          {/if}
          <ol>
            {#each plan as step, i (i)}
              <li>
                <span class="num">{i + 1}</span>
                <span class="min-w-0"><span class="block text-[12.5px] text-fg">{step.title}</span>{#if step.note}<span class="block text-[11.5px] text-fg-muted">{step.note}</span>{/if}</span>
              </li>
            {/each}
          </ol>
        </section>
      {/if}
    </div>
  </div>
{/if}

<style>
  .software {
    display: flex;
    flex-direction: column;
    gap: 24px;
  }
  /* The action sticks to the bottom of the tab while the rest scrolls. */
  .act {
    position: sticky;
    bottom: 0;
    z-index: 1;
    margin: -12px 0 -16px;
    padding: 12px 0 16px;
    background: linear-gradient(to bottom, transparent, var(--layer) 12px);
  }
  @container (min-width: 640px) {
    .software {
      display: grid;
      grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
      grid-template-rows: auto 1fr;
      grid-template-areas:
        "pick how"
        "pick act";
      gap: 16px 28px;
      align-items: start;
    }
    .pick {
      grid-area: pick;
    }
    .how {
      grid-area: how;
    }
    .act {
      grid-area: act;
      /* Side by side the button needn't stick: the column holds it and the plan. */
      position: static;
      margin: -4px 0 0;
      padding: 0;
      background: none;
    }
  }
  .plan ol {
    display: flex;
    flex-direction: column;
  }
  .plan li {
    display: flex;
    gap: 10px;
    padding: 6px 0;
    border-top: 1px solid var(--hairline);
  }
  .plan li:first-child {
    border-top: 0;
  }
  .num {
    width: 20px;
    height: 20px;
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--r-sm);
    background: var(--inset);
    font-size: 11px;
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
  }
  .slots {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
  }
  .slot {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 6px 10px;
    border-radius: var(--r-sm);
    font-family: var(--font-code);
    font-size: 11.5px;
    color: var(--fg-muted);
    border: 1px solid var(--glass-border);
  }
  .slot b {
    font-family: var(--font-ui, inherit);
    font-size: 11.5px;
    font-weight: 600;
    color: var(--fg);
  }
  .slot.next {
    border-color: var(--accent-ring);
    background: var(--accent-tint);
  }
  .problem {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    padding: 10px 14px;
    border-radius: var(--r-card);
    background: var(--err-bg);
    color: var(--err-fg);
    font-size: 13px;
    line-height: 1.45;
  }
  .confirm {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 14px;
    border-radius: var(--r-card);
    border: 1px solid var(--err-bg);
    background: var(--glass);
  }
</style>
