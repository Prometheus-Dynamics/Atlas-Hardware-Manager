// Saving a device's support bundle: ask where, then let the backend write it.

import { api, errorText, pickSavePath, type DeviceRecord } from "$lib/api/client";
import { deviceName } from "$lib/format";
import { toasts } from "$lib/stores/toasts.svelte";

export async function saveSupportBundle(record: DeviceRecord): Promise<void> {
  const stamp = new Date().toISOString().slice(0, 16).replace(/[:T]/g, "-");
  const safe = deviceName(record).replace(/[^\w.-]+/g, "-");
  const path = await pickSavePath(`atlas-${safe}-${stamp}.txt`);
  if (!path) return;
  try {
    await api.saveSupportBundle(record.key, path);
    toasts.success(`Saved the support bundle to ${path}.`);
  } catch (error) {
    toasts.error(`Couldn't save the support bundle: ${errorText(error)}`);
  }
}
