<script lang="ts">
  // "Needs you": one compact bar. A chip per situation runs the action that
  // resolves it; "Details" lists them all in full, with dismiss. Chips that
  // don't fit on the one line are hidden (and skipped by Tab) and counted.
  import { goto } from "$app/navigation";
  import { api, keyString } from "#lib/api/client.ts";
  import { deviceName, jobStatusDetail, sentence } from "#lib/format.ts";
  import { aDevice } from "#lib/present.ts";
  import { insights } from "#lib/stores/insights.svelte.ts";
  import { system } from "#lib/stores/system.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import type { IconName } from "#lib/ui/icons.ts";
  import { errorText } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import { popover, softFade } from "#lib/ui/motion.ts";
  import Banner from "./Banner.svelte";

  interface Item {
    /** Stable identity for animation. */
    key: string;
    /** What a dismissal remembers; changes when the situation changes. */
    id: string;
    icon: IconName;
    tone: "accent" | "err" | "warn";
    title: string;
    /** The chip's label: a few words. */
    short: string;
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
        short: `${deviceName(record)} in USB boot`,
        text: "Pick an image to flash; every byte is checked afterwards.",
        action: "Flash",
        run: () => ui.openDevice(id, "software"),
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
        short: rolled ? `${deviceName(record)} rolled back` : `${deviceName(record)} failed`,
        text: detail ? sentence(detail) : "Open the job to see the log.",
        action: "See why",
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
        short: `${n} update${n === 1 ? "" : "s"} available`,
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
        short: check.label,
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

  const tone = $derived(items.some((i) => i.tone === "err") ? "err" : items.some((i) => i.tone === "accent") ? "accent" : "warn");

  async function act(item: Item) {
    try {
      await item.run();
    } catch (error) {
      toasts.error(errorText(error));
    }
  }

  // Which chips wrapped off the single visible line.
  let row: HTMLUListElement | undefined = $state();
  let hidden = $state(0);
  $effect(() => {
    void items.length;
    if (!row) return;
    const list = row;
    const measure = () => {
      const chips = [...list.children] as HTMLElement[];
      const top = chips[0]?.offsetTop ?? 0;
      let count = 0;
      for (const chip of chips) {
        const off = chip.offsetTop > top;
        chip.toggleAttribute("data-off", off);
        chip.querySelector("button")?.setAttribute("tabindex", off ? "-1" : "0");
        if (off) count++;
      }
      hidden = count;
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(list);
    return () => observer.disconnect();
  });

  let open = $state(false);
  let root: HTMLElement | undefined = $state();
  function onwindowclick(event: MouseEvent) {
    if (open && root && !root.contains(event.target as Node)) open = false;
  }
  $effect(() => {
    if (items.length === 0) open = false;
  });
</script>

<svelte:window onclick={onwindowclick} onkeydown={(e) => open && e.key === "Escape" && (open = false)} />

{#if items.length > 0}
  <div class="bar {tone}" bind:this={root} aria-label="Needs you" role="group" transition:softFade>
    <span class="count">
      <span class="num">{items.length}</span>
      need{items.length === 1 ? "s" : ""} you
    </span>
    <ul class="chips" bind:this={row}>
      {#each items as item (item.key)}
        <li>
          <button type="button" class="chip {item.tone}" title="{item.title}. {item.text}" onclick={() => act(item)}>
            <Icon name={item.icon} size={14} />
            <span class="truncate">{item.short}</span>
            <span class="verb">{item.action}</span>
          </button>
        </li>
      {/each}
    </ul>
    <div class="relative shrink-0">
      <Button variant="ghost" size="sm" iconRight={open ? "chevron-up" : "chevron-down"} onclick={() => (open = !open)}>
        {hidden > 0 ? `+${hidden} more` : "Details"}
      </Button>
      {#if open}
        <div class="pop glass-layer" transition:popover>
          <ul class="flex flex-col gap-2">
            {#each items as item (item.key)}
              <li>
                <Banner
                  icon={item.icon}
                  tone={item.tone}
                  title={item.title}
                  text={item.text}
                  action={item.action}
                  run={item.run}
                  ondismiss={item.dismissable ? () => ui.dismissed.add(item.id) : undefined}
                />
              </li>
            {/each}
          </ul>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .bar {
    --tone: var(--warn);
    position: relative;
    display: flex;
    flex-shrink: 0;
    align-items: center;
    gap: 12px;
    min-height: 46px;
    padding: 6px 8px 6px 16px;
    border-radius: var(--r-card);
    background: var(--glass);
    border: 1px solid var(--glass-border);
  }
  .bar.accent {
    --tone: var(--accent);
    background: linear-gradient(100deg, var(--accent-tint), var(--glass) 40%);
    border-color: var(--accent-tint-strong);
  }
  .bar.err {
    --tone: var(--err);
  }
  .bar::before {
    content: "";
    position: absolute;
    left: 0;
    top: 10px;
    bottom: 10px;
    width: 3px;
    border-radius: 0 3px 3px 0;
    background: var(--tone);
  }
  .count {
    display: inline-flex;
    flex-shrink: 0;
    align-items: baseline;
    gap: 6px;
    font-size: 13px;
    font-weight: 600;
    color: var(--fg);
    white-space: nowrap;
  }
  .num {
    font-size: 16px;
    font-variant-numeric: tabular-nums;
    color: var(--tone);
  }
  /* One line: chips that would wrap are hidden, not squeezed. */
  .chips {
    display: flex;
    flex: 1 1 auto;
    flex-wrap: wrap;
    gap: 6px;
    min-width: 0;
    height: 30px;
    overflow: hidden;
  }
  .chips > li {
    min-width: 0;
    max-width: 100%;
  }
  .chips > :global(li[data-off]) {
    visibility: hidden;
  }
  .chip {
    --c: var(--warn-fg);
    display: inline-flex;
    align-items: center;
    gap: 7px;
    max-width: 100%;
    height: 30px;
    padding: 0 10px;
    border-radius: var(--r-pill);
    font-size: 12.5px;
    color: var(--fg);
    background: var(--glass);
    border: 1px solid var(--glass-border);
    transition:
      background var(--t-fast),
      border-color var(--t-fast);
  }
  .chip:hover {
    background: var(--glass-hover);
    border-color: var(--glass-border-strong);
  }
  .chip :global(svg) {
    flex-shrink: 0;
    color: var(--c);
  }
  .chip.accent {
    --c: var(--accent-text);
  }
  .chip.err {
    --c: var(--err-fg);
  }
  .verb {
    flex-shrink: 0;
    font-weight: 600;
    color: var(--c);
  }
  .pop {
    position: absolute;
    top: calc(100% + 8px);
    right: 0;
    z-index: 35;
    width: min(520px, 80vw);
    max-height: min(60vh, 520px);
    overflow-y: auto;
    padding: 8px;
    border-radius: var(--r-panel);
    transform-origin: top right;
  }
</style>
