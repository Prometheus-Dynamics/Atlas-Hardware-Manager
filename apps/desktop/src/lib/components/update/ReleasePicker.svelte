<script lang="ts">
  // Release choice for one family: a catalog entry, or a free-text version
  // for devices that fetch their own image.
  import type { ReleaseChoice } from "$lib/api/client";
  import Icon from "$lib/components/common/Icon.svelte";
  import { releases } from "$lib/stores/releases.svelte";
  import ReleaseOption from "./ReleaseOption.svelte";

  let {
    family,
    count,
    choice = $bindable(),
    current,
  }: { family: string; count: number; choice: ReleaseChoice; current: string[] } = $props();

  const entries = $derived(releases.forFamily(family));
  const custom = $derived(!choice.release_id);
  const upToDate = $derived(!!choice.version && current.length > 0 && current.every((v) => v === choice.version));

  function pickCustom() {
    choice = { version: choice.version, release_id: null };
  }
</script>

<div class="flex flex-col gap-2">
  <div class="flex items-baseline justify-between gap-2">
    <p class="text-[13px] font-semibold text-fg">{family}</p>
    <p class="truncate text-[12px] text-fg-faint">
      {count} device{count === 1 ? "" : "s"} · now <span class="mono">{[...new Set(current)].join(", ") || "unknown"}</span>
    </p>
  </div>

  {#each entries as entry (entry.id)}
    <ReleaseOption
      {entry}
      name="release-{family}"
      selected={choice.release_id === entry.id}
      onselect={() => (choice = { version: entry.version, release_id: entry.id })}
    />
  {/each}

  <div class="other" class:selected={custom}>
    {#if entries.length > 0}
      <label class="flex items-center gap-2 text-[12.5px] text-fg-muted">
        <input type="radio" name="release-{family}" checked={custom} onchange={pickCustom} class="accent-[var(--accent)]" />
        Another version (the device fetches it)
      </label>
    {/if}
    {#if custom}
      <input class="input mono mt-2" placeholder="e.g. 2026.3.0" bind:value={choice.version} aria-label="{family} version" />
      {#if entries.length === 0}
        <p class="hint mt-1.5">
          No catalog entries for this family. Devices that fetch their own image only need a version; add a file on the
          Releases page for devices that need one.
        </p>
      {/if}
    {/if}
  </div>

  {#if upToDate}
    <p class="flex items-center gap-1.5 text-[12px] text-warn-fg">
      <Icon name="info-circle" size={14} />Already on {choice.version}; this reinstalls it.
    </p>
  {/if}
</div>

<style>
  .other {
    padding: 2px 4px;
  }
</style>
