#!/usr/bin/env python3
"""Drive the real CleeCode binary through a replace across the project and look at the disk.

    python3 scripts/drive_replace.py [path/to/clee]     # default: target/debug/clee

Project search had no driver at all before this one, which is the first thing here: the half that
only reads is checked alongside the half that writes, because the whole design of the box is that
an empty replacement field leaves the search exactly the search it was.

The other half is the one CleeCode had never done before — writing files nobody has open. So the
checks are the promises that makes. A preview of every file before anything happens. Esc leaving
every fixture byte-identical *and* mtime-identical, because "nothing was changed" has to be true
of the filesystem and not only of the screen. One Ctrl+Z taking back a whole file's worth of
replacements in the open buffer. And, on disk, the two things a rewrite silently ruins: a CRLF
file coming back CRLF, and a file that ended without a newline still ending without one.

There is one fixture per concern, and every file that gets written is its own file. A check that
had to reason about which of the earlier writes had already happened would be a check nobody can
read — and the two disk fixtures are read back as *bytes*, since every question here is about
bytes that no screen would show.
"""

import os
import re
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_drive import Report, Session, binary_from_argv  # noqa: E402


# The file that gets a tab. Three occurrences, two of them on one line, which is the shape a
# sweep gets wrong quietly: the first replacement on a line moves the second.
OPEN_FILE = "alfa here\nand alfa alfa again\n"

# LF, and deliberately no final newline. Written out with one it would be a file that grew a
# character nobody typed — invisible on screen, and a whole-file diff in review.
DISK_LF = "one alfa line\nlast alfa line"

# CRLF, which is the other way a rewrite ruins a file it only meant to touch in three places.
DISK_CRLF = "alfa crlf one\r\nalfa crlf two\r\n"

# For the capture groups. Its own file and its own words: `$1` means nothing until it is resolved
# against a real match, and a literal dollar is exactly what it looks like when it is not.
GROUPS = "ada@lovelace\nalan@turing\n"

# More matching lines than a sweep will hold. The limit is 2000 hits, and a list that stopped
# there is part of the project — fine for a list of places to go, and not a thing to replace
# every occurrence in.
FLOOD = "zzflood\n" * 2100


def snapshot(paths):
    """Every fixture's bytes and mtime, for the checks about nothing having happened."""
    return {p: (open(p, "rb").read(), os.stat(p).st_mtime_ns) for p in paths}


def open_box(session, report, name):
    """Ctrl+Shift+H, waited on by the field that only this box has."""
    ok = session.press(session.chord("h"),
                       lambda s: "Replace with" in s.text() or "Sostituisci con" in s.text(), 8)
    report.check(name, ok, session)
    return ok


def status(session):
    return session.lines()[-1].strip()


def main():
    binary = binary_from_argv(sys.argv)
    root = tempfile.mkdtemp(prefix="clee_replace_")
    fixtures = {}
    for name, text in [("open_file.txt", OPEN_FILE), ("disk_lf.txt", DISK_LF),
                       ("disk_crlf.txt", DISK_CRLF), ("groups.txt", GROUPS),
                       ("flood.txt", FLOOD)]:
        path = os.path.join(root, name)
        # Binary mode throughout: the CRLF fixture is about the bytes, and text mode on some
        # platforms would translate the very thing being measured.
        with open(path, "wb") as handle:
            handle.write(text.encode())
        fixtures[name] = path
    every_file = list(fixtures.values())

    report = Report()
    session = Session(binary, root)
    try:
        started = session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20)
        report.check("the app draws its first frame", started, session)
        if not started:
            return 1

        session.send("\x0f")                                  # Ctrl+O, quick-open
        session.wait(lambda s: "open_file.txt" in s.text(), 8)
        session.send("open_file.txt")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        report.check("the fixture with a tab opens",
                     session.wait(lambda s: "alfa here" in s.text(), 8), session)

        # ---- the box ---------------------------------------------------------------------------
        #
        # One box, two fields. The second one existing is what makes this feature reachable at
        # all, and Tab is the only way to it — so the caret is asked where it went, rather than a
        # marker being read off the screen and hoping it means what it looks like.
        if not open_box(session, report, "Ctrl+Shift+H opens the box, now with a replace field"):
            return report.finish()
        first = session.screen.cursor.y
        session.press("\t", lambda s: s.screen.cursor.y != first, 4)
        second = session.screen.cursor.y
        report.check("Tab reaches the replace field", second == first + 2, session,
                     note=f"caret moved from row {first} to row {second}")
        session.press("\x1b[Z", lambda s: s.screen.cursor.y == first, 4)
        report.check("and Shift+Tab comes back to the query",
                     session.screen.cursor.y == first, session)

        # ---- the search, unregressed -----------------------------------------------------------
        #
        # An empty replacement field is the whole of the difference between the two things this
        # box does. With it empty, Enter has to be the search that was always here: a list.
        session.send("alfa")
        session.press("\r", lambda s: "open_file.txt:1" in s.text(), 12)
        listed = session.text()
        report.check("with the replace field empty, Enter still opens the results list",
                     "open_file.txt:1" in listed and "disk_crlf.txt:1" in listed, session,
                     note="the jump picker, exactly as before")
        session.press("\x1b", lambda s: "disk_crlf.txt:1" not in s.text(), 6)

        before = snapshot(every_file)

        # ---- the preview -----------------------------------------------------------------------
        if not open_box(session, report, "the box opens again for the replace"):
            return report.finish()
        session.send("alfa")
        session.send("\t")
        session.send("beta")
        shown = session.press("\r", lambda s: "+ beta here" in s.text(), 15)
        report.check("Enter shows a diff instead of a list", shown, session)
        previewed = session.text()
        report.check("the diff groups the changes by file, both roads together",
                     "--- open_file.txt" in previewed and "--- disk_lf.txt" in previewed
                     and "--- disk_crlf.txt" in previewed, session,
                     note="the file with a tab and the two without one")
        report.check("a line matching twice is one pair of rows, both occurrences replaced",
                     "- and alfa alfa again" in previewed and "+ and beta beta again" in previewed,
                     session)
        report.check("and the title counts what it is about to do",
                     "alfa → beta" in previewed
                     and ("7 change" in previewed or "7 modifiche" in previewed), session,
                     note=repr([l.strip() for l in session.lines() if "→" in l][:1]))

        # ---- Esc changes nothing ---------------------------------------------------------------
        #
        # Asked of the filesystem, not of the screen. The mtimes are half the check: a file
        # rewritten with the bytes it already had is still a file something wrote, and a build
        # watching the tree would have noticed even if this driver did not.
        session.press("\x1b", lambda s: "+ beta here" not in s.text(), 8)
        after = snapshot(every_file)
        report.check("Esc leaves every file byte-identical and untouched",
                     after == before, session,
                     note=repr([os.path.basename(p) for p in every_file
                                if after[p] != before[p]]))
        report.check("and says so rather than going quiet", bool(status(session)), session,
                     note=repr(status(session)))

        # ---- and through ------------------------------------------------------------------------
        if not open_box(session, report, "the box opens a third time"):
            return report.finish()
        session.send("alfa")
        session.send("\t")
        session.send("beta")
        session.press("\r", lambda s: "+ beta here" in s.text(), 15)
        applied = session.press("\r", lambda s: "+ beta here" not in s.text()
                                and any(re.search(r"\s1 beta here\b", l) for l in s.lines()), 15)
        rows = session.lines()
        report.check("Enter changes the open buffer on screen", applied, session,
                     note=repr([l.strip() for l in rows if "beta" in l or "alfa" in l][:3]))
        report.check("both occurrences on one line went, in the buffer",
                     any(re.search(r"\s2 and beta beta again\b", l) for l in rows), session)
        report.check("the sentence afterwards counts the files it rewrote on disk",
                     "2" in status(session)
                     and ("disk" in status(session) or "disco" in status(session)), session,
                     note=repr(status(session)))

        # The file with a tab keeps its bytes: its replacement lives in the rope until somebody
        # saves it. That is not an oversight, it is the whole reason the buffer road exists — a
        # disk write under an open tab is reloaded within the second and takes the undo with it.
        report.check("the file with a tab was not written behind the buffer's back",
                     open(fixtures["open_file.txt"], "rb").read() == OPEN_FILE.encode(),
                     session, note="its change is in the rope, one Ctrl+Z away")

        lf = open(fixtures["disk_lf.txt"], "rb").read()
        crlf = open(fixtures["disk_crlf.txt"], "rb").read()
        report.check("the file with no tab was rewritten on disk, without growing a newline",
                     lf == b"one beta line\nlast beta line", session, note=repr(lf))
        report.check("and the CRLF file came back CRLF",
                     crlf == b"beta crlf one\r\nbeta crlf two\r\n", session, note=repr(crlf))

        # ---- one step of undo -------------------------------------------------------------------
        session.press("\x1a", lambda s: any(re.search(r"\s1 alfa here\b", l) for l in s.lines()), 8)
        rows = session.lines()
        report.check("one Ctrl+Z takes back the whole file's worth at once",
                     any(re.search(r"\s1 alfa here\b", l) for l in rows)
                     and any(re.search(r"\s2 and alfa alfa again\b", l) for l in rows)
                     and not any(re.search(r"\s\d+ .*beta", l) for l in rows), session,
                     note=repr([l.strip() for l in rows if "alfa" in l or "beta" in l][:3]))

        # ---- the groups -------------------------------------------------------------------------
        #
        # With Ctrl+N on, a `$1` is a group. The point of checking it here rather than trusting
        # the Find box's own test is that this replacement is worked out against a *line* found by
        # the project walk, not against a buffer — two different texts, one meaning.
        if not open_box(session, report, "the box opens for the pattern"):
            return report.finish()
        session.send("\x0e")                                  # Ctrl+N, read the query as a pattern
        session.send(r"(\w+)@(\w+)")
        session.send("\t")
        session.send("$2.$1")
        expanded = session.press("\r", lambda s: "+ lovelace.ada" in s.text(), 15)
        report.check("a $1 in the replacement expands the capture it stands for", expanded, session,
                     note=repr([l.strip() for l in session.lines() if "ada" in l][:2]))
        report.check("and the second line's groups are its own, not the first line's",
                     "+ turing.alan" in session.text(), session)
        session.press("\x1b", lambda s: "+ lovelace.ada" not in s.text(), 8)
        report.check("cancelling the pattern sweep leaves that file alone",
                     open(fixtures["groups.txt"], "rb").read() == GROUPS.encode(), session)

        # ---- more than a sweep can hold ---------------------------------------------------------
        #
        # A list that stopped at its limit is still useful: you go to one of the rows. A sweep is
        # a claim about *every* occurrence, so half of them is not a smaller version of the same
        # thing — and the refusal has to say which of the two the reader is looking at.
        if not open_box(session, report, "the box opens for the flood"):
            return report.finish()
        session.send("\x0e")                                  # Ctrl+N off again: a literal query
        session.send("zzflood")
        session.send("\t")
        session.send("x")
        refused = session.press("\r", lambda s: "2000" in status(s), 30)
        report.check("a search that stopped at its limit refuses to sweep", refused, session,
                     note=repr(status(session)))
        report.check("and it refuses instead of previewing part of the project",
                     "--- flood.txt" not in session.text()
                     and open(fixtures["flood.txt"], "rb").read() == FLOOD.encode(), session)
    finally:
        session.close()
        shutil.rmtree(root, ignore_errors=True)
    return report.finish()


if __name__ == "__main__":
    sys.exit(main())
