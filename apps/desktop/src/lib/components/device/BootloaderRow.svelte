<script lang="ts">
  // The "Update bootloader" action for boards that offer it in USB boot.
  import { api, errorText, type DeviceAction, type DeviceRecord } from "$lib/api/client";
  import ConfirmButton from "$lib/components/common/ConfirmButton.svelte";
  import IconTile from "$lib/components/common/IconTile.svelte";
  import { deviceName } from "$lib/format";
  import { toasts } from "$lib/stores/toasts.svelte";

  let { record }: { record: DeviceRecord } = $props();

  let action = $state<DeviceAction | null>(null);

  $effect(() => {
    if (record.presence !== "online") return;
    const key = record.key;
    let stale = false;
    api
      .deviceActions(key)
      .then((list) => {
        if (!stale) action = list.find((a) => a.id === "update-bootloader") ?? null;
      })
      .catch(() => {
        // No actions in this state: the row stays hidden.
      });
    return () => (stale = true);
  });

  async function run() {
    if (!action) return;
    try {
      await api.runDeviceAction(record.key, action.id);
      toasts.success(`Bootloader updated on ${deviceName(record)}.`);
    } catch (error) {
      throw errorText(error);
    }
  }
</script>

{#if action}
  <div class="glass mt-2 flex flex-col gap-3 px-4 py-3.5">
    <div class="flex items-center gap-3">
      <IconTile icon="cpu" size={32} />
      <div class="min-w-0 flex-1">
        <p class="text-[13px] font-medium text-fg">Bootloader</p>
        <p class="text-[12px] text-fg-faint">Writes the bootloader that matches this device package. Only needed for new boards or when an image asks for it.</p>
      </div>
    </div>
    <div>
      <ConfirmButton variant="glass" size="sm" icon="cpu" action={run} prompt="Rewrite the bootloader?" confirmLabel="Update">
        {action.label}
      </ConfirmButton>
    </div>
  </div>
{/if}
