// What a robot's buttons do, shared by its card and its detail view.
import { goto } from "$app/navigation";
import { api } from "#lib/api/client.ts";
import { system } from "#lib/stores/system.svelte.ts";
import { toasts } from "#lib/stores/toasts.svelte.ts";
import { ui } from "#lib/stores/ui.svelte.ts";

/** Opens the update that brings every role to its target, if one is needed. */
export async function makeReady(name: string): Promise<void> {
  const request = await api.robotUpdateRequest(name, system.settings?.staged_default ?? null);
  if (!request) {
    toasts.success(`${name} is already ready.`);
    return;
  }
  ui.openUpdate(request, `Make ${name} ready`);
}

/** The Devices page, filtered to this robot. */
export function showDevices(name: string): void {
  ui.robot = name;
  void goto("/devices");
}
