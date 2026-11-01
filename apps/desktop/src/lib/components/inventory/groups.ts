// Splits the visible inventory into robot groups (already sorted by ui), and
// within a group puts each device under the one Atlas reaches it through.

import { keyString, type DeviceRecord } from "#lib/api/client.ts";

export interface DeviceGroup {
  robot: string | null;
  records: DeviceRecord[];
}

export function groupByRobot(records: DeviceRecord[], grouped: boolean): DeviceGroup[] {
  if (!grouped) return [{ robot: null, records }];
  const groups: DeviceGroup[] = [];
  for (const record of records) {
    const last = groups[groups.length - 1];
    if (last && last.robot === record.robot) last.records.push(record);
    else groups.push({ robot: record.robot, records: [record] });
  }
  return groups;
}

export interface NestedRow {
  record: DeviceRecord;
  /** 0 for a device reached directly; 1 behind a gateway shown above it; ... */
  depth: number;
}

/** The gateway a device is reached through, as a key string. */
function gatewayOf(record: DeviceRecord): string | null {
  return record.link_kind.kind === "gateway" ? keyString(record.link_kind.via) : null;
}

/**
 * The records in their order, except that each device reached through a
 * gateway that's also listed comes right after that gateway (and its other
 * devices), one level deeper: a SystemCore's boards under the SystemCore.
 * A device whose gateway isn't listed stays where it was.
 */
export function nestBehind(records: DeviceRecord[]): NestedRow[] {
  const listed = new Set(records.map((record) => keyString(record.key)));
  const children = new Map<string, DeviceRecord[]>();
  const roots: DeviceRecord[] = [];
  for (const record of records) {
    const via = gatewayOf(record);
    if (via && via !== keyString(record.key) && listed.has(via)) {
      const list = children.get(via) ?? [];
      list.push(record);
      children.set(via, list);
    } else {
      roots.push(record);
    }
  }
  const rows: NestedRow[] = [];
  const placed = new Set<string>();
  const place = (record: DeviceRecord, depth: number) => {
    const id = keyString(record.key);
    if (placed.has(id)) return;
    placed.add(id);
    rows.push({ record, depth });
    for (const child of children.get(id) ?? []) place(child, depth + 1);
  };
  for (const record of roots) place(record, 0);
  // A loop of gateways (each behind the other) has no root: keep its devices.
  for (const record of records) place(record, 0);
  return rows;
}
