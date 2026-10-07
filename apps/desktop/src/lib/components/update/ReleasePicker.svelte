<script lang="ts">
  // Release choice for one family in a fleet update: a catalog entry. A
  // typed version only where the devices fetch their own image.
  import type { ReleaseChoice, ReleaseEntry } from "#lib/api/client.ts";
  import Icon from "#lib/components/common/Icon.svelte";
  import ReleaseOption from "./ReleaseOption.svelte";

  let {
    family,
    count,
    choice = $bindable(),
    current,
    needsFile,
    entries,
  }: {
    family: string;
    count: number;
    choice: ReleaseChoice;
    current: string[];
    needsFile: boolean;
    /** Images that fit these devices, newest first. */
    entries: ReleaseEntry[];
  } = $props();
  // A robot's target that isn't in the catalog: the devices fetch it themselves.
  // svelte-ignore state_referenced_locally
  const requested = !choice.release_id && !choice.path && choice.version.trim() ? choice.version.trim() : null;
  const upToDate = $derived(!!choice.version && current.length > 0 && current.every((v) => v === choice.version));
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

  {#if requested && entries.length > 0}
    <label class="flex items-center gap-2 px-1 text-[12.5px] text-fg-muted">
      <input
        type="radio"
        name="release-{family}"
        checked={!choice.release_id}
        onchange={() => (choice = { version: requested, release_id: null })}
        class="accent-[var(--accent)]"
      />
      <span class="mono">{requested}</span> · the devices fetch it
    </label>
  {:else if entries.length === 0 && needsFile}
    <p class="hint">No images for {family} yet. Add one on the Releases page.</p>
  {:else if entries.length === 0}
    <input class="input mono" placeholder="Version, e.g. 2026.3.0" bind:value={choice.version} aria-label="{family} version" />
    <p class="hint -mt-1">These devices fetch the version themselves.</p>
  {/if}

  {#if upToDate}
    <p class="flex items-center gap-1.5 text-[12px] text-fg-faint">
      <Icon name="info-circle" size={14} />Already on {choice.version}; this installs it again.
    </p>
  {/if}
</div>
