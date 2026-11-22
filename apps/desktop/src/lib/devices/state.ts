// What a person is told about a device's state: one word (or a few) and one tone. The tones are
// the design language's (docs/design-language.md, "Chip"): ok is running, warn needs you later,
// err failed, info is seen or removable, acc needs you now, off is offline.

import type { Device } from "../api/types";

export type Tone = "ok" | "warn" | "err" | "info" | "acc" | "off";
export type StateWord = { tone: Tone; word: string; /** the dot beside the word */ dot: Tone | "ring" };

export function stateOf(device: Device): StateWord {
  if (device.presence === "offline") return { tone: "off", word: "Offline", dot: "off" };
  const { state, lemnos } = device.identity;
  switch (state) {
    case "running":
      if (lemnos.state === "not-answering") return { tone: "err", word: "Lemnos not answering", dot: "err" };
      if (lemnos.state === "revision-mismatch") return { tone: "warn", word: "Needs an update", dot: "warn" };
      return { tone: "ok", word: "Running", dot: "ok" };
    case "recovery":
      return { tone: "warn", word: "Recovery", dot: "warn" };
    case "boot-rom":
      return { tone: "acc", word: "Boot ROM, waiting", dot: "acc" };
    case "removable-disk":
      return { tone: "info", word: "Removable disk", dot: "info" };
    case "robot-device":
      return { tone: "ok", word: "Reporting", dot: "ok" };
  }
}

/** Where a device's row or tree entry leads. */
export function pageOf(device: Device): "board" | "flash" | "disk" {
  if (device.presence === "online" && device.identity.state === "boot-rom") return "flash";
  if (device.identity.state === "removable-disk") return "disk";
  return "board";
}
