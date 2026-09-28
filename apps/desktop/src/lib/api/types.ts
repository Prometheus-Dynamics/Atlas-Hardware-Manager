// Mirrors the serde output of atlas-driver, atlas-core, atlas-release, and
// the atlas-app shell. Field names are snake_case because that is what the
// Rust side serializes. Keep in sync with those crates.

export interface DeviceKey {
  family: string;
  serial: string;
}

export type LinkKind =
  | { kind: "usb-network" }
  | { kind: "ethernet" }
  | { kind: "usb-serial" }
  | { kind: "usb-boot" }
  | { kind: "gateway"; via: DeviceKey }
  | { kind: "simulated" };

export type DeviceMode = "normal" | "recovery";

export interface Identity {
  key: DeviceKey;
  model: string;
  mode: DeviceMode;
  /** Named versions such as `os`, `firmware`, `bootloader`. */
  versions: Record<string, string>;
  name: string | null;
  link: string;
  address: string;
}

export type CapabilityKind =
  | "info"
  | "update"
  | "recover"
  | "config"
  | "logs"
  | "telemetry"
  | "actions"
  | "gateway"
  | "open-ui";

export type Presence = "online" | "offline";

export interface DeviceRecord {
  key: DeviceKey;
  identity: Identity;
  link_kind: LinkKind;
  capabilities: CapabilityKind[];
  presence: Presence;
  first_seen_ms: number;
  last_seen_ms: number;
  label: string | null;
  robot: string | null;
}

export interface ScanReport {
  online: DeviceKey[];
  went_offline: DeviceKey[];
  warnings: string[];
  duration_ms: number;
}

export interface DeviceAction {
  id: string;
  label: string;
  destructive: boolean;
}

export type UpdateStep = "preflight" | "transfer" | "apply" | "reboot" | "confirm";

export type Concurrency = { kind: "parallel" } | { kind: "exclusive"; resource: string };

export interface UpdatePlan {
  steps: UpdateStep[];
  concurrency: Concurrency;
  summary: string;
}

export interface Artifact {
  name: string;
  path: string;
  sha256: string;
  size_bytes: number;
}

export interface ReleaseRef {
  family: string;
  version: string;
  artifact: Artifact | null;
}

export interface PlannedDevice {
  device: DeviceKey;
  name: string;
  from_version: string | null;
  release: ReleaseRef;
  plan: UpdatePlan;
  /** Updated alone first; the rest of its family waits for it. */
  canary: boolean;
}

export interface JobPlan {
  devices: PlannedDevice[];
}

export type DeviceJobStatus =
  | { status: "queued" }
  | { status: "running" }
  | { status: "verified"; version: string }
  | { status: "rolled-back"; reason: string }
  | { status: "needs-recovery"; reason: string }
  | { status: "failed"; error: string }
  | { status: "skipped"; reason: string }
  | { status: "cancelled" };

export interface DeviceJobState {
  device: DeviceKey;
  name: string;
  release: ReleaseRef;
  plan: UpdatePlan;
  canary: boolean;
  status: DeviceJobStatus;
  step: UpdateStep | null;
  /** Progress within the current step, 0 to 1. */
  fraction: number;
  log: string[];
  started_ms: number | null;
  finished_ms: number | null;
}

export type JobId = number;
export type JobState = "running" | "finished" | "cancelled";

export interface JobSummary {
  verified: number;
  rolled_back: number;
  needs_recovery: number;
  failed: number;
  skipped: number;
  cancelled: number;
}

export interface JobRecord {
  id: JobId;
  state: JobState;
  created_ms: number;
  finished_ms: number | null;
  devices: DeviceJobState[];
  summary: JobSummary | null;
}

export type StagedRollout = "auto" | "on" | "off";

export interface ReleaseChoice {
  version: string;
  /** Catalog entry whose file is installed. Omit for devices that fetch their own. */
  release_id?: string | null;
}

export interface UpdateRequestInput {
  devices: DeviceKey[];
  /** Keyed by family. */
  releases: Record<string, ReleaseChoice>;
  staged: StagedRollout;
}

export interface RobotRole {
  role: string;
  family: string;
  device: DeviceKey | null;
}

export interface RobotProfile {
  name: string;
  roles: RobotRole[];
  /** Target version per family. */
  targets: Record<string, string>;
  notes: string | null;
}

export type RobotState = "ready" | "needs-update" | "missing-devices" | "unassigned";

export interface RoleStatus {
  role: string;
  family: string;
  device: DeviceKey | null;
  device_name: string | null;
  presence: Presence | null;
  version: string | null;
  target: string | null;
  up_to_date: boolean | null;
}

export interface RobotStatus {
  name: string;
  state: RobotState;
  roles: RoleStatus[];
  unassigned_devices: DeviceKey[];
}

export type Channel = "stable" | "beta" | "local";

export type ReleaseOrigin = { kind: "local-file" } | { kind: "remote"; source: string; url: string };

export interface ReleaseEntry {
  id: string;
  family: string;
  version: string;
  channel: Channel;
  origin: ReleaseOrigin;
  artifact_name: string;
  sha256: string;
  size_bytes: number;
  /** Null until a remote release is downloaded. */
  path: string | null;
  signed: boolean;
  boards: string[];
  notes_url: string | null;
  added_ms: number;
}

export interface RemoteSource {
  name: string;
  index_url: string;
  /** `ed25519:<hex>` keys. */
  public_keys: string[];
}

export type HealthStatus = "ok" | "warning" | "error";

export interface HealthCheck {
  id: string;
  label: string;
  status: HealthStatus;
  detail: string;
  fix: string | null;
}

export type SimScenario = "demo" | "flaky";

export interface AppSettings {
  simulated: SimScenario | null;
  auto_scan: boolean;
  scan_interval_ms: number;
  staged_default: StagedRollout;
  allow_unsigned_local: boolean;
}

export interface AppPaths {
  data_dir: string;
  cache_dir: string;
  settings_file: string;
  inventory_file: string;
  releases_file: string;
  release_cache_dir: string;
}

export interface AppInfo {
  version: string;
  platform: string;
  arch: string;
  simulated: SimScenario | null;
  paths: AppPaths;
  startup_warnings: string[];
}

/** Payload of `atlas://event`. */
export type AtlasEvent =
  | { type: "scan-started" }
  | { type: "device-seen"; record: DeviceRecord; new: boolean }
  | { type: "device-offline"; key: DeviceKey }
  | { type: "device-forgotten"; key: DeviceKey }
  /** A profile changed, or a device in a robot changed version or presence. */
  | { type: "robots-changed" }
  | { type: "scan-warning"; message: string }
  | { type: "scan-finished"; report: ScanReport }
  | { type: "job-started"; job: JobId; devices: DeviceKey[] }
  | { type: "job-device"; job: JobId; device: DeviceKey; status: DeviceJobStatus }
  | { type: "job-step"; job: JobId; device: DeviceKey; step: UpdateStep }
  | { type: "job-progress"; job: JobId; device: DeviceKey; step: UpdateStep; fraction: number }
  | { type: "job-log"; job: JobId; device: DeviceKey; message: string }
  | { type: "job-finished"; job: JobId; state: JobState; summary: JobSummary };

/** Payload of `atlas://download`. */
export interface DownloadEvent {
  id: string;
  downloaded: number;
  total: number | null;
}

/** `family:serial`, the string form used as a map key and in the CLI. */
export function keyString(key: DeviceKey): string {
  return `${key.family}:${key.serial}`;
}

export function sameKey(a: DeviceKey | null | undefined, b: DeviceKey | null | undefined): boolean {
  return !!a && !!b && a.family === b.family && a.serial === b.serial;
}
