// Global keyboard shortcuts. Ignored while typing in a field.

import { goto } from "$app/navigation";
import { page } from "$app/state";
import { devices } from "$lib/stores/devices.svelte";
import { system } from "$lib/stores/system.svelte";
import { ui } from "$lib/stores/ui.svelte";

const ROUTES = ["/", "/devices", "/robots", "/jobs", "/releases", "/settings"];

function typing(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}

export function handleShortcut(event: KeyboardEvent) {
  if (event.key === "Escape") {
    if (ui.helpOpen) ui.helpOpen = false;
    else if (typing(event.target)) (event.target as HTMLElement).blur();
    else if (ui.panel) ui.close();
    else ui.selection.clear();
    return;
  }
  if (typing(event.target) || event.altKey || event.metaKey) return;

  const onInventory = page.url.pathname === "/devices";
  if (event.ctrlKey) {
    if (onInventory && event.key.toLowerCase() === "a") {
      event.preventDefault();
      ui.selectAllVisible();
    }
    return;
  }

  const index = ["1", "2", "3", "4", "5", "6"].indexOf(event.key);
  if (index >= 0) {
    event.preventDefault();
    void goto(ROUTES[index]);
    return;
  }

  switch (event.key) {
    case "?":
      ui.helpOpen = !ui.helpOpen;
      break;
    case "s":
    case "S":
      void devices.scanNow();
      break;
    default:
      if (onInventory) inventoryKey(event);
  }
}

function inventoryKey(event: KeyboardEvent) {
  // Buttons and links keep their own Space and Enter behavior.
  const onControl = event.target instanceof HTMLElement && event.target.closest("button, a");
  switch (event.key) {
    case "/":
      event.preventDefault();
      ui.filterInput?.focus();
      break;
    case "ArrowDown":
      event.preventDefault();
      ui.moveFocus(1);
      break;
    case "ArrowUp":
      event.preventDefault();
      ui.moveFocus(-1);
      break;
    case " ":
      if (onControl || !ui.focused) return;
      event.preventDefault();
      ui.toggle(ui.focused);
      break;
    case "Enter":
      if (onControl || !ui.focused) return;
      event.preventDefault();
      ui.openDevice(ui.focused);
      break;
    case "u":
    case "U":
      ui.updateSelection(system.settings?.staged_default ?? "auto");
      break;
  }
}
