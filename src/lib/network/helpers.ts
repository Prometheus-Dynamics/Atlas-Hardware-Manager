import type { InstallMode, UpdaterStep, UpdaterStepStatus } from "./models";

export function normalizeOtaStageForUi(value: string | null | undefined): string {
  const normalized = (value ?? "").trim().toLowerCase();
  if (!normalized) {
    return "unknown";
  }
  return normalized.replace(/\s+/g, "_");
}

export function otaStageProgressHintForUi(stage: string): number | null {
  if (stage === "downloading") return 15;
  if (stage === "verifying") return 35;
  if (stage === "awaiting_window") return 55;
  if (stage === "applying" || stage === "installing") return 75;
  if (stage === "committing") return 88;
  if (stage === "finalizing") return 94;
  if (stage === "pending_reboot" || stage === "pending-reboot") return 97;
  if (stage === "rebooting" || stage === "complete") return 100;
  return null;
}

export function generateInstallRunId(): string {
  return typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
    ? crypto.randomUUID()
    : `${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

export function buildStepTemplate(mode: InstallMode): UpdaterStep[] {
  if (mode === "ota") {
    return [
      { key: "resolve-image", label: "Resolve Image", detail: "Download/select update image.", status: "pending" },
      { key: "resolve-target", label: "Resolve Target", detail: "Resolve OTA API endpoint for the selected device.", status: "pending" },
      { key: "ota-upload", label: "Upload OTA", detail: "Upload image to OTA endpoint.", status: "pending" },
      { key: "ota-apply", label: "Apply OTA", detail: "Trigger and track device-side OTA apply stages.", status: "pending" },
      { key: "ota-monitor", label: "Track OTA State", detail: "Read OTA stage/progress from device state.", status: "pending" },
      { key: "ota-reconnect", label: "Wait Reconnect", detail: "Wait for reboot and online reconnect.", status: "pending" },
      { key: "complete", label: "Complete", detail: "OTA workflow finished.", status: "pending" },
    ];
  }

  if (mode === "mount") {
    return [
      { key: "scan-targets", label: "Scan Targets", detail: "Inspect removable flash targets.", status: "pending" },
      { key: "bootloader-check", label: "Detect Bootloader", detail: "Check USB bootloader presence.", status: "pending" },
      { key: "rpiboot", label: "Run rpiboot", detail: "Mount mass-storage gadget if needed.", status: "pending" },
      { key: "select-target", label: "Select Target", detail: "Choose mounted flash disk.", status: "pending" },
      { key: "complete", label: "Complete", detail: "Mount workflow finished.", status: "pending" },
    ];
  }

  return [
    { key: "resolve-image", label: "Resolve Image", detail: "Download/select install image.", status: "pending" },
    { key: "scan-targets", label: "Scan Targets", detail: "Inspect removable flash targets.", status: "pending" },
    { key: "bootloader-check", label: "Detect Bootloader", detail: "Check USB bootloader presence.", status: "pending" },
    { key: "rpiboot", label: "Run rpiboot", detail: "Mount mass-storage gadget if needed.", status: "pending" },
    { key: "select-target", label: "Select Target", detail: "Choose flash disk for write.", status: "pending" },
    { key: "flash", label: "Write Image", detail: "Flash image to selected target.", status: "pending" },
    { key: "verify", label: "Verify Image", detail: "Verify flashed target against source image.", status: "pending" },
    { key: "finalize-target", label: "Finalize Device", detail: "Attempt eject/power-cycle after write.", status: "pending" },
    { key: "complete", label: "Complete", detail: "Install workflow finished.", status: "pending" },
  ];
}

export function calculateInstallProgress(
  steps: UpdaterStep[],
  stepProgressPercent: Record<string, number>,
): number {
  if (steps.length === 0) {
    return 0;
  }

  const otaMode = steps.some((step) => step.key.startsWith("ota-"));
  const flashMode = !otaMode && steps.some((step) => step.key === "flash");
  const weightByStep = new Map<string, number>(
    otaMode
      ? [
          ["resolve-image", 0.05],
          ["resolve-target", 0.05],
          ["ota-upload", 0.2],
          ["ota-apply", 0.1],
          ["ota-monitor", 0.35],
          ["ota-reconnect", 0.25],
          ["complete", 0.0],
        ]
      : flashMode
        ? [
            ["resolve-image", 0.03],
            ["scan-targets", 0.03],
            ["bootloader-check", 0.03],
            ["rpiboot", 0.03],
            ["select-target", 0.03],
            ["flash", 0.7],
            ["verify", 0.1],
            ["finalize-target", 0.05],
            ["complete", 0.0],
          ]
        : [
            ["scan-targets", 0.25],
            ["bootloader-check", 0.2],
            ["rpiboot", 0.2],
            ["select-target", 0.3],
            ["complete", 0.05],
          ],
  );

  let completedWeight = 0;
  let totalWeight = 0;
  for (const step of steps) {
    const weight = weightByStep.get(step.key) ?? 0;
    totalWeight += weight;

    if (weight <= 0) {
      continue;
    }

    if (step.status === "done" || step.status === "skipped" || step.status === "error") {
      completedWeight += weight;
    } else if (step.status === "active") {
      const progress = stepProgressPercent[step.key];
      if (progress != null && Number.isFinite(progress)) {
        completedWeight += weight * (clampPercent(progress) / 100);
      } else {
        completedWeight += weight * 0.5;
      }
    }
  }

  if (totalWeight <= 0) {
    return 0;
  }

  return Math.min(1, Math.max(0, completedWeight / totalWeight));
}

export function formatBytes(value: number): string {
  if (!value || value <= 0) {
    return "0 B";
  }
  const units = ["B", "KB", "MB", "GB", "TB"];
  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  const scaled = value / 1024 ** index;
  return `${scaled.toFixed(scaled >= 10 ? 0 : 1)} ${units[index]}`;
}

export function clampPercent(value: number): number {
  return Math.min(100, Math.max(0, value));
}

export function stepSupportsProgress(stepKey: string): boolean {
  return (
    stepKey === "resolve-image" ||
    stepKey === "flash" ||
    stepKey === "verify" ||
    stepKey.startsWith("ota-")
  );
}

export function chipClass(chip: string): string {
  if (chip === "PhotonVision") {
    return "border-tertiary-500/50 bg-tertiary-500/10 text-tertiary-100";
  }
  if (chip === "HeliOS") {
    return "border-success-500/50 bg-success-500/10 text-success-100";
  }
  if (chip === "Bootloader Device") {
    return "border-warning-500/50 bg-warning-500/10 text-warning-100";
  }
  if (chip === "Mounted") {
    return "border-success-500/50 bg-success-500/10 text-success-100";
  }
  if (chip === "USB IP") {
    return "border-primary-500/50 bg-primary-500/10 text-primary-100";
  }
  return "border-secondary-500/50 bg-secondary-500/10 text-secondary-100";
}

export function statusClass(status: string): string {
  if (status === "bootloader") {
    return "text-warning-300";
  }
  if (status === "mounted") {
    return "text-success-300";
  }
  return "text-success-300";
}

export function statusLabel(status: string): string {
  if (status === "bootloader") {
    return "Bootloader";
  }
  if (status === "mounted") {
    return "Mounted";
  }
  return "Online";
}

export function installStepClass(status: UpdaterStepStatus): string {
  if (status === "done") {
    return "border-success-500/40 bg-success-500/10 text-success-100";
  }
  if (status === "active") {
    return "border-primary-500/50 bg-primary-500/10 text-primary-100";
  }
  if (status === "error") {
    return "border-error-500/50 bg-error-500/10 text-error-100";
  }
  if (status === "skipped") {
    return "border-warning-500/40 bg-warning-500/10 text-warning-100";
  }
  return "border-surface-700/70 bg-surface-900/60 text-surface-300";
}

export function installStepStateLabel(status: UpdaterStepStatus): string {
  if (status === "done") return "Done";
  if (status === "active") return "Running";
  if (status === "error") return "Error";
  if (status === "skipped") return "Skipped";
  return "Pending";
}
