<script lang="ts">
  import { api, errorText, pickReleaseFile } from "$lib/api/client";
  import Panel from "$lib/components/common/Panel.svelte";
  import { sentence } from "$lib/format";
  import { devices } from "$lib/stores/devices.svelte";
  import { releases } from "$lib/stores/releases.svelte";
  import { system } from "$lib/stores/system.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";

  let { onclose }: { onclose: () => void } = $props();

  let path = $state<string | null>(null);
  let family = $state(devices.families[0] ?? releases.families[0] ?? "");
  let version = $state("");
  let saving = $state(false);
  let error = $state<string | null>(null);

  const families = $derived([...new Set([...devices.families, ...releases.families])].sort());

  async function pick() {
    try {
      const picked = await pickReleaseFile();
      if (picked) {
        path = picked;
        // Guess a version from names like image-2026.3.0.img.xz.
        const match = picked.match(/(\d+\.\d+(?:\.\d+)?(?:-[\w.]+)?)/);
        if (match && !version) version = match[1].replace(/\.(img|bin|hex|zip)$/, "");
      }
    } catch (e) {
      toasts.error(errorText(e));
    }
  }

  async function add(event: SubmitEvent) {
    event.preventDefault();
    if (!path || saving) return;
    saving = true;
    error = null;
    try {
      const entry = await api.addLocalRelease(path, family.trim(), version.trim());
      await releases.load();
      toasts.success(`Added ${entry.family} ${entry.version}.`);
      onclose();
    } catch (e) {
      error = sentence(errorText(e));
    } finally {
      saving = false;
    }
  }
</script>

<Panel eyebrow="Add" title="Local release file">
  <form class="grid grid-cols-[1fr_12rem_10rem] items-end gap-3" onsubmit={add}>
    <div class="flex flex-col gap-1">
      <span class="micro-label">File</span>
      <div class="flex items-center gap-2">
        <button type="button" class="btn btn-sm preset-tonal" onclick={pick}>
          <i class="fa-solid fa-folder-open" aria-hidden="true"></i>Choose file…
        </button>
        <span class="truncate font-mono text-[0.7rem] {path ? 'text-surface-200' : 'text-surface-500'}">{path ?? "No file chosen"}</span>
      </div>
    </div>
    <label class="flex flex-col gap-1">
      <span class="micro-label">Family</span>
      <input class="field" list="release-families" bind:value={family} required />
      <datalist id="release-families">
        {#each families as f (f)}<option value={f}></option>{/each}
      </datalist>
    </label>
    <label class="flex flex-col gap-1">
      <span class="micro-label">Version</span>
      <input class="field font-mono" bind:value={version} required placeholder="e.g. 2026.3.0" />
    </label>
    <p class="col-span-3 text-[0.7rem] text-warning-300">
      <i class="fa-solid fa-triangle-exclamation mr-1" aria-hidden="true"></i>Local files are unsigned.
      {#if system.settings && !system.settings.allow_unsigned_local}
        Installing them is turned off in Settings (allow unsigned local files).
      {/if}
    </p>
    {#if error}<p class="col-span-3 text-xs text-error-300" role="alert">{error}</p>{/if}
    <div class="col-span-3 flex gap-2">
      <button type="submit" class="btn btn-sm preset-filled-primary-500" disabled={!path || saving}>
        {#if saving}<i class="fa-solid fa-circle-notch fa-spin" aria-hidden="true"></i>{/if}Add to catalog
      </button>
      <button type="button" class="btn btn-sm preset-tonal" onclick={onclose}>Cancel</button>
    </div>
  </form>
</Panel>
