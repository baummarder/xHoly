#!/usr/bin/env python3
"""Render assets/favicon.ico from the geometry of assets/icon_xholy.svg.

Pure Python standard library only (struct/math) — no Pillow, no external
toolchain. The SVG remains the authoritative 256x256 vector; this script
rasterises the same shapes at 4x supersampling and writes a 32x32 ICO.

Usage: python tools/render_icon_xholy.py
"""
import math
import os
import struct

SIZE = 32
SS = 4               # supersampling factor -> 128x128 samples
U = 256.0            # SVG viewBox units
GOLD = (0xFF, 0xD7, 0x00)
BLACK = (0x05, 0x05, 0x05)
CROSS_FILL = (0x0A, 0x0A, 0x0A)

# Cross-frame polygon (same points as the SVG path).
CROSS = [
    (100, 20), (156, 20), (156, 100), (236, 100),
    (236, 156), (156, 156), (156, 236), (100, 236),
    (100, 156), (20, 156), (20, 100), (100, 100),
]

# X bars: two diagonals of the rotated 26x152 rectangles.
X_BARS = [((74.3, 74.3), (181.7, 181.7)), ((181.7, 74.3), (74.3, 181.7))]
X_HALF_WIDTH = 13.0


def inside_polygon(x, y, poly):
    inside = False
    n = len(poly)
    for i in range(n):
        x1, y1 = poly[i]
        x2, y2 = poly[(i + 1) % n]
        if (y1 > y) != (y2 > y):
            xi = x1 + (y - y1) * (x2 - x1) / (y2 - y1)
            if x < xi:
                inside = not inside
    return inside


def distance_to_segment(px, py, a, b):
    ax, ay = a
    bx, by = b
    dx, dy = bx - ax, by - ay
    length_sq = dx * dx + dy * dy
    if length_sq == 0:
        return math.hypot(px - ax, py - ay)
    t = max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / length_sq))
    return math.hypot(px - (ax + t * dx), py - (ay + t * dy))


def cross_edge_distance(x, y):
    best = float("inf")
    for i in range(len(CROSS)):
        best = min(best, distance_to_segment(x, y, CROSS[i], CROSS[(i + 1) % len(CROSS)]))
    return best


def sample(x, y):
    """Colour at one SVG-unit point."""
    # Outer fine gold border.
    edge = min(x, y, U - x, U - y)
    if edge < 2.5:
        return GOLD

    color = BLACK
    if inside_polygon(x, y, CROSS):
        color = CROSS_FILL
        if cross_edge_distance(x, y) < 2.0:
            color = GOLD

    # Gold X.
    if any(distance_to_segment(x, y, a, b) <= X_HALF_WIDTH for a, b in X_BARS):
        color = GOLD

    # Centre node: dark core, gold ring, gold dot.
    r = math.hypot(x - 128.0, y - 128.0)
    if 12.0 <= r <= 16.5:
        color = GOLD
    elif r < 12.0:
        color = BLACK
    if r <= 5.0:
        color = GOLD

    # Crucifix (top-right).
    if (192.0 <= x <= 200.0 and 42.0 <= y <= 82.0) or (183.0 <= x <= 209.0 and 48.0 <= y <= 56.0):
        color = GOLD
    return color


def raster():
    """Return 32x32 RGB pixels, top-down."""
    rows = []
    for py in range(SIZE):
        row = []
        for px in range(SIZE):
            acc = [0.0, 0.0, 0.0]
            for sy in range(SS):
                for sx in range(SS):
                    ux = (px + (sx + 0.5) / SS) * U / SIZE
                    uy = (py + (sy + 0.5) / SS) * U / SIZE
                    c = sample(ux, uy)
                    for i in range(3):
                        acc[i] += c[i]
            n = SS * SS
            row.append(tuple(int(round(v / n)) for v in acc))
        rows.append(row)
    return rows


def write_ico(rows, path):
    """Write a 32bpp BMP-based ICO (universally supported)."""
    w = h = SIZE
    # BGRA, bottom-up.
    pixels = bytearray()
    for y in range(h - 1, -1, -1):
        for r, g, b in rows[y]:
            pixels += bytes((b, g, r, 0xFF))
    # AND mask: 1 bit per pixel, rows padded to 4 bytes; zeros = use alpha.
    mask_row = ((w + 31) // 32) * 4
    mask = bytes(mask_row * h)
    dib = struct.pack(
        "<IiiHHIIiiII",
        40,            # BITMAPINFOHEADER size
        w, h * 2,      # height includes XOR + AND masks
        1, 32,         # planes, bpp
        0,             # BI_RGB
        len(pixels) + len(mask),
        0, 0, 0, 0,
    )
    image = dib + bytes(pixels) + mask
    header = struct.pack("<HHH", 0, 1, 1)
    entry = struct.pack("<BBBBHHII", w, h, 0, 0, 1, 32, len(image), 22)
    with open(path, "wb") as f:
        f.write(header + entry + image)


def main():
    root = os.path.join(os.path.dirname(__file__), "..")
    out = os.path.join(root, "assets", "favicon.ico")
    write_ico(raster(), out)
    print(f"wrote {out} ({SIZE}x{SIZE}) from assets/icon_xholy.svg")


if __name__ == "__main__":
    main()
