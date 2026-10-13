<script lang="ts">
  // How the device is reached: every address Atlas has seen it on lately,
  // each named by its network and copyable, the one in use marked, and its
  // hostname. The USB route's switch sits below it (UsbRouteCard).
  import type { DeviceRecord } from "#lib/api/client.ts";
  import Icon from "#lib/components/common/Icon.svelte";
  import { linkText } from "#lib/format.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";

  let { record }: { record: DeviceRecord } = $props();

  const attrs = $derived(record.identity.attributes ?? {});
  /** The host of an address (a URL, `[v6]:port`, or a bare IP). */
  function host(address: string): string {
    try {
      return new URL(address.includes("://") ? address : `http://${address}`).hostname.replace(/^\[|\]$/g, "");
    } catch {
      return address;
    }
  }
  const current = $derived(host(record.identity.address));
  const list = $derived.by(() => {
    const all = (attrs.addresses ?? "").split(",").map((a) => a.trim()).filter(Boolean);
    if (current && !all.includes(current)) all.unshift(current);
    return all.map((ip) => ({ ip, via: network(ip), inUse: ip === current }));
  });

  function network(ip: string): string {
    if (ip.startsWith("172.31.")) return "USB";
    if (ip.startsWith("169.254.")) return "link-local";
    if (ip.includes(":")) return ip.toLowerCase().startsWith("fe80") ? "IPv6 link" : "IPv6";
    if (ip.startsWith("10.")) return "robot network";
    return "network";
  }

  async function copy(ip: string) {
    try {
      await navigator.clipboard.writeText(ip);
      toasts.success(`Copied ${ip}.`);
    } catch {
      toasts.error("Couldn't copy it.");
    }
  }
</script>

<section class="card" aria-label="Network">
  <div class="head">
    <h2>Network</h2>
    <span class="sub">reached by {linkText(record.link_kind, devices.nameOf)}</span>
  </div>
  {#if list.length > 0}
    <ul>
      {#each list as a (a.ip)}
        <li>
          <span class="via">{a.via}</span>
          <span class="ip" class:use={a.inUse}>{a.ip}</span>
          {#if a.inUse}<span class="tag">in use</span>{/if}
          <button type="button" class="copy" aria-label="Copy {a.ip}" title="Copy" onclick={() => copy(a.ip)}><Icon name="copy" size={13} /></button>
        </li>
      {/each}
    </ul>
  {/if}
  {#if attrs.hostname}
    <div class="host"><span class="via">hostname</span><span class="ip">{attrs.hostname}</span></div>
  {/if}
</section>

<style>
  .card {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px 14px;
    border-radius: var(--r-card);
    background: var(--glass);
    border: 1px solid var(--glass-border);
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  h2 {
    font-size: 13px;
    font-weight: 600;
  }
  .sub {
    font-size: 11.5px;
    color: var(--fg-muted);
  }
  ul {
    display: flex;
    flex-direction: column;
  }
  li,
  .host {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 26px;
    font-size: 12.5px;
  }
  .via {
    width: 92px;
    flex-shrink: 0;
    font-size: 11.5px;
    color: var(--fg-muted);
  }
  .ip {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-code);
    font-size: 12px;
  }
  .ip.use {
    color: var(--fg);
  }
  .tag {
    font-size: 11px;
    padding: 0 6px;
    border-radius: var(--r-sm);
    background: var(--ok-bg);
    color: var(--ok-fg);
  }
  .copy {
    margin-left: auto;
    width: 24px;
    height: 24px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--r-sm);
    color: var(--fg-faint);
  }
  .copy:hover {
    color: var(--fg);
    background: var(--glass-hover);
  }
</style>
