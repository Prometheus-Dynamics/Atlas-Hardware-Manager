<script lang="ts">
  import { page } from "$app/state";
  import Icon from "#lib/components/common/Icon.svelte";
  import { insights } from "#lib/stores/insights.svelte.ts";
  import { jobs } from "#lib/stores/jobs.svelte.ts";
  import { robots } from "#lib/stores/robots.svelte.ts";
  import { system } from "#lib/stores/system.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import type { IconName } from "#lib/ui/icons.ts";
  import { pop } from "#lib/ui/motion.ts";

  const notReady = $derived(robots.statuses.filter((s) => s.state !== "ready").length);

  const items: { href: string; label: string; icon: IconName; key: string; badge: () => number; accent?: boolean }[] = [
    { href: "/", label: "Overview", icon: "heartbeat", key: "1", badge: () => 0 },
    { href: "/devices", label: "Devices", icon: "layout-grid", key: "2", badge: () => insights.needYou, accent: true },
    { href: "/robots", label: "Robots", icon: "robot", key: "3", badge: () => notReady },
    { href: "/jobs", label: "Jobs", icon: "activity", key: "4", badge: () => jobs.running.length, accent: true },
    { href: "/releases", label: "Releases", icon: "package", key: "5", badge: () => 0 },
    { href: "/networktables", label: "NetworkTables", icon: "sitemap", key: "6", badge: () => 0 },
    { href: "/settings", label: "Settings", icon: "settings", key: "7", badge: () => system.healthProblems },
  ];

  function active(href: string) {
    const path = page.url.pathname;
    return href === "/" ? path === "/" : path.startsWith(href);
  }
</script>

<nav class="rail" aria-label="Main">
  <img src="/logo.svg" alt="Atlas" class="logo" />
  <ul class="flex flex-col items-center gap-1.5">
    {#each items as item (item.href)}
      {@const count = item.badge()}
      <li>
        <a
          href={item.href}
          class="item"
          class:active={active(item.href)}
          aria-current={active(item.href) ? "page" : undefined}
          aria-label={item.label}
          data-tip="{item.label} · {item.key}"
        >
          <Icon name={item.icon} size={20} stroke={1.6} />
          {#if count > 0}
            <span class="badge" class:accent={item.accent} in:pop>{count}</span>
          {/if}
        </a>
      </li>
    {/each}
  </ul>

  <div class="mt-auto flex flex-col items-center gap-1.5">
    {#if system.info?.simulated}
      <span class="item quiet" data-tip="Simulated devices ({system.info.simulated})" aria-label="Simulated devices">
        <Icon name="flask" size={18} stroke={1.6} />
      </span>
    {/if}
    <button
      type="button"
      class="item"
      class:active={ui.helpOpen}
      onclick={() => (ui.helpOpen = !ui.helpOpen)}
      aria-label="Keyboard shortcuts"
      data-tip="Shortcuts · ?"
    >
      <Icon name="keyboard" size={18} stroke={1.6} />
    </button>
  </div>
</nav>

<style>
  .rail {
    position: relative;
    z-index: 40;
    display: flex;
    flex-direction: column;
    align-items: center;
    width: 52px;
    flex-shrink: 0;
    padding: 14px 0 12px;
    background: var(--rail);
    border-right: 1px solid var(--hairline);
  }
  .logo {
    width: 24px;
    height: 24px;
    margin-bottom: 18px;
  }
  .item {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    border-radius: 10px;
    color: var(--fg-faint);
    transition:
      background var(--t-fast),
      color var(--t-fast),
      transform var(--t-fast);
  }
  .item:hover {
    color: var(--fg);
    background: var(--glass);
  }
  .item:active {
    transform: scale(0.94);
  }
  .item.active {
    color: var(--accent-text-strong);
    background: var(--accent-tint-hover);
  }
  .item.quiet {
    color: var(--warn-fg);
    opacity: 0.8;
  }
  .badge {
    position: absolute;
    top: 1px;
    right: 0;
    min-width: 15px;
    height: 15px;
    padding: 0 4px;
    border-radius: var(--r-pill);
    font-size: 10px;
    font-weight: 600;
    line-height: 15px;
    text-align: center;
    background: var(--glass-border-strong);
    color: var(--fg);
    box-shadow: 0 0 0 2px var(--rail);
  }
  .badge.accent {
    background: var(--accent);
    color: var(--on-accent);
  }
  /* Tooltips to the right of the rail. */
  [data-tip]::after {
    content: attr(data-tip);
    position: absolute;
    left: calc(100% + 10px);
    top: 50%;
    padding: 5px 9px;
    border-radius: 8px;
    background: var(--layer-solid);
    border: 1px solid var(--glass-border);
    box-shadow: var(--shadow-lift);
    color: var(--fg);
    font-size: 12px;
    font-weight: 500;
    white-space: nowrap;
    pointer-events: none;
    opacity: 0;
    transform: translate(-4px, -50%);
    transition:
      opacity var(--t-fast),
      transform var(--t-fast) var(--ease-out);
  }
  [data-tip]:hover::after,
  [data-tip]:focus-visible::after {
    opacity: 1;
    transform: translate(0, -50%);
    transition-delay: 250ms;
  }
</style>
