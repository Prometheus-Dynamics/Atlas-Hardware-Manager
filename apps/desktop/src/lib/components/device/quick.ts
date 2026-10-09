import type { DeviceAction } from "#lib/api/client.ts";

/** The actions the device header shows as quick buttons: the first three
 * everyday ones. Update controls belong with the update (Overview's Now,
 * Software), destructive ones with Controls. */
export function quickActions(actions: DeviceAction[]): DeviceAction[] {
  return actions.filter((a) => !a.destructive && !a.id.startsWith("update.")).slice(0, 3);
}
