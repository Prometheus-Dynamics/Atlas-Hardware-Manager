<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { Panel } from "$lib/components";
  import { resetPageHeader, setPageHeader } from "$lib/stores/pageHeader";

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

  let setupStatus: HostSetupStatus | null = null;
  let loadBusy = false;
  let repairBusy = false;
  let loadError: string | null = null;
  let actionMessage: string | null = null;
  let actionWarnings: string[] = [];
  const HIDDEN_ADVISORY_CHECK_IDS = new Set(["linux-elevation", "windows-admin", "macos-root"]);

  $: requiredChecks = setupStatus?.checks.filter((check) => check.required) ?? [];
  $: optionalChecks =
    setupStatus?.checks.filter(
      (check) => !check.required && !HIDDEN_ADVISORY_CHECK_IDS.has(check.id),
    ) ?? [];
  $: missingRequiredCount = requiredChecks.filter((check) => !check.ready).length;
  $: canRelaunchElevated =
    (setupStatus?.platform ?? "") === "windows" || (setupStatus?.platform ?? "") === "macos";

  onMount(() => {
    setPageHeader({
      title: "SETTINGS",
      subtitle: "Host readiness and required runtime tooling",
      actions: [],
    });
    void loadSetupStatus();
    return () => resetPageHeader();
  });

  async function loadSetupStatus(): Promise<void> {
    if (loadBusy) {
      return;
    }
    loadBusy = true;
    loadError = null;

    try {
      setupStatus = await invoke<HostSetupStatus>("get_host_setup_status");
    } catch (error) {
      loadError = error instanceof Error ? error.message : "Failed to load host setup status.";
    } finally {
      loadBusy = false;
    }
  }

  async function runRepair(): Promise<void> {
    if (repairBusy) {
      return;
    }
    repairBusy = true;
    loadError = null;
    actionMessage = null;
    actionWarnings = [];

    try {
      const result = await invoke<HostSetupRepairResult>("run_host_setup_repair");
      setupStatus = result.status;
      actionWarnings = result.warnings ?? [];
      const appliedCount = result.appliedActions?.length ?? 0;
      actionMessage =
        appliedCount > 0
          ? `Repair completed. Applied ${appliedCount} action${appliedCount === 1 ? "" : "s"}.`
          : "Repair completed with no direct changes.";
    } catch (error) {
      loadError = error instanceof Error ? error.message : "Host setup repair failed.";
    } finally {
      repairBusy = false;
    }
  }

  async function relaunchElevated(): Promise<void> {
    loadError = null;
    actionMessage = null;
    actionWarnings = [];

    try {
      const result = await invoke<OperationResult>("relaunch_elevated");
      if (result.success) {
        actionMessage = `${result.message} Close this window after the elevated app opens.`;
      } else {
        loadError = result.stderr || result.message || "Elevated relaunch failed.";
      }
    } catch (error) {
      loadError = error instanceof Error ? error.message : "Elevated relaunch failed.";
    }
  }

  function checkClass(check: HostSetupCheck): string {
    if (check.ready) {
      return "border-success-500/50 bg-success-500/10";
    }
    if (check.required) {
      return "border-error-500/50 bg-error-500/10";
    }
    return "border-warning-500/50 bg-warning-500/10";
  }

  function checkStatusLabel(check: HostSetupCheck): string {
    if (check.ready) {
      return "Ready";
    }
    if (check.required) {
      return "Missing";
    }
    return "Advisory";
  }

  function updatedLabel(): string {
    if (!setupStatus?.generatedAtEpochMs) {
      return "No scan yet";
    }
    return `Updated ${new Date(setupStatus.generatedAtEpochMs).toLocaleString()}`;
  }
</script>

<div class="flex h-full min-h-0 min-w-0 flex-1 flex-col gap-4 overflow-x-hidden overflow-y-auto pr-1">
  <Panel eyebrow="Host Runtime" title="Tool Detection">
    <svelte:fragment slot="actions">
      <button
        class="btn btn-3xs preset-tonal-secondary uppercase tracking-[0.2em] disabled:cursor-not-allowed disabled:opacity-60"
        type="button"
        disabled={loadBusy || repairBusy}
        onclick={() => void loadSetupStatus()}
      >
        {loadBusy ? "Refreshing..." : "Refresh"}
      </button>
      <button
        class="btn btn-3xs preset-tonal-surface uppercase tracking-[0.2em] disabled:cursor-not-allowed disabled:opacity-60"
        type="button"
        disabled={loadBusy || repairBusy}
        onclick={() => void runRepair()}
      >
        {repairBusy ? "Repairing..." : "Run Repair"}
      </button>
      {#if canRelaunchElevated}
        <button
          class="btn btn-3xs preset-tonal-surface uppercase tracking-[0.2em] disabled:cursor-not-allowed disabled:opacity-60"
          type="button"
          disabled={loadBusy || repairBusy}
          onclick={() => void relaunchElevated()}
        >
          Relaunch Elevated
        </button>
      {/if}
    </svelte:fragment>

    {#if loadError}
      <p class="rounded border border-error-500/40 bg-error-500/10 px-3 py-2 text-sm text-error-100">
        {loadError}
      </p>
    {/if}
    {#if actionMessage}
      <p class="rounded border border-primary-500/40 bg-primary-500/10 px-3 py-2 text-sm text-primary-100">
        {actionMessage}
      </p>
    {/if}
    {#if actionWarnings.length}
      <div class="rounded border border-warning-500/40 bg-warning-500/10 px-3 py-2 text-xs text-warning-100">
        {#each actionWarnings as warning (warning)}
          <p>{warning}</p>
        {/each}
      </div>
    {/if}

    <div class="flex flex-wrap items-center gap-2 text-xs">
      <span
        class={`rounded border px-2 py-1 uppercase tracking-[0.22em] ${
          setupStatus?.ready
            ? "border-success-500/50 bg-success-500/10 text-success-100"
            : "border-warning-500/50 bg-warning-500/10 text-warning-100"
        }`}
      >
        {setupStatus?.ready ? "Host Ready" : "Setup Needed"}
      </span>
      <span class="rounded border border-surface-700/70 bg-surface-900/60 px-2 py-1 uppercase tracking-[0.22em] text-surface-300">
        {setupStatus?.platform ?? "unknown"}-{setupStatus?.arch ?? "unknown"}
      </span>
      <span class="text-surface-500">{updatedLabel()}</span>
    </div>

    <p class="text-xs text-surface-400">
      Required checks missing: {missingRequiredCount}
    </p>
  </Panel>

  <Panel eyebrow="Required Tools" title="Must Be Present">
    {#if requiredChecks.length === 0}
      <p class="text-sm text-surface-400">No required checks were reported.</p>
    {:else}
      <div class="grid min-w-0 gap-2 md:grid-cols-2 xl:grid-cols-3">
        {#each requiredChecks as check (check.id)}
          <article class={`min-w-0 rounded border px-3 py-3 ${checkClass(check)}`}>
            <div class="flex items-center justify-between gap-2">
              <p class="text-xs font-semibold uppercase tracking-[0.22em] text-surface-100">{check.label}</p>
              <span class="text-[0.65rem] uppercase tracking-[0.2em] text-surface-200">{checkStatusLabel(check)}</span>
            </div>
            <p class="mt-2 text-xs text-surface-200/90">{check.detail}</p>
            {#if check.detected}
              <p class="mt-2 truncate font-mono text-[0.68rem] text-surface-200/80">{check.detected}</p>
            {/if}
            {#if check.fixHint && !check.ready}
              <p class="mt-2 text-[0.68rem] text-warning-100">{check.fixHint}</p>
            {/if}
          </article>
        {/each}
      </div>
    {/if}
  </Panel>

  {#if optionalChecks.length > 0}
    <Panel eyebrow="Advisory Checks" title="Privileges And Optionals">
      <div class="grid min-w-0 gap-2 md:grid-cols-2 xl:grid-cols-3">
        {#each optionalChecks as check (check.id)}
          <article class={`min-w-0 rounded border px-3 py-3 ${checkClass(check)}`}>
            <div class="flex items-center justify-between gap-2">
              <p class="text-xs font-semibold uppercase tracking-[0.22em] text-surface-100">{check.label}</p>
              <span class="text-[0.65rem] uppercase tracking-[0.2em] text-surface-200">{checkStatusLabel(check)}</span>
            </div>
            <p class="mt-2 text-xs text-surface-200/90">{check.detail}</p>
            {#if check.detected}
              <p class="mt-2 truncate font-mono text-[0.68rem] text-surface-200/80">{check.detected}</p>
            {/if}
            {#if check.fixHint && !check.ready}
              <p class="mt-2 text-[0.68rem] text-warning-100">{check.fixHint}</p>
            {/if}
          </article>
        {/each}
      </div>
    </Panel>
  {/if}
</div>
