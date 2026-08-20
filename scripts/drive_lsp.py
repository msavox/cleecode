#!/usr/bin/env python3
"""Drive the real CleeCode binary against a stub language server and look at the screen.

    python3 scripts/drive_lsp.py [path/to/clee]     # default: target/debug/clee

The client has its own tests in src/lsp.rs, up to and including talking to a real process. What
none of them can answer is the question that matters to somebody using the editor: *does a
squiggle appear under the wrong word*. That needs the whole program — the poll in the frame loop,
the debounce, the span surgery in the renderer — and a screen to look at.

There is no rust-analyzer here, and installing one would make this a test of rust-analyzer. So
scripts/lsp_stub.py is put on PATH under the name CleeCode looks for. It answers the handshake
and publishes two diagnostics against whatever file it is told was opened, which also proves the
path-to-URI encoding round-tripped: a stub inventing its own URI would pass with that broken.

pyte tracks colour and attributes per cell, not just characters, so "underlined" and "red" are
things this can actually check rather than infer.
"""

import os
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


def main():
    binary = binary_from_argv(sys.argv)
    root = tempfile.mkdtemp(prefix="clee_lsp_")
    os.makedirs(os.path.join(root, "src"), exist_ok=True)
    with open(os.path.join(root, "Cargo.toml"), "w") as handle:
        handle.write('[package]\nname = "probe"\nversion = "0.1.0"\nedition = "2021"\n')
    with open(os.path.join(root, "src", "main.rs"), "w") as handle:
        handle.write(SAMPLE)
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
