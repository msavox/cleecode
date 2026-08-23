"""Runs in every Python CleeCode starts, and does nothing unless one of them draws a plot.

Python reads PYTHONSTARTUP only when it is interactive, so a script — `python3 plot.py`, which
is what the Run button does when no prompt is open — never installed the workspace hook. The
figures went nowhere at all: MPLBACKEND already points matplotlib at CleeCode's own backend, the
one that opens no window, so a plot in a script drew nothing and handed nothing over.

`sitecustomize` is the hook Python does honour for a plain script, and it is reachable because
CleeCode puts its own library on PYTHONPATH. This is the same design the Octave side already
has: a PKG_ADD on the load path, so every Octave started in a CleeCode terminal can hand its
plots over whether or not anybody opened the preset.

All this does is leave the hook's state where the rest of it can find it: the figures are handed
over by an exit hook inside CleeCode's matplotlib backend, which is imported late enough to run
before matplotlib destroys them, and `cleecode_pyws.frame()` in a script's own loop needs the
same state.

Everything here is guarded twice: by the environment, so a Python started anywhere else is
untouched, and by a try, so nothing CleeCode does can stop somebody's interpreter from starting.
"""

import os

if os.environ.get("CLEECODE_PY_FIGS"):
    try:
        import cleecode_pyws as _cleecode_pyws

        _cleecode_pyws.capture_on_exit()
    except Exception:  # noqa: BLE001 — a hook that fails must not take the interpreter with it
        pass

# A project of the user's own may have a sitecustomize too, and CleeCode's library is ahead of it
# on the path — so without this theirs would silently stop running for as long as they used this
# editor. Ours is done; theirs gets the name back.
try:
    import importlib.util as _util
    import sys as _sys

    _mine = os.path.dirname(os.path.abspath(__file__))
    for _entry in _sys.path:
        try:
            if not _entry or os.path.abspath(_entry) == _mine:
                continue
            _candidate = os.path.join(_entry, "sitecustomize.py")
            if not os.path.isfile(_candidate):
                continue
            _spec = _util.spec_from_file_location("sitecustomize", _candidate)
            _module = _util.module_from_spec(_spec)
            _sys.modules["sitecustomize"] = _module
            _spec.loader.exec_module(_module)
            break
        except Exception:  # noqa: BLE001, PERF203 — one unreadable entry is not the end of the path
            continue
except Exception:  # noqa: BLE001
    pass
