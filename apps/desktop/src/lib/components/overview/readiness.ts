// One verdict for what the overview shows: is it good to go, and if not,
// the single most useful thing to do next.

import { keyString, type DeviceRecord, type RobotStatus } from "#lib/api/client.ts";
import { deviceName } from "#lib/format.ts";
import { aDevice, isRecovery } from "#lib/present.ts";
import type { IconName } from "#lib/ui/icons.ts";

export type Verdict = {
  tone: "ok" | "warn" | "err" | "accent" | "neutral";
  icon: IconName;
  title: string;
  detail: string;
  /** The next step, when there is one. */
  next?: "make-ready" | "update-all" | "flash" | "open-failed" | "scan";
};

export interface Facts {
  records: DeviceRecord[];
  robot: RobotStatus | null;
  failed: Set<string>;
  outdated: Set<string>;
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

function list(names: string[]): string {
  if (names.length <= 2) return names.join(" and ");
  return `${names.slice(0, 2).join(", ")} and ${names.length - 2} more`;
}

export function verdict({ records, robot, failed, outdated }: Facts): Verdict {
  const online = records.filter((r) => r.presence === "online");
  const broken = records.filter((r) => failed.has(keyString(r.key)));
  const waiting = online.filter(isRecovery);

  if (broken.length > 0) {
    return {
      tone: "err",
      icon: "alert-triangle",
      title: "Needs attention",
      detail: `${list(broken.map(deviceName))} didn't finish ${broken.length === 1 ? "its" : "their"} last update.`,
      next: "open-failed",
    };
  }

  if (robot) {
    switch (robot.state) {
      case "missing-devices": {
        const missing = robot.roles.filter((r) => r.device && r.presence !== "online").map((r) => r.device_name ?? r.role);
        return { tone: "err", icon: "plug-connected-x", title: "Devices missing", detail: `${list(missing)} ${missing.length === 1 ? "isn't" : "aren't"} connected.`, next: "scan" };
      }
      case "needs-update": {
        const stale = robot.roles.filter((r) => r.up_to_date === false);
        return { tone: "warn", icon: "arrow-up", title: "Needs updates", detail: `${plural(stale.length, "device")} behind the robot's target versions.`, next: "make-ready" };
      }
      case "unassigned": {
        const open = robot.roles.filter((r) => !r.device).map((r) => r.role);
        return { tone: "neutral", icon: "circle-dashed", title: "Roles to fill", detail: `No device for ${list(open)} yet.` };
      }
      case "ready":
        return { tone: "ok", icon: "circle-check", title: "Ready to compete", detail: `All ${plural(robot.roles.length, "role")} online and on target.` };
    }
  }

  if (records.length === 0) {
    return { tone: "neutral", icon: "radar-2", title: "Nothing connected yet", detail: "Plug in a device or join its network; Atlas finds it on its own.", next: "scan" };
  }
  if (waiting.length > 0) {
    return { tone: "accent", icon: "usb", title: "Ready to flash", detail: `${aDevice(waiting[0])} is in USB boot, waiting for an image.`, next: "flash" };
  }
  const behind = online.filter((r) => outdated.has(keyString(r.key)));
  if (behind.length > 0) {
    return { tone: "warn", icon: "arrow-up", title: "Updates available", detail: `${plural(behind.length, "device")} can be updated.`, next: "update-all" };
  }
  if (online.length === 0) {
    return { tone: "neutral", icon: "plug-connected-x", title: "Everything is offline", detail: `${plural(records.length, "known device")}, none connected right now.`, next: "scan" };
  }
  return {
    tone: "ok",
    icon: "circle-check",
    title: "All systems go",
    detail: `${plural(online.length, "device")} online${online.length < records.length ? `, ${records.length - online.length} offline` : ""}, all up to date.`,
  };
}
