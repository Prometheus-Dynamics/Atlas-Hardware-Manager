// A board's case for the 3D view: `case.json` in its package under devices/,
// read at build time since the model files ship with the app. A board without
// one gets the plain block.
import { getContext, setContext } from "svelte";
import type { Quat } from "./imu.ts";

export interface BoardCase {
  /** Under static/models. */
  file: string;
  /** Takes case vectors to IMU vectors. */
  case_in_imu: Quat;
  /** LED 0's angle in the case's frame, from +x toward +y. */
  ring_led0_deg: number;
  /** Which way round the other LEDs go from it. */
  ring_direction: number;
}

const cases = import.meta.glob<BoardCase>("../../../../../../devices/*/case.json", {
  eager: true,
  import: "default",
});

export function boardCaseFor(model: string | null | undefined): BoardCase | null {
  if (!model) return null;
  const suffix = `/devices/${model.toLowerCase()}/case.json`;
  const path = Object.keys(cases).find((p) => p.endsWith(suffix));
  return path ? cases[path] : null;
}

const MODEL = Symbol("board-model");

/** The package of the board this part of the page shows (its key's family,
 * not identity.model, which is a display name), for the views under it. */
export function provideBoardModel(model: () => string | null | undefined): void {
  setContext(MODEL, model);
}

/** Call while a component starts; the getter follows the board shown. */
export function boardModel(): () => string | null | undefined {
  return getContext<(() => string | null | undefined) | undefined>(MODEL) ?? (() => null);
}
