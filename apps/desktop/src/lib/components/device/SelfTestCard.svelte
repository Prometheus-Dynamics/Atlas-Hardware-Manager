<script lang="ts">
  // The board's last self-test: each check with what it found, and a button
  // to run it again. Atlas also runs it by itself when a board comes back
  // from a flash or an update; a failure is shown here, never blocking.
  import { keyString, type CheckStatus, type DeviceRecord } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import Pill from "#lib/components/common/Pill.svelte";
  import { timeAgo, type Tone } from "#lib/format.ts";
  import { clock } from "#lib/stores/clock.svelte.ts";
  import { canSelftest, selftests } from "#lib/stores/selftests.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import type { IconName } from "#lib/ui/icons.ts";

  let { record }: { record: DeviceRecord } = $props();

  const NAMES: Record<string, string> = {
    leds: "LED ring",
    fan: "Fan",
    camera: "Camera",
    i2c: "Sensors",
    watchdog: "Watchdog",
    gadget: "USB link",
  };
  const ICONS: Record<CheckStatus, { icon: IconName; color: string; label: string }> = {
    ok: { icon: "circle-check", color: "text-ok-fg", label: "Passed" },
    fail: { icon: "circle-x", color: "text-err-fg", label: "Failed" },
    skip: { icon: "circle-dashed", color: "text-fg-faint", label: "Skipped" },
    unknown: { icon: "info-circle", color: "text-fg-faint", label: "Unknown" },
  };

  const last = $derived(selftests.forDevice(record));
  const can = $derived(canSelftest(record));
  const running = $derived(selftests.running.has(keyString(record.key)));

  const verdict = $derived.by((): { tone: Tone; icon: IconName; label: string } | null => {
    if (!last) return null;
    if (!last.report) return { tone: "warning", icon: "alert-triangle", label: "Couldn't run" };
    if (last.report.ok) return { tone: "success", icon: "circle-check", label: "Passed" };
    const failed = last.report.checks.filter((c) => c.status === "fail").length;
    return { tone: "error", icon: "alert-circle", label: `${failed} failed` };
  });

  const when = $derived.by(() => {
    if (!last) return "";
    const parts = [timeAgo(last.at_ms, clock.now)];
    if (last.trigger === "after-update") parts.push("after its update");
    if (last.report?.package_version) parts.push(`package ${last.report.package_version}`);
    return parts.join(" · ");
  });

  $effect(() => {
    void selftests.load(record);
  });

  async function run() {
    const result = await selftests.run(record);
    if (result.report?.ok) toasts.success("Self-test passed.");
    else if (result.report) toasts.error("The self-test found a problem; see the checks.");
    else toasts.error(`The self-test couldn't run: ${result.error ?? "no report"}`);
  }
</script>

{#if can || last}
  <section class="flex flex-col gap-2.5">
    <div class="flex items-center justify-between gap-3">
      <h3 class="text-[12px] font-medium uppercase tracking-[0.06em] text-fg-faint">Self-test</h3>
      {#if can}
        <Button size="sm" icon="list-check" action={run} busy={running}>{last ? "Run again" : "Run self-test"}</Button>
      {/if}
    </div>

    {#if !last}
      <p class="text-[13px] text-fg-muted">
        Not run yet. It checks the LED ring, fan, camera, sensors, watchdog and USB link, and puts the fan and LEDs back
        as they were. Atlas also runs it when the board comes back from a flash or an update.
      </p>
    {:else}
      <div class="glass flex flex-col gap-3 px-4 py-3.5">
        <div class="flex flex-wrap items-center gap-2">
          {#if running}
            <Pill tone="info" icon="refresh" spin label="Running on the board" />
          {:else if verdict}
            <Pill tone={verdict.tone} icon={verdict.icon} label={verdict.label} />
          {/if}
          <span class="text-[12px] text-fg-faint">{when}</span>
        </div>
        {#if last.error}
          <p class="flex items-start gap-2 text-[13px] text-warn-fg">
            <Icon name="alert-triangle" size={15} class="mt-0.5 shrink-0" />{last.error}
          </p>
        {/if}
        {#if last.report}
          <ul class="flex flex-col gap-2">
            {#each last.report.checks as check (check.id)}
              {@const look = ICONS[check.status] ?? ICONS.unknown}
              <li class="flex items-start gap-2.5 text-[13px]">
                <span class="mt-0.5 shrink-0 {look.color}" title={look.label}><Icon name={look.icon} size={16} /></span>
                <span class="w-20 shrink-0 font-medium text-fg">{NAMES[check.id] ?? check.id}</span>
                <span class="min-w-0 text-fg-muted">{check.message}</span>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    {/if}
  </section>
{/if}
