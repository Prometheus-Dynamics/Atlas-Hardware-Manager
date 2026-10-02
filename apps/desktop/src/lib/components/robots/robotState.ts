import type { RobotState } from "$lib/api/client";
import type { Tone } from "$lib/format";
import type { IconName } from "$lib/ui/icons";

export const ROBOT_STATE: Record<RobotState, { label: string; tone: Tone; icon: IconName }> = {
  ready: { label: "Ready", tone: "success", icon: "circle-check" },
  "needs-update": { label: "Needs updates", tone: "warning", icon: "arrow-up" },
  "missing-devices": { label: "Devices missing", tone: "error", icon: "plug-connected-x" },
  unassigned: { label: "Roles to fill", tone: "neutral", icon: "circle-dashed" },
};
