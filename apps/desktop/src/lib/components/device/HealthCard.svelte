<script lang="ts">
  // A compact health block from the board's status: failed services,
  // temperature and fan, the last boot, the clock, and drift (a count, with
  // the files on expand).
  import type { DeviceStatus, DriftItem } from "#lib/api/client.ts";
  import Icon from "#lib/components/common/Icon.svelte";
  import { timeAgo } from "#lib/format.ts";
  import { clock } from "#lib/stores/clock.svelte.ts";
  import type { IconName } from "#lib/ui/icons.ts";
  import Disclosure from "./Disclosure.svelte";

  let { status, readAt }: { status: DeviceStatus; readAt: number | undefined } = $props();

  type Look = "ok" | "warn" | "err" | "plain";
  interface Tile {
    id: string;
    icon: IconName;
    label: string;
    value: string;
    detail: string | null;
    look: Look;
    title?: string;
  }

  function uptime(s: number): string {
    if (s < 3600) return `${Math.max(1, Math.round(s / 60))}m`;
    if (s < 48 * 3600) return `${Math.floor(s / 3600)}h ${Math.round((s % 3600) / 60)}m`;
    return `${Math.round(s / 86400)}d`;
  }

  function offset(s: number): string {
    const a = Math.abs(s);
    const amount = a < 120 ? `${a}s` : a < 7200 ? `${Math.round(a / 60)} min` : a < 172800 ? `${Math.round(a / 3600)} h` : `${Math.round(a / 86400)} days`;
    return `${amount} ${s < 0 ? "behind" : "ahead"}`;
  }

  const FLAGS: Record<string, string> = {
    cmdline: "kernel command line",
    config: "config.txt",
    sshd_config: "SSH server config",
    update_env: "update settings",
  };
  const CHANGES: Record<string, string> = { changed: "changed", added: "added", missing: "missing", present: "override" };

  const tiles = $derived.by((): Tile[] => {
    const list: Tile[] = [];
    const failed = status.failed_units;
    list.push(
      failed === null
        ? {
            id: "services",
            icon: "list-check",
            label: "Services",
            value: "Unknown",
            detail: "the board couldn't check",
            look: "plain",
          }
        : {
            id: "services",
            icon: "list-check",
            label: "Services",
            value: failed.length ? `${failed.length} failed` : "All running",
            detail: failed.length ? failed.join(", ") : null,
            look: failed.length ? "err" : "ok",
            title: failed.join("\n") || undefined,
          },
    );
    const temp = status.temperatures.find((t) => t.id.includes("cpu")) ?? status.temperatures[0];
    const fan = status.fan;
    const fanText = fan?.rpm != null ? `fan ${fan.rpm} rpm` : fan?.pwm != null ? `fan ${Math.round((fan.pwm * 100) / 255)}%` : null;
    if (temp || fanText) {
      list.push({
        id: "thermal",
        icon: "temperature",
        label: "Temperature",
        value: temp ? `${temp.celsius.toFixed(1)} °C` : "—",
        detail: [fanText, fan?.state != null && fan.max_state != null ? `level ${fan.state} of ${fan.max_state}` : null]
          .filter(Boolean)
          .join(" · ") || null,
        look: temp && temp.celsius > 75 ? "warn" : "plain",
      });
    }
    const boot = status.boot;
    if (boot) {
      const unclean = boot.previous_clean === false;
      list.push({
        id: "boot",
        icon: "refresh",
        label: "Last boot",
        value: [boot.count != null ? `#${boot.count}` : null, unclean ? "after a crash or power cut" : boot.previous_clean ? "clean" : null]
          .filter(Boolean)
          .join(" · ") || "—",
        detail: [boot.slot ? `slot ${boot.slot}` : null, boot.uptime_s != null ? `up ${uptime(boot.uptime_s)}` : null, boot.kernel]
          .filter(Boolean)
          .join(" · ") || null,
        look: unclean ? "warn" : "plain",
        title: unclean ? "The boot before this one didn't shut down cleanly (power cut, watchdog reset or crash)." : undefined,
      });
    }
    if (status.clock_offset_s != null) {
      const off = Math.abs(status.clock_offset_s) > 2;
      list.push({
        id: "clock",
        icon: "clock",
        label: "Clock",
        value: off ? offset(status.clock_offset_s) : "In step",
        detail: status.ntp_synchronized === true ? "NTP synchronized" : status.ntp_synchronized === false ? "no NTP" : null,
        look: off ? "warn" : "ok",
      });
    }
    const drift = status.drift;
    if (drift) {
      list.push({
        id: "drift",
        icon: "shield-check",
        label: "Drift",
        value: drift.count ? `${drift.count} change${drift.count === 1 ? "" : "s"}` : "None",
        detail: drift.flags.length ? drift.flags.map((f) => FLAGS[f] ?? f).join(", ") : drift.root_read_only ? "read-only root" : null,
        look: drift.flags.length ? "warn" : drift.count ? "plain" : "ok",
      });
    }
    return list;
  });

  const driftItems = $derived(status.drift?.items ?? []);
  const shortPath = (item: DriftItem) => (item.area === "boot" ? `boot/${item.path}` : item.path);
</script>

<section class="flex flex-col gap-2.5">
  <h3 class="flex items-center justify-between text-[12px] font-medium uppercase tracking-[0.06em] text-fg-faint">
    <span>Health</span>
    {#if readAt}<span class="normal-case tracking-normal">{timeAgo(readAt, clock.now)}</span>{/if}
  </h3>
  <ul class="auto-grid" style="--min: 160px; --gap: 8px">
    {#each tiles as tile (tile.id)}
      <li class="glass min-w-0 px-3.5 py-2.5" title={tile.title}>
        <p class="flex items-center gap-1.5 text-[12px] text-fg-faint">
          <Icon name={tile.icon} size={13} />{tile.label}
        </p>
        <p class="mt-0.5 truncate text-[13.5px] font-medium {tile.look}">{tile.value}</p>
        {#if tile.detail}<p class="truncate text-[12px] text-fg-muted" title={tile.detail}>{tile.detail}</p>{/if}
      </li>
    {/each}
  </ul>
  {#if driftItems.length > 0}
    <Disclosure title="Drift and overrides" count={driftItems.length}>
      <ul class="flex flex-col gap-1.5 text-[12.5px]">
        {#each driftItems as item (`${item.area}:${item.path}`)}
          <li class="flex items-baseline gap-2">
            <span class="w-16 shrink-0 text-fg-faint">{CHANGES[item.change] ?? item.change}</span>
            <span class="mono min-w-0 flex-1 truncate text-fg" title={item.sha256 ?? undefined}>{shortPath(item)}</span>
          </li>
        {/each}
      </ul>
      {#if status.drift?.checked_at}
        <p class="mt-2 text-[12px] text-fg-faint">
          Checked {timeAgo(status.drift.checked_at * 1000, clock.now)} against {status.drift.baseline === "stage"
            ? "the files the last update installed"
            : "what the board first saw"}.
        </p>
      {/if}
    </Disclosure>
  {/if}
</section>

<style>
  .ok {
    color: var(--ok-fg);
  }
  .warn {
    color: var(--warn-fg);
  }
  .err {
    color: var(--err-fg);
  }
  .plain {
    color: var(--fg);
  }
</style>
