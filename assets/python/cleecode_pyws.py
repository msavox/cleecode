"""Workspace snapshots and figure captures for CleeCode, from a plain Python REPL.

Python has no equivalent of Octave's add_input_event_hook. It has something better:
the REPL calls str(sys.ps1) every time it is about to draw the prompt, so an object
with a __str__ fires — after assignments too, which sys.displayhook does not.

Installed from PYTHONSTARTUP, which is CleeCode's env-var-gated hook, the same role
~/.octaverc plays for Octave.

NOT YET FIT TO SHIP, and the reason is one line of this file. __str__ calls _snapshot()
with no guard, on the belief that the prompt is drawn once per statement. That holds in
the basic REPL and not in PyREPL, which is the one a user actually gets: measured on
2026-08-20, three statements draw the prompt 4 times under PYTHON_BASIC_REPL=1 and 60
times under PyREPL. Since _snapshot() calls _figures(), which calls fig.savefig(), an
open figure means the PNG is rewritten about twenty times per command.

Before wiring this into CleeCode, pick a trigger that fires once per statement for real:
a guard in __str__ comparing against the last snapshot, IPython's post_run_cell (needed
anyway — IPython does not use sys.ps1 at all), or an audit hook on exec. See
docs/ide-mode-python.md and ROADMAP.md, release 0.9.
"""

import json
import os
import sys
import time

_STAT_LIMIT = 1_000_000      # elements above which min/max/mean are not paid for
_PREVIEW = 60                # characters of repr kept


class _Prompt:
    """Stands in for sys.ps1. Whatever happens in __str__, it must still return a
    prompt: an exception here would leave the user staring at a broken REPL."""

    def __init__(self, text, state):
        self._text = text
        self._state = state

    def __str__(self):
        try:
            _snapshot(self._state)
        except Exception:
            pass
        return self._text


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
    state = {"out": out, "figdir": figdir, "seq": 0}
    sys.ps1 = _Prompt(getattr(sys, "ps1", ">>> "), state)
