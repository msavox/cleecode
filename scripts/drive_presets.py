#!/usr/bin/env python3
"""Open each `clee -w <preset>` and check what you actually get.

    python3 scripts/drive_presets.py [path/to/clee]

A preset is a promise about what appears when you type its name, and the only way to check a
promise like that is to type it. Every check here is about the screen: is the interpreter at its
prompt, is there a shell beside it, did the frames land where the width says they should, and —
running the same preset in a narrow window — did they move.

The agent presets are checked the same way, against a stand-in rather than the real Claude Code:
what is being promised is a tab named after the agent that runs the agent, and a shell beside it.
A stub on `PATH` proves that better than the real program would, because it can also say what
reached its prompt — which is how the last two checks read `Ctrl+Shift+A` for what it is: the
context arrives, and *nothing is submitted* until Enter is pressed.

One check is not about a preset at all: an agent typed by hand into a plain shell, installed the
way npm installs Claude Code. There the process table says `node`, nothing has declared the pane
an agent's, and finding it is entirely the process table's job — which is the case the presets
cannot exercise, because a preset pane always carries the command it was opened with.

Skips a language whose interpreter is not installed rather than passing quietly, and skips the
npm-shaped check where there is no node to reproduce it with.
"""

import os
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_drive import Report, Session, binary_from_argv  # noqa: E402

PRESETS = [
    {"name": "octave", "needs": "octave", "prompt": ">>", "tab": "octave"},
    {"name": "pylab", "needs": "python3", "prompt": ">>>", "tab": "python"},
]

AGENTS = ["claude", "opencode", "codex", "gemini"]

# A stand-in for an agent: says who it is, then reads its prompt a line at a time and says what
# it was given. Line at a time is the point — a `read` returns when Enter is pressed and not
# before, so "SUBMITTED" appearing is exactly the thing CleeCode promises never to do on your
# behalf.
STUB = """#!/bin/sh
echo "AGENT-STUB %s ready"
while IFS= read -r line; do
    echo "SUBMITTED: $line"
done
"""

# The same stand-in wearing the shape npm installs Claude Code in: a JavaScript file with a node
# shebang, so running it runs *node*, and the process table shows `node` with this file as its
# argument. That is the whole bug this reproduces — a `claude` started by hand in an ordinary
# shell was invisible to Ctrl+Shift+A, because CleeCode looked for a process called `claude` and
# there is none. Written as a real script run by a real node rather than faked, because a stub
# that only pretended to be node would prove nothing about what the process table says.
NODE_STUB = """#!/usr/bin/env node
process.stdout.write("AGENT-STUB claude ready\\n");
require("readline")
    .createInterface({ input: process.stdin, terminal: false })
    .on("line", (line) => process.stdout.write("SUBMITTED: " + line + "\\n"));
"""


def fake_agents(root):
    """A directory holding a stub for each agent, to be put in front of `PATH`.

    In front, so this is deterministic on a machine that has the real ones installed: the check
    is about the preset, not about whichever agent happens to be on this laptop.
    """
    bin_dir = os.path.join(root, "fakebin")
    os.makedirs(bin_dir, exist_ok=True)
    for name in AGENTS:
        path = os.path.join(bin_dir, name)
        with open(path, "w") as handle:
            handle.write(STUB % name)
        os.chmod(path, 0o755)
    return bin_dir


def open_file(session, name):
    """Quick-open `name`, from wherever the keyboard happens to be.

    A focused terminal has first claim on every Ctrl chord, so Ctrl+O typed at a shell goes to
    the shell; Ctrl+Tab is the one it hands back, and it is the way out of the pane. Same
    manoeuvre as in drive_cells.py, for the same reason.
    """
    for _ in range(4):
        session.send("\x0f")                                   # Ctrl+O
        if session.wait(lambda s: "matches" in s.text(), 2):
            session.send(name)
            session.wait(lambda s: True, 0.5)
            session.send("\r")
            return True
        session.press("\x1b[9;5u", lambda s: True, 0.4)        # Ctrl+Tab, cycle the frames
    return False


def check_agent_preset(binary, name, report):
    """`clee -w <agent>`: a tab of that name running that command, and a shell beside it."""
    root = tempfile.mkdtemp(prefix="clee_agent_")
    # Something for the editor to be looking at, so the shortcut has a place to point at.
    with open(os.path.join(root, "demo.py"), "w") as handle:
        handle.write("value = 1\nprint(value)\n")
    env = {"PATH": fake_agents(root) + os.pathsep + os.environ.get("PATH", "")}

    session = Session(binary, root, env=env, args=["-w", name], cols=190)
    try:
        started = session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20)
        report.check(f"{name}: the preset opens", started, session)
        if not started:
            return
        session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 8)

        # The preset still works — every check below is the same one it passed yesterday — and
        # this release is where it starts saying it will not work forever. The drawer is the
        # agent surface now, and a published command on its way out has to announce it while it
        # still runs, not once it has already gone. Read after the splash, because the splash is
        # drawn instead of the status line and not over it.
        # Matched on the stem, so the sentence reads in either language.
        said = session.wait(lambda s: "deprecat" in s.text(), 6)
        report.check(f"{name}: the status line says the preset is deprecated", said, session,
                     note="and names Ctrl+Shift+A, the drawer that replaces it")

        # The startup command ran, and it ran *that* program: the stub says its own name.
        ready = session.wait(lambda s: f"AGENT-STUB {name} ready" in s.text(), 30)
        report.check(f"{name}: the tab starts the agent by itself", ready, session,
                     note="nothing was typed to start it")

        # On the tab, not merely somewhere on screen: the name is also in the menu bar's
        # workspace label and in the shell echo that started it.
        strip = next((line for line in session.lines() if "shell ✕" in line), "")
        report.check(f"{name}: its tab carries the agent's name", f"{name} ✕" in strip, session,
                     note=repr(strip[-70:]))
        report.check(f"{name}: a plain shell sits beside it in the same window",
                     "shell" in session.text(), session)
        if not ready:
            return

        # Ctrl+Shift+A. The reference has to arrive at the agent's own prompt — read out of that
        # pane's frame, since "demo.py" is in the editor's tab strip the whole time.
        if not open_file(session, "demo"):
            report.check(f"{name}: the file opens", False, session)
            return
        session.wait(lambda s: "value = 1" in s.text(), 8)
        session.send(session.chord("a"))
        arrived = session.wait(
            lambda s: "demo.py:1" in "\n".join(s.frame_of(f"AGENT-STUB {name} ready")), 8)
        report.check(f"{name}: Ctrl+Shift+A writes the reference at the agent's prompt",
                     arrived, session)

        # And left it there. This is the whole discipline of the key: CleeCode never presses
        # Enter for you, so the stub — which speaks only when a line is completed — has said
        # nothing yet.
        report.check(f"{name}: nothing was submitted", "SUBMITTED" not in session.text(), session,
                     note="the agent is only asked when the user presses Enter")

        # Pressing it is what sends, and the keyboard is already in the pane holding the text.
        submitted = session.press("\r", lambda s: "SUBMITTED: demo.py:1" in s.text(), 8)
        report.check(f"{name}: Enter is what sends it", submitted, session)
    finally:
        session.close()
        shutil.rmtree(root, ignore_errors=True)


def check_npm_wrapper(binary, report):
    """A `claude` typed by hand into a plain shell, installed the way npm installs it.

    Deliberately not a preset. A preset pane carries the command it was opened with, and that
    command is CleeCode's second answer to "is there an agent in here" — so a check run against a
    preset would pass whether or not the process table is read correctly. Here nothing has been
    declared: an ordinary shell, in an ordinary window, with the agent started by typing its name.
    The only thing that can find it is the process table, where it appears as `node`.
    """
    if shutil.which("node") is None:
        print("  SKIP  npm wrapper: node is not installed, so the npm shape cannot be reproduced")
        return
    root = tempfile.mkdtemp(prefix="clee_npm_")
    with open(os.path.join(root, "demo.py"), "w") as handle:
        handle.write("value = 1\nprint(value)\n")
    bin_dir = os.path.join(root, "npmbin")
    os.makedirs(bin_dir, exist_ok=True)
    wrapper = os.path.join(bin_dir, "claude")
    with open(wrapper, "w") as handle:
        handle.write(NODE_STUB)
    os.chmod(wrapper, 0o755)
    env = {"PATH": bin_dir + os.pathsep + os.environ.get("PATH", "")}

    session = Session(binary, root, env=env, cols=190)
    try:
        started = session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20)
        report.check("npm wrapper: the editor opens", started, session)
        if not started:
            return
        session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 8)

        # Into the terminal, and type the agent's name at it — which is what somebody who has
        # Claude Code from npm does, every time.
        session.press("\x1b[1;7B", lambda s: True, 1)               # Ctrl+Alt+↓, focus terminal
        session.send("claude\r")
        ready = session.wait(lambda s: "AGENT-STUB claude ready" in s.text(), 30)
        report.check("npm wrapper: the agent starts, running as a node process", ready, session)
        if not ready:
            return

        if not open_file(session, "demo"):
            report.check("npm wrapper: the file opens", False, session)
            return
        session.wait(lambda s: "value = 1" in s.text(), 8)
        session.send(session.chord("a"))
        arrived = session.wait(
            lambda s: "demo.py:1" in "\n".join(s.frame_of("AGENT-STUB claude ready")), 8)
        report.check("npm wrapper: Ctrl+Shift+A finds the agent hiding behind `node`",
                     arrived, session,
                     note="nothing declared this pane an agent's; only the process table knows")
        # Same discipline as everywhere else: the text is there and the question is still the
        # user's to ask.
        report.check("npm wrapper: nothing was submitted", "SUBMITTED" not in session.text(),
                     session)
    finally:
        session.close()
        shutil.rmtree(root, ignore_errors=True)


def open_preset(binary, name, root, cols):
    """A session started as `clee -w <name> .`, in a window `cols` wide."""
    return Session(binary, root, args=["-w", name], cols=cols)


def check_preset(binary, spec, report):
    root = tempfile.mkdtemp(prefix="clee_preset_")
    # A script to work on, so the preset opens over something realistic.
    with open(os.path.join(root, "demo." + ("m" if spec["name"] == "octave" else "py")), "w") as h:
        h.write("%% one\na = 1;\n" if spec["name"] == "octave" else "# %% one\na = 1\n")

    wide = open_preset(binary, spec["name"], root, 190)
    try:
        started = wide.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20)
        report.check(f"{spec['name']}: the preset opens", started, wide)
        if not started:
            return
        wide.send(" ")
        wide.wait(lambda s: "Files" in s.text(), 8)

        # The interpreter starts itself. Nothing was typed at it.
        at_prompt = wide.wait(lambda s: spec["prompt"] in s.text(), 40)
        report.check(f"{spec['name']}: the interpreter is already at its prompt", at_prompt, wide,
                     note="nothing was typed to start it")

        # On the tab, not merely on screen. The name is written in the menu bar's workspace
        # label, in Octave's own banner (three times, in URLs) and in the shell echo that
        # started it — so "somewhere on screen" was satisfied for both presets with the tab
        # strip removed entirely.
        strip = next((line for line in wide.lines() if "shell ✕" in line), "")
        report.check(f"{spec['name']}: its tab carries the interpreter's name",
                     spec["tab"] in strip, wide, note=repr(strip[:60]))
        report.check(f"{spec['name']}: a plain shell sits beside it in the same window",
                     "shell" in wide.text(), wide)

        # Underneath, and at this width on purpose. This check used to require the prompt to
        # be *beside* the editor on a wide window, and it had been failing since 0.9.2 without
        # anyone reading it: the presets put the prompt underneath at every width, because the
        # editor splits to put a figure next to the code that drew it and a third window
        # alongside makes each half a third of the screen. A plot a third of a window wide is a
        # thumbnail. The decision is in docs/ROADMAP.md under 0.9.2; the check now holds it.
        term_col = wide.column_of("shell")
        term_row = wide.row_of("shell")
        report.check(f"{spec['name']}: at 190 columns the prompt is underneath, not beside",
                     term_col is not None and term_col < 95 and term_row is not None and term_row > 10,
                     wide, note=f"the terminal starts at column {term_col} of 190, row {term_row}")
    finally:
        wide.close()

    narrow = open_preset(binary, spec["name"], root, 92)
    try:
        if not narrow.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            report.check(f"{spec['name']}: the preset opens narrow", False, narrow)
            return
        narrow.send(" ")
        narrow.wait(lambda s: "Files" in s.text(), 8)
        narrow.wait(lambda s: spec["prompt"] in s.text(), 40)
        term_col = narrow.column_of("shell")
        term_row = narrow.row_of("shell")
        # And underneath here too — the same arrangement, which is the point. This used to be
        # written as "it *moves* underneath instead", implying the wide window put it elsewhere;
        # it passed against a layout identical to the wide one and so demonstrated nothing.
        report.check(f"{spec['name']}: at 92 columns it is underneath as well",
                     term_col is not None and term_col < 46 and term_row is not None and term_row > 10,
                     narrow, note=f"the terminal starts at column {term_col}, row {term_row}")
        report.check(f"{spec['name']}: the file tree survives the narrow window",
                     "Files" in narrow.text(), narrow)
    finally:
        narrow.close()
        shutil.rmtree(root, ignore_errors=True)


def main():
    binary = binary_from_argv(sys.argv)
    report = Report()
    for spec in PRESETS:
        if shutil.which(spec["needs"]) is None:
            print(f"  SKIP  {spec['name']}: {spec['needs']} not installed")
            continue
        check_preset(binary, spec, report)
    # No skip here: an agent preset needs no agent installed, only a program of that name.
    for name in AGENTS:
        check_agent_preset(binary, name, report)
    check_npm_wrapper(binary, report)
    return report.finish()


if __name__ == "__main__":
    try:
        sys.exit(main())
    except BrokenPipeError:
        os._exit(0)
