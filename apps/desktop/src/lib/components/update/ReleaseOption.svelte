<script lang="ts">
  // One selectable release row: a radio styled as a glass row.
  import type { ReleaseEntry } from "$lib/api/client";
  import Icon from "$lib/components/common/Icon.svelte";
  import { bytes } from "$lib/format";
  import { releases } from "$lib/stores/releases.svelte";

  let {
    entry,
    selected,
    name,
    onselect,
  }: { entry: ReleaseEntry; selected: boolean; name: string; onselect: () => void } = $props();

  const origin = $derived(entry.origin.kind === "local-file" ? "Local file" : entry.channel === "beta" ? "Beta" : "Stable");
  const download = $derived(releases.downloads.get(entry.id));
</script>

<label class="option" class:selected>
  <input type="radio" {name} checked={selected} onchange={onselect} class="sr-only" />
  <span class="radio" aria-hidden="true"><span class="inner"></span></span>
  <span class="min-w-0 flex-1">
    <span class="block truncate text-[13px] font-medium text-fg" title={entry.artifact_name}>{entry.version}</span>
    <span class="block truncate text-[12px] text-fg-faint">
      {origin} ·
      {#if entry.signed}<span class="text-ok-fg">signed</span>{:else}<span class="text-warn-fg">unsigned</span>{/if}
      · {bytes(entry.size_bytes)}{#if download} · downloading…{:else if !entry.path} · downloads first{/if}
    </span>
  </span>
  {#if entry.origin.kind === "local-file"}<Icon name="folder-open" size={15} class="text-fg-faint" />{/if}
</label>

<style>
  .option {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    border-radius: var(--r-card);
    background: var(--glass);
    border: 0.5px solid var(--glass-border);
    cursor: pointer;
    transition:
      background var(--t-fast),
      border-color var(--t-fast),
      box-shadow var(--t-fast);
  }
  .option:hover {
    background: var(--glass-hover);
  }
  .option:focus-within {
    box-shadow: 0 0 0 2px var(--accent-ring);
  }
  .option.selected {
    background: var(--accent-tint);
    border-color: var(--accent-ring);
  }
  .radio {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 1.5px solid var(--fg-faint);
    flex-shrink: 0;
    transition: border-color var(--t-fast);
  }
  .inner {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--accent);
    transform: scale(0);
    transition: transform var(--t-med) var(--ease-out);
  }
  .selected .radio {
    border-color: var(--accent);
  }
  .selected .inner {
    transform: scale(1);
  }
</style>
