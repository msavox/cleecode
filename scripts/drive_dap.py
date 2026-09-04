#!/usr/bin/env python3
"""Debug a real compiled program through a real debug adapter, from the editor's own keyboard.

    python3 scripts/drive_dap.py [path/to/clee]

The compiled debugger's twin of drive_debug.py, and the same claim about a different backend:
there, an Octave session stops at a breakpoint the editor set; here, a C program compiled on this
machine stops under `lldb-dap` or a `gdb` new enough to speak DAP. Nothing is stubbed. The
adapter is the one the machine has, found the way src/dap.rs finds it, and the program it stops
is one this script compiles with `cc -g -O0` a moment before.

C, and not Rust, for two reasons that are both about the fixture being uninteresting: every
machine with a debug adapter on it has a C compiler, and the DWARF a `-O0` C file produces is the
least surprising thing a debugger can be asked to read — so a failure here is a failure of the
editor rather than of the fixture.

What is checked is the loop a person actually performs: a breakpoint set in the gutter, a session
started from the palette with the guess already in the box, a stop that marks the line and fills
the panel, a value that could only have come from the stopped frame, a watch, a step, a continue,
and an ending that takes the panel and the mark away with it. Skips with a sentence where there
is no adapter or no compiler, rather than passing quietly.
"""

import os
import re
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_drive import Report, Session, binary_from_argv  # noqa: E402

# The fixture. Short lines on purpose: the editor's pane is one frame of several on a 110-column
# window, and a check that looks for a line of source has to look for something that is drawn
# whole rather than clipped at the pane's edge.
#
# `twin` is what makes the stop legible from outside. At the marked line it has already been
# assigned and `sum` has not, so a panel showing `twin = 2` on the first call is a panel reading
# this frame's memory — not a guess, not the previous line, and not another frame.
SOURCE = """#include <stdio.h>

int twice(int n)
{
  int twin = n * 2;
  int sum = twin + 10;
  return sum;
}

int main(void)
{
  int total = 0;
  for (int round = 1; round <= 3; round++) {
    total += twice(round);
  }
  printf("total %d\\n", total);
  return 0;
}
"""

# The line the breakpoint goes on, 1-based, found in the text rather than counted by hand: a
# fixture edited one line longer would otherwise put the breakpoint somewhere else and the failure
# would be about the driver rather than about the editor.
STOP_LINE = SOURCE.splitlines().index("  int sum = twin + 10;") + 1
STOP_TEXT = "int sum = twin + 10;"
NEXT_TEXT = "return sum;"

# The gutter number in front of a line of source, read off the screen. The stopped line is
# identified by its own text; this says which line the editor thinks that is. The trailing
# whitespace is the gutter's own space plus whatever the source is indented by, so it is matched
# loosely rather than counted.
GUTTER = re.compile(r"(\d+)\s*$")


def gdb_speaks_dap(program):
    """Whether this gdb has DAP built in, which is to say whether it is 14 or newer.

    The same reading src/dap.rs does, and deliberately the same: a driver that accepted a gdb the
    editor would refuse — or refused one it accepts — would be testing a different machine from
    the one the user has. The major version is the last word of the banner's first line that
    starts with a digit, because distributions push their own packaging in front of it."""
    try:
        banner = subprocess.run(
            [program, "--version"], capture_output=True, text=True, timeout=30
        ).stdout
    except (OSError, subprocess.SubprocessError):
        return False
    first = banner.splitlines()[0] if banner else ""
    for word in reversed(first.split()):
        found = re.match(r"(\d+)", re.sub(r"^[^0-9A-Za-z]+", "", word))
        if found:
            return int(found.group(1)) >= 14
    return False


def find_adapter():
    """The debug adapter this machine has, in the order src/dap.rs looks for it.

    `lldb-dap` on PATH first; then, on a Mac, whatever `xcrun` names — it ships with the Xcode
    command-line tools and is on nobody's PATH — checked against the filesystem before it is
    believed, because `xcrun` prints paths for tools it merely expects to be there; then a gdb new
    enough to have DAP built in."""
    found = shutil.which("lldb-dap")
    if found:
        return found
    if sys.platform == "darwin":
        try:
            asked = subprocess.run(
                ["xcrun", "-f", "lldb-dap"], capture_output=True, text=True, timeout=60
            )
        except (OSError, subprocess.SubprocessError):
            asked = None
        if asked is not None and asked.returncode == 0:
            path = asked.stdout.strip()
            if path and os.path.isfile(path):
                return path
    gdb = shutil.which("gdb")
    if gdb and gdb_speaks_dap(gdb):
        return gdb
    return None


def gutter_digit(session, row, text):
    """The column of the last digit of the line number in front of `text`, or None.

    Found by walking left from the source rather than counted from the pane's edge: the gutter is
    as wide as the file's longest line number, and a check that assumed a width would be checking
    the arithmetic of the fixture."""
    drawn = session.full_line(row)
    at = drawn.index(text)
    for x in range(at - 1, -1, -1):
        if drawn[x].isdigit():
            return x
    return None


def panel(session):
    """The debug panel's own lines, borders excluded.

    Walked out from a word that is only ever drawn inside it, rather than sliced off the right of
    the screen: the panel is the rightmost frame on most rows and not on all of them, and a check
    that some value is *absent* reads a wrong frame as a pass for free."""
    return session.frame_of("Frames")


def panel_text(session):
    return "\n".join(panel(session))


def stopped_at(session, text):
    """Which line the editor is marking as the one the program stopped on, when the mark is on the
    line carrying `text` — and `None` when that line is not marked at all.

    Both halves are read off the drawing, because the drawing is the promise. The stopped line is
    the one line in the editor painted on a ground of its own, so the colour under the source says
    the program is there; the gutter number in front of it says which line that is. Asking the
    status line instead would prove only that a sentence was written."""
    row = session.row_of(text)
    if row is None or row == 0:
        return None
    drawn = session.full_line(row)
    at = drawn.index(text)
    # Against the same column one row up rather than against "no colour at all": a theme is
    # entitled to paint the editor's own background, and then every cell has one.
    if session.cells(row)[at].bg == session.cells(row - 1)[at].bg:
        return None
    number = GUTTER.search(drawn[:at])
    return int(number.group(1)) if number else None


def panel_has_the_keyboard(session):
    """Whether the debug panel is the focused frame.

    Asked of the drawing rather than of a status line, because that is where the answer lives: the
    panel lights the row its arrows are on only while it has the keyboard, which is exactly when
    its single letters mean anything. Compared against another of its own headings so that a theme
    painting the panel's background cannot read as a lit row."""
    here, there = session.row_of("Frames"), session.row_of("Variables")
    if here is None or there is None:
        return False
    lit = session.cells(here)[session.column_of("Frames")].bg
    plain = session.cells(there)[session.column_of("Variables")].bg
    return lit != plain


def focus_the_panel(session):
    """Ctrl+Tab until the panel has the keyboard.

    The frames cycle left to right and the panel is the last of them, so how many presses this
    takes depends on what had the keyboard when the session started. Pressed until the condition
    holds rather than a fixed number of times, which is the same rule every wait here follows."""
    for _ in range(6):
        if panel_has_the_keyboard(session):
            return True
        # Ctrl+Tab in the encoding that can carry it: 0x09 is the same byte as a plain Tab in
        # everything terminals have sent since VT100, so the disambiguating protocol is the only
        # way to say which of the two was pressed. pty_drive answers the query that turns it on.
        session.press("\x1b[9;5u", lambda s: True, 1.0)
    return panel_has_the_keyboard(session)


def open_in_editor(session, name, marker):
    """Ctrl+O, the file's name, Enter."""
    session.send("\x0f")
    session.wait(lambda s: name in s.text(), 8)
    session.send(name)
    session.wait(lambda s: True, 0.6)
    session.send("\r")
    return session.wait(lambda s: marker in s.text(), 10)


def through_the_palette(session, entry, settled):
    """Ctrl+P, the row's own words, Enter.

    The Debug menu's rows have no chords and that is the design's decision, not an omission — so
    the palette is how a driver reaches them, exactly as a user would."""
    session.send("\x10")
    session.wait(lambda s: "Command palette" in s.text(), 8)
    session.send(entry)
    session.wait(lambda s: True, 0.6)
    session.send("\r")
    return session.wait(settled, 20)


def main():
    binary = binary_from_argv(sys.argv)
    adapter = find_adapter()
    if adapter is None:
        print("  SKIP  no debug adapter on this machine — install lldb-dap, or a gdb 14 or newer")
        return 0
    if shutil.which("cc") is None:
        print("  SKIP  no cc to build the fixture with")
        return 0
    print(f"adapter {adapter}")

    # The project root is left exactly as `mkdtemp` spells it, symlinks and all, because that is
    # the shape of the bug this driver caught the first time it ran: on a Mac every temporary
    # directory is reached through one, and an adapter told a source path with a symlink still in
    # it accepts the breakpoint, leaves it unverified, and never stops. Resolving it here would
    # make the driver pass by avoiding the case every user of `/tmp` is in.
    root = tempfile.mkdtemp(prefix="clee_dap_")
    with open(os.path.join(root, "twice.c"), "w") as handle:
        handle.write(SOURCE)
    # -O0 and -g, and both are the point: optimised code has no line to stop on where the source
    # says one is, and without symbols there is nothing for a breakpoint to be about.
    built = subprocess.run(
        ["cc", "-g", "-O0", "twice.c", "-o", "prog"], cwd=root, capture_output=True, text=True
    )
    if built.returncode != 0:
        shutil.rmtree(root, ignore_errors=True)
        print("  FAIL  the fixture would not compile:\n" + built.stderr.strip())
        return 1

    # Without a language server, and that is about the status line rather than about clangd. A C
    # file opened on a machine that has clangd starts it, and it announces itself in the same one
    # row of the screen the debugger says why it stopped in — a race between two features that have
    # nothing to do with each other. The settings file goes in the throwaway config directory
    # `Session` is about to point the editor at.
    config = os.path.join(root, ".config", "cleecode")
    os.makedirs(config, exist_ok=True)
    with open(os.path.join(config, "settings.toml"), "w") as handle:
        handle.write("language_server = false\n")

    report = Report()
    # The window the drivers get by default. Narrower than drive_debug's on purpose: past 120
    # columns the editor splits itself in two to show a file beside your work, and two panes of
    # thirty columns each would clip the very lines these checks read.
    session = Session(binary, root, args=[root])
    try:
        if not session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            report.check("the editor draws its first frame", False, session)
            return 1

        report.check("the fixture opens", open_in_editor(session, "twice.c", STOP_TEXT), session)

        # Down to the line inside the helper, and a breakpoint on it.
        session.press("\x1b[B" * (STOP_LINE - 1), lambda s: True, 1.5)
        set_it = session.press(
            session.chord("p"), lambda s: f"twice.c:{STOP_LINE}" in s.lines()[-1], 8
        )
        report.check("a breakpoint is set from the editor", set_it, session,
                     note=repr(session.lines()[-1][:60]))
        row = session.row_of(STOP_TEXT)
        digit = gutter_digit(session, row, STOP_TEXT) if row is not None else None
        report.check("and it lights the gutter, not only the status line",
                     digit is not None
                     and session.cells(row)[digit].bg != session.cells(row - 1)[digit].bg,
                     session, note="the line number is drawn on the breakpoint's own ground")

        # Debug ▸ Start debugging, from the palette.
        asked = through_the_palette(session, "Start debugging",
                                    lambda s: "Program to debug" in s.text())
        report.check("Debug ▸ Start debugging asks what to run", asked, session)
        # Whitespace dropped before looking: the box is sixty columns wide and a path longer than
        # that is wrapped onto the next row, which is the box doing its job rather than a
        # different answer.
        offered = "".join(session.frame_of("Program to debug")).replace(" ", "")
        report.check("with the guess already in the box", root in offered, session,
                     note="the project root, which is this editor's honest \"I do not know\"")

        # Which is one word away from the binary beside it.
        session.send("/prog")
        session.wait(lambda s: True, 0.6)
        started = session.press("\r", lambda s: "prog" in s.lines()[-1]
                                and "ebug" in s.lines()[-1], 20)
        report.check("and the answer starts a session, named in the status line", started, session,
                     note=repr(session.lines()[-1][:70]))

        # From here the program runs itself into the breakpoint.
        stopped = session.wait(lambda s: stopped_at(s, STOP_TEXT) == STOP_LINE, 60)
        report.check("the program runs to the breakpoint, and the editor marks the line",
                     stopped, session, note=f"twice.c:{STOP_LINE}, marked in the editor")
        if not stopped:
            return report.finish()

        # The adapter's own word for why, which survives the editor following the program into the
        # file: showing a file announces itself, and the announcement must not land on top of the
        # one sentence the reader cannot work out by looking.
        report.check("and the status line says why it stopped",
                     "Stopped" in session.lines()[-1], session,
                     note=repr(session.lines()[-1][:50]))

        frames = panel(session)
        innermost = next((i for i, l in enumerate(frames) if "twice" in l), None)
        outer = next((i for i, l in enumerate(frames) if "main" in l), None)
        report.check("the panel opens on the stack, the helper innermost",
                     innermost is not None and outer is not None and innermost < outer, session,
                     note=repr([l.strip() for l in frames if l.strip()][:4]))

        report.check("and on the frame's own variables, with the value the fixture makes knowable",
                     session.wait(lambda s: "twin = 2" in panel_text(s), 20), session,
                     note="twin is n*2 on the first call, and it is assigned by this line")

        # The one verb in the menu that is about a program which is *running*, asked of one that is
        # not. Its refusal is how a driver reaches it at all: pausing for real needs a program with
        # somewhere to be caught, and this fixture's whole life is three calls long.
        report.check("Pause, at a program already stopped, says which key was meant instead",
                     through_the_palette(session, "Pause",
                                         lambda s: "already stopped" in s.lines()[-1]),
                     session, note=repr(session.lines()[-1][:60]))

        report.check("the panel takes the keyboard", focus_the_panel(session), session)

        # w, and one expression at the debuggee's own prompt.
        session.press("w", lambda s: "Expression to watch" in s.text(), 8)
        session.send("n")
        session.wait(lambda s: True, 0.6)
        session.press("\r", lambda s: True, 1.0)
        report.check("a watch answers with the other local's value",
                     session.wait(lambda s: "n = 1" in panel_text(s), 20), session,
                     note="n is the argument of the call the program is inside")

        # n, one line down.
        session.press("n", lambda s: True, 1.0)
        report.check("n steps, and the mark moves one line with it",
                     session.wait(lambda s: stopped_at(s, NEXT_TEXT) == STOP_LINE + 1, 30),
                     session, note=f"twice.c:{STOP_LINE + 1}")

        # c, on to the next call of the same helper.
        session.press("c", lambda s: True, 1.0)
        report.check("c runs on, and stops at the same breakpoint one call later",
                     session.wait(lambda s: stopped_at(s, STOP_TEXT) == STOP_LINE
                                  and "twin = 4" in panel_text(s), 30),
                     session, note="twin is 4 on the second round of the loop")

        # x, and the session is over.
        session.press("x", lambda s: True, 1.0)
        ended = session.wait(lambda s: "topped debugging" in s.lines()[-1], 20)
        report.check("x ends the session, and the status line says so", ended, session,
                     note=repr(session.lines()[-1][:60]))
        report.check("the panel goes with it, and so does the mark",
                     session.wait(lambda s: s.row_of("Frames") is None
                                  and stopped_at(s, STOP_TEXT) is None, 15), session)

        Report.show("final screen", session)

        # The exit status of a quit the user asked for is part of what they see: started from the
        # Dock, the window closes itself only if the process inside it ended like a program that
        # was asked to stop. A debug session in the same run must not change that.
        status = session.quit()
        report.check("and quitting is still a plain exit 0",
                     status is not None and os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0,
                     None, note=Session.describe_status(status))
    finally:
        session.close()
        shutil.rmtree(root, ignore_errors=True)

    return report.finish()


if __name__ == "__main__":
    try:
        sys.exit(main())
    except BrokenPipeError:
        os._exit(0)
