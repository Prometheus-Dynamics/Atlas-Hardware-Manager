// Typed wrappers for every Tauri command in apps/desktop/src-tauri.
// Argument names are camelCase: Tauri maps them to the Rust snake_case
// parameters. Rejected promises carry a user-facing sentence (a string).

import { invoke } from "@tauri-apps/api/core";
import type {
  ActivityEntry,
  AppInfo,
  DiscoveryStatus,
  OrionConnection,
  AppSettings,
  DeviceAction,
  DeviceKey,
  DeviceRecord,
  DeviceStatus,
  HealthCheck,
  HistoryEntry,
  ImageServerStatus,
  JobId,
  JobPlan,
  JobRecord,
  LogLine,
  Metric,
  ReleaseEntry,
  RemoteSource,
  RobotProfile,
  RobotStatus,
  ScanReport,
  SelfTestRecord,
  StagedRollout,
  UpdateRequestInput,
} from "./types";

export const api = {
  // Devices
  scan: () => invoke<ScanReport>("scan"),
  listDevices: () => invoke<DeviceRecord[]>("list_devices"),
  setDeviceLabel: (key: DeviceKey, label: string | null) =>
    invoke<void>("set_device_label", { key, label }),
  setDeviceRobot: (key: DeviceKey, robot: string | null) =>
    invoke<void>("set_device_robot", { key, robot }),
  /** Only offline devices can be forgotten. */
  forgetDevice: (key: DeviceKey) => invoke<void>("forget_device", { key }),
  deviceActions: (key: DeviceKey) => invoke<DeviceAction[]>("device_actions", { key }),
  runDeviceAction: (key: DeviceKey, action: string) =>
    invoke<void>("run_device_action", { key, action }),
  /**
   * Runs the device's self-test (devices with `self-test`). Resolves with the
   * kept result, also when a check failed or the run couldn't finish.
   */
  runSelftest: (key: DeviceKey) => invoke<SelfTestRecord>("run_selftest", { key }),
  /** The last self-test of this device's board, or null. */
  deviceSelftest: (key: DeviceKey) => invoke<SelfTestRecord | null>("device_selftest", { key }),
  /** Live readings; only for devices with the `telemetry` capability. */
  deviceTelemetry: (key: DeviceKey) => invoke<Metric[]>("device_telemetry", { key }),
  /** Recent log lines, oldest first; only for devices with `logs`. */
  deviceLogs: (key: DeviceKey, lines: number) => invoke<LogLine[]>("device_logs", { key, lines }),
  /**
   * What the device is doing now and how it is; only for devices with
   * `status`. Also brings its history up to date.
   */
  deviceStatus: (key: DeviceKey) => invoke<DeviceStatus>("device_status", { key }),
  /** The device's history, newest first: Atlas's record merged with its board's event log. */
  deviceHistory: (key: DeviceKey, limit: number) => invoke<HistoryEntry[]>("device_history", { key, limit }),
  /** Fleet history, newest first. */
  listActivity: (limit: number) => invoke<ActivityEntry[]>("list_activity", { limit }),
  /** Writes identity, readings, logs, and history to a text file. */
  saveSupportBundle: (key: DeviceKey, path: string) => invoke<void>("save_support_bundle", { key, path }),

  // Jobs
  /**
   * What an update would do, per device. Uses catalog metadata only: remote
   * releases need not be downloaded yet, and nothing is hashed.
   */
  planUpdate: (request: UpdateRequestInput) => invoke<JobPlan>("plan_update", { request }),
  /**
   * Downloads remote releases and re-checks every file's SHA-256, then starts
   * the job. Unsigned local files are refused unless settings allow them.
   * Follow progress with events.
   */
  startUpdate: (request: UpdateRequestInput) => invoke<JobId>("start_update", { request }),
  cancelJob: (id: JobId) => invoke<void>("cancel_job", { id }),
  listJobs: () => invoke<JobRecord[]>("list_jobs"),
  getJob: (id: JobId) => invoke<JobRecord | null>("get_job", { id }),

  // Robots
  listRobots: () => invoke<RobotProfile[]>("list_robots"),
  robotStatuses: () => invoke<RobotStatus[]>("robot_statuses"),
  /** Creates or replaces a profile; pass previousName to rename. */
  saveRobot: (profile: RobotProfile, previousName: string | null = null) =>
    invoke<void>("save_robot", { profile, previousName }),
  deleteRobot: (name: string) => invoke<void>("delete_robot", { name }),
  /** The request that brings the robot to its targets, or null if it is ready. */
  robotUpdateRequest: (name: string, staged: StagedRollout | null = null) =>
    invoke<UpdateRequestInput | null>("robot_update_request", { name, staged }),

  // Releases
  listReleases: () => invoke<ReleaseEntry[]>("list_releases"),
  addLocalRelease: (path: string, family: string, version: string) =>
    invoke<ReleaseEntry>("add_local_release", { path, family, version }),
  removeRelease: (id: string) => invoke<void>("remove_release", { id }),
  /** Pinned releases stay when older local images drop off the list. */
  setReleasePinned: (id: string, pinned: boolean) => invoke<void>("set_release_pinned", { id, pinned }),
  /** Returns warnings such as manifests rejected for bad signatures. */
  refreshReleases: () => invoke<string[]>("refresh_releases"),
  downloadRelease: (id: string) => invoke<ReleaseEntry>("download_release", { id }),
  listReleaseSources: () => invoke<RemoteSource[]>("list_release_sources"),
  setReleaseSource: (source: RemoteSource) => invoke<void>("set_release_source", { source }),
  removeReleaseSource: (name: string) => invoke<void>("remove_release_source", { name }),

  // System
  appInfo: () => invoke<AppInfo>("app_info"),
  /** Null where this build has no Orion (Windows, simulated devices). */
  orionConnection: () => invoke<OrionConnection | null>("orion_connection"),
  setOrionUrl: (url: string | null) => invoke<OrionConnection | null>("set_orion_url", { url }),
  checkOrion: () => invoke<OrionConnection | null>("check_orion"),
  enrollOrionWithKey: (key: string) => invoke<OrionConnection | null>("enroll_orion_with_key", { key }),
  /** Null where this build has no Orion. */
  imageServerStatus: () => invoke<ImageServerStatus | null>("image_server_status"),
  setImageServer: (port: number, host: string | null) =>
    invoke<ImageServerStatus | null>("set_image_server", { port, host }),
  discoveryStatus: () => invoke<DiscoveryStatus>("discovery_status"),
  healthChecks: () => invoke<HealthCheck[]>("health_checks"),
  /** Runs a health check's fix action; resolves with what changed. */
  fixHealth: (action: string) => invoke<string>("fix_health", { action }),
  getSettings: () => invoke<AppSettings>("get_settings"),
  /** Resolves true when a change applies only after restartApp(). */
  saveSettings: (settings: AppSettings) => invoke<boolean>("save_settings", { settings }),
  restartApp: () => invoke<void>("restart_app"),
};

/** Normalizes a rejected command into a displayable sentence. */
export function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}
