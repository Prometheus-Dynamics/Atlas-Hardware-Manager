<script lang="ts">
  import { flip } from "svelte/animate";
  import GlassCard from "$lib/components/common/GlassCard.svelte";
  import { releases } from "$lib/stores/releases.svelte";
  import { ms, rise, softFade } from "$lib/ui/motion";
  import ReleaseRow from "./ReleaseRow.svelte";

  let { family }: { family: string } = $props();

  const entries = $derived(releases.forFamily(family));
</script>

<GlassCard title={family} subtitle="{entries.length} release{entries.length === 1 ? '' : 's'}" icon="package" large pad={false}>
  <ul class="flex flex-col px-2 pb-2">
    {#each entries as entry (entry.id)}
      <li animate:flip={{ duration: ms(220) }} in:rise out:softFade><ReleaseRow {entry} /></li>
    {/each}
  </ul>
</GlassCard>
