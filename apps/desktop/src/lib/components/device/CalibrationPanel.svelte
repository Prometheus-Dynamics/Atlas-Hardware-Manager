<script lang="ts">
  // Calibrating a board's IMU or magnetometer through Lemnos (calibration.*
  // over Orion): how well each part is calibrated, the routines it takes
  // with what to do during each, the running one's progress, then Apply or
  // Discard the result; Reset goes back to the factory calibration. The
  // board keeps an applied calibration across restarts.
  import { api, errorText, type CalibrationRoutine, type CalibrationStatus, type DeviceKey, type HardwareDevice } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import ConfirmButton from "#lib/components/common/ConfirmButton.svelte";
  import ProgressBar from "#lib/components/common/ProgressBar.svelte";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { onDestroy } from "svelte";

  let { device, deviceKey }: { device: HardwareDevice; deviceKey: DeviceKey } = $props();

  const ROUTINES: Record<CalibrationRoutine, { label: string; how: string }> = {
    "accel-six": { label: "Six faces", how: "Rest the board still on each of its six faces in turn, a few seconds each." },
    "gyro-hold": { label: "Gyro at rest", how: "Leave the board completely still until it finishes." },
    "mag-rotate": { label: "Rotate", how: "Turn the board slowly through every direction, away from magnets and motors." },
  };
  const PART_NAMES = { accel: "Accelerometer", gyro: "Gyro", mag: "Magnetometer" } as const;
  const routines = $derived<CalibrationRoutine[]>(device.class === "imu" ? ["accel-six", "gyro-hold"] : ["mag-rotate"]);

  let status = $state<CalibrationStatus | null>(null);
  let error = $state<string | null>(null);
  let poll: ReturnType<typeof setTimeout> | null = null;

  async function read() {
    try {
      status = await api.calibrationStatus(deviceKey, device.id);
      error = null;
    } catch (e) {
      error = errorText(e);
    }
    // While a routine runs, follow it closely; otherwise look again now and then.
    if (poll) clearTimeout(poll);
    poll = setTimeout(read, status?.running ? 400 : 10_000);
  }
  void read();
  onDestroy(() => poll && clearTimeout(poll));

  async function run(step: "start" | "stop" | "apply" | "discard" | "reset", routine?: CalibrationRoutine) {
    try {
      await api.controlHardware(deviceKey, device.id, { command: "calibrate", step, routine });
      if (step === "apply") toasts.success(`${device.id}: calibration applied; the board keeps it.`);
      if (step === "reset") toasts.success(`${device.id}: back to its factory calibration.`);
    } catch (e) {
      toasts.error(errorText(e));
    }
    await read();
  }

  const parts = $derived(status ? (Object.entries(status.parts) as [keyof typeof PART_NAMES, NonNullable<CalibrationStatus["parts"]["accel"]>][]) : []);
  const tone = (c: number) => (c >= 0.8 ? "success" : c >= 0.5 ? "warning" : "error");
</script>

<section class="cal" aria-label="Calibration of {device.id}">
  <header class="flex items-center gap-2">
    <h4 class="text-[12px] font-medium uppercase tracking-[0.06em] text-fg-faint">Calibration</h4>
    {#if status && status.revision > 0}<span class="text-[11.5px] text-fg-faint">revision {status.revision}</span>{/if}
  </header>

  {#if error && !status}
    <p class="text-[12.5px] text-fg-muted">{error}</p>
  {:else if !status}
    <p class="text-[12.5px] text-fg-faint">Reading…</p>
  {:else}
    {#each parts as [name, part] (name)}
      <div class="part">
        <span class="name">{PART_NAMES[name] ?? name}</span>
        <span class="bar"><ProgressBar value={part.confidence} tone={part.active ? tone(part.confidence) : "neutral"} size={5} label="{name} confidence" /></span>
        <span class="val">{part.active ? `${Math.round(part.confidence * 100)}%` : "factory"}</span>
      </div>
    {/each}

    {#if status.running}
      <div class="running">
        <p class="text-[12.5px] text-fg">{ROUTINES[status.running]?.label ?? status.running}: {ROUTINES[status.running]?.how ?? ""}</p>
        <ProgressBar value={status.progress} size={6} label="Calibration progress" />
        <div class="flex justify-end"><Button size="sm" variant="ghost" icon="player-stop" action={() => run("stop")}>Stop</Button></div>
      </div>
    {:else if status.candidate}
      <div class="running">
        <p class="text-[12.5px] text-fg">A new calibration is ready. Apply it to use it now (the board keeps it), or discard it.</p>
        <div class="flex gap-2">
          <Button size="sm" variant="primary" icon="check" action={() => run("apply")}>Apply</Button>
          <Button size="sm" variant="ghost" icon="x" action={() => run("discard")}>Discard</Button>
        </div>
      </div>
    {:else}
      {#if status.failed}<p class="text-[12.5px] text-warn-fg">The last routine didn't finish well enough to use. Try it again.</p>{/if}
      <div class="flex flex-wrap items-center gap-2">
        {#each routines as routine (routine)}
          <Button size="sm" icon="player-play" title={ROUTINES[routine].how} action={() => run("start", routine)}>{ROUTINES[routine].label}</Button>
        {/each}
        {#if parts.some(([, p]) => p.active)}
          <span class="ml-auto">
            <ConfirmButton size="sm" variant="ghost" icon="refresh" prompt="Back to factory?" confirmLabel="Reset" action={() => run("reset")}>Reset</ConfirmButton>
          </span>
        {/if}
      </div>
    {/if}
  {/if}
</section>

<style>
  .cal {
    display: flex;
    flex-direction: column;
    gap: 8px;
    /* Bars at a readable length on a wide card. */
    max-width: 720px;
    margin-top: 12px;
    padding-top: 10px;
    border-top: 1px solid var(--hairline);
  }
  .part {
    display: grid;
    grid-template-columns: 7rem minmax(0, 1fr) 4rem;
    align-items: center;
    gap: 10px;
    font-size: 12.5px;
  }
  .name {
    color: var(--fg-muted);
  }
  .val {
    text-align: right;
    font-variant-numeric: tabular-nums;
    color: var(--fg);
  }
  .running {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    background: var(--inset);
    border: 1px solid var(--hairline);
    border-radius: var(--r-card);
  }
</style>
