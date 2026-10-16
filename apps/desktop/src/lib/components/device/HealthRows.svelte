<script lang="ts">
  // Is it OK, in one look: a row per reading (temperature, CPU, memory,
  // power and undervoltage, link, fan, uptime), its number and a one-line
  // trend. A click opens the reading's last five minutes over the panel;
  // nothing else moves.
  import { keyString, type DeviceRecord, type DeviceStatus } from "#lib/api/client.ts";
  import Sparkline from "#lib/components/common/Sparkline.svelte";
  import { linkText } from "#lib/format.ts";
  import { uptimeText } from "#lib/metrics.ts";
  import { clock } from "#lib/stores/clock.svelte.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { live } from "#lib/stores/live.svelte.ts";

  let {
    record,
    status = null,
    volts = null,
    amps = null,
    voltSeries = [],
  }: {
    record: DeviceRecord;
    status?: DeviceStatus | null;
    volts?: number | null;
    amps?: number | null;
    /** The supply's recent readings (the power monitor's stream). */
    voltSeries?: number[];
  } = $props();

  const id = $derived(keyString(record.key));
  type Tone = "ok" | "warn" | "err" | "none";
  interface Row {
    key: string;
    label: string;
    value: string;
    detail: string;
    tone: Tone;
    series: number[];
    color: string;
    unit: string;
  }

  const metric = (m: string) => live.metric(id, m);
  const tone = (m: string): Tone => {
    const r = metric(m);
    if (!r) return "none";
    if (r.max !== null && r.value >= r.max) return "err";
    if (r.warn_above !== null && r.value > r.warn_above) return "warn";
    return "ok";
  };

  /** The firmware's throttling flags: bit 0 undervoltage now, bit 16 since boot. */
  const under = $derived.by(() => {
    const flags = status?.boot?.throttled;
    if (!flags) return null;
    const n = Number.parseInt(flags, 16);
    if (!Number.isFinite(n)) return null;
    return { now: (n & 1) !== 0, since: (n & 0x10000) !== 0 };
  });

  const rows = $derived.by((): Row[] => {
    const list: Row[] = [];
    const temp = metric("temp");
    const hottest = status?.temperatures?.length ? Math.max(...status.temperatures.map((t) => t.celsius)) : null;
    if (temp || hottest !== null) {
      list.push({
        key: "temp",
        label: "Temperature",
        value: `${(temp?.value ?? hottest ?? 0).toFixed(1)} °C`,
        detail: temp?.warn_above ? `warns above ${temp.warn_above}` : "",
        tone: temp ? tone("temp") : "ok",
        series: live.series(id, "temp"),
        color: "var(--series-3)",
        unit: "°C",
      });
    }
    const cpu = metric("cpu");
    if (cpu) {
      list.push({ key: "cpu", label: "CPU", value: `${Math.round(cpu.value)}%`, detail: cpu.detail ?? "", tone: tone("cpu"), series: live.series(id, "cpu"), color: "var(--series-1)", unit: "%" });
    }
    const memory = metric("memory");
    if (memory) {
      list.push({ key: "memory", label: "Memory", value: `${Math.round(memory.value)}%`, detail: memory.detail ?? "", tone: tone("memory"), series: live.series(id, "memory"), color: "var(--series-2)", unit: "%" });
    }
    if (volts !== null || under) {
      list.push({
        key: "power",
        label: "Power",
        value: volts === null ? "—" : `${volts.toFixed(2)} V`,
        detail: [amps === null ? null : `${amps.toFixed(2)} A`, under ? (under.now ? "undervoltage now" : under.since ? "dipped since boot" : "no dips") : null]
          .filter(Boolean)
          .join(" · "),
        tone: under?.now ? "err" : under?.since ? "warn" : "ok",
        series: voltSeries,
        color: "var(--series-4)",
        unit: "V",
      });
    }
    const streamed = live.streamed(id);
    list.push({
      key: "link",
      label: "Link",
      value: linkText(record.link_kind, devices.nameOf),
      detail: streamed ? "pushed live" : "polled",
      tone: record.presence === "online" ? "ok" : "err",
      series: [],
      color: "",
      unit: "",
    });
    const fan = metric("fan");
    if (fan) {
      list.push({ key: "fan", label: "Fan", value: `${Math.round(fan.value)}${fan.unit === "%" ? "%" : ` ${fan.unit ?? ""}`}`, detail: fan.detail ?? "", tone: tone("fan"), series: live.series(id, "fan"), color: "var(--series-8)", unit: fan.unit ?? "" });
    }
    const up = metric("uptime");
    const boot = status?.boot;
    if (up || boot?.uptime_s) {
      list.push({
        key: "uptime",
        label: "Uptime",
        value: uptimeText(up?.value ?? boot?.uptime_s ?? 0),
        detail: [boot?.count ? `boot ${boot.count}` : null, boot?.previous_clean === false ? "last ended uncleanly" : boot?.previous_clean ? "clean" : null, status?.ntp_synchronized ? "clock synced" : null]
          .filter(Boolean)
          .join(" · "),
        tone: boot?.previous_clean === false ? "warn" : "ok",
        series: [],
        color: "",
        unit: "",
      });
    }
    return list;
  });

  const worst = $derived(rows.some((r) => r.tone === "err") ? "err" : rows.some((r) => r.tone === "warn") ? "warn" : "ok");
  let open = $state<string | null>(null);
  const opened = $derived(rows.find((r) => r.key === open && r.series.length > 1) ?? null);
  const recent = $derived(opened && opened.key !== "power" ? live.window(id, opened.key, clock.now - 5 * 60_000) : []);
</script>

<section class="panel health" aria-label="Health">
  <div class="panel-head">
    <h2 class="panel-title">Health</h2>
    <span class="text-[11.5px]" class:text-ok-fg={worst === "ok"} class:text-warn-fg={worst === "warn"} class:text-err-fg={worst === "err"}>
      {worst === "ok" ? "All OK" : worst === "warn" ? "Needs a look" : "Something's wrong"}
    </span>
  </div>
  {#each rows as row (row.key)}
    <button type="button" class="row" class:open={open === row.key} onclick={() => (open = open === row.key ? null : row.key)} disabled={row.series.length < 2} title={row.detail ? `${row.label}: ${row.value} · ${row.detail}` : `${row.label}: ${row.value}`}>
      <span class="dot {row.tone}"></span>
      <span class="label truncate">{row.label}</span>
      <span class="value mono truncate">{row.value}</span>
      {#if row.series.length > 1}
        <span class="spark"><Sparkline values={row.series} color={row.color} height={16} capacity={120} /></span>
      {:else}
        <span class="detail truncate">{row.detail}</span>
      {/if}
    </button>
  {/each}
  {#if opened}
    <div class="pop" role="dialog" aria-label="{opened.label}, last five minutes">
      <div class="flex items-center justify-between text-[12px]">
        <b class="truncate font-semibold">{opened.label} · {opened.value}{opened.detail ? ` · ${opened.detail}` : ""}</b>
        <button type="button" class="link text-[12px]" onclick={() => (open = null)}>Close</button>
      </div>
      <Sparkline
        values={recent.length > 1 ? recent.map((p) => p.v) : opened.series}
        times={recent.length > 1 ? recent.map((p) => p.t) : []}
        color={opened.color}
        height={110}
        capacity={Math.max(2, recent.length || opened.series.length)}
        format={(v) => `${v.toFixed(opened.unit === "V" ? 2 : 1)} ${opened.unit}`}
      />
      <span class="text-[11px] text-fg-faint">Last 5 minutes · hover for values</span>
    </div>
  {/if}
</section>

<style>
  .health {
    position: relative;
    gap: 0;
  }
  .row {
    display: grid;
    grid-template-columns: 8px 76px 84px minmax(0, 1fr);
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 26px;
    padding: 0 2px;
    border-bottom: 1px solid var(--hairline);
    font-size: 12.5px;
    text-align: left;
    color: var(--fg);
  }
  .row:last-of-type {
    border-bottom: none;
  }
  .row:not(:disabled):hover,
  .row.open {
    background: var(--glass-hover);
  }
  .row:disabled {
    cursor: default;
  }
  .label {
    color: var(--fg-muted);
  }
  .detail {
    font-size: 11.5px;
    color: var(--fg-faint);
  }
  .spark {
    min-width: 0;
    pointer-events: none;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #3a414c;
  }
  .dot.ok {
    background: var(--ok);
  }
  .dot.warn {
    background: var(--warn);
  }
  .dot.err {
    background: var(--err);
  }
  .pop {
    position: absolute;
    left: 6px;
    right: 6px;
    top: 34px;
    z-index: 20;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px;
    border: 1px solid var(--glass-border);
    border-radius: var(--r-panel);
    background: var(--glass);
    box-shadow: 0 10px 26px rgba(0, 0, 0, 0.55);
  }
</style>
