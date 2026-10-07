<script lang="ts">
  import { flip } from "svelte/animate";
  import GlassCard from "#lib/components/common/GlassCard.svelte";
  import { releases } from "#lib/stores/releases.svelte.ts";
  import { DUR, ease, ms, rise, softFade } from "#lib/ui/motion.ts";
  import ReleaseRow from "./ReleaseRow.svelte";

  let { family }: { family: string } = $props();

  const entries = $derived(releases.forFamily(family));
</script>

<!-- Fills its grid cell; a long list scrolls inside the card. -->
<GlassCard title={family} subtitle="{entries.length} release{entries.length === 1 ? '' : 's'}" icon="package" large pad={false} fill class="h-full">
  <ul class="@container flex flex-col px-2 py-2">
    {#each entries as entry (entry.id)}
      <li animate:flip={{ duration: ms(DUR.enter), easing: ease }} in:rise out:softFade><ReleaseRow {entry} /></li>
    {/each}
  </ul>
</GlassCard>
