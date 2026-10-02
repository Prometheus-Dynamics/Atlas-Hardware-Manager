// The simulated fleet and catalog for the browser mock. Mirrors the
// `demo` fleet in crates/atlas-driver-mock.

import type {
  AppSettings,
  CapabilityKind,
  DeviceKey,
  DeviceMode,
  DeviceRecord,
  JobRecord,
  ReleaseEntry,
  RemoteSource,
  RobotProfile,
} from "../types";
import { keyString } from "../types";

export interface SimDevice {
  key: DeviceKey;
  model: string;
  name: string;
  version: string;
  mode: DeviceMode;
  parent: DeviceKey | null;
  online: boolean;
  /** Never confirms its update and rolls back (the `flaky` scenario). */
  neverConfirms: boolean;
}

export interface Stored {
  first_seen_ms: number;
  last_seen_ms: number;
  label: string | null;
  robot: string | null;
  presence: "online" | "offline";
  record: DeviceRecord | null;
}

const HELIOS = "sim-helios";
const MCU = "sim-mcu";

const camera = (serial: string, name: string): SimDevice => ({
  key: { family: HELIOS, serial },
  model: "cm5",
  name,
  version: "2026.2.4",
  mode: "normal",
  parent: null,
  online: true,
  neverConfirms: false,
});

const board = (serial: string, name: string, parent: DeviceKey | null): SimDevice => ({
  key: { family: MCU, serial },
  model: "stm32g4",
  name,
  version: "1.4.0",
  mode: "normal",
  parent,
  online: true,
  neverConfirms: false,
});

const front = { family: HELIOS, serial: "H-1001" };

export const fleet: SimDevice[] = [
  camera("H-1001", "cam-front"),
  camera("H-1002", "cam-left"),
  camera("H-1003", "cam-rear"),
  board("M-2001", "drive-mcu-1", front),
  board("M-2002", "drive-mcu-2", front),
  { ...board("M-2003", "arm-mcu", null), version: "bootloader-2.1", mode: "recovery" },
];

export function simDevice(key: DeviceKey): SimDevice | undefined {
  return fleet.find((d) => keyString(d.key) === keyString(key));
}

function versionName(device: SimDevice): string {
  if (device.mode === "recovery") return "bootloader";
  return device.key.family === HELIOS ? "os" : "firmware";
}

export function capabilities(device: SimDevice): CapabilityKind[] {
  return device.mode === "recovery" ? ["info", "recover"] : ["info", "update", "actions"];
}

export function identityOf(device: SimDevice) {
  return {
    key: device.key,
    model: device.model,
    mode: device.mode,
    versions: { [versionName(device)]: device.version },
    name: device.name,
    link: device.parent ? `gateway:${keyString(device.parent)}` : "sim0",
    address: device.key.serial,
    attributes: {},
  };
}

export function linkKindOf(device: SimDevice): DeviceRecord["link_kind"] {
  return device.parent ? { kind: "gateway", via: device.parent } : { kind: "simulated" };
}

export const inventory = new Map<string, Stored>();

export const robots: RobotProfile[] = [
  {
    name: "Atlas-01",
    roles: [
      { role: "front camera", family: HELIOS, device: { family: HELIOS, serial: "H-1001" } },
      { role: "left camera", family: HELIOS, device: { family: HELIOS, serial: "H-1002" } },
      { role: "drive left", family: MCU, device: { family: MCU, serial: "M-2001" } },
      { role: "drive right", family: MCU, device: { family: MCU, serial: "M-2002" } },
    ],
    targets: { [HELIOS]: "2026.3.0", [MCU]: "1.4.0" },
    notes: "Competition robot.",
  },
];

const now = Date.now();
const day = 86_400_000;

export const releases: ReleaseEntry[] = [
  {
    id: "pd-stable/sim-helios/2026.3.0",
    family: HELIOS,
    version: "2026.3.0",
    channel: "stable",
    origin: { kind: "remote", source: "pd-stable", url: "https://releases.example/helios-2026.3.0.img.xz" },
    artifact_name: "helios-2026.3.0.img.xz",
    sha256: "9f2c".padEnd(64, "0"),
    size_bytes: 1_420_000_000,
    path: null,
    signed: true,
    boards: ["cm5"],
    notes_url: "https://example.com/helios/2026.3.0",
    added_ms: now - 2 * day,
  },
  {
    id: "pd-stable/sim-helios/2026.2.4",
    family: HELIOS,
    version: "2026.2.4",
    channel: "stable",
    origin: { kind: "remote", source: "pd-stable", url: "https://releases.example/helios-2026.2.4.img.xz" },
    artifact_name: "helios-2026.2.4.img.xz",
    sha256: "71ab".padEnd(64, "0"),
    size_bytes: 1_380_000_000,
    path: "/home/atlas/.cache/atlas/releases/helios-2026.2.4.img.xz",
    signed: true,
    boards: ["cm5"],
    notes_url: null,
    added_ms: now - 30 * day,
  },
  {
    id: "pd-beta/sim-mcu/1.5.0",
    family: MCU,
    version: "1.5.0",
    channel: "beta",
    origin: { kind: "remote", source: "pd-beta", url: "https://releases.example/mcu-1.5.0.bin" },
    artifact_name: "mcu-1.5.0.bin",
    sha256: "c0de".padEnd(64, "0"),
    size_bytes: 412_000,
    path: null,
    signed: true,
    boards: ["stm32g4"],
    notes_url: "https://example.com/mcu/1.5.0",
    added_ms: now - 5 * day,
  },
  {
    id: "pd-stable/sim-mcu/1.4.0",
    family: MCU,
    version: "1.4.0",
    channel: "stable",
    origin: { kind: "remote", source: "pd-stable", url: "https://releases.example/mcu-1.4.0.bin" },
    artifact_name: "mcu-1.4.0.bin",
    sha256: "beef".padEnd(64, "0"),
    size_bytes: 398_000,
    path: "/home/atlas/.cache/atlas/releases/mcu-1.4.0.bin",
    signed: true,
    boards: ["stm32g4"],
    notes_url: null,
    added_ms: now - 60 * day,
  },
  {
    id: "local/sim-mcu/1.5.1-dev",
    family: MCU,
    version: "1.5.1-dev",
    channel: "local",
    origin: { kind: "local-file" },
    artifact_name: "mcu-dev.bin",
    sha256: "dead".padEnd(64, "0"),
    size_bytes: 401_222,
    path: "/home/atlas/builds/mcu-dev.bin",
    signed: false,
    boards: [],
    notes_url: null,
    added_ms: now - day,
  },
];

export const sources: RemoteSource[] = [
  {
    name: "pd-stable",
    index_url: "https://releases.example/stable/index.json",
    public_keys: ["ed25519:3b6a27bcceb6a42d62a3a8d02a6f0d73653215771de243a63ac048a18b59da29"],
  },
  {
    name: "pd-beta",
    index_url: "https://releases.example/beta/index.json",
    public_keys: ["ed25519:3b6a27bcceb6a42d62a3a8d02a6f0d73653215771de243a63ac048a18b59da29"],
  },
];

export const settings: AppSettings = {
  simulated: "demo",
  auto_scan: true,
  scan_interval_ms: 3000,
  staged_default: "auto",
};

export const jobs: JobRecord[] = [];
