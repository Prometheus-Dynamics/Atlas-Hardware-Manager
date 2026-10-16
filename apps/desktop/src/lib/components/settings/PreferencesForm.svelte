<script lang="ts">
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { api, errorText, type AppSettings, type ClockSync, type SimScenario, type StagedRollout } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import Field from "#lib/components/common/Field.svelte";
  import GlassCard from "#lib/components/common/GlassCard.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import SegmentedControl from "#lib/components/common/SegmentedControl.svelte";
  import Toggle from "#lib/components/common/Toggle.svelte";
  import { sentence } from "#lib/format.ts";
  import { scale, type ScaleChoice } from "#lib/stores/scale.svelte.ts";
  import { system } from "#lib/stores/system.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { rise } from "#lib/ui/motion.ts";

  let { settings }: { settings: AppSettings } = $props();

  // svelte-ignore state_referenced_locally
  let draft = $state<AppSettings>({ ...settings, app_updates: { ...settings.app_updates } });
  let saving = $state(false);
  let error = $state<string | null>(null);

  const dirty = $derived(JSON.stringify(draft) !== JSON.stringify(system.settings));
  const seconds = $derived(draft.scan_interval_ms / 1000);

  const stagedOptions: { value: StagedRollout; label: string }[] = [
    { value: "auto", label: "Auto" },
    { value: "on", label: "One first" },
    { value: "off", label: "All at once" },
  ];
  const clockOptions: { value: ClockSync; label: string }[] = [
    { value: "usb", label: "USB boards" },
    { value: "all", label: "All boards" },
    { value: "off", label: "Off" },
  ];
  const simOptions: { value: "off" | SimScenario; label: string }[] = [
    { value: "off", label: "Off" },
    { value: "demo", label: "Demo robot" },
    { value: "flaky", label: "Flaky robot" },
  ];
  const scaleOptions: { value: ScaleChoice; label: string }[] = [
    { value: "auto", label: "Auto" },
    { value: "1", label: "100%" },
    { value: "1.1", label: "110%" },
    { value: "1.25", label: "125%" },
    { value: "1.5", label: "150%" },
  ];
  // Applies at once, on this computer only (not part of Save).
  let scaleChoice = $state<ScaleChoice>(scale.choice);
  $effect(() => {
    if (scaleChoice !== scale.choice) scale.set(scaleChoice);
  });

  const DEFAULT_KEY = "~/.ssh/id_ed25519.pub";
  // svelte-ignore state_referenced_locally
  let keyOn = $state(!!settings.ssh_key_file);
  // svelte-ignore state_referenced_locally
  let keyFile = $state(settings.ssh_key_file ?? DEFAULT_KEY);
  $effect(() => {
    draft.ssh_key_file = keyOn && keyFile.trim() ? keyFile.trim() : null;
  });

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
      system.settings = $state.snapshot(draft);
      if (restart) system.restartNeeded = true;
      toasts.success(restart ? "Saved. Restart Atlas to apply every change." : "Settings saved.");
    } catch (e) {
      error = sentence(errorText(e));
    } finally {
      saving = false;
    }
  }

  function revert() {
    draft = { ...settings, app_updates: { ...settings.app_updates } };
    sim = settings.simulated ?? "off";
    keyOn = !!settings.ssh_key_file;
    keyFile = settings.ssh_key_file ?? DEFAULT_KEY;
  }
</script>

<GlassCard title="Preferences" subtitle="Finding devices and rolling out updates" icon="settings" large fill>
  <form class="flex flex-col gap-3" onsubmit={save}>
    <Field inline label="Watch for devices" hint="Devices appear and disappear as they're plugged in, unplugged, or announce themselves on the network.">
      <Toggle bind:checked={draft.auto_scan} label="Watch for devices" />
    </Field>

    <Field inline label="Safety-net check" hint="Every {seconds.toFixed(0)} s, look again for devices that left without saying so.">
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

    <Field inline label="Live readings from Orion" hint="Boards whose Orion node streams their readings push them as they happen, with the last five minutes at once. Other boards are asked every second or two.">
      <Toggle bind:checked={draft.live_streams} label="Live readings from Orion" />
    </Field>

    <Field
      inline
      label="Add my SSH key to boards I flash"
      hint="Puts your public key on the board's boot partition; the board installs it for root on first start. Password login stays off."
    >
      <Toggle bind:checked={keyOn} label="Add my SSH key to boards I flash" />
    </Field>
    {#if keyOn}
      <Field label="Public key file" hint="The .pub file, never the private key.">
        <input class="input mono" bind:value={keyFile} placeholder={DEFAULT_KEY} aria-label="Public key file" />
      </Field>
    {/if}

    <!-- One row each, the choice at the right, like the switches above. -->
    <Field inline label="Staged rollout" hint="Auto updates one device first when a family has three or more.">
      <SegmentedControl options={stagedOptions} bind:value={draft.staged_default} label="Staged rollout default" size="sm" />
    </Field>

    <Field
      inline
      label="Interface size"
      hint="Auto makes everything larger on a big window{scale.choice === 'auto' ? ` (now ${Math.round(scale.factor * 100)}%)` : ''}. Applies at once, on this computer."
    >
      <SegmentedControl options={scaleOptions} bind:value={scaleChoice} label="Interface size" size="sm" />
    </Field>

    <Field
      inline
      label="Keep board clocks synced"
      hint="A board with no clock battery can be hours off; one more than 2 s off gets this computer's time."
    >
      <SegmentedControl options={clockOptions} bind:value={draft.clock_sync} label="Keep board clocks synced" size="sm" />
    </Field>

    <Field inline label="Look for Atlas updates" hint="On start and every few hours, from its GitHub releases. You choose when to install.">
      <Toggle bind:checked={draft.app_updates.auto_check} label="Look for Atlas updates" />
    </Field>
    <Field inline label="Include pre-releases" hint="Release candidates too, before they're stable.">
      <Toggle bind:checked={draft.app_updates.prereleases} label="Include pre-releases" />
    </Field>

    <Field inline label="Simulated devices" hint="A simulated robot instead of real hardware, for demos and training. Needs a restart.">
      <SegmentedControl options={simOptions} bind:value={sim} label="Simulated devices" size="sm" />
    </Field>

    {#if error}<p class="flex items-center gap-2 text-[13px] text-err-fg" role="alert"><Icon name="alert-circle" size={15} />{error}</p>{/if}

    <!-- Save stays in view when the card scrolls. -->
    <div class="save flex items-center gap-2 border-t border-hairline pb-1 pt-4">
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

<style>
  .save {
    position: sticky;
    bottom: 0;
    z-index: 1;
    /* Opaque, but the same colour as the card: its glass over the page. */
    background: linear-gradient(var(--glass), var(--glass)), var(--bg);
  }
</style>
