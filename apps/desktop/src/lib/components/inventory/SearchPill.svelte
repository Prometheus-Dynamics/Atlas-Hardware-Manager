<script lang="ts">
  import Icon from "#lib/components/common/Icon.svelte";
  import { ui } from "#lib/stores/ui.svelte.ts";

  let input: HTMLInputElement | undefined = $state();
  $effect(() => {
    ui.filterInput = input ?? null;
    return () => (ui.filterInput = null);
  });
</script>

<label class="search" class:filled={ui.filterText}>
  <Icon name="search" size={15} class="text-fg-faint" />
  <input
    bind:this={input}
    bind:value={ui.filterText}
    type="search"
    placeholder="Search devices"
    aria-label="Search devices"
    onkeydown={(e) => {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        input?.blur();
        ui.moveFocus(0);
      }
    }}
  />
  {#if ui.filterText}
    <button type="button" class="clear" aria-label="Clear search" onclick={() => (ui.filterText = "")}><Icon name="x" size={14} /></button>
  {:else}
    <kbd>/</kbd>
  {/if}
</label>

<style>
  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 240px;
    height: 34px;
    padding: 0 8px 0 13px;
    border-radius: var(--r-pill);
    background: var(--glass);
    border: 1px solid var(--glass-border);
    transition:
      width var(--t-med) var(--ease-out),
      border-color var(--t-fast),
      box-shadow var(--t-fast),
      background var(--t-fast);
  }
  .search:hover {
    background: var(--glass-hover);
  }
  .search:focus-within,
  .search.filled {
    width: 300px;
    border-color: var(--accent-ring);
    box-shadow: 0 0 0 3px var(--accent-tint);
  }
  input {
    flex: 1;
    min-width: 0;
    background: transparent;
    border: none;
    outline: none;
    font: inherit;
    font-size: 13px;
    color: var(--fg);
  }
  input::placeholder {
    color: var(--fg-faint);
  }
  input::-webkit-search-cancel-button {
    display: none;
  }
  .clear {
    display: inline-flex;
    padding: 3px;
    border-radius: 50%;
    color: var(--fg-faint);
  }
  .clear:hover {
    color: var(--fg);
    background: var(--glass-hover);
  }
</style>
