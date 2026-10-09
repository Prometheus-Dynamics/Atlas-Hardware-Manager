<script lang="ts">
  // Everything about what runs on one device: what is installed, which
  // image to install, and how: in place (keeps settings) or fresh (erases
  // it, through USB boot). One button that says what will happen.
  import { goto } from "$app/navigation";
  import { api, errorText, keyString, type DeviceAction, type DeviceRecord } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import HandoffView from "#lib/components/software/HandoffView.svelte";
  import ImageList from "#lib/components/software/ImageList.svelte";
  import InstalledCard from "#lib/components/software/InstalledCard.svelte";
  import MethodCard from "#lib/components/software/MethodCard.svelte";
  import UsbBootSteps from "#lib/components/software/UsbBootSteps.svelte";
  import { handoffs } from "#lib/components/software/handoff.svelte.ts";
  import { imagesFor, preferredImage, updateMethods, usbBootSteps, USB_BOOT_ACTION, versionPart } from "#lib/components/software/software.ts";
  import { UpdateDraft } from "#lib/components/update/updateDraft.svelte.ts";
  import { deviceName, primaryVersion, sentence } from "#lib/format.ts";
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

  const label = $derived.by(() => {
    if (!version) return "Choose an image";
    if (method === "update") return `Update to ${version} · keeps settings`;
    if (method === "fresh") return `Erase and install ${version}`;
    return "Not available";
  });
  const disabled = $derived.by(() => {
    if (!version || !method) return true;
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

  function go() {
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
      <InstalledCard {record} />
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
          title="Update in place"
          icon="arrow-up"
          selected={method === "update"}
          disabled={updateBlocked}
          onselect={() => ((picked = "update"), (confirming = false))}
        >
          <span>
            Keeps settings and data.{ab ? " If the new version doesn't start, the board goes back to this one by itself." : ""}
          </span>
          {#if method === "update" && summary}<span class="text-fg-faint">{summary}</span>{/if}
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
          busy={draft.starting || handing}
          {disabled}
          onclick={go}
        >
          {label}
        </Button>
      {/if}
      {#if entries.length === 0 && !version}
        <p class="hint text-center">No images for this model yet. Choose a file, or add one on the Releases page.</p>
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
      margin-top: -4px;
    }
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
