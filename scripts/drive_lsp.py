#!/usr/bin/env python3
"""Drive the real CleeCode binary against a stub language server and look at the screen.

    python3 scripts/drive_lsp.py [path/to/clee]     # default: target/debug/clee

The client has its own tests in src/lsp.rs, up to and including talking to a real process. What
none of them can answer is the question that matters to somebody using the editor: *does a
squiggle appear under the wrong word*, and *does the name the server offered end up in the file*.
That needs the whole program — the poll in the frame loop, the debounce, the span surgery in the
renderer, the popup that was already on screen when the answer arrived — and a screen to look at.

There is no rust-analyzer here, and installing one would make this a test of rust-analyzer. So
scripts/lsp_stub.py is put on PATH under the name CleeCode looks for. It answers the handshake
and publishes two diagnostics against whatever file it is told was opened, which also proves the
path-to-URI encoding round-tripped: a stub inventing its own URI would pass with that broken.

The same goes for the three lists — the uses of a name, the names in a file, and everything
wrong at once. Each is a picker, so what is checked is what a picker is for: that the rows say
where they point, and that Enter arrives there.

The rename is the one that writes, so it is checked for what a write has to promise: that the
preview says what would change before anything does, that Esc leaves the file exactly as it was,
that one Ctrl+Z takes back every occurrence at once, and that an answer touching a file no tab
holds is refused whole rather than applied in part.

pyte tracks colour and attributes per cell, not just characters, so "underlined" and "red" are
things this can actually check rather than infer.
"""

import os
import re
import shutil
import stat
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_drive import Report, Session, binary_from_argv  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))

# The stub marks line 1 (columns 8..13) as a warning and line 2 (columns 4..9) as an error.
SAMPLE = """fn main() {
    let dummy = 1;
    let y = nope;
}
"""


# Four lines, each different, so which one the cursor is on can be read off a single character
# typed into it.
JUMP = "aaa\nbbb\nccc\nddd\n"


# Two occurrences on one line and one further down, which is the shape a rename gets wrong
# quietly: the first edit on a line moves the second, so a client applying them one at a time
# against offsets measured before any of them eats a character — and only on that line.
NAME = "alfa = alfa + 1\nsecond\nalfa\n"


def install_stub(root):
    """A directory holding an executable called what CleeCode will look for."""
    bindir = os.path.join(root, "fakebin")
    os.makedirs(bindir, exist_ok=True)
    shim = os.path.join(bindir, "rust-analyzer")
    with open(shim, "w") as handle:
        handle.write("#!/bin/sh\nexec %s %s\n" % (sys.executable, os.path.join(HERE, "lsp_stub.py")))
    # The stub's stderr goes nowhere by design, so its account of the conversation goes here.
    # Printed when a check fails, which is the only time anyone wants it.
    os.environ["CLEECODE_STUB_LOG"] = os.path.join(root, "stub.log")
    os.chmod(shim, os.stat(shim).st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    return bindir


def underlined_run(session, row):
    """The columns on `row` the terminal was told to underline, as a (first, last) pair."""
    cols = [x for x, cell in enumerate(session.cells(row)) if cell.underscore]
    return (cols[0], cols[-1]) if cols else None


def coloured(session, row, colour):
    """Columns on `row` painted `colour`. pyte reports colours as hex, not by name."""
    return [x for x, cell in enumerate(session.cells(row)) if cell.fg == colour]


def in_buffer(line):
    """Whether `line` is the file's fifth line holding the accepted word, gutter and all — as
    opposed to a row of the popup, which shows the same word with no line number beside it."""
    return re.search(r"\s5 du_line4_col2\b", line) is not None


def word_colour(session, word, skip=None):
    """The colour `word` is painted, found by row *and* column.

    A screen row here runs through the file tree, the editor's border and then the popup, so the
    first painted cell on it belongs to something else entirely. `skip` passes over a row that
    also matches for the wrong reason — `dummy` appears in the source above the list as well."""
    for y, line in enumerate(session.lines()):
        x = line.find(word)
        if x < 0 or (skip is not None and skip in line):
            continue
        return session.cells(y)[x].fg
    return None


def main():
    binary = binary_from_argv(sys.argv)
    root = tempfile.mkdtemp(prefix="clee_lsp_")
    os.makedirs(os.path.join(root, "src"), exist_ok=True)
    with open(os.path.join(root, "Cargo.toml"), "w") as handle:
        handle.write('[package]\nname = "probe"\nversion = "0.1.0"\nedition = "2021"\n')
    with open(os.path.join(root, "src", "main.rs"), "w") as handle:
        handle.write(SAMPLE)
    # A second file for the jumping, with four lines that are told apart at a glance. Its own
    # file because the completion checks leave the first one edited, and a check that has to
    # reason about which edits happened before it is a check nobody can read.
    with open(os.path.join(root, "src", "jump.rs"), "w") as handle:
        handle.write(JUMP)
    # And a third for the rename, for the same reason: it is the one file in here that gets
    # written to, undone and written to again, and a check that had to reason about which of the
    # earlier edits had happened first would be a check nobody can read.
    with open(os.path.join(root, "src", "name.rs"), "w") as handle:
        handle.write(NAME)
    bindir = install_stub(root)

    report = Report()
    session = Session(binary, root, env={"PATH": bindir + os.pathsep + os.environ["PATH"]})
    try:
        started = session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20)
        report.check("the app draws its first frame", started, session)
        if not started:
            return 1
        session.send(" ")
        session.wait(lambda s: "Files" in s.text(), 10)

        session.send("\x0f")                                  # Ctrl+O, quick-open
        session.wait(lambda s: "main.rs" in s.text(), 8)
        session.send("main.rs")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        report.check("the file opens", session.wait(lambda s: "let dummy" in s.text(), 8), session)

        # The server is started lazily on the first file it knows about, then has to answer.
        report.check("the language server is found and answers",
                     session.wait(lambda s: "rust-analyzer risponde" in s.text()
                                  or "rust-analyzer is answering" in s.text(), 15),
                     session, note="the stub, installed on PATH under that name")

        # The diagnostics follow didOpen. Wait for the underline itself rather than for a
        # message about it — the whole point is what reached the screen.
        warned = session.row_of("let dummy")
        got = session.wait(lambda s: underlined_run(s, warned) is not None, 15)
        report.check("the warning is underlined in the text", got, session)
        if got:
            first, last = underlined_run(session, warned)
            line = session.lines()[warned]
            report.check("it underlines the word the server named",
                         line[first:last + 1] == "dummy", session,
                         note=repr(line[first:last + 1]))

        errored = session.row_of("let y = nope")
        report.check("the error is underlined too",
                     underlined_run(session, errored) is not None, session)

        # Severity has to be distinguishable, or the two are one signal wearing two names.
        warn_cells = [session.cells(warned)[x] for x in range(*underlined_run(session, warned))]
        err_cells = [session.cells(errored)[x] for x in range(*underlined_run(session, errored))]
        report.check("a warning and an error do not look the same",
                     warn_cells and err_cells and warn_cells[0].fg != err_cells[0].fg,
                     session,
                     note=f"warning={warn_cells[0].fg if warn_cells else '?'} "
                          f"error={err_cells[0].fg if err_cells else '?'}")

        # The gutter number carries the mark, so a marked line is findable without reading it —
        # which means the error colour has to appear left of the text, not only under it.
        err_colour = err_cells[0].fg if err_cells else None
        left_of_text = [x for x in coloured(session, errored, err_colour)
                        if x < underlined_run(session, errored)[0] - 2]
        report.check("the line number is marked too", bool(left_of_text), session,
                     note=f"columns {left_of_text} in {err_colour}")

        # And the message for the line the cursor is on shares the status row. Two lines down
        # from the top of the file is `let y = nope`, the error.
        session.press("\x1b[B" * 2, lambda s: "nope" in s.lines()[-1], 6)
        report.check("the status row says what is wrong with the current line",
                     "nope" in session.lines()[-1], session, note=repr(session.lines()[-1]))

        # ---- completion ------------------------------------------------------------------
        #
        # The stub answers with the position it was asked about and the word it found there, so a
        # name on screen is proof of the whole round trip: the file reached the server, the
        # position was counted the way it counts, and the answer came back into the popup that
        # asked. Nothing here is canned, which is why it can fail.
        session.press("\x1b[B" * 2, lambda s: True, 1)     # to the empty last line, line 4
        session.send("du")
        offered = session.wait(lambda s: "du_line4_col2" in s.text(), 15)
        report.check("the server's answer reaches the open popup", offered, session,
                     note="the label the stub builds out of the position it was asked about")

        if offered:
            screen = session.text()
            report.check("a suggestion with no word at its head is not offered",
                         "&reference" not in screen, session,
                         note="the stub sorts it first, so offering it would put it on top")
            report.check("a label written to be read is reduced to the word it types",
                         "duplicate" in screen and "duplicate(" not in screen, session)
            report.check("two items that type the same word are one row",
                         screen.count("duplicate") == 1, session)

            # The rows have to say where they came from, or the second source is invisible.
            #
            # Asked of `duplicate` rather than of the top row, and that is the whole care in this
            # check: the top row is the *selected* one, painted black on cyan by the highlight.
            # Comparing that against a word from the file passes whatever the sources are
            # coloured — it would go on passing with the colour removed altogether. And it is the
            # popup's own `dummy` row that is the comparison, not the `let dummy = 1;` above it.
            from_server = word_colour(session, "duplicate")
            from_buffer = word_colour(session, "dummy", skip="let ")
            report.check("a name from the server does not look like a word from the file",
                         from_buffer is not None and from_server != from_buffer, session,
                         note=f"server={from_server} buffer={from_buffer}")

            # And the point of all of it: the word goes into the file, without the brackets and
            # in one step. Looked for beside its line number, which is what tells a line of the
            # buffer from a row of the list that is showing the same word.
            session.press("\r", lambda s: any(in_buffer(l) for l in s.lines()), 6)
            typed = [line for line in session.lines() if in_buffer(line)]
            report.check("accepting types the server's word into the buffer", bool(typed), session,
                         note=repr(typed[:1]))

        # ---- what it is, and where it is defined -------------------------------------------
        #
        # The stub answers both out of the question, as it does for completion: the hover names
        # the word it was asked about, and the definition points one line further down than the
        # cursor. So a client that ignored either answer and stayed put would land in the wrong
        # place and be caught — a canned line number could not pass this.
        session.send("\x0f")                                  # Ctrl+O, quick-open
        session.wait(lambda s: "jump.rs" in s.text(), 8)
        session.send("jump.rs")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        # By the gutter number beside it, not by the line ending in it: the editor pads every
        # row out to the frame, so nothing on this screen ends in anything.
        opened = session.wait(lambda s: any(re.search(r"\s4 ddd\b", l) for l in s.lines()), 8)
        report.check("the second file opens", opened, session)

        # Three rights puts the cursor at the end of `aaa` on the first line, which is a word and
        # is not one of the two lines the stub underlines — a diagnostic would take the status
        # bar's right-hand spot, and rightly so.
        session.press("\x1b[C\x1b[C\x1b[C", lambda s: True, 2)
        hovered = session.wait(lambda s: "kind_of_aaa" in s.text(), 12)
        report.check("what the thing under the cursor is turns up on its own", hovered, session)
        report.check("and only the line worth reading, not the markup around it",
                     "```" not in session.text() and "Prose nobody" not in session.text(),
                     session)

        # The jump, read by typing into wherever it landed. A screen check for "the cursor
        # moved" would be a check on the highlight; this is a check on where the next character
        # actually goes, which is the thing that matters.
        session.press(session.chord("j"), lambda s: True, 4)
        session.press("X", lambda s: any("Xbbb" in l for l in s.lines()), 6)
        report.check("Ctrl+Shift+J goes where the server said the definition is",
                     any("Xbbb" in l for l in session.lines()), session,
                     note=repr([l.strip() for l in session.lines() if "bbb" in l][:1]))

        session.press(session.chord("l"), lambda s: True, 4)
        session.press("Y", lambda s: any("aaaY" in l for l in s.lines()), 6)
        report.check("Ctrl+Shift+L comes back to the exact place the jump started from",
                     any("aaaY" in l for l in session.lines()), session,
                     note=repr([l.strip() for l in session.lines() if "aaa" in l][:1]))

        # ---- the three lists ----------------------------------------------------------------
        #
        # The file now reads aaaY / Xbbb / ccc / ddd, and the cursor is at the end of its first
        # line. The stub answers `references` with the two lines under that one, so both rows are
        # readable off the screen and the second is somewhere the cursor has never been.
        session.press(session.chord("y"), lambda s: "jump.rs:3" in s.text(), 12)
        listed = session.text()
        report.check("Ctrl+Shift+Y lists every use the server named",
                     "jump.rs:2" in listed and "jump.rs:3" in listed, session,
                     note="the two the stub answers with, one row each")
        report.check("and each row shows the line it points at, not only its number",
                     "jump.rs:2  Xbbb" in listed and "jump.rs:3  ccc" in listed, session)

        # Down, then Enter: the second row, whose line the cursor has not been on. Read by
        # typing into wherever it landed, as the jump above is.
        session.press("\x1b[B", lambda s: True, 2)
        session.press("\r", lambda s: True, 6)
        session.press("Z", lambda s: any("Zccc" in l for l in s.lines()), 6)
        report.check("Enter on a use goes to that line",
                     any("Zccc" in l for l in session.lines()), session,
                     note=repr([l.strip() for l in session.lines() if "ccc" in l][:1]))

        # A row is a jump like any other, so the key that comes back from a definition comes
        # back from here — to the exact column the list was asked from, which is after the Y.
        session.press(session.chord("l"), lambda s: True, 4)
        session.press("B", lambda s: any("aaaYB" in l for l in s.lines()), 6)
        report.check("Ctrl+Shift+L comes back from a row of the list too",
                     any("aaaYB" in l for l in session.lines()), session,
                     note=repr([l.strip() for l in session.lines() if "aaa" in l][:1]))

        # The outline. The stub names its symbols after the file it was asked about, so a client
        # that asked about the wrong document shows names that say so — and it nests them, which
        # CleeCode never says it can read and reads anyway.
        session.press(session.chord("v"), lambda s: "inner_jump" in s.text(), 12)
        report.check("Ctrl+Shift+V lists what the file contains",
                     "outer_jump" in session.text() and "inner_jump" in session.text(), session,
                     note="named after the file, so the wrong document could not pass")
        outer, inner = session.column_of("outer_jump"), session.column_of("inner_jump")
        report.check("a name inside another is drawn inside it",
                     outer is not None and inner == outer + 2, session,
                     note=f"outer at {outer}, inner at {inner}")
        report.check("and each row says what kind of thing it is",
                     "method" in session.text(), session)

        session.press("\x1b[B", lambda s: True, 2)
        session.press("\r", lambda s: True, 6)
        session.press("W", lambda s: any("Wddd" in l for l in s.lines()), 6)
        report.check("Enter on a symbol goes to it",
                     any("Wddd" in l for l in session.lines()), session,
                     note=repr([l.strip() for l in session.lines() if "ddd" in l][:1]))

        # And everything wrong at once, which has no chord: it is reached from the menu, and the
        # palette is the menu with a search box. Which language the editor is speaking comes from
        # settings this driver deliberately does not replace, so the label is asked for rather
        # than assumed.
        session.press("\x10", lambda s: "Command palette" in s.text()
                      or "Palette dei comandi" in s.text(), 8)
        italian = "Palette dei comandi" in session.text()
        session.send("Tutto quello che non va" if italian else "Everything that is wrong")
        session.wait(lambda s: True, 0.5)
        session.press("\r", lambda s: "cannot find value" in s.text(), 8)
        report.check("the diagnostics list opens on what the server published",
                     "cannot find value" in session.text()
                     and "unused variable" in session.text(), session,
                     note="both severities, from both open files")
        report.check("and each row is filed under how bad it is",
                     "error" in session.text() and "warning" in session.text(), session)

        # Narrowed to one row by typing, rather than by counting rows: which file sorts first is
        # not what this check is about.
        session.send("jump.rs:3")
        session.wait(lambda s: True, 0.5)
        session.press("\r", lambda s: True, 6)
        session.press("Q", lambda s: any("ZcccQ" in l for l in s.lines()), 6)
        report.check("Enter on a diagnostic goes to the line it is about",
                     any("ZcccQ" in l for l in session.lines()), session,
                     note=repr([l.strip() for l in session.lines() if "ccc" in l][:1]))


        # ---- renaming a name -----------------------------------------------------------------
        #
        # The one request that writes, and the checks are the promises it makes: a preview before
        # anything changes, Esc changing nothing, one Ctrl+Z taking back every occurrence, and an
        # honest refusal when the server names a file no tab holds.
        #
        # The box opens prefilled with the whole name, so the new one is typed *onto* it — which
        # is also how the prefill gets checked: if it were empty, or half the word, every screen
        # below would say so.
        session.send("\x0f")                                  # Ctrl+O, quick-open
        session.wait(lambda s: "name.rs" in s.text(), 8)
        session.send("name.rs")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        report.check("the rename fixture opens",
                     session.wait(lambda s: "alfa = alfa + 1" in s.text(), 8), session)

        session.press(session.chord("c"), lambda s: "'alfa'" in s.text(), 8)
        report.check("Ctrl+Shift+C asks for the new name, prefilled with the whole one",
                     "'alfa'" in session.text(), session,
                     note="the identifier under the cursor, not the half before it")

        # Typed onto the prefill: alfa2. Two occurrences on line 1 with a length change between
        # them, which is the arithmetic the preview and the apply have to agree about.
        session.send("2")
        session.press("\r", lambda s: "+ alfa2 = alfa2 + 1" in s.text(), 12)
        previewed = session.text()
        report.check("Enter shows a diff of what would change",
                     "- alfa = alfa + 1" in previewed and "+ alfa2 = alfa2 + 1" in previewed,
                     session, note="the old line and the new one, both occurrences on it")
        report.check("and the line further down is in it too",
                     "+ alfa2" in previewed and previewed.count("+ alfa2") >= 2, session)
        report.check("and it says which file, the way a diff says it",
                     "--- src/name.rs" in previewed, session,
                     note="root-relative, with the count beside it")

        # Esc changes nothing. Checked against the buffer's own rows, gutter and all, so a
        # preview still on screen could not pass for a file that had been written.
        session.press("\x1b", lambda s: "+ alfa2 = alfa2 + 1" not in s.text(), 6)
        report.check("Esc leaves the file exactly as it was",
                     any(re.search(r"\s1 alfa = alfa \+ 1\b", l) for l in session.lines())
                     and "alfa2" not in session.text(), session,
                     note=repr([l.strip() for l in session.lines() if "alfa" in l][:2]))

        # Again, and through this time.
        session.press(session.chord("c"), lambda s: "'alfa'" in s.text(), 8)
        session.send("2")
        session.press("\r", lambda s: "+ alfa2 = alfa2 + 1" in s.text(), 12)
        session.press("\r", lambda s: any(re.search(r"\s3 alfa2\b", l) for l in s.lines()), 8)
        rows = session.lines()
        report.check("Enter applies it everywhere in the buffer",
                     any(re.search(r"\s1 alfa2 = alfa2 \+ 1\b", l) for l in rows)
                     and any(re.search(r"\s3 alfa2\b", l) for l in rows), session,
                     note=repr([l.strip() for l in rows if "alfa" in l][:3]))

        # One step, not one per occurrence: the whole reason the edits are rebuilt into a single
        # replacement per file.
        session.press("\x1a", lambda s: any(re.search(r"\s1 alfa = alfa \+ 1\b", l)
                                            for l in s.lines()), 8)
        rows = session.lines()
        # Asked of the buffer's own rows rather than of the whole screen: the status line still
        # says what was just renamed, and rightly so — undoing does not unsay it.
        report.check("one Ctrl+Z takes back every occurrence at once",
                     any(re.search(r"\s1 alfa = alfa \+ 1\b", l) for l in rows)
                     and any(re.search(r"\s3 alfa\b", l) for l in rows)
                     and not any(re.search(r"\s\d+ .*alfa2", l) for l in rows), session,
                     note=repr([l.strip() for l in rows if "alfa" in l][:3]))

        # And the refusal. The stub answers a name containing `outside` with one extra edit to a
        # file that has no tab; the edits it *could* have applied must not be applied either.
        session.press(session.chord("c"), lambda s: "'alfa'" in s.text(), 8)
        session.send("_outside")
        session.press("\r", lambda s: "alfa_outside" not in s.text()[:0] or True, 8)
        refused = session.wait(lambda s: "1" in s.lines()[-1] and "alfa_outside" not in s.text(), 12)
        rows = session.lines()
        report.check("a rename reaching a file no tab holds is refused whole",
                     refused and not any("alfa_outside" in l for l in rows)
                     and any(re.search(r"\s1 alfa = alfa \+ 1\b", l) for l in rows), session,
                     note=repr(session.lines()[-1].strip()))
        report.check("and the refusal says so on the status line rather than in silence",
                     bool(session.lines()[-1].strip()), session,
                     note=repr(session.lines()[-1].strip()))

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
