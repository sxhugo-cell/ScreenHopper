"""Paused variants of the tray icon.
 - tray-paused-template.png : macOS template (screens faded, pause bars solid)
 - win-normal.png / win-paused.png : Windows notification-area icons (paused = greyed + pause bars)
Run: python3 make_paused_icons.py"""
from PIL import Image, ImageDraw, ImageEnhance

S, N, OUT = 8, 22, 72
img = Image.new("RGBA", (N * S, N * S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)
faded = (0, 0, 0, 110)
lw = round(1.7 * S)
for box in [(1.2, 4.2, 10.2, 13.4), (11.8, 4.2, 20.8, 13.4)]:
    d.rounded_rectangle([v * S for v in box], radius=1.7 * S, outline=faded, width=lw)
# solid pause bars, bottom centre
for x in (8.6, 12.0):
    d.rounded_rectangle([x * S, 14.6 * S, (x + 2.4) * S, 21.4 * S], radius=0.7 * S, fill=(0, 0, 0, 255))
img.resize((OUT, OUT), Image.LANCZOS).save("tray-paused-template.png")

base = Image.open("../128x128.png").convert("RGBA").resize((64, 64), Image.LANCZOS)
base.save("win-normal.png")
grey = ImageEnhance.Color(base).enhance(0.0)
grey = ImageEnhance.Brightness(grey).enhance(0.8)
g = ImageDraw.Draw(grey)
g.rounded_rectangle([34, 34, 62, 62], radius=7, fill=(40, 44, 48, 255))
for x in (41, 50):
    g.rounded_rectangle([x, 40, x + 5, 56], radius=1.5, fill=(255, 255, 255, 255))
grey.save("win-paused.png")

# preview: normal vs paused, dark and light bar
prev = Image.new("RGBA", (4 * 92, 92), (0, 0, 0, 0))
for i, (name, bg, fg) in enumerate([("tray-template.png", (28, 38, 48, 255), 255), ("tray-paused-template.png", (28, 38, 48, 255), 255),
                                    ("tray-template.png", (236, 236, 236, 255), 0), ("tray-paused-template.png", (236, 236, 236, 255), 0)]):
    tile = Image.new("RGBA", (92, 92), bg)
    a = Image.open(name).split()[3]
    glyph = Image.new("RGBA", a.size, (fg, fg, fg, 255)); glyph.putalpha(a)
    tile.alpha_composite(glyph, (10, 10))
    prev.paste(tile, (i * 92, 0))
prev.save("paused-preview.png")
wp = Image.new("RGBA", (150, 72), (255, 255, 255, 255)); wp.alpha_composite(base, (4, 4)); wp.alpha_composite(grey, (80, 4)); wp.save("win-preview.png")
print("ok")
