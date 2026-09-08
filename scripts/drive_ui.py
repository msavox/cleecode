#!/usr/bin/env python3
"""Drive the real CleeCode binary in a pseudo-terminal and check the completion popup.

    python3 scripts/drive_ui.py [path/to/clee]      # default: target/debug/clee

Why this exists. `cargo test` covers the pure functions — ranking, key routing, hit-testing,
where a box goes — but it cannot build an `App`: constructing one spawns two real PTYs and reads
the user's own settings from disk, so a test that did it would depend on the machine it ran on.
That leaves the wiring between those functions untested, and the wiring is where a feature is
either usable or not. This runs the shipped binary the way a person does.

Needs pyte (`pip install pyte`) and a Unix pty, so macOS and Linux. Not wired into `cargo test`
or CI: it waits on conditions with timeouts, and a flaky gate is worse than an honest manual one.

The machinery is in pty_drive.py beside this file, along with the two traps worth knowing about
before adding a check.
"""

import os
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_drive import Report, Session, binary_from_argv  # noqa: E402

SAMPLE = """fn configure_pipeline() {
    let config_path = 1;
    let configuration = 2;
    let other = 3;
}
"""

# Three lines of equal shape, so a column written down them is obvious in the dump when a check
# fails, and a fourth that is too short to reach the column the others share.
COLUMNS = """one alpha
two beta
ab
three gamma
"""

# Real \r\n bytes, written binary so Python never normalises them away before the editor gets a
# chance to detect them.
CRLF_TEXT = b"one\r\ntwo\r\nthree\r\n"

# The two halves of the large-file check, in a language with keywords and colours and no
# language server in `lsp::SERVERS` — so the check is about the mode and not about whatever
# happens to be installed on the machine running it.
#
# `configuration_marker` lives only in the large file and `counter_local` only in the small one,
# so a completion popup says plainly which buffer its words came from.
LARGE_LINE = "    int configuration_marker = 1; // padding padding padding padding pad\n"
SMALL_JAVA = """class Sample {
    int counter_local = 1;
}
"""


def completion(session, report):
    """The word-completion popup, from opening it to undoing what it wrote."""
    report.check("the file opens", session.wait(lambda s: "config_path" in s.text(), 8), session)

    # To the end of the file, then a fresh line to type on.
    session.press("\x1b[B" * 4, lambda s: True, 1)
    session.press("\x1b[F", lambda s: True, 1)          # End
    session.press("\r", lambda s: True, 1)

    report.check("two letters are enough to open it",
                 session.press("co", lambda s: s.popup_open()), session)
    report.check("keywords of the language are offered", "const" in session.text(), session,
                 note="sample.rs, so rust")
    # From the popup's own rows, not from the screen: the editor is showing
    # `let config_path = 1;`, so a whole-screen search was answered by the buffer the popup is
    # supposed to be reading — the check would have passed with the feature deleted. The
    # keyword check beside it is genuine, because `const` is written nowhere in the sample.
    report.check("words from the buffer are offered too",
                 "config_path" in session.popup_words(), session,
                 note=str(session.popup_words()))

    session.press("nf", lambda s: "const" not in s.text())
    report.check("typing narrows the list", session.popup_open() and "const" not in session.text(),
                 session, note="const cannot start with conf")
    words = session.popup_words()
    report.check("all three matching words survive", len(words) == 3, session, note=str(words))
    report.check("the word nearest the cursor is picked", words[:1] == ["configuration"], session,
                 note="the closest of the three to where the cursor is")

    report.check("Esc closes the popup",
                 session.press("\x1b", lambda s: not s.popup_open()), session)
    report.check("Esc left the typed text alone", session.buffer_line(6) == "conf", session,
                 note=repr(session.buffer_line(6)))
    report.check("typing reopens it", session.press("ig", lambda s: s.popup_open()), session)

    session.press("\t", lambda s: not s.popup_open())
    accepted = session.buffer_line(6)
    report.check("Tab accepts the word into the buffer",
                 accepted in ("config_path", "configuration", "configure_pipeline"),
                 session, note=repr(accepted))
    report.check("the popup is gone after accepting", not session.popup_open(), session)
    report.check("one undo step puts back what was typed",
                 session.press("\x1a", lambda s: s.buffer_line(6) == "config"),
                 session, note=repr(session.buffer_line(6)))


def no_file_open(session):
    """Whether the editor frame is showing the nothing-open state, in either language."""
    return "No file open" in session.text() or "Nessun file aperto" in session.text()


def quick_open_up(session):
    """Whether the quick-open box itself is on screen — read off the box, not off the tree.

    Its filter line is a `>` prompt hard against the box's own left border, which is a thing only
    an overlay draws: the border character is the box's, and nothing behind it puts one there.
    That matters because the file tree stays visible underneath the box — with a file open and
    with nothing open alike — so waiting for a project filename to appear in the screen text is
    answered by the tree the instant Ctrl+O is pressed, and would be answered by it just the same
    if the box never came up at all. The tree's names are already on screen before the key is
    sent; the only honest question is whether the overlay is."""
    return "│> " in session.text()


def quick_open_offers(session, typed, name):
    """Whether the box has been told `typed` and is sitting with `name` under its caret.

    Both halves, because both are things Enter depends on and neither implies the other. The
    filter line carrying the typed text says the keystrokes reached the picker and the list has
    been rebuilt around them; the `▶` against the border says the walk that fills the list has
    returned, the filter has been applied to it, and the row Enter would take is the wanted one.
    Waiting on the clock instead — type, sleep, Enter — is what put this driver's CI failures on
    a slow runner: Enter landed on a list that had not been filled yet, the box closed having
    opened nothing, and four checks downstream reported on an editor that had never been asked
    to do anything."""
    return "│> " + typed in session.text() and "│▶ " + name in session.text()


def closing_the_last_tab(binary, root, report):
    """Closing every tab has to leave nothing open, not a fresh untitled buffer.

    It used to put an identical untitled buffer in the last tab's place, which made that tab the
    one tab you could not close: you asked for it to go and something the same took its seat.

    Its own session, because Ctrl+D belongs to the editor and the editor has to have the focus —
    which it gets by opening a file, and that wants a buffer nobody has typed in. What matters
    here as much as the frame saying "no file open" is that the app goes on drawing afterwards:
    every frame reaches for the current editor, and there is no longer one to reach for."""
    session = Session(binary, root)
    try:
        if not session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            report.check("the second session starts", False, session)
            return
        session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 10)
        session.send("\x0f")                            # Ctrl+O, quick-open — and the focus
        session.wait(quick_open_up, 8)
        session.send("sample")
        session.wait(lambda s: quick_open_offers(s, "sample", "sample.rs"), 8)
        session.send("\r")
        session.wait(lambda s: "fn configure_pipeline" in s.text(), 8)

        # One tab, not two: a file opened into the untitled buffer the window started on takes
        # its place rather than sitting beside it, as long as nobody had typed in it.
        report.check("opening a file took the untitled buffer's place",
                     "[untitled]" not in session.text() and "[senza nome]" not in session.text(),
                     session)

        session.press("\x04", no_file_open, 6)          # Ctrl+D, close the only tab
        report.check("closing the last tab leaves nothing open", no_file_open(session), session)
        report.check("and no untitled buffer took its place",
                     "[untitled]" not in session.text() and "[senza nome]" not in session.text(),
                     session)

        # Still alive. A panic here would leave the last frame on screen, which reads as a pass
        # until something is asked to change — so something is. And what is waited for has to be
        # something that was not on screen already, which a project filename is not: closing the
        # last tab does not close the file tree, so `sample.rs` is sitting in it before Ctrl+O is
        # sent and would go on sitting there in the frozen final frame of a process that had
        # stopped drawing. The box is the only thing here that can only exist because a frame was
        # drawn after the keystroke.
        session.send("\x0f")
        report.check("the app is still drawing with nothing open",
                     session.wait(quick_open_up, 8), session)
        session.send("\x1b")
    finally:
        session.close()


def typing_in_a_column_selection(binary, report):
    """A column selection that writes: one key, one character on every line of the block.

    The roadmap's version of multi-cursor — one cursor and one anchor describing a column, rather
    than a list of independent carets — so what has to be checked from outside is that the column
    *survives the keystroke*. The unit tests own the rope arithmetic; what they cannot own is the
    wiring: whether the palette's switch reaches the editor, whether a printable key routed
    through the app's key handler still finds the block up, and whether Esc gets you out of a mode
    in which every letter you type happens four times.

    Its own session with its own fixture, whose third line is deliberately too short to reach the
    column the others share: the rule is that a rectangle over ragged text writes only where there
    is line to write on, and never pads a short line out with spaces nobody typed.

    And its own project directory, not the shared one. This session is a long one and it ends with
    an unsaved buffer, which is exactly what the autosave writes a recovery copy of — and the next
    session started in the same directory would open onto the offer to restore it rather than onto
    the editor the checks after this one are written against."""
    root = tempfile.mkdtemp(prefix="clee_columns_")
    with open(os.path.join(root, "columns.txt"), "w") as handle:
        handle.write(COLUMNS)
    session = Session(binary, root)
    lines = lambda: [session.buffer_line(n) for n in (1, 2, 3, 4)]
    try:
        if not session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            report.check("the column-selection session starts", False, session)
            return
        session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 10)
        session.send("\x0f")                             # Ctrl+O, quick-open
        session.wait(lambda s: "columns.txt" in s.text(), 8)
        session.send("columns")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        if not session.wait(lambda s: "three gamma" in s.text(), 8):
            report.check("the column fixture opens", False, session)
            return

        # Column 4 on every line — past the end of the short one, which is the interesting part.
        # Waited on the position the status bar reports rather than on the clock: a rectangle
        # anchored one column off would still write a column, and every check below would pass
        # while testing something nobody asked for.
        at_column_five = session.press("\x1b[C" * 4, lambda s: s.lines()[-1].endswith("1:5"))
        report.check("the cursor is where the block is about to be anchored", at_column_five,
                     session, note=session.lines()[-1][-8:])
        session.send("\x10")                             # Ctrl+P, the palette
        session.wait(lambda s: "matches" in s.text(), 6)
        session.send("column sel")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        report.check("the palette reaches column selection",
                     session.wait(lambda s: "Column selection on" in s.text(), 6), session)

        session.press("\x1b[1;2B" * 3, lambda s: True, 1)  # Shift+Down, down all four lines
        session.press("#", lambda s: (s.buffer_line(1) or "") == "one #alpha")
        report.check("one keystroke writes on every line of the block",
                     lines()[:2] == ["one #alpha", "two #beta"] and lines()[3] == "thre#e gamma",
                     session, note=str(lines()))
        report.check("a line too short for the column is left alone, not padded out to it",
                     lines()[2] == "ab", session, note=repr(lines()[2]))

        # The block now has no width, so there is nothing shaded and one terminal cursor for four
        # lines. What tells the user their next key happens four times is the column of carets —
        # read here as cell colour, since that is all it is.
        row = session.row_of("one #alpha")
        left = session.full_line(row).index("one #alpha")
        caret = session.cells(row)[left + 5]
        plain = session.cells(row)[left + 1]
        report.check("a caret is drawn at the column the next key would write at",
                     caret.bg != plain.bg, session, note=f"{caret.bg} against {plain.bg}")
        short = session.cells(row + 2)[left + 5]        # the "ab" line, two rows down
        report.check("and no caret on the line that is too short to receive one",
                     short.bg == plain.bg, session, note=f"{short.bg} against {plain.bg}")

        session.press("12", lambda s: (s.buffer_line(1) or "") == "one #12alpha")
        report.check("the characters that follow land as a column",
                     lines() == ["one #12alpha", "two #12beta", "ab", "thre#12e gamma"],
                     session, note=str(lines()))

        session.press("\x7f", lambda s: (s.buffer_line(1) or "") == "one #1alpha")
        report.check("Backspace eats one column back on every line",
                     lines() == ["one #1alpha", "two #1beta", "ab", "thre#1e gamma"],
                     session, note=str(lines()))

        session.press("\x1a", lambda s: (s.buffer_line(1) or "") == "one #12alpha")
        report.check("one Ctrl+Z puts back the whole keystroke, all lines of it",
                     lines() == ["one #12alpha", "two #12beta", "ab", "thre#12e gamma"],
                     session, note=str(lines()))

        # Out of the mode. The block is drawn as a column of carets and nothing else, so the
        # thing to check is not the picture but what the next letter does: one line, not two.
        session.press("\x1b[H", lambda s: True, 1)       # Home, a known column to start from
        session.send("\x10")
        session.wait(lambda s: "matches" in s.text(), 6)
        session.send("column sel")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        session.wait(lambda s: "Column selection on" in s.text(), 6)
        session.press("\x1b[1;2A", lambda s: True, 1)    # Shift+Up: a block over two lines
        session.press("\x1b", lambda s: True, 1)         # Esc
        session.press("Q", lambda s: (s.buffer_line(3) or "").startswith("Q"))
        report.check("Esc drops the block, and typing goes back to one line",
                     lines() == ["one #12alpha", "two #12beta", "Qab", "thre#12e gamma"],
                     session, note=str(lines()))
    finally:
        session.close()
        shutil.rmtree(root, ignore_errors=True)


def click(session, col, row):
    """One press and release, in the SGR encoding CleeCode turns on at startup. One-based."""
    session.send(f"\x1b[<0;{col + 1};{row + 1}M")
    session.send(f"\x1b[<0;{col + 1};{row + 1}m")


def clicks(session, col, row, times):
    """`times` presses on one cell, sent as a single write so the 400ms double-click threshold
    is a fact about the app's clock and never about this script's."""
    one = f"\x1b[<0;{col + 1};{row + 1}M\x1b[<0;{col + 1};{row + 1}m"
    session.send(one * times)


def row_bgs(session, row, start, end):
    """The background colours across a span of one screen row."""
    return {cell.bg for cell in session.cells(row)[start:end]}


def hover(session, col, row):
    """Rest the pointer on a cell without pressing anything — the SGR any-motion report,
    button bits 35, which CleeCode receives because it asks for any-event tracking."""
    session.send(f"\x1b[<35;{col + 1};{row + 1}M")
    session.wait(lambda s: True, 0.4)


def right_click(session, col, row):
    """The other button, pressed and released — SGR button 2."""
    session.send(f"\x1b[<2;{col + 1};{row + 1}M")
    session.send(f"\x1b[<2;{col + 1};{row + 1}m")


def label_spot(session, label):
    """Where `label` sits on screen, as (row, col), or None."""
    return next(((y, line.index(label)) for y, line in enumerate(session.lines())
                 if label in line), None)


# The six handle stripes of the default theme, in the hex pyte spells a truecolor
# background — the same 1977 rainbow theme.rs states in RGB. Quoted rather than asked for,
# for the reason the theme's own test quotes it: a witness that read the values out of the
# binary would be testing the theme against itself.
STRIPES = {"61bb46", "fdb827", "f5821f", "e03a3e", "963d97", "009ddc"}


def label_lit(session, label):
    """Whether the row carrying `label` shows it highlighted. Two honest spellings: the
    classic setting is a run of REVERSED cells, and the default carousel paints the row's
    background with one of the six stripes — one stripe, the whole label, which is what
    tells a highlight from a menu that happens to sit near coloured text."""
    spot = label_spot(session, label)
    if spot is None:
        return False
    y, x = spot
    cells = session.cells(y)[x:x + len(label)]
    if all(cell.reverse for cell in cells):
        return True
    bgs = {cell.bg for cell in cells}
    return len(bgs) == 1 and next(iter(bgs)) in STRIPES


def selecting_with_clicks(binary, report):
    """The click ladder in the editor body: press again on the same cell, quickly, and the
    selection climbs — cursor, word, line, then the structural scale, one rung a press.

    What the unit tests own is the arithmetic: where a word starts, that a line takes its
    newline. What they cannot own is the ladder itself — that the second press within the
    threshold selects rather than re-anchors, that the fourth one on a file with no language
    server leaves the line selection standing instead of tearing it down, and that a drag off
    the second press grows by whole words. All of that is wiring between the mouse and the
    editor, so it is checked here, by colour: a selection is nothing but cells whose background
    is not the page's.

    Its own session and fixture, like the column-selection one and for its reason: these checks
    click by coordinates read off the screen, and sharing a buffer with earlier checks would
    make every coordinate a bet on what they left behind."""
    root = tempfile.mkdtemp(prefix="clee_clicks_")
    with open(os.path.join(root, "words.txt"), "w") as handle:
        handle.write("alpha beta_gamma delta\nsecond line here\nthird\n")
    session = Session(binary, root)
    try:
        if not session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            report.check("the click-ladder session starts", False, session)
            return
        session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 10)
        session.send("\x0f")                             # Ctrl+O, quick-open
        session.wait(lambda s: "words.txt" in s.text(), 8)
        session.send("words")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        if not session.wait(lambda s: "beta_gamma" in s.text(), 8):
            report.check("the words fixture opens", False, session)
            return

        row = session.row_of("alpha beta_gamma delta")
        left = session.full_line(row).index("alpha")
        # The page's own background, read off a cell nothing has selected.
        plain = session.cells(row)[left].bg

        # Two presses on the middle of beta_gamma: the word, all of it, and only it.
        clicks(session, left + 10, row, 2)
        word_lit = session.wait(
            lambda s: len(row_bgs(s, row, left + 6, left + 16)) == 1
            and plain not in row_bgs(s, row, left + 6, left + 16), 6)
        report.check("a double-click lights the whole word under it", word_lit, session,
                     note="beta_gamma, underscore included: the motions' own word_class")
        report.check("and the words either side stay unlit",
                     session.cells(row)[left].bg == plain
                     and session.cells(row)[left + 18].bg == plain, session)

        # The drag off a double-click: whole words. A fresh double-click (the pause above the
        # threshold resets the ladder), the second press held, the pointer walked onto delta.
        session.wait(lambda s: True, 0.6)
        click(session, left + 10, row)
        session.send(f"\x1b[<0;{left + 11};{row + 1}M")          # the held second press
        session.send(f"\x1b[<32;{left + 20};{row + 1}M")         # dragged onto delta
        session.send(f"\x1b[<0;{left + 20};{row + 1}m")
        both_words = session.wait(
            lambda s: plain not in row_bgs(s, row, left + 6, left + 22)
            and s.cells(row)[left].bg == plain, 6)
        report.check("dragging on from the double-click grows by whole words", both_words,
                     session, note="beta_gamma through delta lit, alpha still not")

        # Three presses: the line, edge to edge — alpha and delta both lit now.
        session.wait(lambda s: True, 0.6)
        clicks(session, left + 10, row, 3)
        line_lit = session.wait(
            lambda s: plain not in row_bgs(s, row, left, left + 22), 6)
        report.check("a triple-click lights the whole line", line_lit, session)

        # A fourth, on a .txt nothing serves: the ladder has nowhere to climb, and the honest
        # answer is the line still lit and the status line saying why — never a reset.
        session.wait(lambda s: True, 0.6)
        clicks(session, left + 10, row, 4)
        session.wait(lambda s: "No language server" in s.text(), 6)
        report.check("a fourth click without a server keeps the line and says why",
                     "No language server" in session.text()
                     and plain not in row_bgs(session, row, left, left + 22), session,
                     note=session.lines()[-1].strip()[:80])

        # And the ladder resets like any other click: one press elsewhere is just a cursor.
        session.wait(lambda s: True, 0.6)
        other = session.row_of("second line here")
        click(session, left + 2, other)
        cleared = session.wait(lambda s: plain in row_bgs(s, row, left + 6, left + 16), 6)
        report.check("a single click elsewhere drops the selection as it always has", cleared,
                     session)
    finally:
        session.close()
        shutil.rmtree(root, ignore_errors=True)


def the_status_bar_names_line_endings_and_converts_them(binary, report):
    """The chip beside row:col — "UTF-8 LF" / "UTF-8 CRLF" — and the Edit menu's "Convert line
    endings" that flips it.

    Its own fixture, written with real CRLF bytes on disk, and its own session: this is the one
    check in the file that asserts on raw saved bytes, and it ends with a save whose result the
    next check in this file must not have to account for.

    A second fixture, a garbage .ico, exists only to open a preview tab — extension-gated, so it
    never has to decode — and check the chip is absent there: a preview has no buffer to be UTF-8
    or CRLF about."""
    root = tempfile.mkdtemp(prefix="clee_eol_")
    crlf_path = os.path.join(root, "windows.txt")
    with open(crlf_path, "wb") as handle:
        handle.write(CRLF_TEXT)
    preview_path = os.path.join(root, "not_really.ico")
    with open(preview_path, "wb") as handle:
        handle.write(b"not actually an icon")

    session = Session(binary, root)
    try:
        if not session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            report.check("the line-ending session starts", False, session)
            return
        session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 10)
        session.send("\x0f")                              # Ctrl+O, quick-open
        session.wait(lambda s: "windows.txt" in s.text(), 8)
        session.send("windows")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        if not session.wait(lambda s: "one" in s.text(), 8):
            report.check("the CRLF fixture opens", False, session)
            return

        report.check("the status bar shows CRLF on open",
                     session.wait(lambda s: "UTF-8 CRLF" in s.lines()[-1], 6), session,
                     note=session.lines()[-1])

        session.send("\x10")                              # Ctrl+P, the palette
        session.wait(lambda s: "matches" in s.text(), 6)
        session.send("convert line")
        session.wait(lambda s: True, 0.5)
        report.check("the palette reaches the convert command",
                     "Convert line endings" in session.text(), session)
        session.send("\r")

        report.check("the bar says LF after converting",
                     session.wait(lambda s: "UTF-8 LF" in s.lines()[-1], 6), session,
                     note=session.lines()[-1])
        report.check("the status message names the change",
                     session.wait(lambda s: "CRLF" in s.text() and "LF" in s.text(), 3), session,
                     note=session.lines()[-2:])

        session.send("\x13")                              # Ctrl+S
        session.wait(lambda s: True, 1.5)
        with open(crlf_path, "rb") as handle:
            saved = handle.read()
        report.check("the bytes on disk carry no CRLF after saving",
                     b"\r\n" not in saved, session, note=repr(saved))

        session.send("\x0f")                              # Ctrl+O, open the preview fixture
        session.wait(lambda s: "not_really.ico" in s.text(), 8)
        session.send("not_really")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        session.wait(lambda s: True, 1.5)                 # give the decode a moment to fail
        report.check("the chip is absent on a preview tab",
                     "UTF-8" not in session.lines()[-1], session, note=session.lines()[-1])
    finally:
        session.close()
        shutil.rmtree(root, ignore_errors=True)


def colours_in(session, needle):
    """The distinct foreground colours the editor drew `needle` and the rest of its line in.

    One colour means the run of text was painted flat — which is what an unhighlighted buffer
    looks like, and the only evidence from outside the process that the highlighter did not
    run. Read from the needle up to the pane's own right border: the gutter is styled either
    way, and the border is a colour of its own that would make every line look highlighted."""
    row = session.row_of(needle)
    if row is None:
        return set()
    drawn = session.full_line(row)
    start = drawn.index(needle)
    end = drawn.find("│", start)
    cells = session.cells(row)[start:end if end > 0 else len(drawn)]
    return {cell.fg for cell in cells if (cell.data or " ").strip()}


def a_file_over_the_line_says_what_it_is_not_doing(binary, report):
    """The declared large-file mode: a file past 50 MB opens with no colours, no words of its
    own in the completion popup, a shallow undo history — and says so, twice.

    A real fixture over the real threshold, because the threshold is the thing being tested and
    a test-only way to move it would be a second definition of the mode. It costs about a second
    to write and about a second to open, which is the honest price of checking a limit rather
    than checking a mock of one.

    Its own session, and a small file of the same language beside the large one — both open at
    once, on purpose. The small file is the control that says the missing colours and the
    missing words are the *mode* and not the language, and having the large file sitting in a
    background tab while the popup opens in the small one is the other half of the rule: one
    huge file open anywhere must not tax completion everywhere."""
    root = tempfile.mkdtemp(prefix="clee_large_")
    big_path = os.path.join(root, "big.java")
    with open(big_path, "w") as handle:
        handle.write(LARGE_LINE * ((52 * 1024 * 1024) // len(LARGE_LINE) + 1))
    small_path = os.path.join(root, "small.java")
    with open(small_path, "w") as handle:
        handle.write(SMALL_JAVA)

    session = Session(binary, root)
    try:
        if not session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            report.check("the large-file session starts", False, session)
            return
        session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 10)
        session.send("\x0f")                              # Ctrl+O, quick-open
        session.wait(lambda s: "big.java" in s.text(), 8)
        session.send("big")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        # Generous: this is the one check in the file that reads fifty megabytes off disk.
        if not session.wait(lambda s: "configuration_marker" in s.text(), 30):
            report.check("the large file opens at all", False, session)
            return

        report.check("opening it says the size and what is off",
                     "52 MB" in session.text() and "no highlighting" in session.text(), session,
                     note=session.lines()[-1])
        report.check("the status bar keeps saying so beside row:col",
                     "· large" in session.lines()[-1], session, note=session.lines()[-1])
        report.check("the text is drawn flat, with no highlighting",
                     len(colours_in(session, "configuration_marker")) == 1, session,
                     note=repr(colours_in(session, "configuration_marker")))

        # Typing still works, and still opens a popup — on the language's keywords. What it must
        # not offer is a word out of the fifty-megabyte buffer under the cursor.
        report.check("typing opens the popup on keywords",
                     session.press("co", lambda s: s.popup_open()), session)
        words = session.popup_words()
        report.check("the language's keywords are still offered", "const" in words, session,
                     note=str(words))
        report.check("no word comes out of the large buffer",
                     "configuration_marker" not in words, session, note=str(words))
        session.press("\x1b", lambda s: not s.popup_open(), 3)
        first = session.buffer_line(1) or ""
        report.check("the keystrokes landed in the buffer", first.startswith("co"), session,
                     note=repr(first))

        # Saved rather than left dirty: an unsaved buffer this size would be copied into the
        # recovery directory on the autosave tick and left there when this session is killed,
        # and the driver has no business leaving fifty megabytes on somebody's disk.
        #
        # Waited on the editor's own "no longer dirty" mark, not on the clock: writing fifty
        # megabytes atomically — to a temporary file and then renamed over the original — takes
        # longer than a fixed sleep allows on a slow disk, and reading the file back before the
        # rename lands is how this check failed on a CI runner while passing on a fast laptop.
        # The tab loses its `*` the moment the save completes, which is exactly the moment the
        # bytes are there to read.
        session.send("\x13")                              # Ctrl+S
        saved = session.wait(lambda s: "big.java*" not in s.text(), 20)
        report.check("the large buffer reports itself saved", saved, session,
                     note="the tab drops its dirty mark when the write has landed")
        with open(big_path, "rb") as handle:
            head = handle.read(8)
        report.check("a large buffer saves what was typed into it", head.startswith(b"co"),
                     session, note=repr(head))

        # The control, in the same session and the same language: colours are back, the chip
        # says nothing about size, and the popup offers this buffer's own word — but still not
        # the large tab's, which is sitting right there in the background.
        session.send("\x0f")
        session.wait(lambda s: "small.java" in s.text(), 8)
        session.send("small")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        if not session.wait(lambda s: "counter_local" in s.text(), 10):
            report.check("the small file of the same language opens", False, session)
            return
        report.check("an ordinary file of the same language is highlighted",
                     len(colours_in(session, "int counter_local")) > 1, session,
                     note=repr(colours_in(session, "int counter_local")))
        report.check("and its chip says nothing about size",
                     "· large" not in session.lines()[-1], session, note=session.lines()[-1])

        session.press("\x1b[B", lambda s: True, 1)         # onto the line with the word
        session.press("\x1b[F", lambda s: True, 1)         # End
        report.check("the popup opens in the ordinary file",
                     session.press("co", lambda s: s.popup_open()), session)
        words = session.popup_words()
        report.check("it offers this buffer's own words", "counter_local" in words, session,
                     note=str(words))
        report.check("a large file in a background tab still contributes nothing",
                     "configuration_marker" not in words, session, note=str(words))
    finally:
        session.close()
        shutil.rmtree(root, ignore_errors=True)


def a_pane_is_told_what_it_is_talking_to(binary, root, report):
    """A terminal pane must be told it is talking to CleeCode's parser, not to the terminal
    CleeCode happens to be displayed in.

    Started here under a `TERM` no terminfo database has, which is the shape of the bug this
    exists for: CleeCode on an Ubuntu box, reached over ssh from Ghostty, inherits
    `TERM=xterm-ghostty` — and Ubuntu has no such entry. `clear` then clears nothing, and
    neither does the form feed CleeCode sends to scrub a shell's startup banner, because both
    go through the same terminfo capability. Nothing in the pane is broken; it has been told it
    is a terminal that is not there."""
    session = Session(binary, root, env={"TERM": "xterm-nonexistent-9000"})
    try:
        if not session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            report.check("the session with a hostile TERM starts", False, session)
            return
        session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 10)
        session.send("\x10")                                # Ctrl+P, the palette
        session.wait(lambda s: "matches" in s.text(), 6)
        session.send("focus term")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        session.wait(lambda s: True, 1.0)

        session.send("echo TERM_IS=$TERM\r")
        # Long, because what is being waited for is not the echo but the shell behind it: this is
        # the first prompt of the first shell of a freshly booted CI runner, where the fork, the
        # dynamic linker and the rc file are all cold, and the several seconds that costs are not
        # the editor's doing. The evidence is right — the answer can only come from the pane's own
        # shell — so the only thing to fix is the impatience, and being generous here costs a fast
        # machine nothing, since the wait returns the moment the line is on screen.
        session.wait(lambda s: "TERM_IS=xterm" in s.text(), 30)
        told = next((l for l in session.lines() if "TERM_IS=" in l and "echo" not in l), "")
        report.check("a pane is told it is an xterm-256color, whatever is outside",
                     "TERM_IS=xterm-256color" in told, session, note=repr(told.strip()[:60]))

        # The point of the above, and the thing the user actually notices.
        session.send("echo MARKER_BEFORE_CLEAR\r")
        session.wait(lambda s: "MARKER_BEFORE_CLEAR" in s.text(), 8)
        session.send("clear\r")
        cleared = session.wait(
            lambda s: not any("MARKER_BEFORE_CLEAR" in l for l in s.lines()), 8)
        report.check("clear clears the pane", cleared, session,
                     note="the same terminfo capability the banner scrub uses")
    finally:
        session.close()


def the_startup_banner_is_scrubbed(binary, root, report):
    """A shell whose rc prints in bursts still ends up at a clean prompt.

    The rc here echoes, sleeps, echoes, sleeps, echoes — which is what `fastfetch` on a loaded
    remote box looks like from outside, and what a fast Mac never does. CleeCode scrubs the
    banner by sending a form feed, and it used to send it as soon as the output had been quiet
    for a quarter of a second. Quiet is not the same as listening: an rc waiting on a command it
    started is exactly as quiet as a prompt, and a form feed sent then is not a command but a
    character — echoed straight back as a literal `^L` above a banner that is still arriving.

    So this asserts both halves: nothing of the banner is left, and no `^L` was printed."""
    rc = os.path.join(root, "slow.bashrc")
    with open(rc, "w") as handle:
        handle.write(
            'echo "BANNER_ONE"\n/bin/sleep 0.6\necho "BANNER_TWO"\n'
            '/bin/sleep 0.6\necho "BANNER_THREE"\nPS1="probe$ "\n')
    shell = os.path.join(root, "slowshell.sh")
    with open(shell, "w") as handle:
        handle.write("#!/bin/bash\nexec /bin/bash --rcfile %s -i\n" % rc)
    os.chmod(shell, 0o755)

    session = Session(binary, root, env={"SHELL": shell})
    try:
        if not session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            report.check("the session with a slow rc starts", False, session)
            return
        session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 10)
        # A predicate that is never true, so this really waits — `lambda s: True` returns on the
        # first read and would end the session before the rc had finished, which is how this
        # check first reported a pass it had not earned.
        session.wait(lambda s: False, 6)

        text = session.text()
        left = [part for part in ("BANNER_ONE", "BANNER_TWO", "BANNER_THREE") if part in text]
        report.check("a bursty startup banner is scrubbed", not left, session, note=str(left))
        report.check("and the form feed was never echoed as a character",
                     "^L" not in text, session,
                     note="a literal ^L means it was sent to a shell that was not reading")
    finally:
        session.close()


def quitting_ends_the_process_cleanly(binary, root, report):
    """Ctrl+Q has to end the process with status 0, shells and all.

    Which nobody sees in a shell they typed `clee` in, and everybody sees from the Dock: there
    the editor is given a terminal window of its own, and that window closes when the editor's
    process ends — unless the ending looked like a failure, in which case some terminals keep
    the window open with a line of their own text where the editor was. The panes are the part
    worth testing: quitting hangs up two live shells on the way out, waits for them, and kills
    the ones that would not go, and any of that turning into a signal or a non-zero status
    would leave that window sitting there."""
    session = Session(binary, root)
    try:
        if not session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            report.check("the session to be quit starts", False, session)
            return
        session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 10)
        # A minute, which sounds absurd for a keystroke and is not: quitting hangs up two live
        # shells and waits for each of them to go before the editor's own process may end, and on
        # a runner where merely *starting* one of those shells can take a handful of seconds,
        # stopping both of them can take a good deal longer than the twenty seconds this asked for
        # by default. Running out of patience here reports the process as still running, which is
        # the same picture as the bug this check exists for — a quit that hangs — so the timeout
        # has to be long enough that reaching it means something. The cost of the larger number is
        # paid only by a run that was going to fail anyway.
        status = session.quit(timeout=60)
        report.check("Ctrl+Q ends the process, and with status 0",
                     status is not None and os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0,
                     note=Session.describe_status(status))
    finally:
        session.close()


def open_extras_from_help(session):
    """Clicks Help on the menu bar, then Extras... on the menu it drops. True when the panel
    is up — anchored on chafa, the one row name that appears nowhere else on screen."""
    bar = session.full_line(0)
    if "Help" not in bar:
        return False
    click(session, bar.index("Help") + 1, 0)
    if not session.wait(lambda s: "Extras..." in s.text(), 6):
        return False
    spot = next(((y, line.index("Extras...")) for y, line in enumerate(session.lines())
                 if "Extras..." in line), None)
    if spot is None:
        return False
    click(session, spot[1] + 2, spot[0])
    return session.wait(lambda s: "chafa" in s.text(), 6)


def extras_rows(session):
    """The panel's rows as {first token of the name: has its ✓}, read from the frame chafa is
    in. Parsed off the leading mark rather than searched by name, because the names leak into
    each other's purposes — typst's row says what pandoc leans on."""
    rows = {}
    for line in session.frame_of("chafa"):
        tokens = line.split()
        if not tokens:
            continue
        if tokens[0] == "✓" and len(tokens) > 1:
            rows[tokens[1]] = True
        else:
            rows[tokens[0]] = False
    return rows


def the_extras_panel_and_the_font_offer(binary, report):
    """Help ▸ Extras, and the first-launch font offer it shares its installer with.

    The offer's whole decision is unit-tested; what only the binary can show is the offer
    *behaving*: rising once the splash is down, declining on any key, and never rising again
    on the same config — the state file remembers. So the first two sessions share a root and
    a virgin HOME (the font check looks in $HOME, and pointing HOME at the fixture is what
    makes "not installed" true on any machine), with the driver's own kill switch overridden
    back to on, and SSH_CONNECTION cleared because the offer rightly refuses to install a
    font on the wrong end of an ssh line — which is where these drivers themselves sometimes
    run.

    The third session is the panel: opened from the Help menu, a stubbed typst on the PATH so
    exactly one ✓ is this script's to predict, and the Nerd Font row — missing, HOME being
    virgin again — chosen and accepted, so the panel's action road and the installer run for
    real into the throwaway HOME. The install-command road for a missing *tool* is checked
    only where a machine actually misses one: a ✓ cannot be taken off a row by hiding the
    PATH, because `tools::tool` looks past it on purpose."""
    # ---- the offer, on a machine that has never seen the font --------------------------------
    root = tempfile.mkdtemp(prefix="clee_extras_")
    offer_env = {"HOME": root, "CLEE_FONT_OFFER": "1", "SSH_CONNECTION": ""}
    session = Session(binary, root, env=offer_env)
    try:
        if session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            session.send(" ")  # the splash takes any key; the offer waits for it to be down
        offered = session.wait(lambda s: "Icons need a font" in s.text(), 10)
        report.check("a first launch without the font offers to install it", offered, session,
                     note="the question waits out the splash and says what the font is for")
        if offered:
            session.send("x")
            declined = session.wait(lambda s: "clee --install-font" in s.text(), 6)
            report.check("any other key declines, and the status names the road back",
                         declined, session)
    finally:
        session.close()

    session = Session(binary, root, env=offer_env)
    try:
        if session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 10)
        session.wait(lambda s: True, 1.5)
        report.check("the declined offer never returns on the same config",
                     "Icons need a font" not in session.text(), session,
                     note="asked once is asked: update.toml remembers the question, not the answer")
    finally:
        session.close()

    # ---- the panel ---------------------------------------------------------------------------
    fakebin = os.path.join(root, "fakebin")
    os.makedirs(fakebin, exist_ok=True)
    with open(os.path.join(fakebin, "typst"), "w") as handle:
        handle.write("#!/bin/sh\nexit 0\n")
    os.chmod(os.path.join(fakebin, "typst"), 0o755)
    panel_root = tempfile.mkdtemp(prefix="clee_extras_panel_")
    panel_env = {
        "HOME": panel_root,  # virgin again, so the Nerd Font row is missing for sure
        "PATH": fakebin + os.pathsep + os.environ.get("PATH", ""),
    }
    session = Session(binary, panel_root, env=panel_env)
    try:
        if session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 10)

        opened = open_extras_from_help(session)
        rows = extras_rows(session) if opened else {}
        report.check("Help > Extras opens the panel with every row on it",
                     opened and all(name in rows for name in
                                    ["Nerd", "poppler", "pandoc", "typst", "chafa"]),
                     session, note="rows found: %s" % sorted(rows))
        report.check("the stubbed typst wears its ✓", rows.get("typst") is True, session,
                     note="presence is asked of tools::tool, which reads the PATH this stub is on")
        report.check("and the virgin HOME leaves the Nerd Font row missing",
                     rows.get("Nerd") is False, session)

        session.press("\x1b", lambda s: "chafa" not in s.text(), 4)
        report.check("Esc puts the panel away", "chafa" not in session.text(), session)

        # The Nerd Font row, chosen and accepted: the panel's action road, the same modal the
        # first launch raises, and the real installer — into the throwaway HOME.
        installed = False
        if open_extras_from_help(session):
            session.send("\r")  # row 0 is the font
            if session.wait(lambda s: "Icons need a font" in s.text(), 6):
                session.send("\r")
                installed = session.wait(lambda s: "Font installed" in s.text(), 15)
        report.check("choosing the missing font row asks, and Enter installs for real",
                     installed, session,
                     note="the .ttf lands in the fixture's own HOME, nobody's real one")
        if installed:
            reopened = open_extras_from_help(session)
            report.check("and the row wears its ✓ the next time the panel opens",
                         reopened and extras_rows(session).get("Nerd") is True, session)
            session.press("\x1b", lambda s: "chafa" not in s.text(), 4)

        # A missing tool row types its install command at a prompt, unsent. Only checkable
        # where a tool is really missing: a ✓ cannot be faked off, tools::tool looks past the
        # PATH on purpose. CI's runners miss chafa; a developer's machine may miss nothing.
        rows = extras_rows(session) if open_extras_from_help(session) else {}
        order = ["Nerd", "poppler", "pandoc", "typst", "chafa"]
        missing = next((n for n in ["poppler", "pandoc", "chafa"] if rows.get(n) is False), None)
        if missing is None:
            session.press("\x1b", lambda s: "chafa" not in s.text(), 4)
            print("  SKIP  the typed install command: this machine has every tool the panel lists")
        else:
            session.send("\x1b[B" * order.index(missing) + "\r")
            typed = session.wait(lambda s: "unsent" in s.text() or "no shell" in s.text(), 8)
            report.check("a missing tool's command lands at the prompt unsent", typed, session,
                         note="chose %s; the same promise the drawer's launcher makes" % missing)
            if typed and "unsent" in session.text():
                session.wait(lambda s: True, 1.5)
                report.check("and it is still sitting there, not run",
                             "install" in session.text() and "unsent" in session.text(),
                             session, note="the Enter is the user's, here as everywhere")
    finally:
        session.close()


def the_highlight_follows_the_pointer(binary, report):
    """Pop-up lists follow the pointer now: resting it on a row moves the highlight, no click.
    The arrows always did this; the mouse needed a press to be believed, which left the menus
    reading as dead under a moving pointer. Colour is the witness again: a menu's selection
    wears one of the carousel's stripes (or reverse video, under the classic setting — see
    `label_lit`), so every check is which label wears it after the pointer has rested where —
    and the separator case is the negative that keeps the rule honest, because walking off a
    row must not be a choice.

    Its own session, like every section that clicks by coordinates."""
    root = tempfile.mkdtemp(prefix="clee_hover_")
    with open(os.path.join(root, "hello.txt"), "w") as handle:
        handle.write("hello pointer line\nsecond line\n")
    session = Session(binary, root)
    try:
        if not session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            report.check("the hover session starts", False, session)
            return
        session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 10)

        # The bar's dropdown: open File, rest the pointer on a lower row, read the highlight.
        bar = session.full_line(0)
        if "File" not in bar or "Help" not in bar:
            report.check("the menu bar is on screen", False, session)
            return
        click(session, bar.index("File") + 1, 0)
        opened = session.wait(lambda s: "Save As..." in s.text(), 6)
        report.check("File drops its menu", opened, session)
        if opened:
            spot = label_spot(session, "Save As...")
            hover(session, spot[1] + 2, spot[0])
            report.check("the dropdown's highlight follows the pointer onto a row",
                         session.wait(lambda s: label_lit(s, "Save As..."), 6), session,
                         note="no click: resting the pointer is enough")
            # An open menu follows the pointer along the bar itself, title to title.
            hover(session, bar.index("Help") + 1, 0)
            switched = session.wait(lambda s: "Extras..." in s.text(), 6)
            report.check("sliding along the bar switches the open menu", switched, session)
            # And Enter acts on the row the pointer chose, exactly as it would after arrows.
            if switched:
                spot = label_spot(session, "Extras...")
                hover(session, spot[1] + 2, spot[0])
                lit = session.wait(lambda s: label_lit(s, "Extras..."), 6)
                session.send("\r")
                acted = lit and session.wait(lambda s: "chafa" in s.text(), 6)
                report.check("Enter runs the row the pointer rested on", acted, session)
                if acted:
                    session.press("\x1b", lambda s: "chafa" not in s.text(), 4)

        # The context menu, which has separators and knows not to stop on them.
        session.send("\x0f")
        session.wait(lambda s: "hello.txt" in s.text(), 8)
        session.send("hello")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        if not session.wait(lambda s: "hello pointer line" in s.text(), 8):
            report.check("the hover fixture opens", False, session)
            return
        # Anchored on the whole phrase: the tree shows "hello.txt" on this same screen row,
        # and a shorter needle would hand the click to the sidebar's column.
        row = session.row_of("hello pointer line")
        col = session.full_line(row).index("hello pointer line")
        right_click(session, col + 2, row)
        menu_up = session.wait(lambda s: "Select All" in s.text(), 6)
        report.check("right-click raises the editor's menu", menu_up, session)
        if menu_up:
            spot = label_spot(session, "Go to line...")
            hover(session, spot[1] + 2, spot[0])
            moved = session.wait(lambda s: label_lit(s, "Go to line..."), 6)
            report.check("the context menu's highlight follows the pointer", moved, session)
            if moved:
                # The rule above "Find / Replace..." is a separator: resting the pointer on
                # it moves nothing, and the highlight stays where the last real row put it.
                find = label_spot(session, "Find / Replace...")
                hover(session, find[1] + 2, find[0] - 1)
                session.wait(lambda s: True, 0.6)
                report.check("a separator does not steal the highlight",
                             label_lit(session, "Go to line...")
                             and not label_lit(session, "Find / Replace..."),
                             session, note="walking off a row is not a choice")
            session.press("\x1b", lambda s: "Select All" not in s.text(), 4)
    finally:
        session.close()
        shutil.rmtree(root, ignore_errors=True)


def main():
    binary = binary_from_argv(sys.argv)
    root = tempfile.mkdtemp(prefix="clee_drive_")
    with open(os.path.join(root, "sample.rs"), "w") as handle:
        handle.write(SAMPLE)

    report = Report()
    session = Session(binary, root)
    try:
        # Generous, and deliberately so: the first frame waits on the terminal queries timing out.
        started = session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20)
        report.check("the app draws its first frame", started, session)
        if not started:
            return 1

        session.send(" ")                               # the splash takes any key
        session.wait(lambda s: "Files" in s.text(), 10)
        report.check("the splash gives way to the frame", "Files" in session.text(), session)

        session.send("\x0f")                            # Ctrl+O, quick-open
        session.wait(lambda s: "sample.rs" in s.text(), 8)
        session.send("sample")
        session.wait(lambda s: True, 0.5)
        session.send("\r")

        completion(session, report)
        Report.show("final screen", session)
    finally:
        session.close()

    try:
        typing_in_a_column_selection(binary, report)
        selecting_with_clicks(binary, report)
        the_status_bar_names_line_endings_and_converts_them(binary, report)
        a_file_over_the_line_says_what_it_is_not_doing(binary, report)
        closing_the_last_tab(binary, root, report)
        a_pane_is_told_what_it_is_talking_to(binary, root, report)
        the_startup_banner_is_scrubbed(binary, root, report)
        the_extras_panel_and_the_font_offer(binary, report)
        the_highlight_follows_the_pointer(binary, report)
        quitting_ends_the_process_cleanly(binary, root, report)
    finally:
        shutil.rmtree(root, ignore_errors=True)

    return report.finish()


if __name__ == "__main__":
    try:
        sys.exit(main())
    except BrokenPipeError:
        # Piped into `head`, which walked off before the screen dump finished.
        os._exit(0)
