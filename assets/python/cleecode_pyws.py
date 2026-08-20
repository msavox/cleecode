"""Workspace snapshots and figure captures for CleeCode, from a plain Python REPL.

Python has no equivalent of Octave's add_input_event_hook, and it takes two mechanisms to
build one. Between them they give what Octave cannot: an exact callback per statement,
with no polling and no idle cost at all.

  · An audit hook on `exec` says *which* statement. It carries the code object, and the
    REPL compiles each thing you type under a name of its own — `<python-input-7>`, or
    `<stdin>` in the basic REPL. But it is raised before the statement runs, so it is a
    counter, not a moment.

  · `str(sys.ps1)` says *when the statement finished*, because the prompt is drawn after
    it. But the REPL draws the prompt far more often than that: measured on 2026-08-20,
    four statements restringify it 65 times under PyREPL, and typing twelve characters
    without pressing Enter accounts for 23 more.

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
            if self._state["pending"]:
                self._state["pending"] = False
                _snapshot(self._state)
        except Exception:
            pass
        return self._text


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

    return hook


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
    plt = sys.modules.get("matplotlib.pyplot")
    if plt is None:
        return []
    out = []
    for num in plt.get_fignums():
        fig = plt.figure(num)
        png = os.path.join(state["figdir"], f"fig{num}.png")
        w, h = fig.get_size_inches() * fig.dpi
        fig.savefig(png, dpi=fig.dpi)
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


def _snapshot(state):
    state["seq"] += 1
    doc = {"v": 1, "seq": state["seq"], "time": time.time(), "pid": os.getpid(),
           "cwd": os.getcwd(), "lang": "python",
           "vars": [_describe(n, v) for n, v in sorted(_user_vars().items())],
           "figures": _figures(state)}
    tmp = f"{state['out']}.{os.getpid()}.tmp"
    with open(tmp, "w") as f:
        json.dump(doc, f)
    os.replace(tmp, state["out"])       # atomic: no reader ever sees half a file


def install(out=None, figdir=None):
    out = out or os.environ.get("CLEECODE_PY_WS")
    if not out:
        return
    figdir = figdir or os.environ.get("CLEECODE_PY_FIGS") or os.path.dirname(out)
    os.makedirs(figdir, exist_ok=True)
    # `pending` starts true so the panel has something in it before the first command: an
    # empty workspace is a fact about the session, not a failure to report one.
    state = {"out": out, "figdir": figdir, "seq": 0, "pending": True}
    sys.addaudithook(_statement_watcher(state))
    sys.ps1 = _Prompt(getattr(sys, "ps1", ">>> "), state)
