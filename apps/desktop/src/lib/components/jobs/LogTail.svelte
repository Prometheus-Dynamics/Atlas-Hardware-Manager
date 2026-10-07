<script lang="ts">
  // The last few log lines, expandable to the full log, with copy. With
  // `fill` it takes the height its parent leaves and shows the whole log.
  import Button from "#lib/components/common/Button.svelte";
  import { toasts } from "#lib/stores/toasts.svelte.ts";

  let { lines, tail = 6, fill = false }: { lines: string[]; tail?: number; fill?: boolean } = $props();

  // svelte-ignore state_referenced_locally
  let expanded = $state(fill);
  let box: HTMLPreElement | undefined = $state();
  const shown = $derived(expanded ? lines : lines.slice(-tail));

  // Follow new lines while expanded.
  $effect(() => {
    void shown.length;
    if (box) box.scrollTop = box.scrollHeight;
  });

  async function copy() {
    try {
      await navigator.clipboard.writeText(lines.join("\n"));
      toasts.success("Log copied.");
    } catch {
      toasts.error("Couldn't copy to the clipboard.");
    }
  }
</script>

<div class="log" class:fill>
  <div class="flex items-center justify-between gap-2 px-4 pt-2.5">
    <span class="text-[12px] font-medium text-fg-faint">Log · {lines.length} line{lines.length === 1 ? "" : "s"}</span>
    <div class="flex gap-1">
      {#if lines.length > tail && !fill}
        <Button variant="ghost" size="sm" icon={expanded ? "chevron-up" : "chevron-down"} onclick={() => (expanded = !expanded)}>
          {expanded ? "Show less" : "Show all"}
        </Button>
      {/if}
      <Button variant="ghost" size="sm" icon="copy" disabled={lines.length === 0} onclick={copy}>Copy</Button>
    </div>
  </div>
  <pre bind:this={box} class:expanded>{shown.join("\n") || "Nothing logged yet."}</pre>
</div>

<style>
  .log {
    border-radius: var(--r-card);
    background: var(--inset);
    border: 1px solid var(--hairline);
  }
  pre {
    margin: 0;
    padding: 6px 16px 12px;
    max-height: 11em;
    overflow: auto;
    font-family: var(--font-code);
    font-size: 11.5px;
    line-height: 1.55;
    color: var(--fg-muted);
    white-space: pre-wrap;
    word-break: break-all;
    transition: max-height var(--t-med) var(--ease-out);
  }
  pre.expanded {
    max-height: 320px;
  }
  .log.fill {
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    min-height: 13em;
  }
  .fill pre {
    flex: 1 1 0;
    min-height: 8em;
    max-height: none;
  }
</style>
