# Usage: python tools/make_icon.py assets/icon [dark|light]
"""Draw the Riffly icon: a stack of pages fanning out, as if riffled, on a
rounded monochrome tile. Writes a multi-size .ico and a 512px .png.

`dark` (the default) is white pages on a charcoal tile; `light` is charcoal
pages on an off-white tile."""
import sys
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

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


# ---------------------------------------------------------------- file icons
# What Explorer shows for files Riffly opens: a page with its kind written on
# it and a small Riffly badge in the corner, the way editors mark their files.

def font(size):
    for name in ["segoeuib.ttf", "arialbd.ttf", "DejaVuSans-Bold.ttf"]:
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            continue
    return ImageFont.load_default()


def draw_doc(label, detailed):
    img = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    # The page: white, with a hairline edge and a folded top-right corner.
    left, top, right, bottom = s(170), s(60), s(830), s(964)
    fold = s(190)
    outline = (*P_DOC["edge"], 255)
    page = [(left, top), (right - fold, top), (right, top + fold), (right, bottom), (left, bottom)]
    d.polygon(page, fill=(255, 255, 255, 255), outline=outline, width=s(14))
    d.polygon([(right - fold, top), (right - fold, top + fold), (right, top + fold)],
              fill=(*P_DOC["fold"], 255), outline=outline, width=s(14))
    if detailed:
        # A few lines of text…
        y = s(250)
        for frac in [0.78, 0.92, 0.6]:
            d.rounded_rectangle([left + s(80), y, left + s(80) + int((right - left - s(160)) * frac), y + s(34)],
                                radius=s(17), fill=(*P_DOC["line"], 255))
            y += s(80)
    if label:
        # …and the kind of file, in a band across the page.
        size = {2: 220, 3: 190, 4: 150}.get(len(label), 140)
        f = font(s(size if detailed else size + 30))
        box = d.textbbox((0, 0), label, font=f)
        w, h = box[2] - box[0], box[3] - box[1]
        cx = (left + right) // 2
        cy = s(505) if detailed else s(400)
        d.text((cx - w // 2 - box[0], cy - h // 2 - box[1]), label, font=f, fill=(*P_DOC["label"], 255))
    # The badge: the app icon, small, over the bottom-right corner.
    badge_size = s(420) if detailed else s(520)
    badge = draw(False).resize((badge_size, badge_size), Image.LANCZOS)
    img.alpha_composite(badge, (N - badge_size, N - badge_size))
    return img.resize((1024, 1024), Image.LANCZOS)


P_DOC = {"edge": (190, 189, 185), "fold": (232, 231, 227), "line": (214, 213, 209), "label": (55, 53, 47)}

for name, label in [("riffly-doc", ""), ("riffly-md", "MD"), ("riffly-html", "HTML"), ("riffly-pdf", "PDF")]:
    big_doc = draw_doc(label, True)
    small_doc = draw_doc(label, False)
    big_doc.resize((256, 256), Image.LANCZOS).save(OUT / f"{name}.png")
    doc_frames = []
    for size in [256, 128, 64, 48, 32, 24, 16]:
        src = big_doc if size >= 48 else small_doc
        doc_frames.append(src.resize((size, size), Image.LANCZOS))
    doc_frames[0].save(OUT / f"{name}.ico", format="ICO",
                       sizes=[f.size for f in doc_frames], append_images=doc_frames[1:])
print("wrote file icons")
