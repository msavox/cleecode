#!/usr/bin/env python3
"""Everything the Octave side does, done against a real Python.

    python3 scripts/drive_python.py [path/to/clee]

CleeCode's whole numeric side is one feature with two backends, and the only way to know the
second one is real is to drive it the same way. So this is deliberately the same checks as
drive_workspace, drive_inspect and drive_debug, in one pass: the variables appear by themselves,
a variable can be looked inside, and a breakpoint set in the editor stops the session and shows
the *frame's* variables rather than the module's.

None of it works the way Octave's does underneath, which is the point of checking rather than
assuming. Octave has one idle hook; Python needs two mechanisms glued together. Octave's
`dbstop` is applied from inside that hook; Python's breakpoints go through pdb, and tracing is
switched on for exactly the length of one statement so that typing at the prompt costs nothing.
Octave reports history from `history()`; PyREPL reports none at all through readline and keeps
its own.

Skips if python3 is not installed rather than passing quietly. Figures are checked only if
matplotlib is importable from the same python3 CleeCode will run — on a machine where it is not,
that check is reported as skipped rather than silently dropped.
"""

import os
import re
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_drive import Report, Session, binary_from_argv  # noqa: E402

MODULE = """def calcola(n):
    a = n * 2
    b = a + 10
    return b / 2
"""

SCRIPT = """# %% setup
first = 111

# %% the cell under test
import numpy as np
A = np.arange(36).reshape(6, 6)
testo = 'ciao mondo'
"""


def ws_pane(session, needle="python ·"):
    """The workspace window's own lines, joined — never the whole screen.

    The audit that forced this found three checks here matching the editor, the interpreter's
    startup banner and the terminal transcript: `a `, `n `, `20` and `np.arange` are all written
    in places that have nothing to do with the workspace. `frame_of` walks out to the borders of
    the frame the needle is drawn in, so a row matched here is a row of the pane."""
    return "\n".join(session.frame_of(needle))


def has_matplotlib():
    """Asked of the `python3` CleeCode will start, not of the one running this script.

    Those diverge — this driver runs under whatever python has `pyte`, while the session runs
    the `python3` on PATH — and asking the wrong one is how the figure check stayed SKIPped on a
    machine that could in fact have exercised it."""
    python3 = shutil.which("python3")
    if not python3:
        return False
    try:
        return subprocess.run([python3, "-c", "import matplotlib"],
                              capture_output=True, timeout=60).returncode == 0
    except Exception:
        return False


def to_terminal(session):
    """Put the keyboard in the terminal pane, through the palette."""
    for _ in range(4):
        session.send("\x10")
        if session.wait(lambda s: "matches" in s.text(), 2):
            session.send("focus term")
            session.wait(lambda s: True, 0.5)
            session.send("\r")
            return True
    return False


def to_editor(session, name):
    for _ in range(4):
        session.send("\x0f")
        if session.wait(lambda s: "matches" in s.text(), 3):
            session.send(name)
            session.wait(lambda s: True, 0.5)
            session.send("\r")
            return True
    return False


def main():
    binary = binary_from_argv(sys.argv)
    if shutil.which("python3") is None:
        print("  SKIP  python3 is not installed")
        return 0

    root = tempfile.mkdtemp(prefix="clee_python_")
    with open(os.path.join(root, "calcola.py"), "w") as handle:
        handle.write(MODULE)
    with open(os.path.join(root, "script.py"), "w") as handle:
        handle.write(SCRIPT)

    report = Report()
    session = Session(binary, root, args=["-w", "pylab"], cols=200, rows=40)
    try:
        if not session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            report.check("the pylab preset opens", False, session)
            return 1
        session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 8)
        at_prompt = session.wait(lambda s: ">>>" in s.text(), 40)
        report.check("python reaches its prompt", at_prompt, session)
        if not at_prompt:
            return 1
        report.check("the workspace window says which session it is watching",
                     session.wait(lambda s: "python ·" in s.text(), 20), session,
                     note="named by the snapshot's own lang field, same as Octave's")

        # --- the variables appear by themselves -------------------------------------------
        to_editor(session, "script.py")
        report.check("the script opens", session.wait(lambda s: "arange" in s.text(), 8), session)
        # Onto the second cell. It imports what it needs, because a cell is run on its own and
        # is not entitled to what the cell above it happened to leave behind.
        session.press("\x1b[B" * 4, lambda s: True, 1)
        session.press(session.chord("x"),
                      lambda s: "Cell" in s.lines()[-1] or "Cella" in s.lines()[-1], 8)
        filled = session.wait(lambda s: "2 variables" in s.text(), 30)
        report.check("running a cell fills the workspace with nothing typed to ask",
                     filled, session, note="the audit hook marks, the prompt collects")
        if not filled:
            return 1
        pane = session.text()
        report.check("a numpy array is summarised, not just named",
                     "6x6" in pane and "35" in pane, session,
                     note="shape and range, the same columns Octave's matrices get")

        # --- looking inside a variable ----------------------------------------------------
        picked = session.press(session.chord("i"), lambda s: "Variables" in s.text(), 10)
        report.check("the variables are offered to pick from", picked, session)
        # Waited for by a run of consecutive values, not by "35": the Max of arange(36) is 35
        # and it was already on screen in the workspace's own summary row — the audit printed the
        # predicate as true before Enter was pressed. `30 31 32` in a row exists only once the
        # matrix body itself has been drawn.
        filled = session.press(
            "\r", lambda s: "Asking" not in s.text() and re.search(r"30\s+31\s+32", s.text()), 25)
        report.check("the numbers arrive without anything being typed at the prompt",
                     filled, session, note="asked through the same file the Octave side uses")
        prompt = "\n".join(session.frame_of(">>>"))
        report.check("nothing was typed at the user's prompt to ask",
                     "slice" not in prompt.lower(), session, note=repr(prompt[-90:]))
        session.press("\x1b", lambda s: True, 4)

        # --- the debugger -----------------------------------------------------------------
        to_editor(session, "calcola.py")
        report.check("the module opens", session.wait(lambda s: "b = a + 10" in s.text(), 8),
                     session)
        session.press("\x1b[B" * 2, lambda s: True, 1)
        set_it = session.press(session.chord("p"), lambda s: "reakpoint" in s.lines()[-1], 6)
        report.check("a breakpoint is set from the editor", set_it, session,
                     note=repr(session.lines()[-1][:60]))

        to_terminal(session)
        session.wait(lambda s: True, 1.0)
        session.press("import calcola\r", lambda s: True, 3)
        # Recent, in the pane, right after typing — which is the only moment the newest entry is
        # known. The old spelling of this check looked for `np.arange` anywhere on screen and was
        # answered by the editor showing the script; it also asked at a moment when there was
        # nothing to list, because a cell run is CleeCode's own injection and the producer filters
        # those out. And the list is capped to the rows left under the variables, so anything but
        # the newest command is a check on how tall the pane happens to be.
        report.check("the command just typed is listed under Recent",
                     session.wait(lambda s: "import calcola" in ws_pane(s), 15), session,
                     note="the pane's own rows, and the newest entry — the list is room-capped")
        stopped = session.press("x = calcola.calcola(5)\r", lambda s: "(Pdb)" in s.text(), 25)
        report.check("the session stops at the breakpoint", stopped, session)
        if not stopped:
            return 1
        report.check("the editor says where it stopped",
                     session.wait(lambda s: "calcola" in s.lines()[-1] and "3" in s.lines()[-1], 15),
                     session, note=repr(session.lines()[-1][:70]))
        report.check("the workspace window says it is stopped",
                     session.wait(lambda s: "stopped in calcola" in s.text(), 15), session)
        # Anchored rows of the workspace pane, not substrings of the screen: `a ` and `n ` are
        # in the module source the editor is showing, and `1x1` was already on a workspace row
        # listing the module's variables — the audit printed this predicate as true before the
        # session had even stopped. As written before, a stop that failed to switch the pane to
        # frame variables would still have passed, which is the bug this check exists to catch.
        def frame_rows(s):
            pane = ws_pane(s, "stopped in calcola")
            return (re.search(r"^a\s+1x1", pane, re.M) and re.search(r"^n\s+1x1", pane, re.M))

        report.check("and shows the frame's own variables, not the module's",
                     session.wait(frame_rows, 15), session,
                     note="rows of the pane itself; A and testo are not in scope there")
        report.check("the stack stops at the statement, not in the REPL's machinery",
                     "runcode" not in session.text() and "runsource" not in session.text(),
                     session, note="a true account of how it got here, and no help finding a bug")

        # Stepping is typed at pdb's own prompt, which is where a Python user expects it.
        session.press("n\r", lambda s: True, 3)
        report.check("a step moves the editor with it",
                     session.wait(lambda s: "4" in s.lines()[-1] and "calcola" in s.lines()[-1], 15),
                     session, note=repr(session.lines()[-1][:70]))
        session.press("c\r", lambda s: True, 3)
        report.check("running on is noticed",
                     session.wait(lambda s: "Running again" in s.lines()[-1]
                                  or "Riparte" in s.lines()[-1], 20), session,
                     note=repr(session.lines()[-1][:50]))
        # An `x` row of the pane holding 10 — calcola(5) is (5*2+10)/2. The old spelling waited
        # for "x " and "20" anywhere on screen, and the audit found both already true: "x " in
        # the typed transcript, "20" inside "2026" in Python's own startup banner. It also had
        # the arithmetic wrong, which nothing noticed because nothing was being checked.
        def module_rows(s):
            pane = ws_pane(s)
            row = next((l for l in pane.splitlines() if re.match(r"^x\s+1x1", l)), None)
            return row is not None and "10" in row

        report.check("and the workspace goes back to the module's variables",
                     session.wait(module_rows, 20), session, note="x is 10.0, on the pane's own row")


        # --- figures, if this machine can draw them ---------------------------------------
        if not has_matplotlib():
            print("  SKIP  matplotlib is not importable from this python3 — figures unchecked")
        else:
            session.press("import matplotlib.pyplot as plt\r", lambda s: True, 10)
            session.press("fig, ax = plt.subplots(); _ = ax.plot([1,2,3],[1,4,9])\r",
                          lambda s: True, 6)
            # A tab with a picture in it, which is what "noticed" means here. The old spelling
            # waited for "1 figure" or "figure 1" — words the panel does not write anywhere —
            # so this could only ever have failed, and it never ran to find out: it asked
            # `sys.executable` for matplotlib, which is this script's interpreter rather than
            # the `python3` the session starts.
            drawn = session.wait(
                lambda s: "fig1.png" in s.text()
                and sum(1 for l in s.lines() if l.count("▀") + l.count("▄") > 3) > 2, 25)
            report.check("a figure is noticed without plt.show()", drawn, session,
                         note="a fig1.png tab, with a picture drawn in it")

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
