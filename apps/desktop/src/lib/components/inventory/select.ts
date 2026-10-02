// Pointer selection shared by the card grid and the list.

import { ui } from "$lib/stores/ui.svelte";

/** Click opens; Ctrl/Cmd-click toggles; Shift-click selects a range. */
export function clickDevice(event: MouseEvent, id: string) {
  ui.focused = id;
  if (event.shiftKey) {
    event.preventDefault();
    ui.selectRange(id);
  } else if (event.ctrlKey || event.metaKey) {
    ui.toggle(id);
  } else {
    ui.openDevice(id);
  }
}

/** The selection check: Shift extends the range. */
export function clickCheck(event: MouseEvent, id: string) {
  event.stopPropagation();
  ui.focused = id;
  if (event.shiftKey) ui.selectRange(id);
  else ui.toggle(id);
}
