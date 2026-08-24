"""Workspace snapshots and figure captures for CleeCode, from a plain Python REPL.

Python has no equivalent of Octave's add_input_event_hook, and it takes two mechanisms to
build one. Between them they give what Octave cannot: an exact callback per statement,
with no polling and no idle cost for the snapshots.

(The *inspector* is the one exception to "no polling", and it earned it: see
_slice_watcher, whose reason to exist is that an idle prompt draws nothing.)

  · An audit hook on `exec` says *which* statement. It carries the code object, and the
    REPL compiles each thing you type under a name of its own — `<python-input-7>`, or
    `<stdin>` in the basic REPL. But it is raised before the statement runs, so it is a
    counter, not a moment.

  · `str(sys.ps1)` says *when the statement finished*, because the prompt is drawn after
    it. But the REPL draws the prompt far more often than that: measured on 2026-08-20,
    four statements restringify it 65 times under PyREPL, and typing twelve characters
    without pressing Enter accounts for 23 more.

    And it says nothing at all while the user is *not* typing. A prompt nobody is
    touching is never restringified, so anything that only runs from `__str__` waits
    for the next keystroke — which for the inspector meant "Asking the session…"
    forever, found on 2026-08-22 when a test that had been passing for the wrong
    reason was made to look at the inspector's own frame.

So the hook marks and the prompt collects. Measured together in one session: 65
restringifications become 5 snapshots for 5 statements, each seeing the namespace as the
statement left it, and typing produces none at all.

Both obvious alternatives are dead under PyREPL — the REPL a user actually gets — and
were measured rather than assumed. `readline.get_current_history_length()`, the direct
analogue of the `numel(history())` trick the Octave side leans on, returns 0, because
PyREPL keeps its own history. And an audit hook that does not look at the filename sees
52 execs for 4 statements, since the REPL execs plenty of its own code. Installing the
hook costs nothing measurable: numeric work, pure loops, and two thousand open+write are
identical to three decimals with it and without.

Installed from PYTHONSTARTUP, which is CleeCode's env-var-gated hook, the same role
~/.octaverc plays for Octave.

One path still missing: IPython does not use sys.ps1 at all, so it needs
ip.events.register("post_run_cell", ...) — more official than either half of this.
Detect which REPL is running rather than guessing.
"""

import json
import os
import sys
import time

_STAT_LIMIT = 1_000_000      # elements above which min/max/mean are not paid for
_STATE = None                # the installed session's state, for `frame()` to find
_PREVIEW = 60                # characters of repr kept


class _Prompt:
    """Stands in for sys.ps1, and collects a snapshot when one is owed.

    Whatever happens in here, it must still return a prompt: an exception would leave the
    user staring at a broken REPL, which is a high price for a panel that failed to draw."""

    def __init__(self, text, state):
        self._text = text
        self._state = state

    def __str__(self):
        try:
            # Back at the REPL, so the statement is over: stop tracing until the next one
            # starts, and let go of any frame the debugger stopped in. Reaching this prompt is
            # the one unambiguous sign the debugger is done — pdb returns from `interaction` to
            # step as well as to finish, and those look identical from inside it.
            if sys.gettrace() is not None:
                sys.settrace(None)
            if self._state.get("frame") is not None:
                self._state["frame"] = None
                self._state["pending"] = True
            # Asked at every prompt, whether or not anything ran: a question CleeCode left
            # while the session was busy should not wait for the next command to be answered.
            _answer_slice(self._state)
            _breakpoints(self._state)
            if self._state["pending"]:
                self._state["pending"] = False
                _snapshot(self._state)
        except Exception:
            pass
        return self._text


def sync_plots():
    """Bring this interpreter's idea of where plots go up to date, before matplotlib is imported.

    `CLEECODE_PLOTS` and `MPLBACKEND` are copies taken when the *shell* started, and a shell's
    environment cannot be changed from outside afterwards. So a prompt opened before the
    preference was flipped went on doing what it had always done, and the only cure anybody found
    was to restart the editor — which is how it was reported. CleeCode also writes the answer to
    the file named by `CLEECODE_PLOTS_FILE`, and that is read here, at interpreter start.

    matplotlib picks its backend when it is first imported and never looks at the variable again,
    so this has to run before anything the user wrote. sitecustomize is exactly that moment.

    Only ever swaps CleeCode's own backend in and out: a user who set MPLBACKEND themselves has
    said what they want, and this is not the place to argue.
    """
    ours = "module://cleecode_mpl"
    path = os.environ.get("CLEECODE_PLOTS_FILE")
    said = None
    if path:
        try:
            with open(path, encoding="utf-8") as handle:
                word = handle.read().strip()
            if word in ("tabs", "windows"):
                said = word
        except OSError:
            said = None                      # unreadable is the same as absent
    if said is None:
        return                               # the environment already answered
    os.environ["CLEECODE_PLOTS"] = said
    if said == "windows":
        if os.environ.get("MPLBACKEND") == ours:
            del os.environ["MPLBACKEND"]     # back to whatever matplotlib would pick on its own
    elif not os.environ.get("MPLBACKEND"):
        os.environ["MPLBACKEND"] = ours


def _slice_watcher(state):
    """A daemon thread that answers the inspector while the prompt sits idle.

    Everything else here runs when a statement ends or a prompt is drawn, and that is the
    right shape for snapshots: they describe what a statement did. The inspector is a
    question *CleeCode* asks, at a moment of its own choosing — typically while the user
    is looking at a panel and touching nothing — and a prompt nobody types at is never
    restringified, so the `sys.ps1` path never runs and the question sat unanswered
    forever. Octave does not have this problem because its input hook fires while idle;
    this thread is that hook, built from what Python has.

    One stat of the request file five times a second, nothing else: `_answer_slice`
    already returns without work unless the file's mtime moved. Reading the namespace
    from another thread is safe here because the answer is read-only and the GIL keeps
    each dict read whole — and the prompt path keeps answering too, so a race at worst
    writes the same answer twice, through the same atomic rename."""
    import threading

    def loop():
        while True:
            time.sleep(0.2)
            try:
                _answer_slice(state)
            except Exception:
                pass

    threading.Thread(target=loop, daemon=True, name="cleecode-slice").start()


def _statement_watcher(state):
    """An audit hook that notes when one of the user's own statements is about to run.

    The filename is the whole test. Without it this fires on the REPL's own machinery too
    — 52 times for 4 statements — and with it, exactly once each."""

    def hook(event, args):
        if event != "exec":
            return
        try:
            name = args[0].co_filename
        except Exception:
            return
        if name.startswith("<python-input") or name == "<stdin>":
            state["pending"] = True
            _arm(state)

    return hook


def _arm(state):
    """Start tracing, for the length of one statement.

    This is why the audit hook is worth having twice over. Tracing costs a call per Python line,
    and the prompt is not idle — PyREPL redraws the line on every keystroke, so a trace function
    installed at the prompt would be paying that cost for every character typed and catching
    nothing. The hook fires once, immediately before the user's own statement runs, which is the
    only window where a breakpoint can be reached. The prompt turns it off again afterwards.

    `set_continue` means "run until a breakpoint", which is the only reason we are tracing —
    without it bdb stops at the very first line it sees, which is PyREPL's own. And it only
    means that if `botframe` is set first: bdb reads an unset one as "stop everywhere", so
    leaving it out produces exactly the behaviour asking for `continue` was meant to avoid.

    What botframe is set *to* barely matters, because bdb only ever compares it by identity and
    the user's own frames are all made after this one. Setting it also buys the speed: with it,
    a call into a file that holds no breakpoint is not traced line by line at all, so the cost
    falls on the file being debugged rather than on everything the statement touches."""
    debugger = state.get("dbg")
    if debugger is None:
        return
    try:
        debugger.reset()                # breaks survive this; only the stepping state is cleared
        debugger.botframe = sys._getframe()
        debugger.set_continue()
        sys.settrace(debugger.trace_dispatch)
    except Exception as problem:
        _log("arm failed: %r" % (problem,))
        sys.settrace(None)


def _user_vars():
    """__main__ minus the noise. PYTHONSTARTUP executes in the user's own namespace,
    so this module's import would otherwise show up as a variable in their panel;
    everything it leaves behind is underscore-prefixed and filtered here."""
    out = {}
    for name, val in list(sys.modules["__main__"].__dict__.items()):
        if name.startswith("_") or name in ("In", "Out"):
            continue
        if type(val).__name__ == "module":
            continue
        out[name] = val
    return out


def _describe(name, val):
    v = {"name": name, "class": type(val).__name__, "size": [], "bytes": None,
         "attr": "", "min": None, "max": None, "mean": None, "nans": 0,
         "preview": ""}

    np = sys.modules.get("numpy")
    if np is not None and isinstance(val, np.ndarray):
        v["class"] = f"ndarray[{val.dtype}]"
        v["size"] = list(val.shape)
        v["bytes"] = int(val.nbytes)
        if val.size == 0:
            v["preview"] = "empty"
        elif np.iscomplexobj(val):
            a = np.abs(val)
            v["min"], v["max"], v["mean"] = float(a.min()), float(a.max()), float(a.mean())
            v["attr"], v["preview"] = "c", "|z|"
        elif val.size > _STAT_LIMIT:
            v["preview"] = "(too large to summarise)"
        elif np.issubdtype(val.dtype, np.number) or val.dtype == bool:
            f = val.astype("float64", copy=False)
            v["nans"] = int(np.isnan(f).sum())
            good = f[~np.isnan(f)]
            if good.size:
                v["min"], v["max"], v["mean"] = float(good.min()), float(good.max()), float(good.mean())
            v["preview"] = _clip(np.array2string(val.ravel()[:8], threshold=8))
        else:
            v["preview"] = _clip(repr(val))
        return v

    if isinstance(val, bool):
        v["size"] = [1, 1]
        v["preview"] = repr(val)
    elif isinstance(val, (int, float)):
        v["size"] = [1, 1]
        v["min"] = v["max"] = v["mean"] = float(val)
        v["preview"] = repr(val)
    elif isinstance(val, str):
        v["size"] = [1, len(val)]
        v["preview"] = _clip(val)
    elif isinstance(val, (list, tuple, set, dict)):
        v["size"] = [1, len(val)]
        nums = None
        if isinstance(val, (list, tuple)) and val and all(
                isinstance(x, (int, float)) and not isinstance(x, bool) for x in val):
            nums = [float(x) for x in val]
        if nums:
            v["min"], v["max"] = min(nums), max(nums)
            v["mean"] = sum(nums) / len(nums)
        v["preview"] = _clip(repr(val))
    elif callable(val):
        v["preview"] = _clip(getattr(val, "__name__", repr(val)))
    else:
        v["preview"] = _clip(repr(val))
    return v


def _clip(s):
    s = " ".join(str(s).split())
    return s if len(s) <= _PREVIEW else s[:_PREVIEW - 3] + "..."


def _figures(state):
    """Rasterise every open matplotlib figure, but only if matplotlib is already
    imported — importing it here would cost seconds and surprise anyone not plotting."""
    # A session told to keep matplotlib's own windows has nothing to hand over: the figures are
    # on screen already, and rendering each one a second time is the most expensive thing this
    # hook can do — paid, here, for pictures nobody is going to open.
    if os.environ.get("CLEECODE_PLOTS") == "windows":
        return []
    plt = sys.modules.get("matplotlib.pyplot")
    if plt is None:
        return []
    out = []
    drawn = state.setdefault("drawn", {})
    for num in plt.get_fignums():
        fig = plt.figure(num)
        png = os.path.join(state["figdir"], f"fig{num}.png")
        w, h = fig.get_size_inches() * fig.dpi
        # Only redraw what changed. Rendering a figure is the most expensive thing this hook
        # can do, and it runs at every prompt; a session with a plot open and nothing new to
        # show should cost the same as a session with no plot at all. `stale` is matplotlib's
        # own answer to the question.
        #
        # Cleared by hand afterwards, which is the part this got wrong: savefig does *not*
        # clear it. Measured 2026-08-22 — after a savefig, a set_xlim, a replot and a title,
        # `fig.stale` reads True every time — so the guard was always open and every figure
        # was re-rendered at every prompt, which is the exact cost the guard is here to avoid.
        # Setting it back does what the comment always claimed: False while nothing moves,
        # True again the moment anything does.
        #
        # The Octave side had the mirror image of this, found the same day: there the flag is
        # never *set* under qt, so a figure was printed once and never again.
        if fig.stale or not drawn.get(num) or not os.path.exists(png):
            # Written beside the real name and moved onto it, because the editor is watching
            # this file and reads it the moment it changes. A savefig straight onto the name
            # is a picture that exists half-written for as long as it takes to write, and a
            # frame of an animation caught there decodes as "unexpected end of file" — the tab
            # then says it could not read the picture, which is a lie about a file that is
            # perfectly good a millisecond later. A rename within a directory is atomic: the
            # watcher sees the old picture or the new one, never half of either.
            # Still ending in .png: matplotlib reads the format off the suffix, and a
            # ".part" one is a picture it refuses to write at all.
            part = png + ".part.png"
            fig.savefig(part, dpi=fig.dpi)
            os.replace(part, png)
            fig.stale = False
            drawn[num] = True
        entry = {"fig": num, "png": [int(round(w)), int(round(h))], "path": png, "axes": []}
        for ax in fig.axes:
            p = ax.get_position()
            a = {"pos": [p.x0, p.y0, p.width, p.height],
                 "xlim": list(map(float, ax.get_xlim())),
                 "ylim": list(map(float, ax.get_ylim())),
                 "xscale": ax.get_xscale(), "yscale": ax.get_yscale(),
                 "is3d": hasattr(ax, "get_proj")}
            if a["is3d"]:
                a["view"] = [float(ax.elev), float(ax.azim)]
            entry["axes"].append(a)
        out.append(entry)
    return out


def _history():
    """The last few things typed, newest last, with CleeCode's own injections left out.

    Not from `readline`: under PyREPL that reports nothing at all — measured, it returns 0 while
    the reader is holding a hundred entries. The reader is where they actually are. It is a
    private module, so this is guarded and falls back to readline for the basic REPL, where
    readline is the truth."""
    lines = []
    try:
        from _pyrepl.readline import _get_reader
        lines = [str(line) for line in _get_reader().history]
    except Exception:
        try:
            import readline
            n = readline.get_current_history_length()
            lines = [readline.get_history_item(i) or "" for i in range(max(1, n - 30), n + 1)]
        except Exception:
            return []
    out = []
    for line in reversed(lines):
        line = (line or "").strip()
        # Everything CleeCode types at a prompt carries this comment, so a list of recent
        # commands stays a list of what *you* did.
        if not line or "# cleecode" in line:
            continue
        out.append(line)
        if len(out) >= 12:
            break
    return list(reversed(out))


def _frame_vars(state):
    """The variables in scope: the stopped frame's while debugging, `__main__`'s otherwise.

    While stopped, the frame's locals are what the panel is for — the difference between
    watching a program run and looking at what it left behind."""
    frame = state.get("frame")
    if frame is None:
        return _user_vars()
    return {
        name: value
        for name, value in frame.f_locals.items()
        if not name.startswith("_") and type(value).__name__ != "module"
    }


def _snapshot(state):
    state["seq"] += 1
    doc = {"v": 1, "seq": state["seq"], "time": time.time(), "pid": os.getpid(),
           "cwd": os.getcwd(), "lang": "python",
           "vars": [_describe(n, v) for n, v in sorted(_frame_vars(state).items())],
           "history": _history(),
           "debug": _debug_state(state),
           "figures": _figures(state)}
    tmp = f"{state['out']}.{os.getpid()}.tmp"
    with open(tmp, "w") as f:
        json.dump(doc, f)
    os.replace(tmp, state["out"])       # atomic: no reader ever sees half a file


# ---- answering CleeCode's questions ------------------------------------------------------
#
# Two files, read at each prompt: one asking for a rectangle of a variable, one listing the
# breakpoints wanted. Both are questions CleeCode could have typed at the prompt and deliberately
# does not — a line the user did not write has no business in their transcript, and the Octave
# side learned the hard way that a prompt is not reliably listening anyway.


def _answer_slice(state):
    """Write out the rectangle of a variable CleeCode asked about."""
    req, out = os.environ.get("CLEECODE_PY_SLICE_REQ"), os.environ.get("CLEECODE_PY_SLICE")
    if not req or not out or not os.path.exists(req):
        return
    stamp = os.path.getmtime(req)
    if state.get("slice_seen") == stamp:
        return
    state["slice_seen"] = stamp
    try:
        ask = json.load(open(req))
    except Exception:
        return
    doc = {"name": ask.get("name", ""), "error": "", "rows": 0, "cols": 0,
           "r0": 0, "c0": 0, "text": False, "data": []}
    try:
        value = _frame_vars(state)[doc["name"]]
        np = sys.modules.get("numpy")
        if isinstance(value, str):
            doc.update(text=True, rows=1, cols=len(value), data=[value])
        elif np is not None and isinstance(value, np.ndarray) and value.ndim <= 2:
            block = value if value.ndim == 2 else value.reshape(1, -1)
            doc["rows"], doc["cols"] = block.shape
            r0 = max(1, min(int(ask["r0"]), doc["rows"]))
            c0 = max(1, min(int(ask["c0"]), doc["cols"]))
            r1 = max(r0, min(int(ask["r1"]), doc["rows"]))
            c1 = max(c0, min(int(ask["c1"]), doc["cols"]))
            doc["r0"], doc["c0"] = r0, c0
            doc["data"] = [[float(x) for x in row] for row in block[r0 - 1:r1, c0 - 1:c1]]
        elif isinstance(value, (list, tuple)):
            doc["rows"], doc["cols"] = 1, len(value)
            doc["r0"], doc["c0"] = 1, 1
            doc["data"] = [[float(x) for x in value]]
        else:
            doc["error"] = "%s is a %s — no grid to show" % (doc["name"], type(value).__name__)
    except KeyError:
        doc["error"] = "%s is not defined here" % doc["name"]
    except Exception as problem:
        doc["error"] = str(problem)
    _write(out, doc)


def _log(message):
    """Say why, if anybody is listening.

    Everything in this module runs inside a `try` that must not break the user's REPL, which
    means a mistake in here has exactly one symptom: a panel that quietly stops changing. That
    has cost an hour twice on the Octave side. Setting CLEECODE_DBG_LOG turns the silence off."""
    where = os.environ.get("CLEECODE_DBG_LOG")
    if not where:
        return
    try:
        with open(where, "a") as handle:
            handle.write("%.3f %s\n" % (time.time(), message))
    except Exception:
        pass


def _write(path, doc):
    tmp = "%s.%d.tmp" % (path, os.getpid())
    with open(tmp, "w") as handle:
        json.dump(doc, handle)
    os.replace(tmp, path)          # atomic: no reader ever sees half a file


def _breakpoints(state):
    """Pick up the breakpoints CleeCode wants, and trace only while there are any.

    Tracing costs a call per Python line, so it goes on when the first breakpoint is set and off
    when the last one goes: a session that never sets one pays nothing at all. The whole set
    arrives every time and is applied from scratch, so a missed message cannot leave the session
    and the editor disagreeing about where the breakpoints are."""
    req = os.environ.get("CLEECODE_PY_BREAK")
    if not req or not os.path.exists(req):
        return
    stamp = os.path.getmtime(req)
    if state.get("break_seen") == stamp:
        return
    state["break_seen"] = stamp
    try:
        ask = json.load(open(req))
    except Exception:
        return

    wanted = [(str(one.get("path") or one.get("name", "")), int(one.get("line", 0)))
              for one in ask]
    wanted = [(path, line) for path, line in wanted if path.endswith(".py") and line > 0]
    if not wanted:
        state["dbg"] = None
        sys.settrace(None)
        return

    debugger = _Debugger(state)
    for path, line in wanted:
        problem = debugger.set_break(path, line)
        _log("break %s:%d -> %s" % (path, line, problem or "ok"))
    state["dbg"] = debugger


def _make_debugger_class():
    """Built on first use: importing pdb at startup would cost every session that never debugs."""
    import pdb

    class _Debugger(pdb.Pdb):
        """pdb, with the panel told where we are before the user is asked what to do.

        pdb has a prompt of its own and never consults `sys.ps1`, so a session stopped in the
        debugger would otherwise leave the panel saying whatever it last said — precisely when
        it is worth reading. Publishing from `interaction` covers stepping too, because every
        stop pdb makes comes back through here.

        Nothing clears the stopped state on the way out: `interaction` returns both when the
        program is done and when it is merely stepping to the next line, and those look
        identical from in here. The REPL prompt is the thing that only happens when we are
        genuinely back, so it is what clears it."""

        def __init__(self, state):
            super().__init__()
            self._state = state

        def canonic(self, filename):
            """Resolve symlinks, which bdb's own version does not.

            bdb matches a breakpoint against a running frame by comparing filenames after
            `abspath`, and abspath does not follow links. On a Mac /var is a link to
            /private/var, so an editor that resolved the path and an `import` that did not
            will name the same file two ways and agree about nothing — measured: the session
            ran straight past the breakpoint, with no error anywhere and nothing to see.

            Both sides of that comparison come through here, so resolving here fixes it once.
            The cache is bdb's own, and matters: this is asked for every frame of every call
            while tracing."""
            if filename.startswith("<") and filename.endswith(">"):
                return filename                       # <python-input-3> and friends
            known = self.fncache.get(filename)
            if known is None:
                known = os.path.normcase(os.path.realpath(filename))
                self.fncache[filename] = known
            return known

        def interaction(self, frame, traceback):
            self._state["frame"] = frame
            try:
                # Both of the panel's questions, answered from here as well as from the REPL
                # prompt: being stopped is when looking inside a variable is worth most, and
                # the REPL prompt will not come round again until the program is finished.
                _snapshot(self._state)
                _answer_slice(self._state)
            except Exception:
                pass
            super().interaction(frame, traceback)

        def postcmd(self, stop, line):
            """After every pdb command, including the ones that do not move.

            `interaction` covers arriving somewhere; this covers being asked something while
            there — an inspector opened at the debug prompt gets its answer."""
            try:
                _answer_slice(self._state)
            except Exception:
                pass
            return super().postcmd(stop, line)

    return _Debugger


def _Debugger(state):                                  # noqa: N802 — reads as the class it makes
    global _DEBUGGER_CLASS
    if _DEBUGGER_CLASS is None:
        _DEBUGGER_CLASS = _make_debugger_class()
    return _DEBUGGER_CLASS(state)


_DEBUGGER_CLASS = None


def _debug_state(state):
    frame = state.get("frame")
    if frame is None:
        return {"stopped": False, "name": "", "file": "", "line": 0, "stack": []}
    # Up to the statement the user ran, and no further. Above that is the REPL's own machinery
    # — runcode, runsource, push, the whole interpreter loop — which is a true account of how
    # the program got here and no help at all in finding out why it is wrong.
    stack = []
    walk = frame
    while walk is not None and len(stack) < 20:
        stack.append({"name": walk.f_code.co_name, "line": walk.f_lineno})
        name = walk.f_code.co_filename
        if name.startswith("<python-input") or name == "<stdin>":
            break
        walk = walk.f_back
    return {
        "stopped": True,
        "name": frame.f_code.co_name,
        "file": frame.f_code.co_filename,
        "line": frame.f_lineno,
        "stack": stack,
    }


def capture_on_exit():
    """Arrange for a *script's* figures to reach their tabs when it ends.

    A session installs the full hook from PYTHONSTARTUP, which Python reads only when it is
    interactive. `python3 plot.py` — which is what the Run button does when no prompt is open —
    therefore installed nothing, and that was worse than doing nothing: CleeCode already points
    matplotlib at its own windowless backend, so `plt.show()` in a script drew no window *and*
    handed the figure to nobody. The plot simply did not exist anywhere.

    This is the other half of that decision, and the mirror of what Octave already does through
    the PKG_ADD on its load path: every interpreter CleeCode starts can hand its plots over,
    session or script.

    It leaves `_STATE` behind: that is what `cleecode_pyws.frame()` needs to work inside a
    script's own loop, and what the backend's exit hook writes through when the script ends. A
    script that never draws anything pays for an import and a function call that returns.
    """
    global _STATE                                      # noqa: PLW0603 — one process, one state
    if _STATE is not None:
        return                                         # an interactive session got here first
    out = os.environ.get("CLEECODE_PY_WS")
    figdir = os.environ.get("CLEECODE_PY_FIGS")
    if not out or not figdir:
        return                                         # not started by CleeCode: nothing to do
    try:
        os.makedirs(figdir, exist_ok=True)
    except OSError:
        return
    _STATE = {"out": out, "figdir": figdir, "seq": 0, "pending": True,
              "frame": None, "dbg": None, "drawn": {}}


def capture_now():
    """Hand over whatever figures are open, right now. Called as a script ends.

    Only if it drew any. The snapshot carries the variables as well, and a shell tab whose
    workspace panel filled up with the leftovers of every `python3 something.py` typed in it
    would be reporting a session that does not exist.

    The *when* lives in `cleecode_mpl`, which is imported late enough for its atexit handler to
    run before matplotlib destroys the figures. See the note there.
    """
    state = _STATE
    if state is None:
        return
    plt = sys.modules.get("matplotlib.pyplot")
    if plt is None:
        return
    try:
        if not plt.get_fignums():
            return
        _snapshot(state)
    except Exception:                                  # noqa: BLE001 — an exit is not a place to raise
        pass


def close_figures(*numbers):
    """Close these figures, if they are still open, and say nothing either way.

    Called by CleeCode before it reruns a file it ran before. matplotlib numbers a figure when
    it is created, so `plt.subplots()` in a script run three times makes figures 1,2 then 3,4
    then 5,6 — six tabs of what the person at the keyboard thinks of as two plots. Closing the
    ones the previous run left frees those numbers, and the rerun lands on 1 and 2 again, in the
    tabs that are already open.

    Only the numbers it is given, so a figure made by hand at the prompt is none of its
    business. And nothing is printed or returned: this is typed at the user's own prompt, and a
    line of CleeCode's that answers `[None, None]` into their transcript would be worse than the
    problem it fixes.
    """
    plt = sys.modules.get("matplotlib.pyplot")
    if plt is None:
        return
    for num in numbers:
        try:
            if plt.fignum_exists(num):
                plt.close(num)
        except Exception:                              # noqa: BLE001 — a stale number is not an error
            pass


def frame():
    """Put what the figures look like right now into their tabs. For a loop.

    Everything else here happens at the prompt: the snapshot is written when a statement
    finishes, which is the right design for a panel of variables and means a loop is invisible
    while it runs — the tab holds the frame from before it until it is over. So a loop that
    wants to be watched says so:

        for k in range(200):
            line.set_ydata(np.sin(x + k / 20))
            _cleecode_pyws.frame()

    The underscore is not an accident: PYTHONSTARTUP runs in the user's own namespace, and the
    one name this leaves there is `_cleecode_pyws`, so that `dir()` and the workspace panel stay
    the user's own. This is the one thing on it worth calling by hand.

    Outside CleeCode it does nothing: there is no directory to write to.
    """
    state = _STATE
    if state is None:
        return
    try:
        _snapshot(state)
    except Exception:                                  # noqa: BLE001 — a frame is not worth a raise
        pass


def install(out=None, figdir=None):
    out = out or os.environ.get("CLEECODE_PY_WS")
    if not out:
        return
    figdir = figdir or os.environ.get("CLEECODE_PY_FIGS") or os.path.dirname(out)
    os.makedirs(figdir, exist_ok=True)
    # `pending` starts true so the panel has something in it before the first command: an
    # empty workspace is a fact about the session, not a failure to report one.
    state = {"out": out, "figdir": figdir, "seq": 0, "pending": True,
             "frame": None, "dbg": None, "drawn": {}}
    # Kept where `frame()` can reach it. The hooks below close over `state` and have never
    # needed a global; the one thing a user calls by hand does.
    global _STATE                                      # noqa: PLW0603 — one session, one state
    _STATE = state
    sys.addaudithook(_statement_watcher(state))
    sys.ps1 = _Prompt(getattr(sys, "ps1", ">>> "), state)
    _slice_watcher(state)
