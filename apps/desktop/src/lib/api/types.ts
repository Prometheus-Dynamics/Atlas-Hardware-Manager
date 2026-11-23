// The facts the core sends (apps/desktop/src-tauri/src/view.rs). The words and colours a person
// sees for them are decided in the UI, from the mockups.

export type DeviceState = "running" | "recovery" | "boot-rom" | "removable-disk" | "robot-device";

export type LemnosState =
  | { state: "absent" }
  | { state: "answering" }
  | { state: "not-answering"; reason: string }
  | { state: "revision-mismatch"; reason: string };

export type DeviceKey = { family: string; serial: string } | { node: string } | Record<string, unknown>;

export type LinkKind =
  | { kind: "usb-network" }
  | { kind: "ethernet" }
  | { kind: "usb-serial" }
  | { kind: "usb-boot" }
  | { kind: "local-disk" }
  | { kind: "gateway"; via: DeviceKey }
  | { kind: "simulated" };

export type LinkSighting = { link: string; kind: LinkKind; address: string; last_seen_ms: number };

export type Identity = {
  key: DeviceKey;
  model: string | null;
  state: DeviceState;
  lemnos: LemnosState;
  versions: Record<string, string>;
  name: string | null;
  link: string;
  address: string;
  attributes: Record<string, string>;
};

export type Capability =
  | "info" | "update" | "recover" | "config" | "logs" | "telemetry" | "actions" | "gateway" | "open-ui"
  | "self-test" | "status" | "hardware-control" | "slots" | "install" | "files" | "shell" | "flash";

export type Device = {
  id: string;
  key: DeviceKey;
  name: string;
  identity: Identity;
  presence: "online" | "offline";
  health: "ok" | "away" | "fault";
  links: LinkSighting[];
  first_seen_ms: number;
  last_seen_ms: number;
  label: string | null;
  robot: string | null;
  shown: Capability[];
  offers_reported: boolean;
  /** What the catalog calls the model, when it knows it. */
  model_name: string | null;
  /** `controller`: a robot's controller that does not run Orion yet. */
  role: "controller" | null;
  /** The default slot's version, once a slot list was read for the board (F4 fills it). */
  default_version: string | null;
};

export type Robot = { name: string; team: string | null; notes: string | null };

export type HistoryKind = "found" | "returned" | "absent" | "link-changed" | "changed" | "robot" | "power-cycle" | "label" | "folded" | "action" | "action-failed";

export type ActivityLine = { at_ms: number; level: "info" | "warn" | "err"; kind: HistoryKind; device: string; name: string; message: string };

export type Inventory = { devices: Device[]; robots: Robot[]; activity: ActivityLine[] };

export type ScanProblem = { family: string; link: string; error: string; timed_out: boolean; attempt: number; retry_in_ms: number };

export type Link = { id: string; kind: LinkKind; label: string };

export type Scan = { paused: boolean; links: Link[]; first_round_done: boolean; problems: ScanProblem[] };

export type JobStatus =
  | { status: "queued" } | { status: "running" } | { status: "paused" } | { status: "done" } | { status: "cancelled" }
  | { status: "retrying"; attempt: number; error: string; retry_in_ms: number }
  | { status: "unknown"; reason: string; checking: string | null }
  | { status: "failed"; reason: string }
  | { status: "interrupted"; reason: string };

export type Job = { id: number; kind: string; claim: { devices?: DeviceKey[] }; steps: string[]; completed: number; status: JobStatus };

export type Settings = { keep_last: number; security: boolean; channel: string; theme: "system" | "light" | "dark" };
