#!/usr/bin/env python3
"""Summon the agent drawer and check it is a drawer rather than a fourth terminal.

    python3 scripts/drive_drawer.py [path/to/clee]

The drawer's promises are all about *where* it lives, so every check here is about the screen and
about what survives: does `Ctrl+Shift+A` open it when there is nobody to talk to, does the column
come out of the layout rather than sit over the top of it, does the agent's output keep arriving
after the first frame, does the conversation survive a workspace switch and being hidden, and does
an agent that ends leave the list of four rather than a shell wearing an agent's frame.

Then the column with several agents in it, one to a tab. `Ctrl+Shift+T` — or the View menu — puts
the same full-pane launcher back over the ones already running, `Esc` hands the column straight
back to them, and a name chosen off it arrives as another tab rather than in place of the first.
The border stops carrying a title then and carries a strip of chips instead, so the drawer is
found by both shapes; the window's own ■ still hides the column while a chip's ■ ends the agent
in that chip. The case worth having is the same program twice — two tabs both reading
"Claude Code" — which is why nothing in that section tells the two apart by their names.

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
# Left off the PATH on purpose: the launcher has to show these too, say why it cannot run them,
# and offer to install them. The empty drawer is where you find out what CleeCode knows about.
MISSING = ["opencode", "gemini"]

# What the launcher types at a shell prompt for an agent that is not here — `drawer::
# install_command`, the not-Windows arm, which is the one a run of this file is looking at. Copied
# rather than derived because a check that reads the answer out of the thing it is checking checks
# nothing: if these two drift, this file is supposed to fail.
INSTALL_COMMANDS = {
    "claude": "curl -fsSL https://claude.ai/install.sh | bash",
    "opencode": "curl -fsSL https://opencode.ai/install | bash",
    "codex": "npm install -g @openai/codex",
    "gemini": "npm install -g @google/gemini-cli",
}

# How tall a mark is, in cells — `drawer::ART_ROWS`. The selection frame adds a row above and one
# below, so an entry is ART_ROWS + 2 tall on screen.
ART_ROWS = 5

# The brand colours each mark is drawn in. Fixed values rather than palette roles, exactly as the
# file tree's icons are, which is what makes them nameable here at all: a mark that changed colour
# with the theme would not be that program's mark any more. There are no captions under the big
# marks any more — each carries its name in brick letters — so the first ink of each entry is
# also how this file *finds* that mark on screen: a colour no other mark uses.
MARK_INKS = {
    "claude": ["d97757", "000000"],   # Clawd's coral, and the black of its eyes
    "opencode": ["b4b2b2", "efeded"], # "open" in their grey, "code" in their white
    "codex": ["ffffff"],              # the prompt in the cloud
    "gemini": ["1b80fd", "d7618e"],   # the border's blue, and the pink it arrives at
}
# Codex's cloud is not one colour: it runs lavender at the top into blue at the bottom, and the
# check is that the two ends are the two ends.
CODEX_TOP, CODEX_BOTTOM = "a9a6ff", "3e49ff"

# The ink each mark's *top row* carries, which is how a mark is found on screen — see
# `mark_rows`. opencode's word starts a row into its block (the letters sit on the middle six
# pixel rows), so its "top" is the word's own first row; the others are found at their first.
MARK_TOPS = {
    "claude": "d97757",
    "opencode": "b4b2b2",
    "codex": CODEX_TOP,
    "gemini": "1b80fd",
}

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


# The words the drawer's own top border can carry: the launcher's title, and the label of
# whichever agent is in the pane \u2014 which is also what a tab chip is named after. `Agent::label`,
# copied here for the reason the install commands are: a check that reads the answer out of the
# thing it is checking checks nothing.
DRAWER_TITLES = (" agent ", " Claude Code ", " codex ", " opencode ", " gemini ")


def drawer_column(session):
    """The column the drawer's left border sits at, found from what is written on its own border.

    Read off the screen rather than computed from the percentage, so a check about the layout is
    a check about the layout and not about this file's arithmetic. It has to be the *border*: the
    status line says "the agent is still running in it" the moment the drawer is hidden, and a
    plain search for the word found that sentence and reported the column still there.

    Three forms, and the third is why this is not one `find`. The drawer is always closable \u2014
    launcher or agent, it wears the same close box the terminal windows do \u2014 so with one tab its
    border reads "\u250c \u25a0 \u2500 agent": corner, a padded close box, a padded dash, then the title,
    rather than the bare "\u250c agent" a frame with no close box still carries (the Files panel,
    which nothing ever closes). With *two or more* tabs there is no title at all: the chips ride
    the border in its place, so it reads "\u250c \u25a0  \u25a0 Claude Code  \u25a0 codex \u2500\u2500\u2500" \u2014 the window's own
    box still in its cell, then the strip four columns in, each chip a padded \u25a0 and a name.

    Every form is anchored on the corner and nothing looser, because a title alone is also the
    sentence a status message uses when it names the agent running behind a hidden drawer. The
    chip form is anchored on an *agent's* name for the same kind of reason: a terminal window with
    two shells in it draws exactly this strip, and only the drawer's chips are named after agents.
    """
    boxed = "\u250c \u25a0 \u2500"    # corner, the close box, the dash before the title
    stripped = "\u250c \u25a0  \u25a0"  # corner, the close box and its pad, then the first chip's own box
    for y in range(session.rows):
        line = session.full_line(y)
        for title in DRAWER_TITLES:
            at = line.find(title)
            if at <= 0:
                continue
            if line[at - 1] == "\u250c":
                return at - 1
            if at >= len(boxed) and line[at - len(boxed):at] == boxed:
                return at - len(boxed)
            if at >= len(stripped) and line[at - len(stripped):at] == stripped:
                return at - len(stripped)
    return None


def drawer_border_row(session, left=None):
    """The screen row the drawer's top border is drawn on, or None."""
    left = drawer_column(session) if left is None else left
    if left is None:
        return None
    return next((y for y in range(session.rows) if session.full_line(y)[left] == "\u250c"), None)


def drawer_chips(session):
    """The tab chips on the drawer's top border, left to right \u2014 empty at one tab, or none.

    One dict per chip: where it starts and ends, the column of its own \u25a0, the name written in it,
    and the colour it is filled with, which is how the active tab says it is the active one.

    Found from the \u25a0s along the border rather than from the width of a label, because the chips
    are what the mouse has to hit: `terminal_tab_ranges` puts each box one cell into its chip, and
    a check that clicked a column computed here from a label's length would be checking this
    file's arithmetic rather than the strip. The search starts at the drawer's own left border
    plus four \u2014 the window's \u25a0 sits at plus two, with a pad either side, and it is not a chip."""
    left = drawer_column(session)
    row = drawer_border_row(session, left)
    if left is None or row is None:
        return []
    line = session.full_line(row)
    boxes = [x for x in range(left + 4, session.cols) if line[x] == "\u25a0"]
    chips = []
    for i, box in enumerate(boxes):
        if i + 1 < len(boxes):
            end = boxes[i + 1] - 1
        else:
            # The last chip runs to wherever the border's own fill takes the row back.
            end = next((x for x in range(box, session.cols) if line[x] == "\u2500"), session.cols)
        chips.append({
            "start": box - 1,
            "end": end,
            "close": box,
            "label": line[box + 2:end].strip(),
            "fill": session.cells(row)[box].bg,
        })
    return chips


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
    """Where the drawer's ■ close box is drawn, as a (column, row).

    Found on screen rather than worked out from the width, so what gets clicked is the button a
    person can see. Searched from the drawer's own left border rightwards, because every closable
    frame carries the same glyph in the same corner — the drawer's box is not a control of its
    own, it is `with_close_box` drawn on the drawer's pane exactly as it is on a terminal
    window's, and a plain search for the character would find the nearest pane to the left of it
    just as readily."""
    left = drawer_column(session)
    if left is None:
        return None
    for y in range(session.rows):
        at = session.full_line(y).find("■", left)
        if at >= 0:
            return at, y
    return None


def row_inks(session, y, left):
    """Every colour on row `y` right of column `left` — foregrounds and backgrounds both, because
    a half-block cell whose two halves are different colours carries the lower one as its
    background. That is not an implementation detail to be stepped around here: it is the only
    way a colour can change *inside* a row of a terminal, and it is what draws Codex's gradient."""
    inks = set()
    for cell in session.cells(y)[left + 1:]:
        inks.update(c for c in (cell.fg, cell.bg) if c != "default")
    return inks


def mark_rows(session, name):
    """The rows `name`'s mark is drawn on — its top row found by colour, ART_ROWS from there.

    By colour and not by text, because the big marks carry no caption: the name beside each
    mascot is itself drawn in the mark's own inks, brick letters rather than glyphs, so the one
    thing on screen that says "this row is claude's" is a colour no other mark uses. The ink
    looked for is one the mark's *top* row carries — Codex's is the lavender end of the gradient,
    which exists nowhere else on screen precisely because every other row has already mixed away
    from it. Pinned inside the drawer — right of its left border — for the reason the old caption
    search was pinned to a column: an agent's own output in a terminal pane satisfies any check a
    plain text search can make."""
    left = drawer_column(session)
    if left is None:
        return None
    top_ink = MARK_TOPS[name]
    for y in range(session.rows):
        if top_ink in row_inks(session, y, left):
            return list(range(y, min(y + ART_ROWS, session.rows)))
    return None


def mark_inks(session, name):
    """Every colour used in the rows of `name`'s mark, one set per row, top to bottom."""
    left = drawer_column(session)
    rows = mark_rows(session, name)
    if left is None or rows is None:
        return None
    return [row_inks(session, y, left) for y in rows]


def frame_rows(session):
    """The rows carrying the selection frame's corners, top to bottom, or None.

    The frame is the launcher's answer to "what am I about to start": a ring around the chosen
    mark. Its corner glyphs are the one thing on a launcher screen drawn *inside* the drawer that
    is box-drawing but not the drawer's own border, which sits exactly on `drawer_column` and is
    excluded by starting one column in.

    Both corners, not only the top one. The ring is drawn around the mark, so the mark's rows are
    *between* them — a list holding only the top row makes `ring[0] <= mark[0] <= ring[-1]` a
    question that cannot be answered yes, and `highlight_agent` then walked away from the very
    name it had arrived at. The drawer's own bottom-left corner is at `left` exactly and is
    excluded with the top one."""
    left = drawer_column(session)
    if left is None:
        return None
    rows = [
        y for y in range(session.rows)
        if any(corner in session.full_line(y)[left + 1:] for corner in ("┌", "└"))
    ]
    return rows or None


def focus_launcher(session):
    """Give the open launcher the keyboard, by cycling the frames to it with Ctrl+Tab.

    Not with Ctrl+Shift+A: that key opens the drawer only when there is no agent to talk to, and
    once one is running in a pane it hands *it* the context instead — so pressing it here would
    send a reference to that agent and leave the launcher untouched. The drawer is the last frame
    in the cycle; focus is confirmed by the selection ring answering an arrow, and the arrow is
    stepped back so the ring is where it was."""
    for _ in range(6):
        before = frame_rows(session)
        if before:
            session.press("\x1b[B", lambda s: True, 0.4)          # Down
            if frame_rows(session) != before:
                session.press("\x1b[A", lambda s: True, 0.3)      # and back, ring restored
                return True
        session.press("\x1b[9;5u", lambda s: True, 0.5)           # Ctrl+Tab, next frame
    return False


def highlight_agent(session, name):
    """Walk the launcher's selection ring onto `name`'s mark, and say whether it got there.

    The ring does not always stay where it was left: reopening the launcher — with an agent now
    running in a pane, so the key that opens it is no longer the same gesture as before — can put
    it back on the first agent. A check that pressed Enter trusting the ring to still be on the
    name it clicked a moment ago would then start whichever agent the ring had drifted to, so the
    ring is put where it is wanted rather than assumed. Measured against the mark on screen, the
    same way every walk in this file is."""
    for _ in range(6):
        ring = frame_rows(session)
        mark = mark_rows(session, name)
        if not ring or not mark:
            return False
        if ring[0] <= mark[0] <= ring[-1]:
            return True
        session.press("\x1b[B" if ring[0] < mark[0] else "\x1b[A", lambda s: True, 0.4)
    ring, mark = frame_rows(session), mark_rows(session, name)
    return bool(ring and mark and ring[0] <= mark[0] <= ring[-1])


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


def prompt_line_with(session, command):
    """The shell-prompt line carrying `command`, or None — never the status line.

    Two lines on screen name the install command: the shell prompt it was typed at, `$ curl …`,
    and the status line's sentence about it, `opencode is not installed — `curl …``. Only the
    first is what "sitting at a prompt" and "taken back off the prompt" are about, and the two
    are told apart by what sits before the command — a `$` for the shell, a sentence for the
    status line — and by the status line never being anywhere but the last row. Keying the waits
    on the shell line and not on the command being *anywhere* in the text is the difference
    between a check that watches the shell and one the status line answers for free, before the
    shell has drawn a thing."""
    for y in range(session.rows - 1):   # the status line is the last row; never it
        line = session.full_line(y)
        if command in line and "$" in line.split(command)[0]:
            return line
    return None


def start_agent_in_terminal(session, name):
    """Type an agent's name at a shell and wait for its stub to answer, retried if the first
    line is lost to the pane's startup scrub.

    A pane clears its shell's startup banner once, with a form feed, the moment the shell is
    first reading keys — which on a slow machine can be exactly as this types the name. A shell
    with readline turns that form feed into a redraw and no harm is done; `/bin/sh` on Linux is
    dash, which has none, so the tty keeps it as a literal `^L` at the head of the line and the
    line submits as `^Lcodex` — not a command. The scrub only ever fires once, so a second
    attempt is clean; the line is cleared first each time so nothing is glued onto anything."""
    for _ in range(3):
        focus_terminal(session)
        session.send("\x15")                      # kill any scrub residue already on the line
        session.send(name + "\r")
        if session.wait(lambda s: "AGENT-STUB %s ready" % name in s.text(), 15):
            return True
    return False


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

    # Wider and taller than the default stage: the launcher's marks carry their names in bricks
    # now, and four framed banners with their spacing need the rows a default 30 does not have.
    session = Session(binary, root, env=env, cols=190, rows=42)
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
            # By the marks' own inks, not by text: the names are drawn in brick letters, which a
            # text dump reads as half-blocks, not words.
            report.check("and the launcher is what it opens on",
                         all(mark_rows(session, name) is not None
                             for name in INSTALLED + MISSING), session)
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

            # Back in, so the ■ can be put to the same question — two ways out is the point, and
            # the one in the corner is the one people already know.
            opening = ribbon_handle(session, edge, "‹")
            if opening:
                click(session, edge, opening[0][len(opening[0]) // 2])
                session.wait(lambda s: drawer_column(s) is not None, 8)
            spot = close_cell(session)
            report.check("the drawer's title bar carries a ■ as well", spot is not None, session)
            if spot:
                click(session, *spot)
                shut = session.wait(lambda s: drawer_column(s) is None, 6)
                report.check("clicking the ■ closes the drawer too", shut, session)

        # ---- summoning ---------------------------------------------------------------------
        # There is no agent anywhere, so the key that hands an agent the context has nobody to
        # hand it to. It opens the panel whose job that is instead of reporting a dead end.
        summoned = session.press(
            session.chord("a"), lambda s: drawer_column(s) is not None, 8)
        report.check("Ctrl+Shift+A with no agent anywhere opens the drawer", summoned, session,
                     note="the key summons the panel when there is nobody to talk to")
        if not summoned:
            return

        names = [n for n in INSTALLED + MISSING if mark_rows(session, n) is not None]
        report.check("the launcher shows all four agents", len(names) == 4, session,
                     note="found %s" % names)

        # ---- the marks --------------------------------------------------------------------
        # Each entry is that program's own mark drawn in cells, its name in brick letters beside
        # the mascot — there is no caption under a banner that already says who it is. These
        # checks are that each entry is the *right* mark, told apart by the only property a
        # driver can read at this resolution — the colours, which are the owners' own and fixed.
        for name, inks in MARK_INKS.items():
            drawn = mark_inks(session, name)
            report.check("%s's mark is drawn in the launcher" % name, drawn is not None, session,
                         note="five rows of half-blocks, name in bricks beside the mascot")
            if drawn is None:
                continue
            everywhere = set().union(*drawn)
            report.check("and it is drawn in %s's own colours" % name,
                         all(ink in everywhere for ink in inks), session,
                         note="wanted %s, found %s" % (inks, sorted(everywhere)))
            # And nobody else's: four marks in a column that shared a colour would be four
            # decorations rather than four marks.
            others = [other for other in INSTALLED + MISSING if other != name]
            elsewhere = set()
            for other in others:
                for row in mark_inks(session, other) or []:
                    elsewhere |= row
            report.check("and that colour belongs to it alone",
                         not (set(inks) & elsewhere), session,
                         note="%s also appears above %s" % (set(inks) & elsewhere, others))

        # Codex's is the one that is not a colour but a run of them: lavender at the top of the
        # cloud into blue at the bottom. A flat mark would satisfy every check above and be the
        # wrong picture, so the two ends are asked for by name.
        cloud = mark_inks(session, "codex")
        report.check("codex's cloud is drawn in the launcher", cloud is not None, session)
        if cloud:
            report.check("and it runs lavender at the top into blue at the bottom",
                         CODEX_TOP in cloud[0] and CODEX_BOTTOM in cloud[-1], session,
                         note="top row %s, bottom row %s"
                              % (sorted(cloud[0]), sorted(cloud[-1])))
            report.check("which is a gradient and not two halves",
                         len(set().union(*cloud)) > len(cloud), session,
                         note="%d colours down %d rows: it changes inside a row as well, which is "
                              "what the half-block's background is for"
                              % (len(set().union(*cloud)), len(cloud)))

        # ---- the selection frame ------------------------------------------------------------
        # The launcher's highlight is a ring around the chosen mark — the marks carry their own
        # names, so the frame's only job is to be seen, and to move when the choice does.
        ringed = frame_rows(session)
        report.check("a frame rings the chosen mark", ringed is not None, session,
                     note="box-drawing inside the drawer that is not the drawer's own border")
        if ringed:
            was = ringed[0]
            session.press("\x1b[B", lambda s: (frame_rows(s) or [was])[0] != was, 4)
            moved = frame_rows(session)
            report.check("and the frame follows the arrows",
                         moved is not None and moved[0] != was, session,
                         note="top of the ring went from row %s to %s"
                              % (was, moved and moved[0]))
            session.press("\x1b[A", lambda s: (frame_rows(s) or [None])[0] == was, 4)

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
            # The mark itself is dimmed with the terminal's own DIM attribute, which pyte does
            # not surface — so what this checks is the half it can see: the honest phrase, in
            # words, sitting under the banner of the agent it is about and not somewhere vague.
            said = session.row_of("not installed")
            rows = mark_rows(session, absent)
            report.check("an agent that is not installed says so under its own banner",
                         said is not None and rows is not None
                         and rows[0] <= said <= rows[-1] + 2, session,
                         note="the phrase is on row %s, %s's mark on rows %s"
                              % (said, absent, rows))

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

        # ---- the drawer holds several agents, one per tab -------------------------------------
        # Everything from here to the end of this section runs on the stub started above, so it is
        # gated exactly as the rest of the file is: nothing below the `if not ready: return` a few
        # lines up runs on a machine where the launcher could not start anything.
        #
        # The interesting case is the same program twice. Two claude tabs is what a person actually
        # ends up with — one agent on the bug and one on the branch — and it is the case a strip
        # naming agents rather than tabs has to survive: both chips read "Claude Code", so nothing
        # here may tell them apart by their labels. They are told apart the only honest way, by
        # what has been typed at each one's prompt.
        talk = "\n".join(session.frame_of("AGENT-STUB claude ready"))
        first, second, third = "TAB_ONE", "TAB_TWO", "TAB_THREE"

        session.send(session.chord("t"))
        chose = session.wait(
            lambda s: drawer_column(s) is not None
            and all(mark_rows(s, name) is not None for name in INSTALLED + MISSING), 8)
        report.check("Ctrl+Shift+T in the drawer puts the four names back over the running agent",
                     chose, session,
                     note="another tab here is a choice, not a spawn: the launcher is the door")
        report.check("and the conversation is behind it rather than ended",
                     "AGENT-STUB claude ready" not in session.text(), session,
                     note="the launcher takes the whole column, so the pane is hidden, not closed")

        session.press("\x1b", lambda s: "AGENT-STUB claude ready" in s.text(), 8)
        report.check("Esc hands the column straight back to the agent",
                     "\n".join(session.frame_of("AGENT-STUB claude ready")) == talk, session,
                     note="the same pty, byte for byte: a cancelled choice touched nothing")
        report.check("and the drawer is still a column of its own",
                     drawer_column(session) is not None, session,
                     note="cancelling a choice must not cost the panel it was made in")
        # Where the next keystroke lands is the only definition of focus that matters — and the
        # marker it leaves is how this section tells the first tab from the second one later.
        session.send(first)
        typed_here = session.wait(
            lambda s: first in "\n".join(s.frame_of("AGENT-STUB claude ready")), 8)
        report.check("and the keyboard never left the drawer either", typed_here, session,
                     note="Esc cancelled the launcher, not the frame the launcher was in")

        # The other door to the same screen, since the chord only works from inside the column.
        if menu_action(session, "v", "New agent tab...", report, "the View menu"):
            report.check("the View menu's New agent tab reaches the launcher too",
                         session.wait(lambda s: mark_rows(s, "gemini") is not None, 8), session,
                         note="the chord needs the keyboard to be in the drawer; this does not")
        two = False
        if focus_launcher(session) and highlight_agent(session, "claude"):
            session.send("\r")
            two = session.wait(
                lambda s: "AGENT-STUB claude ready" in s.text() and first not in s.text(), 30)
        report.check("choosing a second agent starts it beside the first, not over it", two,
                     session, note="the same program twice is allowed, and is the ordinary case")
        if two:
            session.send(second)
            session.wait(lambda s: second in "\n".join(
                s.frame_of("AGENT-STUB claude ready")), 8)

        chips = drawer_chips(session)
        report.check("two tabs put a strip of chips on the border where the title was",
                     len(chips) == 2, session,
                     note="found %s" % [c["label"] for c in chips])
        if len(chips) == 2:
            left = drawer_column(session)
            report.check("the window's own ■ keeps its cell, and the chips start after it",
                         close_cell(session) == (left + 2, drawer_border_row(session, left))
                         and chips[0]["start"] == left + 4, session,
                         note="box at %s, first chip at %s" % (left + 2, chips[0]["start"]))
            report.check("each chip is its agent's name with a ■ of its own",
                         [c["label"] for c in chips] == ["Claude Code", "Claude Code"]
                         and len({c["close"] for c in chips}) == 2
                         and left + 2 not in {c["close"] for c in chips}, session,
                         note="boxes at %s" % [c["close"] for c in chips])
            report.check("and one of the two is lit as the tab on screen",
                         chips[0]["fill"] != chips[1]["fill"], session,
                         note="%s and %s" % (chips[0]["fill"], chips[1]["fill"]))

            lit = [c["fill"] for c in chips]
            session.press("\x1b[1;6D", lambda s: first in s.text(), 8)   # Ctrl+Shift+←
            report.check("Ctrl+Shift+← in the drawer shows the tab beside it",
                         first in session.text() and second not in session.text(), session,
                         note="told apart by what was typed at each prompt, not by their names")
            report.check("and the lit chip moved with it",
                         [c["fill"] for c in drawer_chips(session)] == list(reversed(lit)),
                         session, note="the strip says which conversation is on screen")
            session.press("\x1b[1;6C", lambda s: second in s.text(), 8)  # Ctrl+Shift+→
            report.check("and Ctrl+Shift+→ comes back to the one it left",
                         second in session.text() and first not in session.text(), session)

            # An agent ending by itself with another still talking. Ctrl+U first, for the reason
            # the section at the end of this file gives: end-of-file on a half-typed line is not
            # end of anything.
            session.press("\x15", lambda s: second not in s.text(), 6)
            session.send("\x04")
            survived = session.wait(
                lambda s: first in s.text() and drawer_column(s) is not None, 20)
            report.check("an agent that ends with another still talking leaves the rest running",
                         survived, session,
                         note="the drawer does not go back to the list while a conversation is on")
            ended = "Claude Code in the drawer has ended — its tab is gone"
            report.check("and the status line says which one went",
                         ended in session.full_line(session.rows - 1), session,
                         note=session.full_line(session.rows - 1).strip()[:110])
            report.check("and the border goes back to carrying a title",
                         drawer_chips(session) == [] and drawer_column(session) is not None,
                         session, note="one tab is a name on a border, not a strip of one chip")

        # A third, so the chip's own ■ has two to choose between. It arrives active, which makes
        # the chip pressed below the *background* one — a control that killed whatever was on
        # screen instead of the tab it is drawn on would pass a check made on the active tab.
        third_up = False
        if session.wait(lambda s: drawer_column(s) is not None, 4):
            session.send(session.chord("t"))
            if session.wait(lambda s: mark_rows(s, "gemini") is not None, 8) \
                    and focus_launcher(session) and highlight_agent(session, "claude"):
                session.send("\r")
                third_up = session.wait(
                    lambda s: len(drawer_chips(s)) == 2 and first not in s.text(), 30)
                if third_up:
                    session.send(third)
                    session.wait(lambda s: third in s.text(), 8)
        report.check("a third agent joins the strip the same way", third_up, session)
        if third_up:
            chips = drawer_chips(session)
            click(session, chips[0]["close"], drawer_border_row(session))
            killed = session.wait(lambda s: len(drawer_chips(s)) == 0, 10)
            report.check("a chip's ■ closes that tab and no other", killed, session,
                         note="the window's ■ hides the column; a chip's ends the agent in it")
            report.check("and the conversation the chip did not name is still going",
                         third in session.text() and first not in session.text(), session)
            session.send(session.chord("k"))
            back = session.wait(
                lambda s: all(mark_rows(s, name) is not None for name in INSTALLED + MISSING), 15)
            report.check("Ctrl+Shift+K closes the last tab and the launcher comes back", back,
                         session, note="never a respawned shell, here no more than anywhere else")

        # And an agent again for the rest of the file, which is written for a drawer with one.
        if session.wait(lambda s: frame_rows(s) is not None, 4):
            if focus_launcher(session):
                highlight_agent(session, "claude")
                session.send("\r")
        restarted = session.wait(lambda s: "AGENT-STUB claude ready" in s.text(), 30)
        report.check("and the launcher starts one again for the checks that follow", restarted,
                     session)
        if not restarted:
            return

        # ---- precedence ----------------------------------------------------------------------
        # A second agent, in an ordinary terminal, started by typing its name. Ctrl+Shift+A now
        # has two to choose between, and the drawer is the one it means.
        other = start_agent_in_terminal(session, "codex")
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
        # The ■ over a running agent is the one that has to be trusted: it is drawn in the cell
        # every other pane's close button lives in, and every other pane's close button ends what
        # is inside. This one hides the column and leaves the conversation going, which is the
        # only thing closing the drawer has ever meant.
        talk = "\n".join(session.frame_of("AGENT-STUB claude ready"))
        open_borders = vertical_borders(session, middle)
        spot = close_cell(session)
        report.check("the drawer wears its ■ with an agent in it too", spot is not None, session)
        if spot and talk.strip():
            click(session, *spot)
            shut = session.wait(lambda s: drawer_column(s) is None, 6)
            report.check("the ■ over a running agent hides the drawer", shut, session)
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
                             session, note="the ■ hid the column and never touched the pty")

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
            still_four = all(mark_rows(session, name) is not None
                             for name in INSTALLED + MISSING)
            report.check("and the four marks are on offer again", still_four, session)

        # ---- a name that is not here offers to install it -----------------------------------
        # Last, because it is the only section that leaves the keyboard somewhere else on purpose:
        # the command goes to a shell and the focus follows it, since the Enter this deliberately
        # did not press has to land where the line is.
        #
        # Skipped on a machine that really has both, for the same reason the dim check above is:
        # CleeCode finds an agent installed on this laptop however the driver arranges PATH, and
        # then offering to install it would be the bug.
        if absent is None:
            print("  SKIP  the install offer: this machine really has %s" % ", ".join(MISSING))
        elif gone:
            command = INSTALL_COMMANDS[absent]
            rows = mark_rows(session, absent)
            row = rows[len(rows) // 2] if rows else None
            report.check("the launcher still lists %s to press" % absent, row is not None, session)
            for how in ("clicking", "Enter"):
                if row is None:
                    break
                if how == "clicking":
                    click(session, drawer_column(session) + 4, row)
                else:
                    # Enter is the other hand making the same gesture the click did — the point of
                    # checking both. The launcher is still open from the click; what it needs is
                    # the keyboard, and *not* by way of Ctrl+Shift+A: with the agent from the
                    # precedence check still running in a pane, that key sends it the context
                    # rather than opening anything, and the Enter would land in that agent's
                    # prompt. So the keyboard is walked to the drawer through the frame cycle
                    # instead, the ring put back on this name, and then Enter.
                    if not focus_launcher(session):
                        report.check("the launcher takes the keyboard for Enter", False, session)
                        break
                    highlight_agent(session, absent)
                    session.send("\r")
                # Waited on the shell prompt drawing the command, not on the command being
                # anywhere in the text: the status line names it the instant the key is pressed,
                # so `command in text` passes before the shell — freshly opened when the agents
                # are running in the other panes — has caught up and echoed a thing. On a slow
                # machine that gap is real, and keying on it is what makes this watch the shell.
                typed = session.wait(lambda s: prompt_line_with(s, command) is not None, 15)
                report.check("%s an agent that is not installed types its install command"
                             % how, typed, session, note=command)
                if not typed:
                    break
                line = prompt_line_with(session, command)
                at = line.index(command)
                seam = drawer_column(session)
                report.check("and types it into a shell, not into the drawer",
                             seam is None or at < seam, session,
                             note="column %s, drawer border at %s — the drawer is the agent's "
                                  "home and the agent is the thing that is missing" % (at, seam))
                report.check("and it is sitting at a prompt, unsent",
                             "$" in line.split(command)[0], session,
                             note="the shell's own prompt is still on the line: %r"
                                  % line.strip()[:70])
                report.check("nothing was submitted", "SUBMITTED" not in session.text(), session)
                status = session.full_line(session.rows - 1)
                report.check("and the status line says what happened and why",
                             absent in status and command in status, session,
                             note=status.strip()[:110])
                # Disarmed before anything else in this file can press Enter. A driver that walks
                # away leaving `curl … | bash` on a live prompt is a driver that installs things.
                # Cleared off the shell line, which is the prompt — the status line goes on naming
                # the command in its sentence, and waiting for it to leave *there* would wait for
                # ever.
                session.press("\x15", lambda s: prompt_line_with(s, command) is None, 5)
                report.check("and the line can be taken back off the prompt",
                             prompt_line_with(session, command) is None, session,
                             note="typed is not run: nothing here ever became a command")
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
