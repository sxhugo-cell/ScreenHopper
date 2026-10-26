"""Renders the DMG installer background (design 5, "Hop In") at 1x and 2x.

Layout matches tauri.conf.json: window 660x400 pt, app icon at (180,170),
Applications at (480,170). Run: python3 make_background.py
"""
import math
from PIL import Image, ImageDraw, ImageFont, ImageFilter

W, H = 660, 400
STOPS = [(0.00, (0x34, 0xD3, 0x99)), (0.38, (0x10, 0xB9, 0x81)), (0.70, (0x0E, 0x9F, 0x9A)), (1.00, (0x0B, 0x6B, 0x8F))]
BOLD = ("/System/Library/Fonts/HelveticaNeue.ttc", 1)   # Helvetica Neue Bold
MONO = ("/System/Library/Fonts/Menlo.ttc", 0)


def lerp(a, b, t):
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


def color_at(t):
    for (t0, c0), (t1, c1) in zip(STOPS, STOPS[1:]):
        if t <= t1:
            return lerp(c0, c1, (t - t0) / (t1 - t0))
    return STOPS[-1][1]


def font(spec, size):
    return ImageFont.truetype(spec[0], size, index=spec[1])


def quad(p0, p1, p2, n=200):
    return [((1 - t) ** 2 * p0[0] + 2 * (1 - t) * t * p1[0] + t * t * p2[0],
             (1 - t) ** 2 * p0[1] + 2 * (1 - t) * t * p1[1] + t * t * p2[1]) for t in (i / n for i in range(n + 1))]


def render(s):
    w, h = W * s, H * s
    # 120deg CSS gradient: direction vector (sin120, -cos120)
    dx, dy = math.sin(math.radians(120)), -math.cos(math.radians(120))
    half = (abs(w * dx) + abs(h * dy)) / 2
    grad = Image.new("RGB", (w, h))
    px = grad.load()
    for y in range(h):
        for x in range(w):
            t = ((x - w / 2) * dx + (y - h / 2) * dy) / (2 * half) + 0.5
            px[x, y] = color_at(min(1, max(0, t)))
    img = grad.convert("RGBA")

    # Oversized watermark
    wm = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    ImageDraw.Draw(wm).text((0.05 * w, 0.03 * h), "HOP HOP", font=font(BOLD, round(59 * s)), fill=(255, 255, 255, 41))
    img = Image.alpha_composite(img, wm)

    # Dotted hop path between the icons, plus arrowhead
    layer = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    pts = quad((250, 150), (290, 80), (330, 150)) + quad((330, 150), (370, 80), (410, 150))[1:]
    gap, acc, last = 13, 0.0, pts[0]
    r = 2.1 * s
    d.ellipse([last[0] * s - r, last[1] * s - r, last[0] * s + r, last[1] * s + r], fill="white")
    tip = pts[-1]
    for p in pts[1:]:
        acc += math.dist(p, last)
        last = p
        if acc >= gap and math.dist(p, tip) > 16:
            acc = 0
            d.ellipse([p[0] * s - r, p[1] * s - r, p[0] * s + r, p[1] * s + r], fill="white")
    # Chevron aligned with the curve's final direction
    ang = math.atan2(150 - 80, 410 - 370)
    for side in (-1, 1):
        a = ang + math.pi + side * math.radians(38)
        end = (tip[0] + 13 * math.cos(a), tip[1] + 13 * math.sin(a))
        d.line([(tip[0] * s, tip[1] * s), (end[0] * s, end[1] * s)], fill="white", width=round(3.6 * s))
        for x, y in (tip, end):
            rr = 1.8 * s
            d.ellipse([x * s - rr, y * s - rr, x * s + rr, y * s + rr], fill="white")

    # Tagline
    big, small = font(BOLD, round(20 * s)), font(MONO, round(10.5 * s))
    for text, f, y, fill in [("Hop in.", big, 296, (255, 255, 255, 255)),
                             ("DRAG THE APP INTO APPLICATIONS", small, 324, (255, 255, 255, 190))]:
        tw = d.textlength(text, font=f)
        d.text(((w - tw) / 2, y * s), text, font=f, fill=fill)
    img = Image.alpha_composite(img, layer)
    return img.convert("RGB")


render(1).save("background.png")
render(2).save("background@2x.png")
print("ok")
