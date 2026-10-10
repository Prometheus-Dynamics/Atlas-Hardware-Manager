// NetworkTables for the browser mock: any target is a robot's NT server
// with a few dozen topics (a PhotonVision camera among them) whose numbers
// move at 50 Hz, and the local server keeps its topics in memory; a
// simulated camera connects to it a moment after it starts.

import type { NtCameraAddress, NtFrame, NtServerFrame, NtServerInfo, NtServerTopic, NtTopic, NtValue } from "../types";

const t = () => performance.now() / 1000;
const num = (type: "double" | "int" | "float", value: number): NtValue => ({ type, value });

const ROBOT: Record<string, { type: string; value: () => NtValue }> = {
  "/FMSInfo/IsRedAlliance": { type: "boolean", value: () => ({ type: "boolean", value: true }) },
  "/FMSInfo/MatchNumber": { type: "int", value: () => num("int", 12) },
  "/FMSInfo/EventName": { type: "string", value: () => ({ type: "string", value: "Practice" }) },
  "/SmartDashboard/Drive/Speed": { type: "double", value: () => num("double", 2.4 + Math.sin(t() * 1.7) * 1.1) },
  "/SmartDashboard/Drive/Heading": { type: "double", value: () => num("double", ((t() * 20) % 360) - 180) },
  "/SmartDashboard/Arm/Angle": { type: "double", value: () => num("double", 45 + Math.sin(t() * 0.6) * 30) },
  "/SmartDashboard/Arm/AtSetpoint": { type: "boolean", value: () => ({ type: "boolean", value: Math.sin(t() * 0.6) > 0.8 }) },
  "/SmartDashboard/Battery": { type: "double", value: () => num("double", 12.4 - Math.abs(Math.sin(t() * 1.7)) * 1.3) },
  "/SmartDashboard/Auto/Selected": { type: "string", value: () => ({ type: "string", value: "Two piece amp" }) },
  "/Robot/Pose": { type: "double[]", value: () => ({ type: "double[]", value: [3.2 + Math.sin(t() / 3), 4.1 + Math.cos(t() / 3), (t() * 10) % 360] }) },
  "/photonvision/cam-front/hasTarget": { type: "boolean", value: () => ({ type: "boolean", value: Math.sin(t()) > -0.3 }) },
  "/photonvision/cam-front/targetYaw": { type: "double", value: () => num("double", Math.sin(t() * 0.8) * 12 + (Math.random() - 0.5) * 0.4) },
  "/photonvision/cam-front/targetPitch": { type: "double", value: () => num("double", 4 + Math.cos(t() * 0.5) * 3) },
  "/photonvision/cam-front/latencyMillis": { type: "double", value: () => num("double", 18 + Math.random() * 6) },
  "/photonvision/cam-front/pipelineIndex": { type: "int", value: () => num("int", 0) },
  "/photonvision/cam-front/rawBytes": { type: "raw", value: () => ({ type: "raw", value: [1, 2, 3, 4] }) },
};

export function ntConnect(target: string, port: number | null, onFrame: (frame: NtFrame) => void): Promise<() => void> {
  const host = /^\d{1,5}$/.test(target.trim()) ? `10.${Math.floor(+target / 100)}.${+target % 100}.2` : target.trim();
  const timers: ReturnType<typeof setTimeout>[] = [];
  onFrame({ type: "status", connected: false, host, port: port ?? 5810, reason: "connecting" });
  timers.push(
    setTimeout(() => {
      onFrame({ type: "status", connected: true, host, port: port ?? 5810, reason: null });
      const announced: NtTopic[] = Object.entries(ROBOT).map(([name, topic]) => ({ name, type: topic.type, properties: {} }));
      onFrame({ type: "topics", announced, gone: [] });
      const start = performance.now();
      timers.push(
        setInterval(() => {
          const now = Math.round((performance.now() - start) * 1000);
          onFrame({ type: "values", values: Object.entries(ROBOT).map(([name, topic]) => ({ name, t_us: now, value: topic.value() })) });
        }, 20),
      );
    }, 400),
  );
  return Promise.resolve(() => timers.forEach((timer) => clearInterval(timer)));
}

// The local server.
let running: { port: number; topics: Map<string, NtServerTopic>; started: number } | null = null;
const watchers: ((frame: NtServerFrame) => void)[] = [];
let camera: ReturnType<typeof setTimeout> | null = null;

function info(): NtServerInfo | null {
  if (!running) return null;
  const connected = performance.now() - running.started > 2500;
  return {
    port: running.port,
    addresses: ["192.168.1.40", "172.31.209.218"],
    topics: [...running.topics.values()].sort((a, b) => a.name.localeCompare(b.name)),
    clients: connected ? [{ id: 1, name: "photonvision", subscriptions: 2, publications: 6 }] : [],
  };
}

const changed = () => watchers.forEach((w) => w({ type: "changed" }));

export const ntServer = {
  start(port: number | null): Promise<NtServerInfo> {
    if (!running) {
      running = { port: port ?? 5810, topics: new Map(), started: performance.now() };
      // A camera finds the server and publishes its results.
      camera = setTimeout(() => {
        if (!running) return;
        for (const name of ["hasTarget", "targetYaw", "latencyMillis"]) {
          const type = name === "hasTarget" ? "boolean" : "double";
          running.topics.set(`/photonvision/cam-front/${name}`, {
            name: `/photonvision/cam-front/${name}`,
            type,
            value: type === "boolean" ? { type: "boolean", value: true } : { type: "double", value: 3.5 },
            t_us: 1,
            owner: "client",
            publisher: "photonvision",
            persistent: false,
          });
        }
        changed();
        watchers.forEach((w) => w({ type: "wrote", client: "photonvision", name: "/photonvision/cam-front/targetYaw", value: { type: "double", value: 3.5 } }));
      }, 2500);
    }
    return Promise.resolve(info()!);
  },
  stop(): Promise<void> {
    running = null;
    if (camera) clearTimeout(camera);
    watchers.length = 0;
    return Promise.resolve();
  },
  info: () => Promise.resolve(info()),
  watch(onFrame: (frame: NtServerFrame) => void): Promise<void> {
    watchers.push(onFrame);
    return Promise.resolve();
  },
  set(name: string, typeName: string | null, value: NtValue | null): Promise<void> {
    if (!running) return Promise.reject("the local NetworkTables server isn't running");
    const topic = running.topics.get(name);
    const type = topic?.type ?? typeName ?? value?.type;
    if (!type) return Promise.reject("a new topic needs a type");
    if (value && topic && value.type !== topic.type) return Promise.reject(`${name} is a ${topic.type} topic`);
    running.topics.set(name, {
      name,
      type,
      value: value ?? topic?.value ?? null,
      t_us: Math.round(performance.now() * 1000),
      owner: topic?.owner ?? "local",
      publisher: topic?.publisher ?? null,
      persistent: topic?.persistent ?? false,
    });
    changed();
    return Promise.resolve();
  },
  persistent(name: string, persistent: boolean): Promise<void> {
    const topic = running?.topics.get(name);
    if (topic) topic.persistent = persistent;
    changed();
    return Promise.resolve();
  },
  delete(name: string): Promise<void> {
    running?.topics.delete(name);
    changed();
    return Promise.resolve();
  },
};

export const ntCameraAddresses = (): Promise<NtCameraAddress[]> =>
  Promise.resolve([
    { name: "photonvision", device_ip: "172.31.209.217", address: "172.31.209.218" },
    { name: "cam-front", device_ip: "10.53.38.11", address: "10.53.38.5" },
  ]);
