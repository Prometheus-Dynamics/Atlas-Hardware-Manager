#!/usr/bin/env python3
"""Builds the Raze case model the app's IMU view draws (raze-case.glb).

Reads the assembled case (an OBJ with one group per part, exported from the
FreeCAD assembly) and the parts it lacks (the side button, as an OBJ), keeps
the named parts, welds and lightly simplifies each one by vertex clustering
(GRID mm), and writes a binary glTF with one mesh per part, named after it.

The model is in the IMU's frame, in metres, centred on the case: x along
the case's long side, y up the drawing, z out of the front face (the lens
and LED ring), so the app turns it by the IMU's orientation as it is.
That mapping (from the CAD's x, z, -y) is the one assumed for Gen 1 until
it is checked on a board.

    devices/tools/case_model.py <assembly.obj> <button.obj> <out.glb>
"""

import json
import struct
import sys

import numpy as np

KEEP = ["TOP", "BOTTOM", "HEATSINK", "DIFFUSER"]
GRID = 0.25  # mm


def read_obj(path: str) -> tuple[np.ndarray, dict[str, list[list[int]]]]:
    verts: list[list[float]] = []
    groups: dict[str, list[list[int]]] = {}
    cur = "default"
    with open(path) as f:
        for line in f:
            if line.startswith("v "):
                verts.append([float(x) for x in line.split()[1:4]])
            elif line.startswith(("g ", "o ")):
                cur = line.split(maxsplit=1)[1].strip()
                groups.setdefault(cur, [])
            elif line.startswith("f "):
                idx = [int(t.split("/")[0]) for t in line.split()[1:]]
                # Fan-triangulate polygons.
                for i in range(1, len(idx) - 1):
                    groups.setdefault(cur, []).append([idx[0], idx[i], idx[i + 1]])
    return np.array(verts, dtype=np.float64), groups


def cluster(points: np.ndarray, faces: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """Welds points within a GRID cell; drops triangles that collapse."""
    cells = np.floor(points / GRID).astype(np.int64)
    _, first, inverse = np.unique(cells, axis=0, return_index=True, return_inverse=True)
    inverse = inverse.reshape(-1)
    # Each cell's point: the mean of the points in it.
    sums = np.zeros((len(first), 3))
    np.add.at(sums, inverse, points)
    counts = np.bincount(inverse, minlength=len(first))[:, None]
    merged = sums / counts
    tri = inverse[faces]
    keep = (tri[:, 0] != tri[:, 1]) & (tri[:, 1] != tri[:, 2]) & (tri[:, 0] != tri[:, 2])
    tri = tri[keep]
    # Drop duplicate triangles, keeping each one's winding.
    _, first_seen = np.unique(np.sort(tri, axis=1), axis=0, return_index=True)
    tri = tri[np.sort(first_seen)]
    used = np.unique(tri)
    remap = np.full(len(merged), -1)
    remap[used] = np.arange(len(used))
    return merged[used], remap[tri]


def normals(points: np.ndarray, tri: np.ndarray) -> np.ndarray:
    n = np.zeros_like(points)
    face = np.cross(points[tri[:, 1]] - points[tri[:, 0]], points[tri[:, 2]] - points[tri[:, 0]])
    for k in range(3):
        np.add.at(n, tri[:, k], face)
    length = np.linalg.norm(n, axis=1, keepdims=True)
    length[length == 0] = 1
    return n / length


def to_imu(points: np.ndarray, centre: np.ndarray) -> np.ndarray:
    """CAD (mm) to the IMU's frame (m): x = x, y = z, z = -y, centred."""
    p = points - centre
    return np.stack([p[:, 0], p[:, 2], -p[:, 1]], axis=1) / 1000.0


def main() -> None:
    assembly, button, out = sys.argv[1:4]
    verts, groups = read_obj(assembly)
    bverts, bgroups = read_obj(button)
    parts: list[tuple[str, np.ndarray, np.ndarray]] = []
    for name in KEEP:
        faces = np.array(groups[name]) - 1
        used = np.unique(faces)
        remap = np.full(len(verts), -1)
        remap[used] = np.arange(len(used))
        parts.append((name, verts[used], remap[faces]))
    bfaces = np.concatenate([np.array(f) for f in bgroups.values() if f]) - 1
    parts.append(("BUTTON", bverts, bfaces))

    every = np.concatenate([p for _, p, _ in parts])
    centre = (every.min(axis=0) + every.max(axis=0)) / 2

    buffer = bytearray()
    views, accessors, meshes, nodes = [], [], [], []

    def add(data: bytes, target: int) -> int:
        while len(buffer) % 4:
            buffer.append(0)
        views.append({"buffer": 0, "byteOffset": len(buffer), "byteLength": len(data), "target": target})
        buffer.extend(data)
        return len(views) - 1

    for name, points, faces in parts:
        p, tri = cluster(points, faces)
        p = to_imu(p, centre).astype(np.float32)
        n = normals(p.astype(np.float64), tri).astype(np.float32)
        index_type, index_bytes = (5123, tri.astype(np.uint16).tobytes()) if len(p) < 65536 else (5125, tri.astype(np.uint32).tobytes())
        pos = add(p.tobytes(), 34962)
        nor = add(n.tobytes(), 34962)
        ind = add(index_bytes, 34963)
        accessors += [
            {"bufferView": pos, "componentType": 5126, "count": len(p), "type": "VEC3",
             "min": p.min(axis=0).tolist(), "max": p.max(axis=0).tolist()},
            {"bufferView": nor, "componentType": 5126, "count": len(n), "type": "VEC3"},
            {"bufferView": ind, "componentType": index_type, "count": tri.size, "type": "SCALAR"},
        ]
        a = len(accessors) - 3
        meshes.append({"name": name, "primitives": [{"attributes": {"POSITION": a, "NORMAL": a + 1}, "indices": a + 2}]})
        nodes.append({"name": name, "mesh": len(meshes) - 1})
        print(f"{name:9s} {len(faces):6d} -> {len(tri):6d} triangles, {len(p)} vertices", file=sys.stderr)

    gltf = {
        "asset": {"version": "2.0", "generator": "atlas devices/tools/case_model.py"},
        "scene": 0,
        "scenes": [{"nodes": list(range(len(nodes)))}],
        "nodes": nodes,
        "meshes": meshes,
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [{"byteLength": len(buffer)}],
    }
    text = json.dumps(gltf, separators=(",", ":")).encode()
    text += b" " * (-len(text) % 4)
    while len(buffer) % 4:
        buffer.append(0)
    total = 12 + 8 + len(text) + 8 + len(buffer)
    with open(out, "wb") as f:
        f.write(struct.pack("<III", 0x46546C67, 2, total))
        f.write(struct.pack("<II", len(text), 0x4E4F534A) + text)
        f.write(struct.pack("<II", len(buffer), 0x004E4942) + bytes(buffer))
    print(f"wrote {out}: {total / 1024:.0f} KiB", file=sys.stderr)


if __name__ == "__main__":
    main()
