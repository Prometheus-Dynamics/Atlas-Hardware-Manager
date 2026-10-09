// How live readings are shown. Known metric ids get an icon, a gauge, and a
// place in robot summaries; any other metric a device reports still shows,
// as a plain value with its own label and unit.

import type { Metric } from "#lib/api/client.ts";
import type { IconName } from "#lib/ui/icons.ts";

const ICON: Record<string, IconName> = {
  cpu: "cpu",
  temp: "temperature",
  fan: "propeller",
  memory: "database",
  fps: "eye",
  uptime: "clock",
  voltage: "battery-charging",
  current: "bolt",
};

/** Display order: the ones people look at first. */
const ORDER = ["temp", "cpu", "fan", "fps", "memory", "voltage", "current"];

export function metricIcon(metric: Metric): IconName {
  return ICON[metric.id] ?? "gauge";
}

export type MetricTone = "ok" | "warn" | "err" | "neutral";

export function metricTone(metric: Metric): MetricTone {
  if (metric.warn_above === null) return "neutral";
  if (metric.max !== null && metric.value >= metric.max) return "err";
  return metric.value > metric.warn_above ? "warn" : "ok";
}

/** 0 to 1 within the metric's range, or null without one. */
export function metricFraction(metric: Metric): number | null {
  if (metric.max === null || metric.max <= 0) return null;
  return Math.min(1, Math.max(0, metric.value / metric.max));
}

/** "3h 17m", "2d 4h", "42s". */
export function uptimeText(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${s % 60}s`;
  return `${s}s`;
}

/** Decimal places that suit the size of a reading: 1234, 54.5, 3.21. */
export function metricDecimals(metric: Metric): number {
  if (Number.isInteger(metric.value)) return 0;
  const abs = Math.abs(metric.value);
  return abs >= 100 ? 0 : abs >= 10 ? 1 : 2;
}

/** Uptime reads better as text; everything else is a number that can glide. */
export function isTextMetric(metric: Metric): boolean {
  return metric.id === "uptime" && (metric.unit === "s" || metric.unit === null);
}

/** The value with its unit, formatted for reading at a glance. */
export function metricValue(metric: Metric): { value: string; unit: string } {
  if (metric.id === "uptime" && (metric.unit === "s" || metric.unit === null)) {
    return { value: uptimeText(metric.value), unit: "" };
  }
  const abs = Math.abs(metric.value);
  const digits = abs >= 100 ? 0 : abs >= 10 ? 1 : 2;
  const text =
    abs >= 1000
      ? metric.value.toLocaleString(undefined, { maximumFractionDigits: 0 })
      : metric.value.toFixed(Number.isInteger(metric.value) ? 0 : digits).replace(/\.0+$/, "");
  return { value: text, unit: metric.unit ?? "" };
}

/** Milliseconds per frame at `fps`: "16.9 ms". */
export const frameTime = (fps: number) => `${(1000 / fps).toFixed(1)} ms`;

/**
 * The line under a reading: the device's own raw numbers, or for a frame
 * rate the time per frame it means.
 */
export function metricDetail(metric: Metric): string | null {
  if (metric.detail) return metric.detail;
  if (metric.id === "fps" && metric.value > 0) return `${frameTime(metric.value)} per frame`;
  return null;
}

/** Per-core CPU readings (`cpu.core.<n>`), shown inside the CPU tile. */
export const CORE_PREFIX = "cpu.core.";
export const isCore = (metric: Metric) => metric.id.startsWith(CORE_PREFIX);

/** The cores in kernel order. */
export function coreMetrics(metrics: Metric[]): Metric[] {
  const index = (m: Metric) => Number(m.id.slice(CORE_PREFIX.length));
  return metrics.filter(isCore).sort((a, b) => index(a) - index(b));
}

export function sortMetrics(metrics: Metric[]): Metric[] {
  const rank = (m: Metric) => {
    const i = ORDER.indexOf(m.id);
    return i < 0 ? ORDER.length : i;
  };
  return [...metrics].sort((a, b) => rank(a) - rank(b) || a.label.localeCompare(b.label));
}

/** The colour a tone draws with. */
export function toneColor(tone: MetricTone): string {
  switch (tone) {
    case "ok":
      return "var(--ok)";
    case "warn":
      return "var(--warn)";
    case "err":
      return "var(--err)";
    case "neutral":
      return "var(--info)";
  }
}
