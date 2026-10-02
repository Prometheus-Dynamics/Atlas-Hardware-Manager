<script lang="ts">
  import { devices } from "$lib/stores/devices.svelte";
  import { api, errorText, type AppSettings, type SimScenario, type StagedRollout } from "$lib/api/client";
  import Button from "$lib/components/common/Button.svelte";
  import Field from "$lib/components/common/Field.svelte";
  import GlassCard from "$lib/components/common/GlassCard.svelte";
  import Icon from "$lib/components/common/Icon.svelte";
  import Pill from "$lib/components/common/Pill.svelte";
  import SegmentedControl from "$lib/components/common/SegmentedControl.svelte";
  import Toggle from "$lib/components/common/Toggle.svelte";
  import { sentence } from "$lib/format";
  import { system } from "$lib/stores/system.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { rise } from "$lib/ui/motion";

  let { settings }: { settings: AppSettings } = $props();

  // svelte-ignore state_referenced_locally
  let draft = $state<AppSettings>({ ...settings });
  let saving = $state(false);
  let error = $state<string | null>(null);

  const dirty = $derived(JSON.stringify(draft) !== JSON.stringify(system.settings));
  const seconds = $derived(draft.scan_interval_ms / 1000);

  const stagedOptions: { value: StagedRollout; label: string }[] = [
    { value: "auto", label: "Auto" },
    { value: "on", label: "One first" },
    { value: "off", label: "All at once" },
  ];
  const simOptions: { value: "off" | SimScenario; label: string }[] = [
    { value: "off", label: "Off" },
    { value: "demo", label: "Demo robot" },
    { value: "flaky", label: "Flaky robot" },
  ];
  // svelte-ignore state_referenced_locally
  let sim = $state<"off" | SimScenario>(settings.simulated ?? "off");
  $effect(() => {
    draft.simulated = sim === "off" ? null : sim;
  });

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (saving) return;
    saving = true;
    error = null;
    try {
      const restart = await api.saveSettings($state.snapshot(draft));
      void devices.loadDiscovery();
      system.settings = { ...draft };
      if (restart) system.restartNeeded = true;
      toasts.success(restart ? "Saved. Restart Atlas to apply every change." : "Settings saved.");
    } catch (e) {
      error = sentence(errorText(e));
    } finally {
      saving = false;
    }
  }

  function revert() {
    draft = { ...settings };
    sim = settings.simulated ?? "off";
  }
</script>

<GlassCard title="Preferences" subtitle="How Atlas looks for devices and rolls out updates" icon="settings" large>
  <form class="flex flex-col gap-5" onsubmit={save}>
    <Field inline label="Watch for devices" hint="Devices appear and disappear as they're plugged in, unplugged, or announce themselves on the network.">
      <Toggle bind:checked={draft.auto_scan} label="Watch for devices" />
    </Field>

    <Field inline label="Safety-net check" hint="Every {seconds.toFixed(0)} s Atlas also looks again, for devices that leave without saying so.">
      <input
        class="input mono w-28"
        type="number"
        min="5000"
        max="300000"
        step="5000"
        bind:value={draft.scan_interval_ms}
        disabled={!draft.auto_scan}
        aria-label="Safety-net check interval in milliseconds"
      />
    </Field>

    <div class="flex flex-col gap-1.5">
      <span class="text-[13px] font-medium text-fg">Staged rollout</span>
      <span class="text-[12px] text-fg-faint">Auto updates one device first when a family has three or more.</span>
      <div class="mt-1"><SegmentedControl options={stagedOptions} bind:value={draft.staged_default} label="Staged rollout default" size="sm" /></div>
    </div>

    <div class="flex flex-col gap-1.5">
      <span class="flex items-center gap-2 text-[13px] font-medium text-fg">Simulated devices <Pill tone="warning" label="Needs a restart" /></span>
      <span class="text-[12px] text-fg-faint">Replace real hardware with a simulated robot for demos and training.</span>
      <div class="mt-1"><SegmentedControl options={simOptions} bind:value={sim} label="Simulated devices" size="sm" /></div>
    </div>

    {#if error}<p class="flex items-center gap-2 text-[13px] text-err-fg" role="alert"><Icon name="alert-circle" size={15} />{error}</p>{/if}

    <div class="flex items-center gap-2 border-t border-hairline pt-4">
      <Button type="submit" variant="primary" busy={saving} disabled={!dirty}>Save</Button>
      <Button variant="ghost" disabled={!dirty || saving} onclick={revert}>Revert</Button>
      {#if system.restartNeeded}
        <span class="ml-auto flex items-center gap-2 text-[13px] text-warn-fg" in:rise>
          Restart to apply.
          <Button size="sm" icon="power" action={() => api.restartApp()}>Restart now</Button>
        </span>
      {/if}
    </div>
  </form>
</GlassCard>
