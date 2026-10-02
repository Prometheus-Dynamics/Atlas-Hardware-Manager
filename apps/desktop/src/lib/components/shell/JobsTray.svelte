<script lang="ts">
  // The persistent jobs tray: running jobs at a glance, a click from detail.
  import { goto } from "$app/navigation";
  import { slide } from "svelte/transition";
  import { keyString, type JobRecord } from "$lib/api/client";
  import Button from "$lib/components/common/Button.svelte";
  import Icon from "$lib/components/common/Icon.svelte";
  import ProgressBar from "$lib/components/common/ProgressBar.svelte";
  import StageDots from "$lib/components/common/StageDots.svelte";
  import StatusDot from "$lib/components/common/StatusDot.svelte";
  import { overallFraction } from "$lib/format";
  import { jobTitle, outcomeText } from "$lib/present";
  import { jobs } from "$lib/stores/jobs.svelte";
  import { ui } from "$lib/stores/ui.svelte";
  import { DUR, ease, ms } from "$lib/ui/motion";

  const running = $derived(jobs.running);
  const last = $derived(jobs.sorted.find((j) => j.state !== "running"));

  function progress(job: JobRecord): number {
    if (job.devices.length === 0) return 0;
    return job.devices.reduce((sum, d) => sum + overallFraction(d), 0) / job.devices.length;
  }

  function problems(job: JobRecord): number {
    const s = job.summary;
    return s ? s.failed + s.rolled_back + s.needs_recovery : 0;
  }

  function openJob(id: number) {
    ui.selectedJob = id;
    void goto("/jobs");
  }
</script>

{#if running.length > 0 || last}
  <section class="tray glass-layer" aria-label="Jobs" transition:slide={{ duration: ms(DUR.med), easing: ease }}>
    {#if running.length > 0}
      {#each running as job (job.id)}
        {@const p = progress(job)}
        <div class="row">
          <StatusDot state="busy" label="Running" />
          <button type="button" class="min-w-0 truncate text-left text-[13px] font-medium text-fg hover:underline" onclick={() => openJob(job.id)}>
            {jobTitle(job)}
          </button>
          {#if job.devices.length === 1}<StageDots job={job.devices[0]} />{/if}
          <span class="flex-1"><ProgressBar value={p} size={5} label="{jobTitle(job)} progress" /></span>
          <span class="w-10 text-right text-[12.5px] tabular-nums text-accent-text">{Math.round(p * 100)}%</span>
          {#if job.devices.length > 1}
            <Button
              variant="ghost"
              size="sm"
              icon={ui.trayOpen ? "chevron-down" : "chevron-up"}
              label={ui.trayOpen ? "Hide devices" : "Show devices"}
              onclick={() => (ui.trayOpen = !ui.trayOpen)}
            />
          {/if}
          <Button variant="ghost" size="sm" iconRight="arrow-right" onclick={() => openJob(job.id)}>Open</Button>
        </div>
        {#if ui.trayOpen && job.devices.length > 1}
          <ul class="devices" transition:slide={{ duration: ms(DUR.med), easing: ease }}>
            {#each job.devices as state (keyString(state.device))}
              <li class="flex items-center gap-3 text-[12.5px]">
                <span class="w-32 truncate text-fg-muted" title={state.name}>{state.name}</span>
                <StageDots job={state} />
                <span class="flex-1"><ProgressBar value={overallFraction(state)} size={4} label="{state.name} progress" /></span>
                <span class="w-24 truncate text-right text-fg-faint">
                  {state.status.status === "running" ? `${Math.round(overallFraction(state) * 100)}%` : state.status.status === "verified" ? "Verified" : outcomeText(state)}
                </span>
              </li>
            {/each}
          </ul>
        {/if}
      {/each}
    {:else if last}
      <button type="button" class="row idle" onclick={() => openJob(last.id)}>
        <Icon
          name={problems(last) > 0 ? "alert-circle" : "circle-check"}
          size={16}
          class={problems(last) > 0 ? "text-err-fg" : "text-ok-fg"}
        />
        <span class="truncate text-[12.5px] text-fg-muted">
          {jobTitle(last)}{#if last.summary}{` · ${last.summary.verified} verified`}{#if problems(last)}, <span class="text-err-fg">{problems(last)} didn't finish</span>{/if}{/if}
        </span>
        <span class="ml-auto text-[12px] text-fg-faint">Nothing running</span>
      </button>
    {/if}
  </section>
{/if}

<style>
  .tray {
    margin: 0 12px 12px;
    padding: 4px 6px;
    border-radius: var(--r-card);
    flex-shrink: 0;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 40px;
    padding: 0 8px 0 12px;
  }
  .row.idle {
    width: 100%;
    text-align: left;
    border-radius: 10px;
    transition: background var(--t-fast);
  }
  .row.idle:hover {
    background: var(--glass);
  }
  .devices {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 4px 12px 12px 32px;
  }
</style>
