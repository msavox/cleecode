#!/usr/bin/env python3
"""Drive a real CleeCode and a real `clee --mcp` against each other.

    python3 scripts/drive_mcp.py [path/to/clee]     # default: target/debug/clee

The protocol has its own tests in src/mcp.rs, and they cover the conversation thoroughly — but
they cover it with the server as a function over two buffers, which is exactly the part that
cannot be wrong in an interesting way. What no unit test can answer is whether the two *processes*
find each other: whether the editor really creates the session directory, really publishes the
file you opened into it, really notices the request file, and really puts the file on screen
without stealing the keyboard from you. That is four separate places where a path or a poll can
be wrong, and all four are invisible to `cargo test`.

So this launches the editor in a pty, finds its session directory the way an agent's environment
would name it, speaks NDJSON JSON-RPC to a `clee --mcp` of its own, and then looks at the screen.

The window is deliberately wide. Opening a file "beside" means opening the split, and CleeCode
only opens one unasked when there is room for it — in a narrow window the file lands in the pane
you are looking at, which is correct behaviour and unusable as a check on "beside".
"""

import json
import os
import select
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_drive import Report, Session, binary_from_argv  # noqa: E402

# Wide enough that CleeCode opens the split by itself: see the module docstring.
COLS, ROWS = 140, 34

MAIN = """fn main() {
    let anchor_main = 1;
    println!("{anchor_main}");
}
"""

# Distinctive on sight and absent from the other file, so "did it appear on screen" is a question
# with one answer rather than a question about which pane a common word landed in.
#
# Long, and the line asked for is deep in it, on purpose: a file opened at line 1 and a file
# opened at the line the agent named look identical when the file is four lines long. Only a line
# that cannot be on screen unless the editor scrolled to it proves the line travelled.
OPEN_AT = 150
OTHER = "fn helper() {\n" + "".join(
    "    let zzz_marker_%d = %d;\n" % (n, n) for n in range(2, 201)
) + "}\n"


class Mcp:
    """A `clee --mcp` on a pipe, spoken to in newline-delimited JSON-RPC."""

    def __init__(self, binary, session_dir):
        env = dict(os.environ)
        if session_dir is None:
            env.pop("CLEE_SESSION", None)
        else:
            env["CLEE_SESSION"] = session_dir
        self.proc = subprocess.Popen(
            [binary, "--mcp"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            env=env,
            text=True,
            bufsize=1,
        )
        self.next_id = 0

    def send(self, method, params=None, notify=False):
        """One message out; the reply, or None for a notification.

        Reading is guarded by `select`, because a server that never answers must cost this
        driver a failed check rather than a hang somebody has to notice and kill."""
        message = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            message["params"] = params
        if not notify:
            self.next_id += 1
            message["id"] = self.next_id
        self.proc.stdin.write(json.dumps(message) + "\n")
        self.proc.stdin.flush()
        if notify:
            return None
        ready, _, _ = select.select([self.proc.stdout], [], [], 10.0)
        if not ready:
            return None
        line = self.proc.stdout.readline()
        return json.loads(line) if line.strip() else None

    def tool(self, name, arguments=None):
        """A tools/call, unwrapped to (parsed content, isError)."""
        reply = self.send("tools/call", {"name": name, "arguments": arguments or {}})
        if reply is None or "result" not in reply:
            return None, True
        result = reply["result"]
        text = ""
        for block in result.get("content", []):
            if block.get("type") == "text":
                text += block.get("text", "")
        try:
            parsed = json.loads(text)
        except json.JSONDecodeError:
            parsed = text
        return parsed, bool(result.get("isError"))

    def close(self):
        """Closing stdin is how an MCP server is told to stop; it must then exit by itself."""
        try:
            self.proc.stdin.close()
        except OSError:
            pass
        try:
            return self.proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            return None


def wait_for(predicate, timeout=15.0, session=None):
    """Poll until `predicate()` holds, keeping the editor's pty drained while we wait.

    Draining matters: CleeCode only reaches the poll that publishes its state if it is not
    blocked writing frames into a pty nobody is reading."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        if session is not None:
            session.drain()
        if predicate():
            return True
        time.sleep(0.05)
    return False


def read_json(path):
    try:
        with open(path) as handle:
            return json.load(handle)
    except (OSError, json.JSONDecodeError):
        return None


def main():
    binary = binary_from_argv(sys.argv)
    root = tempfile.mkdtemp(prefix="clee_mcp_")
    os.makedirs(os.path.join(root, "src"), exist_ok=True)
    with open(os.path.join(root, "src", "main.rs"), "w") as handle:
        handle.write(MAIN)
    with open(os.path.join(root, "src", "other.rs"), "w") as handle:
        handle.write(OTHER)

    report = Report()

    # ---- The server on its own, with no editor behind it -------------------------------------
    #
    # First, because it is the state every agent starts in until somebody wires it up, and a
    # server that dies here would take the agent's whole connection with it.
    lonely = Mcp(binary, None)
    handshake = lonely.send("initialize", {"protocolVersion": "2025-06-18",
                                           "clientInfo": {"name": "drive_mcp", "version": "0"}})
    report.check("a bare `clee --mcp` completes the handshake",
                 bool(handshake) and handshake.get("result", {}).get("serverInfo", {}).get("name") == "clee",
                 note=repr(handshake))
    outside, failed = lonely.tool("open_files")
    report.check("outside a session the tools say so instead of guessing",
                 failed and "not inside a CleeCode session" in str(outside),
                 note=repr(outside))
    still_there = lonely.send("ping")
    report.check("and the server is still answering afterwards",
                 bool(still_there) and "result" in still_there, note=repr(still_there))
    report.check("closing stdin ends it cleanly", lonely.close() == 0)

    session = Session(binary, root, cols=COLS, rows=ROWS)
    mcp = None
    try:
        started = session.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20)
        report.check("the app draws its first frame", started, session)
        if not started:
            return 1
        session.send(" ")                                     # past the splash
        session.wait(lambda s: "Files" in s.text(), 10)

        session.send("\x0f")                                  # Ctrl+O, quick-open
        session.wait(lambda s: "main.rs" in s.text(), 8)
        session.send("main.rs")
        session.wait(lambda s: True, 0.5)
        session.send("\r")
        report.check("the fixture file opens",
                     session.wait(lambda s: "anchor_main" in s.text(), 8), session)

        # Where an agent started in a pane would find us. Named from the editor's pid, which is
        # what the pty child became, so this driver names it exactly as `CLEE_SESSION` does.
        session_dir = os.path.join(root, ".config", "cleecode", "sessions", str(session.pid))
        state_path = os.path.join(session_dir, "state.json")
        report.check("the editor made a session directory and published into it",
                     wait_for(lambda: os.path.exists(state_path), 15, session), session,
                     note=session_dir)

        state = read_json(state_path)
        report.check("the published state names the open file and the project root",
                     bool(state)
                     and any(f.endswith("src/main.rs") for f in state.get("open_files", []))
                     and str(state.get("root", "")).endswith(os.path.basename(root)),
                     session, note=repr(state and {k: state[k] for k in ("root", "open_files")}))

        # ---- Now the agent's side, as an agent would have it ---------------------------------
        mcp = Mcp(binary, session_dir)
        reply = mcp.send("initialize", {"protocolVersion": "2025-06-18",
                                        "clientInfo": {"name": "drive_mcp", "version": "0"}})
        report.check("initialize, against a running editor",
                     bool(reply) and reply.get("result", {}).get("protocolVersion") == "2025-06-18",
                     note=repr(reply))
        mcp.send("notifications/initialized", {}, notify=True)

        listed = mcp.send("tools/list")
        names = [t.get("name") for t in (listed or {}).get("result", {}).get("tools", [])]
        report.check("tools/list offers the four tools",
                     names == ["open_files", "selection", "diagnostics", "open_file"],
                     note=repr(names))

        files, failed = mcp.tool("open_files")
        report.check("open_files reports the file the editor really has open",
                     not failed and isinstance(files, dict)
                     and any(str(f).endswith("src/main.rs") for f in files.get("files", []))
                     and str(files.get("active", "")).endswith("src/main.rs"),
                     session, note=repr(files))

        where, failed = mcp.tool("selection")
        report.check("selection reports the active file and a 1-based cursor",
                     not failed and isinstance(where, dict)
                     and str(where.get("path", "")).endswith("src/main.rs")
                     and where.get("line", 0) >= 1 and where.get("column", 0) >= 1,
                     session, note=repr(where))

        diags, failed = mcp.tool("diagnostics")
        report.check("diagnostics answers with a list, even an empty one",
                     not failed and isinstance(diags, dict) and isinstance(diags.get("diagnostics"), list),
                     session, note=repr(diags))

        # ---- The one action ------------------------------------------------------------------
        asked, failed = mcp.tool("open_file", {"path": "src/other.rs", "line": OPEN_AT})
        report.check("open_file is accepted and answered at once",
                     not failed and isinstance(asked, dict) and asked.get("status") == "requested",
                     session, note=repr(asked))

        appeared = session.wait(lambda s: "zzz_marker" in s.text(), 10)
        report.check("the editor really opened the file the agent asked for", appeared, session)

        # Line 150 of a 200-line file cannot be on a screen that opened at line 1.
        marker = "zzz_marker_%d " % OPEN_AT
        report.check("and at the line the agent named, not at the top",
                     session.wait(lambda s: marker in s.text(), 6), session, note=marker)

        # Beside, not instead: the file that was already there is still on screen.
        report.check("it opened beside the work rather than over it",
                     "anchor_main" in session.text() and "zzz_marker" in session.text(),
                     session)

        # And the keyboard never moved. Typed rather than inferred from a highlight: where the
        # next character lands is the only definition of "who has the focus" that matters.
        session.send("Q")
        typed = session.wait(lambda s: "Qfn main" in s.text() or "Qfn helper" in s.text(), 6)
        report.check("and it did not take the keyboard",
                     typed and "Qfn main" in session.text(), session,
                     note="the character must land in the file the user was in")

        # The request file is consumed rather than replayed for the rest of the session.
        requests = os.path.join(session_dir, "requests")
        left = [n for n in os.listdir(requests) if n.startswith("req-")] if os.path.isdir(requests) else []
        report.check("the request was consumed", left == [], session, note=repr(left))

        report.check("the server exits when its stdin closes", mcp.close() == 0)
        mcp = None

        Report.show("final screen", session)
    finally:
        if mcp is not None:
            mcp.close()
        # SIGKILL, which is what this harness has always used. Convenient here: a process killed
        # outright never runs its Drop, so what is left behind is exactly the orphan the sweep
        # below exists for. (The tidy exit is covered by the unit test that drops a Session.)
        session.close()

    orphan = os.path.join(root, ".config", "cleecode", "sessions", str(session.pid))
    report.check("a killed editor does leave its session directory behind",
                 os.path.exists(orphan), note=orphan)

    # The next CleeCode to start in the same config directory sweeps it, because the pid it is
    # named after is not running any more.
    second = Session(binary, root, cols=COLS, rows=ROWS)
    try:
        report.check("the next editor sweeps away the dead session's directory",
                     wait_for(lambda: not os.path.exists(orphan), 25, second), second,
                     note=orphan)
        fresh = os.path.join(root, ".config", "cleecode", "sessions", str(second.pid))
        report.check("and has a session directory of its own", os.path.isdir(fresh), second,
                     note=fresh)
    finally:
        second.close()

    shutil.rmtree(root, ignore_errors=True)
    return report.finish()


if __name__ == "__main__":
    try:
        sys.exit(main())
    except BrokenPipeError:
        os._exit(0)
