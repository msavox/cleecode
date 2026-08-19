#!/usr/bin/env python3
"""Draws the CleeCode app icon and packs it into an .icns.

The icon is generated rather than drawn by hand so it can be changed by editing numbers
here — and so the .icns committed next to it can always be reproduced. Run it after any
change, from anywhere:

    python3 assets/icon/make-icon.py        # needs Pillow; iconutil and sips ship with macOS

It writes cleecode.png (1024px, the master) and CleeCode.icns (what the app bundle uses,
and what --install-app has compiled into it).

The subject is the project's turtle, seen from above, with the shell's scutes drawn as
terminal windows — the middle one focused, cyan-bordered and showing a prompt, the way a
focused frame looks in the app itself.
"""
import math
import os
import shutil
import subprocess
import sys
import tempfile

from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
S = 4096                      # drawn at 4x and downsampled: cheap, uniform antialiasing
U = S / 1024.0                # every number below is on a 1024 grid
CORNER = int(S * 0.2237)      # the macOS squircle's corner radius, as a fraction of the side

BG_TOP, BG_BOT = (32, 36, 42), (12, 13, 16)
SHELL, PLATE, EDGE, SKIN = (47, 158, 68), (72, 195, 90), (18, 74, 40), (126, 214, 168)
SCREEN, DIM, TEXT = (13, 15, 18), (74, 84, 94), (222, 226, 230)
CYAN, GREEN = (45, 212, 191), (63, 185, 80)


def backdrop():
    """The rounded square: a near-black vertical gradient with a lit top edge."""
    column = Image.new("RGB", (1, S))
    for y in range(S):
        t = y / (S - 1)
        column.putpixel((0, y), tuple(int(BG_TOP[i] + (BG_BOT[i] - BG_TOP[i]) * t) for i in range(3)))
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, S - 1, S - 1], radius=CORNER, fill=255)
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    img.paste(column.resize((S, S)), (0, 0), mask)
    ImageDraw.Draw(img).rounded_rectangle(
        [6, 6, S - 7, S - 7], radius=CORNER - 6, outline=(255, 255, 255, 30), width=int(4 * U)
    )
    return img


def ellipse(d, cx, cy, rx, ry, angle, fill):
    """A rotated ellipse. PIL's own has no rotation, and the flippers need one."""
    a = math.radians(angle)
    pts = []
    for i in range(96):
        t = 2 * math.pi * i / 96
        x, y = rx * math.cos(t), ry * math.sin(t)
        pts.append((cx + x * math.cos(a) - y * math.sin(a), cy + x * math.sin(a) + y * math.cos(a)))
    d.polygon(pts, fill=fill)


def hexagon(cx, cy, r, rot=90):
    return [
        (cx + r * math.cos(math.radians(rot + 60 * i)), cy + r * math.sin(math.radians(rot + 60 * i)))
        for i in range(6)
    ]


def scute(d, cx, cy, r, focused):
    """One plate of the shell, drawn as a small terminal: dark screen, prompt, output."""
    d.polygon(
        hexagon(cx, cy, r),
        fill=SCREEN,
        outline=CYAN if focused else PLATE,
        width=int((7 if focused else 5) * U),
    )
    # Lines are inset from the hexagon's full width so they clear the slanted sides.
    half = r * math.sqrt(3) / 2 * 0.74
    rows = [(GREEN, 0.58), (DIM, 0.46), (GREEN, 0.16)] if focused else [(DIM, 0.55), (DIM, 0.34), (DIM, 0.66)]
    height, step = 11 * U, 30 * U
    top = cy - step * (len(rows) - 1) / 2
    for i, (colour, width) in enumerate(rows):
        y = top + i * step
        x = cx - half
        if colour is GREEN:  # a prompt mark, then the line being typed at it
            d.rounded_rectangle([x, y - height / 2, x + 16 * U, y + height / 2], radius=int(4 * U), fill=GREEN)
            d.rounded_rectangle(
                [x + 26 * U, y - height / 2, x + 26 * U + 2 * half * width, y + height / 2],
                radius=int(4 * U),
                fill=DIM if i else TEXT,
            )
        else:
            d.rounded_rectangle(
                [x, y - height / 2, x + 2 * half * width, y + height / 2], radius=int(4 * U), fill=colour
            )


def draw():
    img = backdrop()
    d = ImageDraw.Draw(img)
    c = S / 2
    ellipse(d, c, c - 292 * U, 86 * U, 98 * U, 0, SKIN)                      # head
    for x, y, angle in [(-232, -196, -38), (232, -196, 38), (-232, 196, 38), (232, 196, -38)]:
        ellipse(d, c + x * U, c + y * U, 128 * U, 66 * U, angle, SKIN)       # flippers
    ellipse(d, c, c + 300 * U, 40 * U, 62 * U, 0, SKIN)                      # tail
    for x in (-36, 36):
        ellipse(d, c + x * U, c - 322 * U, 13 * U, 15 * U, 0, (16, 26, 20))  # eyes
    d.ellipse([c - 264 * U, c - 264 * U, c + 264 * U, c + 264 * U], fill=EDGE)
    d.ellipse([c - 250 * U, c - 250 * U, c + 250 * U, c + 250 * U], fill=SHELL)
    # Seven scutes in a honeycomb: centres a hexagon's flat-to-flat apart, so they tile
    # without overlapping, then shrunk slightly to leave a green gutter between them.
    r = 82 * U
    pitch = r * math.sqrt(3)
    scute(d, c, c, r * 0.93, True)
    for i in range(6):
        a = math.radians(60 * i)
        scute(d, c + pitch * math.cos(a), c + pitch * math.sin(a), r * 0.93, False)
    return img.resize((1024, 1024), Image.LANCZOS)


def main():
    png = os.path.join(HERE, "cleecode.png")
    draw().save(png)
    print(f"wrote {png}")

    if sys.platform != "darwin":
        print("skipping the .icns: iconutil is macOS-only")
        return
    master = Image.open(png)
    with tempfile.TemporaryDirectory() as tmp:
        iconset = os.path.join(tmp, "CleeCode.iconset")
        os.makedirs(iconset)
        # The sizes Finder, the Dock, Launchpad and the Get Info panel actually ask for.
        for size in (16, 32, 128, 256, 512):
            master.resize((size, size), Image.LANCZOS).save(os.path.join(iconset, f"icon_{size}x{size}.png"))
            master.resize((size * 2, size * 2), Image.LANCZOS).save(
                os.path.join(iconset, f"icon_{size}x{size}@2x.png")
            )
        icns = os.path.join(HERE, "CleeCode.icns")
        subprocess.run(["iconutil", "-c", "icns", iconset, "-o", icns], check=True)
        print(f"wrote {icns} ({os.path.getsize(icns) // 1024} KB)")


if __name__ == "__main__":
    main()
