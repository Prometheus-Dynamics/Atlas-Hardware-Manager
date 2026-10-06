<script lang="ts">
  // "Needs you": one plain-language banner per situation, each with the
  // action that resolves it.
  import { goto } from "$app/navigation";
  import { api, keyString } from "#lib/api/client.ts";
  import { deviceName, jobStatusDetail, sentence } from "#lib/format.ts";
  import { aDevice } from "#lib/present.ts";
  import { insights } from "#lib/stores/insights.svelte.ts";
  import { system } from "#lib/stores/system.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import type { IconName } from "#lib/ui/icons.ts";
  import { DUR, ease, ms, rise, softFade } from "#lib/ui/motion.ts";
  import { flip } from "svelte/animate";
  import Banner from "./Banner.svelte";

  interface Item {
    /** Stable identity for animation. */
    key: string;
    /** What a dismissal remembers; changes when the situation changes. */
    id: string;
    icon: IconName;
    tone: "accent" | "err" | "warn";
    title: string;
    text: string;
    action: string;
    run: () => unknown;
    dismissable: boolean;
  }

  const items = $derived.by(() => {
    const list: Item[] = [];
    for (const record of insights.waiting) {
      const id = keyString(record.key);
      list.push({
        key: `wait:${id}`,
        id: `wait:${id}`,
        icon: "usb",
        tone: "accent",
        title: `${aDevice(record)} is waiting in USB boot`,
        text: "Pick an image and Atlas will flash it and check every byte.",
        action: "Flash",
        run: () => ui.openDevice(id, "flash"),
        dismissable: false,
      });
    }
    for (const record of insights.failed) {
      const id = keyString(record.key);
      const outcome = insights.lastOutcome.get(id);
      if (!outcome) continue;
      const detail = jobStatusDetail(outcome.state.status);
      const rolled = outcome.state.status.status === "rolled-back";
      list.push({
        key: `fail:${id}`,
        id: `fail:${outcome.job}:${id}`,
        icon: "alert-circle",
        tone: "err",
        title: rolled ? `${deviceName(record)} went back to its old version` : `${deviceName(record)} didn't finish updating`,
        text: detail ? sentence(detail) : "Open the job to see the log.",
        action: "See what happened",
        run: () => {
          ui.selectedJob = outcome.job;
          void goto("/jobs");
        },
        dismissable: true,
      });
    }
    const outdated = insights.outdated;
    if (outdated.length > 0) {
      const n = outdated.length;
      list.push({
        key: "outdated",
        id: `outdated:${outdated.map((d) => keyString(d.key)).join(",")}`,
        icon: "arrow-up",
        tone: "warn",
        title: `${n} device${n === 1 ? " has an update" : "s have updates"} available`,
        text: outdated
          .slice(0, 3)
          .map((d) => deviceName(d))
          .join(", ") + (n > 3 ? ` and ${n - 3} more` : "") + ". Select them to review first, or use Update all.",
        action: "Select them",
        run: () => {
          ui.selection.clear();
          outdated.forEach((d) => ui.selection.add(keyString(d.key)));
        },
        dismissable: true,
      });
    }
    for (const check of insights.fixable) {
      list.push({
        key: `health:${check.id}`,
        id: `health:${check.id}`,
        icon: "tool",
        tone: "warn",
        title: check.label,
        text: check.fix ?? check.detail,
        action: "Fix",
        run: async () => {
          toasts.success(await api.fixHealth(check.fix_action!));
          await system.checkHealth();
        },
        dismissable: true,
      });
    }
    return list.filter((item) => !ui.dismissed.has(item.id));
  });
</script>

{#if items.length > 0}
  <div class="flex flex-col gap-2" aria-label="Needs you">
    {#each items as item (item.key)}
      <div animate:flip={{ duration: ms(DUR.enter), easing: ease }} in:rise out:softFade>
        <Banner
          icon={item.icon}
          tone={item.tone}
          title={item.title}
          text={item.text}
          action={item.action}
          run={item.run}
          ondismiss={item.dismissable ? () => ui.dismissed.add(item.id) : undefined}
        />
      </div>
    {/each}
  </div>
{/if}
