// Small display helpers shared by every screen.

import type { DeviceJobState, DeviceJobStatus, DeviceRecord, Identity, LinkKind } from "#lib/api/client.ts";
import { keyString } from "#lib/api/client.ts";

export function deviceName(record: DeviceRecord): string {
  return record.label ?? record.identity.name ?? keyString(record.key);
}

export function primaryVersion(identity: Identity): string | null {
  const v = identity.versions;
  return v.os ?? v.firmware ?? v.bootloader ?? Object.values(v)[0] ?? null;
}

export function linkText(link: LinkKind, nameOf: (key: string) => string): string {
  switch (link.kind) {
    case "usb-network":
      return "USB network";
    case "usb-serial":
      return "USB serial";
    case "usb-boot":
      return "USB boot";
    case "ethernet":
      return "Ethernet";
    case "simulated":
      return "Simulated";
    case "gateway":
      return `via ${nameOf(keyString(link.via))}`;
  }
}

export function timeAgo(ms: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 5) return "just now";
  if (s < 60) return `${s}s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 48) return `${h}h ago`;
  return `${Math.round(h / 24)}d ago`;
}

export function clockTime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function duration(ms: number): string {
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, "0")}s`;
}

export function bytes(n: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = n;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit++;
  }
  return `${value < 10 && unit > 0 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

export type Tone = "success" | "warning" | "error" | "primary" | "neutral" | "info";

export function jobStatusTone(status: DeviceJobStatus): Tone {
  switch (status.status) {
    case "verified":
      return "success";
    case "rolled-back":
      return "warning";
    case "needs-recovery":
    case "failed":
      return "error";
    case "running":
      return "primary";
    default:
      return "neutral";
  }
}

export function jobStatusLabel(status: DeviceJobStatus): string {
  const text = status.status.replace("-", " ");
  return text[0].toUpperCase() + text.slice(1);
}

/** The sentence attached to a finished status, if any. */
export function jobStatusDetail(status: DeviceJobStatus): string | null {
  switch (status.status) {
    case "verified":
      return `Now on ${status.version}`;
    case "rolled-back":
    case "needs-recovery":
    case "skipped":
      return status.reason;
    case "failed":
      return status.error;
    default:
      return null;
  }
}

/** Newest first by dotted numeric parts, falling back to string order. */
export function compareVersionsDesc(a: string, b: string): number {
  const pa = a.split(/[.\-+]/);
  const pb = b.split(/[.\-+]/);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const x = pa[i] ?? "";
    const y = pb[i] ?? "";
    const nx = Number(x);
    const ny = Number(y);
    if (!Number.isNaN(nx) && !Number.isNaN(ny) && x !== "" && y !== "") {
      if (nx !== ny) return ny - nx;
    } else if (x !== y) {
      // A pre-release suffix sorts below the plain version.
      if (x === "") return -1;
      if (y === "") return 1;
      return y.localeCompare(x);
    }
  }
  return 0;
}

/** Progress across all planned steps, 0 to 1. */
export function overallFraction(state: DeviceJobState): number {
  if (state.status.status === "verified") return 1;
  const steps = state.plan.steps;
  const index = state.step ? steps.indexOf(state.step) : -1;
  if (index < 0 || steps.length === 0) return 0;
  return Math.min(1, (index + state.fraction) / steps.length);
}

/** Backend errors are lowercase clauses; show them as sentences. */
export function sentence(text: string): string {
  const trimmed = text.trim();
  if (!trimmed) return trimmed;
  const first = trimmed[0].toUpperCase() + trimmed.slice(1);
  return /[.!?]$/.test(first) ? first : `${first}.`;
}
