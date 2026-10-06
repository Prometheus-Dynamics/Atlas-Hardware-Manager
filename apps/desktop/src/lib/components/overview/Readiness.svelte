<script lang="ts">
  // The verdict, big: is this robot good to go, and the one button that
  // gets it there.
  import { api, keyString, sameKey, type DeviceRecord } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import { isRecovery } from "#lib/present.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { insights } from "#lib/stores/insights.svelte.ts";
  import { robots } from "#lib/stores/robots.svelte.ts";
  import { system } from "#lib/stores/system.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import { softFade } from "#lib/ui/motion.ts";
  import { verdict } from "./readiness";

  let { records, robot }: { records: DeviceRecord[]; robot: string | null } = $props();

  const status = $derived(robot ? (robots.status(robot) ?? null) : null);
  const v = $derived(verdict({ records, robot: status, failed: insights.failedIds, outdated: insights.outdatedIds }));

  const NEXT = {
    "make-ready": { label: "Make ready", icon: "arrow-up" },
    "update-all": { label: "Update all", icon: "arrow-up" },
    flash: { label: "Flash it", icon: "bolt" },
    "open-failed": { label: "See what happened", icon: "history" },
    scan: { label: "Scan again", icon: "radar-2" },
  } as const;

  async function next() {
    switch (v.next) {
      case "make-ready": {
        if (!robot) return;
        const request = await api.robotUpdateRequest(robot, system.settings?.staged_default ?? null);
        if (request) ui.openUpdate(request, `Make ${robot} ready`);
        else toasts.success(`${robot} is already ready.`);
        return;
      }
      case "update-all": {
        const request = insights.updateAllRequest();
        if (!request) return;
        const devicesInView = request.devices.filter((key) => records.some((r) => sameKey(r.key, key)));
        ui.openUpdate({ ...request, devices: devicesInView }, "Update all");
        return;
      }
      case "flash": {
        const waiting = records.find((r) => r.presence === "online" && isRecovery(r));
        if (waiting) ui.openDevice(keyString(waiting.key), "flash");
        return;
      }
      case "open-failed": {
        const broken = records.find((r) => insights.failedIds.has(keyString(r.key)));
        if (broken) ui.openDevice(keyString(broken.key), "history");
        return;
      }
      case "scan":
        await devices.scanNow();
    }
  }
</script>

<section class="hero glass {v.tone}">
  <div class="mark">
    <Icon name={v.icon} size={30} stroke={1.7} />
  </div>
  {#key v.title + v.detail}
    <div class="min-w-0 flex-1" in:softFade>
      <h2 class="text-[24px] font-semibold tracking-[-0.02em] text-fg">{v.title}</h2>
      <p class="mt-0.5 text-[13.5px] text-fg-muted">{v.detail}</p>
    </div>
  {/key}
  {#if v.next}
    {@const step = NEXT[v.next]}
    <Button variant={v.next === "scan" ? "glass" : "primary"} size="lg" icon={step.icon} action={next} busy={v.next === "scan" && devices.scanning}>
      {step.label}
    </Button>
  {/if}
</section>

<style>
  .hero {
    --tone: var(--info);
    position: relative;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 16px 20px;
    padding: 22px 24px;
    overflow: hidden;
    border-radius: var(--r-panel);
  }
  .hero.ok {
    --tone: var(--ok);
  }
  .hero.warn {
    --tone: var(--warn);
  }
  .hero.err {
    --tone: var(--err);
  }
  .hero.accent {
    --tone: var(--accent);
  }
  .hero.neutral {
    --tone: var(--fg-faint);
  }
  /* A solid status edge: the one place the verdict colour lives. */
  .hero::before {
    content: "";
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 3px;
    background: var(--tone);
  }
  .mark {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 60px;
    height: 60px;
    flex-shrink: 0;
    border-radius: 50%;
    color: var(--tone);
    background: color-mix(in srgb, var(--tone) 14%, var(--layer-solid));
    border: 1px solid color-mix(in srgb, var(--tone) 40%, transparent);
  }
</style>
