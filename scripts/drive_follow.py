#!/usr/bin/env python3
"""Write to the project from outside and watch CleeCode react, the way an agent makes it react.

    python3 scripts/drive_follow.py [path/to/clee]     # default: target/debug/clee

An agent does not type: it writes the whole file, atomically, over and over. Everything here is
therefore done with `open(...).write(...)` from this script while the editor is running, which is
indistinguishable — from CleeCode's side — from `claude`, `codex`, `opencode` or a `sed` in one
of its own terminal panes.

Three claims, in the order a user meets them:

  · a file open in a tab reloads on its own when something rewrites it
  · the lines that arrived are lit in the gutter, in a colour the other lines do not have
  · with follow mode on, a file that is *not* open opens beside the work — and the keyboard
    stays where it was, which is checked by typing and looking at where the character landed

The window is opened wide on purpose: below 120 columns CleeCode will not split the editor, and
the whole point of the third claim is a file arriving *beside* what you are reading.
"""

import os
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_drive import Report, Session, binary_from_argv  # noqa: E402

OPEN_FILE = "opened.txt"
UNOPENED_FILE = "untouched_by_you.txt"
# Two more, written in the same breath, to see a burst arrive as a sequence rather than be lost.
BURST = ("first_of_two.txt", "second_of_two.txt")

FIRST = "alpha\nbeta\ngamma\ndelta\n"
# The same file with one line different — what an agent's edit actually looks like on disk.
REWRITTEN = "alpha\nbeta\nAN AGENT WROTE THIS\ndelta\n"


def git(root, *args):
    env = dict(os.environ, GIT_TERMINAL_PROMPT="0", GIT_PAGER="cat")
    subprocess.run(["git", *args], cwd=root, env=env, capture_output=True, text=True)


def make_repo(root):
    """A committed repository, so every later write shows up as a change against HEAD.

    `.config` is ignored because the driver puts CleeCode's own settings inside the project: left
    visible it would be a changed file like any other, and follow mode would dutifully open
    settings.toml the moment a setting was saved."""
    git(root, "init", "-q", "-b", "main")
    git(root, "config", "user.name", "Driver")
    git(root, "config", "user.email", "driver@example.invalid")
    git(root, "config", "commit.gpgsign", "false")
    write(root, ".gitignore", ".config/\n")
    write(root, OPEN_FILE, FIRST)
    write(root, UNOPENED_FILE, "nothing has happened here yet\n")
    for name in BURST:
        write(root, name, "nothing has happened here yet\n")
    git(root, "add", "-A")
    git(root, "commit", "-q", "-m", "Everything as it was before the agent ran")


def write(root, name, text):
    with open(os.path.join(root, name), "w") as handle:
        handle.write(text)


def open_file(session, name):
    """Ctrl+O, the quick-open, then the name and Enter."""
    session.send("\x0f")
    session.wait(lambda s: "matches" in s.text() or "corrisponden" in s.text(), 8)
    session.send(name)
    session.wait(lambda s: True, 0.6)
    session.send("\r")
    return session.wait(lambda s: name in s.text(), 8)


def gutter_colour(session, needle):
    """The foreground colour of the line number beside the row `needle` is drawn on.

    The gutter carries the mark rather than a column of its own — a red number is a breakpoint, a
    green one a line that just arrived — so the colour of the digits is the thing to read, and
    pyte keeps it on the cell."""
    row = session.row_of(needle)
    if row is None:
        return None
    for cell in session.cells(row):
        if cell.data.isdigit():
            return cell.fg
    return None


def settings_row(session, needle):
    """The line of the settings panel whose label contains `needle`."""
    for line in session.lines():
        if needle in line:
            return line
    return None


def follow_row(session):
    return settings_row(session, "Follow") or settings_row(session, "Segui") or ""


def switch_follow_on(session):
    """Open the settings panel, walk down to the follow row, and turn it on."""
    session.send(session.chord("o"))
    if not session.wait(lambda s: follow_row(s) != "", 8):
        return False
    session.send("\x1b[B" * 9)
    session.wait(lambda s: True, 0.5)
    # Waited on rather than read straight after the key: the panel repaints on the next frame,
    # and reading it in the same breath as pressing Enter reads the old value.
    return session.press("\r", lambda s: follow_row(s).rstrip(" │").endswith("on"), 8)


def outside_a_repository(binary, report):
    """The other half of the promise: where there is no repository, follow mode says so.

    Its own session, in a folder that was never `git init`-ed. There is nothing to see happen
    here — the point is that the editor admits it, rather than leaving a switch reading "on"
    over a feature that can never fire."""
    root = tempfile.mkdtemp(prefix="clee_follow_norepo_")
    write(root, "lonely.txt", "no repository anywhere above this\n")
    session = Session(binary, root, cols=190, rows=34)
    try:
        if not session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            report.check("CleeCode starts outside a repository", False, session)
            return
        session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 8)
        report.check("follow mode can be switched on anywhere", switch_follow_on(session), session)
        session.send("\x1b")
        said = session.wait(
            lambda s: "repository" in s.lines()[-1] or "repositor" in s.lines()[-1], 8
        )
        report.check("with no repository it says so instead of pretending", said, session,
                     note=repr(session.lines()[-1].strip()))
    finally:
        session.close()
        shutil.rmtree(root, ignore_errors=True)


def main():
    binary = binary_from_argv(sys.argv)
    if shutil.which("git") is None:
        print("  SKIP  git is not installed")
        return 0

    root = tempfile.mkdtemp(prefix="clee_follow_")
    make_repo(root)

    report = Report()
    # Wide enough that the editor will split: a file arriving "beside" needs two halves.
    session = Session(binary, root, cols=190, rows=34)
    try:
        if not session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            report.check("CleeCode starts", False, session)
            return 1
        session.send(" ")                                  # past the splash
        session.wait(lambda s: "Files" in s.text(), 8)

        report.check("the file opens", open_file(session, OPEN_FILE), session)
        report.check("its text is on screen", "gamma" in session.text(), session)

        # ---- 3a: the reload, and the lines that came with it ------------------------------
        #
        # A whole second, because the reload is decided by the file's mtime and a filesystem that
        # keeps them to the second cannot tell two writes in the same one apart.
        time.sleep(1.1)
        write(root, OPEN_FILE, REWRITTEN)
        reloaded = session.wait(lambda s: "AN AGENT WROTE THIS" in s.text(), 12)
        report.check("a file rewritten from outside reloads by itself", reloaded, session)
        report.check("and the line it replaced is gone", "gamma" not in session.text(), session)

        arrived = gutter_colour(session, "AN AGENT WROTE THIS")
        untouched = gutter_colour(session, "delta")
        report.check("the line that arrived is lit in the gutter",
                     arrived is not None and arrived != untouched, session,
                     note=f"arrived={arrived} untouched={untouched}")

        # ---- 3b: follow mode ---------------------------------------------------------------
        report.check("the settings panel switches follow mode on", switch_follow_on(session),
                     session, note=repr(follow_row(session)))
        session.send("\x1b")                               # close the panel
        session.wait(lambda s: "Follow" not in s.text() and "Segui" not in s.text(), 6)

        # A file nobody has opened, written from outside. Nothing but `git status` — which is
        # swept for the sidebar's dots anyway — tells CleeCode it happened.
        #
        # Waited for by its *contents*, never by its name: the file tree has been listing that
        # name since the window opened, so a check that looked for it would have passed with
        # follow mode switched off. Only an open tab can put the text on screen.
        written = "the agent wrote a file you never opened"
        write(root, UNOPENED_FILE, written + "\n")
        appeared = session.wait(lambda s: written in s.text(), 20)
        report.check("a file you never opened opens itself", appeared, session)
        report.check("and it landed beside the work, not over it",
                     "AN AGENT WROTE THIS" in session.text(), session,
                     note="the file being read is still on screen alongside it")

        # Two files written inside one sweep. Only one of them can open on that sweep — the
        # window is not to jump twice — so the other has to be remembered and shown on the next
        # one. Read off the tab strip rather than the pane: only the front tab shows its text,
        # and the file tree has been listing both names all along.
        for name in BURST:
            write(root, name, "part of a burst\n")
        both = session.wait(lambda s: all(name in s.lines()[1] for name in BURST), 25)
        report.check("a burst is remembered rather than dropped", both, session,
                     note=repr(session.lines()[1].strip()))

        # The claim the whole thing rests on: it did not take the keyboard. Typing has to land
        # in the file that was already under the cursor.
        session.press("Z", lambda s: "Zalpha" in s.text(), 6)
        report.check("the keyboard never moved", "Zalpha" in session.text(), session,
                     note="the character landed in the file that was already open")
        report.check("and the file that arrived was not typed into",
                     "Zthe agent wrote" not in session.text(), session)

        Report.show("final screen", session)
    finally:
        session.close()
        shutil.rmtree(root, ignore_errors=True)

    outside_a_repository(binary, report)
    return report.finish()


if __name__ == "__main__":
    try:
        sys.exit(main())
    except BrokenPipeError:
        os._exit(0)
