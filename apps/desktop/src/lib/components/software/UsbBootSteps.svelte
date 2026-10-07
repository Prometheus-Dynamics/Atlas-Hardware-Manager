<script lang="ts">
  // The manual steps into USB boot, numbered.
  import { rise, stagger } from "#lib/ui/motion.ts";

  let { steps }: { steps: string[] } = $props();
</script>

<ol class="flex flex-col gap-1.5">
  {#each steps as step, i (i)}
    <li class="flex gap-2.5" in:rise={{ delay: stagger(i) }}>
      <span class="num">{i + 1}</span>
      <span class="text-[12.5px] leading-relaxed text-fg">{step}</span>
    </li>
  {/each}
</ol>

<style>
  .num {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    margin-top: 1px;
    flex-shrink: 0;
    border-radius: 50%;
    background: var(--glass-strong);
    color: var(--fg-muted);
    font-size: 11.5px;
    font-weight: 600;
  }
</style>
