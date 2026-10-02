// Splits the visible inventory into robot groups (already sorted by ui).

import type { DeviceRecord } from "$lib/api/client";

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
