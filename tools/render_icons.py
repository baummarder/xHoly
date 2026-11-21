#!/usr/bin/env python3
"""Render xHoly-icon PNGs from the same geometry as xHoly-logo.svg.

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
BEAM_W, BEAM_H = 60, 336
SERIF_W, SERIF_H = 192, 34
NODE_R, NODE_RING_W, NODE_DOT_R = 30, 9, 9
CORNER_R = 112  # background corner radius


def cross_layer(size: int, angle: float) -> Image.Image:
    """Draw one crucifix-cross (beam + top serif) and rotate it."""
    layer = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    s = size / 512.0
    cx = cy = size / 2.0

    beam_w, beam_h = BEAM_W * s, BEAM_H * s
    serif_w, serif_h = SERIF_W * s, SERIF_H * s
    beam_x0, beam_y0 = cx - beam_w / 2, cy - beam_h / 2
    # serif sits near the top of the beam
    serif_x0 = cx - serif_w / 2
    serif_y0 = beam_y0 + (BEAM_H - SERIF_H) * s * 0.18

    d.rounded_rectangle([beam_x0, beam_y0, beam_x0 + beam_w, beam_y0 + beam_h],
                        radius=beam_w * 0.28, fill=GOLD)
    d.rounded_rectangle([serif_x0, serif_y0, serif_x0 + serif_w, serif_y0 + serif_h],
                        radius=serif_h * 0.42, fill=GOLD)
    return layer.rotate(angle, resample=Image.BICUBIC, center=(cx, cy))


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

    # Two crosses
    img.alpha_composite(cross_layer(size, -45))
    img.alpha_composite(cross_layer(size, 45))

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
