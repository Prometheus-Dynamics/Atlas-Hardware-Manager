// What a robot's own device says about itself, in words. The robot drivers read these without
// a login (crates/drivers/robot): a model, a firmware or library version, and a few facts such
// as the radio's mode and each band's link. They report no stream of readings.

import type { Device, Inventory } from "../api/types";
import { gatewayOf } from "../devices/tree";

const byId = (a: Device, b: Device) => a.id.localeCompare(b.id);

/** The devices of one robot that are the robot's own, controller first. */
export function ownDevices(inventory: Inventory, robot: string): Device[] {
  return inventory.devices
    .filter((device) => device.robot === robot && device.identity.state === "robot-device")
    .sort((a, b) => Number(b.role === "controller") - Number(a.role === "controller") || byId(a, b));
}

/** The robot's controller that does not run Orion yet, when the robot drivers found one. */
export function controllerOf(inventory: Inventory, robot: string): Device | undefined {
  return inventory.devices.find((device) => device.robot === robot && device.role === "controller");
}

/** The boards reached through a device. */
export function behind(inventory: Inventory, parent: Device): Device[] {
  return inventory.devices.filter((device) => gatewayOf(device) === parent.id).sort(byId);
}

/** The faint line under a robot device's name: what it is and its version. */
export function whatOf(device: Device): string {
  const { model, versions } = device.identity;
  const version = Object.entries(versions)
    .map(([name, value]) => `${name} ${value}`)
    .join(" · ");
  return [device.model_name ?? model, version].filter(Boolean).join(" · ") || "reported by the robot";
}

/** The facts a robot device reported, as one line: `robot radio · channel 53 · 6 GHz linked, -58 dBm, good`. */
export function reported(device: Device): string {
  const a = device.identity.attributes;
  const out: string[] = [];
  if (a["radio.mode"]) out.push(a["radio.mode"]);
  if (a["radio.channel"]) out.push(`channel ${a["radio.channel"]}`);
  for (const band of ["2.4", "6"]) {
    const link = a[`radio.${band}.link`];
    if (link && link !== "off") out.push(`${band} GHz ${link}`);
  }
  if (a["robot_code"]) out.push(`robot code ${a["robot_code"]}`);
  if (a["services"]) out.push(a["services"].split(",").join(", "));
  if (out.length === 0 && a["team"]) out.push(`team ${a["team"]}`);
  return out.join(" · ");
}
