// The topic `board:<id>` (apps/desktop/src-tauri/src/board.rs): one board as its page reads it.

export type BoardReading = {
  name: string;
  number: number | null;
  text: string | null;
  unit: string | null;
  kind: string | null;
  low: number | null;
  high: number | null;
  warn_low: number | null;
  warn_high: number | null;
  settable: boolean | null;
  history: number[];
};

export type BoardDevice = {
  name: string;
  shown: string;
  bus: string | null;
  kind: string | null;
  state: "running" | "retrying" | "off" | null;
  reason: string | null;
  health: string | null;
  listed: boolean;
  readings: BoardReading[];
};

export type BoardAction = { name: string; label: string | null; description: string | null; direct: boolean; args: number | null };

export type Slot = {
  id: string;
  product: string;
  board: string;
  version: string;
  display_name: string;
  good: string;
  signed: string;
  size: number;
  pinned: boolean;
  failed: boolean;
  failures: number;
  running: boolean;
  default: boolean;
  previous: boolean;
  trial: boolean;
};

export type SlotList = {
  running: string;
  default: string;
  previous: string;
  trial_slot: string;
  state: string;
  error: string;
  free_bytes: number;
  keep: number;
  slots: Slot[];
  trial: {
    slot: string;
    attempts_left: number | null;
    attempts: { n: number; time: string }[];
    last: string | null;
    last_time: string;
    reason: string;
    record_error: string | null;
  } | null;
  resolved: { slot: string; outcome: string; by: string; caller: string; reason: string; time: string } | null;
};

export type Slots =
  | { state: "ok"; list: SlotList }
  | { state: "none" }
  | { state: "not-reported" }
  | { state: "error"; error: string };

export type Install = {
  stage: "preparing" | "waiting" | "sending" | "installing" | "unknown" | "done" | "failed" | "cancelled";
  percent: number | null;
  detail: string;
  final: boolean;
  file: string;
  sent: number | null;
  total: number | null;
};

export type BoardView = {
  id: string;
  reach: "orion" | "none";
  read_ms: number;
  said: { what: string; ok: boolean; words: string; at_ms: number } | null;
  connection?: { live: true } | { live: false; reason: string; stale_ms: number };
  hardware?: BoardDevice[];
  logs?: { time_ms: number; text: string }[];
  logs_missed?: number;
  actions?: BoardAction[] | null;
  actions_error?: string;
  slots?: Slots | null;
  install?: Install;
};
