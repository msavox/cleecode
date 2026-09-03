#!/usr/bin/env python3
"""Drive the real CleeCode binary, kill it the way a crash does, and ask for the work back.

    python3 scripts/drive_recover.py [path/to/clee]     # default: target/debug/clee

Everything else in the test suite can be checked inside one process. This cannot: the whole
feature is about what survives a process that stops existing, and a unit test necessarily runs in
one that does not. So the editor is started for real, typed into, and ended with `SIGKILL` — no
teardown, no `Drop`, no exit path, nothing the program could have done on the way out. Then a
second CleeCode is started on the same project and asked what it found.

`kill -9` is also the honest signal to use. A `SIGTERM` or a hangup would let the editor unwind
and could pass on the strength of code that never runs in the case this exists for; `SIGKILL`
cannot be caught, so what is on disk at that moment is exactly what the tick had written.

Four things are checked, and they are the four promises:

    the copy is written              — a dirty buffer appears in the recovery directory within
                                       one tick, holding the text that was typed
    the offer is made, and honest    — the next session lists the file, Enter puts the text back,
                                       the buffer is *dirty* (restoring is not saving), and one
                                       Ctrl+Z is the file that is on disk
    a clean save ends it             — saving removes the copy, and a session that exits cleanly
                                       leaves the next one with nothing to offer
    unnamed buffers too              — the work that used to die without a trace, since a buffer
                                       with no path is dropped from `last_open_files` entirely

The untitled case runs in a project of its own, deliberately: the same one would have a resume
from the earlier checks in it, and the buffer being typed into would be a reopened file rather
than the never-named buffer this is about.
"""

import os
import re
import shutil
import signal
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_drive import Report, Session, binary_from_argv  # noqa: E402


# What is on disk before anything is typed. One line, its own words, so "the disk version is
# back" can be read off the screen without ambiguity.
DISK = "line from disk\n"

# Typed into the file. Deliberately not a word that appears anywhere in CleeCode's own interface,
# so finding it on screen means the buffer and nothing else.
TYPED = "ZZALFA"

# Typed into the buffer that never had a name.
UNNAMED = "ZZBETA"


def copies(session):
    """Every recovery copy in this session's sandboxed config directory."""
    where = os.path.join(session.config, "cleecode", "recovery")
    if not os.path.isdir(where):
        return []
    return sorted(
        os.path.join(where, name)
        for name in os.listdir(where)
        if name.endswith(".clee-recovery")
    )


def until(session, condition, timeout=25.0):
    """Wait on something that is true of the *disk*, keeping the screen drained meanwhile.

    `Session.wait` asks a question about the picture; these checks ask questions about files the
    editor writes on a five-second tick. The pty still has to be read while waiting or the editor
    blocks on a full buffer and the tick never comes round.
    """
    deadline = time.time() + timeout
    while time.time() < deadline:
        session.drain()
        if condition():
            return True
        time.sleep(0.1)
    return False


def hard_kill(session):
    """End the editor the way a crash does: SIGKILL, then reap it.

    Reaping matters more than it looks. The offer for an unnamed buffer is made on the strength
    of its pid no longer being alive, and an unreaped child is a zombie that `sysinfo` still
    reports as a process — so a driver that skipped this would be testing the case where the
    entry is correctly *withheld*, and calling it a failure.
    """
    try:
        pgid = os.getpgid(session.pid)
    except OSError:
        pgid = None
    try:
        os.kill(session.pid, signal.SIGKILL)
    except OSError:
        pass
    deadline = time.time() + 5
    while time.time() < deadline:
        try:
            if os.waitpid(session.pid, os.WNOHANG)[0]:
                break
        except OSError:
            break
        time.sleep(0.02)
    # The shells CleeCode had running are in its process group and outlive it, holding the pty
    # open. Never the driver's own group, which is what the guard is for.
    if pgid is not None and pgid != os.getpgrp():
        try:
            os.killpg(pgid, signal.SIGKILL)
        except OSError:
            pass
    try:
        os.close(session.fd)
    except OSError:
        pass


def started(session, report, needle, name="the app draws its first frame"):
    """Wait for the real interface, not merely for pixels.

    The splash is drawn before anything else and is several non-empty lines of its own, so
    "something is on screen" is true of it too — and a key pressed at it is swallowed dismissing
    it. Waiting for something only the working window shows avoids both.
    """
    ok = session.wait(lambda s: needle in s.text(), timeout=25)
    report.check(name, ok, session)
    return ok


def open_file(session, report, name):
    """Ctrl+O, the name, Enter — and the file's own text on screen to prove it landed."""
    session.send("\x0f")
    session.wait(lambda s: name in s.text(), 8)
    session.send(name)
    session.wait(lambda s: True, 0.5)
    session.send("\r")
    ok = session.wait(lambda s: DISK.strip() in s.text(), 8)
    report.check(f"{name} opens", ok, session)
    return ok


def focus_editor(session):
    """The palette rather than a chord: Ctrl+Tab needs the disambiguating key protocol, and this
    reads the same on every terminal the drivers might one day run on."""
    session.send("\x10")
    session.wait(lambda s: "Focus editor" in s.text(), 6)
    session.send("focus editor")
    session.wait(lambda s: True, 0.4)
    session.send("\r")
    return session.wait(lambda s: "Focus editor" not in s.text(), 6)


def type_text(session, text):
    """Type, then Esc to put away the completion popup so it cannot sit over what is checked."""
    session.send(text)
    session.wait(lambda s: text in s.text(), 6)
    session.press("\x1b", lambda s: True, 0.4)


def main():
    binary = binary_from_argv(sys.argv)
    project = tempfile.mkdtemp(prefix="clee_recover_")
    scratch = tempfile.mkdtemp(prefix="clee_recover_untitled_")
    notes = os.path.join(project, "notes.txt")
    with open(notes, "w") as handle:
        handle.write(DISK)

    report = Report()
    session = None
    try:
        # ---- a dirty buffer is copied, and the copy survives the process -----------------------
        session = Session(binary, project)
        if not started(session, report, "notes.txt"):
            return report.finish()
        if not open_file(session, report, "notes.txt"):
            return report.finish()
        type_text(session, TYPED)
        report.check("the buffer is dirty after typing", "notes.txt*" in session.text(), session)

        written = until(session, lambda: len(copies(session)) == 1)
        report.check("a copy of the unsaved buffer appears within a tick", written, session,
                     note=f"{copies(session)}")
        if not written:
            return report.finish()
        copy = copies(session)[0]
        body = open(copy).read()
        header, _, text = body.partition("\n")
        report.check("the copy says which file it belongs to",
                     header.startswith("clee-recovery 1 file ") and header.endswith("notes.txt"),
                     note=repr(header))
        report.check("and holds what was typed, not what is on disk",
                     text.startswith(TYPED) and DISK.strip() in text, note=repr(text))
        # The file on disk is untouched: a recovery copy is not a save, and must never look like
        # one from the outside.
        report.check("the file itself was not written", open(notes).read() == DISK)

        hard_kill(session)
        session = None
        report.check("the copy is still there once the process is gone",
                     os.path.exists(copy) and open(copy).read() == body)

        # ---- the next session offers it, and restoring is not saving ----------------------------
        session = Session(binary, project)
        offered = session.wait(lambda s: "Unsaved work from a session that ended" in s.text(), 25)
        report.check("the next start offers the unsaved work", offered, session)
        if not offered:
            return report.finish()
        listed = session.text()
        report.check("the offer names the file and how old the copy is",
                     "notes.txt" in listed and re.search(r"notes\.txt\s+·\s+\S", listed) is not None,
                     session)

        back = session.press("\r", lambda s: TYPED in s.text(), 10)
        report.check("Enter puts the text back in the buffer", back, session)
        report.check("and leaves it dirty — restoring is not saving",
                     "notes.txt*" in session.text(), session)
        report.check("the file on disk is still the file on disk", open(notes).read() == DISK)
        report.check("the copy is taken off disk once it is in a buffer", not os.path.exists(copy))
        report.check("and the offer is gone with it",
                     "Unsaved work from a session that ended" not in session.text(), session)

        # One undo, and the buffer is what the file says. The whole replacement went in as a
        # single step for exactly this: recovering has to be as easy to take back as it was to
        # accept, or accepting it is a decision rather than a look.
        undone = session.press("\x1a", lambda s: TYPED not in s.text(), 8)
        report.check("one Ctrl+Z brings back the version that is on disk", undone, session)
        report.check("which is the file's own text, whole",
                     DISK.strip() in session.text(), session)

        # ---- a save ends the copy, and a clean exit leaves nothing behind -----------------------
        #
        # The buffer is dirty again after the undo, so the tick writes a fresh copy — which is
        # what makes the next check mean something rather than passing on an empty directory.
        again = until(session, lambda: len(copies(session)) == 1)
        report.check("an edited buffer is copied again on the next tick", again, session)
        saved = session.press("\x13", lambda s: "notes.txt*" not in s.text(), 10)
        report.check("Ctrl+S saves", saved, session)
        report.check("and a successful save removes the copy it no longer needs",
                     until(session, lambda: copies(session) == [], timeout=8), session,
                     note=f"{copies(session)}")

        status = session.quit()
        report.check("the editor quits cleanly", status is not None and os.WIFEXITED(status)
                     and os.WEXITSTATUS(status) == 0, note=Session.describe_status(status))
        session.close()

        after = Session(binary, project)
        session = after
        if started(after, report, "notes.txt", "the project opens again"):
            report.check("a session that ended cleanly leaves nothing to offer",
                         "Unsaved work from a session that ended" not in after.text(), after)
        after.close()
        session = None

        # ---- the buffer that used to die without a trace ------------------------------------
        #
        # Its own project, so the initial buffer really is the never-named one: in the project
        # above, the resume would reopen notes.txt over it.
        session = Session(binary, scratch)
        if not started(session, report, "▶", "the second project opens"):
            return report.finish()
        report.check("it starts on a buffer with no name", "[untitled]" in session.text(), session)
        report.check("the keyboard reaches the editor", focus_editor(session), session)
        type_text(session, UNNAMED)
        report.check("the unnamed buffer is dirty after typing",
                     "[untitled]*" in session.text(), session)

        written = until(session, lambda: len(copies(session)) == 1)
        report.check("an unnamed buffer is copied too — nothing else remembers it exists",
                     written, session, note=f"{copies(session)}")
        if not written:
            return report.finish()
        copy = copies(session)[0]
        report.check("its copy is keyed to the session that wrote it, not to a path",
                     os.path.basename(copy).startswith("untitled-"), note=os.path.basename(copy))
        report.check("and holds the typed text", UNNAMED in open(copy).read())

        hard_kill(session)
        session = None

        session = Session(binary, scratch)
        offered = session.wait(lambda s: "Unsaved work from a session that ended" in s.text(), 25)
        report.check("the next start offers the unnamed buffer back", offered, session)
        if offered:
            report.check("listed as what it is, with no file name to give",
                         "[untitled]" in session.text(), session)
            back = session.press("\r", lambda s: UNNAMED in s.text(), 10)
            report.check("Enter opens it as a buffer again", back, session)
            report.check("dirty, and still without a name",
                         "[untitled]*" in session.text(), session)
    finally:
        if session is not None:
            session.close()
        shutil.rmtree(project, ignore_errors=True)
        shutil.rmtree(scratch, ignore_errors=True)

    return report.finish()


if __name__ == "__main__":
    sys.exit(main())
