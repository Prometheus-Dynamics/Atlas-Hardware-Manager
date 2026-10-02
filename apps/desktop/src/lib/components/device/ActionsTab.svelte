<script lang="ts">
  import { api, errorText, type DeviceAction, type DeviceRecord } from "$lib/api/client";
  import Button from "$lib/components/common/Button.svelte";
  import ConfirmButton from "$lib/components/common/ConfirmButton.svelte";
  import Icon from "$lib/components/common/Icon.svelte";
  import Skeleton from "$lib/components/common/Skeleton.svelte";
  import { deviceName } from "$lib/format";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { rise } from "$lib/ui/motion";

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
  <p class="flex items-center gap-2 text-[13px] text-err-fg"><Icon name="alert-circle" size={16} />{error}</p>
{:else if !actions}
  <div class="flex flex-col gap-2">
    {#each [0, 1, 2] as i (i)}<Skeleton height={48} />{/each}
  </div>
{:else if actions.length === 0}
  <p class="text-[13px] text-fg-muted">This device offers no actions.</p>
{:else}
  <ul class="flex flex-col gap-2">
    {#each actions as action, i (action.id)}
      <li class="glass flex items-center justify-between gap-3 px-4 py-2.5" in:rise={{ delay: i * 30 }}>
        <span class="text-[13px] text-fg">
          {action.label}
          {#if action.destructive}<span class="ml-1.5 text-[12px] text-err-fg">Can't be undone</span>{/if}
        </span>
        {#if action.destructive}
          <ConfirmButton action={() => run(action)} size="sm" prompt="{action.label}?" confirmLabel={action.label}>
            {action.label}
          </ConfirmButton>
        {:else}
          <Button size="sm" action={() => run(action)}>{action.label}</Button>
        {/if}
      </li>
    {/each}
  </ul>
{/if}
