<script lang="ts">
  import type { JobRecord } from "$lib/api/client";
  import Tag from "$lib/components/common/Tag.svelte";
  import { clockTime } from "$lib/format";
  import { jobs } from "$lib/stores/jobs.svelte";
  import { ui } from "$lib/stores/ui.svelte";

  let { selectedId }: { selectedId: number | null } = $props();

  function problems(job: JobRecord): number {
    const s = job.summary;
    return s ? s.failed + s.rolled_back + s.needs_recovery : 0;
  }

  function stateTone(job: JobRecord) {
    if (job.state === "running") return "info" as const;
    if (job.state === "cancelled") return "neutral" as const;
    return problems(job) > 0 ? ("warning" as const) : ("success" as const);
  }
</script>

<ul class="flex flex-col gap-1" aria-label="Jobs">
  {#each jobs.sorted as job (job.id)}
    {@const s = job.summary}
    <li>
      <button
        type="button"
        class="w-full rounded-base border px-3 py-2 text-left transition-colors
          {selectedId === job.id ? 'border-primary-500/70 bg-primary-500/10' : 'border-surface-800 hover:border-surface-600'}"
        onclick={() => (ui.selectedJob = job.id)}
        aria-current={selectedId === job.id}
      >
        <div class="flex items-center justify-between gap-2">
          <span class="text-xs font-semibold text-surface-50">Job #{job.id}</span>
          <Tag tone={stateTone(job)} label={job.state} />
        </div>
        <p class="mt-0.5 text-[0.65rem] text-surface-400">
          {clockTime(job.created_ms)} · {job.devices.length} device{job.devices.length === 1 ? "" : "s"}
        </p>
        {#if s}
          <p class="mt-1 flex flex-wrap gap-x-2 text-[0.65rem]">
            <span class="text-success-400">{s.verified} verified</span>
            {#if s.rolled_back}<span class="text-warning-400">{s.rolled_back} rolled back</span>{/if}
            {#if s.needs_recovery}<span class="text-error-400">{s.needs_recovery} need recovery</span>{/if}
            {#if s.failed}<span class="text-error-400">{s.failed} failed</span>{/if}
            {#if s.skipped}<span class="text-surface-400">{s.skipped} skipped</span>{/if}
            {#if s.cancelled}<span class="text-surface-400">{s.cancelled} cancelled</span>{/if}
          </p>
        {/if}
      </button>
    </li>
  {/each}
</ul>
