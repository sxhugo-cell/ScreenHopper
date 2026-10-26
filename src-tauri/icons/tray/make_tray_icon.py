"""Menu-bar icon: two screens + cursor, outline. Black on transparent = macOS 'template' image
(macOS draws it white on a dark menu bar and black on a light one). Run: python3 make_tray_icon.py"""
from PIL import Image, ImageDraw

S = 8                       # supersample
N = 22                      # design grid (points)
OUT = 72                    # exported pixels (tray-icon scales to the menu-bar height)
img = Image.new("RGBA", (N * S, N * S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)
BLACK = (0, 0, 0, 255)
lw = round(1.7 * S)

def rr(box, r):
    d.rounded_rectangle([v * S for v in box], radius=r * S, outline=BLACK, width=lw)

rr((1.2, 4.2, 10.2, 13.4), 1.7)      # left screen
rr((11.8, 4.2, 20.8, 13.4), 1.7)     # right screen
# cursor arrow dropping between and below the screens
cur = [(9.4, 11.0), (9.4, 20.6), (11.8, 18.4), (13.6, 21.6), (15.3, 20.7), (13.5, 17.6), (16.8, 17.4)]
d.polygon([(x * S, y * S) for x, y in cur], fill=BLACK)
# knock a thin gap around the cursor so it reads against the screen outlines
gap = Image.new("L", img.size, 0)
ImageDraw.Draw(gap).line([(x * S, y * S) for x, y in cur + [cur[0]]], fill=255, width=round(2.6 * S), joint="curve")
inner = Image.new("L", img.size, 0)
ImageDraw.Draw(inner).polygon([(x * S, y * S) for x, y in cur], fill=255)
a = img.split()[3]
cut = Image.eval(gap, lambda v: 255 - v)
a = Image.composite(a, Image.new("L", img.size, 0), cut)
a = Image.composite(Image.new("L", img.size, 255), a, inner)
img.putalpha(a)
img = img.resize((OUT, OUT), Image.LANCZOS)
img.save("tray-template.png")
# preview on a dark and a light bar
prev = Image.new("RGBA", (OUT * 2 + 30, OUT + 20), (0, 0, 0, 0))
for i, (bg, fg) in enumerate([((28, 38, 48, 255), 255), ((236, 236, 236, 255), 0)]):
    tile = Image.new("RGBA", (OUT + 10, OUT + 20), bg)
    glyph = Image.new("RGBA", img.size, (fg, fg, fg, 255)); glyph.putalpha(img.split()[3])
    tile.alpha_composite(glyph, (5, 10))
    prev.paste(tile, (i * (OUT + 20), 0))
prev.save("tray-preview.png")
print("ok")
