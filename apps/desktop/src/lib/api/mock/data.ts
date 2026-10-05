// The simulated fleet and catalog for the browser mock. Mirrors the
// `demo` fleet in crates/atlas-driver-mock.

import type {
  AppSettings,
  CapabilityKind,
  DeviceAction,
  DeviceKey,
  DeviceMode,
  DeviceRecord,
  JobRecord,
  ReleaseEntry,
  RemoteSource,
  RobotProfile,
} from "../types";
import { keyString } from "../types";
import { TEST_PATTERN } from "./pattern";

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
  /** Real-driver extras: Raze boards report these (see atlas-driver-pd/-rpi). */
  attributes?: Record<string, string>;
  versions?: Record<string, string>;
  link?: DeviceRecord["link_kind"];
  caps?: CapabilityKind[];
  actions?: DeviceAction[];
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
  attributes: {
    os: "helios",
    hostname: `${name}.local`,
    manage_url: `http://${name}.local:5800/`,
    camera_stream: TEST_PATTERN,
    pipeline: "apriltag",
  },
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
  attributes: { bus: "CAN 1 Mbit/s", can_id: String(parseInt(serial.slice(2), 10) % 60) },
});

const front = { family: HELIOS, serial: "H-1001" };

const RPI = "rpi";
const RAZE = "raze";

const RAZE_STEPS = [
  "Power the Raze off.",
  "Hold the boot button on the Raze.",
  "While holding it, connect the Raze's regular USB port (the one used for flashing) to the computer running Atlas, then release the button once the device appears.",
].join("\n");

/** A Raze in USB boot, as atlas-driver-rpi reports one. */
const razeInUsbBoot: SimDevice = {
  key: { family: RPI, serial: "port-3-2" },
  model: "Raze",
  name: "Raze",
  version: "",
  mode: "recovery",
  parent: null,
  online: true,
  neverConfirms: false,
  versions: {},
  attributes: { chip: "BCM2712", model: "raze", storage: "emmc", recovery_steps: RAZE_STEPS },
  link: { kind: "usb-boot" },
  caps: ["info", "recover", "actions"],
  actions: [
    { id: "open-as-disk", label: "Open as USB disk", destructive: false },
    { id: "update-bootloader", label: "Update bootloader (EEPROM)", destructive: true },
  ],
};

/** A running Raze on the robot network, as atlas-driver-pd reports one. */
const razeRunning: SimDevice = {
  key: { family: RAZE, serial: "8f3a1c2d" },
  model: "Raze",
  name: "raze-8f3a1c2d",
  version: "2026.1.2",
  mode: "normal",
  parent: null,
  online: true,
  neverConfirms: false,
  versions: { bootloader: "2025-12-08", device_package: "1.0.4" },
  attributes: {
    os: "HeliOS",
    os_version: "2026.1.2",
    revision: "gen1",
    hostname: "raze-8f3a1c2d",
    manage_url: "http://raze-8f3a1c2d.local:5800",
    "mac.eth0": "d8:3a:dd:8f:3a:1c",
    "mac.usb0": "02:3a:dd:8f:3a:1d",
    update_methods: "atlas-ota, rpiboot-recovery",
    contract: "1",
    camera_stream: TEST_PATTERN,
  },
  link: { kind: "usb-network" },
  caps: ["info", "update", "actions", "telemetry", "logs"],
  actions: [
    { id: "locate", label: "Find it", destructive: false },
    { id: "reboot", label: "Restart", destructive: false },
  ],
};

export const fleet: SimDevice[] = [
  camera("H-1001", "cam-front"),
  camera("H-1002", "cam-left"),
  camera("H-1003", "cam-rear"),
  board("M-2001", "drive-mcu-1", front),
  board("M-2002", "drive-mcu-2", front),
  { ...board("M-2003", "arm-mcu", null), version: "bootloader-2.1", mode: "recovery" },
  razeRunning,
  razeInUsbBoot,
];

export function simDevice(key: DeviceKey): SimDevice | undefined {
  return fleet.find((d) => keyString(d.key) === keyString(key));
}

function versionName(device: SimDevice): string {
  if (device.mode === "recovery") return "bootloader";
  return device.key.family === HELIOS || device.key.family === RAZE ? "os" : "firmware";
}

export function capabilities(device: SimDevice): CapabilityKind[] {
  if (device.caps) return device.caps;
  return device.mode === "recovery" ? ["info", "recover"] : ["info", "update", "actions", "telemetry", "logs"];
}

export function identityOf(device: SimDevice) {
  return {
    key: device.key,
    model: device.model,
    mode: device.mode,
    versions: device.version
      ? { [versionName(device)]: device.version, ...(device.versions ?? {}) }
      : { ...(device.versions ?? {}) },
    name: device.name,
    link: device.parent ? `gateway:${keyString(device.parent)}` : device.link?.kind === "usb-boot" ? "usb:3-2" : "sim0",
    address: device.link?.kind === "usb-network" ? "fe80::3a:ddff:fe8f:3a1d%usb0" : device.key.serial,
    attributes: device.mode === "recovery" && !device.caps ? {} : { ...(device.attributes ?? {}) },
  };
}

export function linkKindOf(device: SimDevice): DeviceRecord["link_kind"] {
  if (device.link) return device.link;
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
    id: "pd-stable/rpi/helios-raze-2026.3.0",
    family: RPI,
    version: "helios-raze-2026.3.0",
    channel: "stable",
    origin: { kind: "remote", source: "pd-stable", url: "https://releases.example/helios-raze-2026.3.0.img.xz" },
    artifact_name: "helios-raze-2026.3.0.img.xz",
    sha256: "a11e".padEnd(64, "0"),
    size_bytes: 1_910_000_000,
    path: null,
    signed: true,
    boards: ["raze"],
    notes_url: "https://example.com/helios/2026.3.0",
    added_ms: now - 2 * day,
  },
  {
    id: "pd-stable/rpi/photonvision-raze-2026.1.0",
    family: RPI,
    version: "photonvision-raze-2026.1.0",
    channel: "stable",
    origin: { kind: "remote", source: "pd-stable", url: "https://releases.example/photonvision-raze-2026.1.0.img.xz" },
    artifact_name: "photonvision-raze-2026.1.0.img.xz",
    sha256: "b0b0".padEnd(64, "0"),
    size_bytes: 1_240_000_000,
    path: "/home/atlas/.cache/atlas/releases/photonvision-raze-2026.1.0.img.xz",
    signed: true,
    boards: ["raze"],
    notes_url: null,
    added_ms: now - 12 * day,
  },
  {
    id: "pd-stable/raze/2026.3.0",
    family: RAZE,
    version: "2026.3.0",
    channel: "stable",
    origin: { kind: "remote", source: "pd-stable", url: "https://releases.example/raze-ota-2026.3.0.tar.zst" },
    artifact_name: "raze-ota-2026.3.0.tar.zst",
    sha256: "0ca7".padEnd(64, "0"),
    size_bytes: 412_000_000,
    path: null,
    signed: true,
    boards: ["raze"],
    notes_url: "https://example.com/helios/2026.3.0",
    added_ms: now - 2 * day,
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
  scan_interval_ms: 20000,
  staged_default: "auto",
  ssh_key_file: null,
  orion_url: null,
};

export const jobs: JobRecord[] = [];
