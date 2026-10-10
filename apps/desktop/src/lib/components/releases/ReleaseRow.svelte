<script lang="ts">
  import { api, openExternal, type ReleaseEntry } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import ConfirmButton from "#lib/components/common/ConfirmButton.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import Pill from "#lib/components/common/Pill.svelte";
  import ProgressBar from "#lib/components/common/ProgressBar.svelte";
  import { bytes, type Tone } from "#lib/format.ts";
  import { releases } from "#lib/stores/releases.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";

  let { entry }: { entry: ReleaseEntry } = $props();

  const progress = $derived(releases.downloads.get(entry.id));
  const channel: Record<ReleaseEntry["channel"], { label: string; tone: Tone }> = {
    stable: { label: "Stable", tone: "success" },
    beta: { label: "Beta", tone: "info" },
    local: { label: "Local file", tone: "neutral" },
  };

  async function togglePin() {
    await api.setReleasePinned(entry.id, !entry.pinned);
    await releases.load();
  }

  async function remove() {
    await api.removeRelease(entry.id);
    await releases.load();
    toasts.success(`Removed ${entry.family} ${entry.version}.`);
  }
</script>

<div class="row">
  <div class="main min-w-0">
    <p class="mono truncate text-[13px] font-medium text-fg" title={entry.version}>{entry.version}</p>
    <p class="truncate text-[12px] text-fg-faint" title={entry.path ?? entry.artifact_name}>
      <span class="mono">{entry.artifact_name}</span> · {entry.origin.kind === "remote" ? `from ${entry.origin.source}` : "added by you"}{entry.boards.length ? ` · ${entry.boards.join(", ")}` : ""}
    </p>
  </div>
  <div class="pills flex flex-wrap items-center gap-1.5">
    <Pill tone={channel[entry.channel].tone} label={channel[entry.channel].label} />
    {#if entry.signed}
      <Pill tone="success" icon="shield-check" label="Signed" />
    {:else}
      <Pill tone="warning" icon="alert-triangle" label="Unsigned" title="No signature: where this file came from can't be verified" />
    {/if}
  </div>
  <span class="size text-right text-[12.5px] tabular-nums text-fg-muted">{bytes(entry.size_bytes)}</span>
  <div class="state flex min-w-0 items-center justify-end">
    {#if progress}
      <div class="flex w-full flex-col gap-1">
        <ProgressBar value={progress.total ? progress.downloaded / progress.total : null} label="Download progress" size={5} />
        <span class="text-[11.5px] tabular-nums text-fg-faint">
          {bytes(progress.downloaded)}{progress.total ? ` of ${bytes(progress.total)}` : ""}
        </span>
      </div>
    {:else if entry.path}
      <span class="flex items-center gap-1.5 whitespace-nowrap text-[12.5px] text-ok-fg"><Icon name="device-sd-card" size={15} />On this computer</span>
    {:else}
      <Button size="sm" icon="download" onclick={() => releases.download(entry.id)}>Download</Button>
    {/if}
  </div>
  <div class="tools flex items-center justify-end gap-1">
    {#if entry.notes_url}
      <Button variant="ghost" size="sm" iconRight="external-link" onclick={() => openExternal(entry.notes_url!)}>Notes</Button>
    {/if}
    {#if entry.channel === "local"}
      <Button
        variant="ghost"
        size="sm"
        icon={entry.pinned ? "pinned" : "pin"}
        label={entry.pinned ? "Unpin" : "Pin"}
        title={entry.pinned ? "Pinned: stays in the list" : "Pin to keep it; unpinned images drop off after the 5 newest"}
        action={togglePin}
      />
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
    border-radius: var(--r-card);
    transition: background var(--t-fast);
  }
  .row:hover {
    background: var(--glass);
  }
  /* In a narrower family card: two lines. What it is and where it stands on
     top; channel, signature, size and the row's tools below. */
  @container (max-width: 760px) {
    .row {
      grid-template-columns: minmax(0, 1fr) auto auto;
      grid-template-areas:
        "main main state"
        "pills size tools";
      gap: 6px 12px;
      padding: 10px 12px;
    }
    .main {
      grid-area: main;
    }
    .pills {
      grid-area: pills;
    }
    .size {
      grid-area: size;
    }
    .state {
      grid-area: state;
    }
    .tools {
      grid-area: tools;
    }
  }
</style>
