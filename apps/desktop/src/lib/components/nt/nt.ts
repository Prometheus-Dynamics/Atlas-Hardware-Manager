// NetworkTables helpers for the page: values as text and back, which
// topics graph, and the topic tree (names split on "/").

import type { NtValue } from "#lib/api/client.ts";

/** The types a topic can be created with here, in the order offered. */
export const TYPES = ["double", "int", "boolean", "string", "float", "double[]", "int[]", "boolean[]", "string[]"] as const;

export const isNumberType = (type: string) => type === "double" || type === "float" || type === "int";

/** A topic's number, for graphs (booleans graph as 0/1). */
export function numberOf(value: NtValue | null | undefined): number | null {
  if (!value) return null;
  if (value.type === "double" || value.type === "float" || value.type === "int") return value.value;
  if (value.type === "boolean") return value.value ? 1 : 0;
  return null;
}

const short = (n: number) => (Number.isInteger(n) ? String(n) : n.toLocaleString(undefined, { maximumFractionDigits: 4 }));

/** A value as one line: `2.4135`, `true`, `"Practice"`, `[3.2, 4.1, 10]`, `4 bytes`. */
export function formatNt(value: NtValue | null | undefined, limit = 6): string {
  if (!value) return "—";
  switch (value.type) {
    case "boolean":
      return value.value ? "true" : "false";
    case "double":
    case "float":
    case "int":
      return short(value.value);
    case "string":
      return JSON.stringify(value.value);
    case "raw":
      return `${value.value.length} bytes`;
    default: {
      const items = (value.value as unknown[]).map((item) =>
        typeof item === "number" ? short(item) : typeof item === "string" ? JSON.stringify(item) : String(item),
      );
      const shown = items.slice(0, limit).join(", ");
      return `[${shown}${items.length > limit ? `, … ${items.length - limit} more` : ""}]`;
    }
  }
}

/** Text as a value of `type`: numbers, true/false, a string, or a comma list (or JSON) for arrays. */
export function parseNt(type: string, text: string): NtValue {
  const trimmed = text.trim();
  const number = (s: string) => {
    const n = Number(s.trim());
    if (s.trim() === "" || !Number.isFinite(n)) throw `"${s.trim()}" isn't a number`;
    return n;
  };
  const integer = (s: string) => {
    const n = number(s);
    if (!Number.isInteger(n)) throw `"${s.trim()}" isn't a whole number`;
    return n;
  };
  const boolean = (s: string) => {
    const v = s.trim().toLowerCase();
    if (["true", "1", "yes", "on"].includes(v)) return true;
    if (["false", "0", "no", "off"].includes(v)) return false;
    throw `"${s.trim()}" isn't true or false`;
  };
  const list = (): string[] => {
    if (trimmed.startsWith("[")) {
      const parsed: unknown = JSON.parse(trimmed);
      if (!Array.isArray(parsed)) throw "not a list";
      return parsed.map((item) => (typeof item === "string" ? item : String(item)));
    }
    return trimmed === "" ? [] : trimmed.split(",").map((item) => item.trim());
  };
  switch (type) {
    case "boolean":
      return { type, value: boolean(trimmed) };
    case "double":
    case "float":
      return { type, value: number(trimmed) };
    case "int":
      return { type, value: integer(trimmed) };
    case "string":
      return { type, value: text };
    case "boolean[]":
      return { type, value: list().map(boolean) };
    case "double[]":
    case "float[]":
      return { type, value: list().map(number) };
    case "int[]":
      return { type, value: list().map(integer) };
    case "string[]":
      return { type, value: list() };
    default:
      throw `${type} topics can't be edited here`;
  }
}

/** A value as editable text (the inverse of parseNt). */
export function editText(value: NtValue | null | undefined): string {
  if (!value) return "";
  if (value.type === "string") return value.value;
  if (value.type === "raw") return "";
  if (Array.isArray(value.value)) return (value.value as unknown[]).map(String).join(", ");
  return String(value.value);
}

/** A row of the topic tree: a folder (a name prefix) or a topic. */
export interface TreeRow {
  path: string;
  label: string;
  depth: number;
  folder: boolean;
}

/**
 * The visible rows for `names` (sorted), with folders collapsed in
 * `collapsed`; with a search, the matching topics and their folders, open.
 */
export function treeRows(names: string[], collapsed: Set<string>, search: string): TreeRow[] {
  const needle = search.trim().toLowerCase();
  const shown = needle ? names.filter((name) => name.toLowerCase().includes(needle)) : names;
  const rows: TreeRow[] = [];
  const folders = new Set<string>();
  for (const name of [...shown].sort()) {
    const parts = name.split("/").filter(Boolean);
    let hidden = false;
    for (let i = 0; i < parts.length - 1; i++) {
      const path = `/${parts.slice(0, i + 1).join("/")}/`;
      if (!folders.has(path)) {
        folders.add(path);
        if (!hidden) rows.push({ path, label: parts[i], depth: i, folder: true });
      }
      if (!needle && collapsed.has(path)) hidden = true;
    }
    if (!hidden) rows.push({ path: name, label: parts.at(-1) ?? name, depth: Math.max(0, parts.length - 1), folder: false });
  }
  return rows;
}
