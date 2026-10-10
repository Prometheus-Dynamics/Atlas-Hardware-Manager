// The IMU's orientation from its live samples: a Mahony filter (the gyro
// integrated, pulled toward the accelerometer's gravity so tilt doesn't
// drift). Heading (yaw) has nothing to pull it back, so it drifts slowly;
// `zeroYaw` makes the current heading straight ahead.

/** A unit quaternion, w first: the board's frame to the world's (z up). */
export type Quat = [number, number, number, number];

/** How hard the accelerometer pulls (1/s): higher follows it faster but shakes more. */
const KP = 2;
/** A gap longer than this (s) between samples isn't integrated across. */
const MAX_DT = 0.1;
/** Accelerometer magnitudes outside this band (m/s²) are a shake, not gravity. */
const G_BAND: [number, number] = [6, 13.6];

export function normalize(q: Quat): Quat {
  const n = Math.hypot(...q) || 1;
  return [q[0] / n, q[1] / n, q[2] / n, q[3] / n];
}

/** The rotation that takes the board's `up` (measured gravity) to world z: tilt only. */
export function fromGravity(ax: number, ay: number, az: number): Quat {
  const n = Math.hypot(ax, ay, az) || 1;
  const [x, y, z] = [ax / n, ay / n, az / n];
  // Rotate (x, y, z) onto (0, 0, 1): axis (y, -x, 0), angle acos(z).
  const half = Math.acos(Math.max(-1, Math.min(1, z))) / 2;
  const s = Math.hypot(x, y);
  if (s < 1e-9) return z >= 0 ? [1, 0, 0, 0] : [0, 1, 0, 0];
  const k = Math.sin(half) / s;
  return normalize([Math.cos(half), y * k, -x * k, 0]);
}

/** One Mahony step: `gyro` rad/s, `accel` m/s² (the support force, up), over `dt` s. */
export function step(q: Quat, gyro: [number, number, number], accel: [number, number, number], dt: number): Quat {
  let [gx, gy, gz] = gyro;
  const [w, x, y, z] = q;
  const an = Math.hypot(...accel);
  if (an > G_BAND[0] && an < G_BAND[1]) {
    const [ax, ay, az] = [accel[0] / an, accel[1] / an, accel[2] / an];
    // World up in the board's frame, from q.
    const vx = 2 * (x * z - w * y);
    const vy = 2 * (w * x + y * z);
    const vz = w * w - x * x - y * y + z * z;
    // Turn the estimate toward the measurement: a × v.
    gx += KP * (ay * vz - az * vy);
    gy += KP * (az * vx - ax * vz);
    gz += KP * (ax * vy - ay * vx);
  }
  const h = 0.5 * dt;
  return normalize([
    w + h * (-x * gx - y * gy - z * gz),
    x + h * (w * gx + y * gz - z * gy),
    y + h * (w * gy - x * gz + z * gx),
    z + h * (w * gz + x * gy - y * gx),
  ]);
}

/** Roll, pitch, yaw in degrees (ZYX), for the readout. */
export function euler(q: Quat): { roll: number; pitch: number; yaw: number } {
  const [w, x, y, z] = q;
  const deg = 180 / Math.PI;
  return {
    roll: Math.atan2(2 * (w * x + y * z), 1 - 2 * (x * x + y * y)) * deg,
    pitch: Math.asin(Math.max(-1, Math.min(1, 2 * (w * y - z * x)))) * deg,
    yaw: Math.atan2(2 * (w * z + x * y), 1 - 2 * (y * y + z * z)) * deg,
  };
}

/** `q` turned about world z so its heading is zero. */
export function zeroYaw(q: Quat): Quat {
  const half = (-euler(q).yaw * Math.PI) / 360;
  const r: Quat = [Math.cos(half), 0, 0, Math.sin(half)];
  // r ⊗ q
  return normalize([
    r[0] * q[0] - r[3] * q[3],
    r[0] * q[1] - r[3] * q[2],
    r[0] * q[2] + r[3] * q[1],
    r[0] * q[3] + r[3] * q[0],
  ]);
}

/**
 * The CSS matrix3d for `q` in a scene whose x is right, y is toward the
 * viewer (down the page) and z is up out of the floor: the world's y flipped.
 */
export function cssMatrix(q: Quat): string {
  const [w, x, y, z] = q;
  const r = [
    [1 - 2 * (y * y + z * z), 2 * (x * y - w * z), 2 * (x * z + w * y)],
    [2 * (x * y + w * z), 1 - 2 * (x * x + z * z), 2 * (y * z - w * x)],
    [2 * (x * z - w * y), 2 * (y * z + w * x), 1 - 2 * (x * x + y * y)],
  ];
  // S R S with S = diag(1, -1, 1).
  const s = [1, -1, 1];
  const m = r.map((row, i) => row.map((v, j) => v * s[i] * s[j]));
  // matrix3d is column-major.
  return `matrix3d(${m[0][0]},${m[1][0]},${m[2][0]},0,${m[0][1]},${m[1][1]},${m[2][1]},0,${m[0][2]},${m[1][2]},${m[2][2]},0,0,0,0,1)`;
}

/** Feeds a LiveSeries' new samples to the filter; `channels` are the indices of ax, ay, az, gx, gy, gz. */
export class Orientation {
  q: Quat = [1, 0, 0, 0];
  private lastUs = -Infinity;
  private started = false;

  constructor(private readonly channels: number[]) {}

  /** Folds in every sample newer than the last seen. */
  update(series: { count: number; t: Float64Array; v: Float64Array[]; index(i: number): number; firstFrom(t: number): number }) {
    const [ax, ay, az, gx, gy, gz] = this.channels;
    for (let i = series.firstFrom(this.lastUs + 1); i < series.count; i++) {
      const at = series.index(i);
      const t = series.t[at];
      const a: [number, number, number] = [series.v[ax][at], series.v[ay][at], series.v[az][at]];
      const g: [number, number, number] = [series.v[gx][at], series.v[gy][at], series.v[gz][at]];
      if (a.some(Number.isNaN)) continue;
      if (!this.started) {
        this.q = fromGravity(...a);
        this.started = true;
      } else {
        const dt = (t - this.lastUs) / 1_000_000;
        if (dt > 0 && dt <= MAX_DT) this.q = step(this.q, g.some(Number.isNaN) ? [0, 0, 0] : g, a, dt);
      }
      this.lastUs = t;
    }
  }

  zero() {
    this.q = zeroYaw(this.q);
  }
}
