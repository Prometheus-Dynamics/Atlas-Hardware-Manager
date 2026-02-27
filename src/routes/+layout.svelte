<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { page } from "$app/stores";
  import { pageHeader } from "$lib/stores/pageHeader";
  import "../app.css";

  let isSidebarCollapsed = false;
  let statusPollHandle: ReturnType<typeof setInterval> | null = null;
  let statusRequestInFlight = false;
  const sidebarStorageKey = "atlas-sidebar-collapsed";
  const setupGateStorageKey = "atlas-host-setup-gate-dismissed-v1";

  interface HostSetupCheck {
    id: string;
    label: string;
    required: boolean;
    ready: boolean;
    detail: string;
    detected?: string | null;
    fixHint?: string | null;
  }

  interface HostSetupStatus {
    platform: string;
    arch: string;
    ready: boolean;
    generatedAtEpochMs: number;
    checks: HostSetupCheck[];
  }

  interface HostSetupRepairResult {
    status: HostSetupStatus;
    attemptedActions: string[];
    appliedActions: string[];
    warnings: string[];
  }

  interface OperationResult {
    success: boolean;
    exitCode?: number | null;
    durationMs: number;
    stdout: string;
    stderr: string;
    message: string;
    timedOut: boolean;
  }

  interface ConnectionStatusSnapshot {
    connected: boolean;
    route: string;
    label: string;
    detail: string;
    targetIp?: string | null;
    interfaceName?: string | null;
    generatedAtEpochMs: number;
  }

  const defaultConnectionStatus: ConnectionStatusSnapshot = {
    connected: false,
    route: "unknown",
    label: "Checking connection...",
    detail: "Waiting for connection probe.",
    targetIp: null,
    interfaceName: null,
    generatedAtEpochMs: 0,
  };

  let connectionStatus: ConnectionStatusSnapshot = defaultConnectionStatus;
  let setupGateVisible = false;
  let setupGateDismissed = false;
  let setupGateBusy = false;
  let setupGateRepairBusy = false;
  let setupGateError: string | null = null;
  let setupGateMessage: string | null = null;
  let setupGateStatus: HostSetupStatus | null = null;
  let setupGateWarnings: string[] = [];
  let setupGateAppliedActions: string[] = [];

  const navItems = [
    { label: "Network Devices", description: "USB/IP discovery", href: "/network", iconClass: "fa-solid fa-network-wired" },
    { label: "Settings", description: "Host tool readiness", href: "/settings", iconClass: "fa-solid fa-gear" },
  ];

  const bodyClasses = ["bg-surface-950", "text-surface-50", "font-geologica", "antialiased"];

  const toggleSidebar = () => {
    isSidebarCollapsed = !isSidebarCollapsed;
    try {
      localStorage.setItem(sidebarStorageKey, isSidebarCollapsed ? "1" : "0");
    } catch {
      // no-op when storage is unavailable
    }
  };

  const normalize = (path: string) =>
    path !== "/" && path.endsWith("/") ? path.slice(0, -1) : path;

  const isActive = (currentPath: string, href: string) => {
    const normalizedHref = normalize(href);
    const normalizedPath = normalize(currentPath);
    if (normalizedHref === "/") {
      return normalizedPath === "/";
    }
    return (
      normalizedPath === normalizedHref || normalizedPath.startsWith(`${normalizedHref}/`)
    );
  };

  async function refreshConnectionStatus(): Promise<void> {
    if (statusRequestInFlight) {
      return;
    }
    statusRequestInFlight = true;

    try {
      connectionStatus = await invoke<ConnectionStatusSnapshot>("get_connection_status");
    } catch (error) {
      connectionStatus = {
        connected: false,
        route: "unavailable",
        label: "Connection probe unavailable",
        detail: error instanceof Error ? error.message : "Connection probe failed.",
        targetIp: null,
        interfaceName: null,
        generatedAtEpochMs: Date.now(),
      };
    } finally {
      statusRequestInFlight = false;
    }
  }

  function statusPingClass(status: ConnectionStatusSnapshot): string {
    if (!status.connected) {
      return "bg-error-400/60";
    }
    if (status.route === "usb-network" || status.route === "helios-usb") {
      return "bg-warning-400/60";
    }
    if (status.route === "driver-station" || status.route === "rio-network") {
      return "bg-secondary-400/60";
    }
    if (status.route.startsWith("helios")) {
      return "bg-primary-400/60";
    }
    return "bg-success-400/60";
  }

  function statusDotClass(status: ConnectionStatusSnapshot): string {
    if (!status.connected) {
      return "bg-error-400";
    }
    if (status.route === "usb-network" || status.route === "helios-usb") {
      return "bg-warning-400";
    }
    if (status.route === "driver-station" || status.route === "rio-network") {
      return "bg-secondary-400";
    }
    if (status.route.startsWith("helios")) {
      return "bg-primary-400";
    }
    return "bg-success-400";
  }

  function requiresHostSetup(status: HostSetupStatus | null): boolean {
    if (!status) {
      return false;
    }
    return status.checks.some((check) => check.required && !check.ready);
  }

  function canRelaunchElevated(status: HostSetupStatus | null): boolean {
    const platform = status?.platform ?? "";
    return platform === "windows" || platform === "macos";
  }

  async function refreshSetupGateStatus(options: { respectDismissal?: boolean } = {}): Promise<void> {
    if (!$page.url.pathname.startsWith("/settings")) {
      setupGateVisible = false;
      return;
    }
    const respectDismissal = options.respectDismissal ?? true;
    if (setupGateBusy || setupGateRepairBusy) {
      return;
    }
    setupGateBusy = true;
    setupGateError = null;

    try {
      const status = await invoke<HostSetupStatus>("get_host_setup_status");
      setupGateStatus = status;
      if (requiresHostSetup(status) && (!respectDismissal || !setupGateDismissed)) {
        setupGateVisible = true;
      } else if (!requiresHostSetup(status)) {
        setupGateVisible = false;
      }
    } catch (error) {
      setupGateError =
        error instanceof Error ? error.message : "Failed to load host setup status.";
      if (!setupGateDismissed) {
        setupGateVisible = true;
      }
    } finally {
      setupGateBusy = false;
    }
  }

  async function runSetupGateRepair(): Promise<void> {
    if (setupGateRepairBusy) {
      return;
    }
    setupGateRepairBusy = true;
    setupGateError = null;
    setupGateMessage = null;
    setupGateWarnings = [];
    setupGateAppliedActions = [];

    try {
      const result = await invoke<HostSetupRepairResult>("run_host_setup_repair");
      setupGateStatus = result.status;
      setupGateWarnings = result.warnings ?? [];
      setupGateAppliedActions = result.appliedActions ?? [];
      if (requiresHostSetup(result.status)) {
        setupGateMessage = "Repair completed, but required setup checks are still missing.";
      } else {
        setupGateMessage = "Host setup repair completed successfully.";
        setupGateVisible = false;
      }
    } catch (error) {
      setupGateError = error instanceof Error ? error.message : "Host setup repair failed.";
    } finally {
      setupGateRepairBusy = false;
    }
  }

  async function relaunchElevatedFromSetupGate(): Promise<void> {
    setupGateError = null;
    setupGateMessage = null;

    try {
      const result = await invoke<OperationResult>("relaunch_elevated");
      setupGateMessage = result.message;
      if (result.success) {
        setupGateMessage = `${result.message} Close this window after the elevated app opens.`;
        await getCurrentWindow().minimize();
      } else {
        setupGateError = result.stderr || result.message || "Elevated relaunch failed.";
      }
    } catch (error) {
      setupGateError = error instanceof Error ? error.message : "Elevated relaunch failed.";
    }
  }

  function dismissSetupGate(): void {
    setupGateVisible = false;
    setupGateDismissed = true;
    try {
      localStorage.setItem(setupGateStorageKey, "1");
    } catch {
      // no-op when storage is unavailable
    }
  }

  onMount(() => {
    try {
      isSidebarCollapsed = localStorage.getItem(sidebarStorageKey) === "1";
    } catch {
      isSidebarCollapsed = false;
    }
    try {
      setupGateDismissed = localStorage.getItem(setupGateStorageKey) === "1";
    } catch {
      setupGateDismissed = false;
    }

    const { classList, dataset } = document.body;
    dataset.theme = "helios";
    classList.add(...bodyClasses);
    void refreshConnectionStatus();
    void refreshSetupGateStatus({ respectDismissal: true });
    statusPollHandle = setInterval(() => {
      void refreshConnectionStatus();
    }, 8000);
    return () => {
      if (statusPollHandle) {
        clearInterval(statusPollHandle);
      }
      classList.remove(...bodyClasses);
      delete dataset.theme;
    };
  });
</script>

<svelte:head>
  <title>Atlas Hardware Manager</title>
</svelte:head>

<div class="flex min-h-screen max-h-screen overflow-hidden bg-surface-950 text-surface-50">
  <aside
    class={`hidden shrink-0 border-r border-surface-800/80 bg-surface-900/80 transition-[width] duration-200 lg:flex lg:sticky lg:top-0 lg:h-screen lg:overflow-y-auto ${
      isSidebarCollapsed ? "w-20" : "w-72"
    }`}
  >
    <div class="flex h-full w-full flex-col">
      <div
        class={`flex items-center border-b border-surface-800/70 ${
          isSidebarCollapsed ? "justify-center px-2 py-4" : "gap-3 px-6 py-6"
        }`}
      >
        <img
          src="/logo.svg"
          alt="Atlas Hardware Manager"
          class={`rounded-md border border-surface-700/80 bg-surface-900 object-contain p-1 ${
            isSidebarCollapsed ? "h-9 w-9" : "h-10 w-10"
          }`}
        />
        {#if !isSidebarCollapsed}
          <div class="space-y-1">
            <p class="text-xs font-semibold text-surface-200">Atlas Hardware Manager</p>
            <p class="text-[0.6rem] uppercase tracking-[0.28em] text-surface-500">
              Version · 2026.0.0
            </p>
          </div>
        {/if}
        <button
          class={`btn btn-3xs preset-tonal-surface ${isSidebarCollapsed ? "ml-0" : "ml-auto"} p-2`}
          type="button"
          aria-label={isSidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
          onclick={toggleSidebar}
        >
          <svg class="h-3.5 w-3.5 text-surface-200" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            {#if isSidebarCollapsed}
              <path d="M9 18l6-6-6-6"></path>
            {:else}
              <path d="M15 18l-6-6 6-6"></path>
            {/if}
          </svg>
        </button>
      </div>

      <nav class={`flex-1 overflow-y-auto ${isSidebarCollapsed ? "px-2 py-4" : "px-4 py-6"}`}>
        <ul class="space-y-2">
          {#each navItems as item}
            <li>
              <a
                href={item.href}
                data-sveltekit-prefetch
                title={item.label}
                aria-current={isActive($page.url.pathname, item.href) ? "page" : undefined}
                class={`group flex w-full items-center rounded border border-transparent transition hover:border-surface-700/60 hover:bg-surface-900/70 ${
                  isActive($page.url.pathname, item.href)
                    ? "border-primary-500/70 bg-primary-500/10"
                    : ""
                } ${isSidebarCollapsed ? "justify-center px-2 py-3" : "justify-between px-4 py-3 text-left"}`}
              >
                <div class={`flex items-center ${isSidebarCollapsed ? "justify-center" : "gap-3"}`}>
                  <span class="inline-flex h-8 w-8 items-center justify-center rounded-md border border-surface-700/80 bg-surface-900/70 text-surface-100">
                    <i class={`${item.iconClass} text-sm`} aria-hidden="true"></i>
                  </span>
                  {#if !isSidebarCollapsed}
                    <div>
                      <p class="text-sm font-semibold text-surface-50">{item.label}</p>
                      <p class="text-xs text-surface-400">{item.description}</p>
                    </div>
                  {/if}
                </div>
                {#if !isSidebarCollapsed}
                  <span
                    class={`h-2 w-2 rounded transition ${
                      isActive($page.url.pathname, item.href) ? "bg-primary-400" : "bg-surface-700"
                    }`}
                  ></span>
                {/if}
              </a>
            </li>
          {/each}
        </ul>
      </nav>

      <div class={`border-t border-surface-800/70 ${isSidebarCollapsed ? "px-2 py-4" : "px-6 py-5"}`}>
        {#if isSidebarCollapsed}
          <div class="flex items-center justify-center" title={`System status: ${connectionStatus.label}`}>
            <span class="relative flex h-2 w-2">
              <span class={`absolute inline-flex h-full w-full animate-ping rounded ${statusPingClass(connectionStatus)}`}></span>
              <span class={`relative inline-flex h-2 w-2 rounded ${statusDotClass(connectionStatus)}`}></span>
            </span>
          </div>
        {:else}
          <p class="text-xs font-semibold uppercase tracking-[0.32em] text-surface-500">
            System Status
          </p>
          <div class="mt-3 flex items-center justify-between rounded-md border border-surface-800/70 bg-surface-900/70 px-4 py-3 text-xs">
            <span class="flex items-center gap-2 text-surface-300">
              <span class="relative flex h-2 w-2">
                <span class={`absolute inline-flex h-full w-full animate-ping rounded ${statusPingClass(connectionStatus)}`}></span>
                <span class={`relative inline-flex h-2 w-2 rounded ${statusDotClass(connectionStatus)}`}></span>
              </span>
              {connectionStatus.label}
            </span>
          </div>
          <p class="mt-2 text-[0.65rem] text-surface-500">
            {connectionStatus.detail}
          </p>
        {/if}
      </div>
    </div>
  </aside>

  <div class="flex flex-1 flex-col">
    <header class="sticky top-0 z-10 border-b border-surface-800/70 bg-surface-950/90 backdrop-blur">
      <div class="flex flex-col gap-2 px-6 py-3 lg:flex-row lg:items-center lg:justify-between">
        <div class="flex w-full flex-col justify-center gap-1">
          {#if $pageHeader.eyebrow}
            <p class="uppercase text-[0.55rem] tracking-[0.3em] text-secondary-300">
              {$pageHeader.eyebrow}
            </p>
          {/if}

          <h1 class="text-xl font-semibold uppercase text-surface-50 lg:text-2xl">
            {$pageHeader.title}
          </h1>

          {#if $pageHeader.subtitle}
            <p class="text-xs text-surface-400">
              {$pageHeader.subtitle}
            </p>
          {/if}
        </div>

        <div class="flex flex-wrap items-center gap-2">
          {#if $pageHeader.actions?.length}
            <div
              class="flex max-w-full items-center gap-2 overflow-x-auto rounded border border-surface-800/70 bg-surface-900/70 px-3 py-2 shadow-[0_0_25px_-18px_rgba(0,0,0,1)]"
            >
              {#each $pageHeader.actions as action (action.label)}
                <button
                  class={`btn btn-3xs whitespace-nowrap ${action.className ?? "preset-tonal-surface"} uppercase tracking-[0.28em]`}
                >
                  {action.label}
                </button>
              {/each}
            </div>
          {/if}
        </div>
      </div>
    </header>

    <main class="flex-1 overflow-y-auto bg-surface-900/30 px-6 py-8 lg:px-10 lg:py-10">
      <slot />
    </main>
  </div>
</div>

{#if setupGateVisible && $page.url.pathname.startsWith("/settings")}
  <div class="fixed inset-0 z-50 flex items-center justify-center bg-surface-950/85 px-4 py-6 backdrop-blur-sm">
    <div class="w-full max-w-3xl rounded border border-surface-700/80 bg-surface-900 shadow-[0_0_50px_-22px_rgba(0,0,0,1)]">
      <header class="border-b border-surface-800/70 px-5 py-4">
        <p class="uppercase text-[0.58rem] tracking-[0.32em] text-warning-300">System Setup Required</p>
        <h2 class="mt-1 text-lg font-semibold text-surface-50">Host Runtime Setup Is Incomplete</h2>
        <p class="mt-1 text-xs text-surface-400">
          Atlas detected missing required host tools. Repair now before using update/flash workflows.
        </p>
      </header>

      <div class="space-y-3 px-5 py-5">
        {#if setupGateError}
          <p class="rounded border border-error-500/40 bg-error-500/10 px-3 py-2 text-sm text-error-100">
            {setupGateError}
          </p>
        {/if}

        {#if setupGateMessage}
          <p class="rounded border border-primary-500/40 bg-primary-500/10 px-3 py-2 text-sm text-primary-100">
            {setupGateMessage}
          </p>
        {/if}

        {#if setupGateAppliedActions.length}
          <div class="rounded border border-success-500/40 bg-success-500/10 px-3 py-2 text-xs text-success-100">
            {#each setupGateAppliedActions as action (action)}
              <p>{action}</p>
            {/each}
          </div>
        {/if}

        {#if setupGateWarnings.length}
          <div class="rounded border border-warning-500/40 bg-warning-500/10 px-3 py-2 text-xs text-warning-100">
            {#each setupGateWarnings as warning (warning)}
              <p>{warning}</p>
            {/each}
          </div>
        {/if}

        <div class="grid gap-2 md:grid-cols-2">
          {#each (setupGateStatus?.checks ?? []).filter((check) => check.required && !check.ready) as check (check.id)}
            <article class="rounded border border-error-500/40 bg-error-500/10 px-3 py-3">
              <p class="text-[0.62rem] uppercase tracking-[0.22em] text-error-100">{check.label}</p>
              <p class="mt-1 text-xs text-error-100/90">{check.detail}</p>
              {#if check.detected}
                <p class="mt-1 truncate font-mono text-[0.66rem] text-error-100/85">{check.detected}</p>
              {/if}
              {#if check.fixHint}
                <p class="mt-2 text-[0.66rem] text-warning-100">{check.fixHint}</p>
              {/if}
            </article>
          {/each}
        </div>
      </div>

      <footer class="flex flex-wrap items-center justify-end gap-3 border-t border-surface-800/70 px-5 py-4">
        <a
          class="btn btn-3xs preset-tonal-surface uppercase tracking-[0.2em]"
          href="/settings"
          data-sveltekit-preload-data
          onclick={dismissSetupGate}
        >
          Open Settings
        </a>
        {#if canRelaunchElevated(setupGateStatus)}
          <button
            class="btn btn-3xs preset-tonal-surface uppercase tracking-[0.2em] disabled:cursor-not-allowed disabled:opacity-60"
            type="button"
            disabled={setupGateBusy || setupGateRepairBusy}
            onclick={() => void relaunchElevatedFromSetupGate()}
          >
            Relaunch Elevated
          </button>
        {/if}
        <button
          class="btn btn-3xs preset-tonal-secondary uppercase tracking-[0.2em] disabled:cursor-not-allowed disabled:opacity-60"
          type="button"
          disabled={setupGateBusy || setupGateRepairBusy}
          onclick={() => void runSetupGateRepair()}
        >
          {setupGateRepairBusy ? "Repairing..." : "Run Repair"}
        </button>
        <button
          class="btn btn-3xs preset-tonal-surface uppercase tracking-[0.2em] disabled:cursor-not-allowed disabled:opacity-60"
          type="button"
          disabled={setupGateBusy || setupGateRepairBusy}
          onclick={dismissSetupGate}
        >
          Continue For Now
        </button>
      </footer>
    </div>
  </div>
{/if}
