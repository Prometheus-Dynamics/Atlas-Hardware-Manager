<script lang="ts">
  import { api, errorText } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import ConfirmButton from "#lib/components/common/ConfirmButton.svelte";
  import Field from "#lib/components/common/Field.svelte";
  import GlassCard from "#lib/components/common/GlassCard.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import { sentence } from "#lib/format.ts";
  import { releases } from "#lib/stores/releases.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { rise } from "#lib/ui/motion.ts";

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

<GlassCard title="Where releases come from" subtitle="Signed manifests Atlas checks for new releases" icon="world-www" large>
  {#snippet actions()}
    {#if !adding}<Button size="sm" icon="plus" onclick={() => (adding = true)}>Add source</Button>{/if}
  {/snippet}

  {#if releases.sources.length === 0 && !adding}
    <p class="text-[13px] text-fg-muted">No remote sources. Only local files are available.</p>
  {/if}

  <ul class="flex flex-col gap-2">
    {#each releases.sources as source (source.name)}
      <li class="glass flex items-start justify-between gap-3 px-4 py-3">
        <div class="min-w-0">
          <p class="text-[13px] font-semibold text-fg">{source.name}</p>
          <p class="mono truncate text-[12px] text-fg-muted">{source.index_url}</p>
          {#each source.public_keys as key (key)}
            <p class="mono mt-0.5 flex items-center gap-1.5 truncate text-[11.5px] text-fg-faint" title={key}><Icon name="key" size={12} />{key}</p>
          {:else}
            <p class="mt-1 text-[12.5px] text-warn-fg">No public keys: every manifest from this source is rejected.</p>
          {/each}
        </div>
        <ConfirmButton action={() => remove(source.name)} size="sm" variant="ghost" icon="trash" label="Remove {source.name}" prompt="Remove {source.name}?" confirmLabel="Remove" />
      </li>
    {/each}
  </ul>

  {#if adding}
    <form class="glass mt-3 grid grid-cols-[10rem_1fr] gap-3 p-4" onsubmit={add} in:rise>
      <Field label="Name"><input class="input" bind:value={name} required placeholder="pd-stable" /></Field>
      <Field label="Index URL"><input class="input mono" type="url" bind:value={url} required placeholder="https://…/index.json" /></Field>
      <Field label="Public keys, one per line" class="col-span-2">
        <textarea class="textarea mono" bind:value={keys} placeholder="ed25519:…"></textarea>
      </Field>
      {#if error}<p class="col-span-2 flex items-center gap-2 text-[13px] text-err-fg" role="alert"><Icon name="alert-circle" size={15} />{error}</p>{/if}
      <div class="col-span-2 flex gap-2">
        <Button type="submit" busy={saving} icon="device-floppy">Save source</Button>
        <Button variant="ghost" onclick={() => (adding = false)}>Cancel</Button>
      </div>
    </form>
  {/if}
</GlassCard>
