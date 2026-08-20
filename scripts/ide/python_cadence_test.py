#!/usr/bin/env python3
"""How often the Python workspace hook fires, which is the thing it is easiest to get wrong.

    cd assets/python && python3 ../../scripts/ide/python_cadence_test.py

Runs against **PyREPL**, the REPL a user actually gets, and refuses to run under the basic one.
That distinction is the whole point of this file: the first design fired once per statement when
measured with `TERM=dumb`, which quietly makes CPython fall back to the basic REPL, and about
twenty times per statement in real use — where each of those is a full workspace scan and a
rewrite of every open figure's PNG.

Checks three things a snapshot trigger has to get right:

  · one snapshot per statement, however many times the prompt is drawn;
  · the snapshot sees the statement's *result*, not the state before it — an audit hook on
    `exec` alone fires too early, because it is raised on the way in;
  · typing without pressing Enter produces none, because the prompt is redrawn as you type.
"""

import json
import os
import pty
import struct
import sys
import termios
import time
import fcntl
import shutil
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
LIB = os.path.abspath(os.path.join(HERE, "..", "..", "assets", "python"))


class Repl:
    def __init__(self, snapshot, figdir):
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.environ.update(
                TERM="xterm-256color",
                PYTHONSTARTUP=os.path.join(LIB, "pythonstartup.py"),
                PYTHONPATH=LIB,
                CLEECODE_PY_WS=snapshot,
                CLEECODE_PY_FIGS=figdir,
            )
            # Explicitly *not* set: it would select the basic REPL and make this test agree
            # with the measurement it exists to contradict.
            os.environ.pop("PYTHON_BASIC_REPL", None)
            try:
                os.execv(sys.executable, [sys.executable])
            finally:
                os._exit(127)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 200, 0, 0))
        os.set_blocking(self.fd, False)
        self.seen = b""

    def pump(self, seconds):
        end = time.time() + seconds
        while time.time() < end:
            try:
                self.seen += os.read(self.fd, 1 << 16)
            except (BlockingIOError, OSError):
                time.sleep(0.02)

    def send(self, text, settle=0.6):
        os.write(self.fd, text.encode())
        self.pump(settle)

    def close(self):
        try:
            os.killpg(os.getpgid(self.pid), 9)
        except OSError:
            pass
        try:
            os.waitpid(self.pid, os.WNOHANG)
        except OSError:
            pass


def read(path):
    try:
        with open(path) as handle:
            return json.load(handle)
    except Exception:
        return None


def main():
    root = tempfile.mkdtemp(prefix="clee_cadence_")
    snapshot = os.path.join(root, "ws.json")
    figdir = os.path.join(root, "figs")
    failed = []

    def check(name, ok, note=""):
        print(("  PASS  " if ok else "  FAIL  ") + name + (f"   {note}" if note else ""))
        if not ok:
            failed.append(name)

    repl = Repl(snapshot, figdir)
    try:
        repl.pump(2.0)
        check("the hook installs itself from PYTHONSTARTUP", read(snapshot) is not None,
              note="a snapshot exists before anything was typed")

        base = (read(snapshot) or {}).get("seq", 0)
        statements = ["a = 1", "b = 2", "c = a + b", "d = [3, 1, 4]", "a = a + 10"]
        for statement in statements:
            repl.send(statement + "\n")
        after = read(snapshot)
        check("one snapshot per statement, not one per prompt redraw",
              after and after["seq"] - base == len(statements),
              note=f"seq went {base} -> {after['seq'] if after else '?'} "
                   f"for {len(statements)} statements")

        names = {v["name"]: v for v in (after or {}).get("vars", [])}
        check("the snapshot shows what the statement did, not what came before it",
              names.get("a", {}).get("max") == 11 and "d" in names,
              note="a = a + 10 has to be visible in the snapshot it triggered")

        # The prompt is restringified as you type. Nothing was run, so nothing is owed.
        before_typing = read(snapshot)["seq"]
        for ch in "print('not run')":
            repl.send(ch, settle=0.12)
        repl.send("\x15", settle=0.5)                 # Ctrl+U wipes the line unrun
        check("typing without running produces no snapshots",
              read(snapshot)["seq"] == before_typing,
              note=f"seq stayed at {before_typing} through 16 keystrokes")
    finally:
        repl.close()
        shutil.rmtree(root, ignore_errors=True)

    print()
    if failed:
        print("FAILED: " + ", ".join(failed))
        return 1
    print("ALL CHECKS PASSED")
    return 0


if __name__ == "__main__":
    sys.exit(main())
