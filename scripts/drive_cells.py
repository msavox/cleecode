#!/usr/bin/env python3
"""Send a cell from the editor to a real interpreter, and check it landed in that session.

    python3 scripts/drive_cells.py [path/to/clee]

The point of running a cell rather than the file is that the *session* keeps what the cell did.
Nothing short of a real interpreter can show that: a stub would prove a string reached a prompt,
which is the easy half. So this starts a genuine Octave and a genuine Python inside CleeCode's
own terminals, runs a cell at each, and then asks that session for the variable afterwards.

Skips a language whose interpreter is not installed, and says so rather than passing quietly.
"""

import os
import shutil
import shlex
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_drive import Report, Session, binary_from_argv  # noqa: E402

# Two cells each. The second one is the one that gets run, so a harness that accidentally ran
# the whole file would be caught by `first` being defined when it should not be.
OCTAVE = """%% setup
first = 111;

%% the cell under test
marker = 4242;
"""

PYTHON = """# %% setup
first = 111

# %% the cell under test
marker = 4242
"""

LANGUAGES = [
    {
        "name": "Octave",
        "file": "demo.m",
        "source": OCTAVE,
        "programs": ["octave"],
        "start": "octave --no-gui --quiet",
        "prompt": ">>",
        # Doubled, and the absence question answers in words. Both for the same reason: the
        # editor is showing `marker = 4242` and a bare `0` is somewhere on almost any screen, so
        # asking for either back would be answered by the screen rather than by the interpreter.
        "ask": "disp(marker * 2)",
        "says": "8484",
        "ask_absent": "printf(\"cell_only=%d\\n\", exist('first'))",
        "absent_says": "cell_only=0",
    },
    {
        "name": "Python",
        "file": "demo.py",
        "source": PYTHON,
        "programs": ["python3"],
        "start": "python3",
        "prompt": ">>>",
        "ask": "print(marker * 2)",
        "says": "8484",
        "ask_absent": "print('cell_only=%d' % ('first' in dir()))",
        "absent_says": "cell_only=0",
    },
]


def installed(program):
    return shutil.which(program) is not None


def focus_terminal(session):
    """Ctrl+P, then pick the palette entry that moves the keyboard into a shell."""
    session.send("\x10")
    session.wait(lambda s: "matches" in s.text(), 6)
    session.send("focus term")
    session.wait(lambda s: True, 0.5)
    session.send("\r")
    return session.wait(lambda s: "sh-" in s.text() or "$" in s.text(), 6)


def open_file(session, name):
    """Quick-open `name`, from wherever the keyboard happens to be.

    A focused terminal has first claim on every Ctrl chord — that is the bargain CleeCode
    strikes so vim and readline keep working inside it — so Ctrl+O typed at a shell goes to the
    shell. Ctrl+Tab is the one it holds back, and it is how you get out of the pane. Encoded as
    CSI u because plain 0x09 is indistinguishable from an ordinary Tab."""
    for _ in range(4):
        session.send("\x0f")                                   # Ctrl+O
        if session.wait(lambda s: "matches" in s.text(), 2):
            session.send(name)
            session.wait(lambda s: True, 0.5)
            session.send("\r")
            return True
        session.press("\x1b[9;5u", lambda s: True, 0.4)        # Ctrl+Tab, cycle the frames
    return False


def run_one(binary, spec, report):
    root = tempfile.mkdtemp(prefix="clee_cells_")
    path = os.path.join(root, spec["file"])
    with open(path, "w") as handle:
        handle.write(spec["source"])

    session = Session(binary, root)
    try:
        if not session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            report.check(f"{spec['name']}: the app starts", False, session)
            return
        session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 8)

        # An interpreter of its own, started the way a person would: by typing it.
        report.check(f"{spec['name']}: the keyboard reaches a shell", focus_terminal(session), session)
        session.send(spec["start"] + "\r")
        at_prompt = session.wait(lambda s: spec["prompt"] in s.text(), 30)
        report.check(f"{spec['name']}: the interpreter reaches its prompt", at_prompt, session,
                     note=spec["start"])
        if not at_prompt:
            return

        # Back to the editor, open the file, and put the cursor inside the *second* cell.
        open_file(session, os.path.splitext(spec["file"])[0])
        opened = session.wait(lambda s: "marker" in s.text(), 8)
        report.check(f"{spec['name']}: the file opens", opened, session)
        if not opened:
            return
        session.press("\x1b[B" * 4, lambda s: True, 1)         # down into the last cell

        # Ctrl+Shift+X. Only reachable because the harness answers the keyboard-protocol query.
        sent = session.press(session.chord("x"),
                             lambda s: "Cell" in s.lines()[-1] or "Cella" in s.lines()[-1], 8)
        report.check(f"{spec['name']}: the status row reports the cell it sent", sent, session,
                     note=repr(session.lines()[-1][:70]))

        # The real question: did that session keep it? Asked of the interpreter, and read back
        # out of the terminal's own frame — the editor is showing `marker = 4242` all along, so
        # a whole-screen search for that number is answered before the question is asked. The
        # interpreter is asked to double it for the same reason: 8484 is written nowhere.
        focus_terminal(session)
        prompt = spec["prompt"]
        session.send(spec["ask"] + "\r")
        landed = session.wait(lambda s: spec["says"] in "\n".join(s.frame_of(prompt)), 15)
        report.check(f"{spec['name']}: the variable exists in the live session", landed, session,
                     note=f"{spec['ask']} → {spec['says']}")

        # And only the cell ran, not the whole file — otherwise "run cell" is a fancy Run. The
        # answer is a labelled word rather than a bare `0`, which was on screen already and made
        # this pass whatever the interpreter said.
        session.send(spec["ask_absent"] + "\r")
        only_cell = session.wait(
            lambda s: spec["absent_says"] in "\n".join(s.frame_of(prompt)), 10)
        report.check(f"{spec['name']}: the other cell did not run", only_cell, session,
                     note=f"{spec['ask_absent']} should say {spec['absent_says']}")
    finally:
        session.close()
        shutil.rmtree(root, ignore_errors=True)


def run_lands_in_the_live_session(binary, report):
    """Press Run with a Python prompt open, and check the file ran *there*.

    Two things at once, and they were one bug.

    A shell command typed at a prompt that is not a shell's has now been written three times in
    three hats: `octave-cli-11.3.0` in 0.9.1, a capital `Python` in the 0.10 driver audit, and
    Run's own "if no shell is idle use the one you were last in" — which is precisely the
    interpreter you were working in. What the user saw was their own transcript growing a mistake
    with their name on it:

        >>> python3 /home/ada/hello.py
        NameError: name 'python3' is not defined

    And underneath it a second one: Run for Python always started a fresh shell, so the script
    ran in a process that exited immediately. No variables afterwards, an empty workspace panel,
    and figures drawn by something that no longer existed — three symptoms, one cause, and none
    of them said so. Octave had handed the file to the live session since 0.9; Python now does
    the same, which is what makes `clee -w pylab` behave like the notebook it looks like.

    So the check is on all three halves: nothing shell-shaped reaches the prompt, the file runs,
    and **the session still has what the file defined** — the last is the one that matters, and
    the only one a fresh shell could not fake.
    """
    root = tempfile.mkdtemp(prefix="clee_run_")
    open(os.path.join(root, "hello.py"), "w").write(
        "marker = 8484\nprint('ciao dal file')\n")
    session = Session(binary, root)
    try:
        if not session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, 20):
            report.check("Run: the app draws its first frame", False, session)
            return
        session.send(" ")
        session.wait(lambda s: "hello.py" in s.text(), 10)

        # The file first, and *then* the prompt. The other order does not work from a driver and
        # the reason is worth writing down: with the terminal focused, Ctrl+O does not reach the
        # quick-open box, so the name typed after it goes to whatever is at the prompt — which in
        # this test is Python, and `hello` is not defined there. Which is the same class of
        # mistake this whole check is about, arriving from the driver's end.
        session.send("\x0f")                                  # Ctrl+O, quick-open
        session.wait(lambda s: "hello.py" in s.text(), 8)
        session.send("hello.py")
        session.wait(lambda s: True, 0.6)
        session.send("\r")
        session.wait(lambda s: "print('ciao dal file')" in s.text(), 8)

        # The prompt has to be up *before* Run is pressed: `shell_running` reads the process
        # table, and a Python that has not finished starting is not in it yet.
        session.send("\x1b[1;7B")                             # Ctrl+Alt+↓, focus the terminal
        session.wait(lambda s: True, 1.5)
        session.send("python3\r")
        up = session.wait(lambda s: ">>>" in s.text(), 25)
        report.check("Run: a Python prompt is open in the only terminal", up, session)
        if not up:
            return

        session.press(session.chord("r"), lambda s: "ciao dal file" in s.text(), 12)
        report.check("Run does not type a shell command at the interpreter's prompt",
                     "NameError" not in session.text(), session,
                     note=repr([l.strip()[:60] for l in session.lines() if "NameError" in l]))
        report.check("Run hands the file to the session that is already open",
                     any("exec(open(" in line for line in session.lines()), session,
                     note=repr([l.strip()[:70] for l in session.lines() if "exec(open(" in l][:1]))
        # And no pane nobody asked for: handing it to the session must not also start a shell.
        # A terminal's border carries the System 7 close box — "┌ □ ─ Terminal", not "┌ Terminal".
        report.check("without opening a terminal of its own",
                     sum(1 for l in session.lines() if "┌ □ ─ Terminal" in l) == 1, session,
                     note=repr([l.strip()[:24] for l in session.lines() if "Terminal" in l and "┌" in l]))

        # The half a fresh shell could not fake: ask the session, afterwards, for what the file
        # defined. This is the whole reason for running it there.
        session.send("print('marker vale', marker * 2)\r")
        report.check("and the session still has what the file defined",
                     session.wait(lambda s: "marker vale 16968" in s.text(), 10), session)
    finally:
        session.close()
        shutil.rmtree(root, ignore_errors=True)


def main():
    binary = binary_from_argv(sys.argv)
    report = Report()
    for spec in LANGUAGES:
        missing = [p for p in spec["programs"] if not installed(p)]
        if missing:
            print(f"  SKIP  {spec['name']}: {', '.join(missing)} not installed")
            continue
        run_one(binary, spec, report)
    # Not about cells, and here because this is the file that already knows how to get a real
    # interpreter prompt up inside CleeCode — which is the whole setup the check needs.
    if installed("python3"):
        run_lands_in_the_live_session(binary, report)
    else:
        print("  SKIP  Run at a prompt: python3 not installed")
    return report.finish()


if __name__ == "__main__":
    try:
        sys.exit(main())
    except BrokenPipeError:
        os._exit(0)
