export interface DiscoveredDevice {
  id: string;
  displayName: string;
  status: string;
  connectionChips: string[];
  ipAddress?: string | null;
  macAddress?: string | null;
  interfaceName?: string | null;
  usbLocation?: string | null;
  vendorProduct?: string | null;
  runtimeProduct?: string | null;
  firmwareVersion?: string | null;
  osVersion?: string | null;
  telemetrySummary?: string | null;
  detail: string;
}

export interface DeviceDiscoverySnapshot {
  generatedAtEpochMs: number;
  devices: DiscoveredDevice[];
  warnings: string[];
}

export interface DeviceDiscoveryProgressEvent {
  generatedAtEpochMs: number;
  devices: DiscoveredDevice[];
  warnings: string[];
  inProgress: boolean;
}

export interface ReleaseImageOption {
  releaseTag: string;
  releaseName: string;
  prerelease: boolean;
  assetName: string;
  downloadUrl: string;
  sizeBytes: number;
  publishedAt?: string | null;
}

export interface ReleaseDownloadCacheClearResult {
  removedFiles: number;
  removedBytes: number;
  cacheDirectory: string;
  message: string;
}

export interface OperationResult {
  success: boolean;
  exitCode?: number | null;
  durationMs?: number | null;
  stdout?: string | null;
  stderr?: string | null;
  message?: string | null;
  timedOut?: boolean;
}

export interface ReleaseInstallResult {
  success: boolean;
  mode: string;
  imagePath?: string | null;
  selectedTargetPath?: string | null;
  rpiboot?: OperationResult | null;
  flash?: OperationResult | null;
  message: string;
}

export interface InstallLogEntry {
  timestamp: number;
  level: "info" | "warn" | "error";
  text: string;
}

export interface UpdaterProgressEvent {
  runId?: string | null;
  mode: string;
  step: string;
  status: "running" | "success" | "error" | "skipped" | "info";
  message: string;
  timestampEpochMs: number;
  stdout?: string | null;
  stderr?: string | null;
  exitCode?: number | null;
  durationMs?: number | null;
  imagePath?: string | null;
  targetPath?: string | null;
  progressPercent?: number | null;
  bytesWritten?: number | null;
  bytesTotal?: number | null;
}

export type UpdaterStepStatus = "pending" | "active" | "done" | "error" | "skipped";

export interface UpdaterStep {
  key: string;
  label: string;
  detail: string;
  status: UpdaterStepStatus;
}

export interface DeviceFact {
  label: string;
  value: string;
}

export interface DeviceLogResult {
  success: boolean;
  sourceUrl?: string | null;
  lineCount: number;
  truncated: boolean;
  fetchedAtEpochMs: number;
  logText: string;
  message: string;
}

export interface DeviceTelemetryEvent {
  streamId: string;
  status: "connecting" | "data" | "error" | "stopped";
  message: string;
  timestampEpochMs: number;
  runtimeProduct?: string | null;
  osVersion?: string | null;
  telemetrySummary?: string | null;
  sourceUrl?: string | null;
}

export interface WpilibLogEntry {
  id: string;
  fileName: string;
  remotePath: string;
  sizeBytes: number;
  modifiedEpochMs?: number | null;
}

export interface WpilibLogListResult {
  success: boolean;
  fetchedAtEpochMs: number;
  entries: WpilibLogEntry[];
  message: string;
}

export interface WpilibLogSelection {
  fileName: string;
  remotePath: string;
}

export interface WpilibLogActionItemResult {
  fileName: string;
  remotePath: string;
  localPath?: string | null;
  success: boolean;
  message: string;
}

export interface WpilibLogActionResult {
  success: boolean;
  processedCount: number;
  successCount: number;
  failedCount: number;
  completedAtEpochMs: number;
  downloadDirectory?: string | null;
  results: WpilibLogActionItemResult[];
  message: string;
}

export interface UpdaterRecoverySession {
  runId: string;
  mode: string;
  targetIpAddress?: string | null;
  expectedUpdateId?: string | null;
  startedAtEpochMs: number;
  resumedAtEpochMs: number;
  note?: string | null;
}

export interface ExistingOtaUpdateProbeResult {
  targetIpAddress: string;
  updateId?: string | null;
  stage?: string | null;
  progressPercent?: number | null;
  lastError?: string | null;
}

export interface PowerCycleReminder {
  runId?: string | null;
  deviceId?: string | null;
  message: string;
  createdAtEpochMs: number;
}

export interface InstallDeviceHint {
  usbLocation?: string | null;
  macAddress?: string | null;
  ipAddress?: string | null;
  vendorProduct?: string | null;
  runtimeProduct?: string | null;
  displayName?: string | null;
}

export type OtaApplyTransport = "api" | "usb";

export type InstallMode = "flash" | "mount" | "ota";
