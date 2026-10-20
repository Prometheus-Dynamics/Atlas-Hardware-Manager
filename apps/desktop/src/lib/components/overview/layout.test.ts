// The connection map's topology (bun test): devices hang off the path Atlas
// reached them on. The bench as found on team 6390's robot: this computer on
// the USB of the Raze "right", which routes onto the robot network, where
// the radio, the roboRIO and the Razes "left" and "front" are.
import { expect, mock, test } from "bun:test";
import type { DeviceKey, DeviceRecord, LinkKind } from "#lib/api/types.ts";

mock.module("#lib/api/client.ts", () => ({
  keyString: (key: { family: string; serial: string }) => `${key.family}:${key.serial}`,
}));
const { HOST, layoutMap, parentOf } = await import("./layout.ts");

function record(family: string, serial: string, name: string, address: string, link_kind: LinkKind): DeviceRecord {
  return {
    key: { family, serial },
    identity: { model: family, mode: "normal", versions: {}, name, link: "x", address, attributes: {} },
    link_kind,
    capabilities: ["info"],
    presence: "online",
    first_seen_ms: 0,
    last_seen_ms: 0,
    label: null,
    robot: null,
  } as unknown as DeviceRecord;
}

const RIGHT: DeviceKey = { family: "raze", serial: "80cf24535f30fd87" };
const via = (key: DeviceKey): LinkKind => ({ kind: "gateway", via: key });

const right = record("raze", RIGHT.serial, "right", "http://172.31.167.9:5899/.well-known/pd-device", { kind: "usb-network" });
const left = record("raze", "2da7d9c5e9ff7b61", "left", "http://10.63.90.10:5899/.well-known/pd-device", via(RIGHT));
const front = record("raze", "6acf6699775f28dd", "front", "http://10.63.90.11:5899/.well-known/pd-device", via(RIGHT));
const radio = record("frc-radio", "6390", "radio-6390", "10.63.90.4", via(RIGHT));
const rio = record("roborio", "6390", "roboRIO-6390-FRC", "10.63.90.2", via(RIGHT));
const bench = [right, left, front, radio, rio];

test("devices behind a board's USB route hang off the radio, the radio off the board", () => {
  expect(parentOf(right, bench)).toBe(HOST);
  expect(parentOf(radio, bench)).toBe("raze:80cf24535f30fd87");
  for (const device of [left, front, rio]) expect(parentOf(device, bench)).toBe("frc-radio:6390");

  const { edges, depth } = layoutMap(bench);
  const edge = (to: string) => edges.find((e) => e.to === to);
  expect(edge("raze:80cf24535f30fd87")).toMatchObject({ from: HOST, label: "USB" });
  expect(edge("frc-radio:6390")).toMatchObject({ from: "raze:80cf24535f30fd87", label: "Robot net" });
  expect(edge("roborio:6390")).toMatchObject({ from: "frc-radio:6390", label: "Network" });
  expect(edge("raze:2da7d9c5e9ff7b61")?.from).toBe("frc-radio:6390");
  expect(depth).toBe(3);
});

test("without a radio, routed devices hang off the board", () => {
  const noRadio = [right, left, rio];
  expect(parentOf(left, noRadio)).toBe("raze:80cf24535f30fd87");
  const { edges } = layoutMap(noRadio);
  expect(edges.find((e) => e.to === "roborio:6390")).toMatchObject({ from: "raze:80cf24535f30fd87", label: "Robot net" });
});

test("a robot network this computer is on directly: its devices off the radio, the radio off this computer", () => {
  const radioDirect = record("frc-radio", "6390", "radio-6390", "10.63.90.4", { kind: "ethernet" });
  const camDirect = record("raze", "a", "cam", "http://10.63.90.11:5899/.well-known/pd-device", { kind: "ethernet" });
  const elsewhere = record("raze", "b", "lab", "http://192.168.1.20:5899/.well-known/pd-device", { kind: "ethernet" });
  const all = [radioDirect, camDirect, elsewhere];
  expect(parentOf(radioDirect, all)).toBe(HOST);
  expect(parentOf(camDirect, all)).toBe("frc-radio:6390");
  expect(parentOf(elsewhere, all)).toBe(HOST);
});

test("a device behind a gateway that isn't shown hangs off this computer", () => {
  expect(parentOf(left, [left])).toBe(HOST);
  expect(layoutMap([left]).edges).toEqual([{ from: HOST, to: "raze:2da7d9c5e9ff7b61", label: "Robot net", online: true }]);
});
