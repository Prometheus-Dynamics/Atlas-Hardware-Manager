<script lang="ts">
  // Release choice for one family: a catalog entry, or a free-text version
  // for devices that fetch their own image.
  import type { ReleaseChoice } from "$lib/api/client";
  import Tag from "$lib/components/common/Tag.svelte";
  import { bytes } from "$lib/format";
  import { releases } from "$lib/stores/releases.svelte";

  const CUSTOM = "\u0000custom";

  let {
    family,
    count,
    choice = $bindable(),
    current,
  }: { family: string; count: number; choice: ReleaseChoice; current: string[] } = $props();

  const entries = $derived(releases.forFamily(family));
  const selectedId = $derived(choice.release_id ?? CUSTOM);
  const entry = $derived(entries.find((e) => e.id === choice.release_id));
  const upToDate = $derived(!!choice.version && current.length > 0 && current.every((v) => v === choice.version));

  function pick(id: string) {
    if (id === CUSTOM) {
      choice = { version: entry?.version ?? choice.version, release_id: null };
      return;
    }
    const next = entries.find((e) => e.id === id);
    if (next) choice = { version: next.version, release_id: next.id };
  }
</script>

<div class="rounded-base border border-surface-800 bg-surface-950/40 p-3">
  <div class="mb-2 flex items-baseline justify-between gap-2">
    <p class="text-xs font-semibold text-surface-100">{family}</p>
    <p class="text-[0.65rem] text-surface-500">
      {count} device{count === 1 ? "" : "s"} · now {[...new Set(current)].join(", ") || "unknown"}
    </p>
  </div>

  {#if entries.length > 0}
    <label class="flex flex-col gap-1">
      <span class="micro-label">Release</span>
      <select class="field" value={selectedId} onchange={(e) => pick(e.currentTarget.value)}>
        {#each entries as option (option.id)}
          <option value={option.id}>
            {option.version} · {option.channel}{option.signed ? "" : " · unsigned"}{option.path ? "" : " · not downloaded"}
          </option>
        {/each}
        <option value={CUSTOM}>Other version (device fetches it)…</option>
      </select>
    </label>
  {/if}

  {#if !entry}
    <label class="mt-2 flex flex-col gap-1">
      <span class="micro-label">Version</span>
      <input class="field font-mono" placeholder="e.g. 2026.3.0" bind:value={choice.version} />
    </label>
    {#if entries.length === 0}
      <p class="mt-1 text-[0.65rem] text-surface-500">
        No catalog entries for this family. Devices that fetch their own image only need a version; add a file on the
        Releases page for devices that need one.
      </p>
    {/if}
  {:else}
    <div class="mt-2 flex flex-wrap items-center gap-1.5 text-[0.65rem] text-surface-400">
      {#if entry.signed}
        <Tag tone="success" icon="fa-signature" label="signed" />
      {:else}
        <Tag tone="warning" icon="fa-triangle-exclamation" label="unsigned" title="Local file without a signature" />
      {/if}
      <span class="truncate font-mono">{entry.artifact_name}</span>
      <span>{bytes(entry.size_bytes)}</span>
      {#if !entry.path}<span class="text-secondary-300">downloads before the job starts</span>{/if}
    </div>
  {/if}

  {#if upToDate}
    <p class="mt-2 text-[0.65rem] text-warning-300">
      <i class="fa-solid fa-circle-info mr-1" aria-hidden="true"></i>Already on {choice.version}; this reinstalls it.
    </p>
  {/if}
</div>
