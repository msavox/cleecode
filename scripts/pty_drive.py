"""Driving CleeCode in a pseudo-terminal, and reading back what it drew.

Shared by the drivers beside this file. Rendering with pyte means the assertions are about the
grid of characters — and their colours and attributes — that a person would see, rather than
about the escape sequences that produced it.

Two things to know before writing a check. CleeCode asks the terminal what it can do (device
attributes, the kitty keyboard and graphics protocols) and draws its first frame only once those
queries time out, because nothing in a bare pty answers them — so wait on a *condition*, never on
the clock. And `▶` alone does not identify the completion popup: the toolbar's "▶ Run" button
carries one the whole time.
"""

import fcntl
import os
import pty
import re
import signal
import struct
import sys
import termios
import time

ROWS, COLS = 30, 110

try:
    import pyte
except ImportError:
    sys.exit("needs pyte:  pip install pyte")


class Session:
    """A CleeCode running in a pty, with a rendered picture of its screen."""

    def __init__(self, binary, root, env=None):
        # Its own config directory, inside the throwaway project. Without this a driver reads the
        # settings of whoever is running it — so a check would depend on whether they happen to
        # have turned the feature off — and CleeCode's resume would reopen their last project
        # instead of the fixture. That is not hypothetical: it produced a run where every check
        # failed against an empty tree, which looks exactly like a broken editor.
        self.config = os.path.join(root, ".config")
        os.makedirs(self.config, exist_ok=True)
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.environ["TERM"] = "xterm-256color"
            os.environ["SHELL"] = "/bin/sh"
            os.environ["XDG_CONFIG_HOME"] = self.config
            os.environ.update(env or {})
            os.chdir(root)
            try:
                os.execv(binary, [binary, "."])
            finally:
                # execv only returns if it failed, and a child falling through here would go on
                # running the rest of the driver as though it were the driver.
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

    def cells(self, row):
        """The row as pyte Char objects, which carry colour and attributes as well as text."""
        return [self.screen.buffer[row][x] for x in range(COLS)]

    def row_of(self, needle):
        """The first screen row containing `needle`, or None."""
        for y, line in enumerate(self.lines()):
            if needle in line:
                return y
        return None

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
        waited on: a harness that can hang is worse than no harness.
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

    def finish(self):
        print()
        if self.failed:
            print("FAILED: " + ", ".join(self.failed))
            return 1
        print("ALL CHECKS PASSED")
        return 0


def binary_from_argv(argv):
    path = os.path.abspath(argv[1] if len(argv) > 1 else "target/debug/clee")
    if not os.access(path, os.X_OK):
        sys.exit(f"no runnable binary at {path} — cargo build first")
    return path
