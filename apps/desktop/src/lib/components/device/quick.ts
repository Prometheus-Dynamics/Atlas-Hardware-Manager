import type { DeviceAction } from "#lib/api/client.ts";

/** The device's actions shown as header buttons: finding it, and what a
 * board in USB boot offers (open as disk, browse, eject, add a key).
 * Everything else is in the ⋯ menu (DeviceMenu), and what takes the
 * device offline is asked first there. */
const HEADER = ["locate", "identify", "open-as-disk", "browse-files", "eject", "add-ssh-key"];

export function quickActions(actions: DeviceAction[]): DeviceAction[] {
  return HEADER.flatMap((id) => actions.filter((a) => a.id === id && !a.destructive)).slice(0, 3);
}
