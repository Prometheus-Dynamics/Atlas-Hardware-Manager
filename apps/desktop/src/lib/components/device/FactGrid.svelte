<script lang="ts">
  // Label/value facts: one panel of compact rows (two or three across as
  // there is room), or a plain two-column list.
  let {
    facts,
    plain = false,
  }: { facts: { label: string; value: string | null; mono?: boolean }[]; plain?: boolean } = $props();
</script>

{#if plain}
  <dl class="grid grid-cols-[8.5rem_1fr] gap-x-3 gap-y-2 text-[13px]">
    {#each facts as fact (fact.label)}
      <dt class="text-fg-faint">{fact.label[0].toUpperCase() + fact.label.slice(1)}</dt>
      <dd class="break-all text-fg" class:mono={fact.mono}>{fact.value ?? "—"}</dd>
    {/each}
  </dl>
{:else}
  <dl class="facts">
    {#each facts as fact (fact.label)}
      <div class="fact">
        <dt>{fact.label}</dt>
        <dd class:mono={fact.mono} title={fact.value ?? undefined}>{fact.value}</dd>
      </div>
    {/each}
  </dl>
{/if}

<style>
  .facts {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    background: var(--glass);
    border: 1px solid var(--glass-border);
    border-radius: var(--r-card);
    overflow: hidden;
  }
  .fact {
    display: flex;
    align-items: baseline;
    gap: 10px;
    min-width: 0;
    padding: 6px 12px;
    border-bottom: 1px solid var(--hairline);
    font-size: 12.5px;
  }
  dt {
    flex-shrink: 0;
    width: 6.5rem;
    color: var(--fg-faint);
  }
  dd {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
    color: var(--fg);
  }
</style>
