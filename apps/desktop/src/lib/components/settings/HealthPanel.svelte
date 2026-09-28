<script lang="ts">
  import Panel from "$lib/components/common/Panel.svelte";
  import { system } from "$lib/stores/system.svelte";

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
        </div>
      </li>
    {:else}
      <li class="text-xs text-surface-400">{system.checking ? "Checking…" : "No checks reported."}</li>
    {/each}
  </ul>
</Panel>
