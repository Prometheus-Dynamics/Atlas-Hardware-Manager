<script lang="ts">
  import { api, errorText, type AppSettings } from "$lib/api/client";
  import AsyncButton from "$lib/components/common/AsyncButton.svelte";
  import Panel from "$lib/components/common/Panel.svelte";
  import { sentence } from "$lib/format";
  import { system } from "$lib/stores/system.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";

  let { settings }: { settings: AppSettings } = $props();

  // svelte-ignore state_referenced_locally
  let draft = $state<AppSettings>({ ...settings });
  let saving = $state(false);
  let error = $state<string | null>(null);

  const dirty = $derived(JSON.stringify(draft) !== JSON.stringify(system.settings));
  const seconds = $derived(draft.scan_interval_ms / 1000);

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (saving) return;
    saving = true;
    error = null;
    try {
      const restart = await api.saveSettings($state.snapshot(draft));
      system.settings = { ...draft };
      if (restart) system.restartNeeded = true;
      toasts.success(restart ? "Saved. Restart Atlas to apply every change." : "Settings saved.");
    } catch (e) {
      error = sentence(errorText(e));
    } finally {
      saving = false;
    }
  }
</script>

<Panel eyebrow="Preferences" title="How Atlas works">
  <form class="flex flex-col gap-4" onsubmit={save}>
    <label class="flex items-start gap-3">
      <input type="checkbox" class="checkbox mt-0.5 h-3.5 w-3.5 rounded-sm border-surface-600 bg-surface-900" bind:checked={draft.auto_scan} />
      <span class="text-xs">
        <span class="block text-surface-100">Scan automatically</span>
        <span class="text-surface-400">Look for devices in the background so the inventory stays live.</span>
      </span>
    </label>

    <label class="grid grid-cols-[1fr_8rem] items-center gap-3">
      <span class="text-xs">
        <span class="block text-surface-100">Scan interval</span>
        <span class="text-surface-400">Every {seconds.toFixed(1)} s. At least 1 s.</span>
      </span>
      <input
        class="field font-mono"
        type="number"
        min="1000"
        step="500"
        bind:value={draft.scan_interval_ms}
        disabled={!draft.auto_scan}
        aria-label="Scan interval in milliseconds"
      />
    </label>

    <label class="grid grid-cols-[1fr_8rem] items-center gap-3">
      <span class="text-xs">
        <span class="block text-surface-100">Staged rollout default</span>
        <span class="text-surface-400">Auto updates one device first when a family has three or more.</span>
      </span>
      <select class="field" bind:value={draft.staged_default}>
        <option value="auto">Auto</option>
        <option value="on">On</option>
        <option value="off">Off</option>
      </select>
    </label>


    <label class="grid grid-cols-[1fr_8rem] items-center gap-3">
      <span class="text-xs">
        <span class="block text-surface-100">Simulated devices <span class="tag ml-1 text-warning-300">restart required</span></span>
        <span class="text-surface-400">Replace real hardware with a simulated robot for demos and training.</span>
      </span>
      <select class="field" bind:value={draft.simulated}>
        <option value={null}>Off</option>
        <option value="demo">Demo robot</option>
        <option value="flaky">Flaky robot</option>
      </select>
    </label>

    {#if error}<p class="text-xs text-error-300" role="alert">{error}</p>{/if}

    <div class="flex items-center gap-2 border-t border-surface-800 pt-3">
      <button type="submit" class="btn btn-sm preset-filled-primary-500" disabled={!dirty || saving}>
        {#if saving}<i class="fa-solid fa-circle-notch fa-spin" aria-hidden="true"></i>{/if}Save
      </button>
      <button type="button" class="btn btn-sm preset-tonal" disabled={!dirty || saving} onclick={() => (draft = { ...settings })}>Revert</button>
      {#if system.restartNeeded}
        <span class="ml-auto flex items-center gap-2 text-xs text-warning-300">
          Restart to apply.
          <AsyncButton class="btn btn-sm preset-outlined-warning-500" action={() => api.restartApp()}>
            <i class="fa-solid fa-power-off" aria-hidden="true"></i>Restart now
          </AsyncButton>
        </span>
      {/if}
    </div>
  </form>
</Panel>
