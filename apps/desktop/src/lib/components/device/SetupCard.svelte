<script lang="ts">
  // The board's setup in brief: team (from its robot address, 10.TE.AM.x),
  // addresses to copy, hostname, the USB route and the security switch.
  // Changing them is in Settings.
  import type { DeviceRecord } from "#lib/api/client.ts";
  import Icon from "#lib/components/common/Icon.svelte";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { usbRoute } from "#lib/present.ts";

  let { record, onsettings }: { record: DeviceRecord; onsettings?: () => void } = $props();

  const attrs = $derived(record.identity.attributes ?? {});
  const host = (a: string) => a.replace(/^\w+:\/\//, "").replace(/[/:].*$/, "");
  const addresses = $derived.by(() => {
    const all = (attrs.addresses ?? "").split(",").map((a) => a.trim()).filter(Boolean);
    const current = host(record.identity.address ?? "");
    if (current && !all.includes(current)) all.unshift(current);
    return all;
  });
  const robotIp = $derived(addresses.find((a) => a.startsWith("10.")) ?? null);
  const usbIp = $derived(addresses.find((a) => a.startsWith("172.31.")) ?? null);
  const team = $derived.by(() => {
    if (attrs.team) return attrs.team;
    const m = robotIp ? /^10\.(\d{1,2})\.(\d{1,2})\./.exec(robotIp) : null;
    return m ? String(Number(m[1]) * 100 + Number(m[2])) : null;
  });
  const route = $derived(usbRoute(record));
  const security = $derived(attrs.security === "on" ? "on: enrolled computers only" : attrs.security === "off" ? "off (FRC default)" : null);

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      toasts.success(`Copied ${text}`);
    } catch {
      toasts.info(text);
    }
  }
</script>

<section class="panel setup" aria-label="Setup">
  <div class="panel-head">
    <h2 class="panel-title">Setup</h2>
    <button type="button" class="link truncate text-[11.5px]" onclick={() => onsettings?.()}>
      Change in Settings
    </button>
  </div>
  <div class="kv"><span>Team</span><span class="mono truncate" title={team ?? "unknown"}>{team ?? "—"}</span><span></span></div>
  {#if robotIp}
    <div class="kv"><span>Robot IP</span><span class="mono truncate" title={robotIp}>{robotIp}</span><button type="button" class="copy" aria-label="Copy {robotIp}" title="Copy" onclick={() => void copy(robotIp)}><Icon name="copy" size={13} /></button></div>
  {/if}
  {#if usbIp}
    <div class="kv"><span>USB IP</span><span class="mono truncate" title={usbIp}>{usbIp}</span><button type="button" class="copy" aria-label="Copy {usbIp}" title="Copy" onclick={() => void copy(usbIp)}><Icon name="copy" size={13} /></button></div>
  {/if}
  {#if attrs.hostname}
    <div class="kv"><span>Hostname</span><span class="mono truncate" title={attrs.hostname}>{attrs.hostname}</span><button type="button" class="copy" aria-label="Copy {attrs.hostname}" title="Copy" onclick={() => void copy(`${attrs.hostname}.local`)}><Icon name="copy" size={13} /></button></div>
  {/if}
  {#if route}
    {@const routeText = route.state === "active" ? "On, sharing" : route.state === "on" ? "On, idle" : "Off"}
    <div class="kv"><span title="The robot network, reached from this computer over USB through this board">Robot net</span><span class="truncate" title={route.state === "active" ? "On: this computer reaches the robot network through the board" : route.state === "on" ? "On, not routing now" : "Off"}>{routeText}</span><span></span></div>
  {/if}
  {#if security}
    <div class="kv"><span>Security</span><span class="truncate" title={security}>{security}</span><span></span></div>
  {/if}
</section>

<style>
  .setup {
    gap: 0;
  }
  .kv {
    display: grid;
    grid-template-columns: 82px minmax(0, 1fr) 20px;
    align-items: center;
    gap: 8px;
    height: 25px;
    border-bottom: 1px solid var(--hairline);
    font-size: 12.5px;
  }
  .kv:last-child {
    border-bottom: none;
  }
  .kv > span:first-child {
    color: var(--fg-faint);
  }
  .copy {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border-radius: var(--r-sm);
    color: var(--fg-faint);
  }
  .copy:hover {
    color: var(--fg);
    background: var(--glass-hover);
  }
</style>
