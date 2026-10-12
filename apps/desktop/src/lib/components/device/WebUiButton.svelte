<script lang="ts">
  // The device's web page: opens it by IP, which works without mDNS; the
  // chevron lists the other ways in (the board's other IPs, its .local
  // name) and copies an address.
  import { openExternal, type DeviceRecord } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { popover } from "#lib/ui/motion.ts";
  import { webUiTargets } from "#lib/webui.ts";

  let { record, size = "sm", label = "Web UI" }: { record: DeviceRecord; size?: "sm" | "md"; label?: string } =
    $props();

  const targets = $derived(webUiTargets(record));
  const primary = $derived(targets[0] ?? null);

  let open = $state(false);
  let root: HTMLElement | undefined = $state();

  function onwindowclick(event: MouseEvent) {
    if (open && root && !root.contains(event.target as Node)) open = false;
  }

  function go(url: string) {
    open = false;
    void openExternal(url);
  }

  async function copy(url: string) {
    open = false;
    try {
      await navigator.clipboard.writeText(url);
      toasts.success(`Copied ${url}`);
    } catch {
      toasts.error("Couldn't copy to the clipboard.");
    }
  }
</script>

<svelte:window onclick={onwindowclick} onkeydown={(e) => e.key === "Escape" && (open = false)} />

{#if primary}
  <div class="relative inline-flex" bind:this={root}>
    <Button {size} icon="world-www" iconRight="external-link" onclick={() => go(primary.url)} title={primary.url} class="joined-left">
      {label}
    </Button>
    <Button {size} icon="chevron-down" label="Other ways to open the web UI" onclick={() => (open = !open)} class="joined-right" />
    {#if open}
      <ul class="menu glass-layer" role="menu" aria-label="Open the web UI" transition:popover>
        {#each targets as target (target.url)}
          <li class="row">
            <button type="button" role="menuitem" class="open" onclick={() => go(target.url)} title={target.url}>
              <Icon name={target.kind === "ip" ? "router" : "world-www"} size={16} />
              <span class="flex-1 truncate text-left">
                <span class="mono">{target.host}</span>
                {#if target.kind === "name"}<span class="hint">needs mDNS</span>{/if}
              </span>
              <Icon name="external-link" size={14} class="text-fg-faint" />
            </button>
            <button type="button" class="copy" title="Copy {target.url}" aria-label="Copy {target.url}" onclick={() => copy(target.url)}>
              <Icon name="copy" size={14} />
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
{/if}

<style>
  .relative :global(.joined-left) {
    border-top-right-radius: 0;
    border-bottom-right-radius: 0;
  }
  .relative :global(.joined-right) {
    margin-left: 1px;
    border-top-left-radius: 0;
    border-bottom-left-radius: 0;
  }
  .menu {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 35;
    min-width: 260px;
    padding: 6px;
    border-radius: var(--r-panel);
    transform-origin: top right;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .row button {
    display: flex;
    align-items: center;
    border-radius: var(--r-md);
    color: var(--fg);
    font-size: 13px;
  }
  .row button:hover {
    background: var(--glass-hover);
  }
  .open {
    flex: 1;
    gap: 10px;
    min-width: 0;
    padding: 8px 10px;
  }
  .copy {
    padding: 8px;
    color: var(--fg-muted);
  }
  .hint {
    margin-left: 6px;
    font-size: 11px;
    color: var(--fg-faint);
  }
</style>
