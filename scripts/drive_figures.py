#!/usr/bin/env python3
"""Plot from a cell and watch the figure arrive as a tab, against a real Octave.

    python3 scripts/drive_figures.py [path/to/clee]

A live Qt figure window cannot be reparented into a terminal, so a plot reaches CleeCode as a
picture instead: the interpreter is told to open no window, prints each figure it has touched to
a PNG at the end of a command, and the editor opens that as a preview tab.

What is worth checking is not that a file appeared — that is the easy half — but that the tab
tracks the session: re-plotting has to change what is on screen, and a figure must not steal the
keyboard from the script being written.

The pictures themselves render as half-blocks here, because a bare pty answers no graphics
query. That is the same fallback a terminal without kitty or sixel gets, so it is a real path
rather than a testing artefact.

Skips if octave is not installed rather than passing quietly.
"""

import os
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_drive import Report, Session, binary_from_argv  # noqa: E402

SCRIPT = """%% first plot
x = 1:100;
figure(1); plot(x, sin(x/10)); title('seno'); grid on;

%% a second figure
figure(2); plot(x, x.^2); title('quadrato');
"""


def picture_rows(session):
    """Rows drawn with the half-block characters a rendered picture is made of."""
    return sum(1 for line in session.lines() if line.count("▀") + line.count("▄") > 3)


def main():
    binary = binary_from_argv(sys.argv)
    if shutil.which("octave") is None:
        print("  SKIP  octave is not installed")
        return 0

    root = tempfile.mkdtemp(prefix="clee_figures_")
    with open(os.path.join(root, "grafico.m"), "w") as handle:
        handle.write(SCRIPT)

    report = Report()
    session = Session(binary, root, args=["-w", "octave"], cols=190, rows=40)
    try:
        if not session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            report.check("the preset opens", False, session)
            return 1
        session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 8)
        if not session.wait(lambda s: ">>" in s.text(), 40):
            report.check("octave reaches its prompt", False, session)
            return 1

        session.send("\x0f")
        session.wait(lambda s: "grafico.m" in s.text(), 8)
        session.send("grafico")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        report.check("the script opens", session.wait(lambda s: "plot" in s.text(), 8), session)

        # The first cell, which plots. Nothing is asked for beyond running it.
        session.press(session.chord("x"),
                      lambda s: "Cell" in s.lines()[-1] or "Cella" in s.lines()[-1], 8)
        arrived = session.wait(lambda s: "fig1" in s.text(), 40)
        report.check("the figure arrives as a tab, with no window opening anywhere",
                     arrived, session)
        if not arrived:
            return 1
        report.check("and it is drawn, not just named", picture_rows(session) > 2, session,
                     note=f"{picture_rows(session)} rows of picture")
        report.check("the script it came from is still beside it",
                     "grafico.m" in session.text(), session,
                     note="split view opened for the pair")

        # The keyboard did not move: typing still goes into the script.
        before = session.buffer_line(1)
        session.press("%", lambda s: s.buffer_line(1) != before, 4)
        report.check("a figure does not take the keyboard from what you were writing",
                     session.buffer_line(1) != before, session,
                     note=f"{before!r} became {session.buffer_line(1)!r}")
        session.press("\x1a", lambda s: s.buffer_line(1) == before, 4)   # undo the probe

        # A second figure is a second tab, not a replacement.
        session.press("\x1b[B" * 4, lambda s: True, 1)
        session.press(session.chord("x"),
                      lambda s: "Cell" in s.lines()[-1] or "Cella" in s.lines()[-1], 8)
        report.check("a second figure is a second tab",
                     session.wait(lambda s: "fig2" in s.text(), 40), session)

        Report.show("final screen", session)
    finally:
        session.close()
        shutil.rmtree(root, ignore_errors=True)

    return report.finish()


if __name__ == "__main__":
    try:
        sys.exit(main())
    except BrokenPipeError:
        os._exit(0)
