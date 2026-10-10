<script lang="ts">
  import type { JobRecord } from "#lib/api/client.ts";
  import StatusDot from "#lib/components/common/StatusDot.svelte";
  import { timeAgo } from "#lib/format.ts";
  import { jobTitle } from "#lib/present.ts";
  import { clock } from "#lib/stores/clock.svelte.ts";
  import { jobs } from "#lib/stores/jobs.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import { rise } from "#lib/ui/motion.ts";

  let { selectedId }: { selectedId: number | null } = $props();

  function problems(job: JobRecord): number {
    const s = job.summary;
    return s ? s.failed + s.rolled_back + s.needs_recovery : 0;
  }

  function dot(job: JobRecord) {
    if (job.state === "running") return "busy" as const;
    if (problems(job) > 0) return "failed" as const;
    if (job.state === "cancelled") return "offline" as const;
    return "online" as const;
  }

  function line(job: JobRecord): string {
    const s = job.summary;
    if (job.state === "running") return "Running now";
    if (!s) return job.state;
    const parts = [`${s.verified} verified`];
    if (problems(job)) parts.push(`${problems(job)} didn't finish`);
    if (s.skipped) parts.push(`${s.skipped} skipped`);
    if (s.cancelled) parts.push(`${s.cancelled} cancelled`);
    return parts.join(" · ");
  }
</script>

<ul class="flex flex-col gap-1" aria-label="Jobs">
  {#each jobs.sorted as job (job.id)}
    <li in:rise>
      <button
        type="button"
        class="item"
        class:active={selectedId === job.id}
        onclick={() => (ui.selectedJob = job.id)}
        aria-current={selectedId === job.id}
      >
        <StatusDot state={dot(job)} label={job.state} />
        <span class="min-w-0 flex-1">
          <span class="block truncate text-[13px] font-medium text-fg">{jobTitle(job)}</span>
          <span class="block truncate text-[12px] text-fg-faint">#{job.id} · {timeAgo(job.created_ms, clock.now)} · {line(job)}</span>
        </span>
      </button>
    </li>
  {/each}
</ul>

<style>
  .item {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 10px 12px;
    border-radius: var(--r-card);
    text-align: left;
    border: 1px solid transparent;
    transition:
      background var(--t-fast),
      border-color var(--t-fast);
  }
  .item:hover {
    background: var(--glass);
  }
  .item.active {
    background: var(--glass-strong);
    border-color: var(--glass-border);
  }
</style>
