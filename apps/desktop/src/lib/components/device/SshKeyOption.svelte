<script lang="ts">
  // "Add my SSH key" right where a flash starts, so it can't be missed. The
  // same remembered setting as in Settings; turning it on checks the key.
  import { api, errorText } from "#lib/api/client.ts";
  import Checkbox from "#lib/components/common/Checkbox.svelte";
  import { system } from "#lib/stores/system.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";

  const DEFAULT_KEY = "~/.ssh/id_ed25519.pub";
  const keyFile = $derived(system.settings?.ssh_key_file ?? null);
  let saving = $state(false);

  async function toggle() {
    if (!system.settings || saving) return;
    saving = true;
    const next = { ...system.settings, ssh_key_file: keyFile ? null : DEFAULT_KEY };
    try {
      await api.saveSettings(next);
      system.settings = next;
    } catch (error) {
      toasts.error(`${errorText(error)}. Set the key file in Settings.`);
    } finally {
      saving = false;
    }
  }
</script>

{#if system.settings}
  <div class="flex items-center gap-2.5 text-[13px]">
    <Checkbox checked={!!keyFile} label="Add my SSH key to this board" onclick={toggle} />
    <span class="min-w-0 text-fg">
      Add my SSH key
      <span class="mono text-[11.5px] text-fg-faint">{keyFile ?? DEFAULT_KEY}</span>
    </span>
  </div>
  <p class="hint -mt-1 pl-8">Lets you log in as root over SSH once the board starts. Remembered for next time.</p>
{/if}
