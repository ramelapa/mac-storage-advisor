#!/usr/bin/env python3
"""Draw the Mac Storage Advisor icon into apps/cli/icons.

The mark is a disk ring on a pine rounded square: cream for the ring, amber for
the used portion. Run this again after changing the drawing. The Mac bundle
build reads the files it writes; it does not run this script.
"""

from __future__ import annotations

import math
import struct
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "apps" / "cli" / "icons"

PINE = (28, 63, 58, 255)
CREAM = (243, 237, 226, 255)
AMBER = (227, 154, 45, 255)

# Coordinate space is -0.5..0.5. The ring starts at 12 o'clock and runs clockwise.
SQUIRCLE_HALF = 0.40
SQUIRCLE_RADIUS = 0.12
RING_OUTER = 0.25
RING_INNER = 0.145
USED_SWEEP = 0.68 * math.tau
ARC_START = -math.pi / 2


def clamp01(value: float) -> float:
    return 0.0 if value < 0.0 else 1.0 if value > 1.0 else value


def mix(src: tuple[int, int, int, int], dst: tuple[int, int, int, int], cover: float) -> tuple[int, int, int, int]:
    if cover <= 0.0:
        return src
    if cover >= 1.0:
        return dst
    return tuple(int(round(a + (b - a) * cover)) for a, b in zip(src, dst))  # type: ignore[return-value]


def squircle_sdf(px: float, py: float) -> float:
    half = SQUIRCLE_HALF - SQUIRCLE_RADIUS
    qx = abs(px) - half
    qy = abs(py) - half
    return math.hypot(max(qx, 0.0), max(qy, 0.0)) + min(max(qx, qy), 0.0) - SQUIRCLE_RADIUS


def ring_sdf(dist: float) -> float:
    return max(dist - RING_OUTER, RING_INNER - dist)


def clockwise_delta(angle: float, start: float) -> float:
    return (angle - start) % math.tau


def pixel(x: int, y: int, size: int) -> tuple[int, int, int, int]:
    px = (x + 0.5) / size - 0.5
    py = (y + 0.5) / size - 0.5
    width = 1.0 / size
    plate = clamp01(0.5 - squircle_sdf(px, py) / width)
    if plate <= 0.0:
        return (0, 0, 0, 0)

    color = mix((0, 0, 0, 0), PINE, plate)
    dist = math.hypot(px, py)
    on_ring = clamp01(0.5 - ring_sdf(dist) / width)
    if on_ring <= 0.0:
        return color

    color = mix(color, CREAM, on_ring)
    # y grows downward, so atan2(py, px) increases clockwise on screen.
    delta = clockwise_delta(math.atan2(py, px), ARC_START)
    if dist > 0.0:
        edge = min(delta, USED_SWEEP - delta) if delta <= USED_SWEEP else min(delta - USED_SWEEP, math.tau - delta)
        signed = -edge if delta <= USED_SWEEP else edge
        amber = clamp01(0.5 - (signed * dist) / width)
        color = mix(color, AMBER, amber * on_ring)
    return color


def png_bytes(size: int) -> bytes:
    raw = bytearray()
    for y in range(size):
        raw.append(0)
        for x in range(size):
            raw.extend(pixel(x, y, size))

    def chunk(tag: bytes, data: bytes) -> bytes:
        crc = zlib.crc32(tag + data) & 0xFFFFFFFF
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", crc)

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    compressed = zlib.compress(bytes(raw), 9)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", compressed) + chunk(b"IEND", b"")


def icns_bytes(images: list[tuple[bytes, bytes]]) -> bytes:
    body = bytearray()
    for ostype, payload in images:
        if len(ostype) != 4:
            raise SystemExit(f"icns type must be 4 bytes, got {ostype!r}")
        body += ostype
        body += struct.pack(">I", 8 + len(payload))
        body += payload
    return b"icns" + struct.pack(">I", 8 + len(body)) + body


def ascii_preview(size: int = 32) -> str:
    rows = []
    for y in range(size):
        cells = []
        for x in range(size):
            red, _green, blue, alpha = pixel(x, y, size)
            if alpha < 16:
                cells.append(" ")
            elif red > blue + 40:
                cells.append("#")
            elif red > 180:
                cells.append("+")
            else:
                cells.append(".")
        rows.append("".join(cells))
    return "\n".join(rows)


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    files = {
        32: OUT / "32x32.png",
        128: OUT / "128x128.png",
        256: OUT / "128x128@2x.png",
        1024: OUT / "icon.png",
    }
    encoded = {size: png_bytes(size) for size in (16, 32, 64, 128, 256, 512, 1024)}
    for size, path in files.items():
        path.write_bytes(encoded[size])
    icns = icns_bytes(
        [
            (b"ic11", encoded[32]),
            (b"ic12", encoded[64]),
            (b"ic07", encoded[128]),
            (b"ic08", encoded[256]),
            (b"ic13", encoded[256]),
            (b"ic09", encoded[512]),
            (b"ic14", encoded[512]),
            (b"ic10", encoded[1024]),
        ]
    )
    (OUT / "icon.icns").write_bytes(icns)
    print(ascii_preview())
    print(f"wrote {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
