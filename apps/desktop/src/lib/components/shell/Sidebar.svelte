<script lang="ts">
  // The app's sidebar: which robot, then sections of pages with
  // the robot's devices nested under Devices (each with its state and a live
  // reading), and at the foot how Atlas is finding devices and Settings.
  import { page } from "$app/state";
  import { keyString, type DeviceRecord } from "#lib/api/client.ts";
  import Icon from "#lib/components/common/Icon.svelte";
  import { deviceName } from "#lib/format.ts";
  import { metricTone } from "#lib/metrics.ts";
  import { isRecovery } from "#lib/present.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { insights } from "#lib/stores/insights.svelte.ts";
  import { jobs } from "#lib/stores/jobs.svelte.ts";
  import { live, SLOW_MS, watchLive } from "#lib/stores/live.svelte.ts";
  import { robots } from "#lib/stores/robots.svelte.ts";
  import { appUpdate } from "#lib/stores/appUpdate.svelte.ts";
  import { system } from "#lib/stores/system.svelte.ts";
  import { devicePath, NO_ROBOT, ui } from "#lib/stores/ui.svelte.ts";
  import type { IconName } from "#lib/ui/icons.ts";
  import DiscoveryStatus from "./DiscoveryStatus.svelte";

  type Item = { href: string; label: string; icon: IconName; key: string; badge?: () => number; tone?: "accent" | "warn" };
  const sections: { title: string; items: Item[] }[] = [
    {
      title: "Robot",
      items: [
        { href: "/", label: "Overview", icon: "heartbeat", key: "1" },
        { href: "/devices", label: "Devices", icon: "layout-grid", key: "2", badge: () => insights.needYou, tone: "accent" },
        { href: "/monitor", label: "Monitor", icon: "chart-line", key: "3" },
        { href: "/networktables", label: "NetworkTables", icon: "sitemap", key: "7" },
      ],
    },
    {
      title: "Maintain",
      items: [
        { href: "/jobs", label: "Updates and flashes", icon: "download", key: "5", badge: () => jobs.running.length, tone: "warn" },
        { href: "/releases", label: "Releases", icon: "package", key: "6" },
        { href: "/robots", label: "Robots", icon: "robot", key: "4", badge: () => robots.statuses.filter((s) => s.state !== "ready").length },
      ],
    },
  ];

  function active(href: string) {
    const path = page.url.pathname;
    if (href === "/devices") return path === "/devices";
    return href === "/" ? path === "/" : path === href || path.startsWith(`${href}/`);
  }

  // The robot's devices: what needs you first, then online, then offline.
  const mine = $derived(
    devices.all
      .filter((d) => ui.robot === null || (ui.robot === NO_ROBOT ? !d.robot : d.robot === ui.robot))
      .sort((a, b) => rank(a) - rank(b) || deviceName(a).localeCompare(deviceName(b), undefined, { numeric: true })),
  );
  const online = $derived(mine.filter((d) => d.presence === "online").length);
  $effect(() => watchLive(mine.filter((d) => d.presence === "online"), SLOW_MS));

  function rank(d: DeviceRecord): number {
    if (d.presence !== "online") return 3;
    if (isRecovery(d)) return 0;
    const temp = live.metric(keyString(d.key), "temp");
    return temp && metricTone(temp) !== "ok" && metricTone(temp) !== "neutral" ? 1 : 2;
  }

  /** The dot, and what is worth a glance: USB boot, offline, else its temperature. */
  function glance(d: DeviceRecord): { dot: string; note: string; tone: string } {
    if (d.presence !== "online") return { dot: "var(--offline)", note: "offline", tone: "var(--fg-faint)" };
    if (isRecovery(d)) return { dot: "var(--accent)", note: "USB boot", tone: "var(--accent-text)" };
    const temp = live.metric(keyString(d.key), "temp");
    if (!temp) return { dot: "var(--ok)", note: "", tone: "var(--fg-faint)" };
    const tone = metricTone(temp);
    const hot = tone === "warn" || tone === "err";
    return { dot: hot ? "var(--warn)" : "var(--ok)", note: `${Math.round(temp.value)}°`, tone: hot ? "var(--warn-fg)" : "var(--fg-faint)" };
  }

  const robotTitle = $derived(ui.robot === null ? "All devices" : ui.robot === NO_ROBOT ? "Not on a robot" : ui.robot);
  let robotMenu = $state(false);

</script>

<svelte:window onclick={() => (robotMenu = false)} />

<nav class="side" aria-label="Main">
  <div class="brand">
    <img src="/logo.svg" alt="" class="logo" />
    <span>Atlas</span>
    {#if system.info?.simulated}
      <span class="sim" title="Simulated devices ({system.info.simulated})"><Icon name="flask" size={13} /> sim</span>
    {/if}
  </div>

  <div class="relative px-2.5 pb-2">
    <button
      type="button"
      class="robot"
      aria-haspopup="listbox"
      aria-expanded={robotMenu}
      onclick={(e) => {
        e.stopPropagation();
        robotMenu = !robotMenu;
      }}
    >
      <span class="min-w-0 flex-1">
        <span class="block truncate font-semibold">{robotTitle}</span>
        <span class="flex items-center gap-1.5 text-[11.5px] text-fg-muted">
          <span class="dot" style:background={online > 0 ? "var(--ok)" : "var(--offline)"}></span>{online} of {mine.length} online
        </span>
      </span>
      <Icon name="chevron-down" size={14} class="text-fg-faint" />
    </button>
    {#if robotMenu}
      <ul class="menu" role="listbox" aria-label="Show devices for">
        {#each [{ value: null, label: "All devices" }, ...robots.names.map((n) => ({ value: n as string | null, label: n })), { value: NO_ROBOT, label: "Not on a robot" }] as option (option.value ?? "all")}
          <li>
            <button type="button" role="option" aria-selected={ui.robot === option.value} onclick={() => (ui.robot = option.value)}>
              <span class="flex-1 truncate text-left">{option.label}</span>
              {#if ui.robot === option.value}<Icon name="check" size={14} class="text-accent-text" />{/if}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>


  <div class="scroll">
    {#each sections as section (section.title)}
      <div>
        <div class="section">{section.title}</div>
        <ul class="flex flex-col gap-px">
          {#each section.items as item (item.href)}
            {@const count = item.badge?.() ?? 0}
            <li>
              <a href={item.href} class="item" class:active={active(item.href)} aria-current={active(item.href) ? "page" : undefined} title="{item.label} · {item.key}">
                <Icon name={item.icon} size={16} stroke={1.7} />
                <span class="flex-1 truncate">{item.label}</span>
                {#if count > 0}<span class="badge {item.tone ?? ''}">{count}</span>
                {:else if item.href === "/devices"}<span class="count">{mine.length}</span>{/if}
              </a>
              {#if item.href === "/devices" && mine.length > 0}
                <ul class="devices">
                  {#each mine as d (keyString(d.key))}
                    {@const id = keyString(d.key)}
                    {@const g = glance(d)}
                    <li>
                      <a
                        href={devicePath(id)}
                        class="device"
                        class:active={page.url.pathname === "/device" && ui.viewing === id}
                        aria-current={page.url.pathname === "/device" && ui.viewing === id ? "page" : undefined}
                      >
                        <span class="dot" style:background={g.dot}></span>
                        <span class="flex-1 truncate">{deviceName(d)}</span>
                        <span class="note" style:color={g.tone}>{g.note}</span>
                      </a>
                    </li>
                  {/each}
                </ul>
              {/if}
            </li>
          {/each}
        </ul>
      </div>
    {/each}
  </div>

  <div class="foot">
    {#if appUpdate.available && appUpdate.status?.phase === "idle"}
      <a href="/settings" class="item update" title="Atlas {appUpdate.available.version} is out: install it from Settings">
        <Icon name="arrow-up" size={16} stroke={1.9} />
        <span class="flex-1 truncate">Atlas {appUpdate.available.version}</span>
        <span class="badge accent">Update</span>
      </a>
    {:else if appUpdate.busy && appUpdate.status?.phase !== "checking"}
      <a href="/settings" class="item update" title="Atlas is updating">
        <Icon name="loader-2" size={16} class="spin" />
        <span class="flex-1 truncate">Updating Atlas…</span>
      </a>
    {/if}
    <DiscoveryStatus />
    <div class="flex gap-1">
      <a href="/settings" class="item flex-1" class:active={active("/settings")} aria-current={active("/settings") ? "page" : undefined} title="Settings · 8">
        <Icon name="settings" size={16} stroke={1.7} />
        <span class="flex-1">Settings</span>
        {#if system.healthProblems > 0}<span class="badge warn">{system.healthProblems}</span>{/if}
      </a>
      <button type="button" class="item square" class:active={ui.helpOpen} onclick={() => (ui.helpOpen = !ui.helpOpen)} aria-label="Keyboard shortcuts" title="Shortcuts · ?">
        <Icon name="keyboard" size={16} stroke={1.7} />
      </button>
    </div>
  </div>
</nav>

<style>
  .item.update {
    color: var(--accent-text);
  }
  .side {
    position: relative;
    z-index: 40;
    display: flex;
    flex-direction: column;
    width: 232px;
    flex-shrink: 0;
    background: var(--rail);
    border-right: 1px solid var(--hairline);
    font-size: 13px;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 14px 14px 10px;
    font-weight: 600;
    font-size: 14px;
  }
  .logo {
    width: 22px;
    height: 22px;
  }
  .sim {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 3px;
    padding: 1px 6px;
    border-radius: var(--r-sm);
    background: var(--warn-bg);
    color: var(--warn-fg);
    font-size: 11px;
    font-weight: 500;
  }
  .robot {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 10px;
    border: 1px solid var(--glass-border);
    border-radius: var(--r-card);
    background: var(--glass);
    text-align: left;
    transition: background var(--t-fast);
  }
  .robot:hover {
    background: var(--glass-hover);
  }
  .menu {
    position: absolute;
    left: 10px;
    right: 10px;
    top: calc(100% - 4px);
    z-index: 50;
    padding: 4px;
    border-radius: var(--r-card);
    border: 1px solid var(--glass-border-strong);
    background: var(--layer-solid);
    box-shadow: var(--shadow-layer);
  }
  .menu button {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding: 0 8px;
    border-radius: var(--r-sm);
  }
  .menu button:hover {
    background: var(--glass-hover);
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 10px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .section {
    padding: 0 8px 4px;
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.09em;
    text-transform: uppercase;
    color: var(--fg-faint);
  }
  .item {
    display: flex;
    align-items: center;
    gap: 9px;
    height: 31px;
    padding: 0 8px;
    border-radius: var(--r-sm);
    color: #b9c0c9;
    transition:
      background var(--t-fast),
      color var(--t-fast);
  }
  .item :global(svg) {
    color: var(--fg-faint);
  }
  .item:hover {
    background: var(--glass-hover);
    color: var(--fg);
  }
  .item.active {
    background: var(--glass-strong);
    color: var(--fg);
  }
  .item.active :global(svg) {
    color: var(--accent);
  }
  .item.square {
    width: 31px;
    justify-content: center;
    padding: 0;
  }
  .badge {
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    box-sizing: border-box;
    border-radius: var(--r-sm);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 11px;
    font-weight: 600;
    background: var(--neutral-bg);
    color: var(--fg);
  }
  .badge.accent {
    background: var(--err-bg);
    color: var(--accent-text);
  }
  .badge.warn {
    background: var(--warn-bg);
    color: var(--warn-fg);
  }
  .count {
    font-size: 11.5px;
    color: var(--fg-faint);
    font-variant-numeric: tabular-nums;
  }
  .devices {
    margin: 2px 0 4px 16px;
    padding-left: 9px;
    border-left: 1px solid var(--hairline);
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .device {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 27px;
    padding: 0 8px;
    border-radius: var(--r-sm);
    font-size: 12.5px;
    color: #b9c0c9;
  }
  .device:hover {
    background: var(--glass-hover);
    color: var(--fg);
  }
  .device.active {
    background: var(--glass-strong);
    color: var(--fg);
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .note {
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .foot :global(.status) {
    width: 100%;
  }
  .foot :global(.rescan) {
    flex: 1;
  }
  .foot {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px;
    border-top: 1px solid var(--hairline);
  }
</style>
