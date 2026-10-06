// Lays out the connection map: this computer on the left, devices plugged
// into it in the next column, and devices reached through a gateway further
// right. Pure geometry; the component draws it.

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

function edgeLabel(record: DeviceRecord): string {
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
      return record.identity.attributes?.bus?.split(" ")[0] ?? "";
  }
}

const byName = (a: DeviceRecord, b: DeviceRecord) =>
  (a.label ?? a.identity.name ?? a.key.serial).localeCompare(b.label ?? b.identity.name ?? b.key.serial, undefined, { numeric: true });

export function layoutMap(records: DeviceRecord[]): { nodes: MapNode[]; edges: MapEdge[]; rows: number; depth: number } {
  const ids = new Set(records.map((r) => keyString(r.key)));
  const children = new Map<string, DeviceRecord[]>();
  for (const record of records) {
    const via = record.link_kind.kind === "gateway" ? keyString(record.link_kind.via) : HOST;
    // A gateway outside this view: hang the device off this computer.
    const parent = via !== HOST && ids.has(via) ? via : HOST;
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
        edges.push({ from: id, to: kidId, label: edgeLabel(kid), online: kid.presence === "online" && (record?.presence ?? "online") === "online" });
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
