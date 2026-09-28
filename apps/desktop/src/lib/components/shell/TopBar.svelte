<script lang="ts">
  import { page } from "$app/state";
  import { devices } from "$lib/stores/devices.svelte";
  import { robots } from "$lib/stores/robots.svelte";
  import { system } from "$lib/stores/system.svelte";
  import { NO_ROBOT, ui } from "$lib/stores/ui.svelte";
  import { timeAgo } from "$lib/format";
  import { clock } from "$lib/stores/clock.svelte";


  const onInventory = $derived(page.url.pathname === "/");
  const count = $derived(ui.updatable.length);
  const recover = $derived(count > 0 && ui.updatable.every((d) => d.identity.mode === "recovery"));
  const staged = $derived(system.settings?.staged_default ?? "auto");
</script>

<header class="flex h-12 shrink-0 items-center gap-3 border-b border-surface-800 bg-surface-900/60 px-4">
  <label class="flex items-center gap-2">
    <span class="micro-label">Robot</span>
    <select class="field py-0.5 pr-7 text-xs" bind:value={ui.robot} aria-label="Robot filter">
      <option value={null}>All devices</option>
      {#each robots.names as name (name)}
        <option value={name}>{name}</option>
      {/each}
      <option value={NO_ROBOT}>No robot</option>
    </select>
  </label>

  <div class="flex items-center gap-2 text-xs text-surface-300" aria-live="polite">
    <span class="inline-block h-2 w-2 rounded-full {devices.online > 0 ? 'bg-success-500' : 'bg-surface-600'}"></span>
    <span><strong class="text-surface-50">{devices.online}</strong> online</span>
    <span class="text-surface-600">·</span>
    <span class={devices.offline > 0 ? "text-error-300" : ""}>{devices.offline} offline</span>
    {#if system.info?.simulated}
      <span class="tag text-tertiary-300" title="Devices are simulated ({system.info.simulated})">
        <i class="fa-solid fa-flask text-[0.55rem]" aria-hidden="true"></i>sim · {system.info.simulated}
      </span>
    {/if}
  </div>

  <div class="ml-auto flex items-center gap-2">
    <span class="hidden text-[0.65rem] text-surface-500 lg:inline">
      {#if devices.scanning}scanning…{:else if devices.lastScanAt}scanned {timeAgo(devices.lastScanAt, clock.now)}{/if}
    </span>
    <button
      type="button"
      class="btn btn-sm preset-tonal"
      onclick={() => devices.scanNow()}
      disabled={devices.scanning}
      title="Scan now (S)"
    >
      <i class="fa-solid fa-satellite-dish {devices.scanning ? 'animate-pulse' : ''}" aria-hidden="true"></i>
      Scan
    </button>
    <button
      type="button"
      class="btn-icon btn-icon-sm preset-tonal"
      onclick={() => (ui.helpOpen = !ui.helpOpen)}
      title="Keyboard shortcuts (?)"
      aria-label="Keyboard shortcuts"
    >
      <i class="fa-solid fa-keyboard" aria-hidden="true"></i>
    </button>
    {#if onInventory || count > 0}
      <button
        type="button"
        class="btn btn-sm preset-filled-primary-500 uppercase tracking-[0.12em]"
        disabled={count === 0}
        onclick={() => ui.updateSelection(staged)}
        title={count === 0 ? "Select online devices to update" : "Update the selection (U)"}
      >
        <i class="fa-solid {recover ? 'fa-kit-medical' : 'fa-arrow-up-from-bracket'}" aria-hidden="true"></i>
        {recover ? "Recover" : "Update"}{count > 0 ? ` (${count})` : ""}
      </button>
    {/if}
  </div>
</header>
