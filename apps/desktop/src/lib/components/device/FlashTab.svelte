<script lang="ts">
  // Flash a device in USB boot: how to get it there (left), which image
  // to write and what Atlas checks (right), then one red button.
  import { goto } from "$app/navigation";
  import { api, errorText, pickReleaseFile, type DeviceRecord } from "$lib/api/client";
  import Button from "$lib/components/common/Button.svelte";
  import Icon from "$lib/components/common/Icon.svelte";
  import FlashChecks from "./FlashChecks.svelte";
  import ReleaseOption from "$lib/components/update/ReleaseOption.svelte";
  import { UpdateDraft } from "$lib/components/update/updateDraft.svelte";
  import { sentence } from "$lib/format";
  import { displayModel, versionFromFileName } from "$lib/present";
  import { releases } from "$lib/stores/releases.svelte";
  import { system } from "$lib/stores/system.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { ui } from "$lib/stores/ui.svelte";
  import { rise } from "$lib/ui/motion";
  import BootloaderRow from "./BootloaderRow.svelte";

  let { record }: { record: DeviceRecord } = $props();

  // svelte-ignore state_referenced_locally
  const family = record.key.family;
  // svelte-ignore state_referenced_locally
  const draft = new UpdateDraft({ devices: [record.key], releases: {}, staged: system.settings?.staged_default ?? "auto" });

  const entries = $derived(releases.forFamily(family));
  const choice = $derived(draft.choices[family]);
  const entry = $derived(entries.find((e) => e.id === choice?.release_id));
  const steps = $derived(
    (record.identity.attributes?.recovery_steps ?? "")
      .split("\n")
      .map((s) => s.trim())
      .filter(Boolean),
  );

  let adding = $state(false);

  async function chooseFile() {
    if (adding) return;
    adding = true;
    try {
      const path = await pickReleaseFile();
      if (!path) return;
      const added = await api.addLocalRelease(path, family, versionFromFileName(path));
      await releases.load();
      draft.choose(family, added.id);
      toasts.success(`Added ${added.artifact_name}.`);
    } catch (error) {
      toasts.error(sentence(errorText(error)));
    } finally {
      adding = false;
    }
  }

  async function flash(ignoreChecksum = false) {
    const job = await draft.start(ignoreChecksum);
    if (job === null) return;
    ui.selectedJob = job;
    ui.close();
    void goto("/jobs");
  }
</script>

<div class="grid grid-cols-1 gap-6 md:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)]">
  <section class="flex flex-col gap-3">
    <h3 class="section-title">Getting it into USB boot</h3>
    {#if steps.length > 0}
      <ol class="flex flex-col gap-2">
        {#each steps as step, i (i)}
          <li class="glass flex gap-3 px-3.5 py-3" in:rise={{ delay: i * 40 }}>
            <span class="num">{i + 1}</span>
            <span class="text-[13px] leading-relaxed text-fg">{step}</span>
          </li>
        {/each}
      </ol>
      <p class="hint flex items-center gap-1.5"><Icon name="circle-check" size={14} class="text-ok-fg" />It's in USB boot now, so you can flash right away.</p>
    {:else}
      <p class="text-[13px] leading-relaxed text-fg-muted">
        This {displayModel(record)} is connected in USB boot and ready for an image.
      </p>
    {/if}
    <BootloaderRow {record} />
  </section>

  <section class="flex flex-col gap-3">
    <h3 class="section-title">Flash an image</h3>
    <div class="flex flex-col gap-2">
      {#each entries as option (option.id)}
        <div in:rise>
          <ReleaseOption entry={option} name="flash-image" selected={choice?.release_id === option.id} onselect={() => draft.choose(family, option.id)} />
        </div>
      {/each}
      <button type="button" class="choose" onclick={chooseFile} disabled={adding}>
        <span class="tile"><Icon name={adding ? "loader-2" : "folder-open"} size={18} class={adding ? "spin" : ""} /></span>
        <span class="min-w-0 flex-1 text-left">
          <span class="block text-[13px] font-medium text-fg">Choose image file…</span>
          <span class="block text-[12px] text-fg-faint">An .img or .img.xz you built or downloaded</span>
        </span>
        <Icon name="chevron-right" size={16} class="text-fg-faint" />
      </button>
    </div>

    {#if entry}
      <FlashChecks {entry} />
    {/if}

    {#if draft.planError}
      <p class="problem" role="alert" in:rise><Icon name="alert-circle" size={16} />{draft.planError}</p>
      {#if draft.checksumFailed}
        <div class="flex gap-2">
          <Button size="sm" icon="refresh" disabled={draft.starting} onclick={() => flash(false)}>Download again</Button>
          <Button size="sm" variant="danger" icon="alert-triangle" disabled={draft.starting} onclick={() => flash(true)}>Flash anyway</Button>
        </div>
      {/if}
    {/if}

    <Button
      variant="primary"
      size="lg"
      full
      icon="bolt"
      busy={draft.starting}
      disabled={!draft.plan || record.presence !== "online"}
      onclick={() => flash(false)}
    >
      Flash this {displayModel(record)}
    </Button>
    {#if !entry && entries.length === 0}
      <p class="hint text-center">Choose an image file to continue.</p>
    {/if}
  </section>
</div>

<style>
  .num {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    flex-shrink: 0;
    border-radius: 50%;
    background: var(--glass-strong);
    color: var(--fg-muted);
    font-size: 12px;
    font-weight: 600;
  }
  .choose {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    border-radius: var(--r-card);
    border: 1px dashed var(--glass-border-strong);
    background: transparent;
    transition:
      background var(--t-fast),
      border-color var(--t-fast),
      transform var(--t-fast);
  }
  .choose:hover {
    background: var(--glass);
    border-color: var(--accent-ring);
  }
  .choose:active {
    transform: scale(0.99);
  }
  .tile {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border-radius: 9px;
    background: var(--accent-tint);
    color: var(--accent-text-strong);
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
</style>
