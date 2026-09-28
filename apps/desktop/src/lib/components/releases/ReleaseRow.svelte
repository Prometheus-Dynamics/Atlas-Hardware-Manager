<script lang="ts">
  import { api, openExternal, type ReleaseEntry } from "$lib/api/client";
  import ConfirmButton from "$lib/components/common/ConfirmButton.svelte";
  import ProgressBar from "$lib/components/common/ProgressBar.svelte";
  import Tag from "$lib/components/common/Tag.svelte";
  import { bytes, type Tone } from "$lib/format";
  import { releases } from "$lib/stores/releases.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";

  let { entry }: { entry: ReleaseEntry } = $props();

  const progress = $derived(releases.downloads.get(entry.id));
  const channelTone: Record<ReleaseEntry["channel"], Tone> = { stable: "success", beta: "info", local: "neutral" };

  async function remove() {
    await api.removeRelease(entry.id);
    await releases.load();
    toasts.success(`Removed ${entry.family} ${entry.version}.`);
  }
</script>

<tr class="border-b border-surface-800/70 text-xs last:border-b-0">
  <td class="px-3 py-1.5 font-mono text-surface-50">{entry.version}</td>
  <td class="px-2 py-1.5"><Tag tone={channelTone[entry.channel]} label={entry.channel} /></td>
  <td class="px-2 py-1.5">
    {#if entry.signed}
      <Tag tone="success" icon="fa-signature" label="signed" />
    {:else}
      <Tag tone="warning" icon="fa-triangle-exclamation" label="unsigned" title="No signature: Atlas cannot verify where this file came from" />
    {/if}
  </td>
  <td class="px-2 py-1.5">
    <span class="block truncate font-mono text-[0.68rem] text-surface-300" title={entry.path ?? entry.artifact_name}>{entry.artifact_name}</span>
    <span class="block text-[0.6rem] text-surface-500">
      {entry.origin.kind === "remote" ? `from ${entry.origin.source}` : "local file"}{entry.boards.length ? ` · ${entry.boards.join(", ")}` : ""}
    </span>
  </td>
  <td class="px-2 py-1.5 text-right text-surface-300">{bytes(entry.size_bytes)}</td>
  <td class="w-40 px-2 py-1.5">
    {#if progress}
      <ProgressBar value={progress.total ? progress.downloaded / progress.total : null} label="Download progress" />
      <span class="text-[0.6rem] text-surface-400">
        {bytes(progress.downloaded)}{progress.total ? ` of ${bytes(progress.total)}` : ""}
      </span>
    {:else if entry.path}
      <span class="text-success-400"><i class="fa-solid fa-hard-drive mr-1" aria-hidden="true"></i>on disk</span>
    {:else}
      <button type="button" class="btn btn-sm preset-tonal" onclick={() => releases.download(entry.id)}>
        <i class="fa-solid fa-download" aria-hidden="true"></i>Download
      </button>
    {/if}
  </td>
  <td class="px-2 py-1.5">
    {#if entry.notes_url}
      <button type="button" class="text-secondary-300 hover:underline" onclick={() => openExternal(entry.notes_url!)}>
        Notes <i class="fa-solid fa-arrow-up-right-from-square text-[0.55rem]" aria-hidden="true"></i>
      </button>
    {/if}
  </td>
  <td class="px-3 py-1.5 text-right">
    <ConfirmButton action={remove} prompt="Remove?" confirmLabel="Remove" class="text-surface-500 hover:text-error-300">
      <i class="fa-solid fa-trash" aria-hidden="true"></i><span class="sr-only">Remove {entry.version}</span>
    </ConfirmButton>
  </td>
</tr>
