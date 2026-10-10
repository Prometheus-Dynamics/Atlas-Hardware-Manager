// The NetworkTables page's types (src-tauri nt.rs). Re-exported from types.ts.

/** An NT4 value as `{type, value}` (the NT4 type string: double, string[], …). */
export type NtValue =
  | { type: "boolean"; value: boolean }
  | { type: "double" | "float" | "int"; value: number }
  | { type: "string"; value: string }
  | { type: "raw"; value: number[] }
  | { type: "boolean[]"; value: boolean[] }
  | { type: "double[]" | "float[]" | "int[]"; value: number[] }
  | { type: "string[]"; value: string[] };

export interface NtTopic {
  name: string;
  type: string;
  properties: Record<string, unknown>;
}

/** What a viewer sends: its connection, topics that came and went, values (server clock, µs). */
export type NtFrame =
  | { type: "status"; connected: boolean; host: string; port: number; reason: string | null }
  | { type: "topics"; announced: NtTopic[]; gone: string[] }
  | { type: "values"; values: { name: string; t_us: number; value: NtValue }[] };

export interface NtServerTopic {
  name: string;
  type: string;
  value: NtValue | null;
  t_us: number | null;
  /** local: made here; client: published by `publisher`; unpublished: kept without one. */
  owner: "local" | "client" | "unpublished";
  publisher: string | null;
  persistent: boolean;
}

export interface NtServerClient {
  id: number;
  name: string;
  subscriptions: number;
  publications: number;
}

export interface NtServerInfo {
  port: number;
  /** This computer's addresses the server answers on. */
  addresses: string[];
  topics: NtServerTopic[];
  clients: NtServerClient[];
  time_sync: NtTimeSync;
}

/** PhotonVision time sync: Atlas answering the cameras' pings (UDP 5810), and what each camera publishes about it. */
export interface NtTimeSync {
  port: number;
  /** Answering pings; false when the port couldn't be bound (`error`). */
  listening: boolean;
  error: string | null;
  /** Addresses that pinged, with the pongs sent (last_ms: this computer's clock). */
  peers: { address: string; pongs: number; last_ms: number }[];
  /** Each camera's own view (µs), from /photonvision/.timesync/<host>/. */
  cameras: {
    name: string;
    offset_us: number | null;
    rtt2_us: number | null;
    pings: number | null;
    pongs: number | null;
    last_pong_us: number | null;
  }[];
}

export type NtServerFrame =
  | { type: "changed" }
  | { type: "wrote"; client: string; name: string; value: NtValue }
  | { type: "warning"; message: string };

/** A camera Atlas knows, and this computer's address it reaches. */
export interface NtCameraAddress {
  name: string;
  device_ip: string;
  address: string;
}
