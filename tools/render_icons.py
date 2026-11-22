#!/usr/bin/env python3
"""Render xHoly-icon PNGs from the same geometry as xHoly-logo.svg.

Logo: bold gold X + small crucifix at the top right, inside a sphere
ring on a dark rounded-square background.

Usage: python tools/render_icons.py
Outputs PNGs into assets/icons/.
"""
from PIL import Image, ImageDraw, ImageFilter
import math, os

BG = (0x14, 0x14, 0x14, 255)
BG_MID = (0x1C, 0x1C, 0x1C, 255)
GOLD = (0xFF, 0xD7, 0x00, 255)
GOLD_HI = (0xFF, 0xE6, 0x80, 255)
GOLD_LO = (0xD9, 0xA8, 0x00, 255)

# Geometry in 512-viewBox units (must mirror xHoly-logo.svg)
RING_R = 196
RING_W = 9
INNER_RING_R = 178
INNER_RING_W = 2
X_HALF = 128      # half-length of each X bar
X_W = 68          # X bar width
NODE_R, NODE_RING_W, NODE_DOT_R = 34, 9, 10
CRUC_POS = (350, 140)   # crucifix center (512 units)
CRUC_VW, CRUC_VH = 36, 116   # vertical beam w/h
CRUC_HW, CRUC_HH = 96, 30    # horizontal beam w/h
CORNER_R = 112  # background corner radius


def _vgrad(size: int) -> Image.Image:
    """Vertical gold gradient (top bright → bottom deep) as RGBA."""
    g = Image.new("RGBA", (1, size))
    for y in range(size):
        t = y / max(1, size - 1)
        # gold_hi -> gold -> gold_lo
        if t < 0.45:
            f = t / 0.45
            c = tuple(int(GOLD_HI[i] + (GOLD[i] - GOLD_HI[i]) * f) for i in range(3))
        else:
            f = (t - 0.45) / 0.55
            c = tuple(int(GOLD[i] + (GOLD_LO[i] - GOLD[i]) * f) for i in range(3))
        g.putpixel((0, y), c + (255,))
    return g.resize((size, size))


def _gold_shape(layer: Image.Image, box, radius: float) -> None:
    """Fill `box` on `layer` with the vertical gold gradient."""
    x0, y0, x1, y1 = [int(v) for v in box]
    w, h = x1 - x0, y1 - y0
    if w <= 0 or h <= 0:
        return
    grad = _vgrad(h).resize((w, h))
    mask = Image.new("L", (w, h), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, w - 1, h - 1],
                                           radius=int(radius), fill=255)
    layer.paste(grad, (x0, y0), mask)


def _rotated_bar(size: int, w_units: float, h_units: float,
                 angle: float) -> Image.Image:
    """One gradient gold bar centered on the image, rotated by `angle`."""
    layer = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    s = size / 512.0
    cx = cy = size / 2.0
    w, h = w_units * s, h_units * s
    _gold_shape(layer, (cx - w / 2, cy - h / 2, cx + w / 2, cy + h / 2),
                min(w, h) * 0.25)
    # subtle top highlight
    d = ImageDraw.Draw(layer)
    d.rounded_rectangle([cx - w / 2, cy - h / 2, cx + w / 2,
                         cy - h / 2 + h * 0.22],
                        radius=min(w, h) * 0.25,
                        fill=(255, 255, 255, 26))
    return layer.rotate(angle, resample=Image.BICUBIC, center=(cx, cy))


def draw_x(img: Image.Image, size: int,
           half: float = X_HALF, w: float = X_W) -> None:
    """Composite the X (two rotated gradient bars) onto img."""
    for angle in (-45, 45):
        img.alpha_composite(_rotated_bar(size, w, half * 2, angle))


def draw_crucifix(img: Image.Image, size: int,
                  pos=CRUC_POS,
                  vw: float = CRUC_VW, vh: float = CRUC_VH,
                  hw: float = CRUC_HW, hh: float = CRUC_HH) -> None:
    """Draw the upright gradient crucifix at `pos` (512-unit coords)."""
    s = size / 512.0
    cx, cy = pos[0] * s, pos[1] * s
    vw_, vh_ = vw * s, vh * s
    hw_, hh_ = hw * s, hh * s
    # vertical beam
    _gold_shape(img, (cx - vw_ / 2, cy - vh_ / 2, cx + vw_ / 2, cy + vh_ / 2),
                vw_ * 0.25)
    # horizontal beam near the top
    hy = cy - vh_ / 2 + (vh_ - hh_) * 0.30
    _gold_shape(img, (cx - hw_ / 2, hy, cx + hw_ / 2, hy + hh_), hh_ * 0.35)
    # top highlight on the vertical beam
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([cx - vw_ / 2, cy - vh_ / 2, cx + vw_ / 2,
                         cy - vh_ / 2 + vh_ * 0.22],
                        radius=vw_ * 0.25, fill=(255, 255, 255, 26))


def render(size: int) -> Image.Image:
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    s = size / 512.0
    cx = cy = size / 2.0

    # Background: vertical gradient dark
    bg = _vgrad(size)
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [0, 0, size - 1, size - 1], radius=CORNER_R * s, fill=255)
    img.paste(bg, (0, 0), mask)
    img = img.convert("RGBA")

    # Sphere ring + faint inner ring
    ring_w = max(1, int(RING_W * s))
    d = ImageDraw.Draw(img)
    d.ellipse([cx - RING_R * s, cy - RING_R * s, cx + RING_R * s, cy + RING_R * s],
              outline=GOLD, width=ring_w)
    d.ellipse([cx - INNER_RING_R * s, cy - INNER_RING_R * s,
               cx + INNER_RING_R * s, cy + INNER_RING_R * s],
              outline=GOLD, width=max(1, int(INNER_RING_W * s)))

    # Ambient glow behind the mark
    glow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    gd = ImageDraw.Draw(glow)
    gr = 118 * s
    gd.ellipse([cx - gr, cy - gr, cx + gr, cy + gr], fill=(255, 215, 0, 18))
    glow = glow.filter(ImageFilter.GaussianBlur(radius=max(1.0, 10 * s)))
    img = Image.alpha_composite(img, glow)

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
