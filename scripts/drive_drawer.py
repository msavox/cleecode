#!/usr/bin/env python3
"""Summon the agent drawer and check it is a drawer rather than a fourth terminal.

    python3 scripts/drive_drawer.py [path/to/clee]

The drawer's promises are all about *where* it lives, so every check here is about the screen and
about what survives: does `Ctrl+Shift+A` open it when there is nobody to talk to, does the column
come out of the layout rather than sit over the top of it, does the agent's output keep arriving
after the first frame, does the conversation survive a workspace switch and being hidden, and does
an agent that ends leave the list of four rather than a shell wearing an agent's frame.

Then the same drawer in the other mode. On autocollapse it is on screen exactly while it holds
the keyboard, and it is *painted over* the frames rather than carved out of them — so the proof
is a picture compared with itself: every cell left of the seam has to be the cell it already was
while the drawer was away. Nothing under it was resized, so no pty was told anything happened.

Two of the four agents are stubbed onto the front of `PATH` and two are deliberately left off it,
which is what lets the launcher's honesty be read as a colour: a name that is not installed is
shown anyway, dim, with the reason beside it. Those two checks are skipped where the machine
running this really has opencode or gemini installed — there the screen would be right and the
check wrong.

Nothing here needs a real agent. A stub says who it is and then reads its prompt a line at a
time, so `SUBMITTED` appearing is exactly the thing CleeCode promises never to do on your behalf.
"""

import os
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_drive import Report, Session, binary_from_argv  # noqa: E402

# Stubbed, so the launcher can start them and the process table can be asked about them.
INSTALLED = ["claude", "codex"]
# Left off the PATH on purpose: the launcher has to show these too, and say why it cannot run
# them. The empty drawer is where you find out what CleeCode knows about.
MISSING = ["opencode", "gemini"]

# The same stand-in drive_presets.py uses, and for the same reason: a `read` returns when Enter is
# pressed and not before, so a line only ever appears as SUBMITTED because a person pressed it.
STUB = """#!/bin/sh
echo "AGENT-STUB %s ready session=${CLEE_SESSION:+yes}"
while IFS= read -r line; do
    echo "SUBMITTED: $line"
done
echo "AGENT-STUB %s done"
"""


def fake_agents(root):
    """A directory holding a stub for each *installed* agent, to be put in front of `PATH`."""
    bin_dir = os.path.join(root, "fakebin")
    os.makedirs(bin_dir, exist_ok=True)
    for name in INSTALLED:
        path = os.path.join(bin_dir, name)
        with open(path, "w") as handle:
            handle.write(STUB % (name, name))
        os.chmod(path, 0o755)
    return bin_dir


def really_installed(name):
    """Whether this machine actually has `name`, which is when the not-installed checks have to
    be skipped rather than failed. CleeCode looks past the PATH into the package managers'
    directories, so this looks there too."""
    if shutil.which(name):
        return True
    extra = ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"]
    return any(os.access(os.path.join(d, name), os.X_OK) for d in extra)


# ---- reading the drawer off the screen ----------------------------------------------------


def drawer_column(session):
    """The column the drawer's left border sits at, found from the title on its own border.

    Read off the screen rather than computed from the percentage, so a check about the layout is
    a check about the layout and not about this file's arithmetic. It has to be the *border*: the
    status line says "the agent is still running in it" the moment the drawer is hidden, and a
    plain search for the word found that sentence and reported the column still there."""
    for y in range(session.rows):
        line = session.full_line(y)
        for title in (" agent ", " Claude Code ", " codex ", " opencode ", " gemini "):
            at = line.find(title)
            if at > 0 and line[at - 1] == "\u250c":
                return at - 1
    return None


def vertical_borders(session, row):
    return session.full_line(row).count("│")


def caption_style(session, name):
    """The colour the launcher drew an agent's lower-case name in.

    The caption, not the wordmark: it is one span of one style, so a single cell answers, and it
    is drawn in the same colour as the wordmark above it."""
    for y in range(session.rows):
        line = session.full_line(y)
        at = line.find(name)
        if at < 0:
            continue
        # Only the launcher's own caption: it is preceded by the two-column marker gutter and
        # followed by a space or the honest phrase, never by more letters.
        after = line[at + len(name):at + len(name) + 1]
        if after and after not in " ·":
            continue
        return session.cells(y)[at].fg
    return None


def menu_action(session, menu_letter, label, report, note):
    """Runs a menu item by name: open the bar, jump to the menu by its first letter, walk down to
    the row and press Enter.

    The walk is measured against the highlight actually on screen — the selected row is drawn
    reversed — rather than by counting items in the source, so a menu that grows an entry does
    not silently make this press the wrong one."""
    if not session.press(session.chord("b"), lambda s: "View" in s.text(), 4):
        report.check(f"the menu bar opens for {label}", False, session)
        return False
    session.press(menu_letter, lambda s: label in s.text(), 4)
    if label not in session.text():
        report.check(f"{label} is in the menu", False, session, note=note)
        session.send("\x1b")
        return False
    for _ in range(24):
        want = session.row_of(label)
        if want is None:
            break
        # Only inside the dropdown. The open menu's own title on the top row is drawn reversed
        # too, and taking the first reversed row on screen found that one every time — a walk
        # that never arrived because it was measuring from somewhere else entirely.
        row = session.full_line(want)
        left = row.rfind("\u2502", 0, row.index(label))
        here = next(
            (y for y in range(1, session.rows)
             if any(c.reverse for c in session.cells(y)[max(left, 0):])),
            None,
        )
        if here is None:
            break
        if here == want:
            session.press("\r", lambda s: True, 2)
            return True
        session.press("\x1b[B" if here < want else "\x1b[A", lambda s: True, 0.4)
    report.check(f"{label} can be reached in the menu", False, session)
    session.send("\x1b")
    return False


def settings_toggle(session, label, report):
    """Opens the settings box, walks to the row called `label`, picks it and closes the box.

    Returns the row as it read *after* the change, with the box still open — a two-state row is
    checked by what it says it is, and it says it in there.

    The walk is measured against the marker actually on screen — the chosen row is the one
    written with `> ` in front of it — rather than by counting rows in the source, so a setting
    added above this one flips nothing by surprise."""
    if not session.press(session.chord("o"), lambda s: label in s.text(), 6):
        report.check(f"the settings box opens for {label}", False, session)
        return None
    for _ in range(24):
        want = session.row_of(label)
        here = next((y for y in range(session.rows) if "│> " in session.full_line(y)), None)
        if want is None or here is None:
            break
        if here == want:
            session.press("\r", lambda s: True, 0.5)
            picked = session.full_line(session.row_of(label) or want)
            session.press("\x1b", lambda s: label not in s.text(), 3)
            return picked
        session.press("\x1b[B" if here < want else "\x1b[A", lambda s: True, 0.3)
    report.check(f"{label} can be reached in the settings box", False, session)
    session.send("\x1b")
    return None


def main_rows_left_of(session, column):
    """Everything drawn left of `column`, between the menu bar and the status line.

    The two rows that are left out are the two that say what just happened rather than what the
    layout is — the status message and, while a menu is open, the bar. What is left is the frames
    themselves, which is the thing an overlay must not have touched."""
    return [session.full_line(y)[:column] for y in range(1, session.rows - 1)]


def focus_terminal(session):
    """Ctrl+Alt+↓ from wherever the keyboard is, twice — from the drawer it goes left to the
    editor first, and from the editor down to the shells."""
    session.press("\x1b[1;7D", lambda s: True, 0.4)   # Ctrl+Alt+←
    session.press("\x1b[1;7B", lambda s: True, 0.4)   # Ctrl+Alt+↓


def open_file(session, name):
    """Quick-open `name`. A focused pane has first claim on every Ctrl chord, so Ctrl+Tab is the
    way back out before Ctrl+O can be heard — the same manoeuvre as in drive_presets.py."""
    for _ in range(5):
        session.send("\x0f")                                   # Ctrl+O
        if session.wait(lambda s: "matches" in s.text(), 2):
            session.send(name)
            session.wait(lambda s: True, 0.5)
            session.send("\r")
            return True
        session.press("\x1b[9;5u", lambda s: True, 0.4)        # Ctrl+Tab, cycle the frames
    return False


# ---- the run ------------------------------------------------------------------------------


def check_drawer(binary, report):
    root = tempfile.mkdtemp(prefix="clee_drawer_")
    with open(os.path.join(root, "demo.py"), "w") as handle:
        handle.write("value = 1\nprint(value)\n")
    env = {"PATH": fake_agents(root) + os.pathsep + os.environ.get("PATH", "")}

    session = Session(binary, root, env=env, cols=190)
    try:
        started = session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20)
        report.check("the editor opens", started, session)
        if not started:
            return
        session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 8)
        middle = session.rows // 2
        borders_before = vertical_borders(session, middle)

        # ---- summoning ---------------------------------------------------------------------
        # There is no agent anywhere, so the key that hands an agent the context has nobody to
        # hand it to. It opens the panel whose job that is instead of reporting a dead end.
        summoned = session.press(
            session.chord("a"), lambda s: drawer_column(s) is not None, 8)
        report.check("Ctrl+Shift+A with no agent anywhere opens the drawer", summoned, session,
                     note="the key summons the panel when there is nobody to talk to")
        if not summoned:
            return

        names = [n for n in INSTALLED + MISSING if n in session.text()]
        report.check("the launcher shows all four agents", len(names) == 4, session,
                     note="found %s" % names)

        # ---- the column is part of the layout, not over the top of it -----------------------
        left = drawer_column(session)
        report.check("the drawer is a column on the right",
                     left is not None and left > session.cols // 2, session,
                     note="its left border is at column %s of %d" % (left, session.cols))
        report.check("the frames made room for it",
                     vertical_borders(session, middle) > borders_before, session,
                     note="%d vertical borders before, %d after"
                          % (borders_before, vertical_borders(session, middle)))

        # ---- the honest half of the empty state ---------------------------------------------
        # Whichever of the two this machine genuinely does not have. Not a failure when it has
        # both: CleeCode looks past the PATH into the package managers' directories, so an agent
        # installed on this laptop is found however the driver arranges the environment — and
        # then drawing it lit is correct.
        absent = next((name for name in MISSING if not really_installed(name)), None)
        if absent is None:
            print("  SKIP  the dim names: this machine really has %s" % ", ".join(MISSING))
        else:
            here = caption_style(session, "codex")
            gone = caption_style(session, absent)
            report.check("an agent that is not installed is drawn dimmer than one that is",
                         here is not None and gone is not None and here != gone, session,
                         note="codex is %r, %s is %r" % (here, absent, gone))
            report.check("and says so in words",
                         "not installed" in session.text(), session)

        # ---- arrows and Enter ----------------------------------------------------------------
        # Four presses is a full turn of the ring, so this lands back on the first name having
        # been through every one of them — which is the arrows working, and the wrap with them.
        for _ in range(4):
            session.press("\x1b[B", lambda s: True, 0.3)
        session.send("\r")
        ready = session.wait(lambda s: "AGENT-STUB claude ready" in s.text(), 30)
        report.check("Enter starts the highlighted agent", ready, session)
        if not ready:
            return

        # In the drawer, not somewhere on screen: the name is also in the status line.
        in_drawer = "AGENT-STUB claude ready" in "\n".join(
            session.frame_of("AGENT-STUB claude ready"))
        banner_col = session.column_of("AGENT-STUB claude ready")
        report.check("the agent's output arrives inside the drawer",
                     in_drawer and banner_col is not None and banner_col > session.cols // 2,
                     session,
                     note="output flows into a pane that is not in app.terminals")

        # ---- typing reaches it ---------------------------------------------------------------
        session.send("hello")
        echoed = session.wait(
            lambda s: "hello" in "\n".join(s.frame_of("AGENT-STUB claude ready")), 6)
        report.check("typing reaches the agent's prompt", echoed, session)
        # The pane is spawned exactly like every other one, so it inherits CLEE_SESSION — and an
        # MCP server the agent starts in here is joined to *this* CleeCode by descent rather than
        # by going looking for one. Nothing in the drawer had to arrange that; it is what comes of
        # the drawer being an ordinary pane in an unusual place.
        report.check("the agent in the drawer inherits this editor's MCP session",
                     "session=yes" in "\n".join(session.frame_of("AGENT-STUB claude ready")),
                     session, note="clee --mcp started from here finds this CleeCode")
        report.check("and nothing was submitted", "SUBMITTED" not in session.text(), session)
        # Taken back off the prompt, so what arrives next is read for itself.
        for _ in range(5):
            session.send("\x7f")
        session.wait(lambda s: True, 0.5)

        # ---- precedence ----------------------------------------------------------------------
        # A second agent, in an ordinary terminal, started by typing its name. Ctrl+Shift+A now
        # has two to choose between, and the drawer is the one it means.
        focus_terminal(session)
        session.send("codex\r")
        other = session.wait(lambda s: "AGENT-STUB codex ready" in s.text(), 30)
        report.check("a second agent starts in an ordinary terminal", other, session)

        if not open_file(session, "demo"):
            report.check("the file opens", False, session)
            return
        session.wait(lambda s: "value = 1" in s.text(), 8)
        session.send(session.chord("a"))
        arrived = session.wait(
            lambda s: "demo.py:1" in "\n".join(s.frame_of("AGENT-STUB claude ready")), 8)
        report.check("Ctrl+Shift+A prefers the drawer over a terminal holding an agent",
                     arrived, session,
                     note="the reference lands at the drawer's prompt, not the terminal's")
        report.check("and still submits nothing", "SUBMITTED" not in session.text(), session)
        if arrived:
            in_terminal = "\n".join(session.frame_of("AGENT-STUB codex ready"))
            report.check("the terminal's agent was not given it too",
                         "demo.py:1" not in in_terminal, session)

        # ---- the conversation survives a workspace switch -------------------------------------
        # The layout workspace rebuilds every terminal window there is. The drawer is not one of
        # them, and this is the check that says so.
        before = "\n".join(session.frame_of("AGENT-STUB claude ready"))
        if menu_action(session, "w", "Open workspace", report, "the Workspace menu"):
            picked = session.wait(lambda s: "Default layout" in s.text(), 6)
            report.check("the workspace picker opens", picked, session)
            if picked:
                session.press("\r", lambda s: "Default layout" not in s.text()
                              or "AGENT-STUB" in s.text(), 15)
                session.wait(lambda s: True, 1.0)
                after = "\n".join(session.frame_of("AGENT-STUB claude ready"))
                report.check("switching workspace leaves the drawer's conversation untouched",
                             after.strip() != "" and after == before, session,
                             note="rebuild_terminals never reaches the drawer")

        # ---- hidden is not killed --------------------------------------------------------------
        if menu_action(session, "v", "Agent drawer", report, "the View menu"):
            hidden = session.wait(lambda s: drawer_column(s) is None, 6)
            report.check("the View menu hides the drawer", hidden, session)
            report.check("and says the agent is still running in it",
                         "still running" in session.text(), session)
            if menu_action(session, "v", "Agent drawer", report, "the View menu"):
                back = session.wait(lambda s: "AGENT-STUB claude ready" in s.text(), 8)
                report.check("showing it again brings the conversation back", back, session,
                             note="hiding the column never touched the pty")

        # ---- the other mode: autocollapse ----------------------------------------------------
        # The setting is flipped through the box a user would flip it in, so the row and the
        # behaviour are checked as one thing. Everything from here on is about a drawer that is
        # on screen exactly while it holds the keyboard — and about the pty behind it never
        # noticing any of it.
        if not open_file(session, "demo"):
            report.check("the file opens again", False, session)
            return
        session.wait(lambda s: "value = 1" in s.text(), 8)
        pinned_borders = vertical_borders(session, middle)
        conversation = "\n".join(session.frame_of("AGENT-STUB claude ready"))
        # The keyboard goes into the drawer before the mode changes: where the focus is is the
        # whole of what autocollapse depends on, and a panel you are working in must not vanish
        # from under you because a setting was flipped.
        session.press("\x1b[1;7C", lambda s: True, 0.5)   # Ctrl+Alt+\u2192, into the drawer

        row = settings_toggle(session, "Agent drawer", report)
        if row:
            report.check("the setting reads out its new mode",
                         "over the frames" in row, session,
                         note="a two-state row says which state it is in, not on/off: %r"
                              % row.strip())
            # Switching modes touched no pty and closed nothing: the keyboard is still in the
            # drawer, and that is the whole of what decides whether it is on screen.
            still_there = session.wait(lambda s: drawer_column(s) is not None, 4)
            report.check("switching mode while it is open leaves it open", still_there, session)

            # And out. One arrow leaves the drawer for the editor, which is the signal — in a
            # TUI there is no pointer leaving a panel, but there is always a keyboard arriving
            # somewhere else.
            session.press("\x1b[1;7D", lambda s: drawer_column(s) is None, 6)   # Ctrl+Alt+←
            gone = drawer_column(session) is None
            report.check("the focus leaving withdraws the drawer", gone, session,
                         note="autocollapse: the signal is the focus, not the mouse")
            collapsed_borders = vertical_borders(session, middle)
            report.check("and the frames have the screen back",
                         collapsed_borders < pinned_borders, session,
                         note="%d vertical borders with the column, %d without it"
                              % (pinned_borders, collapsed_borders))

            # Summoned again from the menu, so nothing is typed into the agent and the two
            # pictures of the conversation can be compared byte for byte.
            underneath = main_rows_left_of(session, session.cols)
            if menu_action(session, "v", "Agent drawer", report, "the View menu"):
                back = session.wait(lambda s: drawer_column(s) is not None, 8)
                report.check("summoning brings it back", back, session)
                if back:
                    seam = drawer_column(session)
                    report.check("the conversation is exactly where it was",
                                 "\n".join(session.frame_of("AGENT-STUB claude ready"))
                                 == conversation, session,
                                 note="the same pty: hiding the column never touched it")
                    # The mode's whole reason: it paints over the frames instead of taking a
                    # column from them, so nothing under it was resized and no pty was sent a
                    # SIGWINCH on the way in or the way out.
                    report.check("it is painted over the frames, which were not resized",
                                 [row[:seam] for row in underneath]
                                 == main_rows_left_of(session, seam), session,
                                 note="every cell left of the seam is the cell it already was")

            # ---- Ctrl+Shift+A on a collapsed drawer ------------------------------------------
            # The founding rule of the key: the text has to arrive where it can be read. A
            # drawer that is away still holds the agent and still wins the precedence, so the
            # key has to reopen it before the reference lands.
            session.press("\x1b[1;7D", lambda s: drawer_column(s) is None, 6)   # Ctrl+Alt+←
            report.check("it withdraws again on the way back to the editor",
                         drawer_column(session) is None, session)
            session.send(session.chord("a"))
            reopened = session.wait(
                lambda s: drawer_column(s) is not None
                and "demo.py:1" in "\n".join(s.frame_of("AGENT-STUB claude ready")), 8)
            report.check("Ctrl+Shift+A reopens the collapsed drawer and puts the reference in it",
                         reopened, session,
                         note="text at a prompt nobody can see is worse than no text")
            report.check("and still submits nothing", "SUBMITTED" not in session.text(), session)

            # Back to pinned, so the checks below run in the mode the rest of this file assumes.
            # The keyboard is in the drawer, so it stays on screen across the change.
            settings_toggle(session, "Agent drawer", report)

        # ---- an agent that ends leaves the list, not a shell -------------------------------------
        # Ctrl+D at the stub's prompt closes its stdin, which ends it. The pane was started with
        # `exec`, so the pane ends with the agent — and what is left is the choice, not a shell
        # sitting in an agent's frame pretending to be one.
        #
        # Ctrl+U first, because there is still a reference sitting unsent at that prompt from the
        # precedence check above, and end-of-file on a half-typed line is not end of anything.
        # Reopening the drawer put the keyboard back in it, so no focus move is needed — and an
        # arrow sent here would be typed at the agent, which is the drawer working correctly.
        session.press("\x15", lambda s: True, 0.5)
        session.send("\x04")
        gone = session.wait(
            lambda s: "AGENT-STUB claude ready" not in s.text()
            and drawer_column(s) is not None, 15)
        report.check("an agent that ends returns the drawer to the list of four", gone, session,
                     note="never a respawned shell: that would look like the agent still being there")
        if gone:
            still_four = all(name in session.text() for name in INSTALLED + MISSING)
            report.check("and the four names are on offer again", still_four, session)
    finally:
        session.close()
        shutil.rmtree(root, ignore_errors=True)


def main():
    binary = binary_from_argv(sys.argv)
    report = Report()
    check_drawer(binary, report)
    return report.finish()


if __name__ == "__main__":
    try:
        sys.exit(main())
    except BrokenPipeError:
        os._exit(0)
