#!/usr/bin/env python3
"""Open an animated GIF and watch it actually move.

    python3 scripts/drive_gif.py [path/to/clee]

A `.gif` used to open as its own first frame and stand there. What is being checked here is the
only thing that can prove it no longer does: the *pane changes by itself*, on the file's own
clock, with nobody touching a key — and then comes back round, because an animation that stops
on its last frame is indistinguishable from one that broke.

Three rules ride along with it, and each is a check of its own. Nothing blinks: no frame of
`Loading`, and no empty pane, between two pictures. Nothing is stolen: the keyboard stays in the
file being written in the other half of the split, and the character typed there lands while the
picture beside it goes on moving. And nothing is a fixed tick: the file asks for 200 ms a frame,
so a couple of seconds must hold roughly ten changes — not sixty, which is what an animation
driven by the event loop rather than by the file would give.

The GIF is written here, byte by byte, rather than made with a library. PIL is not something a
machine running this can be assumed to have, and a checked-in binary fixture would state the
frames and their delays somewhere nobody reading this file can see them. Forty lines of GIF89a
is a small price for a fixture whose every byte is in the source.

The pictures render as half-blocks, because a bare pty answers no graphics query — the same
fallback a terminal without kitty or sixel gets. What the checks read is the *colour* those
cells are painted in: two frames of two different colours are two different screens, and the
characters are the same in both.
"""

import os
import shutil
import sys
import tempfile
import time
from collections import Counter

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_drive import Report, Session, binary_from_argv  # noqa: E402

# The two frames: a solid red and a solid blue, as far apart as two colours can be so that no
# amount of resampling can make one look like the other.
COLOURS = [(0xE0, 0x20, 0x20), (0x20, 0x40, 0xE0)]
# Twenty hundredths of a second, which is what the file asks for and what the tab has to honour.
DELAY_CS = 20
SIZE = 32


def lzw_frame(index, pixels):
    """The pixel data of a solid frame, LZW-coded the way a GIF wants it.

    Compression proper is not needed and not attempted: emitting a clear code every two literals
    keeps the code table from ever growing past three bits, so every code is the same width and
    the packer stays five lines long. The decoder on the other side is a real one — that is the
    point — and this is the form of the stream it is happiest with."""
    minimum = 2                     # the smallest a GIF code size may be
    clear, end = 1 << minimum, (1 << minimum) + 1
    width = minimum + 1
    out, acc, bits = bytearray(), 0, 0

    def emit(code):
        nonlocal acc, bits
        acc |= code << bits
        bits += width
        while bits >= 8:
            out.append(acc & 0xFF)
            acc >>= 8
            bits -= 8

    emit(clear)
    since = 0
    for _ in range(pixels):
        emit(index)
        since += 1
        if since == 2:              # before the table can need a fourth bit
            emit(clear)
            since = 0
    emit(end)
    if bits:
        out.append(acc & 0xFF)

    # The stream goes out in sub-blocks of at most 255 bytes, terminated by an empty one.
    blocks = bytearray([minimum])
    for at in range(0, len(out), 255):
        chunk = out[at:at + 255]
        blocks.append(len(chunk))
        blocks.extend(chunk)
    blocks.append(0)
    return bytes(blocks)


def write_gif(path, size=SIZE, delay_cs=DELAY_CS):
    """A GIF89a of one solid frame per colour, each shown for `delay_cs` hundredths."""
    def word(n):
        return bytes((n & 0xFF, (n >> 8) & 0xFF))

    data = bytearray(b"GIF89a")
    # Logical screen: global colour table present, two entries (2 ** (0 + 1)).
    data += word(size) + word(size) + bytes((0xF0, 0x00, 0x00))
    for red, green, blue in COLOURS:
        data += bytes((red, green, blue))
    # Loop for ever, which is what every animated GIF in the world carries.
    data += b"\x21\xFF\x0BNETSCAPE2.0\x03\x01\x00\x00\x00"
    for index in range(len(COLOURS)):
        # Graphic control: no disposal, no transparency, this many hundredths on screen.
        data += b"\x21\xF9\x04\x00" + word(delay_cs) + b"\x00\x00"
        # Image descriptor: the whole canvas, no local colour table.
        data += b"\x2C" + word(0) + word(0) + word(size) + word(size) + b"\x00"
        data += lzw_frame(index, size * size)
    data += b"\x3B"
    with open(path, "wb") as handle:
        handle.write(bytes(data))


NOTE = """riga uno
riga due
riga tre
"""


# The two frames as they reach the screen, in the form pyte reports a 24-bit colour.
FRAME_COLOURS = {"%02x%02x%02x" % colour for colour in COLOURS}

# Fewer painted cells than this is not a picture. The pane holds some four hundred of them.
ENOUGH = 50


def picture_cells(session):
    """The cells the picture is painted in.

    Colour, not characters. A solid frame reaches the screen as coloured *spaces*: the
    half-block renderer collapses a cell whose two halves are the same colour into a space with
    that colour behind it, and only a cell straddling an edge in the picture keeps its `▀`. So
    a check reading the text would watch a red picture become a blue one and call them
    identical — which is exactly what the first version of this driver did."""
    return [
        cell
        for row in range(session.rows)
        for cell in session.cells(row)
        if cell.fg == cell.bg and cell.fg in FRAME_COLOURS
    ]


def frame_colour(session):
    """Which of the two frames is on screen, or None when no picture is."""
    cells = picture_cells(session)
    if len(cells) < ENOUGH:
        return None
    return Counter(cell.fg for cell in cells).most_common(1)[0][0]


def loading_showing(session):
    """Whether the word a tab shows while it decodes is on screen, in either language."""
    text = session.text()
    return "Loading the picture" in text or "Carico l'immagine" in text


def sample(session, seconds, every=0.04):
    """What the picture looked like over `seconds`, sampled without touching a key.

    Returns the colours seen in order, and whatever was caught blinking: a `Loading` word, or a
    moment with no picture at all. Both are the flicker this feature is built not to have."""
    colours, blinks = [], []
    deadline = time.time() + seconds
    while time.time() < deadline:
        session.drain()
        if loading_showing(session):
            blinks.append("Loading")
        colour = frame_colour(session)
        if colour is None:
            blinks.append("empty pane")
        else:
            colours.append(colour)
        time.sleep(every)
    return colours, blinks


def runs(colours):
    """The colours in order with repeats collapsed: what actually changed, and when."""
    out = []
    for colour in colours:
        if not out or out[-1] != colour:
            out.append(colour)
    return out


def main():
    binary = binary_from_argv(sys.argv)
    root = tempfile.mkdtemp(prefix="clee_gif_")
    write_gif(os.path.join(root, "moto.gif"))
    with open(os.path.join(root, "nota.txt"), "w") as handle:
        handle.write(NOTE)

    report = Report()
    session = Session(binary, root)
    try:
        if not session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            report.check("the editor starts", False, session)
            return 1
        session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 10)

        # A text file first, so there is somewhere for the keyboard to be that is not the
        # picture — and then the split, so both are on screen at once.
        session.send("\x0f")                              # Ctrl+O, quick-open
        session.wait(lambda s: "nota.txt" in s.text(), 8)
        session.send("nota")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        if not session.wait(lambda s: "riga uno" in s.text(), 8):
            report.check("the text file opens", False, session)
            return 1
        session.send("\x0c")                              # Ctrl+L, split the editor
        session.wait(lambda s: True, 1)

        session.send("\x0f")                              # Ctrl+O, quick-open
        session.wait(lambda s: "moto.gif" in s.text(), 8)
        session.send("moto")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        opened = session.wait(lambda s: frame_colour(s) is not None, 10)
        report.check("the gif opens as a picture tab and is drawn", opened, session,
                     note=f"{len(picture_cells(session))} cells of picture")
        if not opened:
            return 1

        # Nothing is pressed from here on. Whatever changes, changes by itself.
        colours, blinks = sample(session, 2.4)
        changes = runs(colours)
        report.check("the picture changes on its own, with no key pressed",
                     len(changes) > 1, session,
                     note=f"{len(changes)} frames seen in {len(colours)} samples: "
                          f"{changes[:6]}")
        report.check("and it keeps cycling rather than stopping on the last frame",
                     len(changes) > 2 and changes[0] in changes[2:], session,
                     note=f"{changes[0]} comes round again")
        # Ten changes in two and a bit seconds is the file's own 200 ms. A fixed tick on the
        # event loop would be sixty-odd, and a timer that ignored the file entirely could be
        # anything; both fail this and only this.
        report.check("the file's own delay is what it is played at, not a fixed tick",
                     4 <= len(changes) <= 24, session,
                     note=f"{len(changes)} changes in 2.4s, {DELAY_CS * 10} ms a frame asks for "
                          f"about {int(2400 / (DELAY_CS * 10))}")
        report.check("nothing blinks between two frames: no Loading, no empty pane",
                     not blinks, session, note=f"{len(blinks)} blinks: {blinks[:3]}")

        # The other half of the promise: a picture that moves must not move the keyboard.
        # Nothing is pressed to get back to the text, because nothing took the keyboard away
        # from it — the preview opened in the half that was not being worked in, and the
        # animation has been running in it ever since. Typing is the proof.
        before = session.buffer_line(1)
        typed = session.press("Z", lambda s: s.buffer_line(1) != before, 5)
        report.check("the keyboard stays in the file you were writing",
                     typed, session, note=f"{before!r} became {session.buffer_line(1)!r}")
        after, _ = sample(session, 1.2)
        report.check("and the picture beside it goes on moving while you type",
                     len(runs(after)) > 1, session, note=f"{len(runs(after))} frames while typing")

        session.press("\x1a", lambda s: s.buffer_line(1) == before, 4)   # undo the probe
    finally:
        session.close()
        shutil.rmtree(root, ignore_errors=True)
    return report.finish()


if __name__ == "__main__":
    sys.exit(main())
