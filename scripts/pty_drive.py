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

# The window a driver gets unless it asks for another. Wide enough that CleeCode's own presets
# choose their side-by-side shape, which is what most checks want to be looking at.
ROWS, COLS = 30, 110

# Every timeout in this file is written for a developer's machine and multiplied by this. The
# drivers' waits are all conditions, so a fast machine never pays for the slack — a wait returns
# the moment its predicate holds — but a CI runner pays dearly without it: on a fresh macos-14
# image the first command a brand-new shell executes has been seen to take over thirty seconds
# (cold dyld caches and the malware scan, neither of them the editor's doing), and a 52 MB write
# on ubuntu's disks outlives a patience that a laptop's SSD made look generous. Stretching the
# clock everywhere at once, from the environment, keeps the checks' evidence exactly as it is
# and fixes only their impatience — the alternative is finding these one timeout at a time, one
# red run each.
PATIENCE = float(os.environ.get("CLEE_DRIVE_PATIENCE", "1"))

try:
    import pyte
except ImportError:
    sys.exit("needs pyte:  pip install pyte")


class Session:
    """A CleeCode running in a pty, with a rendered picture of its screen."""

    def __init__(self, binary, root, env=None, args=None, cols=COLS, rows=ROWS):
        # Its own config directory, inside the throwaway project. Without this a driver reads the
        # settings of whoever is running it — so a check would depend on whether they happen to
        # have turned the feature off — and CleeCode's resume would reopen their last project
        # instead of the fixture. That is not hypothetical: it produced a run where every check
        # failed against an empty tree, which looks exactly like a broken editor.
        self.cols, self.rows = cols, rows
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
                os.execv(binary, [binary] + list(args or []) + ["."])
            finally:
                # execv only returns if it failed, and a child falling through here would go on
                # running the rest of the driver as though it were the driver.
                os._exit(127)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        self.screen = pyte.Screen(cols, rows)
        self.stream = pyte.ByteStream(self.screen)
        os.set_blocking(self.fd, False)

    # ---- reading the screen ----------------------------------------------------------

    # What a real terminal answers when asked what it can do. Without these CleeCode waits for
    # each query to time out before drawing its first frame — and, more importantly, it only
    # enables the disambiguating key protocol when the terminal says it has one. Every
    # application shortcut in CleeCode is a Ctrl+Shift chord, and those cannot be encoded at all
    # in what terminals have sent since VT100, so without this reply not one of them is reachable
    # and a driver can only ever test the keys that were already unambiguous.
    REPLIES = [
        (b"\x1b[?u", b"\x1b[?1u"),        # kitty keyboard protocol: yes, flag 1
        (b"\x1b[c", b"\x1b[?62;c"),       # primary device attributes: a VT220
        (b"\x1b[5n", b"\x1b[0n"),         # status report: fine, thanks
    ]

    def drain(self):
        """Feed everything waiting on the pty into the screen, answering its questions."""
        while True:
            try:
                data = os.read(self.fd, 1 << 16)
            except (BlockingIOError, OSError):
                return
            if not data:
                return
            for query, answer in self.REPLIES:
                if query in data:
                    try:
                        os.write(self.fd, answer)
                    except OSError:
                        pass
            self.stream.feed(data)

    def chord(self, letter):
        """Ctrl+Shift+<letter>, in the encoding that can carry it.

        `CSI unicode ; modifiers u`, where the modifier field is 1 plus the bits: shift 1,
        alt 2, ctrl 4. So Ctrl+Shift is 1+1+4 = 6."""
        return "\x1b[%d;6u" % ord(letter.lower())

    def wait(self, predicate, timeout=10.0, settle=0.25):
        """Read until `predicate(self)` holds, then a moment longer so the frame finishes.

        The timeout is scaled by PATIENCE, the settle is not: the first is how long a condition
        may take on the machine running this, the second is how long a finished frame takes to
        stop moving, which is the editor's clock rather than the runner's."""
        deadline = time.time() + timeout * PATIENCE
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

    def quit(self, timeout=20.0):
        """Ctrl+Q, then the status the editor's process ends with.

        Worth measuring because somebody reads it. Started from the Dock, CleeCode is given a
        terminal window of its own, and that window closes itself when the process inside it
        ends — as long as it ended the way a program that was asked to stop ends. So the exit
        status of a quit the user requested is part of what they see: it has to be a plain 0.

        Returns the `waitpid` status, or None if the editor was still running when the wait ran
        out. The pty is deliberately left open throughout: closing it first hangs the editor up,
        which would measure a different ending than the one being asked about."""
        self.send("\x11")
        deadline = time.time() + timeout * PATIENCE
        while time.time() < deadline:
            self.drain()
            try:
                pid, status = os.waitpid(self.pid, os.WNOHANG)
            except OSError:
                return None
            if pid:
                return status
            time.sleep(0.03)
        return None

    @staticmethod
    def describe_status(status):
        """A `waitpid` status as a phrase to put beside a failed check."""
        if status is None:
            return "still running"
        if os.WIFEXITED(status):
            return "exit %d" % os.WEXITSTATUS(status)
        if os.WIFSIGNALED(status):
            return "killed by signal %d" % os.WTERMSIG(status)
        return "status %d" % status

    def lines(self):
        """The screen as text, rendered from the cell buffer.

        Not pyte's own `display`, for two reasons. It raises on a cell whose character is the
        empty string — which happens beside a double-width one, and took a 190-column window to
        show up — and it accounts for wide characters by shifting what follows, so a column index
        from `display` and one from the buffer would not agree. Rendering both from the buffer
        means a position found in the text is the position to ask `cells` about."""
        return [
            "".join(cell.data or " " for cell in self.cells(y)).rstrip()
            for y in range(self.rows)
        ]

    def text(self):
        return "\n".join(self.lines())

    def cells(self, row):
        """The row as pyte Char objects, which carry colour and attributes as well as text."""
        return [self.screen.buffer[row][x] for x in range(self.cols)]

    def full_line(self, y):
        """The row as text, without the rstrip `lines()` does — so a column index always lands
        on the character that is drawn there, including in the blank right-hand end of a row."""
        return "".join(cell.data or " " for cell in self.cells(y))

    def frame_of(self, needle):
        """The lines inside the bordered frame that `needle` is drawn in.

        Slicing a row at its last `││` is how these drivers used to mean "the right-hand pane",
        and it lands in whichever frame happens to be rightmost *on that row* — which, on a
        screen whose frames do not line up, is often not the one wanted. For a check that some
        text is absent, reading the wrong frame is a pass for free. This walks out to the borders
        of the frame the text is really in, so what is being read is never in doubt."""
        lines = self.lines()
        here = next(((y, line.index(needle)) for y, line in enumerate(lines) if needle in line), None)
        if here is None:
            return []
        y, x = here
        row = self.full_line(y)
        left = row.rfind("\u2502", 0, x)
        right = row.find("\u2502", x)
        if left < 0 or right < 0:
            return []
        top, bottom = y, y
        while top > 0 and self.full_line(top - 1)[left] == "\u2502":
            top -= 1
        while bottom + 1 < self.rows and self.full_line(bottom + 1)[left] == "\u2502":
            bottom += 1
        return [self.full_line(r)[left + 1:right] for r in range(top, bottom + 1)]

    def row_of(self, needle):
        """The first screen row containing `needle`, or None."""
        for y, line in enumerate(self.lines()):
            if needle in line:
                return y
        return None

    def column_of(self, needle):
        """The column `needle` starts at, or None. Which frame something is in is a question
        about columns as often as about rows: a pane on the right and a pane underneath both
        have their title near the top."""
        for line in self.lines():
            at = line.find(needle)
            if at >= 0:
                return at
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
    # Said out loud, because the default is a path and not a build: a stale target/debug/clee
    # sits there being executable for as long as nobody rebuilds it, and every driver run
    # against it passes or fails on behalf of a version nobody is looking at.
    print(f"driving {path}")
    return path
