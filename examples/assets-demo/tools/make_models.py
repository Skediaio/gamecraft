#!/usr/bin/env python3
"""Makes the two models of the assets demo pack: a wooden crate and a fence segment.

Each is a self-contained GLB (glTF 2.0, metres, Y up, the origin at the base) with one
material and one embedded PNG texture drawn here from plain arithmetic, and a licence record
beside it (CC0-1.0, the GameCraft project as author). Standard library only; the output is the
same bytes on every run.

    python3 examples/assets-demo/tools/make_models.py [pack folder]

The pack folder defaults to the folder above this script.
"""

import json
import os
import struct
import sys
import zlib

# ---------------------------------------------------------------------------- textures


def png(width, height, pixel):
    """An RGB PNG of `pixel(x, y) -> (r, g, b)` (bytes 0 to 255)."""
    rows = bytearray()
    for y in range(height):
        rows.append(0)  # no filter
        for x in range(width):
            rows.extend(pixel(x, y))

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(bytes(rows), 9))
        + chunk(b"IEND", b"")
    )


def grain(x, y, seed):
    """A small deterministic value noise in 0..1 (integer hashing, no randomness)."""
    h = (x * 374761393 + y * 668265263 + seed * 2147483647) & 0xFFFFFFFF
    h = ((h ^ (h >> 13)) * 1274126177) & 0xFFFFFFFF
    return ((h ^ (h >> 16)) & 0xFFFF) / 65535.0


def wood(x, y, plank, seed, base):
    """Wood: long streaks along x, a dark seam between planks of `plank` pixels."""
    streak = grain(x // 7, y, seed) * 0.25 + grain(x // 23, y // 2, seed + 1) * 0.15
    seam = 0.45 if y % plank in (0, plank - 1) else 1.0
    shade = (0.8 + streak) * seam
    return tuple(max(0, min(255, int(c * shade))) for c in base)


def crate_texture(x, y):
    """Planks with a darker frame around the face (64 x 64; each face uses the whole image)."""
    frame = x < 6 or x > 57 or y < 6 or y > 57
    if frame:
        return wood(y, x, 64, 7, (120, 78, 40))
    return wood(x, y, 13, 3, (176, 124, 70))


def fence_texture(x, y):
    """Weathered grey-brown boards."""
    return wood(x, y, 16, 11, (130, 116, 96))


# ---------------------------------------------------------------------------- geometry

# The six faces of a box: normal, then the two axes the face's u and v run along.
FACES = [
    ((1, 0, 0), (0, 0, -1), (0, 1, 0)),
    ((-1, 0, 0), (0, 0, 1), (0, 1, 0)),
    ((0, 1, 0), (1, 0, 0), (0, 0, -1)),
    ((0, -1, 0), (1, 0, 0), (0, 0, 1)),
    ((0, 0, 1), (1, 0, 0), (0, 1, 0)),
    ((0, 0, -1), (-1, 0, 0), (0, 1, 0)),
]


def box(lo, hi, uv_scale, mesh):
    """Adds a box from corner `lo` to `hi`: 4 vertices and 2 triangles per face, outward."""
    centre = [(a + b) / 2 for a, b in zip(lo, hi)]
    half = [(b - a) / 2 for a, b in zip(lo, hi)]
    for normal, u, v in FACES:
        base = len(mesh["positions"])
        for su, sv in ((-1, -1), (1, -1), (1, 1), (-1, 1)):
            p = [
                centre[i] + half[i] * (normal[i] + su * u[i] + sv * v[i])
                for i in range(3)
            ]
            mesh["positions"].append(p)
            mesh["normals"].append(list(normal))
            # The extent of the face along u and v, so a long rail repeats its boards.
            ext_u = sum(abs(u[i]) * half[i] * 2 for i in range(3))
            ext_v = sum(abs(v[i]) * half[i] * 2 for i in range(3))
            mesh["uvs"].append(
                [
                    (su + 1) / 2 * (ext_u * uv_scale if uv_scale else 1.0),
                    (1 - (sv + 1) / 2) * (ext_v * uv_scale if uv_scale else 1.0),
                ]
            )
        mesh["indices"] += [base, base + 1, base + 2, base, base + 2, base + 3]


def crate_mesh():
    mesh = {"positions": [], "normals": [], "uvs": [], "indices": []}
    box([-0.45, 0.0, -0.45], [0.45, 0.9, 0.45], 0, mesh)
    return mesh


def fence_mesh():
    mesh = {"positions": [], "normals": [], "uvs": [], "indices": []}
    for x in (-0.95, 0.95):
        box([x - 0.05, 0.0, -0.05], [x + 0.05, 1.0, 0.05], 1.0, mesh)
    for y in (0.4, 0.8):
        box([-1.0, y - 0.04, -0.02], [1.0, y + 0.04, 0.02], 1.0, mesh)
    return mesh


# ---------------------------------------------------------------------------- GLB


def pad4(data, fill):
    return data + fill * ((4 - len(data) % 4) % 4)


def glb(name, mesh, texture, roughness, repeat):
    """A GLB of one mesh with one textured material."""
    floats = lambda rows: b"".join(struct.pack("<%df" % len(r), *r) for r in rows)
    parts = [
        floats(mesh["positions"]),
        floats(mesh["normals"]),
        floats(mesh["uvs"]),
        struct.pack("<%dH" % len(mesh["indices"]), *mesh["indices"]),
        texture,
    ]
    blob = b""
    views = []
    for i, part in enumerate(parts):
        view = {"buffer": 0, "byteOffset": len(blob), "byteLength": len(part)}
        if i < 3:
            view["target"] = 34962
        elif i == 3:
            view["target"] = 34963
        views.append(view)
        blob = pad4(blob + part, b"\0")
    n = len(mesh["positions"])
    lo = [min(p[i] for p in mesh["positions"]) for i in range(3)]
    hi = [max(p[i] for p in mesh["positions"]) for i in range(3)]
    wrap = 10497 if repeat else 33071
    doc = {
        "asset": {"version": "2.0", "generator": "GameCraft examples/assets-demo/tools/make_models.py"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"name": name, "mesh": 0}],
        "meshes": [
            {
                "name": name,
                "primitives": [
                    {
                        "attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2},
                        "indices": 3,
                        "material": 0,
                    }
                ],
            }
        ],
        "materials": [
            {
                "name": name,
                "pbrMetallicRoughness": {
                    "baseColorTexture": {"index": 0},
                    "metallicFactor": 0.0,
                    "roughnessFactor": roughness,
                },
            }
        ],
        "textures": [{"source": 0, "sampler": 0}],
        "samplers": [{"magFilter": 9729, "minFilter": 9987, "wrapS": wrap, "wrapT": wrap}],
        "images": [{"bufferView": 4, "mimeType": "image/png"}],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": n, "type": "VEC3", "min": lo, "max": hi},
            {"bufferView": 1, "componentType": 5126, "count": n, "type": "VEC3"},
            {"bufferView": 2, "componentType": 5126, "count": n, "type": "VEC2"},
            {"bufferView": 3, "componentType": 5123, "count": len(mesh["indices"]), "type": "SCALAR"},
        ],
        "bufferViews": views,
        "buffers": [{"byteLength": len(blob)}],
    }
    text = pad4(json.dumps(doc, separators=(",", ":"), sort_keys=True).encode(), b" ")
    total = 12 + 8 + len(text) + 8 + len(blob)
    return (
        b"glTF"
        + struct.pack("<II", 2, total)
        + struct.pack("<I", len(text))
        + b"JSON"
        + text
        + struct.pack("<I", len(blob))
        + b"BIN\0"
        + blob
    )


def record(title):
    return (
        json.dumps(
            {
                "license": "CC0-1.0",
                "author": "The GameCraft project",
                "source_url": "https://github.com/Skediaio/gamecraft/tree/main/examples/assets-demo/tools/make_models.py",
                "title": title,
            },
            indent=2,
        )
        + "\n"
    )


def main():
    pack = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "..")
    models = os.path.join(pack, "assets", "models")
    os.makedirs(models, exist_ok=True)
    made = {
        "crate.glb": (glb("crate", crate_mesh(), png(64, 64, crate_texture), 0.8, False), "Wooden crate"),
        "fence.glb": (glb("fence", fence_mesh(), png(32, 32, fence_texture), 0.9, True), "Fence segment"),
    }
    for name, (data, title) in made.items():
        with open(os.path.join(models, name), "wb") as f:
            f.write(data)
        with open(os.path.join(models, name + ".license.json"), "w") as f:
            f.write(record(title))
        print(f"{name}: {len(data)} bytes")


if __name__ == "__main__":
    main()
