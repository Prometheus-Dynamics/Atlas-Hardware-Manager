// Simulated update jobs: plans like atlas-core and ticks through steps on
// timers, emitting the same job-* events the real shell forwards.

import type {
  Concurrency,
  DeviceJobState,
  DeviceJobStatus,
  JobId,
  JobPlan,
  JobRecord,
  JobSummary,
  PlannedDevice,
  UpdateRequestInput,
  UpdateStep,
} from "../types";
import { keyString } from "../types";
import { emit, sleep } from "./bus";
import { AB_ACTIONS, AB_METHODS, fleet, inventory, jobs, releases, simDevice, type SimDevice } from "./data";
import { scan, touch } from "./scan";
import { record } from "./observe";

const STEPS: UpdateStep[] = ["preflight", "transfer", "apply", "reboot", "confirm"];
const STEP_MS: Record<UpdateStep, number> = { preflight: 700, transfer: 3200, apply: 2400, reboot: 1800, confirm: 900 };
const STAGED_THRESHOLD = 3;

function concurrency(device: SimDevice): Concurrency {
  if (device.mode === "recovery") return { kind: "exclusive", resource: "usb-boot" };
  if (device.parent) return { kind: "exclusive", resource: `gateway:${keyString(device.parent)}` };
  return { kind: "parallel" };
}

export function plan(request: UpdateRequestInput): JobPlan {
  const seen = new Set<string>();
  const keys = request.devices.filter((k) => !seen.has(keyString(k)) && seen.add(keyString(k)));
  if (keys.length === 0) throw "no devices selected";
  const devices: PlannedDevice[] = keys.map((key) => {
    const id = keyString(key);
    const stored = inventory.get(id);
    const sim = simDevice(key);
    if (!stored || !sim) throw `no device ${id} in the inventory`;
    if (stored.presence !== "online") throw `${id} is offline; reconnect it or run a scan`;
    const choice = request.releases[key.family];
    if (!choice) throw `no release chosen for the ${key.family} family`;
    if (!choice.version.trim()) throw "the release has no version";
    const entry = choice.path ? undefined : choice.release_id ? releases.find((r) => r.id === choice.release_id) : undefined;
    if (choice.release_id && !choice.path && !entry) throw `no release ${choice.release_id} in the catalog`;
    const fileName = entry?.artifact_name ?? choice.path?.split(/[\\/]/).pop() ?? null;
    const caps = sim.caps ?? [];
    if (sim.mode !== "recovery" && sim.caps && !caps.includes("update"))
      throw `${stored.label ?? sim.name} can't update in place; install it fresh over USB`;
    const ab = (sim.attributes?.update_methods ?? "").includes("ab-tryboot");
    if (ab && fileName && !fileUsesAb(fileName)) throw `${fileName} doesn't use the A/B layout; install it fresh over USB`;
    const from = sim.version;
    return {
      device: key,
      name: stored.label ?? sim.name,
      from_version: from,
      release: {
        family: key.family,
        version: choice.version,
        artifact: entry
          ? { name: entry.artifact_name, path: entry.path ?? "", sha256: entry.sha256, size_bytes: entry.size_bytes }
          : choice.path
            ? { name: fileName ?? "image", path: choice.path, sha256: "", size_bytes: 1_800_000_000 }
            : null,
      },
      plan: {
        steps: STEPS,
        concurrency: concurrency(sim),
        summary:
          sim.mode === "recovery"
            ? `Recovery write of ${choice.version} over USB boot`
            : ab
              ? `Update to ${choice.version} over SSH: the board writes its spare slot, restarts into it, and keeps it once it's healthy`
              : `${from} to ${choice.version}; the device reboots`,
      },
      canary: false,
    };
  });
  const perFamily = new Map<string, number>();
  devices.forEach((d) => perFamily.set(d.device.family, (perFamily.get(d.device.family) ?? 0) + 1));
  const chosen = new Set<string>();
  for (const device of devices) {
    const count = perFamily.get(device.device.family)!;
    const staged =
      request.staged === "on" ? count > 1 : request.staged === "off" ? false : count >= STAGED_THRESHOLD;
    if (staged && !chosen.has(device.device.family)) {
      chosen.add(device.device.family);
      device.canary = true;
    }
  }
  return { devices };
}

/** In the simulation, every image but PhotonVision's uses the A/B layout. */
function fileUsesAb(name: string): boolean {
  return !/photonvision/i.test(name);
}

/**
 * A freshly installed board starts its OS: the running record of the same
 * board comes back on the new version, and the USB boot record is folded
 * into it, as atlas-core's lineage does.
 */
function comesBackRunning(recovery: SimDevice, version: string, fileName: string) {
  const board = recovery.attributes?.board_serial;
  const running = fleet.find((d) => d.mode === "normal" && board && d.attributes?.board_serial === board);
  if (!running) return;
  setTimeout(() => {
    const ab = fileUsesAb(fileName);
    const os = version.replace(/^[a-z]+-raze-/i, "");
    running.version = os;
    running.online = true;
    running.attributes = {
      ...running.attributes,
      os_version: os,
      update_methods: ab ? AB_METHODS : "image-write",
      ...(ab ? { slot_active: "A", update_state: "committed" } : {}),
    };
    running.caps = ab ? ["info", "update", "actions", "telemetry", "logs"] : ["info", "actions", "telemetry", "logs"];
    running.actions = ab ? AB_ACTIONS : AB_ACTIONS.slice(0, 2);
    const index = fleet.indexOf(recovery);
    if (index >= 0) fleet.splice(index, 1);
    void scan().then(() => {
      inventory.delete(keyString(recovery.key));
      emit({ type: "device-forgotten", key: recovery.key });
    });
  }, 2500);
}

let nextJob = 1;
const cancelled = new Set<JobId>();
const locks = new Map<string, Promise<void>>();

/** Serializes work on one exclusive resource. */
async function withResource<T>(concurrency: Concurrency, work: () => Promise<T>): Promise<T> {
  if (concurrency.kind === "parallel") return work();
  const previous = locks.get(concurrency.resource) ?? Promise.resolve();
  let release!: () => void;
  const mine = new Promise<void>((resolve) => (release = resolve));
  locks.set(concurrency.resource, previous.then(() => mine));
  await previous;
  try {
    return await work();
  } finally {
    release();
  }
}

export function start(request: UpdateRequestInput): JobId {
  const jobPlan = plan(request);
  const id = nextJob++;
  const job: JobRecord = {
    id,
    state: "running",
    created_ms: Date.now(),
    finished_ms: null,
    summary: null,
    devices: jobPlan.devices.map((p) => ({
      device: p.device,
      name: p.name,
      release: p.release,
      plan: p.plan,
      canary: p.canary,
      status: { status: "queued" },
      step: null,
      fraction: 0,
      log: [],
      started_ms: null,
      finished_ms: null,
      last_activity_ms: null,
    })),
  };
  jobs.push(job);
  emit({ type: "job-started", job: id, devices: job.devices.map((d) => d.device) });
  void run(job);
  return id;
}

export function cancel(id: JobId) {
  const job = jobs.find((j) => j.id === id);
  if (!job) throw `no job ${id}`;
  if (job.state === "running") cancelled.add(id);
}

function log(job: JobRecord, state: DeviceJobState, message: string) {
  const line = `${new Date().toISOString().slice(11, 19)} ${message}`;
  state.log.push(line);
  state.last_activity_ms = Date.now();
  emit({ type: "job-log", job: job.id, device: state.device, message: line });
}

function setStatus(job: JobRecord, state: DeviceJobState, status: DeviceJobStatus) {
  state.status = status;
  if (status.status !== "running" && status.status !== "queued") state.finished_ms = Date.now();
  emit({ type: "job-device", job: job.id, device: state.device, status });
  const name = state.name;
  if (status.status === "verified") record("update-result", "success", state.device, `${name} updated to ${status.version}`);
  else if (status.status === "rolled-back") record("update-result", "warning", state.device, `${name} rolled back its update: ${status.reason}`);
  else if (status.status === "needs-recovery") record("update-result", "error", state.device, `${name} needs recovery: ${status.reason}`);
  else if (status.status === "failed") record("update-result", "error", state.device, `${name} update failed: ${status.error}`);
}

async function runDevice(job: JobRecord, state: DeviceJobState): Promise<boolean> {
  const sim = simDevice(state.device)!;
  return withResource(state.plan.concurrency, async () => {
    if (cancelled.has(job.id)) {
      setStatus(job, state, { status: "cancelled" });
      return false;
    }
    state.started_ms = Date.now();
    state.last_activity_ms = state.started_ms;
    setStatus(job, state, { status: "running" });
    for (const step of state.plan.steps) {
      state.step = step;
      state.fraction = 0;
      state.last_activity_ms = Date.now();
      emit({ type: "job-step", job: job.id, device: state.device, step });
      log(job, state, `${step}: started`);
      const ticks = 8;
      for (let i = 1; i <= ticks; i++) {
        await sleep(STEP_MS[step] / ticks);
        if (cancelled.has(job.id) && (step === "preflight" || step === "transfer")) {
          log(job, state, `${step}: cancelled; nothing was changed on the device`);
          setStatus(job, state, { status: "cancelled" });
          return false;
        }
        state.fraction = i / ticks;
        state.last_activity_ms = Date.now();
        emit({ type: "job-progress", job: job.id, device: state.device, step, fraction: state.fraction });
      }
      if (step === "confirm" && sim.neverConfirms) {
        const reason = `${state.name} did not report ${state.release.version} after reboot`;
        log(job, state, `confirm: ${reason}; rolled back`);
        setStatus(job, state, { status: "rolled-back", reason });
        return false;
      }
      log(job, state, `${step}: done`);
    }
    sim.version = state.release.version;
    if (sim.link?.kind === "usb-boot") {
      // A flashed board reboots out of USB boot; the next scan sees it leave.
      sim.online = false;
      comesBackRunning(sim, state.release.version, state.release.artifact?.name ?? "");
    } else {
      sim.mode = "normal";
      if (sim.attributes?.os_version) {
        sim.version = state.release.version.replace(/^[a-z]+-raze-/i, "");
        sim.attributes = { ...sim.attributes, os_version: sim.version };
      }
      touch(state.device);
    }
    setStatus(job, state, { status: "verified", version: state.release.version });
    return true;
  });
}

async function run(job: JobRecord) {
  const families = new Map<string, DeviceJobState[]>();
  job.devices.forEach((d) => families.set(d.device.family, [...(families.get(d.device.family) ?? []), d]));
  await Promise.all(
    [...families.values()].map(async (group) => {
      const canary = group.find((d) => d.canary);
      const rest = group.filter((d) => d !== canary);
      if (canary && !(await runDevice(job, canary))) {
        for (const d of rest) {
          setStatus(job, d, { status: "skipped", reason: `staged rollout stopped: ${canary.name} did not verify` });
        }
        return;
      }
      await Promise.all(rest.map((d) => runDevice(job, d)));
    }),
  );
  const summary: JobSummary = { verified: 0, rolled_back: 0, needs_recovery: 0, failed: 0, skipped: 0, cancelled: 0 };
  for (const d of job.devices) {
    const s = d.status.status;
    if (s === "verified") summary.verified++;
    else if (s === "rolled-back") summary.rolled_back++;
    else if (s === "needs-recovery") summary.needs_recovery++;
    else if (s === "failed") summary.failed++;
    else if (s === "skipped") summary.skipped++;
    else if (s === "cancelled") summary.cancelled++;
  }
  job.summary = summary;
  job.state = cancelled.has(job.id) ? "cancelled" : "finished";
  job.finished_ms = Date.now();
  cancelled.delete(job.id);
  emit({ type: "job-finished", job: job.id, state: job.state, summary });
}
