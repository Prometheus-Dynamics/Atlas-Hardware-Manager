<script lang="ts">
  // One robot, large: every role as the device filling it (live card) or an
  // empty slot, plus devices on the robot that aren't in a role yet.
  import { keyString, type RobotProfile, type RobotStatus } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import GlassCard from "#lib/components/common/GlassCard.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import Pill from "#lib/components/common/Pill.svelte";
  import MonitorChart from "#lib/components/device/MonitorChart.svelte";
  import DeviceCard from "#lib/components/inventory/DeviceCard.svelte";
  import { isCore, isTextMetric, sortMetrics } from "#lib/metrics.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { live, SLOW_MS, watchLive } from "#lib/stores/live.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import { makeReady, showDevices } from "./robotActions";
  import { ROBOT_STATE } from "./robotState";

  let { profile, status }: { profile: RobotProfile; status: RobotStatus | undefined } = $props();

  const badge = $derived(status ? ROBOT_STATE[status.state] : null);
  const roles = $derived(status?.roles ?? []);
  const loose = $derived((status?.unassigned_devices ?? []).map((key) => devices.get(key)).filter((r) => !!r));
  const members = $derived([...roles.map((r) => (r.device ? devices.get(r.device) : undefined)).filter((r) => !!r), ...loose]);

  $effect(() => watchLive(members, SLOW_MS));

  const title = (role: string) => role[0]?.toUpperCase() + role.slice(1);

  // The robot's readings side by side, a line per device: the first three
  // any of them reports (temperature, CPU, vision…).
  const COLORS = Array.from({ length: 8 }, (_, i) => `var(--series-${i + 1})`);
  const charted = $derived(members.filter((d) => d.presence === "online").map((record, i) => ({ record, color: COLORS[i % COLORS.length] })));
  const metrics = $derived.by(() => {
    const seen = new Map<string, import("#lib/api/client.ts").Metric>();
    for (const { record } of charted) {
      for (const m of live.metrics.get(keyString(record.key)) ?? []) if (!isCore(m) && !isTextMetric(m) && !seen.has(m.id)) seen.set(m.id, m);
    }
    return sortMetrics([...seen.values()]).slice(0, 3);
  });
</script>

<GlassCard fill large icon="robot" title={profile.name} subtitle={profile.notes ?? `${roles.length} role${roles.length === 1 ? "" : "s"}`} scroll={false} class="h-full">
  {#snippet actions()}
    {#if badge}<Pill tone={badge.tone} icon={badge.icon} label={badge.label} />{/if}
    {#if status && status.state !== "ready"}
      <Button variant="tint" size="sm" icon="wand" action={() => makeReady(profile.name)}>Make ready</Button>
    {/if}
    <Button size="sm" icon="layout-grid" onclick={() => showDevices(profile.name)}>Devices</Button>
    <Button variant="ghost" size="sm" icon="pencil" onclick={() => ui.openRobot(profile.name)}>Edit</Button>
  {/snippet}

  <!-- Scrolls itself, so the live charts can take the height that's left. -->
  <div class="absolute inset-0 flex flex-col gap-5 overflow-y-auto px-4 pb-3 pt-3">
    <section class="flex flex-col gap-2.5">
      <h3 class="section-title">Roles</h3>
      {#if roles.length === 0}
        <p class="text-[13px] text-fg-faint">No roles yet. Edit the robot to add some.</p>
      {:else}
        <ul class="auto-grid fit" style="--min: 230px; --max: 6">
          {#each roles as role, i (i)}
            {@const record = role.device ? devices.get(role.device) : undefined}
            <li class="flex min-w-0 flex-col gap-1.5">
              <div class="flex min-w-0 items-baseline justify-between gap-2 px-1 text-[12.5px]">
                <span class="truncate font-medium text-fg-muted" title={role.role}>{title(role.role)}</span>
                <span class="mono shrink-0 text-[11.5px]">
                  {#if role.up_to_date}
                    <span class="text-ok-fg">{role.version}</span>
                  {:else if role.version && role.target}
                    <span class="text-warn-fg">{role.version}</span><span class="text-fg-faint"> → {role.target}</span>
                  {:else}
                    <span class="text-fg-faint">{role.version ?? "—"}{role.target ? ` / ${role.target}` : ""}</span>
                  {/if}
                </span>
              </div>
              {#if record}
                <DeviceCard {record} />
              {:else}
                <div class="slot">
                  <Icon name={role.device ? "plug-connected-x" : "circle-dashed"} size={20} />
                  <p class="text-[13px] text-fg-muted">
                    {role.device ? `${role.device_name ?? keyString(role.device)} hasn't been seen` : `Empty · ${role.family}`}
                  </p>
                  {#if !role.device}
                    <button type="button" class="link text-[12.5px]" onclick={() => ui.openRobot(profile.name)}>Assign a device</button>
                  {/if}
                </div>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    {#if loose.length > 0}
      <section class="flex flex-col gap-2.5">
        <h3 class="section-title">On this robot, not in a role</h3>
        <ul class="auto-grid" style="--min: 230px">
          {#each loose as record (keyString(record.key))}
            <li><DeviceCard {record} /></li>
          {/each}
        </ul>
      </section>
    {/if}

    {#if metrics.length > 0}
      <section class="live" aria-label="Live readings">
        {#each metrics as m (m.id)}
          <MonitorChart metric={m.id} title={m.label} unit={m.unit ?? ""} devices={charted} windowMs={600_000} />
        {/each}
      </section>
    {/if}
  </div>
</GlassCard>

<style>
  /* The rest of the height: its readings over the last ten minutes. */
  .live {
    flex: 1 1 220px;
    min-height: 220px;
    display: flex;
    gap: 10px;
  }
  .live > :global(*) {
    flex: 1 1 0;
  }
  .slot {
    display: flex;
    flex: 1;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    min-height: 168px;
    padding: 16px;
    text-align: center;
    border-radius: var(--r-card);
    border: 1px dashed var(--glass-border-strong);
    color: var(--fg-faint);
  }
</style>
