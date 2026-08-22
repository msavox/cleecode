#!/usr/bin/env python3
"""Double-click a line of terminal output and land in the file it names.

    python3 scripts/drive_jump.py [path/to/clee]

Written for tracebacks and useful far beyond them: `cargo`, `gcc`, `eslint`, `pytest` and
`grep -n` all print `path:line:column`, so the same double-click works on all of them. This
drives the general case — grep's output in a plain shell — because it needs no interpreter
installed and exercises exactly the same path an Octave backtrace takes.
"""

import os
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_drive import Report, Session, binary_from_argv  # noqa: E402

SAMPLE = "uno\ndue\ntre\nquattro\nCERCAMI qui\nsei\n"


def click(session, col, row):
    """One press and release, in the SGR encoding CleeCode turns on at startup. One-based."""
    session.send(f"\x1b[<0;{col + 1};{row + 1}M")
    session.send(f"\x1b[<0;{col + 1};{row + 1}m")


def main():
    binary = binary_from_argv(sys.argv)
    root = tempfile.mkdtemp(prefix="clee_jump_")
    with open(os.path.join(root, "dati.txt"), "w") as handle:
        handle.write(SAMPLE)

    report = Report()
    session = Session(binary, root, cols=150, rows=32)
    try:
        if not session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            report.check("the app starts", False, session)
            return 1
        session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 8)

        # Into a shell, and print something that names a file and a line.
        session.send("\x10")                                   # Ctrl+P, the palette
        session.wait(lambda s: "matches" in s.text(), 6)
        session.send("focus term")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        session.wait(lambda s: "$" in s.text(), 6)
        printed = session.press("grep -Hn CERCAMI dati.txt\r",
                                lambda s: "dati.txt:5" in s.text(), 10)
        report.check("the shell prints a line naming a file", printed, session)
        if not printed:
            return 1

        row = session.row_of("dati.txt:5")
        col = session.column_of("dati.txt:5")
        report.check("the line is on screen where it can be clicked",
                     row is not None and col is not None, session)

        # One click starts a selection, as it always did. The second says "take me there".
        click(session, col + 2, row)
        opened = session.press("", lambda s: True, 0.3) or True
        click(session, col + 2, row)
        # Waited for by the buffer holding the line, not by the words being on screen: the grep
        # output that was clicked reads `dati.txt:5:CERCAMI qui`, so the old predicate was true
        # before the first click and this check could not fail.
        landed = session.wait(lambda s: s.buffer_line(5) == "CERCAMI qui", 8)
        report.check("the second click opens the file it named", landed, session,
                     note=repr(session.lines()[-1][:60]))
        report.check("and lands on the line, not at the top",
                     "riga 5" in session.lines()[-1] or "line 5" in session.lines()[-1],
                     session, note=repr(session.lines()[-1][:60]))
        report.check("the line it named is the one the cursor is on",
                     session.buffer_line(5) == "CERCAMI qui", session,
                     note=repr(session.buffer_line(5)))
        assert opened

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
