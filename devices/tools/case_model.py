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

    devices/tools/case_model.py <assembly.obj> <button.obj> <board.wrl> <board.kicad_pcb> <out.glb>

The board comes from the assembly's VRML export (FreeCAD, gzip or not): not
the whole PCB, a simplified slab of it (PCB) and the parts at its edges, the
connectors and the side switch, so the case's openings show what plugs in
there. The VRML's colours are the 3D models' and some are wrong (a beige
USB-C), so each edge part is matched to its footprint on the KiCad board
(the nearest, placed by the CM5 connectors' fit below) and coloured by what
it is (PART_LOOKS); a part with no footprint near it keeps the VRML colour.
"""

import gzip
import json
import re
import struct
import sys

import numpy as np

KEEP = ["TOP", "BOTTOM", "HEATSINK", "DIFFUSER"]
GRID = 0.25  # mm


class Vrml:
    """The face sets of a VRML 2 file, placed (nested Transforms applied), with colours."""

    def __init__(self, path: str):
        raw = open(path, "rb").read()
        if raw[:2] == b"\x1f\x8b":
            raw = gzip.decompress(raw)
        self.tokens = re.findall(r"[{}\[\]]|[^\s{}\[\],]+", raw.decode("utf-8", "replace"))
        self.pos = 0
        self.coords: dict[str, np.ndarray] = {}
        self.shapes: list[tuple[np.ndarray, np.ndarray, tuple[float, float, float]]] = []
        tokens = self.tokens
        while self.pos < len(tokens):
            if tokens[self.pos] in ("Group", "Transform") and tokens[self.pos + 1] == "{":
                kind = tokens[self.pos]
                self.pos += 1
                self.node(kind, np.eye(3), np.zeros(3), (0.7, 0.7, 0.7))
            else:
                self.pos += 1

    @staticmethod
    def axis_angle(ax: float, ay: float, az: float, angle: float) -> np.ndarray:
        a = np.array([ax, ay, az], dtype=float)
        n = np.linalg.norm(a)
        if n == 0 or angle == 0:
            return np.eye(3)
        x, y, z = a / n
        c, s = np.cos(angle), np.sin(angle)
        C = 1 - c
        return np.array([[c + x * x * C, x * y * C - z * s, x * z * C + y * s],
                         [y * x * C + z * s, c + y * y * C, y * z * C - x * s],
                         [z * x * C - y * s, z * y * C + x * s, c + z * z * C]])

    def numbers(self, bare: int) -> list[float]:
        t = self.tokens
        out = []
        if t[self.pos] == "[":
            self.pos += 1
            while t[self.pos] != "]":
                out.append(float(t[self.pos]))
                self.pos += 1
            self.pos += 1
        else:
            for _ in range(bare):
                out.append(float(t[self.pos]))
                self.pos += 1
        return out

    def node(self, kind, matrix, offset, color, name=None):
        t = self.tokens
        self.pos += 1  # {
        tr = np.zeros(3); rot = np.eye(3); sc = np.ones(3)
        local = color
        points = None; index = None; pending = None
        while t[self.pos] != "}":
            tok = t[self.pos]
            if tok == "translation":
                tr = np.array([float(t[self.pos + k]) for k in range(1, 4)]); self.pos += 4
            elif tok == "rotation":
                rot = self.axis_angle(*[float(t[self.pos + k]) for k in range(1, 5)]); self.pos += 5
            elif tok == "scale":
                sc = np.array([float(t[self.pos + k]) for k in range(1, 4)]); self.pos += 4
            elif tok == "scaleOrientation":
                self.pos += 5
            elif tok == "center":
                self.pos += 4
            elif tok == "diffuseColor":
                local = tuple(float(t[self.pos + k]) for k in range(1, 4)); self.pos += 4
            elif tok == "point":
                self.pos += 1
                points = np.array(self.numbers(3)).reshape(-1, 3)
            elif tok == "coordIndex":
                self.pos += 1
                index = [int(v) for v in self.numbers(1)]
            elif tok in ("vector", "normalIndex", "colorIndex", "color", "texCoordIndex") and t[self.pos + 1] == "[":
                self.pos += 1
                self.numbers(0)
            elif tok == "DEF":
                pending = t[self.pos + 1]; self.pos += 2
            elif tok == "USE":
                used = t[self.pos + 1]; self.pos += 2
                if used in self.coords:
                    points = self.coords[used]
            elif t[self.pos + 1] == "{" and tok[0].isupper():
                self.pos += 1
                m, o = (matrix @ (rot * sc), offset + matrix @ tr) if kind == "Transform" else (matrix, offset)
                child = self.node(tok, m, o, local, pending)
                if tok == "Coordinate" and child is not None:
                    points = child
                elif tok in ("Appearance", "Material") and child is not None:
                    local = child
                pending = None
                continue
            else:
                self.pos += 1
        self.pos += 1
        if kind in ("Appearance", "Material"):
            return local if local != color else None
        if kind == "Coordinate":
            if name is not None and points is not None:
                self.coords[name] = points
            return points
        if kind == "IndexedFaceSet" and points is not None and index:
            faces, cur = [], []
            for v in index:
                if v < 0:
                    faces += [[cur[0], cur[k], cur[k + 1]] for k in range(1, len(cur) - 1)]
                    cur = []
                else:
                    cur.append(v)
            if faces:
                self.shapes.append(((matrix @ points.T).T + offset, np.array(faces, dtype=np.int64), color))
        return None


def board_parts(path: str, pcb_path: str) -> list[tuple[str, np.ndarray, np.ndarray, tuple[float, float, float]]]:
    """The PCB (the largest flat face set) and the parts reaching its edges."""
    shapes = Vrml(path).shapes
    known = footprints(pcb_path)
    used: dict[str, int] = {}
    size = lambda p: p.max(axis=0) - p.min(axis=0)
    flat = [s for s in shapes if size(s[0])[1] < 2.5 and size(s[0])[0] > 40]
    pcb = max(flat, key=lambda s: size(s[0])[0] * size(s[0])[2])
    lo, hi = pcb[0].min(axis=0), pcb[0].max(axis=0)
    parts = [("PCB", pcb[0], pcb[1], pcb[2])]
    n = 0
    for points, faces, color in shapes:
        if points is pcb[0]:
            continue
        plo, phi = points.min(axis=0), points.max(axis=0)
        # On the board (not the case), at least 3.5 mm, and at its outline.
        if size(points)[0] > 50 or size(points).max() < 3.5:
            continue
        if plo[1] < lo[1] - 12 or phi[1] > hi[1] + 6:
            continue
        if plo[0] <= lo[0] + 1 or phi[0] >= hi[0] - 1 or plo[2] <= lo[2] + 1 or phi[2] >= hi[2] - 1:
            n += 1
            name, look = f"IO_{n}", color
            centre = (plo + phi) / 2
            near = min(known, key=lambda f: (f[1] - centre[0]) ** 2 + (f[2] - centre[2]) ** 2, default=None)
            # A connector-sized part only: thin strips and pins keep their colour.
            sized = size(points).max() >= 5 and np.sort(size(points))[1] >= 2
            if sized and near and ((near[1] - centre[0]) ** 2 + (near[2] - centre[2]) ** 2) ** 0.5 < 9:
                for pattern, kind, rgb, metal in PART_LOOKS:
                    if re.search(pattern, near[0]):
                        used[kind] = used.get(kind, 0) + 1
                        name = kind if used[kind] == 1 else f"{kind}_{used[kind]}"
                        look = (*rgb, 1.0 if metal else 0.0)
                        break
            print(f"  {name:8s} at x={centre[0]:.1f} z={centre[2]:.1f} <- {near[0][:40] if near else '-'}", file=sys.stderr)
            parts.append((name, points, faces, look))
    return parts


# What a connector looks like, by its footprint's value or name: name in the
# model, colour (sRGB 0-1), metal.
PART_LOOKS = [
    (r"U-A-|USB_A", "USB_A", (0.80, 0.82, 0.85), True),  # HRO U-A-39DS: USB 3 A, steel shell
    (r"TYPE-C|UC23|USB_C|USB-C", "USB_C", (0.80, 0.82, 0.85), True),  # steel shell
    (r"MJ88|RJ45", "RJ45", (0.78, 0.80, 0.82), True),  # shielded RJ45, steel
    (r"JL212", "POWER", (0.09, 0.09, 0.10), False),  # JILN JL212R-50002B01: 2-pin 5 mm terminal block, black (B01)
    (r"BM04B|JST", "JST", (0.92, 0.89, 0.80), False),  # JST SH, natural nylon
    (r"TS24|^SW", "SWITCH", (0.70, 0.72, 0.75), True),  # tactile switch, metal frame
]

# KiCad (mm, y down) to the case CAD (mm): fitted from the CM5 connectors
# CN1/CN2 and the side switch SW1 against the assembly.
def kicad_to_cad(kx: float, ky: float) -> tuple[float, float]:
    return ky - 82.56, 129.1 - kx


def footprints(path: str) -> list[tuple[str, float, float]]:
    """(value or footprint name, CAD x, CAD z) of the board's connectors and switches."""
    text = open(path, encoding="utf-8").read()
    out = []
    for block in text.split("\n\t(footprint ")[1:]:
        name = block.split("\n", 1)[0].strip().strip('"')
        at = re.search(r"\(at ([-\d.]+) ([-\d.]+)", block)
        ref = re.search(r'\(property "Reference" "([^"]+)"', block)
        value = re.search(r'\(property "Value" "([^"]+)"', block)
        if not at or not ref or not re.match(r"(USB|RJ|J|U|SW)\d", ref.group(1)):
            continue
        label = f"{value.group(1) if value else ''} {name}"
        if any(re.search(pattern, label) for pattern, *_ in PART_LOOKS):
            x, z = kicad_to_cad(float(at.group(1)), float(at.group(2)))
            out.append((label, x, z))
    return out


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
    assembly, button, board, pcb, out = sys.argv[1:6]
    verts, groups = read_obj(assembly)
    bverts, bgroups = read_obj(button)
    parts: list[tuple[str, np.ndarray, np.ndarray, tuple[float, float, float] | None]] = []
    for name in KEEP:
        faces = np.array(groups[name]) - 1
        used = np.unique(faces)
        remap = np.full(len(verts), -1)
        remap[used] = np.arange(len(used))
        parts.append((name, verts[used], remap[faces], None))
    bfaces = np.concatenate([np.array(f) for f in bgroups.values() if f]) - 1
    parts.append(("BUTTON", bverts, bfaces, None))
    # The case alone sets the centre, so the board sits where it is in it.
    every = np.concatenate([p for _, p, _, _ in parts])
    parts += board_parts(board, pcb)
    centre = (every.min(axis=0) + every.max(axis=0)) / 2

    buffer = bytearray()
    views, accessors, meshes, nodes, materials = [], [], [], [], []

    def add(data: bytes, target: int) -> int:
        while len(buffer) % 4:
            buffer.append(0)
        views.append({"buffer": 0, "byteOffset": len(buffer), "byteLength": len(data), "target": target})
        buffer.extend(data)
        return len(views) - 1

    for name, points, faces, color in parts:
        p, tri = cluster(points, faces)
        if len(tri) == 0:
            continue
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
        primitive = {"attributes": {"POSITION": a, "NORMAL": a + 1}, "indices": a + 2}
        if color is not None:
            # A fourth component is the metal flag PART_LOOKS gives; the VRML's
            # own colours are plastic.
            metal = color[3] if len(color) > 3 else 0.0
            materials.append({"name": name, "pbrMetallicRoughness": {
                "baseColorFactor": [*color[:3], 1.0], "metallicFactor": 0.85 if metal else 0.05,
                "roughnessFactor": 0.3 if metal else 0.6}})
            primitive["material"] = len(materials) - 1
        meshes.append({"name": name, "primitives": [primitive]})
        nodes.append({"name": name, "mesh": len(meshes) - 1})
        print(f"{name:9s} {len(faces):6d} -> {len(tri):6d} triangles, {len(p)} vertices", file=sys.stderr)

    gltf = {
        "asset": {"version": "2.0", "generator": "atlas devices/tools/case_model.py"},
        "scene": 0,
        "scenes": [{"nodes": list(range(len(nodes)))}],
        "nodes": nodes,
        "meshes": meshes,
        **({"materials": materials} if materials else {}),
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
