#!/usr/bin/env python3
"""Set a breakpoint from the editor and watch the session stop on it, against a real Octave.

    python3 scripts/drive_debug.py [path/to/clee]

What is worth checking here is where each half of the work happens. Setting a breakpoint leaves
no line in the user's transcript, because `dbstop` is applied by the session's own idle hook
rather than typed at the prompt — measured to work, unlike stepping, which returns without an
error and without moving when driven the same way. So `dbstep` and `dbcont` stay something the
user types at the `debug>` prompt, and the editor's job is to say where the program is and what
is in scope there.

That last part is the one that makes it a debugger rather than a breakpoint setter: while
stopped, the workspace window shows the *frame's* variables, not the base workspace's.

Skips if octave is not installed rather than passing quietly.
"""

import os
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_drive import Report, Session, binary_from_argv  # noqa: E402

FUNCTION = """function r = calcola (n)
  a = n * 2;
  b = a + 10;
  r = b / 2;
end
"""


def workspace_pane(session):
    """The whole screen. The phrases looked for below appear only in the workspace window, and
    hunting for that window's own border is brittle — the panes are arranged differently
    depending on whether the editor is split."""
    return session.text()


def main():
    binary = binary_from_argv(sys.argv)
    if shutil.which("octave") is None:
        print("  SKIP  octave is not installed")
        return 0

    root = tempfile.mkdtemp(prefix="clee_debug_")
    with open(os.path.join(root, "calcola.m"), "w") as handle:
        handle.write(FUNCTION)

    report = Report()
    session = Session(binary, root, args=["-w", "octave"], cols=200, rows=40)
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
        session.wait(lambda s: "calcola.m" in s.text(), 8)
        session.send("calcola")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        report.check("the function opens", session.wait(lambda s: "b = a + 10" in s.text(), 8),
                     session)

        # Onto line 3, and a breakpoint there.
        session.press("\x1b[B" * 2, lambda s: True, 1)
        set_it = session.press(session.chord("p"),
                               lambda s: "reakpoint" in s.lines()[-1], 6)
        report.check("a breakpoint is set from the editor", set_it, session,
                     note=repr(session.lines()[-1][:60]))
        report.check("nothing was typed at the prompt to set it",
                     "dbstop" not in workspace_pane(session), session,
                     note="the session's own hook applied it")

        # Call the function from the prompt. The session should stop.
        for _ in range(4):
            session.send("\x10")
            if session.wait(lambda s: "matches" in s.text(), 2):
                session.send("focus term")
                session.wait(lambda s: True, 0.5)
                session.send("\r")
                break
        session.wait(lambda s: True, 1.0)
        stopped = session.press("x = calcola(5)\r", lambda s: "debug>" in s.text(), 20)
        report.check("the session stops at the breakpoint", stopped, session)
        if not stopped:
            return 1

        report.check("the editor says where it stopped",
                     session.wait(lambda s: "calcola" in s.lines()[-1] and "3" in s.lines()[-1], 15),
                     session, note=repr(session.lines()[-1][:70]))

        report.check("the workspace window says it is stopped",
                     session.wait(lambda s: "stopped in calcola" in s.text(), 15), session)
        pane = workspace_pane(session)
        report.check("and shows the frame's own variables, not the base workspace's",
                     "a    1x1" in pane and "n    1x1" in pane, session,
                     note="a and n are locals of calcola, not of the base workspace")

        # Stepping is typed at the prompt, by design: driven from the hook it does nothing.
        session.press("dbstep\r", lambda s: True, 3)
        report.check("a step moves the editor with it",
                     session.wait(lambda s: "4" in s.lines()[-1] and "calcola" in s.lines()[-1], 15),
                     session, note=repr(session.lines()[-1][:70]))

        # Not by looking for "stopped in" to disappear: Octave printed that line itself when it
        # stopped, and it stays in the transcript for good. What changes is CleeCode's own
        # status row, and the workspace going back to the base variables.
        session.press("dbcont\r", lambda s: True, 3)
        report.check("running on is noticed",
                     session.wait(lambda s: "Running again" in s.lines()[-1]
                                  or "Riparte" in s.lines()[-1], 20),
                     session, note=repr(session.lines()[-1][:50]))
        report.check("and the workspace goes back to the base variables",
                     session.wait(lambda s: "x    1x1" in s.text(), 20), session,
                     note="x is what calcola returned, and it lives in the base workspace")

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
