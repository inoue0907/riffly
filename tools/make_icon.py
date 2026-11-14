# Usage: python tools/make_icon.py assets/icon [dark|light]
"""Draw the Riffly icon: a stack of pages fanning out, as if riffled, on a
rounded monochrome tile. Writes a multi-size .ico and a 512px .png.

`dark` (the default) is white pages on a charcoal tile; `light` is charcoal
pages on an off-white tile."""
import sys
from pathlib import Path
from PIL import Image, ImageDraw

OUT = Path(sys.argv[1])
OUT.mkdir(parents=True, exist_ok=True)
THEME = sys.argv[2] if len(sys.argv) > 2 else "dark"

PALETTES = {
    "dark": {
        "tile": (38, 37, 34),          # warm charcoal
        "border": None,
        "page": (255, 255, 255),
        "fan_alpha": [80, 150, 255],
        "line": (208, 207, 202),
        "box": (38, 37, 34),
        "check": (255, 255, 255),
    },
    "light": {
        "tile": (247, 247, 245),       # warm off-white
        "border": (222, 221, 217),     # so the tile holds its edge on white
        "page": (38, 37, 34),
        "fan_alpha": [60, 120, 255],
        "line": (118, 117, 112),
        "box": (247, 247, 245),
        "check": (38, 37, 34),
    },
}
P = PALETTES[THEME]

SS = 4            # supersampling
N = 1024 * SS     # working canvas


def s(v):
    return int(v * SS)


def page_layer(w, h, alpha, lines, line_w, cx=470):
    """One page, centred on a full canvas so it can be rotated about a pivot."""
    layer = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    x0 = s(cx) - s(w) // 2
    y0 = s(190)
    d.rounded_rectangle([x0, y0, x0 + s(w), y0 + s(h)], radius=s(44),
                        fill=(*P["page"], alpha))
    if lines:
        line = (*P["line"], 255)
        pad = s(78)
        y = y0 + s(120)
        for frac in [1.0, 0.86, 0.94, 0.58]:
            length = int((s(w) - 2 * pad) * frac)
            d.rounded_rectangle([x0 + pad, y, x0 + pad + length, y + s(line_w)],
                                radius=s(line_w) // 2, fill=line)
            y += s(line_w) + s(58)
        # the line that has been looked at: a ticked box
        y += s(10)
        d.rounded_rectangle([x0 + pad, y, x0 + pad + s(66), y + s(66)],
                            radius=s(16), fill=(*P["box"], 255))
        d.line([(x0 + pad + s(16), y + s(35)), (x0 + pad + s(29), y + s(48)),
                (x0 + pad + s(51), y + s(20))], fill=(*P["check"], 255),
               width=s(11), joint="curve")
        d.rounded_rectangle([x0 + pad + s(96), y + s(20), x0 + pad + s(300), y + s(20) + s(line_w)],
                            radius=s(line_w) // 2, fill=line)
    return layer


def draw(detailed):
    img = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    inset = s(40) if detailed else s(16)
    border = P["border"]
    d.rounded_rectangle([inset, inset, N - inset, N - inset],
                        radius=s(230) if detailed else s(200), fill=(*P["tile"], 255),
                        outline=(*border, 255) if border else None,
                        width=s(10) if border else 0)

    # The pages fan out from the bottom-left corner, where a thumb riffles them.
    pivot = (s(250), s(830))
    w, h = (470, 600) if detailed else (520, 640)
    fan = list(zip([-22, -11, 0], P["fan_alpha"]))
    for i, (angle, alpha) in enumerate(fan):
        front = i == len(fan) - 1
        layer = page_layer(w, h, alpha, lines=front and detailed, line_w=34,
                           cx=455 if detailed else 450)
        layer = layer.rotate(angle, resample=Image.BICUBIC, center=pivot)
        img.alpha_composite(layer)
    return img.resize((1024, 1024), Image.LANCZOS)


big = draw(True)
small = draw(False)
big.resize((512, 512), Image.LANCZOS).save(OUT / "riffly.png")

frames = []
for size in [256, 128, 64, 48, 32, 24, 16]:
    src = big if size >= 48 else small
    frames.append(src.resize((size, size), Image.LANCZOS))
frames[0].save(OUT / "riffly.ico", format="ICO",
               sizes=[f.size for f in frames], append_images=frames[1:])
print("wrote", sorted(p.name for p in OUT.iterdir()))
