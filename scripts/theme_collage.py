#!/usr/bin/env python3
"""Composes the nine theme screenshots into the one picture the README shows.

    python3 scripts/theme_collage.py

Nine full-size screenshots in a README is nine screenfuls of scrolling for a section whose whole
point is that you can compare them at a glance. Composed into a grid they are one image, sized so
that the thing a reader is actually comparing — the colour of the frame, the bar, the tree — still
reads at thumbnail size even though the code in them does not.

Reads docs/screenshots/theme-*.png, writes docs/screenshots/themes.png.
"""
import os
import sys

from PIL import Image, ImageDraw, ImageFont

# Dark first, then light, in the order the theme drop-down lists them.
THEMES = [
    ("theme-cleecode.png", "CleeCode"),
    ("theme-turbo.png", "Turbo"),
    ("theme-solarized-dark.png", "Solarized Dark"),
    ("theme-eighties.png", "Eighties"),
    ("theme-mocha.png", "Mocha"),
    ("theme-cleecode-light.png", "CleeCode Light"),
    ("theme-solarized-light.png", "Solarized Light"),
    ("theme-ocean-light.png", "Ocean Light"),
    ("theme-github.png", "GitHub"),
]

COLUMNS = 3
TILE_WIDTH = 460
GUTTER = 10
LABEL_HEIGHT = 30
# Neutral, so the grid reads the same whether the page around it is light or dark, and so that no
# one theme's background is quietly acting as the mount for the other eight.
MOUNT = (24, 24, 26)
LABEL = (216, 216, 220)

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SHOTS = os.path.join(HERE, "docs", "screenshots")

FONT_CANDIDATES = [
    "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
    "/System/Library/Fonts/Helvetica.ttc",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
]


def font(size):
    for path in FONT_CANDIDATES:
        if os.path.exists(path):
            try:
                return ImageFont.truetype(path, size)
            except OSError:
                pass
    return ImageFont.load_default()


def main():
    tiles = []
    for name, label in THEMES:
        path = os.path.join(SHOTS, name)
        if not os.path.exists(path):
            sys.exit(f"missing screenshot: {path} — run vhs docs/shots-themes.tape first")
        shot = Image.open(path).convert("RGB")
        # A screenshot that never got past the shell is a few kilobytes of flat colour, and
        # `getcolors` returns a list for it and `None` for a real one — it gives up past its
        # limit. Catching it here beats finding it in the README: vhs reports success either way.
        if shot.getcolors(maxcolors=64) is not None:
            sys.exit(f"{name} looks blank — re-record it before composing")
        height = round(shot.height * TILE_WIDTH / shot.width)
        tiles.append((shot.resize((TILE_WIDTH, height), Image.LANCZOS), label))

    tile_height = max(t.height for t, _ in tiles)
    rows = (len(tiles) + COLUMNS - 1) // COLUMNS
    width = COLUMNS * TILE_WIDTH + (COLUMNS + 1) * GUTTER
    height = rows * (tile_height + LABEL_HEIGHT) + (rows + 1) * GUTTER

    sheet = Image.new("RGB", (width, height), MOUNT)
    draw = ImageDraw.Draw(sheet)
    name_font = font(19)

    for i, (tile, label) in enumerate(tiles):
        col, row = i % COLUMNS, i // COLUMNS
        x = GUTTER + col * (TILE_WIDTH + GUTTER)
        y = GUTTER + row * (tile_height + LABEL_HEIGHT + GUTTER)
        sheet.paste(tile, (x, y))
        draw.text(
            (x + TILE_WIDTH // 2, y + tile_height + LABEL_HEIGHT // 2),
            label,
            font=name_font,
            fill=LABEL,
            anchor="mm",
        )

    out = os.path.join(SHOTS, "themes.png")
    sheet.save(out, optimize=True)
    print(f"{out}  {sheet.width}x{sheet.height}")


if __name__ == "__main__":
    main()
