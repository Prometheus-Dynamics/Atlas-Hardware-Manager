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
    case "string": {
      // A JSON object or list (HeliOS's pipeline outputs) reads as JSON, not
      // as an escaped string.
      const json = jsonOf(value.value);
      return json === undefined ? JSON.stringify(value.value) : JSON.stringify(json);
    }
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

/** A string's JSON when it holds an object or a list, else undefined. */
export function jsonOf(text: string): unknown {
  const head = text.trimStart()[0];
  if (head !== "{" && head !== "[") return undefined;
  try {
    return JSON.parse(text);
  } catch {
    return undefined;
  }
}

const scalar = (v: unknown) =>
  typeof v === "number" ? short(v) : typeof v === "string" ? v : typeof v === "boolean" ? String(v) : v === null ? "null" : null;

/** JSON in one short line: `3 · ids 1, 3, 5`, `x 1.2, y 0.4, theta 12`, `none`. */
export function summarizeJson(json: unknown, keys = 3): string {
  if (json === null || json === undefined) return "none";
  if (Array.isArray(json)) {
    if (json.length === 0) return "none";
    // What names each item: an id (tags), else a class or label (objects).
    const pick = (keys: string[]) =>
      json
        .map((item) => (item && typeof item === "object" ? keys.map((k) => (item as Record<string, unknown>)[k]).find((v) => v !== undefined) : undefined))
        .filter((v) => v !== undefined && v !== null && scalar(v) !== null);
    const ids = pick(["id", "fiducial_id", "fiducialId"]);
    const names = ids.length ? ids : pick(["class", "label", "name"]);
    if (names.length === 0) return `${json.length} ${json.length === 1 ? "item" : "items"}`;
    const shown = names.slice(0, 5).map(scalar).join(", ");
    return `${json.length} · ${ids.length ? "ids " : ""}${shown}${names.length > 5 ? ", …" : ""}`;
  }
  if (typeof json === "object") {
    const entries = Object.entries(json);
    // HeliOS wraps an output in one key ({"detections": [...]}, {"value": []}): the inside says it.
    if (entries.length === 1 && entries[0][1] !== null && typeof entries[0][1] === "object") return summarizeJson(entries[0][1], keys);
    const parts: string[] = [];
    for (const [key, v] of entries) {
      // An empty field (`"value": null` beside a status) says nothing.
      const text = v === null ? null : Array.isArray(v) ? `${v.length}` : scalar(v);
      if (text === null) continue;
      parts.push(`${key} ${text}`);
      if (parts.length >= keys) break;
    }
    return parts.join(", ") || `${entries.length} fields`;
  }
  return scalar(json) ?? "";
}

/** The numbers HeliOS publishes next to each pipeline's outputs. */
const PIPELINE_STATS = new Set(["latency_ms", "fps", "capture_time_us"]);

/** A HeliOS pipeline: <prefix>/<name>/{fps,latency_ms,capture_time_us} and its outputs. */
export interface Pipeline {
  /** Its folder, e.g. `/helios/default/`. */
  path: string;
  name: string;
  fps: number | null;
  latencyMs: number | null;
  /** The frame's age when its outputs were published (ms, on the server's clock), from capture_time_us. */
  ageMs: number | null;
  /** Its outputs' topics (every other topic directly in its folder). */
  outputs: string[];
}

/**
 * The pipelines among `topics`: each folder holding an `fps` and a
 * `latency_ms` (HeliOS's prefix is a setting, "/helios" by default).
 * `t_us` is when the server got each value, on its clock.
 */
export function pipelinesOf(topics: Map<string, { value: NtValue | null; t_us?: number }>): Pipeline[] {
  const folders = new Map<string, Pipeline>();
  const folderOf = (name: string) => name.slice(0, name.lastIndexOf("/") + 1);
  for (const name of topics.keys()) {
    if (!name.endsWith("/fps")) continue;
    const path = folderOf(name);
    if (!topics.has(`${path}latency_ms`)) continue;
    folders.set(path, { path, name: path.split("/").filter(Boolean).at(-1) ?? path, fps: null, latencyMs: null, ageMs: null, outputs: [] });
  }
  if (folders.size === 0) return [];
  for (const [name, topic] of topics) {
    const pipeline = folders.get(folderOf(name));
    if (!pipeline) continue;
    const leaf = name.slice(pipeline.path.length);
    if (leaf === "fps") pipeline.fps = numberOf(topic.value);
    else if (leaf === "latency_ms") pipeline.latencyMs = numberOf(topic.value);
    else if (leaf === "capture_time_us") {
      const captured = numberOf(topic.value);
      if (captured !== null && topic.t_us) pipeline.ageMs = Math.max(0, (topic.t_us - captured) / 1000);
    } else if (!PIPELINE_STATS.has(leaf)) pipeline.outputs.push(name);
  }
  return [...folders.values()].map((p) => ({ ...p, outputs: p.outputs.sort() })).sort((a, b) => a.path.localeCompare(b.path));
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
