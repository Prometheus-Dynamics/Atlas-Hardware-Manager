// What the Settings page is sent (apps/desktop/src-tauri/src/trust.rs, support.rs).

export type TrustedKey = {
  name: string;
  kind: string;
  key_id: string;
  owner: string;
  contact: string;
  purpose: "dev" | "production";
  created: string;
  expires: string | null;
  note: string;
  /** Where the entry came from: `built in`, or the file it was added from. */
  source: string;
  fingerprint: string;
  /** Added by the person, and so theirs to remove. */
  yours: boolean;
};

export type Check = { at_ms: number; subject: string; what: string; result: string; refused: boolean };

export type Trust = { keys: TrustedKey[]; problems: string[]; checks: Check[] };

export type SupportInfo = { log_path: string | null; log_bytes: number; app_bytes: number | null };

export type Bundle = { path: string; bytes: number; parts: { name: string; bytes: number }[] };

/** `minisign 1AB0…EAA6`: enough of a fingerprint to tell keys apart in a row. */
export function shortFingerprint(key: TrustedKey): string {
  const id = key.key_id.toUpperCase();
  return `${key.kind} ${id.slice(0, 4)}…${id.slice(-4)}`;
}

export const purposeWord = (key: TrustedKey) => (key.purpose === "dev" ? "dev and example builds" : "production releases");
