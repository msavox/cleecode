#!/usr/bin/env python3
"""The minimal-mode screenshot the landing page shows.

    cargo build --release && python3 scripts/shot_minimal.py

Not a vhs tape like the other stills, for two reasons. The picture has to be taken somewhere
else: `clee -e` is the editor with everything else hidden, and photographing it inside this
repository would put a project's worth of context behind a picture whose whole claim is that
there is none — so it runs on one file in a throwaway directory, which is the situation the mode
is actually for. And the tape pipeline goes through ffmpeg, which stopped producing anything at
all on ffmpeg 9; the pty drivers beside this file do not, and they were already reading back the
same screen with its colours attached. This just draws that screen instead of asserting about it.

Writes docs/screenshots/minimal.png.
"""
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pty_drive  # noqa: E402

try:
    from PIL import Image, ImageDraw, ImageFont
except ImportError:
    sys.exit("needs pillow:  pip install pillow")

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FONT = os.path.join(HERE, "assets", "fonts", "CleeCodeMonoNerdFont-Regular.ttf")
OUT = os.path.join(HERE, "docs", "screenshots", "minimal.png")

# The window the picture is taken in, and it is a count of characters rather than of pixels: the
# page gives every figure the same width, so the grid alone decides how big the type comes out.
# The vhs stills beside this one are 1400 pixels of 16-point JetBrains Mono, which is about 145
# columns; a narrower window here would print the same page at half again the size and read as a
# screenshot taken by somebody who could not find their glasses.
COLS, ROWS = 126, 30

# Drawn at twice the size the page shows, so the picture survives a retina screen. Everything
# below is in these pixels; nothing is scaled afterwards.
FONT_PX = 30
PAD = 24

# pyte reports a cell that was never given a colour as "default". A terminal resolves those
# against its own palette, and CleeCode's dark surface is what the other screenshots were taken
# against.
DEFAULT_FG = "#e5e5e5"
DEFAULT_BG = "#181818"

# The eight names pyte hands back instead of a hex triple, in the xterm values the vhs shots
# were rendered with, so a colour the editor asks for by name lands where it always did.
NAMED = {
    "black": "#000000", "red": "#cd0000", "green": "#00cd00", "brown": "#cdcd00",
    "blue": "#0000ee", "magenta": "#cd00cd", "cyan": "#00cdcd", "white": "#e5e5e5",
    "brightblack": "#7f7f7f", "brightred": "#ff0000", "brightgreen": "#00ff00",
    "brightbrown": "#ffff00", "brightblue": "#5c5cff", "brightmagenta": "#ff00ff",
    "brightcyan": "#00ffff", "brightwhite": "#ffffff",
}

# A .txt and not a .md on purpose. Markdown opens with the formatting toolbar above the buffer,
# and a toolbar is the one thing a picture captioned "nothing around it" cannot have in it.
NOTE = """\
notes


today
  - cut 0.29.1, push the tap
  - reply to the packaging thread
  - read the pty resize patch back before it goes anywhere


to check
  - the 80x24 floor: what happens at 79 columns?
  - glibc on the arm64 box - which version does it really have?
  - does the splash still show under -w with an argument?
  - the counter says downloads_at is from Tuesday, so the refresh is failing


later
  A completion popup that ranks by distance from the cursor rather than by how
  often a word turns up: the name three lines above is the one you meant, and the
  one in a file opened on Tuesday is not. No model, no network, no ghost text -
  just the words already in the buffer, offered in the order a person would guess
  them.

  And it has to be non-modal. A popup that eats the next keystroke is a popup that
  gets turned off on the second day.
"""


def colour(value, fallback):
    """pyte's colour, as something PIL will take."""
    if value == "default":
        return fallback
    if value in NAMED:
        return NAMED[value]
    return "#" + value


def take(root):
    """Run `clee -e notes.txt` on one file and hand back the screen it drew."""
    with open(os.path.join(root, "notes.txt"), "w") as handle:
        handle.write(NOTE)

    # project=None: `-e` names the file itself, and a path argument beside it would be a second,
    # different instruction. The wait is on a line from the middle of the note, so the picture
    # cannot be taken against a buffer that is still being read in.
    session = pty_drive.Session(
        os.path.join(HERE, "target", "release", "clee"), root,
        args=["-e", "notes.txt"], project=None, cols=COLS, rows=ROWS,
    )
    session.wait(lambda s: "arm64 box" in "\n".join(s.screen.display), timeout=20)
    time.sleep(1)
    session.drain()
    return session


def draw(screen):
    font = ImageFont.truetype(FONT, FONT_PX)
    # The cell is the font's own advance width and a line height a terminal would use. Measuring
    # the advance rather than assuming 0.6em keeps the box-drawing characters joined up: the
    # frame CleeCode draws is made of them, and a cell half a pixel off shows as a dashed border.
    cell_w = round(font.getlength("M"))
    cell_h = round(FONT_PX * 1.2)

    image = Image.new("RGB", (COLS * cell_w + 2 * PAD, ROWS * cell_h + 2 * PAD), DEFAULT_BG)
    pen = ImageDraw.Draw(image)

    for y in range(ROWS):
        for x in range(COLS):
            char = screen.buffer[y][x]
            fg = colour(char.fg, DEFAULT_FG)
            bg = colour(char.bg, DEFAULT_BG)
            if char.reverse:
                fg, bg = bg, fg
            left, top = PAD + x * cell_w, PAD + y * cell_h
            if bg != DEFAULT_BG:
                pen.rectangle([left, top, left + cell_w, top + cell_h], fill=bg)
            if char.data and char.data != " ":
                pen.text((left, top), char.data, font=font, fill=fg)
                # Only the regular weight is vendored, so bold is drawn rather than picked: the
                # same glyph again a pixel across. At this size that is the thickening a bold
                # face would give and not a visible double image.
                if char.bold:
                    pen.text((left + 1, top), char.data, font=font, fill=fg)

    return image


def main():
    import tempfile

    with tempfile.TemporaryDirectory() as root:
        session = take(root)
        try:
            image = draw(session.screen)
        finally:
            session.close()

    image.save(OUT)
    print("%s  %dx%d" % (os.path.relpath(OUT, HERE), image.width, image.height))


if __name__ == "__main__":
    main()
