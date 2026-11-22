// The `flash` topic (apps/desktop/src-tauri/src/flash.rs): the chosen image, the loader run, the
// disk write, and what was flashed here before.

export type FlashImage =
  | { state: "reading"; path: string; file: string; bytes: number }
  | { state: "ready"; path: string; file: string; bytes: number; raw_bytes: number; board: string; sha256: string; signed_by: string | null }
  | { state: "refused"; path: string; file: string; bytes: number; words: string };

export type FlashLine = { at_ms: number; level: "info" | "warn" | "err"; text: string };

export type RunStatus = "queued" | "running" | "retrying" | "stopped" | "loaded" | "failed" | "cancelled" | "interrupted" | "unknown";

export type FlashRun = {
  job: number;
  device: string;
  name: string;
  family: string;
  port: string;
  status: RunStatus;
  attempt: number;
  error: string;
  retry_in_ms: number;
  retry_at_ms: number;
  started_ms: number;
  ended_ms: number | null;
  loaded: { words: string; facts: [string, string][] } | null;
  log: FlashLine[];
};

export type Recent = { name: string; at_ms: number; result: string; retried: boolean };

export type DiskRun = {
  device: string;
  name: string;
  file: string;
  status: "writing" | "done" | "failed" | "cancelled";
  phase: "checking" | "preparing" | "writing" | "dropped" | "reattaching" | "resuming";
  input_done: number;
  input_total: number;
  written: number;
  drops: number;
  words: string;
  report: { bytes_written: number; sha256: string; verified: boolean; seconds: number } | null;
  started_ms: number;
};

export type Rom = { family: string; label: string; loader: string };

export type FlashView = {
  image: FlashImage | null;
  run: FlashRun | null;
  recent: Recent[];
  disk: DiskRun | null;
  roms: Rom[];
  can_write_disks: boolean;
  backoff_cap_ms: number;
};

/** What the Signature line says: what the file carries, and what the security switch makes of it.
 *  Atlas does not check a signing record itself; a board does, when security is on. */
export function signatureWords(image: FlashImage | null, security: boolean): string {
  if (image?.state !== "ready") return "·";
  const key = image.signed_by ? `signed, key ${image.signed_by.slice(0, 8)}…` : "unsigned";
  if (!security) return image.signed_by ? `${key} · not checked (security is off)` : "not checked (security is off)";
  return image.signed_by ? `${key} · the board checks it` : "unsigned: a board with security on refuses it";
}

/** How a finished flash is named under "Recent flashes here". */
export function recentWord(recent: Recent): { tone: "ok" | "warn" | "err" | "off"; word: string } {
  switch (recent.result) {
    case "loaded":
      return recent.retried ? { tone: "warn", word: "retried" } : { tone: "ok", word: "loader ran" };
    case "written":
      return { tone: "ok", word: "written" };
    case "cancelled":
      return { tone: "off", word: "cancelled" };
    default:
      return { tone: "err", word: "failed" };
  }
}
