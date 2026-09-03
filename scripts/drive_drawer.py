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


def click(session, col, row):
    """One press and release, in the SGR encoding CleeCode turns on at startup. One-based."""
    session.send(f"\x1b[<0;{col + 1};{row + 1}M")
    session.send(f"\x1b[<0;{col + 1};{row + 1}m")


def drag(session, col, row, to_col):
    """Press on a cell, move to another column, release there. The SGR motion report carries the
    button bits plus 32, so a left-button drag is 32."""
    session.send(f"\x1b[<0;{col + 1};{row + 1}M")
    session.wait(lambda s: True, 0.3)
    session.send(f"\x1b[<32;{to_col + 1};{row + 1}M")
    session.wait(lambda s: True, 0.3)
    session.send(f"\x1b[<0;{to_col + 1};{row + 1}m")


def hover(session, col, row):
    """Rest the pointer on a cell without pressing anything. The SGR motion report with no button
    held is 32 + 3; CleeCode asks for any-event tracking, so these arrive."""
    session.send(f"\x1b[<35;{col + 1};{row + 1}M")
    session.wait(lambda s: True, 0.4)


# The six bands the handle is extended with, top to bottom.
#
# Every theme declares its own set — `Palette::handle_stripes` — and these are the *default*
# theme's, which is what a driver run against a fresh config directory is looking at: the 1977
# Apple rainbow. The set is pinned by a unit test in theme.rs, so what is checked here is that the
# thing the palette declares is the thing that reaches the screen, in order and unrepainted.
APPLE_STRIPES = ["61bb46", "fdb827", "f5821f", "e03a3e", "963d97", "009ddc"]


def ribbon_bands(session, col):
    """The theme's bands in column `col`, as (colour, rows) top to bottom.

    Runs, not rows, so a two-row band and a one-row band read the same and the check is about the
    order of the colours and their evenness rather than about a height this file has decided on."""
    runs = []
    for y in range(1, session.rows - 1):
        bg = session.cells(y)[col].bg
        if bg not in APPLE_STRIPES:
            continue
        if runs and runs[-1][0] == bg and runs[-1][1][-1] == y - 1:
            runs[-1][1].append(y)
        else:
            runs.append((bg, [y]))
    return runs


def ribbon_handle(session, col, mark):
    """The handle drawn in column `col`: (its rows, the background it is filled with), or None.

    The chevron's own block, which is the control — the bands around it are `ribbon_bands`. A
    block and not a run of ticks, which is what makes it read as something to press rather than as
    something the theme did to the border. Found by taking the chevron's own background and
    walking out along the column while it holds, so the check is about the filled block itself and
    not about a colour this file would have to know the name of."""
    cells = {y: session.cells(y)[col] for y in range(1, session.rows - 1)}
    middle = next((y for y, cell in cells.items() if cell.data == mark), None)
    if middle is None:
        return None
    bg = cells[middle].bg
    rows = [middle]
    y = middle - 1
    while y in cells and cells[y].bg == bg:
        rows.insert(0, y)
        y -= 1
    y = middle + 1
    while y in cells and cells[y].bg == bg:
        rows.append(y)
        y += 1
    return rows, bg


def close_cell(session):
    """Where the drawer's ✕ is drawn, as a (column, row).

    Found on screen rather than worked out from the width, so what gets clicked is the button a
    person can see. Searched from the drawer's own left border rightwards, because the terminal
    windows carry the same character in the same corner."""
    left = drawer_column(session)
    if left is None:
        return None
    for y in range(session.rows):
        at = session.full_line(y).find("✕", left)
        if at >= 0:
            return at, y
    return None


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

        # ---- the way in with a mouse ---------------------------------------------------------
        # The drawer has never been summoned in this session, so what is on the right edge is the
        # opening handle: a filled pill in the middle of one carved column, with a ‹ in it. It is
        # the only thing on screen that says the drawer exists to a hand that is not on the
        # keyboard, so it has to look like a control and not like a decoration.
        edge = session.cols - 1
        handle = ribbon_handle(session, edge, "‹")
        report.check("a closed drawer leaves a handle on the right edge",
                     handle is not None, session,
                     note="a ‹ in the last column, column %d" % edge)
        if handle:
            rows, bg = handle
            report.check("the handle is a filled pill, not a scattering of marks",
                         bg != "default" and len(rows) >= 3, session,
                         note="%d contiguous filled cells at rows %s, background %r"
                              % (len(rows), rows, bg))
            report.check("and it sits inside the main area",
                         rows[0] >= 1 and rows[-1] <= session.rows - 2, session,
                         note="never over the menu bar or the status line")

            # The bands: the 1977 rainbow, three above the block and three below, in order.
            bands = ribbon_bands(session, edge)
            report.check("the handle is extended with the theme's six colours",
                         [colour for colour, _ in bands] == APPLE_STRIPES, session,
                         note="the default theme's declared set, top to bottom: %s"
                              % ", ".join(c for c, _ in bands))
            if len(bands) == 6:
                heights = {len(band) for _, band in bands}
                report.check("the bands are even, and three sit either side of the block",
                             len(heights) == 1
                             and all(band[-1] < rows[0] for _, band in bands[:3])
                             and all(band[0] > rows[-1] for _, band in bands[3:]), session,
                             note="%d rows each, rows %s"
                                  % (heights.pop(), [band[0] for _, band in bands]))
                report.check("and the whole mark leaves the ends of the column alone",
                             bands[0][1][0] >= 2 and bands[-1][1][-1] <= session.rows - 3,
                             session, note="a row of edge above it and below it")

            # The pointer resting on it lights it. A control at the edge of the window has to
            # answer when it is about to be pressed, or nobody finds out it was a control.
            hover(session, edge, rows[len(rows) // 2])
            lit = ribbon_handle(session, edge, "‹")
            report.check("the handle lights up under the pointer",
                         lit is not None and lit[1] != bg, session,
                         note="%r at rest, %r under the pointer"
                              % (bg, lit[1] if lit else None))
            report.check("and the colours do not answer the pointer",
                         [colour for colour, _ in ribbon_bands(session, edge)] == APPLE_STRIPES,
                         session,
                         note="the bands are the mark, not the state: only the block answers")
            hover(session, 4, session.rows // 2)
            rested = ribbon_handle(session, edge, "‹")
            report.check("and goes back to itself when the pointer leaves",
                         rested is not None and rested[1] == bg, session)

            # A click on it is the mouse's Ctrl+Shift+A: the summoning half of that key and no
            # more of it. It has to win over the editor's scrollbar, which rides the right of the
            # frame beside it — the carve is what keeps the two apart.
            click(session, edge, rows[len(rows) // 2])
            opened = session.wait(lambda s: drawer_column(s) is not None, 8)
            report.check("clicking the handle opens the drawer", opened, session,
                         note="the same path the chord takes when there is nobody to talk to")
            report.check("and the launcher is what it opens on",
                         all(name in session.text() for name in INSTALLED + MISSING), session)
            report.check("the opening handle is gone while the drawer is up",
                         ribbon_handle(session, edge, "‹") is None, session,
                         note="the drawer and the way back to it are never both on screen")

        # ---- the way out, on the drawer's own edge ---------------------------------------------
        # The mirror: the same pill on the open drawer's left border, with the chevron the other
        # way round. That column is also the width seam, and the two are told apart by what the
        # hand does — so both gestures are put to it here, the drag first so the click is not
        # measured against a drawer this file has already moved.
        seam = drawer_column(session)
        handle = ribbon_handle(session, seam, "›") if seam is not None else None
        report.check("an open drawer carries a closing handle on its left edge",
                     handle is not None, session,
                     note="a › in column %s, the drawer's own border" % seam)
        if handle:
            rows, bg = handle
            report.check("the closing handle is a filled pill too",
                         bg != "default" and len(rows) >= 3, session,
                         note="%d contiguous filled cells at rows %s" % (len(rows), rows))
            report.check("and carries the same six colours",
                         [colour for colour, _ in ribbon_bands(session, seam)] == APPLE_STRIPES,
                         session,
                         note="the two handles are one mark, drawn on whichever edge applies")

            # Press, move, release: that is a resize, and it has to stay one. The seam goes right
            # (a narrower drawer) and then back, so what follows runs at the width it started at.
            grab = rows[len(rows) // 2]
            drag(session, seam, grab, seam + 8)
            moved = session.wait(lambda s: (drawer_column(s) or seam) > seam, 6)
            report.check("a press that moves on that edge still resizes the drawer", moved,
                         session,
                         note="seam at %s, now at %s" % (seam, drawer_column(session)))
            report.check("and the drawer is still open after it",
                         drawer_column(session) is not None, session,
                         note="a drag is not a click, however far it went")
            back = drawer_column(session)
            if back is not None and back != seam:
                drag(session, back, grab, seam)
                session.wait(lambda s: (drawer_column(s) or 0) <= seam + 1, 6)
                report.check("and dragging it back puts the seam where it was",
                             abs((drawer_column(session) or 0) - seam) <= 1, session,
                             note="back at %s" % drawer_column(session))

            # And a press that does *not* move, on the same cells: that is the handle.
            seam = drawer_column(session) or seam
            click(session, seam, grab)
            shut = session.wait(lambda s: drawer_column(s) is None, 6)
            report.check("a clean click on that edge closes the drawer", shut, session,
                         note="the drag begins on the first movement, so a still press is a click")
            report.check("and the frames have their columns back",
                         vertical_borders(session, middle) == borders_before, session,
                         note="%d vertical borders before it opened, %d after it closed"
                              % (borders_before, vertical_borders(session, middle)))
            report.check("and the opening handle is back on the window's edge",
                         ribbon_handle(session, edge, "‹") is not None, session)

            # Back in, so the ✕ can be put to the same question — two ways out is the point, and
            # the one in the corner is the one people already know.
            opening = ribbon_handle(session, edge, "‹")
            if opening:
                click(session, edge, opening[0][len(opening[0]) // 2])
                session.wait(lambda s: drawer_column(s) is not None, 8)
            spot = close_cell(session)
            report.check("the drawer's title bar carries a ✕ as well", spot is not None, session)
            if spot:
                click(session, *spot)
                shut = session.wait(lambda s: drawer_column(s) is None, 6)
                report.check("clicking the ✕ closes the drawer too", shut, session)

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

        # ---- the same round trip, with the mouse alone -----------------------------------------
        # The ✕ over a running agent is the one that has to be trusted: it is drawn in the cell
        # every other pane's close button lives in, and every other pane's close button ends what
        # is inside. This one hides the column and leaves the conversation going, which is the
        # only thing closing the drawer has ever meant.
        talk = "\n".join(session.frame_of("AGENT-STUB claude ready"))
        open_borders = vertical_borders(session, middle)
        spot = close_cell(session)
        report.check("the drawer wears its ✕ with an agent in it too", spot is not None, session)
        if spot and talk.strip():
            click(session, *spot)
            shut = session.wait(lambda s: drawer_column(s) is None, 6)
            report.check("the ✕ over a running agent hides the drawer", shut, session)
            report.check("and says the agent is still running in it",
                         "still running" in session.text(), session,
                         note="the View menu's own close path, not the terminal panel's")
            report.check("and the frames took the column back",
                         vertical_borders(session, middle) < open_borders, session,
                         note="%d vertical borders with the drawer, %d without it"
                              % (open_borders, vertical_borders(session, middle)))
            opening = ribbon_handle(session, session.cols - 1, "‹")
            report.check("the handle comes back when the drawer goes away",
                         opening is not None, session)
            if opening:
                rows, _ = opening
                click(session, session.cols - 1, rows[len(rows) // 2])
                returned = session.wait(lambda s: drawer_column(s) is not None, 8)
                report.check("clicking the handle brings the agent back", returned, session,
                             note="summoning, not the launcher: there is somebody in there")
                report.check("with the conversation exactly where it was",
                             "\n".join(session.frame_of("AGENT-STUB claude ready")) == talk,
                             session, note="the ✕ hid the column and never touched the pty")

            # And the closing handle over the same conversation, which is the pair of gestures a
            # hand on the mouse would actually use: out by the edge, back by the edge, and the
            # agent none the wiser.
            closing = ribbon_handle(session, drawer_column(session), "›")
            report.check("the closing handle is there over a running agent too",
                         closing is not None, session)
            if closing:
                rows, _ = closing
                grab = rows[len(rows) // 2]
                click(session, drawer_column(session), grab)
                gone = session.wait(lambda s: drawer_column(s) is None, 6)
                report.check("a clean click on it hides the drawer", gone, session)
                report.check("and says the agent is still running in it",
                             "still running" in session.text(), session)
                opening = ribbon_handle(session, session.cols - 1, "‹")
                if opening:
                    rows, _ = opening
                    click(session, session.cols - 1, rows[len(rows) // 2])
                    session.wait(lambda s: drawer_column(s) is not None, 8)
                    report.check("and the conversation comes back untouched",
                                 "\n".join(session.frame_of("AGENT-STUB claude ready")) == talk,
                                 session,
                                 note="the edge closes the column, never the pty behind it")

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
