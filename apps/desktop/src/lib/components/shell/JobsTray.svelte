<script lang="ts">
  import { goto } from "$app/navigation";
  import ProgressBar from "$lib/components/common/ProgressBar.svelte";
  import { jobStatusLabel, jobStatusTone, overallFraction, STEP_LABELS, toneText } from "$lib/format";
  import { jobs } from "$lib/stores/jobs.svelte";
  import { ui } from "$lib/stores/ui.svelte";

  const running = $derived(jobs.running);
  const last = $derived(jobs.sorted.find((j) => j.state !== "running"));

  function openJob(id: number) {
    ui.selectedJob = id;
    void goto("/jobs");
  }
</script>

<section class="shrink-0 border-t border-surface-800 bg-surface-900/80" aria-label="Jobs tray">
  <div class="flex h-8 items-center gap-3 px-4 text-xs">
    <span class="micro-label">Jobs</span>
    {#if running.length > 0}
      <span class="text-secondary-200">
        <i class="fa-solid fa-circle-notch fa-spin mr-1" aria-hidden="true"></i>{running.length} running
      </span>
      <button
        type="button"
        class="ml-auto text-[0.65rem] uppercase tracking-[0.14em] text-surface-400 hover:text-surface-100"
        onclick={() => (ui.trayOpen = !ui.trayOpen)}
        aria-expanded={ui.trayOpen}
      >
        {ui.trayOpen ? "Collapse" : "Expand"}
        <i class="fa-solid {ui.trayOpen ? 'fa-chevron-down' : 'fa-chevron-up'} ml-1" aria-hidden="true"></i>
      </button>
    {:else if last}
      <button type="button" class="truncate text-surface-400 hover:text-surface-100" onclick={() => openJob(last.id)}>
        Idle · last job #{last.id}
        {#if last.summary}
          — {last.summary.verified} verified{#if last.summary.failed + last.summary.rolled_back + last.summary.needs_recovery > 0}, <span class="text-warning-400">{last.summary.failed + last.summary.rolled_back + last.summary.needs_recovery} with problems</span>{/if}
        {/if}
      </button>
    {:else}
      <span class="text-surface-500">Idle</span>
    {/if}
  </div>

  {#if running.length > 0 && ui.trayOpen}
    <div class="max-h-44 overflow-y-auto border-t border-surface-800 px-4 py-2">
      {#each running as job (job.id)}
        <div class="mb-2 last:mb-0">
          <button
            type="button"
            class="mb-1 text-[0.65rem] uppercase tracking-[0.14em] text-surface-300 hover:text-surface-50"
            onclick={() => openJob(job.id)}
          >
            Job #{job.id} · {job.devices.length} device{job.devices.length === 1 ? "" : "s"}
            <i class="fa-solid fa-arrow-right ml-1" aria-hidden="true"></i>
          </button>
          <ul class="grid grid-cols-[repeat(auto-fill,minmax(15rem,1fr))] gap-x-4 gap-y-1">
            {#each job.devices as state (state.device.family + state.device.serial)}
              {@const tone = jobStatusTone(state.status)}
              <li class="grid grid-cols-[7rem_1fr_4.5rem] items-center gap-2 text-xs">
                <span class="truncate text-surface-200" title={state.name}>{state.name}</span>
                <ProgressBar
                  value={overallFraction(state)}
                  tone={tone === "error" ? "error" : tone === "warning" ? "warning" : tone === "success" ? "success" : "primary"}
                  label="{state.name} progress"
                />
                <span class="truncate text-right text-[0.65rem] {toneText[tone]}">
                  {state.status.status === "running" && state.step ? STEP_LABELS[state.step] : jobStatusLabel(state.status)}
                </span>
              </li>
            {/each}
          </ul>
        </div>
      {/each}
    </div>
  {/if}
</section>
