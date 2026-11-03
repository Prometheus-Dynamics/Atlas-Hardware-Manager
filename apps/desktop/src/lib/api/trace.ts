// Atlas logs its own trouble. A command that fails, or hasn't answered
// after STUCK_MS, goes to atlas.log with an error id and the last things
// the user did (breadcrumbs); an error toast about it shows the same id,
// so a screenshot of the toast finds the log line.

import { Channel, invoke as tauriInvoke } from "@tauri-apps/api/core";

const STUCK_MS = 15_000;
const CRUMBS = 12;
/** A command failing the same way again within this reuses its id, unlogged. */
const REPEAT_MS = 60_000;

interface Failure {
  key: string;
  text: string;
  id: string;
  at: number;
}

const crumbs: string[] = [];
let failures: Failure[] = [];

/** Normalizes a rejected command into a displayable sentence. */
export function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}

const clock = () => new Date().toTimeString().slice(0, 8);
const newId = () =>
  Array.from(crypto.getRandomValues(new Uint8Array(3)), (b) => b.toString(16).padStart(2, "0")).join("");

/** Something the user did, kept for the next log line. */
export function crumb(what: string) {
  crumbs.push(`${clock()} ${what}`);
  if (crumbs.length > CRUMBS) crumbs.shift();
}

async function report(kind: string, message: string, id: string) {
  const after = crumbs.length ? ` | after: ${crumbs.join("; ")}` : "";
  try {
    await tauriInvoke("report_ui_error", { kind, message: `[${id}] ${message}${after}`, at: location.pathname });
  } catch {
    // Logging is best effort.
  }
}

/** Tauri's invoke, logging failures and stuck calls. */
export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const started = performance.now();
  const secs = () => ((performance.now() - started) / 1000).toFixed(1);
  // A call with a progress channel is long by design (a flash, an install).
  const long = args !== undefined && Object.values(args).some((v) => v instanceof Channel);
  let stuck: string | null = null;
  const timer = long
    ? undefined
    : setTimeout(() => {
        stuck = newId();
        void report("UI-STUCK", `${cmd} hasn't answered in ${STUCK_MS / 1000} s`, stuck);
      }, STUCK_MS);
  try {
    return await tauriInvoke<T>(cmd, args);
  } catch (error) {
    const text = errorText(error);
    const key = `${cmd}\n${text}`;
    const now = Date.now();
    failures = failures.filter((f) => now - f.at < REPEAT_MS);
    const seen = failures.find((f) => f.key === key);
    if (seen) {
      seen.at = now;
    } else {
      const id = newId();
      failures.push({ key, text, id, at: now });
      void report("UI-CALL", `${cmd} failed after ${secs()} s: ${text}`, id);
    }
    throw error;
  } finally {
    clearTimeout(timer);
    if (stuck) void report("UI-STUCK", `${cmd} answered after ${secs()} s`, stuck);
  }
}

/** The id of a recent failure this message is about, for its toast. */
export function errorId(message: string): string | null {
  let found: Failure | null = null;
  for (const f of failures) {
    if (f.text && message.includes(f.text) && (!found || f.at > found.at)) found = f;
  }
  return found?.id ?? null;
}

/** Breadcrumbs from clicks on controls (pages add theirs on navigation). */
export function startCrumbs() {
  document.addEventListener(
    "click",
    (event) => {
      const el = (event.target as Element | null)?.closest("button, a, [role=tab], [role=menuitem], [role=switch]");
      if (!el) return;
      const label = el.getAttribute("aria-label") || el.getAttribute("title") || el.textContent || el.tagName;
      crumb(`click "${label.trim().replace(/\s+/g, " ").slice(0, 40)}"`);
    },
    { capture: true },
  );
}
