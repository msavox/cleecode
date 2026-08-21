#!/usr/bin/env python3
"""Look inside a variable, against a real Octave.

    python3 scripts/drive_inspect.py [path/to/clee]

The workspace window says what a variable *is*. This is about what it *contains*, which is not in
the snapshot — a large matrix is millions of numbers and nobody wants those written to disk on
the chance somebody looks. So a screenful is asked for and answered through a file.

Asked through a file, not typed at the prompt. That matters twice: the user's transcript stays
theirs, and the answer does not depend on catching a line editor at the right moment — which it
demonstrably does not. Byte-identical writes to the same terminal were acted on the second time
and ignored the first, and no amount of waiting changed it. This checks the channel that has no
such problem.

Skips if octave is not installed rather than passing quietly.
"""

import os
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_drive import Report, Session, binary_from_argv  # noqa: E402

SCRIPT = """%% p
A = magic(6);
testo = 'ciao mondo';
"""


def panel(session):
    """The inspector's own rows, from its frame."""
    start = next((y for y, l in enumerate(session.lines()) if "┌ A " in l or "┌ testo " in l), None)
    if start is None:
        return []
    out = []
    for line in session.lines()[start:]:
        at = max(line.find("┌ A "), line.find("┌ testo "))
        out.append(line[at:] if at >= 0 else line[line.rfind("│", 0, 120) + 1:])
    return out


def main():
    binary = binary_from_argv(sys.argv)
    if shutil.which("octave") is None:
        print("  SKIP  octave is not installed")
        return 0

    root = tempfile.mkdtemp(prefix="clee_inspect_")
    with open(os.path.join(root, "m.m"), "w") as handle:
        handle.write(SCRIPT)

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
        session.wait(lambda s: "m.m" in s.text(), 8)
        session.send("m.m")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        session.wait(lambda s: "magic" in s.text(), 8)
        session.press(session.chord("x"),
                      lambda s: "Cell" in s.lines()[-1] or "Cella" in s.lines()[-1], 8)
        report.check("the session holds two variables",
                     session.wait(lambda s: "2 variables" in s.text(), 25), session)

        picked = session.press(session.chord("i"), lambda s: "Variables" in s.text(), 10)
        report.check("the variables are offered to pick from", picked, session)
        filled = session.press("\r", lambda s: "Asking" not in s.text() and "35" in s.text(), 20)
        report.check("the numbers arrive without anything being typed at the prompt",
                     filled, session, note="asked through a file the session's own hook reads")
        if not filled:
            return 1

        rows = panel(session)
        report.check("the panel says which variable and how big it is",
                     any("6x6" in r for r in rows), session, note=repr(rows[0][:40] if rows else ""))
        report.check("the values are the ones the session holds",
                     any("35" in r and "26" in r for r in rows), session,
                     note="the first row of magic(6)")
        report.check("rows and columns are numbered, so you know where you are",
                     any(r.strip().startswith("1 ") or r.strip().startswith("1  ") for r in rows),
                     session)

        # The user's prompt is untouched: nothing was typed there to produce any of this.
        prompt = "\n".join(
            l[l.rfind("││") + 2:] for l in session.lines() if "││" in l
        )
        report.check("nothing was typed at the user's prompt to ask",
                     "cleecode_slice" not in prompt, session,
                     note="the request went through a file")

        # Its own frame, not the whole screen. "6x6" is also in the workspace table, which
        # stays there — and used to not: the panel followed the newest .json in the snapshot
        # directory, so the slice file this inspector had just written became the newest thing
        # and blanked it. Watching the whole screen made a passing check out of that bug.
        report.check("Esc closes it",
                     session.press("\x1b", lambda s: not panel(s), 6), session,
                     note="and the workspace table is still there behind it")
        report.check("closing it leaves the workspace panel alone",
                     "6x6" in session.text(), session,
                     note="the inspector's question must not displace the session it asked")
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
