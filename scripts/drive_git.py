#!/usr/bin/env python3
"""Drive the real CleeCode binary against a real repository and make it write to it.

    python3 scripts/drive_git.py [path/to/clee]     # default: target/debug/clee

`src/git.rs` has its own tests, but they are all about *parsing* — the two letters git prints,
the field a rename carries, the path with a space in it. None of them can answer the question
that matters once the panel can write: does pressing `S` put that file in the index, and is it
the file the cursor was on.

So this makes a repository, changes it, and then checks the repository itself with `git` after
every action, rather than reading the panel back to itself. A panel that drew "staged" without
staging anything would pass a screen-only check, and the whole point of the write side is that
something happened on disk.

The one action that cannot be undone gets the most attention here: it must not fire without the
question, the question must not be answered by the keys that do something to the list behind it,
and when it does fire the file has to be back to what `HEAD` says.
"""

import os
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_drive import Report, Session, binary_from_argv  # noqa: E402

COMMITTED = "fn main() {\n    println!(\"one\");\n}\n"
EDITED = "fn main() {\n    println!(\"two\");\n}\n"


def git(root, *args):
    """git, told to say nothing it does not have to and to ask nothing at all."""
    env = dict(os.environ, GIT_TERMINAL_PROMPT="0", GIT_PAGER="cat")
    out = subprocess.run(["git", *args], cwd=root, env=env, capture_output=True, text=True)
    return out.stdout.strip()


def make_repo(root):
    """A repository with one commit in it, then one edit and one file git has never seen."""
    git(root, "init", "-q", "-b", "main")
    # Set on the repository rather than read from the machine: a commit made here must not depend
    # on whoever is running this having configured git, and must not use their name if they have.
    git(root, "config", "user.name", "Driver")
    git(root, "config", "user.email", "driver@example.invalid")
    git(root, "config", "commit.gpgsign", "false")
    with open(os.path.join(root, "main.rs"), "w") as handle:
        handle.write(COMMITTED)
    git(root, "add", "-A")
    git(root, "commit", "-q", "-m", "The commit that was already there")
    with open(os.path.join(root, "main.rs"), "w") as handle:
        handle.write(EDITED)
    with open(os.path.join(root, "notes.txt"), "w") as handle:
        handle.write("something git has never been told about\n")


def status(root):
    """The repository's own account of itself, as `XY path` strings.

    Not through `git()`, which strips: the first of the two letters is a space for a file that is
    changed and not staged, and stripping it turns `" M main.rs"` into `"M main.rs"` — which is
    what a *staged* file would look like if its second letter were stripped too. The leading
    space is data."""
    env = dict(os.environ, GIT_TERMINAL_PROMPT="0", GIT_PAGER="cat")
    out = subprocess.run(["git", "status", "--porcelain"], cwd=root, env=env,
                         capture_output=True, text=True)
    return [line for line in out.stdout.split("\n") if line.strip()]


def open_panel(session):
    """Opens the panel and waits for git to have answered.

    Waiting for the tab label would be waiting for nothing: the tabs are drawn on the first frame
    and the list arrives from a thread some frames later, which is the whole reason the panel
    says "Asking git…" in the meantime."""
    session.send(session.chord("d"))
    # Waited for by the two letters git prints in front of the name, not by the name: the file
    # tree behind the panel lists the same files, and a predicate it satisfies is a predicate
    # that was true before the panel was even open.
    return session.wait(lambda s: "?? notes.txt" in s.text(), 12)


def row_with(session, needle):
    """The screen row showing `needle` inside the panel, or None."""
    for y, line in enumerate(session.lines()):
        if needle in line:
            return y
    return None


def selected_row(session):
    """The highlighted row of the panel's list — the cursor, read off the colours rather than
    inferred from how many times Down was pressed.

    Found by "a run of cells with a background", not by naming a colour: ratatui writes its
    colours as 256-colour codes and pyte hands those back as hex, so `cyan` never matches. The
    lit tab in the header carries the same paint, so the search starts below it."""
    header = row_with(session, "History")
    if header is None:
        return None
    for y in range(header + 1, len(session.lines())):
        cells = session.cells(y)
        if sum(1 for c in cells if c.bg != "default") >= 8:
            return "".join(c.data or " " for c in cells).strip()
    return None


def main():
    binary = binary_from_argv(sys.argv)
    root = tempfile.mkdtemp(prefix="clee_git_")
    make_repo(root)

    report = Report()
    report.check("the repository starts with one edit and one untracked file",
                 sorted(status(root)) == [" M main.rs", "?? notes.txt"],
                 note=str(status(root)))

    session = Session(binary, root)
    try:
        started = session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20)
        report.check("the app draws its first frame", started, session)
        if not started:
            return 1
        session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 10)

        report.check("Ctrl+Shift+D opens the panel with both changed files on it",
                     open_panel(session), session)
        report.check("the cursor starts on the first of them",
                     "main.rs" in (selected_row(session) or ""), session,
                     note=repr(selected_row(session)))

        # ---- staging -----------------------------------------------------------------------
        #
        # Checked against the repository, not against the panel: a panel that drew a file as
        # staged without staging it would pass every screen-only check there is.
        session.press("s", lambda s: True, 2)
        staged = session.wait(lambda s: "M  main.rs" in status(root), 8)
        report.check("S stages the file the cursor is on", staged, session, note=str(status(root)))
        report.check("and left the other one alone", "?? notes.txt" in status(root), session)

        session.press("u", lambda s: True, 2)
        report.check("U takes it back out",
                     session.wait(lambda s: " M main.rs" in status(root), 8), session,
                     note=str(status(root)))

        session.press("a", lambda s: True, 2)
        report.check("A stages everything, the untracked file included",
                     session.wait(lambda s: sorted(status(root)) == ["A  notes.txt", "M  main.rs"], 8),
                     session, note=str(status(root)))

        # ---- committing --------------------------------------------------------------------
        before = git(root, "rev-parse", "HEAD")
        session.press("c", lambda s: "Esc" in s.text(), 4)
        session.send("A commit typed into the panel")
        session.press("\r", lambda s: True, 3)
        committed = session.wait(lambda s: git(root, "rev-parse", "HEAD") != before, 12)
        report.check("C then Enter makes the commit", committed, session)
        report.check("with the message that was typed",
                     git(root, "log", "-1", "--pretty=%s") == "A commit typed into the panel",
                     session, note=git(root, "log", "-1", "--pretty=%s"))
        report.check("and the list is empty afterwards",
                     session.wait(lambda s: not status(root), 8), session, note=str(status(root)))

        # ---- discarding --------------------------------------------------------------------
        #
        # The one action nothing brings back. Three things are checked, and the middle one is
        # the one worth the trouble: the question must not be answerable by the keys that do
        # something to the list behind it.
        with open(os.path.join(root, "main.rs"), "w") as handle:
            handle.write("fn main() { /* about to be thrown away */ }\n")
        session.press("r", lambda s: True, 3)
        report.check("the edit shows up after a refresh",
                     session.wait(lambda s: row_with(s, "main.rs") is not None
                                  and " M main.rs" in status(root), 8), session)

        session.press("x", lambda s: "Y / N" in s.text() or "S / N" in s.text(), 5)
        report.check("X asks before throwing anything away",
                     "Y / N" in session.text() or "S / N" in session.text(), session)

        # `s` is stage on the list underneath. If the question let it through, this would both
        # answer yes and lose the file's contents.
        session.press("s" if "Y / N" in session.text() else "y", lambda s: True, 2)
        report.check("a key that is not the answer cancels rather than confirming",
                     " M main.rs" in status(root), session, note=str(status(root)))

        session.press("x", lambda s: "Y / N" in s.text() or "S / N" in s.text(), 5)
        answer = "y" if "Y / N" in session.text() else "s"
        session.press(answer, lambda s: True, 3)
        gone = session.wait(lambda s: not status(root), 10)
        report.check("answering yes throws the changes away", gone, session, note=str(status(root)))
        report.check("and the file is what the last commit says it is",
                     open(os.path.join(root, "main.rs")).read() != "fn main() { /* about to be thrown away */ }\n",
                     session)

        # ---- branches ----------------------------------------------------------------------
        git(root, "branch", "spike")
        session.press("r", lambda s: True, 3)
        session.press("\t\t\t", lambda s: "spike" in s.text(), 6)      # round to Branches
        report.check("the branches tab lists them", "spike" in session.text(), session)
        row = selected_row(session)
        report.check("a branch is picked out rather than only listed", row is not None, session,
                     note=repr(row))
        if row is not None and "spike" not in row:
            session.press("\x1b[B", lambda s: "spike" in (selected_row(s) or ""), 4)
        session.press("\r", lambda s: True, 3)
        report.check("Enter moves to the branch",
                     session.wait(lambda s: git(root, "rev-parse", "--abbrev-ref", "HEAD") == "spike", 10),
                     session, note=git(root, "rev-parse", "--abbrev-ref", "HEAD"))

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
