#!/usr/bin/env python3
"""Cuts every CleeCode icon from the one master rendering, and packs the .icns.

The icon is no longer drawn here: it is `site/icon-master.png`, a finished rendering,
and this script's whole job is the *cut* — masking everything outside the mark's own
rounded square to transparency, which takes the outer glow with it, and then exporting
each size from that one cleaned image so none of them can drift from the others. Run it
after the master changes, from anywhere:

    python3 assets/icon/make-icon.py        # needs Pillow; iconutil ships with macOS

It writes:
  assets/icon/cleecode.png   the 1024 app-icon master: the mark on Apple's icon grid
                             (the rounded square at 824 of 1024), transparent margins —
                             what a macOS icon is expected to be shaped like
  assets/icon/CleeCode.icns  what the app bundle uses, and what --install-app has
                             compiled into it
  site/icon-512.png          the favicon family: the cleaned mark full-bleed,
  site/icon-180.png          transparent outside its rounded corners
  site/icon-32.png
"""
import os
import subprocess
import sys
import tempfile

from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
MASTER = os.path.join(ROOT, "site", "icon-master.png")

# Where the mark's rounded square sits in the master, measured off the rendering itself:
# the outer edge of the luminous rim, plus two pixels so the rim keeps its antialiasing.
# Everything outside this box — which is exactly the halo bleeding into the black — goes.
BOX = (42, 32, 679, 686)
# The rim's own corner radius at that edge, measured the same way.
RADIUS = 126
# Supersampling for the mask: drawn at 4x and downsampled, so the cut edge is as smooth
# as the rim it follows.
SS = 4


def cleaned():
    """The master with everything outside its rounded square made transparent."""
    img = Image.open(MASTER).convert("RGBA")
    mask = Image.new("L", (img.width * SS, img.height * SS), 0)
    x0, y0, x1, y1 = (v * SS for v in BOX)
    ImageDraw.Draw(mask).rounded_rectangle([x0, y0, x1, y1], radius=RADIUS * SS, fill=255)
    img.putalpha(mask.resize(img.size, Image.LANCZOS))
    return img.crop(BOX)


def on_apple_grid(mark):
    """The mark placed as a macOS app icon: 824 of a transparent 1024, centred.

    The margin is not padding for its own sake — it is Apple's icon grid, the frame every
    Dock icon is drawn in, and a full-bleed icon sits visibly oversized beside them.
    """
    side = 824
    scale = side / max(mark.size)
    fitted = mark.resize((round(mark.width * scale), round(mark.height * scale)), Image.LANCZOS)
    canvas = Image.new("RGBA", (1024, 1024), (0, 0, 0, 0))
    canvas.paste(fitted, ((1024 - fitted.width) // 2, (1024 - fitted.height) // 2), fitted)
    return canvas


def square(mark, side):
    """The mark full-bleed at `side`, on a transparent square — the favicon shape."""
    scale = side / max(mark.size)
    fitted = mark.resize((round(mark.width * scale), round(mark.height * scale)), Image.LANCZOS)
    canvas = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    canvas.paste(fitted, ((side - fitted.width) // 2, (side - fitted.height) // 2), fitted)
    return canvas


def main():
    mark = cleaned()

    for size in (512, 180, 32):
        path = os.path.join(ROOT, "site", f"icon-{size}.png")
        square(mark, size).save(path)
        print(f"wrote {path}")

    png = os.path.join(HERE, "cleecode.png")
    master = on_apple_grid(mark)
    master.save(png)
    print(f"wrote {png}")

    if sys.platform != "darwin":
        print("skipping the .icns: iconutil is macOS-only")
        return
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
