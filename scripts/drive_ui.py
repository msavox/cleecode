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
        session.wait(lambda s: "sample.rs" in s.text(), 8)
        session.send("sample")
        session.wait(lambda s: True, 0.5)
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
        # until something is asked to change — so something is.
        session.send("\x0f")
        report.check("the app is still drawing with nothing open",
                     session.wait(lambda s: "sample.rs" in s.text(), 8), session)
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
        session.wait(lambda s: "TERM_IS=xterm" in s.text(), 8)
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
        status = session.quit()
        report.check("Ctrl+Q ends the process, and with status 0",
                     status is not None and os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0,
                     note=Session.describe_status(status))
    finally:
        session.close()


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
        the_status_bar_names_line_endings_and_converts_them(binary, report)
        closing_the_last_tab(binary, root, report)
        a_pane_is_told_what_it_is_talking_to(binary, root, report)
        the_startup_banner_is_scrubbed(binary, root, report)
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
