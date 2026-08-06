#!/usr/bin/env python3
"""Generate Magi's menu-bar icons.

The mark is three nodes in a triangle — Melchior, Balthasar and Casper, the
three Magi. It is not decoration: the three nodes are also the state language.
Idle is three outlines, and later milestones fill them in as the agent listens
and thinks, so one mark carries every state instead of four unrelated icons.

These are macOS **template images**: pure black with an alpha channel, no colour.
macOS inverts them itself — dark on a light menu bar, light on a dark one — and
highlights them correctly when clicked. A full-colour icon cannot do that and
goes unreadable when the user switches theme.

Pure standard library on purpose. Committing the generator alongside the PNG
means the icon has a source: the geometry is a reviewable diff rather than
something you have to reverse-engineer in an image editor. It also keeps Pillow
and ImageMagick out of the project's prerequisites.

Usage:
    python3 tools/generate_tray_icon.py

Writes the states that have a working design into src-tauri/icons/tray/. Only
`idle` is wired up today; `listening` and `thinking` are the same mark filled in
and wait for M4 and M6.

`degraded` is deliberately absent. A cancel slash across three separated nodes
does not work at menu-bar size: the mark is discontinuous, so the bar alternates
between empty space and ring, and at every crossing you must either eat the ring
or break the bar. Neither reads at 22pt. It needs a different idea, designed in
M3 when the state exists and can be looked at in a real menu bar.
"""

from __future__ import annotations

import math
import struct
import zlib
from pathlib import Path

# 44px is a 22pt menu-bar slot at 2x. macOS scales down for 1x displays; going
# the other way would blur.
CANVAS = 44

# Distance from the centre of the canvas to each node's centre.
TRIANGLE_RADIUS = 13.0

NODE_RADIUS = 6.5

# Ring thickness for unfilled nodes. This is 1.3px at 1x, which is about the
# thinnest stroke that survives a menu bar; thinner rings vanish into the
# antialiasing rather than looking delicate.
RING_WIDTH = 2.6

# Coverage is computed by sampling each pixel on a grid; 4x4 is enough to make
# the curves smooth at this size and keeps the script instant.
SUPERSAMPLE = 4

OUTPUT_DIR = Path(__file__).resolve().parent.parent / "src-tauri" / "icons" / "tray"

# Which of the three nodes are solid, clockwise from the apex.
#
# The states mirror `ShellState` in src-tauri/src/tray.rs. Keep the two in step:
# `tray_icon_name` returns these file stems.
STATES: dict[str, tuple[bool, bool, bool]] = {
    "tray-idle": (False, False, False),
    "tray-listening": (True, False, False),
    "tray-thinking": (True, True, True),
    # No "tray-degraded" — see the module docstring.
}


def node_centres() -> list[tuple[float, float]]:
    """Three points on a circle, apex up, optically centred.

    A triangle's centroid is not the centre of its bounding box: with the apex
    up, the box runs from `-R - r` to `+R/2 + r`, so its middle sits `R/4` above
    the centroid. Placing the nodes on the raw centroid leaves the mark sitting
    high in the slot and reading as though it is falling out of the menu bar.
    Shifting down by `R/4` centres the box instead.
    """
    middle = CANVAS / 2
    optical_shift = TRIANGLE_RADIUS / 4
    return [
        (
            middle + TRIANGLE_RADIUS * math.cos(math.radians(angle)),
            middle + TRIANGLE_RADIUS * math.sin(math.radians(angle)) + optical_shift,
        )
        for angle in (-90, 30, 150)
    ]



def coverage_at(x: float, y: float, filled: tuple[bool, bool, bool]) -> bool:
    """Whether a single sample point falls inside the mark."""
    for (cx, cy), is_filled in zip(node_centres(), filled):
        distance = math.hypot(x - cx, y - cy)
        if is_filled:
            if distance <= NODE_RADIUS:
                return True
        elif NODE_RADIUS - RING_WIDTH <= distance <= NODE_RADIUS:
            return True

    return False


def render(filled: tuple[bool, bool, bool]) -> bytes:
    """Render to raw 8-bit grayscale + alpha rows."""
    step = 1.0 / SUPERSAMPLE
    offset = step / 2
    samples = SUPERSAMPLE * SUPERSAMPLE

    rows = bytearray()
    for py in range(CANVAS):
        rows.append(0)  # PNG filter type 0 (None) for this scanline
        for px in range(CANVAS):
            hits = 0
            for sy in range(SUPERSAMPLE):
                for sx in range(SUPERSAMPLE):
                    if coverage_at(
                        px + sx * step + offset,
                        py + sy * step + offset,
                        filled,
                    ):
                        hits += 1
            # Template images are black; only the alpha channel carries shape.
            rows.extend((0, round(255 * hits / samples)))
    return bytes(rows)


def write_png(path: Path, raw: bytes) -> None:
    """Write an 8-bit grayscale+alpha PNG (colour type 4)."""

    def chunk(kind: bytes, payload: bytes) -> bytes:
        return (
            struct.pack(">I", len(payload))
            + kind
            + payload
            + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)
        )

    header = struct.pack(">IIBBBBB", CANVAS, CANVAS, 8, 4, 0, 0, 0)
    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )
    path.write_bytes(png)


def main() -> None:
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    for name, filled in STATES.items():
        path = OUTPUT_DIR / f"{name}.png"
        write_png(path, render(filled))
        print(f"wrote {path.relative_to(OUTPUT_DIR.parents[2])} ({path.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
