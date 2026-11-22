#!/usr/bin/env python3
"""Render xHoly-icon PNGs from the same geometry as xHoly-logo.svg.

Logo: bold gold X + small crucifix at the top right, inside a sphere
ring on a dark rounded-square background.

Usage: python tools/render_icons.py
Outputs PNGs into assets/icons/.
"""
from PIL import Image, ImageDraw
import math, os

BG = (0x1A, 0x1A, 0x1A, 255)
GOLD = (0xFF, 0xD7, 0x00, 255)

# Geometry in 512-viewBox units (must mirror xHoly-logo.svg)
RING_R = 196
RING_W = 10
X_HALF = 130      # half-length of each X bar
X_W = 64          # X bar width
X_RX = 16         # X bar corner radius
CRUC_POS = (352, 148)   # crucifix center (512 units)
CRUC_VW, CRUC_VH = 34, 112   # vertical beam w/h
CRUC_HW, CRUC_HH = 92, 28    # horizontal beam w/h
CRUC_RX = 9
NODE_R, NODE_RING_W, NODE_DOT_R = 30, 9, 9
CORNER_R = 112  # background corner radius


def _rotated_bar(size: int, w_units: float, h_units: float,
                 angle: float) -> Image.Image:
    """One rounded gold bar centered on the image, rotated by `angle`."""
    layer = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    s = size / 512.0
    cx = cy = size / 2.0
    w, h = w_units * s, h_units * s
    d.rounded_rectangle([cx - w / 2, cy - h / 2, cx + w / 2, cy + h / 2],
                        radius=min(w, h) * 0.25, fill=GOLD)
    return layer.rotate(angle, resample=Image.BICUBIC, center=(cx, cy))


def draw_x(img: Image.Image, size: int,
           half: float = X_HALF, w: float = X_W) -> None:
    """Composite the X (two rotated bars) onto img."""
    for angle in (-45, 45):
        img.alpha_composite(_rotated_bar(size, w, half * 2, angle))


def draw_crucifix(img: Image.Image, size: int,
                  pos=CRUC_POS,
                  vw: float = CRUC_VW, vh: float = CRUC_VH,
                  hw: float = CRUC_HW, hh: float = CRUC_HH) -> None:
    """Draw the upright crucifix at `pos` (512-unit coords)."""
    d = ImageDraw.Draw(img)
    s = size / 512.0
    cx, cy = pos[0] * s, pos[1] * s
    vw_, vh_ = vw * s, vh * s
    hw_, hh_ = hw * s, hh * s
    rx = CRUC_RX * s
    # vertical beam
    d.rounded_rectangle([cx - vw_ / 2, cy - vh_ / 2,
                         cx + vw_ / 2, cy + vh_ / 2],
                        radius=rx, fill=GOLD)
    # horizontal beam near the top
    hy = cy - vh_ / 2 + (vh_ - hh_) * 0.30
    d.rounded_rectangle([cx - hw_ / 2, hy, cx + hw_ / 2, hy + hh_],
                        radius=rx, fill=GOLD)


def render(size: int) -> Image.Image:
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    s = size / 512.0
    cx = cy = size / 2.0

    # Background
    d.rounded_rectangle([0, 0, size - 1, size - 1], radius=CORNER_R * s, fill=BG)

    # Sphere ring
    ring_w = max(1, int(RING_W * s))
    d.ellipse([cx - RING_R * s, cy - RING_R * s, cx + RING_R * s, cy + RING_R * s],
              outline=GOLD, width=ring_w)

    # X + crucifix
    draw_x(img, size)
    draw_crucifix(img, size)

    # Center node: dark fill, gold ring, gold dot
    d = ImageDraw.Draw(img)
    node_r = NODE_R * s
    d.ellipse([cx - node_r, cy - node_r, cx + node_r, cy + node_r], fill=BG)
    d.ellipse([cx - node_r, cy - node_r, cx + node_r, cy + node_r],
              outline=GOLD, width=max(1, int(NODE_RING_W * s)))
    dot_r = NODE_DOT_R * s
    d.ellipse([cx - dot_r, cy - dot_r, cx + dot_r, cy + dot_r], fill=GOLD)

    return img


def main() -> None:
    out_dir = os.path.join(os.path.dirname(__file__), "..", "assets", "icons")
    os.makedirs(out_dir, exist_ok=True)
    targets = {
        "xHoly-icon.png": 1024,
        "xHoly-icon-256.png": 256,
        "xHoly-icon-32.png": 32,
        "xHoly-icon-16.png": 16,
        "favicon.ico": None,  # handled below
    }
    for name, size in targets.items():
        if size is None:
            continue
        render(size).save(os.path.join(out_dir, name))
        print(f"wrote {name} ({size}x{size})")

    # favicon.ico bundling 16/32/48
    ico = render(48)
    ico.save(os.path.join(out_dir, "favicon.ico"),
             sizes=[(16, 16), (32, 32), (48, 48)])
    print("wrote favicon.ico (16/32/48)")


if __name__ == "__main__":
    main()
