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
  const reached = $derived.by(() => {
    const how = linkText(record.link_kind, devices.nameOf);
    return how.startsWith("via ") ? `reached ${how}` : `reached by ${how}`;
  });
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
    // The one in use first, then IPv4, then IPv6, link-local last.
    const rank = (ip: string) => (ip === current ? 0 : !ip.includes(":") ? 1 : ip.toLowerCase().startsWith("fe80") ? 3 : 2);
    return all.sort((a, b) => rank(a) - rank(b)).map((ip) => ({ ip, via: network(ip), inUse: ip === current }));
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

<section class="panel" aria-label="Network">
  <div class="panel-head">
    <h2 class="panel-title">Network</h2>
    <span class="panel-note">{reached}</span>
  </div>
  <dl>
    {#each list as a (a.ip)}
      <div class="row">
        <dt>{a.via}</dt>
        <dd class="ip" class:use={a.inUse} class:v6={a.ip.includes(":")}>{a.ip}</dd>
        {#if a.inUse}<span class="tag">in use</span>{/if}
        <button type="button" class="copy" aria-label="Copy {a.ip}" title="Copy" onclick={() => copy(a.ip)}><Icon name="copy" size={13} /></button>
      </div>
    {/each}
    {#if attrs.hostname}
      <div class="row">
        <dt>hostname</dt>
        <dd class="ip use">{attrs.hostname}</dd>
        <button type="button" class="copy" aria-label="Copy {attrs.hostname}" title="Copy" onclick={() => copy(attrs.hostname ?? "")}><Icon name="copy" size={13} /></button>
      </div>
    {/if}
  </dl>
</section>

<style>
  dl {
    display: flex;
    flex-direction: column;
  }
  .row {
    display: grid;
    grid-template-columns: 88px minmax(0, 1fr) auto 24px;
    align-items: center;
    column-gap: 8px;
    min-height: 26px;
    border-top: 1px solid var(--hairline);
  }
  .row:first-child {
    border-top: 0;
  }
  dt {
    font-size: 11.5px;
    color: var(--fg-muted);
  }
  .ip {
    grid-column: 2;
    min-width: 0;
    font-family: var(--font-code);
    font-size: 12px;
    color: var(--fg-muted);
    overflow-wrap: anywhere;
  }
  /* IPv6 addresses are long: smaller, and wrapped rather than cut. */
  .ip.v6 {
    font-size: 11px;
    line-height: 1.35;
    padding: 3px 0;
  }
  .ip.use {
    color: var(--fg);
  }
  .tag {
    grid-column: 3;
    white-space: nowrap;
    font-size: 11px;
    padding: 0 6px;
    border-radius: var(--r-sm);
    background: var(--ok-bg);
    color: var(--ok-fg);
  }
  .copy {
    grid-column: 4;
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
