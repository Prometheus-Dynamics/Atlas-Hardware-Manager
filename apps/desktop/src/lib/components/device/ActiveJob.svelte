<script lang="ts">
  // Shown instead of Flash or Update while this device is already in a job,
  // so a second job can't be started on top of the first.
  import { goto } from "$app/navigation";
  import type { DeviceJobState, JobId } from "$lib/api/client";
  import Button from "$lib/components/common/Button.svelte";
  import StageProgress from "$lib/components/jobs/StageProgress.svelte";
  import { isRecoveryPlan } from "$lib/present";
  import { ui } from "$lib/stores/ui.svelte";

  let { job, state }: { job: JobId; state: DeviceJobState } = $props();

  const verb = $derived(isRecoveryPlan(state.plan) ? "Flashing" : "Updating");

  function open() {
    ui.selectedJob = job;
    ui.close();
    void goto("/jobs");
  }
</script>

<div class="flex flex-col gap-4">
  <div class="flex items-center justify-between gap-3">
    <p class="text-[13.5px] text-fg">
      {verb} to <span class="mono">{state.release.version}</span>{state.status.status === "queued" ? ", waiting for its turn" : ""}.
    </p>
    <Button size="sm" icon="activity" onclick={open}>Open job #{job}</Button>
  </div>
  <StageProgress job={state} />
</div>
