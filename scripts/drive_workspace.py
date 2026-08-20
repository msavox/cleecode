#!/usr/bin/env python3
"""Watch the workspace window fill in by itself, against a real interpreter.

    python3 scripts/drive_workspace.py [path/to/clee]

The claim being checked is the one that makes this worth having: you run a cell and your
variables appear, without typing anything to ask. Nothing short of a real Octave can show that —
the hook only fires at an interactive prompt, and what it reports is what the interpreter
actually holds.

Skips if octave is not installed rather than passing quietly.
"""

import os
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_drive import Report, Session, binary_from_argv  # noqa: E402

SCRIPT = """%% setup
first = 111;

%% the cell under test
b = magic(4);
nn = [1 NaN 5];
s = 'ciao';
"""


def workspace_pane(session):
    """The lines of the rightmost frame, which is where the workspace window is."""
    out = []
    for line in session.lines():
        if "││" in line:
            out.append(line[line.rfind("││") + 2:].rstrip("│ "))
    return out


def main():
    binary = binary_from_argv(sys.argv)
    if shutil.which("octave") is None:
        print("  SKIP  octave is not installed")
        return 0

    root = tempfile.mkdtemp(prefix="clee_workspace_")
    with open(os.path.join(root, "script.m"), "w") as handle:
        handle.write(SCRIPT)

    report = Report()
    session = Session(binary, root, args=["-w", "octave"], cols=190, rows=34)
    try:
        if not session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            report.check("the preset opens", False, session)
            return 1
        session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 8)

        report.check("the workspace window is there before anything has run",
                     session.wait(lambda s: "workspace" in s.text(), 10), session)
        at_prompt = session.wait(lambda s: ">>" in s.text(), 40)
        report.check("octave reaches its prompt", at_prompt, session)
        if not at_prompt:
            return 1

        # The hook installs itself from the preset's own startup command. Nothing was written to
        # the user's home directory to make this happen.
        report.check("the view says which session it is watching",
                     session.wait(lambda s: "octave ·" in s.text(), 20), session,
                     note="named by the snapshot's own lang field")
        report.check("an empty workspace says so rather than looking broken",
                     "empty" in session.text(), session)

        # Open the script and send its second cell.
        session.send("\x0f")
        session.wait(lambda s: "script.m" in s.text(), 8)
        session.send("script")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        report.check("the script opens", session.wait(lambda s: "magic" in s.text(), 8), session)
        session.press("\x1b[B" * 4, lambda s: True, 1)
        session.press(session.chord("x"),
                      lambda s: "Cell" in s.lines()[-1] or "Cella" in s.lines()[-1], 8)

        filled = session.wait(lambda s: "3 variables" in s.text(), 25)
        report.check("the variables appear without anything being typed to ask", filled, session,
                     note="nothing was sent to the prompt but the cell itself")
        pane = "\n".join(workspace_pane(session))
        report.check("the cell's variables are the ones listed",
                     all(name in pane for name in ("b", "nn", "s")), session, note=repr(pane[-200:]))
        report.check("and the other cell's are not",
                     "first" not in pane, session, note="only the cell ran, not the file")

        rows = {line.split()[0]: line for line in workspace_pane(session) if line[:1].isalpha()}
        report.check("a matrix is summarised, not just named",
                     "4x4" in rows.get("b", "") and "16" in rows.get("b", ""),
                     session, note=repr(rows.get("b")))
        report.check("a NaN is reported where it can be seen",
                     "NaN" in rows.get("nn", ""), session, note=repr(rows.get("nn")))
        report.check("a statistic that makes no sense shows as a dash, not a zero",
                     "-" in rows.get("s", "") and " 0 " not in rows.get("s", ""),
                     session, note=repr(rows.get("s")))
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
