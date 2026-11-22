#!/usr/bin/env python3
"""Render xHoly-app-icon / xHoly-window PNGs matching the SVG icons.

Usage: python tools/render_app_icon.py
Outputs PNGs into assets/icons/ at 256/128/64/32 px.
"""
from PIL import Image, ImageDraw, ImageFilter
import math, os, sys

sys.path.insert(0, os.path.dirname(__file__))
from render_icons import cross_layer, GOLD, BG

OUT_DIR = os.path.join(os.path.dirname(__file__), "..", "assets", "icons")

# App-icon specific geometry (512 viewBox units)
APP_CORNER_R = 116
APP_RING_R = 188
APP_FRAME_INSET = 14
APP_FRAME_R = 102
WIN_CORNER_R = 96
WIN_RING_R = 180


def render_app_icon(size: int) -> Image.Image:
    s = size / 512.0
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    cx = cy = size / 2.0

    # Rounded-square background with subtle vertical sheen
    d.rounded_rectangle([0, 0, size - 1, size - 1], radius=APP_CORNER_R * s,
                        fill=BG)
    sheen = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    sd = ImageDraw.Draw(sheen)
    sd.rounded_rectangle([0, 0, size - 1, size // 3],
                         radius=APP_CORNER_R * s, fill=(255, 255, 255, 14))
    img = Image.alpha_composite(img, sheen)
    d = ImageDraw.Draw(img)

    # Sphere ring
    ring_w = 10 * s
    d.ellipse([cx - APP_RING_R * s - ring_w / 2, cy - APP_RING_R * s - ring_w / 2,
               cx + APP_RING_R * s + ring_w / 2, cy + APP_RING_R * s + ring_w / 2],
              outline=GOLD, width=max(1, int(ring_w)))

    # Two crucifix crosses (reuse logo geometry, slightly smaller)
    for angle in (-45, 45):
        layer = cross_layer(size, angle)
        # cross_layer uses logo metrics; scale it down a touch via a
        # centered crop-mask is overkill — instead draw at 0.94 zoom by
        # past through a pre-scaled layer.
        zoom = layer.resize((int(size * 0.94), int(size * 0.94)),
                            Image.LANCZOS)
        ox = (size - zoom.width) // 2
        oy = (size - zoom.height) // 2
        img.alpha_composite(zoom, (ox, oy))

    # Center node
    node_r = 30 * s
    ring_w2 = 9 * s
    dot_r = 9 * s
    d.ellipse([cx - node_r, cy - node_r, cx + node_r, cy + node_r], fill=BG)
    d.ellipse([cx - node_r, cy - node_r, cx + node_r, cy + node_r],
              outline=GOLD, width=max(1, int(ring_w2)))
    d.ellipse([cx - dot_r, cy - dot_r, cx + dot_r, cy + dot_r], fill=GOLD)

    # Gold frame: blurred glow pass + crisp pass
    inset = APP_FRAME_INSET * s
    frame_r = APP_FRAME_R * s
    fw = max(1, int(10 * s))
    glow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    gd = ImageDraw.Draw(glow)
    gd.rounded_rectangle([inset, inset, size - inset - 1, size - inset - 1],
                         radius=frame_r, outline=GOLD, width=fw)
    glow = glow.filter(ImageFilter.GaussianBlur(radius=max(1.0, 8 * s)))
    img = Image.alpha_composite(img, glow)
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([inset, inset, size - inset - 1, size - inset - 1],
                        radius=frame_r, outline=GOLD, width=max(1, int(5 * s)))

    return img


def render_window_icon(size: int) -> Image.Image:
    s = size / 512.0
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    cx = cy = size / 2.0

    d.rounded_rectangle([0, 0, size - 1, size - 1], radius=WIN_CORNER_R * s,
                        fill=BG)

    ring_w = 9 * s
    d.ellipse([cx - WIN_RING_R * s - ring_w / 2, cy - WIN_RING_R * s - ring_w / 2,
               cx + WIN_RING_R * s + ring_w / 2, cy + WIN_RING_R * s + ring_w / 2],
              outline=GOLD, width=max(1, int(ring_w)))

    for angle in (-45, 45):
        layer = cross_layer(size, angle)
        zoom = layer.resize((int(size * 0.90), int(size * 0.90)),
                            Image.LANCZOS)
        ox = (size - zoom.width) // 2
        oy = (size - zoom.height) // 2
        img.alpha_composite(zoom, (ox, oy))

    node_r = 27 * s
    d.ellipse([cx - node_r, cy - node_r, cx + node_r, cy + node_r], fill=BG)
    d.ellipse([cx - node_r, cy - node_r, cx + node_r, cy + node_r],
              outline=GOLD, width=max(1, int(8 * s)))
    dot_r = 8 * s
    d.ellipse([cx - dot_r, cy - dot_r, cx + dot_r, cy + dot_r], fill=GOLD)

    return img


def main() -> None:
    os.makedirs(OUT_DIR, exist_ok=True)
    for name, fn in [("xHoly-app-icon", render_app_icon),
                     ("xHoly-window", render_window_icon)]:
        for size in (256, 128, 64, 32):
            fn(size).save(os.path.join(OUT_DIR, f"{name}-{size}.png"))
            print(f"wrote {name}-{size}.png")


if __name__ == "__main__":
    main()
