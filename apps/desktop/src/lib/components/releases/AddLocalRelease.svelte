<script lang="ts">
  import { api, errorText, pickReleaseFile } from "$lib/api/client";
  import Button from "$lib/components/common/Button.svelte";
  import Field from "$lib/components/common/Field.svelte";
  import GlassCard from "$lib/components/common/GlassCard.svelte";
  import Icon from "$lib/components/common/Icon.svelte";
  import { sentence } from "$lib/format";
  import { versionFromFileName } from "$lib/present";
  import { devices } from "$lib/stores/devices.svelte";
  import { releases } from "$lib/stores/releases.svelte";
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
        if (!version) version = versionFromFileName(picked);
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

<GlassCard title="Add a file from this computer" subtitle="Images and firmware you built or downloaded" icon="file-plus" large>
  <form class="flex flex-col gap-4" onsubmit={add}>
    <div class="flex items-center gap-3">
      <Button icon="folder-open" onclick={pick}>Choose file…</Button>
      <span class="mono truncate text-[12.5px] {path ? 'text-fg' : 'text-fg-faint'}">{path ?? "No file chosen"}</span>
    </div>
    <div class="grid grid-cols-2 gap-3">
      <Field label="Device family" hint="Which devices this file is for, e.g. rpi for boards in USB boot.">
        <input class="input" list="release-families" bind:value={family} required />
        <datalist id="release-families">
          {#each families as f (f)}<option value={f}></option>{/each}
        </datalist>
      </Field>
      <Field label="Version">
        <input class="input mono" bind:value={version} required placeholder="e.g. 2026.3.0" />
      </Field>
    </div>
    <p class="flex items-center gap-2 text-[12.5px] text-warn-fg">
      <Icon name="alert-triangle" size={15} />Local files are unsigned. Atlas hashes them now and warns if they change before installing.
    </p>
    {#if error}<p class="flex items-center gap-2 text-[13px] text-err-fg" role="alert"><Icon name="alert-circle" size={15} />{error}</p>{/if}
    <div class="flex gap-2">
      <Button type="submit" variant="primary" busy={saving} disabled={!path}>Add to catalog</Button>
      <Button variant="ghost" onclick={onclose}>Cancel</Button>
    </div>
  </form>
</GlassCard>
