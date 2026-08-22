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
        closing_the_last_tab(binary, root, report)
        a_pane_is_told_what_it_is_talking_to(binary, root, report)
    finally:
        shutil.rmtree(root, ignore_errors=True)

    return report.finish()


if __name__ == "__main__":
    try:
        sys.exit(main())
    except BrokenPipeError:
        # Piped into `head`, which walked off before the screen dump finished.
        os._exit(0)
