<script lang="ts">
  // Label/value facts: glass tiles, or a plain two-column list.
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
  <dl class="grid grid-cols-2 gap-2">
    {#each facts as fact (fact.label)}
      <div class="glass min-w-0 px-3.5 py-2.5">
        <dt class="text-[12px] text-fg-faint">{fact.label}</dt>
        <dd class="mt-0.5 truncate text-[13.5px] font-medium text-fg" class:mono={fact.mono} title={fact.value ?? undefined}>
          {fact.value}
        </dd>
      </div>
    {/each}
  </dl>
{/if}
