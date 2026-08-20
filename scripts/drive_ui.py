#!/usr/bin/env python3
"""Drive the real CleeCode binary in a pseudo-terminal and check what lands on the screen.

    python3 scripts/drive_ui.py [path/to/clee]      # default: target/debug/clee

Why this exists. `cargo test` covers the pure functions — ranking, key routing, hit-testing,
where a box goes — but it cannot build an `App`: constructing one spawns two real PTYs and reads
the user's own settings from disk, so a test that did it would depend on the machine it ran on.
That leaves the wiring between those functions untested, and the wiring is where a feature is
either usable or not. This runs the shipped binary the way a person does, renders its output with
pyte, and reads the resulting grid of characters back.

Needs pyte (`pip install pyte`) and a Unix pty, so macOS and Linux. Not wired into `cargo test`
or CI: it waits on conditions with timeouts, and a flaky gate is worse than an honest manual one.

Two things to know before adding a check. CleeCode asks the terminal what it can do — device
attributes, the kitty keyboard and graphics protocols — and draws its first frame only once those
queries have timed out, because nothing here answers them; so wait on a *condition*, never on the
clock. And `▶` alone does not find the completion popup: the toolbar's "▶ Run" button carries one
the whole time, which is why `popup_open` looks for the marker hard against a box border.
"""

import fcntl
import os
import pty
import re
import shutil
import signal
import struct
import sys
import tempfile
import termios
import time

ROWS, COLS = 30, 110

try:
    import pyte
except ImportError:
    sys.exit("needs pyte:  pip install pyte")


class Session:
    """A CleeCode running in a pty, with a rendered picture of its screen."""

    def __init__(self, binary, root):
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.environ["TERM"] = "xterm-256color"
            os.environ["SHELL"] = "/bin/sh"
            os.chdir(root)
            try:
                os.execv(binary, [binary, "."])
            finally:
                # execv only returns if it failed, and a child falling through here would go on
                # running the rest of this script as though it were the driver.
                os._exit(127)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
        self.screen = pyte.Screen(COLS, ROWS)
        self.stream = pyte.ByteStream(self.screen)
        os.set_blocking(self.fd, False)

    # ---- reading the screen ----------------------------------------------------------

    def drain(self):
        """Feed everything waiting on the pty into the screen."""
        while True:
            try:
                data = os.read(self.fd, 1 << 16)
            except (BlockingIOError, OSError):
                return
            if not data:
                return
            self.stream.feed(data)

    def wait(self, predicate, timeout=10.0, settle=0.25):
        """Read until `predicate(self)` holds, then a moment longer so the frame finishes."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            self.drain()
            if predicate(self):
                quiet = time.time() + settle
                while time.time() < quiet:
                    self.drain()
                    time.sleep(0.02)
                return True
            time.sleep(0.03)
        return False

    def send(self, keys):
        os.write(self.fd, keys.encode() if isinstance(keys, str) else keys)

    def press(self, keys, predicate, timeout=6.0):
        self.send(keys)
        return self.wait(predicate, timeout)

    def lines(self):
        return [self.screen.display[y].rstrip() for y in range(ROWS)]

    def text(self):
        return "\n".join(self.lines())

    # ---- reading particular things off it ---------------------------------------------

    def buffer_line(self, n):
        """The text of buffer line `n`, taken from the editor pane past its gutter."""
        found = re.search(r"│\s*%d ([^│]*)" % n, self.text())
        return found.group(1).rstrip() if found else None

    def popup_open(self):
        return re.search(r"│▶ [A-Za-z_]", self.text()) is not None

    def popup_words(self):
        """The candidates the completion popup is offering, best first."""
        return [
            word
            for line in self.lines()
            for _, word in re.findall(r"│(▶ | {2})([A-Za-z_][A-Za-z_0-9]*) *│", line)
        ]

    def close(self):
        """Take the session down without ever blocking.

        The pty child is a session leader, so the signal goes to the whole group — CleeCode's
        own shells are in it, and they hold the slave open. Reaping is then polled rather than
        waited on: a harness that can hang is worse than no harness, and there is nothing left
        worth waiting for once the signal is sent.
        """
        try:
            os.close(self.fd)
        except OSError:
            pass
        for kill in (lambda: os.killpg(os.getpgid(self.pid), signal.SIGKILL),
                     lambda: os.kill(self.pid, signal.SIGKILL)):
            try:
                kill()
                break
            except OSError:
                continue
        deadline = time.time() + 3
        while time.time() < deadline:
            try:
                if os.waitpid(self.pid, os.WNOHANG)[0]:
                    return
            except OSError:
                return
            time.sleep(0.05)


class Report:
    def __init__(self):
        self.failed = []

    def check(self, name, ok, session=None, note=""):
        print(("  PASS  " if ok else "  FAIL  ") + name + (f"   {note}" if note else ""))
        if not ok:
            self.failed.append(name)
            if session is not None:
                self.show("screen at failure: " + name, session)

    @staticmethod
    def show(label, session):
        print(f"\n===== {label} =====")
        for y, line in enumerate(session.lines()):
            if line:
                print(f"{y:2} |{line}")


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
    report.check("words from the buffer are offered too", "config_path" in session.text(), session)

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


def main():
    binary = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else "target/debug/clee")
    if not os.access(binary, os.X_OK):
        sys.exit(f"no runnable binary at {binary} — cargo build first")

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
        shutil.rmtree(root, ignore_errors=True)

    print()
    if report.failed:
        print("FAILED: " + ", ".join(report.failed))
        return 1
    print("ALL CHECKS PASSED")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except BrokenPipeError:
        # Piped into `head`, which walked off before the screen dump finished.
        os._exit(0)
