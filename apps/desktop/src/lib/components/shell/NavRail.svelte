<script lang="ts">
  import { page } from "$app/state";
  import { jobs } from "$lib/stores/jobs.svelte";
  import { robots } from "$lib/stores/robots.svelte";
  import { system } from "$lib/stores/system.svelte";

  const notReady = $derived(robots.statuses.filter((s) => s.state !== "ready").length);

  const items = $derived([
    { href: "/", label: "Inventory", icon: "fa-microchip", key: "1", badge: 0, tone: "" },
    { href: "/robots", label: "Robots", icon: "fa-robot", key: "2", badge: notReady, tone: "bg-warning-500 text-surface-950" },
    { href: "/jobs", label: "Jobs", icon: "fa-list-check", key: "3", badge: jobs.running.length, tone: "bg-secondary-500 text-white" },
    { href: "/releases", label: "Releases", icon: "fa-box-archive", key: "4", badge: 0, tone: "" },
    { href: "/settings", label: "Settings", icon: "fa-gear", key: "5", badge: system.healthProblems, tone: "bg-error-500 text-white" },
  ]);

  function active(href: string) {
    const path = page.url.pathname;
    return href === "/" ? path === "/" : path.startsWith(href);
  }
</script>

<nav
  class="flex w-[4.6rem] shrink-0 flex-col items-stretch border-r border-surface-800 bg-surface-900/80 py-3"
  aria-label="Main"
>
  <div class="mb-4 flex justify-center">
    <img src="/logo.svg" alt="Atlas" class="h-7 w-7" />
  </div>
  <ul class="flex flex-col gap-1 px-1.5">
    {#each items as item (item.href)}
      <li>
        <a
          href={item.href}
          class="relative flex flex-col items-center gap-1 rounded-base px-1 py-2 text-[0.58rem] uppercase tracking-[0.14em] transition-colors
            {active(item.href)
            ? 'bg-primary-500/20 text-surface-50 ring-1 ring-primary-500/60'
            : 'text-surface-400 hover:bg-surface-800 hover:text-surface-100'}"
          aria-current={active(item.href) ? "page" : undefined}
          title="{item.label} ({item.key})"
        >
          <i class="fa-solid {item.icon} text-base" aria-hidden="true"></i>
          {item.label}
          {#if item.badge > 0}
            <span class="absolute right-1.5 top-1 min-w-4 rounded-full px-1 text-center text-[0.55rem] leading-4 tracking-normal {item.tone}">
              {item.badge}
            </span>
          {/if}
        </a>
      </li>
    {/each}
  </ul>
</nav>
