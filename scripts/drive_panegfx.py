#!/usr/bin/env python3
"""What a program in a terminal pane draws, and where it lands.

    python3 scripts/drive_panegfx.py [path/to/clee]

Two things are checked here, and they were two separate bugs.

The first is positioning. `vt100` implements CUP (`ESC [ y ; x H`) and not HVP
(`ESC [ y ; x f`), which the standard says is the same instruction — so a program that uses
only the second, as `btop` and `mpv` both do, had every absolute move it made dropped and drew
its whole screen as one wrapped stream. The check writes two marks at two positions with HVP
and asks whether they are where they were put.

The second is pictures. A pane now reads the kitty graphics protocol out of its own output and
draws the picture itself. In a bare pty CleeCode's own graphics query goes unanswered, so it
renders as half-blocks — the same fallback a terminal without kitty gets, which makes this a
real path rather than a testing artefact, and makes the picture visible to pyte.

Skips the picture half if chafa is not installed, rather than passing quietly.
"""

import os
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_drive import Report, Session, binary_from_argv  # noqa: E402


def focus_terminal(session):
    """Ctrl+P, then the palette entry that moves the keyboard into a shell."""
    session.send("\x10")
    session.wait(lambda s: "matches" in s.text(), 6)
    session.send("focus term")
    session.wait(lambda s: True, 0.5)
    session.send("\r")
    return session.wait(lambda s: "sh-" in s.text() or "$" in s.text(), 6)


def run(session, line, predicate, timeout=10.0):
    session.send(line + "\r")
    return session.wait(predicate, timeout)


def picture_rows(session):
    """Rows drawn with the half-blocks a rendered picture is made of."""
    return [
        y
        for y, line in enumerate(session.lines())
        if line.count("▀") + line.count("▄") > 3
    ]


def write_test_image(path):
    """A 64x64 gradient, as a PPM — no encoder needed, and chafa reads it."""
    width = height = 64
    rows = bytearray()
    for y in range(height):
        for x in range(width):
            rows += bytes(((x * 4) % 256, (y * 4) % 256, ((x + y) * 2) % 256))
    with open(path, "wb") as f:
        f.write(b"P6\n%d %d\n255\n" % (width, height))
        f.write(bytes(rows))


def main(argv):
    binary = binary_from_argv(argv)
    report = Report()
    root = tempfile.mkdtemp(prefix="clee-panegfx-")
    write_test_image(os.path.join(root, "gradient.ppm"))

    # A bare pty has no terminal name to inherit, so one is planted: the check below is that a
    # pane does not pass it on.
    session = Session(binary, root, env={"TERM_PROGRAM": "ghostty"})
    try:
        session.wait(lambda s: "Project Files" in s.text(), 20)
        report.check("the keyboard reaches a shell", focus_terminal(session), session)

        # ---- positioning -------------------------------------------------------------
        #
        # Written bottom mark first and on purpose: if the moves are dropped, both land on
        # one line in the order they were written, and "BOTTOM" is then above nothing at all
        # while "TOP" sits to its right on the same row. So the check is not "are they on
        # screen" — they always were — but "did they go where they were sent".
        run(
            session,
            r"printf '\033[2J\033[7;24fBOTTOMMARK\033[3;6fTOPMARK\n'",
            lambda s: "TOPMARK" in s.text() and "BOTTOMMARK" in s.text(),
        )
        top = session.row_of("TOPMARK")
        bottom = session.row_of("BOTTOMMARK")
        report.check(
            "HVP moves the cursor: the two marks are on different rows",
            top is not None and bottom is not None and top < bottom,
            session,
            f"TOPMARK row {top}, BOTTOMMARK row {bottom}",
        )
        top_col = session.column_of("TOPMARK")
        bottom_col = session.column_of("BOTTOMMARK")
        report.check(
            "HVP moves the cursor sideways too",
            top_col is not None and bottom_col is not None and top_col < bottom_col,
            session,
            f"TOPMARK col {top_col}, BOTTOMMARK col {bottom_col}",
        )

        # ---- what a pane says it is --------------------------------------------------
        #
        # The host terminal's own markers are inherited by everything CleeCode starts, and
        # inside a pane every one of them is a lie: a program that reads `TERM_PROGRAM` and
        # finds a terminal with a graphics protocol will use one, write it into a pty that is
        # parsed into cells, and be neither seen nor told. Driven from a bare pty there is
        # nothing to inherit, so the marker is set on the session on purpose and the check is
        # whether it survived into the shell.
        run(session, "clear", lambda s: True, 3)
        run(
            session,
            'echo "PROGRAM[$TERM_PROGRAM]TERM[$TERM]CLEE[$CLEECODE]"',
            lambda s: "PROGRAM[" in s.text(),
        )
        text = session.text()
        report.check(
            "the host terminal's name does not reach the pane",
            "PROGRAM[]" in text,
            session,
            "TERM_PROGRAM should be empty inside a pane",
        )
        report.check(
            "the pane still says what it is",
            "TERM[xterm-256color]" in text and "CLEE[1]" in text,
            session,
        )

        # The size questions. The reply comes back as input, so it is read rather than typed
        # at the prompt — which is also what a program asking one does.
        run(session, "clear", lambda s: True, 3)
        run(
            session,
            r"printf '\033[18t'; read -t 2 -d t -r r; echo CELLS-${r#*[}",
            lambda s: "CELLS-" in s.text(),
            10,
        )
        answered = [line for line in session.lines() if "CELLS-8;" in line]
        report.check(
            "a pane answers how many rows and columns it has",
            bool(answered),
            session,
            (answered[0].strip() if answered else "no CSI 18t reply"),
        )

        # Whatever the reply left on the command line goes before anything else is typed: it
        # arrives as input, so an unread one is sitting at the prompt.
        session.send("\x15")
        session.wait(lambda s: True, 0.4)

        # ---- pictures ----------------------------------------------------------------
        if shutil.which("chafa") is None:
            print("skip: chafa not installed, the picture half is not checked")
        else:
            run(session, "clear", lambda s: True, 3)
            drew = run(
                session,
                "chafa -f kitty --size 16x5 gradient.ppm",
                lambda s: len(picture_rows(s)) >= 4,
                15,
            )
            report.check(
                "a kitty picture written by a program in a pane is drawn",
                drew,
                session,
                f"{len(picture_rows(session))} picture rows",
            )

            # And it goes when something is written over it, which is what a preview pane
            # scrolling its list does — the cells stop being empty, so the picture stops
            # being there.
            gone = run(
                session,
                "clear",
                lambda s: len(picture_rows(s)) == 0,
                10,
            )
            report.check(
                "the picture goes when the cells under it are written over",
                gone,
                session,
                f"{len(picture_rows(session))} picture rows left",
            )

            # Half-blocks still work, because they are only text and always did. Checked so a
            # regression in the graphics road cannot be mistaken for one here.
            run(session, "clear", lambda s: True, 3)
            fallback = run(
                session,
                "chafa -f symbols --size 16x5 gradient.ppm",
                lambda s: len(picture_rows(s)) >= 2 or "█" in s.text(),
                15,
            )
            report.check("the half-block fallback still draws", fallback, session)
    finally:
        session.close()
        shutil.rmtree(root, ignore_errors=True)
    return report.finish()


if __name__ == "__main__":
    sys.exit(main(sys.argv))
