export type NetworkSnapshotAccent = "primary" | "secondary" | "tertiary" | "success" | "warning" | "error";

export interface NetworkSnapshotMetric {
  label: string;
  value: number;
  unit: string;
  accent: NetworkSnapshotAccent;
}

export interface TopologyNote {
  id: string;
  title: string;
  detail: string;
}
