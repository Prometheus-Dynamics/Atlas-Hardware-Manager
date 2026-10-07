<script lang="ts">
  // One list of images for this board: the catalog, a file used once, and
  // "Choose file…". A picked file is used once; "Keep in list" saves it.
  import { api, errorText, pickReleaseFile, type ReleaseEntry } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import ReleaseOption from "#lib/components/update/ReleaseOption.svelte";
  import type { UpdateDraft } from "#lib/components/update/updateDraft.svelte.ts";
  import { sentence } from "#lib/format.ts";
  import { versionFromFileName } from "#lib/present.ts";
  import { releases } from "#lib/stores/releases.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { rise, stagger } from "#lib/ui/motion.ts";

  let { draft, family, entries }: { draft: UpdateDraft; family: string; entries: ReleaseEntry[] } = $props();

  const choice = $derived(draft.choices[family]);
  const oneTime = $derived(choice?.path ?? null);
  const oneTimeName = $derived(oneTime ? (oneTime.split(/[\\/]/).pop() ?? oneTime) : null);
  let adding = $state(false);

  async function chooseFile() {
    if (adding) return;
    adding = true;
    try {
      const path = await pickReleaseFile();
      if (!path) return;
      draft.chooseFile(family, path, versionFromFileName(path));
    } catch (error) {
      toasts.error(sentence(errorText(error)));
    } finally {
      adding = false;
    }
  }

  async function keepInList() {
    if (!oneTime) return;
    try {
      const added = await api.addLocalRelease(oneTime, family, choice.version);
      await releases.load();
      draft.choose(family, added.id);
      toasts.success(`Saved ${added.artifact_name} to your images.`);
    } catch (error) {
      toasts.error(sentence(errorText(error)));
    }
  }
</script>

<div class="flex flex-col gap-2">
  {#each entries as option, i (option.id)}
    <div in:rise={{ delay: stagger(i) }}>
      <ReleaseOption
        entry={option}
        name="software-image"
        selected={choice?.release_id === option.id && !oneTime}
        onselect={() => draft.choose(family, option.id)}
      />
    </div>
  {/each}
  {#if oneTime}
    <div class="once" in:rise>
      <span class="tile"><Icon name="file-text" size={18} /></span>
      <span class="min-w-0 flex-1">
        <span class="mono block truncate text-[13px] font-medium text-fg" title={oneTime}>{oneTimeName}</span>
        <span class="block text-[12px] text-fg-faint">Just this once · not added to your images</span>
      </span>
      <Button size="sm" variant="ghost" icon="pin" action={keepInList}>Keep in list</Button>
    </div>
  {/if}
  <button type="button" class="choose" onclick={chooseFile} disabled={adding}>
    <span class="tile"><Icon name={adding ? "loader-2" : "folder-open"} size={18} class={adding ? "spin" : ""} /></span>
    <span class="min-w-0 flex-1 text-left">
      <span class="block text-[13px] font-medium text-fg">Choose file…</span>
      <span class="block text-[12px] text-fg-faint">An .img, .img.xz or .img.zst image; used once unless you keep it</span>
    </span>
    <Icon name="chevron-right" size={16} class="text-fg-faint" />
  </button>
</div>

<style>
  .once {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    border-radius: var(--r-card);
    border: 1px solid var(--accent-ring);
    background: var(--accent-tint);
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
</style>
