#!/usr/bin/env python3
"""Compose App Store screenshots from raw simulator captures.

Canvas is 1320x2868 — the 6.9" slot (iPhone 17 Pro Max), which App Store Connect
also downscales for the smaller sizes, so this one set covers the listing.

The device capture bleeds off the bottom edge on purpose: it reads as a phone
continuing past the frame rather than a picture of a phone pasted on a colour.
"""
import os
from PIL import Image, ImageDraw, ImageFont

W, H = 1320, 2868
RAW = "/tmp/tacet-ss"
OUT = "/tmp/tacet-appstore"

NY = "/System/Library/Fonts/NewYork.ttf"
SF = "/System/Library/Fonts/SFNS.ttf"


def hexc(h):
    return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4))


# Every palette is taken from the app's own tokens; a marketing colour that is
# not in the product is a promise the product does not keep.
PALETTES = {
    "paper": dict(bg="F7F3EA", head="131C2E", sub="4C5568", accent="7E6410", mode="light"),
    "night": dict(bg="0B1220", head="E9E4D8", sub="97A1B4", accent="C9A227", mode="dark"),
    "ink":   dict(bg="131C2E", head="F7F3EA", sub="A9B2C4", accent="C9A227", mode="light"),
}

# (raw file, headline, subline). The headline claims only what the frame shows.
FRAMES_LIGHT = [
    ("05-now.png",       "Runs on your phone.\nNot in a data centre.",
     "The core never needs the network."),
    ("K-doc-light.png",  "Reads your spreadsheet.\nWrites a real one back.",
     "The .xlsx carries a live =SUM(), not a frozen number."),
    ("10-approval.png",  "Nothing leaves\nwithout your yes.",
     "You see the exact text before it is sent."),
    ("11-vat.png",       "Maths is run in code,\nnever guessed.",
     "Every tool it touches stays on screen."),
    ("06-hist.png",      "Calendar, notes, files,\ndocuments.",
     "It asks for a permission the moment it needs it — not before."),
    ("09-hist.png",      "Your conversations\nstay on this phone.",
     "No account. No sync. No telemetry."),
]

FRAMES_DARK = [
    ("probe.png",        "Runs on your phone.\nNot in a data centre.",
     "The core never needs the network."),
    ("K-doc-dark.png",   "Reads your spreadsheet.\nWrites a real one back.",
     "The .xlsx carries a live =SUM(), not a frozen number."),
    ("12-vat-dark.png",  "Maths is run in code,\nnever guessed.",
     "Every tool it touches stays on screen."),
]


def rounded(img, radius):
    mask = Image.new("L", img.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, img.size[0], img.size[1]], radius, fill=255)
    out = img.convert("RGBA")
    out.putalpha(mask)
    return out


def enso(draw, cx, cy, r, colour, width):
    """The brand mark: a brush circle left open, with a dot in the opening."""
    draw.arc([cx - r, cy - r, cx + r, cy + r], start=-24, end=287, fill=colour, width=width)
    import math
    a = math.radians(-48.5)
    dx, dy = cx + r * math.cos(a), cy + r * math.sin(a)
    d = width * 1.15
    draw.ellipse([dx - d, dy - d, dx + d, dy + d], fill=colour)


def wrap_center(draw, text, font, cx, y, fill, leading):
    for line in text.split("\n"):
        w = draw.textbbox((0, 0), line, font=font)[2]
        draw.text((cx - w / 2, y), line, font=font, fill=fill)
        y += leading
    return y


def compose(raw_name, headline, subline, palette_name):
    p = PALETTES[palette_name]
    canvas = Image.new("RGB", (W, H), hexc(p["bg"]))
    draw = ImageDraw.Draw(canvas)

    head_f = ImageFont.truetype(NY, 82)
    sub_f = ImageFont.truetype(SF, 40)

    enso(draw, W // 2, 250, 34, hexc(p["accent"]), 9)

    y = wrap_center(draw, headline, head_f, W // 2, 360, hexc(p["head"]), 104)
    wrap_center(draw, subline, sub_f, W // 2, y + 26, hexc(p["sub"]), 54)

    shot = Image.open(os.path.join(RAW, raw_name)).convert("RGB")
    target_w = 1136
    shot = shot.resize((target_w, int(shot.height * target_w / shot.width)), Image.LANCZOS)
    shot = rounded(shot, 78)

    top = 760
    shadow = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle(
        [(W - target_w) // 2, top + 10, (W + target_w) // 2, top + shot.height], 78,
        fill=(0, 0, 0, 60 if p["mode"] == "light" else 110))
    canvas = Image.alpha_composite(canvas.convert("RGBA"), shadow.filter(__import__("PIL.ImageFilter", fromlist=["x"]).GaussianBlur(26)))
    canvas.paste(shot, ((W - target_w) // 2, top), shot)

    return canvas.convert("RGB")


def main():
    os.makedirs(OUT, exist_ok=True)
    made = 0
    for palette in PALETTES:
        frames = FRAMES_DARK if palette == "night" else FRAMES_LIGHT
        folder = os.path.join(OUT, palette)
        os.makedirs(folder, exist_ok=True)
        for i, (raw, head, sub) in enumerate(frames, 1):
            if not os.path.exists(os.path.join(RAW, raw)):
                print(f"  ! missing {raw}, skipped")
                continue
            img = compose(raw, head, sub, palette)
            path = os.path.join(folder, f"{i:02d}.png")
            img.save(path, optimize=True)
            made += 1
            print(f"  {palette}/{i:02d}.png  {img.size[0]}x{img.size[1]}")
    print(f"\n{made} frames -> {OUT}")


if __name__ == "__main__":
    main()
