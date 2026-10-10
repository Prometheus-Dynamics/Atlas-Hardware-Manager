// Mirrors the serde output of atlas-driver, atlas-core, atlas-release, and
// the atlas-app shell. Field names are snake_case because that is what the
// Rust side serializes. Keep in sync with those crates.

import type { HardwareSnapshot } from "./hardware-types";
export type {
  NtCameraAddress,
  NtFrame,
  NtServerClient,
  NtServerFrame,
  NtServerInfo,
  NtServerTopic,
  NtTimeSync,
  NtTopic,
  NtValue,
} from "./nt-types";

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
  /** Driver facts such as `os`, `revision`, `hostname`, `manage_url`, `mac.usb0`. */
  attributes: Record<string, string>;
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
  | "open-ui"
  /** The device can check its own hardware (see SelfTestRecord). */
  | "self-test"
  /** The device reports its state and an event log (see DeviceStatus). */
  | "status"
  /** The board's devices take commands (see HardwareCommand). */
  | "hardware-control";

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

/** One live reading, for example temperature in °C. */
export interface Metric {
  id: string;
  label: string;
  value: number;
  unit: string | null;
  /** Top of the normal range, for gauges. */
  max: number | null;
  /** Above this the reading is a warning. */
  warn_above: number | null;
  /** The raw numbers behind the value: "1.2 GiB of 4.0 GiB", "load 0.42 · 0.38 · 0.30". */
  detail?: string | null;
}

export type LogLevel = "debug" | "info" | "warning" | "error";

export interface LogLine {
  at_ms: number | null;
  level: LogLevel;
  source: string | null;
  message: string;
}

export type ActivityKind =
  | "device-found"
  | "device-online"
  | "device-offline"
  | "version-changed"
  | "mode-changed"
  | "update-result"
  | "action-run"
  | "self-test"
  /** The board's event log reported something Orion or a person on it did. */
  | "device-event";

export type ActivityLevel = "info" | "success" | "warning" | "error";

export interface ActivityEntry {
  at_ms: number;
  kind: ActivityKind;
  level: ActivityLevel;
  device: DeviceKey | null;
  robot: string | null;
  message: string;
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
  /** When the driver last reported a step, progress, or log line. */
  last_activity_ms: number | null;
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
  /** Use the file even if it fails its SHA-256 check ("flash anyway"). */
  ignore_checksum?: boolean;
  /** A file used once, without adding it to the release list. */
  path?: string | null;
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
  /** Kept when older unpinned local images drop off the list. */
  pinned: boolean;
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
  /** When set, `api.fixHealth(fix_action)` fixes it (may ask for a password). */
  fix_action: string | null;
}

export type SimScenario = "demo" | "flaky";

/** usb: boards on USB (the default); all: every board; off: offered as a control only. */
export type ClockSync = "usb" | "all" | "off";

export interface AppSettings {
  simulated: SimScenario | null;
  auto_scan: boolean;
  scan_interval_ms: number;
  staged_default: StagedRollout;
  /** Which boards get this computer's time when their clock is off, while watching. */
  clock_sync: ClockSync;
  /** Public key file put on boards Atlas flashes (SSH as root); null is off. */
  ssh_key_file: string | null;
  /** The Orion node Atlas connects to (orion+tcp://host:port); null is off. */
  orion_url: string | null;
  /** TCP port boards download update images from (Orion updates). */
  image_server_port: number;
  /** Host put in image URLs when the route to a board can't be told; null is automatic. */
  image_host: string | null;
}

/** The HTTP server boards download update images from. */
export interface ImageServerStatus {
  listening: boolean;
  /** The configured listen address, like `0.0.0.0:7700`. */
  bind: string;
  port: number;
  /** Addresses boards can use, while listening. */
  addresses: string[];
  host: string | null;
  /** Why it can't listen, when it can't. */
  error: string | null;
  /** Images on offer right now. */
  registrations: number;
}

/** Atlas's connection to Orion as an operator. */
export interface OrionConnection {
  url: string | null;
  /** `operator:<name>`: what an administrator enrolls. */
  operator_id: string;
  /** `sha256:<32 hex>`, matched against `orionctl operators list`. */
  fingerprint: string;
  /** What an administrator runs on a node to let Atlas in. */
  enroll_command: string;
  connected: boolean;
  enrolled: boolean;
  node_id: string | null;
  node_fingerprint: string | null;
  error: string | null;
}

export interface AppPaths {
  data_dir: string;
  cache_dir: string;
  settings_file: string;
  inventory_file: string;
  releases_file: string;
  release_cache_dir: string;
  /** What Atlas did and what went wrong; attach it to bug reports. */
  log_file: string;
}

export interface AppInfo {
  version: string;
  platform: string;
  arch: string;
  simulated: SimScenario | null;
  paths: AppPaths;
  startup_warnings: string[];
}

export type CheckStatus = "ok" | "skip" | "fail" | "unknown";

/** One check of a device's self-test, as the device reports it. */
export interface SelfTestCheck {
  /** For example `fan` or `camera`. */
  id: string;
  status: CheckStatus;
  message: string;
  /** What the check measured; shape is up to the device. */
  data: Record<string, unknown> | null;
}

/** How a running self-test is going (atlas-driver SelfTestStep). */
export type SelfTestStep =
  | { step: "planned"; checks: string[] }
  | { step: "started"; check: string }
  | { step: "finished"; check: string; status: CheckStatus; message: string };

/** The device's `selftest --json` report (format 1). */
export interface SelfTestReport {
  version: number;
  board_serial: string | null;
  model: string | null;
  package_version: string | null;
  /** Unix seconds on the device's clock. */
  at: number | null;
  interactive: boolean;
  ok: boolean;
  checks: SelfTestCheck[];
}

export type SelfTestTrigger = "manual" | "after-update";

/** A self-test run as atlas-core keeps it, per board serial. */
export interface SelfTestRecord {
  /** Atlas's board serial (last 8 hex digits). */
  board_serial: string;
  device: DeviceKey;
  at_ms: number;
  trigger: SelfTestTrigger;
  /** Null when the run itself failed; see `error`. */
  report: SelfTestReport | null;
  error: string | null;
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
  | { type: "job-finished"; job: JobId; state: JobState; summary: JobSummary }
  | { type: "activity"; entry: ActivityEntry }
  /** A self-test finished or couldn't run: the board's latest result. */
  | { type: "self-test"; record: SelfTestRecord }
  /** A running self-test's progress: the checks it will run, one starting, one done. */
  | ({ type: "self-test-progress"; key: DeviceKey } & SelfTestStep)
  /** New events from the device's board: re-read its history. */
  | { type: "device-history"; key: DeviceKey }
  /** The board's push channel said its state changed: re-read its status now. */
  | { type: "device-status"; key: DeviceKey };

// Device status and history (atlas-driver status.rs, atlas-core history.rs).

/** Who asked: Atlas over SSH, Orion through the board's agent, or someone on the board. */
export type EventSource = "atlas" | "orion" | "local" | "unknown";

export interface BootInfo {
  id: string | null;
  /** Boots since the board's data was made. */
  count: number | null;
  slot: string | null;
  kernel: string | null;
  uptime_s: number | null;
  /** Whether the boot before this one shut down cleanly; null when unknown. */
  previous_clean: boolean | null;
}

export interface Temperature {
  id: string;
  celsius: number;
}

export interface FanState {
  state: number | null;
  max_state: number | null;
  /** Duty, 0-255. */
  pwm: number | null;
  rpm: number | null;
}

/** The board's own A/B update state (the writer's `update status`). */
export interface UpdateState {
  /** idle, staging, staged, rebooting, trying, confirmed, rolled-back, cancelled, error */
  state: string;
  slot_active: string | null;
  slot_staged: string | null;
  version_active: string | null;
  version_staged: string | null;
  /** What a rollback goes back to, when there is something. */
  version_previous: string | null;
  /** Per mille of the current step. */
  progress: number;
  error: string | null;
  started_by: EventSource | null;
}

export interface DriftItem {
  /** boot (the running boot slot), root, etc or data (overrides). */
  area: string;
  path: string;
  /** changed, added, missing, or present (an override). */
  change: string;
  sha256: string | null;
}

export interface Drift {
  checked_at: number | null;
  slot: string | null;
  root_read_only: boolean | null;
  /** What the boot files were compared with: stage, first-seen, none. */
  baseline: string | null;
  count: number;
  /** cmdline, config, sshd_config, update_env */
  flags: string[];
  items: DriftItem[];
}

export type {
  HardwareCommand,
  HardwareControl,
  HardwareDevice,
  HardwareFrame,
  HardwareReading,
  HardwareSnapshot,
  LiveChannel,
  LiveDevice,
} from "./hardware-types";

/** What a device is doing now and how it is; absent parts are unknown. */
export interface DeviceStatus {
  time: number | null;
  boot: BootInfo | null;
  /** null: the device can't tell; [] means none failed. */
  failed_units: string[] | null;
  temperatures: Temperature[];
  fan: FanState | null;
  update: UpdateState | null;
  /** Seconds the device's clock is off from this computer's (negative: behind). */
  clock_offset_s: number | null;
  ntp_synchronized: boolean | null;
  drift: Drift | null;
  /** null: no fresh reading from the board's hardware service. */
  hardware: HardwareSnapshot | null;
}

/** One line of a board's event log. */
export interface DeviceEvent {
  /** The board's clock, Unix seconds. */
  t: number;
  boot_id: string;
  /** Dotted: update.staged, boot, clock.set, … */
  kind: string;
  source: EventSource;
  message: string;
  data: Record<string, string>;
  /** The board's number for it, in write order across boots (null from older boards). */
  seq?: number | null;
  uptime_s?: number | null;
  /** By this computer's clock (Unix ms), when the boot's clock offset was known. */
  at_ms?: number;
}

/** One line of a device's history: Atlas's record or the board's event log. */
export interface HistoryEntry {
  at_ms: number;
  level: ActivityLevel;
  /** An activity kind (update-result, …) or a board event kind (update.staged, boot, …). */
  kind: string;
  source: EventSource;
  origin: "atlas" | "board";
  message: string;
  boot_id: string | null;
  data: Record<string, string>;
}

/** How Atlas keeps the inventory current. */
export interface DiscoveryStatus {
  /** True while Atlas watches for devices. */
  live: boolean;
  /** Sources that push changes, such as `USB` and `Network`. */
  watching: string[];
  /** Sources only checked on the safety-net timer. */
  polled: string[];
  fallback_ms: number;
}

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
