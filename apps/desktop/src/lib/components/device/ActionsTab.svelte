<script lang="ts">
  import { api, errorText, type DeviceAction, type DeviceRecord } from "$lib/api/client";
  import AsyncButton from "$lib/components/common/AsyncButton.svelte";
  import ConfirmButton from "$lib/components/common/ConfirmButton.svelte";
  import { deviceName } from "$lib/format";
  import { toasts } from "$lib/stores/toasts.svelte";

  let { record }: { record: DeviceRecord } = $props();

  let actions = $state<DeviceAction[] | null>(null);
  let error = $state<string | null>(null);

  $effect(() => {
    const key = record.key;
    let stale = false;
    api
      .deviceActions(key)
      .then((list) => !stale && (actions = list))
      .catch((e) => !stale && (error = errorText(e)));
    return () => (stale = true);
  });

  async function run(action: DeviceAction) {
    await api.runDeviceAction(record.key, action.id);
    toasts.success(`${action.label}: sent to ${deviceName(record)}.`);
  }
</script>

{#if error}
  <p class="text-xs text-error-300">{error}</p>
{:else if !actions}
  <p class="text-xs text-surface-400"><i class="fa-solid fa-circle-notch fa-spin mr-1" aria-hidden="true"></i>Loading actions…</p>
{:else if actions.length === 0}
  <p class="text-xs text-surface-400">This device offers no actions.</p>
{:else}
  <ul class="flex flex-col gap-2">
    {#each actions as action (action.id)}
      <li class="flex items-center justify-between gap-3 rounded-base border border-surface-800 px-3 py-2">
        <span class="text-xs text-surface-100">
          {action.label}
          {#if action.destructive}<span class="ml-1 text-[0.6rem] uppercase tracking-[0.12em] text-error-300">destructive</span>{/if}
        </span>
        {#if action.destructive}
          <ConfirmButton action={() => run(action)} prompt="{action.label} {deviceName(record)}?" confirmLabel={action.label}>
            {action.label}
          </ConfirmButton>
        {:else}
          <AsyncButton action={() => run(action)}>{action.label}</AsyncButton>
        {/if}
      </li>
    {/each}
  </ul>
{/if}
