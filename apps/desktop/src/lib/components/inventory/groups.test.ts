// Devices behind a gateway nest under it (bun test): a SystemCore with a
// Raze behind HeliOS and a board in USB boot behind its USB agent.
import { expect, mock, test } from "bun:test";
import type { DeviceKey, DeviceRecord, LinkKind } from "#lib/api/types.ts";

mock.module("#lib/api/client.ts", () => ({
  keyString: (key: { family: string; serial: string }) => `${key.family}:${key.serial}`,
}));
const { nestBehind } = await import("./groups.ts");

function record(family: string, serial: string, link_kind: LinkKind): DeviceRecord {
  return {
    key: { family, serial },
    identity: { model: family, mode: "normal", versions: {}, name: serial, link: "x", address: "", attributes: {} },
    link_kind,
    capabilities: ["info"],
    presence: "online",
    first_seen_ms: 0,
    last_seen_ms: 0,
    label: null,
    robot: null,
  } as unknown as DeviceRecord;
}

const via = (key: DeviceKey): LinkKind => ({ kind: "gateway", via: key });
const CORE: DeviceKey = { family: "systemcore", serial: "6390" };
const RAZE: DeviceKey = { family: "raze", serial: "0708c973b00a37bc" };

const names = (rows: { record: DeviceRecord; depth: number }[]) => rows.map((row) => `${"  ".repeat(row.depth)}${row.record.key.serial}`);

test("a SystemCore's boards come right under it, one level in", () => {
  const rows = nestBehind([
    record("raze", RAZE.serial, via(CORE)),
    record("other", "laptop-cam", { kind: "usb-network" }),
    record("systemcore", CORE.serial, { kind: "ethernet" }),
    record("rpi", "e5226d57", via(CORE)),
  ]);
  expect(names(rows)).toEqual(["laptop-cam", "6390", "  0708c973b00a37bc", "  e5226d57"]);
});

test("deeper gateways nest deeper; a missing gateway leaves its device in place", () => {
  const behindRaze: DeviceKey = { family: "cam", serial: "usb-cam" };
  const rows = nestBehind([
    record("systemcore", CORE.serial, { kind: "ethernet" }),
    record("raze", RAZE.serial, via(CORE)),
    record("cam", behindRaze.serial, via(RAZE)),
    record("rpi", "orphan", via({ family: "systemcore", serial: "9999" })),
  ]);
  expect(names(rows)).toEqual(["6390", "  0708c973b00a37bc", "    usb-cam", "orphan"]);
});

test("a loop of gateways keeps every device", () => {
  const a: DeviceKey = { family: "x", serial: "a" };
  const b: DeviceKey = { family: "x", serial: "b" };
  const rows = nestBehind([record("x", "a", via(b)), record("x", "b", via(a))]);
  expect(rows.map((row) => row.record.key.serial).sort()).toEqual(["a", "b"]);
});
