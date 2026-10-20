// Lays out the connection map: this computer on the left, devices plugged
// into it in the next column, and devices reached through a gateway further
// right. Pure geometry; the component draws it.
//
// The path is the one Atlas reached each device on: a device found behind a
// board's USB route (link `gateway`, via that board) hangs off the board,
// and when the robot network's radio was found the same way, off the radio
// (it is the robot network's switch); the radio hangs off the board. The
// same for a robot network this computer is on directly: its devices hang
// off the radio, the radio off this computer.

import { keyString, type DeviceRecord } from "#lib/api/client.ts";

export const HOST = "host";

export interface MapNode {
  id: string;
  record: DeviceRecord | null;
  depth: number;
  /** Centre, in layout units (row index scaled by the caller). */
  row: number;
}

export interface MapEdge {
  from: string;
  to: string;
  label: string;
  online: boolean;
}

/** The robot radio's family (atlas_driver::robot_net::RADIO_FAMILY). */
export const RADIO_FAMILY = "frc-radio";

const isRadio = (record: DeviceRecord) => record.key.family === RADIO_FAMILY;

/** The device's IPv4 address, from its address or URL. */
function ipv4(record: DeviceRecord): string | null {
  const host = (record.identity.address ?? "").replace(/^\w+:\/\//, "").replace(/[/:].*$/, "");
  return /^\d+\.\d+\.\d+\.\d+$/.test(host) ? host : null;
}

const net24 = (ip: string) => ip.split(".").slice(0, 3).join(".");

/** The radio a device sits behind: one reached the same way (behind the same board, or directly) on the same robot network. */
function radioFor(record: DeviceRecord, radios: DeviceRecord[]): DeviceRecord | null {
  if (isRadio(record)) return null;
  const via = record.link_kind.kind === "gateway" ? keyString(record.link_kind.via) : null;
  const ip = ipv4(record);
  return (
    radios.find((radio) => {
      const radioVia = radio.link_kind.kind === "gateway" ? keyString(radio.link_kind.via) : null;
      if (via !== null) return radioVia === via;
      // Directly on a robot network: the radio of that /24.
      const radioIp = ipv4(radio);
      return radioVia === null && record.link_kind.kind === "ethernet" && radio.link_kind.kind === "ethernet" && !!ip && !!radioIp && net24(ip) === net24(radioIp);
    }) ?? null
  );
}

/** Which node each device hangs off: its gateway, the robot network's radio, or this computer. */
export function parentOf(record: DeviceRecord, records: DeviceRecord[]): string {
  const ids = new Set(records.map((r) => keyString(r.key)));
  const radio = radioFor(record, records.filter(isRadio));
  if (radio) return keyString(radio.key);
  const via = record.link_kind.kind === "gateway" ? keyString(record.link_kind.via) : HOST;
  // A gateway outside this view: hang the device off this computer.
  return via !== HOST && ids.has(via) ? via : HOST;
}

function edgeLabel(record: DeviceRecord, parent: DeviceRecord | null): string {
  if (parent && isRadio(parent)) return "Network";
  switch (record.link_kind.kind) {
    case "usb-network":
      return "USB";
    case "usb-serial":
      return "Serial";
    case "usb-boot":
      return "USB boot";
    case "ethernet":
      return "Network";
    case "simulated":
      return "Sim";
    case "gateway":
      // A bus behind the gateway, or the robot network behind a board's USB route.
      return record.identity.attributes?.bus?.split(" ")[0] ?? "Robot net";
  }
}

const byName = (a: DeviceRecord, b: DeviceRecord) =>
  (a.label ?? a.identity.name ?? a.key.serial).localeCompare(b.label ?? b.identity.name ?? b.key.serial, undefined, { numeric: true });

export function layoutMap(records: DeviceRecord[]): { nodes: MapNode[]; edges: MapEdge[]; rows: number; depth: number } {
  const children = new Map<string, DeviceRecord[]>();
  for (const record of records) {
    const parent = parentOf(record, records);
    children.set(parent, [...(children.get(parent) ?? []), record]);
  }

  const nodes: MapNode[] = [];
  const edges: MapEdge[] = [];
  let nextRow = 0;
  let maxDepth = 0;
  const placed = new Set<string>();

  // Leaves take the next row; parents sit centred on their children.
  function place(id: string, record: DeviceRecord | null, depth: number): number {
    placed.add(id);
    maxDepth = Math.max(maxDepth, depth);
    const kids = (children.get(id) ?? []).filter((k) => !placed.has(keyString(k.key))).sort(byName);
    let row: number;
    if (kids.length === 0) {
      row = nextRow++;
    } else {
      const rows = kids.map((kid) => {
        const kidId = keyString(kid.key);
        edges.push({ from: id, to: kidId, label: edgeLabel(kid, record), online: kid.presence === "online" && (record?.presence ?? "online") === "online" });
        return place(kidId, kid, depth + 1);
      });
      row = (rows[0] + rows[rows.length - 1]) / 2;
    }
    nodes.push({ id, record, depth, row });
    return row;
  }

  place(HOST, null, 0);
  return { nodes, edges, rows: Math.max(1, nextRow), depth: maxDepth };
}
