import type { RobotState } from "$lib/api/client";
import type { Tone } from "$lib/format";

export const ROBOT_STATE: Record<RobotState, { label: string; tone: Tone; icon: string }> = {
  ready: { label: "ready", tone: "success", icon: "fa-circle-check" },
  "needs-update": { label: "needs update", tone: "warning", icon: "fa-arrow-up-from-bracket" },
  "missing-devices": { label: "missing devices", tone: "error", icon: "fa-plug-circle-xmark" },
  unassigned: { label: "roles unassigned", tone: "neutral", icon: "fa-circle-question" },
};
