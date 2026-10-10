// Simulated device self-tests for the browser mock: a Raze with the
// `self-test` capability answers with a report shaped like
// /usr/lib/board/selftest --json, and a board back from a flash or an
// update is tested by itself, as atlas-core does.

import type { DeviceKey, SelfTestCheck, SelfTestRecord, SelfTestReport, SelfTestTrigger } from "../types";
import { keyString } from "../types";
import { emit, sleep } from "./bus";
import { fleet, inventory, simDevice, type SimDevice } from "./data";
import { record } from "./observe";

/** The last run per board serial. */
export const selftests = new Map<string, SelfTestRecord>();

const boardOf = (device: SimDevice) => device.attributes?.board_serial ?? device.key.serial;

function report(device: SimDevice): SelfTestReport {
  const serial = boardOf(device);
  // The board ending in "e1" has no camera on its connector, to show a failure.
  const cameraMissing = serial.endsWith("e1");
  const checks: SelfTestCheck[] = [
    {
      id: "leds",
      status: "ok",
      message: "/dev/leds0 takes frames (redrew the ring's current state)",
      data: { device: "/dev/leds0", count: 16, offset: 5, direction: 1, wire_format: "grb24", frame: "redrawn" },
    },
    {
      id: "fan",
      status: "ok",
      message: "states 0-4 set duty 179 212 245 255 255 as the package says; no tachometer; back to automatic",
      data: {
        cooling_device: "cooling_device0",
        max_state: 4,
        levels: [179, 212, 245, 255, 255],
        steps: [179, 212, 245, 255, 255].map((pwm, state) => ({ state, pwm, expected: pwm, rpm: null })),
        tachometer: false,
      },
    },
    cameraMissing
      ? {
          id: "camera",
          status: "fail",
          message: "no ov9782 at 0x60 on any I2C bus: is the camera connected, and does raze-device.txt load its overlay?",
          data: { sensor: "ov9782", address: 96, i2c_client: null },
        }
      : {
          id: "camera",
          status: "ok",
          message: "ov9782 bound to ov9282 at 10-0060 (chip id accepted), in the media graph (/dev/media0), 4 rp1-cfe video nodes",
          data: { sensor: "ov9782", address: 96, i2c_client: "10-0060", driver: "ov9282", receiver_video_nodes: 4 },
        },
    {
      id: "i2c",
      status: "ok",
      message: "all 4 devices answer",
      data: {
        probe: "i2cdetect",
        devices: [
          { id: "imu-accel", part: "BMI088", bus: 4, address: 24, result: "present" },
          { id: "imu-gyro", part: "BMI088", bus: 4, address: 104, result: "present" },
          { id: "magnetometer", part: "BMM150", bus: 1, address: 16, result: "present" },
          { id: "power-monitor", part: "INA238", bus: 1, address: 64, result: "present" },
        ],
      },
    },
    {
      id: "watchdog",
      status: "ok",
      message: "/dev/watchdog0 (bcm2835-wdt) is kept by systemd every 15s",
      data: { device: "/dev/watchdog0", runtime_watchdog: "15s" },
    },
    {
      id: "gadget",
      status: "ok",
      message: "gadget g1 is bound to 1000480000.usb; usbbr0 is up at 172.31.250.1/24",
      data: { gadget: "g1", bridge: "usbbr0", up: true, addresses: ["172.31.250.1/24"] },
    },
  ];
  return {
    version: 1,
    board_serial: `10000000${serial}`,
    model: "raze",
    package_version: device.versions?.device_package ?? null,
    at: Math.floor(Date.now() / 1000),
    interactive: false,
    ok: !checks.some((c) => c.status === "fail"),
    checks,
  };
}

function keep(device: SimDevice, run: SelfTestRecord) {
  selftests.set(run.board_serial, run);
  const name = inventory.get(keyString(device.key))?.label ?? device.name;
  const after = run.trigger === "after-update" ? " after its update" : "";
  const checks = run.report?.checks ?? [];
  const passed = checks.filter((c) => c.status === "ok").length;
  const failed = checks.filter((c) => c.status === "fail").map((c) => c.id);
  const summary = `${passed} of ${checks.length} checks passed${failed.length ? `; ${failed.join(", ")} failed` : ""}`;
  if (run.report?.ok) record("self-test", "success", device.key, `${name} passed its self-test${after}: ${summary}`);
  else record("self-test", "warning", device.key, `${name} failed its self-test${after}: ${summary}`);
  emit({ type: "self-test", record: run });
}

/** Runs the simulated test: about 4 s, like stepping the fan on a board. */
export async function runSelftest(key: DeviceKey, trigger: SelfTestTrigger = "manual"): Promise<SelfTestRecord> {
  const device = simDevice(key);
  const stored = inventory.get(keyString(key));
  if (!device || stored?.presence !== "online") throw `${keyString(key)} is offline; reconnect it or run a scan`;
  if (!device.caps?.includes("self-test")) throw `${keyString(key)} does not report a self-test`;
  const result = report(device);
  // Each check as the board would tell it: the plan, then one at a time.
  const checks = result?.checks ?? [];
  emit({ type: "self-test-progress", key, step: "planned", checks: checks.map((c) => c.id) });
  for (const check of checks) {
    emit({ type: "self-test-progress", key, step: "started", check: check.id });
    await sleep(check.id === "fan" ? 1800 : 450);
    emit({ type: "self-test-progress", key, step: "finished", check: check.id, status: check.status, message: check.message });
  }
  const run: SelfTestRecord = {
    board_serial: boardOf(device),
    device: key,
    at_ms: Date.now(),
    trigger,
    report: result,
    error: null,
  };
  keep(device, run);
  return run;
}

export function lastSelftest(key: DeviceKey): SelfTestRecord | null {
  const device = simDevice(key);
  return (device && selftests.get(boardOf(device))) ?? null;
}

/** After a verified flash or update: test the board once it runs again. */
export function selftestWhenBack(board: string) {
  const started = Date.now();
  const tick = () => {
    const running = fleet.find(
      (d) => d.mode === "normal" && d.online && boardOf(d) === board && d.caps?.includes("self-test"),
    );
    if (running && inventory.get(keyString(running.key))?.presence === "online") {
      void runSelftest(running.key, "after-update").catch(() => {});
    } else if (Date.now() - started < 30_000) {
      setTimeout(tick, 1000);
    }
  };
  setTimeout(tick, 1500);
}

/** A result from "earlier" for the A/B board, so the card has something to show. */
export function seedSelftests() {
  const device = fleet.find((d) => d.caps?.includes("self-test"));
  if (!device || selftests.size > 0) return;
  selftests.set(boardOf(device), {
    board_serial: boardOf(device),
    device: device.key,
    at_ms: Date.now() - 2.5 * 3_600_000,
    trigger: "after-update",
    report: { ...report(device), at: Math.floor((Date.now() - 2.5 * 3_600_000) / 1000) },
    error: null,
  });
}
