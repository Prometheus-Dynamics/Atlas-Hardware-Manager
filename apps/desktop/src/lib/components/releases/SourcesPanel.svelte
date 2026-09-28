<script lang="ts">
  import { api, errorText } from "$lib/api/client";
  import ConfirmButton from "$lib/components/common/ConfirmButton.svelte";
  import Panel from "$lib/components/common/Panel.svelte";
  import { sentence } from "$lib/format";
  import { releases } from "$lib/stores/releases.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";

  let adding = $state(false);
  let name = $state("");
  let url = $state("");
  let keys = $state("");
  let saving = $state(false);
  let error = $state<string | null>(null);

  async function add(event: SubmitEvent) {
    event.preventDefault();
    if (saving) return;
    saving = true;
    error = null;
    const public_keys = keys.split(/[\s,]+/).map((k) => k.trim()).filter(Boolean);
    try {
      await api.setReleaseSource({ name: name.trim(), index_url: url.trim(), public_keys });
      await releases.load();
      toasts.success(`Added source ${name.trim()}. Refresh to fetch its releases.`);
      name = url = keys = "";
      adding = false;
    } catch (e) {
      error = sentence(errorText(e));
    } finally {
      saving = false;
    }
  }

  async function remove(source: string) {
    await api.removeReleaseSource(source);
    await releases.load();
    toasts.success(`Removed source ${source}.`);
  }
</script>

<Panel eyebrow="Sources" title="Remote release sources" subtitle="Signed manifests Atlas checks for new releases.">
  {#snippet actions()}
    {#if !adding}
      <button type="button" class="btn btn-sm preset-tonal" onclick={() => (adding = true)}>
        <i class="fa-solid fa-plus" aria-hidden="true"></i>Add source
      </button>
    {/if}
  {/snippet}

  {#if releases.sources.length === 0 && !adding}
    <p class="text-xs text-surface-400">No remote sources. Only local files are available.</p>
  {/if}

  <ul class="flex flex-col gap-2">
    {#each releases.sources as source (source.name)}
      <li class="flex items-start justify-between gap-3 rounded-base border border-surface-800 px-3 py-2">
        <div class="min-w-0 text-xs">
          <p class="font-semibold text-surface-50">{source.name}</p>
          <p class="truncate font-mono text-[0.68rem] text-surface-300">{source.index_url}</p>
          {#each source.public_keys as key (key)}
            <p class="truncate font-mono text-[0.6rem] text-surface-500" title={key}>
              <i class="fa-solid fa-key mr-1" aria-hidden="true"></i>{key}
            </p>
          {:else}
            <p class="text-[0.65rem] text-warning-300">No public keys: every manifest from this source is rejected.</p>
          {/each}
        </div>
        <ConfirmButton action={() => remove(source.name)} prompt="Remove {source.name}?" confirmLabel="Remove" class="text-surface-500 hover:text-error-300">
          <i class="fa-solid fa-trash" aria-hidden="true"></i><span class="sr-only">Remove {source.name}</span>
        </ConfirmButton>
      </li>
    {/each}
  </ul>

  {#if adding}
    <form class="mt-3 grid grid-cols-[10rem_1fr] gap-3 rounded-base border border-surface-700 p-3" onsubmit={add}>
      <label class="flex flex-col gap-1">
        <span class="micro-label">Name</span>
        <input class="field" bind:value={name} required placeholder="pd-stable" />
      </label>
      <label class="flex flex-col gap-1">
        <span class="micro-label">Index URL</span>
        <input class="field font-mono" type="url" bind:value={url} required placeholder="https://…/index.json" />
      </label>
      <label class="col-span-2 flex flex-col gap-1">
        <span class="micro-label">Public keys (one per line)</span>
        <textarea class="field min-h-14 font-mono text-[0.7rem]" bind:value={keys} placeholder="ed25519:…"></textarea>
      </label>
      {#if error}<p class="col-span-2 text-xs text-error-300" role="alert">{error}</p>{/if}
      <div class="col-span-2 flex gap-2">
        <button type="submit" class="btn btn-sm preset-filled-primary-500" disabled={saving}>
          {#if saving}<i class="fa-solid fa-circle-notch fa-spin" aria-hidden="true"></i>{/if}Save source
        </button>
        <button type="button" class="btn btn-sm preset-tonal" onclick={() => (adding = false)}>Cancel</button>
      </div>
    </form>
  {/if}
</Panel>
