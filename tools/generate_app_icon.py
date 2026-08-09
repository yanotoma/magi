#!/usr/bin/env python3
"""Generate Magi's application icon.

The same mark as the menu bar — three nodes in a triangle, Melchior, Balthasar
and Casper — and a deliberately different treatment, because the two icons answer
different questions.

The tray icon is a macOS **template image**: pure black plus alpha, monochrome by
requirement, because macOS inverts it to suit the menu bar and a coloured one
would go unreadable when the user switches theme. It also carries state, filling
nodes in as the agent listens and thinks.

An application icon has neither constraint and one extra job: to be recognisable
at 16 pixels in a Finder list, in a keychain dialog, and in the Dock flash when
the app launches. So this one is full colour, high contrast, and always three
solid nodes — state belongs to the menu bar, where the user is actually looking
while Magi works.

Pure standard library, like `generate_tray_icon.py`, and for the same reason:
committing the generator beside the PNGs means the icon has a reviewable source
instead of being something to reverse-engineer in an image editor, and it keeps
Pillow and ImageMagick out of the project's prerequisites. `iconutil` does the
`.icns` packing and ships with macOS.

Usage:
    python3 tools/generate_app_icon.py

Writes every size `src-tauri/tauri.conf.json` asks for, plus the Windows
`Square*Logo.png` set that `tauri build` expects to exist.
"""

from __future__ import annotations

import math
import shutil
import struct
import subprocess
import zlib
from pathlib import Path

OUTPUT_DIR = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"

# Colours are fixed rather than derived from the CSS system colours the rest of
# the UI uses. An application icon is composited by Finder, the Dock and
# Keychain Access against backgrounds Magi does not control and cannot query, so
# "follows the theme" is not available here — unlike every surface inside the
# app, where `Canvas` and `CanvasText` do the work.
#
# Dark ground with a warm mark, which is the MAGI of the source material: amber
# text on a black terminal. It also happens to be the highest-contrast pairing
# available, which is what survives being drawn at sixteen pixels.
BACKGROUND = (0x1A, 0x1D, 0x24)
NODE = (0xFF, 0xA5, 0x2B)

# Fractions of the canvas, so every size is the same drawing rather than a
# resize. A 16px icon rendered directly is legible; a 1024px icon scaled down to
# 16px is mush, because antialiasing cannot recover a 2px ring.
# macOS icons float in their canvas rather than filling it: since Big Sur the
# squircle occupies roughly 80% of the side, and the transparent margin is what
# lets the Dock and Finder add their own shadow and spacing. An icon drawn edge to
# edge looks a size larger than its neighbours and slightly wrong in a way that is
# only obvious in a row of other icons.
SQUIRCLE_SIDE = 0.80

# Distance from the centre of the canvas to the centre of each node, and each
# node's radius, both as fractions of the canvas. Chosen together so the outer
# edge of a node lands at 0.35 — comfortably inside the 0.40 squircle edge. An
# earlier pass left a two-percent gap, which read as three dots pressed against
# the frame rather than a mark sitting in it.
TRIANGLE_RADIUS = 0.235
NODE_RADIUS = 0.115

# The exponent of the superellipse |x|^n + |y|^n = 1 that approximates the macOS
# icon shape. 5 is the usual fit; a plain rounded rectangle reads as visibly
# wrong beside other icons in the Dock, in a way that is hard to name until you
# see them side by side.
SQUIRCLE_EXPONENT = 5.0

SUPERSAMPLE = 4

# Every size the project needs. The `Square*Logo.png` names are Windows Store
# assets that `tauri build` expects to find; Magi does not target Windows yet,
# and a missing file there is a build failure rather than a missing feature.
SIZES: dict[str, int] = {
    "32x32.png": 32,
    "128x128.png": 128,
    "128x128@2x.png": 256,
    "icon.png": 512,
    "Square30x30Logo.png": 30,
    "Square44x44Logo.png": 44,
    "Square71x71Logo.png": 71,
    "Square89x89Logo.png": 89,
    "Square107x107Logo.png": 107,
    "Square142x142Logo.png": 142,
    "Square150x150Logo.png": 150,
    "Square284x284Logo.png": 284,
    "Square310x310Logo.png": 310,
    "StoreLogo.png": 50,
}

# The sizes `iconutil` wants inside an `.iconset`, as (file stem, pixels).
ICONSET: list[tuple[str, int]] = [
    ("icon_16x16", 16),
    ("icon_16x16@2x", 32),
    ("icon_32x32", 32),
    ("icon_32x32@2x", 64),
    ("icon_128x128", 128),
    ("icon_128x128@2x", 256),
    ("icon_256x256", 256),
    ("icon_256x256@2x", 512),
    ("icon_512x512", 512),
    ("icon_512x512@2x", 1024),
]


def inside_squircle(x: float, y: float, size: int) -> bool:
    """Whether a point is inside the rounded-square ground.

    A superellipse rather than a rounded rectangle. The difference is invisible
    described and obvious in the Dock: macOS icons share one silhouette, and an
    icon with the wrong one looks subtly foreign next to its neighbours.
    """
    half = size / 2
    # Normalised so the curve meets the squircle's own edge rather than the
    # canvas's, leaving the transparent margin macOS expects.
    reach = half * SQUIRCLE_SIDE
    nx = abs(x - half) / reach
    ny = abs(y - half) / reach
    return nx**SQUIRCLE_EXPONENT + ny**SQUIRCLE_EXPONENT <= 1.0


def node_centres(size: int) -> list[tuple[float, float]]:
    """Three points on a circle, apex up, optically centred.

    The same optical correction as the tray icon, and for the same reason: a
    triangle's centroid is not the centre of its bounding box, so nodes placed on
    the raw centroid leave the mark sitting high and looking as though it is
    sliding out of the frame. Shifting down by a quarter of the radius centres
    the box instead.
    """
    middle = size / 2
    radius = TRIANGLE_RADIUS * size
    shift = radius / 4
    return [
        (
            middle + radius * math.cos(math.radians(angle)),
            middle + radius * math.sin(math.radians(angle)) + shift,
        )
        for angle in (-90, 30, 150)
    ]


def sample(x: float, y: float, size: int) -> tuple[int, int, int, int] | None:
    """Which layer a single sample point lands on, or `None` for transparent."""
    if not inside_squircle(x, y, size):
        return None

    node_radius = NODE_RADIUS * size
    for cx, cy in node_centres(size):
        if math.hypot(x - cx, y - cy) <= node_radius:
            return (*NODE, 255)

    return (*BACKGROUND, 255)


def render(size: int) -> bytes:
    """Render to raw 8-bit RGBA scanlines with a PNG filter byte per row.

    Averages colour *and* alpha across the samples, which is what makes both the
    outer silhouette and the nodes' edges smooth. Compositing the nodes over an
    already-flattened background would leave them aliased against it.
    """
    step = 1.0 / SUPERSAMPLE
    offset = step / 2
    total = SUPERSAMPLE * SUPERSAMPLE

    rows = bytearray()
    for py in range(size):
        rows.append(0)  # PNG filter type 0 (None)
        for px in range(size):
            red = green = blue = alpha = 0
            for sy in range(SUPERSAMPLE):
                for sx in range(SUPERSAMPLE):
                    hit = sample(px + sx * step + offset, py + sy * step + offset, size)
                    if hit is not None:
                        red += hit[0]
                        green += hit[1]
                        blue += hit[2]
                        alpha += hit[3]

            if alpha == 0:
                rows.extend((0, 0, 0, 0))
                continue

            # Colour averaged over the samples that were *opaque*, so a partly
            # covered edge pixel keeps its own colour and only its alpha falls
            # off. Dividing by the full sample count instead would darken every
            # edge toward black and put a halo around the whole icon.
            covered = alpha // 255
            rows.extend(
                (
                    red // covered,
                    green // covered,
                    blue // covered,
                    alpha // total,
                )
            )
    return bytes(rows)


def write_png(path: Path, size: int, raw: bytes) -> None:
    """Write an 8-bit RGBA PNG (colour type 6)."""

    def chunk(kind: bytes, payload: bytes) -> bytes:
        return (
            struct.pack(">I", len(payload))
            + kind
            + payload
            + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)
        )

    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def write_ico(path: Path, sources: dict[int, bytes]) -> None:
    """Write a Windows `.ico` containing PNGs at each size.

    Hand-written because the alternative is a dependency for a platform Magi does
    not target yet, and the format is six fields and a directory. PNG-in-ICO is
    supported from Windows Vista; the older BMP form would mean writing a second
    encoder for no reader.
    """
    entries = bytearray()
    payloads = bytearray()
    offset = 6 + 16 * len(sources)

    for size in sorted(sources):
        data = sources[size]
        entries.extend(
            struct.pack(
                "<BBBBHHII",
                # 0 means 256 in this field, which is why it is a single byte.
                size if size < 256 else 0,
                size if size < 256 else 0,
                0,  # palette size: none, the PNG carries its own
                0,  # reserved
                1,  # colour planes
                32,  # bits per pixel
                len(data),
                offset,
            )
        )
        payloads.extend(data)
        offset += len(data)

    path.write_bytes(struct.pack("<HHH", 0, 1, len(sources)) + entries + payloads)


def main() -> None:
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)

    # Rendered once per distinct pixel size and reused, because several outputs
    # want the same dimensions and rendering is the slow part.
    rendered: dict[int, bytes] = {}

    def png_bytes(size: int) -> bytes:
        if size not in rendered:
            temporary = OUTPUT_DIR / f".render-{size}.png"
            write_png(temporary, size, render(size))
            rendered[size] = temporary.read_bytes()
            temporary.unlink()
        return rendered[size]

    for name, size in SIZES.items():
        (OUTPUT_DIR / name).write_bytes(png_bytes(size))
        print(f"wrote icons/{name} ({size}px)")

    # `.icns` via iconutil, which ships with macOS and knows the format's
    # retina-pair rules better than anything worth hand-rolling.
    if shutil.which("iconutil"):
        iconset = OUTPUT_DIR / "icon.iconset"
        if iconset.exists():
            shutil.rmtree(iconset)
        iconset.mkdir()
        for stem, size in ICONSET:
            (iconset / f"{stem}.png").write_bytes(png_bytes(size))

        subprocess.run(
            ["iconutil", "--convert", "icns", str(iconset), "--output", str(OUTPUT_DIR / "icon.icns")],
            check=True,
        )
        shutil.rmtree(iconset)
        print("wrote icons/icon.icns")
    else:
        print("iconutil not found; icon.icns left alone (run this on macOS)")

    write_ico(OUTPUT_DIR / "icon.ico", {size: png_bytes(size) for size in (16, 32, 48, 64, 128, 256)})
    print("wrote icons/icon.ico")


if __name__ == "__main__":
    main()
