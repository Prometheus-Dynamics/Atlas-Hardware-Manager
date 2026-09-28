<script lang="ts">
  import type { DeviceJobState } from "$lib/api/client";
  import ProgressBar from "$lib/components/common/ProgressBar.svelte";
  import Stepper from "$lib/components/common/Stepper.svelte";
  import Tag from "$lib/components/common/Tag.svelte";
  import { duration, jobStatusDetail, jobStatusLabel, jobStatusTone, overallFraction } from "$lib/format";
  import { clock } from "$lib/stores/clock.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";

  let { job }: { job: DeviceJobState } = $props();

  let showLog = $state(false);
  const tone = $derived(jobStatusTone(job.status));
  const detail = $derived(jobStatusDetail(job.status));
  const elapsed = $derived(
    job.started_ms ? (job.finished_ms ?? clock.now) - job.started_ms : null,
  );
  const barTone = $derived(tone === "error" ? "error" : tone === "warning" ? "warning" : tone === "success" ? "success" : "primary");

  async function copyLog() {
    try {
      await navigator.clipboard.writeText(job.log.join("\n"));
      toasts.success("Log copied.");
    } catch {
      toasts.error("Could not copy to the clipboard.");
    }
  }
</script>

<li class="rounded-base border border-surface-800 bg-surface-950/30 px-3 py-2">
  <div class="grid grid-cols-[minmax(8rem,1fr)_auto_auto] items-center gap-3">
    <div class="min-w-0">
      <p class="truncate text-xs font-semibold text-surface-50">
        {job.name}
        {#if job.canary}<span class="ml-1 text-[0.6rem] uppercase tracking-[0.12em] text-warning-300">first (staged)</span>{/if}
      </p>
      <p class="font-mono text-[0.65rem] text-surface-500">
        {job.device.family}:{job.device.serial} → {job.release.version}
      </p>
    </div>
    <Stepper steps={job.plan.steps} current={job.step} status={job.status} />
    <div class="flex items-center gap-2">
      {#if elapsed !== null}<span class="font-mono text-[0.65rem] text-surface-400">{duration(elapsed)}</span>{/if}
      <Tag {tone} label={jobStatusLabel(job.status)} />
    </div>
  </div>
  <div class="mt-2"><ProgressBar value={overallFraction(job)} tone={barTone} label="{job.name} progress" /></div>
  {#if detail}<p class="mt-1.5 text-xs {tone === 'success' ? 'text-surface-300' : 'text-surface-200'}">{detail}</p>{/if}

  <div class="mt-1.5 flex items-center gap-3">
    <button
      type="button"
      class="text-[0.65rem] uppercase tracking-[0.14em] text-surface-400 hover:text-surface-100"
      aria-expanded={showLog}
      onclick={() => (showLog = !showLog)}
    >
      <i class="fa-solid {showLog ? 'fa-chevron-down' : 'fa-chevron-right'} mr-1" aria-hidden="true"></i>Log ({job.log.length})
    </button>
    {#if showLog && job.log.length > 0}
      <button type="button" class="text-[0.65rem] uppercase tracking-[0.14em] text-surface-400 hover:text-surface-100" onclick={copyLog}>
        <i class="fa-solid fa-copy mr-1" aria-hidden="true"></i>Copy
      </button>
    {/if}
  </div>
  {#if showLog}
    <pre class="mt-1 max-h-48 overflow-auto rounded-base bg-surface-950 p-2 font-mono text-[0.65rem] leading-relaxed text-surface-300">{job.log.join("\n") || "No log lines yet."}</pre>
  {/if}
</li>
