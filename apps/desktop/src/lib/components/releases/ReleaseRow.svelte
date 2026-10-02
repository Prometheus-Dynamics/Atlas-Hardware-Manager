<script lang="ts">
  import { api, openExternal, type ReleaseEntry } from "$lib/api/client";
  import Button from "$lib/components/common/Button.svelte";
  import ConfirmButton from "$lib/components/common/ConfirmButton.svelte";
  import Icon from "$lib/components/common/Icon.svelte";
  import Pill from "$lib/components/common/Pill.svelte";
  import ProgressBar from "$lib/components/common/ProgressBar.svelte";
  import { bytes, type Tone } from "$lib/format";
  import { releases } from "$lib/stores/releases.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";

  let { entry }: { entry: ReleaseEntry } = $props();

  const progress = $derived(releases.downloads.get(entry.id));
  const channel: Record<ReleaseEntry["channel"], { label: string; tone: Tone }> = {
    stable: { label: "Stable", tone: "success" },
    beta: { label: "Beta", tone: "info" },
    local: { label: "Local file", tone: "neutral" },
  };

  async function remove() {
    await api.removeRelease(entry.id);
    await releases.load();
    toasts.success(`Removed ${entry.family} ${entry.version}.`);
  }
</script>

<div class="row">
  <div class="min-w-0">
    <p class="mono truncate text-[13px] font-medium text-fg">{entry.version}</p>
    <p class="truncate text-[12px] text-fg-faint" title={entry.path ?? entry.artifact_name}>
      <span class="mono">{entry.artifact_name}</span> · {entry.origin.kind === "remote" ? `from ${entry.origin.source}` : "added by you"}{entry.boards.length ? ` · ${entry.boards.join(", ")}` : ""}
    </p>
  </div>
  <div class="flex flex-wrap items-center gap-1.5">
    <Pill tone={channel[entry.channel].tone} label={channel[entry.channel].label} />
    {#if entry.signed}
      <Pill tone="success" icon="shield-check" label="Signed" />
    {:else}
      <Pill tone="warning" icon="alert-triangle" label="Unsigned" title="No signature: Atlas can't verify where this file came from" />
    {/if}
  </div>
  <span class="text-right text-[12.5px] tabular-nums text-fg-muted">{bytes(entry.size_bytes)}</span>
  <div class="flex min-w-0 items-center justify-end">
    {#if progress}
      <div class="flex w-full flex-col gap-1">
        <ProgressBar value={progress.total ? progress.downloaded / progress.total : null} label="Download progress" size={5} />
        <span class="text-[11.5px] tabular-nums text-fg-faint">
          {bytes(progress.downloaded)}{progress.total ? ` of ${bytes(progress.total)}` : ""}
        </span>
      </div>
    {:else if entry.path}
      <span class="flex items-center gap-1.5 text-[12.5px] text-ok-fg"><Icon name="device-sd-card" size={15} />On this computer</span>
    {:else}
      <Button size="sm" icon="download" onclick={() => releases.download(entry.id)}>Download</Button>
    {/if}
  </div>
  <div class="flex items-center justify-end gap-1">
    {#if entry.notes_url}
      <Button variant="ghost" size="sm" iconRight="external-link" onclick={() => openExternal(entry.notes_url!)}>Notes</Button>
    {/if}
    <ConfirmButton action={remove} size="sm" variant="ghost" icon="trash" label="Remove {entry.version}" prompt="Remove?" confirmLabel="Remove" />
  </div>
</div>

<style>
  .row {
    display: grid;
    grid-template-columns: minmax(200px, 1.6fr) minmax(170px, 1fr) 70px minmax(140px, 0.8fr) minmax(110px, auto);
    align-items: center;
    gap: 16px;
    padding: 10px 14px;
    border-radius: 12px;
    transition: background var(--t-fast);
  }
  .row:hover {
    background: var(--glass);
  }
</style>
