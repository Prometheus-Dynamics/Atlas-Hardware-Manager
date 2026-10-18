<script lang="ts">
  // A kernel log the board kept (pstore), fetched when opened: what the
  // kernel printed until the boot before `boot` ended, panic records first.
  import { api, errorText, type DeviceKey } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { portal } from "#lib/ui/portal.ts";

  let { deviceKey, boot, title, onclose }: { deviceKey: DeviceKey; boot: string; title: string; onclose: () => void } = $props();

  let text = $state<string | null>(null);
  let error = $state<string | null>(null);
  $effect(() => {
    let stale = false;
    text = null;
    error = null;
    api
      .deviceKernelRecord(deviceKey, boot)
      .then((t) => !stale && (text = t))
      .catch((e) => !stale && (error = errorText(e)));
    return () => (stale = true);
  });

  async function copy() {
    if (text === null) return;
    await navigator.clipboard.writeText(text);
    toasts.success("Kernel log copied.");
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onclose()} />

<div class="modal" use:portal>
  <button type="button" class="backdrop" aria-label="Close" onclick={onclose}></button>
  <div class="dialog panel" role="dialog" aria-modal="true" aria-label={title}>
    <div class="panel-head">
      <h2 class="panel-title">{title}</h2>
      <span class="panel-note mono">{boot.slice(0, 8)}</span>
      <Button size="sm" variant="ghost" icon="copy" label="Copy" disabled={text === null} action={copy} />
      <Button size="sm" variant="ghost" icon="x" label="Close (Esc)" onclick={onclose} />
    </div>
    {#if error}
      <p class="text-[12.5px] text-warn-fg">{error}</p>
    {:else if text === null}
      <p class="text-[12.5px] text-fg-muted">Reading the kernel log from the board…</p>
    {:else}
      <pre class="log">{#each text.split("\n") as line, i (i)}<span class:head={line.startsWith("== ")} class:bad={/panic|LOCKUP|Oops|BUG:|Call trace/i.test(line)}>{line}</span>
{/each}</pre>
    {/if}
  </div>
</div>

<style>
  .modal {
    position: fixed;
    inset: 0;
    z-index: 95;
    display: grid;
    place-items: center;
  }
  .backdrop {
    position: absolute;
    inset: 0;
    background: rgba(5, 6, 8, 0.65);
    cursor: default;
  }
  .dialog {
    position: relative;
    width: min(860px, calc(100vw - 32px));
    max-height: calc(100vh - 64px);
  }
  .log {
    margin: 0;
    min-height: 0;
    overflow: auto;
    padding: 8px 10px;
    font-family: var(--font-code);
    font-size: 11.5px;
    line-height: 1.45;
    white-space: pre;
    background: var(--inset);
    border: 1px solid var(--glass-border);
    border-radius: var(--r-sm);
    color: var(--fg-muted);
  }
  .head {
    color: var(--fg);
    font-weight: 600;
  }
  .bad {
    color: var(--err-fg);
  }
</style>
