// The inventory as the tree and the list group it (docs/mockups/devices.html): a robot with its
// own devices and the boards behind its controller, the boards on this computer, removable
// disks, and what is offline. One record per board, whatever links it is seen on.

import type { Device, Inventory } from "../api/types";
import { pageOf } from "./state";

export type RobotEntry = {
  device: Device;
  /** 0 for the robot's own devices, 1 for a board reached through one of them. */
  depth: number;
  page: "board" | "flash" | "disk" | "systemcore";
};

export type RobotGroup = { name: string; entries: RobotEntry[] };

export type Groups = {
  robots: RobotGroup[];
  /** Online boards that are in no robot. */
  boards: Device[];
  disks: Device[];
  offline: Device[];
};

const byName = (a: Device, b: Device) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id);

/** The device a record is reached through, when its followed link is a gateway. */
export function gatewayOf(device: Device): string | undefined {
  const followed = device.links.find((sighting) => sighting.link === device.identity.link);
  if (followed?.kind.kind !== "gateway") return undefined;
  const via = followed.kind.via as { family?: string; serial?: string; node?: string };
  return via.family !== undefined ? `${via.family}:${via.serial}` : via.node;
}

/** Whether a record is a robot's controller: a robot device that relays to boards. */
export function isController(device: Device): boolean {
  return device.identity.state === "robot-device" && device.shown.includes("gateway");
}

export function tree(inventory: Inventory): Groups {
  const devices = [...inventory.devices].sort(byName);
  const online = devices.filter((device) => device.presence === "online");
  const robots: RobotGroup[] = [];
  const names = new Set([...inventory.robots.map((robot) => robot.name), ...devices.flatMap((d) => (d.robot ? [d.robot] : []))]);
  for (const name of [...names].sort((a, b) => a.localeCompare(b))) {
    const members = devices.filter((device) => device.robot === name);
    const entries: RobotEntry[] = [];
    const behind = (parent: Device) => members.filter((device) => gatewayOf(device) === parent.id);
    for (const device of members.filter((member) => gatewayOf(member) === undefined || !members.some((m) => m.id === gatewayOf(member)))) {
      entries.push({ device, depth: 0, page: isController(device) ? "systemcore" : pageOf(device) });
      for (const child of behind(device)) entries.push({ device: child, depth: 1, page: pageOf(child) });
    }
    robots.push({ name, entries });
  }
  const loose = online.filter((device) => device.robot === null);
  return {
    robots,
    boards: loose.filter((device) => device.identity.state !== "removable-disk"),
    disks: loose.filter((device) => device.identity.state === "removable-disk"),
    offline: devices.filter((device) => device.presence === "offline" && device.robot === null),
  };
}
