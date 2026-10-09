// A board's hardware as the board or Orion reports it, and the commands its
// devices take. Re-exported from types.ts.

/** One value a hardware device reports; null when it is unknown. */
export interface HardwareReading {
  name: string;
  value: number | null;
  unit: string;
}

/** A sensor, fan or other part of a board, as lemnosd reports it. */
export interface HardwareDevice {
  id: string;
  class: string;
  model: string;
  /** available, degraded, faulted or missing. */
  status: string;
  /** Why it isn't available, when the source says. */
  reason?: string | null;
  readings: HardwareReading[];
  /** Controls the device offers. */
  controls: HardwareControl[];
}

/**
 * One control of a board device, with what the source knows: its value now
 * and its range, in `unit`. A board's own snapshot names it only; Orion
 * describes it in full.
 */
export interface HardwareControl {
  name: string;
  value: number | null;
  min: number | null;
  max: number | null;
  unit: string;
}

/** A command to a board device (devices with `hardware-control`). */
export type HardwareCommand =
  | { command: "set"; control: string; value: number }
  /** Undoes this computer's writes: to `control`, or to every control. */
  | { command: "restore"; control: string | null }
  /** Hands a fan back to the board's own cooling. */
  | { command: "release" };

/** The board's hardware at one moment (Unix seconds). */
export interface HardwareSnapshot {
  at: number;
  devices: HardwareDevice[];
}
