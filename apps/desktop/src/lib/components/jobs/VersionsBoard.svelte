<script lang="ts">
  // Where the robot's software stands: counts on top (up to date, behind,
  // waiting in USB boot, last job failed), then every device that takes
  // updates with the version it runs, the one it should, and how its last
  // update or flash ended. A row opens the device's Software tab.
  import type { Snippet } from "svelte";
  import { keyString, type ActivityEntry, type DeviceRecord } from "#lib/api/client.ts";
  import { deviceName, primaryVersion, timeAgo } from "#lib/format.ts";
  import { activity } from "#lib/stores/activity.svelte.ts";
  import { clock } from "#lib/stores/clock.svelte.ts";
  import { displayModel, isRecovery } from "#lib/present.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { insights } from "#lib/stores/insights.svelte.ts";
  import { canUpdate, NO_ROBOT, ui } from "#lib/stores/ui.svelte.ts";

  /** actions: what can start now (Flash, Update), in the header. */
  let { actions }: { actions?: Snippet } = $props();

  type State = "current" | "behind" | "usb-boot" | "failed" | "offline" | "unknown";

  // Each device's newest logged update or flash, for the devices this
  // session's jobs haven't touched.
  const logged = $derived.by(() => {
    const map = new Map<string, ActivityEntry>();
    for (const e of activity.entries) {
      if (!e.device || !(e.kind === "version-changed" || e.kind === "update-result" || e.kind === "mode-changed")) continue;
      const id = keyString(e.device);
      const known = map.get(id);
      if (!known || e.at_ms > known.at_ms) map.set(id, e);
    }
    return map;
  });
  const rows = $derived(
    devices.all
      .filter((d) => ui.robot === null || (ui.robot === NO_ROBOT ? !d.robot : d.robot === ui.robot))
      .filter((d) => canUpdate(d) || isRecovery(d) || primaryVersion(d.identity))
      .map((d) => {
        const id = keyString(d.key);
        const version = primaryVersion(d.identity);
        const target = insights.target(d);
        const outcome = insights.lastOutcome.get(id);
        return { record: d, id, version, target, outcome, state: stateOf(d, id, version, target) };
      })
      .sort((a, b) => RANK[a.state] - RANK[b.state] || deviceName(a.record).localeCompare(deviceName(b.record), undefined, { numeric: true })),
  );

  const RANK: Record<State, number> = { "usb-boot": 0, failed: 1, behind: 2, current: 3, unknown: 4, offline: 5 };
  const LABEL: Record<State, string> = {
    current: "Up to date",
    behind: "Behind",
    "usb-boot": "In USB boot",
    failed: "Last job failed",
    offline: "Offline",
    unknown: "No release known",
  };

  function stateOf(d: DeviceRecord, id: string, version: string | null, target: string | null): State {
    if (d.presence !== "online") return "offline";
    if (isRecovery(d)) return "usb-boot";
    if (insights.failedIds.has(id)) return "failed";
    if (insights.outdatedIds.has(id)) return "behind";
    if (!target || !version) return "unknown";
    return "current";
  }

  // Every state there is, so the counts add up to the devices listed.
  const counts = $derived(
    (["current", "behind", "usb-boot", "failed", "unknown", "offline"] as State[])
      .map((s) => ({ state: s, label: LABEL[s], n: rows.filter((r) => r.state === s).length }))
      .filter((c) => c.n > 0 || c.state === "current" || c.state === "behind"),
  );
  const total = $derived(Math.max(1, rows.length));
</script>

<section class="board" aria-label="Versions">
  <div class="head">
    <h2>Versions on the robot</h2>
    <span class="sub">{rows.length} devices that take updates</span>
    <span class="flex-1"></span>
    {#if actions}<div class="flex items-center gap-1.5">{@render actions()}</div>{/if}
  </div>

  <div class="summary">
    <div class="stack" role="img" aria-label={counts.map((c) => `${c.n} ${c.label.toLowerCase()}`).join(", ")}>
      {#each counts as c (c.state)}
        {#if c.n > 0}<span class="seg {c.state}" style:flex={c.n}></span>{/if}
      {/each}
    </div>
    <dl class="counts">
      {#each counts as c (c.state)}
        <div>
          <dt><span class="swatch {c.state}"></span>{c.label}</dt>
          <dd>{c.n} <span>{Math.round((c.n / total) * 100)}%</span></dd>
        </div>
      {/each}
    </dl>
  </div>

  <div class="list">
    <table>
      <thead>
        <tr><th scope="col">Device</th><th scope="col">Runs</th><th scope="col">Should run</th><th scope="col">State</th><th scope="col">Last update or flash</th></tr>
      </thead>
      <tbody>
        {#each rows as r (r.id)}
          <tr>
            <td>
              <button type="button" class="name" onclick={() => ui.openDevice(r.id, "software")}>
                <span class="font-medium">{deviceName(r.record)}</span><span class="muted">{displayModel(r.record)}</span>
              </button>
            </td>
            <td class="mono">{r.version ?? "—"}</td>
            <td class="mono" class:behind={r.state === "behind"}>{r.target ?? "—"}</td>
            <td><span class="chip {r.state}">{LABEL[r.state]}</span></td>
            <td class="muted">
              {#if r.outcome}{r.outcome.state.status.status.replaceAll("-", " ")}
              {:else if logged.get(r.id)}
                {@const e = logged.get(r.id)!}
                <span class="logged {e.level}" title={e.message}
                  >{e.kind === "mode-changed" ? "mode changed" : e.level === "error" ? "failed" : e.level === "warning" ? "rolled back" : "updated"}</span
                ><span class="when">{timeAgo(e.at_ms, clock.now)}</span>
              {:else}—{/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
</section>

<style>
  .board {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-height: 0;
    padding: 12px 14px;
    border-radius: var(--r-card);
    background: var(--glass);
    border: 1px solid var(--glass-border);
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  h2 {
    font-size: 13px;
    font-weight: 600;
  }
  .logged {
    color: var(--fg-muted);
  }
  .logged.error,
  .logged.warning {
    color: var(--warn-fg);
  }
  .when {
    margin-left: 6px;
    color: var(--fg-faint);
    white-space: nowrap;
  }
  .sub,
  .muted {
    font-size: 12px;
    color: var(--fg-muted);
  }
  .summary {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .stack {
    display: flex;
    gap: 2px;
    height: 12px;
  }
  .seg {
    border-radius: 1px;
  }
  .counts {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 10px;
  }
  dt {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11.5px;
    color: var(--fg-muted);
  }
  dd {
    font-size: 20px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  dd span {
    font-size: 12px;
    font-weight: 400;
    color: var(--fg-faint);
  }
  .swatch {
    width: 8px;
    height: 8px;
    border-radius: 1px;
  }
  .current {
    background: var(--ok);
  }
  .behind {
    background: var(--warn);
  }
  .usb-boot {
    background: var(--accent);
  }
  .failed {
    background: var(--err);
  }
  .seg.unknown,
  .swatch.unknown {
    background: var(--glass-border-strong);
  }
  .seg.offline,
  .swatch.offline {
    background: var(--offline);
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow: auto;
    margin: 0 -14px -12px;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
  }
  th {
    position: sticky;
    top: 0;
    background: var(--glass);
    text-align: left;
    font-size: 11px;
    font-weight: 500;
    color: var(--fg-faint);
    padding: 8px;
    border-bottom: 1px solid var(--hairline);
  }
  td {
    padding: 6px 8px;
    border-bottom: 1px solid var(--hairline);
    white-space: nowrap;
  }
  /* Long image names ("photonvision-raze-2026.1.0") don't push the table wide. */
  td.mono {
    max-width: 13rem;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  th:first-child,
  td:first-child {
    padding-left: 14px;
  }
  .name {
    display: flex;
    align-items: baseline;
    gap: 8px;
    color: var(--fg);
  }
  .name:hover {
    color: var(--accent-text);
  }
  .mono {
    font-family: var(--font-code);
    font-size: 12px;
  }
  td.behind {
    background: none;
    color: var(--warn-fg);
  }
  .chip {
    display: inline-block;
    padding: 1px 7px;
    border-radius: var(--r-sm);
    font-size: 11.5px;
  }
  .chip.current {
    background: var(--ok-bg);
    color: var(--ok-fg);
  }
  .chip.behind {
    background: var(--warn-bg);
    color: var(--warn-fg);
  }
  .chip.usb-boot,
  .chip.failed {
    background: var(--err-bg);
    color: var(--accent-text);
  }
  .chip.offline,
  .chip.unknown {
    background: var(--neutral-bg);
    color: var(--fg-muted);
  }
</style>
