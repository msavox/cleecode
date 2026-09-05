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

The window is wide — wider than the 120 columns CleeCode calls room for two panes — and that is
now a check rather than a convenience. **An agent's `open_file` never opens the split.** A tool
call is another program rearranging the frames of somebody who is typing in them, and "an agent
read a file" must not be the answer to "why did my window just split", so with room to spare and
the split off the file has to arrive as an ordinary tab and the layout has to stay exactly as it
was. Where that tab goes is decided by where the keyboard was at that moment: in the editor it
goes *behind* the file being typed in, and the status line is the only thing that moves; anywhere
else — a terminal, which is where somebody talking to an agent usually is — it comes to the
front, because there is nothing being typed for it to cover. Both halves are driven here, from
the two places the keyboard can be.

The split still matters to the range checks — a span highlighted in a pane nobody is focused on
is the thing worth seeing — so this driver opens one itself, with the key a user would press,
before it asks for a range. That is the honest arrangement now that the editor will not open one
unasked: what an agent gets is the split the user has.

The debugger and the Octave figures are the other side of that rule and still open a split of
their own; drive_dap.py and drive_figures.py are where that is checked.
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

# Wider than SPLIT_FOR_FIGURES_COLS (120), the width CleeCode calls room for two panes — so a
# window that stayed one pane stayed one pane on purpose. See the module docstring.
COLS, ROWS = 140, 34

# `mcp::MAX_SAY`, mirrored here rather than imported: this driver speaks to the binary as a
# process, not as a crate, and the number is small and stable enough that copying it beats
# teaching Python to parse Rust.
MAX_SAY = 120

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


def file_tool_call(mcp, name, arguments):
    """Files a `tools/call` without waiting for the reply.

    `Mcp.send` gives up after ten seconds, which is right for every tool but `edit_buffer`: that
    one can be sitting on the other side of a consent question nobody has answered yet, and
    answering it — draining the editor's pty, then pressing a key — is this driver's job. Filing
    the call and reading the reply are split in two so that can happen in between."""
    mcp.next_id += 1
    message = {"jsonrpc": "2.0", "id": mcp.next_id, "method": "tools/call",
               "params": {"name": name, "arguments": arguments}}
    mcp.proc.stdin.write(json.dumps(message) + "\n")
    mcp.proc.stdin.flush()
    return mcp.next_id


def await_tool_reply(mcp, call_id, session, timeout=15.0):
    """The other half of `file_tool_call`: waits for that call's reply, draining the editor's pty
    throughout — the same discipline `wait_for` uses, and for the same reason. `edit_buffer`'s
    reply is written by the editor's own poll loop from a request the server is holding a
    synchronous call open for, and that loop only runs if nothing is blocked writing frames into
    a pty nobody is reading."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        session.drain()
        ready, _, _ = select.select([mcp.proc.stdout], [], [], 0.05)
        if ready:
            line = mcp.proc.stdout.readline()
            if line.strip():
                reply = json.loads(line)
                if reply.get("id") == call_id:
                    return reply
    return None


def unwrap_tool_reply(reply):
    """A `tools/call` reply already in hand, unwrapped the way `Mcp.tool` unwraps one it read for
    itself — the non-blocking half of that method, for a call filed with `file_tool_call`."""
    if reply is None or "result" not in reply:
        return None, True
    result = reply["result"]
    text = "".join(b.get("text", "") for b in result.get("content", []) if b.get("type") == "text")
    try:
        parsed = json.loads(text)
    except json.JSONDecodeError:
        parsed = text
    return parsed, bool(result.get("isError"))


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


def editor_panes(session):
    """How many editor frames are on screen: one, or two while the split is open.

    Counted from the top-left corners along the editor frames' own border row, which is the row
    directly under the tab strip — the strip is the row the ▶ Run button rides, and the file
    tree's corner is up there with it rather than down here. Read off the picture rather than
    inferred from what is written in a pane, because the question is about the *layout*: which
    file happens to be in front changes under half the checks below, and the number of frames
    must not."""
    row = session.row_of("▶ Run")
    return session.full_line(row + 1).count("┌") if row is not None else None


def editor_border_ink(session):
    """The colour the editor frame's own top-left corner is drawn in.

    A frame with the keyboard in it wears its border in the accent and one without it does not —
    `focused_border_style` in src/ui.rs — so this is where the focus is, read off the picture. It
    is what a focus move is *waited on* here: the repo's rule is to wait for the thing itself and
    never for a moment of the clock, and a keystroke that moves the keyboard has this to show for
    itself the instant it has landed."""
    row = session.row_of("▶ Run")
    if row is None:
        return None
    at = session.full_line(row + 1).find("┌")
    return session.cells(row + 1)[at].fg if at >= 0 else None


def tab_strip(session):
    """The editor's tab strip, which is the row the ▶ Run button rides.

    A tab opened behind the file somebody is typing in shows nowhere else: its contents are not on
    screen, by design, and the strip is the whole of what a user sees happen. So this is what
    "the file was opened" is asked of when the answer must not be "and it took the front"."""
    row = session.row_of("▶ Run")
    return session.full_line(row) if row is not None else ""


def read_json(path):
    try:
        with open(path) as handle:
            return json.load(handle)
    except (OSError, json.JSONDecodeError):
        return None


def main():
    binary = binary_from_argv(sys.argv)
    root = tempfile.mkdtemp(prefix="clee_mcp_")
    # Sessions live under the temp dir (see `mcp::sessions_root`), and this driver gives the
    # editors a temp dir of their own so the paths below are deterministic and two drivers
    # cannot see each other's sessions. Both editors share it: the sweep check needs the second
    # editor to find the first one's orphan.
    tmp = tempfile.mkdtemp(prefix="clee_mcp_tmp_")
    sessions = os.path.join(tmp, "cleecode-sessions")
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

    session = Session(binary, root, env={"TMPDIR": tmp}, cols=COLS, rows=ROWS)
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
        session_dir = os.path.join(sessions, str(session.pid))
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
        report.check("tools/list offers the seven tools",
                     names == ["open_files", "selection", "diagnostics", "open_file",
                                "preview", "say", "edit_buffer"],
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

        # ---- say: the sanitizer's OUTPUT is what lands on screen ------------------------------
        #
        # The server cuts a `say` to one line and `MAX_SAY` characters before it ever touches
        # disk, and the editor cuts it again on the way out (`mcp::say_line`, called at both
        # ends). Sending something that needs both cuts and then reading the *status bar* rather
        # than the tool's own echo is the only way to know the two ends agree about where
        # "truncated" actually lands.
        say_prefix = "say_marker_"
        say_pad = "A" * (MAX_SAY - len(say_prefix))          # prefix + pad is exactly MAX_SAY
        say_tail = "TAIL_MUST_NEVER_BE_SEEN"
        expected_say = say_prefix + say_pad
        said, failed = mcp.tool("say", {"text": say_prefix + say_pad + say_tail +
                                         "\nsecond_line_must_never_be_seen"})
        report.check("say echoes exactly the sanitizer's MAX_SAY-character cut",
                     not failed and isinstance(said, dict) and said.get("text") == expected_say,
                     note=repr(said))
        report.check("and that cut, agent-marked, is what reaches the status bar",
                     session.wait(lambda s: "agent: " + expected_say in s.text(), 8), session)
        report.check("nothing past the cut, and no second line, ever reaches the screen",
                     say_tail not in session.text()
                     and "second_line_must_never_be_seen" not in session.text(),
                     session)

        # ---- The one action, and where it is allowed to put the file --------------------------
        #
        # The keyboard is in the editor: the fixture was opened with Ctrl+O a moment ago and
        # nothing has moved it since. That is precisely the case the rule is written for — a tab
        # arriving over the line somebody is typing in is the layout changing under them — so
        # what has to happen here is a tab in the strip, the user's own file still in front of
        # it, and a sentence on the status line as the only thing that moved.
        one_pane = editor_panes(session)
        asked, failed = mcp.tool("open_file", {"path": "src/other.rs", "line": OPEN_AT})
        report.check("open_file is accepted and answered at once",
                     not failed and isinstance(asked, dict) and asked.get("status") == "requested",
                     session, note=repr(asked))

        appeared = session.wait(lambda s: "other.rs" in tab_strip(s), 10)
        report.check("the editor really opened the file the agent asked for", appeared, session,
                     note="a tab in the strip, which is where an ordinary tab goes")

        # The whole of the change: an agent may not rearrange the frames. Twenty columns of room
        # to spare and the window still has one editor pane in it.
        report.check("and it did not open the split to do it",
                     editor_panes(session) == one_pane, session,
                     note="%s editor frame before the request, %s after — in a window %d columns "
                          "wide, which is wider than CleeCode's own two-pane threshold"
                          % (one_pane, editor_panes(session), COLS))
        report.check("the file the user was typing in is the one still in front",
                     "anchor_main" in session.text() and "zzz_marker" not in session.text(),
                     session,
                     note="opened behind the work: the keyboard was in this very pane")
        report.check("and the status line is what says the file was opened",
                     "other.rs" in session.full_line(session.rows - 1), session,
                     note=session.full_line(session.rows - 1).strip()[:110])

        # ---- the other half of the rule, from the other side of the keyboard ------------------
        #
        # Focus anywhere but the editor is the ordinary case: somebody asks an agent something in
        # a terminal and looks up to see the answer. Nothing is being typed for the tab to cover,
        # so the same request puts the file in front — which is also the only way the line it was
        # asked for can be looked at.
        in_editor = editor_border_ink(session)
        # Ctrl+Alt+↓, into the terminal panel — waited on the editor's border going out, which is
        # the frame saying it no longer has the keyboard.
        left_editor = session.press(
            "\x1b[1;7B", lambda s: editor_border_ink(s) not in (None, in_editor), 8)
        report.check("the keyboard can be put where somebody talking to an agent has it",
                     left_editor, session,
                     note="the editor's border stops being the focused one: %s, now %s"
                          % (in_editor, editor_border_ink(session)))
        again, failed = mcp.tool("open_file", {"path": "src/other.rs", "line": OPEN_AT})
        report.check("the same open_file is accepted again",
                     not failed and isinstance(again, dict) and again.get("status") == "requested",
                     session, note=repr(again))
        # Line 150 of a 200-line file cannot be on a screen that opened at line 1.
        marker = "zzz_marker_%d " % OPEN_AT
        fronted = session.wait(lambda s: marker in s.text(), 10)
        report.check("with the keyboard outside the editor the file comes to the front instead",
                     fronted, session, note="nothing was being typed for it to arrive over")
        report.check("and at the line the agent named, not at the top", marker in session.text(),
                     session, note=marker)
        report.check("and it still did not open the split",
                     editor_panes(session) == one_pane, session,
                     note="where the file lands is the keyboard's business; the frames are the "
                          "user's either way")

        # Back to the editor and back to the user's own file, which is the state the rest of this
        # file is written for — the checks below type a character and ask where it landed.
        session.press("\x1b[1;7A", lambda s: editor_border_ink(s) == in_editor, 8)  # Ctrl+Alt+↑
        session.press("\x1b[1;6D", lambda s: "anchor_main" in s.text(), 8)   # Ctrl+Shift+←
        report.check("the user can walk back to their own tab", "anchor_main" in session.text(),
                     session, note="two tabs in the strip, and the other one is theirs")

        # ---- a split the *user* opened --------------------------------------------------------
        #
        # Ctrl+L, the key a person presses. Everything from here down is about a file arriving in
        # the pane beside the work, which is still what happens — an agent gets the split the user
        # has, and this is the user giving it one.
        session.send("\x0c")
        split = session.wait(lambda s: (editor_panes(s) or one_pane) > one_pane, 8)
        report.check("Ctrl+L gives the window two panes", split, session,
                     note="%s editor frames, up from %s" % (editor_panes(session), one_pane))

        # ---- open_file with a range: end_line marks it, not just scrolls to it ----------------
        #
        # A line well above the one just visited, so the jump scrolls *up* (see `adjust_scroll`
        # in src/editor.rs) and lands the first line of the range at the very top of the
        # viewport, with the rest of the range and an unselected neighbour below it all on screen
        # together — a downward jump like the one above would instead put the named line at the
        # *bottom* of the viewport, with nothing of the range left to look at.
        #
        # `highlight_selection` (src/ui.rs) paints a span in the selection colour regardless of
        # which pane has focus — see `show_beside_without_focus`'s own doc comment — so the range
        # reading as *selected* is a fact about the colour under those characters, not merely
        # about which lines are in view.
        RANGE_START, RANGE_END = 30, 32
        ranged, failed = mcp.tool("open_file",
                                   {"path": "src/other.rs", "line": RANGE_START, "end_line": RANGE_END})
        report.check("open_file with end_line is accepted and echoes both ends",
                     not failed and ranged.get("line") == RANGE_START
                     and ranged.get("end_line") == RANGE_END,
                     note=repr(ranged))

        first_marker = "zzz_marker_%d " % RANGE_START
        last_marker = "zzz_marker_%d " % RANGE_END
        below_marker = "zzz_marker_%d " % (RANGE_END + 1)
        ranged_on_screen = session.wait(
            lambda s: first_marker in s.text() and last_marker in s.text() and below_marker in s.text(),
            8)
        report.check("the whole range, and its unselected neighbour right after it, are on screen",
                     ranged_on_screen, session)

        # With a split there *is* somewhere else to put it, so it goes there — the half of the old
        # behaviour that never changed. Both files on screen at once is the whole of "beside".
        report.check("a split the user opened is one the agent's file lands in the other half of",
                     "anchor_main" in session.text() and "zzz_marker" in session.text(), session,
                     note="beside the work, not over it: the frames the user made are the frames "
                          "the agent gets")

        if ranged_on_screen:
            first_row, last_row, below_row = (session.row_of(first_marker), session.row_of(last_marker),
                                               session.row_of(below_marker))
            first_col = session.lines()[first_row].index(first_marker)
            last_col = session.lines()[last_row].index(last_marker)
            below_col = session.lines()[below_row].index(below_marker)
            selected_bg = session.cells(first_row)[first_col].bg
            below_bg = session.cells(below_row)[below_col].bg
            report.check("the range reads as selected: its background differs from the "
                         "unselected line right after it",
                         selected_bg != below_bg, session,
                         note=f"selected={selected_bg} plain={below_bg}")
            report.check("...and the same highlight reaches the far end of the range",
                         session.cells(last_row)[last_col].bg == selected_bg, session)
            report.check("the range starts at the top of the pane: the line the agent named, "
                         "before the rest of it",
                         first_row < last_row < below_row, session)
        else:
            report.check("at minimum, the line the agent named is where the range starts",
                         first_marker in session.text(), session)

        # And the keyboard never moved. Typed rather than inferred from a highlight: where the
        # next character lands is the only definition of "who has the focus" that matters.
        session.send("Q")
        typed = session.wait(lambda s: "Qfn main" in s.text() or "Qfn helper" in s.text(), 6)
        report.check("and it did not take the keyboard",
                     typed and "Qfn main" in session.text(), session,
                     note="the character must land in the file the user was in")

        # ---- dirty in open_files: the unsaved buffer that "Q" just made ------------------------
        dirty_before, failed = mcp.tool("open_files")
        report.check("open_files lists the just-typed-into buffer as dirty",
                     not failed and any(f.endswith("src/main.rs") for f in dirty_before.get("dirty", [])),
                     session, note=repr(dirty_before))

        saved = session.press("\x13", lambda s: "main.rs*" not in s.text(), 8)
        report.check("Ctrl+S saves it", saved, session)

        dirty_after, failed = mcp.tool("open_files")
        report.check("...and a save takes it back out of the dirty list",
                     not failed
                     and not any(f.endswith("src/main.rs") for f in dirty_after.get("dirty", [])),
                     session, note=repr(dirty_after))

        # ---- preview: the rendered surface, not the source, and no stolen focus ---------------
        note_path = os.path.join(root, "src", "note.md")
        with open(note_path, "w") as handle:
            handle.write("# Preview marker\n\na paragraph with **preview_bold_marker** inside it.\n")

        previewed, failed = mcp.tool("preview", {"path": "src/note.md"})
        report.check("preview is accepted and answered at once",
                     not failed and isinstance(previewed, dict) and previewed.get("status") == "requested",
                     session, note=repr(previewed))

        rendered = session.wait(lambda s: "preview_bold_marker" in s.text(), 8)
        # Beside, in the split the *driver* opened above — `preview` has no more right to the
        # frames than `open_file` does, and in one pane this tab would be behind the user's work
        # for exactly the same reason. What is checked here is the other arm: given a second pane,
        # the rendered document goes into it rather than over the file being typed in.
        report.check("the preview tab opens in the pane the user is not typing in", rendered,
                     session)
        # Markdown's `**` never reaches the rendered lines — `Event::Start(Tag::Strong)` only
        # flips a style bit (src/preview.rs) — so its absence is what tells the preview surface
        # apart from the raw source, which would still be showing them.
        report.check("it shows the rendered document, not the raw markdown source",
                     "**preview_bold_marker**" not in session.text(), session)

        session.send("P")
        kept_focus = session.wait(lambda s: "QPfn main" in s.text(), 6)
        report.check("previewing a file did not take the keyboard either",
                     kept_focus, session,
                     note="the character must still land in the file the user was typing in")

        # ---- edit_buffer, declined: the question is asked, and "no" really means no -----------
        #
        # `agent_edits` is left at its default, "ask". main.rs is clean again after the Ctrl+S
        # above — `edit_buffer` works on a clean buffer too (only "is it open" is asked in
        # `apply_agent_edit`), which is exactly what lets the rest of this section reuse it.
        OLD_ANCHOR, NEW_ANCHOR = "let anchor_main = 1;", "let anchor_main = 2;"
        decline_id = file_tool_call(mcp, "edit_buffer",
                                     {"path": "src/main.rs", "old_string": OLD_ANCHOR,
                                      "new_string": NEW_ANCHOR})
        asked = session.wait(lambda s: "wants to change" in s.text(), 8)
        report.check("edit_buffer asks before touching the buffer", asked, session)
        session.send("n")
        declined, failed = unwrap_tool_reply(await_tool_reply(mcp, decline_id, session, 10))
        report.check("declining answers isError with the declined sentence",
                     failed and isinstance(declined, str) and "declined" in declined,
                     note=repr(declined))
        report.check("and the buffer on screen never changed",
                     "anchor_main = 1" in session.text() and "anchor_main = 2" not in session.text(),
                     session)

        # ---- edit_buffer, allowed once: applied, dirty, and NOT saved --------------------------
        allow_id = file_tool_call(mcp, "edit_buffer",
                                   {"path": "src/main.rs", "old_string": OLD_ANCHOR,
                                    "new_string": NEW_ANCHOR})
        asked_again = session.wait(lambda s: "wants to change" in s.text(), 8)
        report.check("asked again for the same edit", asked_again, session)
        session.send("y")
        applied, failed = unwrap_tool_reply(await_tool_reply(mcp, allow_id, session, 10))
        report.check("saying yes once applies the edit",
                     not failed and isinstance(applied, dict) and applied.get("status") == "applied",
                     note=repr(applied))
        report.check("the new text lands on screen",
                     session.wait(lambda s: "anchor_main = 2" in s.text(), 6), session)
        report.check("the buffer shows as modified", "main.rs*" in session.text(), session)
        dirty_now, failed = mcp.tool("open_files")
        report.check("...and dirty, via open_files",
                     not failed and any(f.endswith("src/main.rs") for f in dirty_now.get("dirty", [])),
                     note=repr(dirty_now))
        with open(os.path.join(root, "src", "main.rs")) as handle:
            on_disk = handle.read()
        report.check("but not saved: the file on disk still holds the old text",
                     "anchor_main = 1" in on_disk and "anchor_main = 2" not in on_disk,
                     note=repr(on_disk))

        # ---- edit_buffer, session-wide: 'A' answers this edit and every later one -------------
        OLD_PRINTLN = 'println!("{anchor_main}");'
        NEW_PRINTLN = 'println!("{anchor_main} once");'
        session_id = file_tool_call(mcp, "edit_buffer",
                                     {"path": "src/main.rs", "old_string": OLD_PRINTLN,
                                      "new_string": NEW_PRINTLN})
        asked_third = session.wait(lambda s: "wants to change" in s.text(), 8)
        report.check("a third edit asks too, before 'A' has been pressed", asked_third, session)
        session.send("a")
        via_a, failed = unwrap_tool_reply(await_tool_reply(mcp, session_id, session, 10))
        report.check("'A' applies this edit",
                     not failed and isinstance(via_a, dict) and via_a.get("status") == "applied",
                     note=repr(via_a))

        after_a, failed = mcp.tool("edit_buffer",
                                    {"path": "src/main.rs", "old_string": "fn main() {",
                                     "new_string": "fn main() { // session-wide"})
        session.drain()
        report.check("and the next edit this session applies with no prompt at all",
                     not failed and isinstance(after_a, dict) and after_a.get("status") == "applied"
                     and "wants to change" not in session.text(),
                     session, note=repr(after_a))

        # ---- old_string discipline: ambiguous, so nothing moves --------------------------------
        ambiguous, failed = mcp.tool("edit_buffer",
                                      {"path": "src/main.rs", "old_string": "anchor_main",
                                       "new_string": "renamed"})
        report.check("an old_string that matches twice is refused, and says how many times",
                     failed and isinstance(ambiguous, str)
                     and "2" in ambiguous and "matches" in ambiguous,
                     note=repr(ambiguous))
        report.check("nothing changed: the buffer still reads exactly as the earlier edits left it",
                     "anchor_main = 2" in session.text() and "renamed" not in session.text(), session)

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

    orphan = os.path.join(sessions, str(session.pid))
    report.check("a killed editor does leave its session directory behind",
                 os.path.exists(orphan), note=orphan)

    # The next CleeCode to start with the same temp dir sweeps it, because the pid it is
    # named after is not running any more.
    second = Session(binary, root, env={"TMPDIR": tmp}, cols=COLS, rows=ROWS)
    try:
        report.check("the next editor sweeps away the dead session's directory",
                     wait_for(lambda: not os.path.exists(orphan), 25, second), second,
                     note=orphan)
        fresh = os.path.join(sessions, str(second.pid))
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
