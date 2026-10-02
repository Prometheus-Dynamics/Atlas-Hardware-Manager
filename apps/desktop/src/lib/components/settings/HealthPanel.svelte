<script lang="ts">
  import { api, errorText } from "$lib/api/client";
  import Panel from "$lib/components/common/Panel.svelte";
  import { system } from "$lib/stores/system.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";

  let fixing = $state<string | null>(null);

  async function runFix(action: string) {
    if (fixing) return;
    fixing = action;
    try {
      toasts.success(await api.fixHealth(action));
      await system.checkHealth();
    } catch (error) {
      toasts.error(errorText(error));
    } finally {
      fixing = null;
    }
  }

  const style = {
    ok: { icon: "fa-circle-check", cls: "text-success-400" },
    warning: { icon: "fa-triangle-exclamation", cls: "text-warning-400" },
    error: { icon: "fa-circle-xmark", cls: "text-error-400" },
  };
</script>

<Panel eyebrow="Host health" title={system.healthProblems === 0 ? "Everything checks out" : `${system.healthProblems} problem${system.healthProblems === 1 ? "" : "s"} found`}>
  {#snippet actions()}
    <button type="button" class="btn btn-sm preset-tonal" onclick={() => system.checkHealth()} disabled={system.checking}>
      <i class="fa-solid fa-rotate {system.checking ? 'fa-spin' : ''}" aria-hidden="true"></i>Re-check
    </button>
  {/snippet}
  <ul class="flex flex-col gap-2">
    {#each system.healthSorted as check (check.id)}
      <li class="grid grid-cols-[1.25rem_1fr] gap-2 rounded-base border border-surface-800 px-3 py-2">
        <i class="fa-solid {style[check.status].icon} {style[check.status].cls} mt-0.5" aria-label={check.status}></i>
        <div class="text-xs">
          <p class="font-semibold text-surface-50">{check.label}</p>
          <p class="text-surface-300">{check.detail}</p>
          {#if check.fix}
            <p class="mt-1 text-surface-400"><span class="micro-label mr-1">Fix</span>{check.fix}</p>
          {/if}
          {#if check.fix_action}
            {@const action = check.fix_action}
            <button
              type="button"
              class="btn btn-sm preset-filled-primary-500 mt-2"
              disabled={fixing !== null}
              onclick={() => runFix(action)}
            >
              <i class="fa-solid {fixing === action ? 'fa-circle-notch fa-spin' : 'fa-wrench'}" aria-hidden="true"></i>Fix
            </button>
          {/if}
        </div>
      </li>
    {:else}
      <li class="text-xs text-surface-400">{system.checking ? "Checking…" : "No checks reported."}</li>
    {/each}
  </ul>
</Panel>
