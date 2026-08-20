# The same thing for Python

Companion to `ide-mode-octave.md`. Same question — live workspace panel plus figures as editor
tabs — asked of a plain Python REPL. Answer: yes, and it comes out **simpler than Octave
on every axis**. Measured on Python 3.13.14, matplotlib 3.11.1, numpy 2.5.2.

Working prototype in `assets/python/`, exercised by `scripts/ide/python_ws_test.py`.

## The hook

Python has no `add_input_event_hook`. It has something better.

`sys.ps1` does not have to be a string. The REPL calls `str(sys.ps1)` every time it is
about to draw the prompt, so an object with a `__str__` fires **exactly once per
statement** — including after assignments, which `sys.displayhook` does not see (it only
fires for expressions that produce a value).

```python
class _Prompt:
    def __init__(self, text, state): self._text, self._state = text, state
    def __str__(self):
        try: _snapshot(self._state)
        except Exception: pass       # a broken panel must never break the REPL
        return self._text
```

*Measured*: fires once per statement in **both** Python 3.13 REPLs — the new PyREPL and
the old one (`PYTHON_BASIC_REPL=1`). This mattered: 3.13 rewrote the REPL, so it was
genuinely open whether the trick survived.

> **Correction, measured in this repo on 2026-08-20 — the paragraph above is wrong about
> PyREPL, and it is the load-bearing claim of this document.** Same Python 3.13.14, same PTY,
> three statements:
>
> | REPL | fed whole lines | typed character by character |
> |---|---|---|
> | basic (`PYTHON_BASIC_REPL=1`) | 4 | 4 |
> | PyREPL (the default) | **60** | **60** |
>
> Four is right for the basic REPL — three statements plus the closing `print`. PyREPL
> stringifies the prompt around twenty times per statement, whether the text arrives all at
> once or a keystroke at a time. The reason the original measurement missed it is in the
> harness: `scripts/ide/python_ws_test.py` sets `TERM="dumb"`, which makes CPython fall back
> to the basic REPL, so the "both REPLs" check only ever exercised one of them.
>
> This does not sink the mechanism, but it did sink "no polling, no idle cost, no change
> detection needed" as written. `assets/python/cleecode_pyws.py` called `_snapshot()` from
> `__str__` with no guard, and `_snapshot()` calls `_figures()`, which calls `fig.savefig()` —
> so under the REPL a user actually gets, one open figure meant the PNG being rewritten about
> twenty times per command.
>
> **Settled on 2026-08-20, and the headline above survives after all — it just needed a second
> mechanism.** The prompt is the wrong signal on its own, but it is the right *moment*: it is
> drawn after the statement completes. What was missing is something that says a statement ran
> at all, and an audit hook on `exec` says exactly that, once, if it looks at the code object's
> filename — the REPL compiles each thing you type as `<python-input-7>`, or `<stdin>` in the
> basic REPL. So the hook marks and the prompt collects.
>
> Measured on one session: 65 restringifications become 5 snapshots for 5 statements, each
> seeing the namespace as its statement left it, and sixteen keystrokes that were never run
> produce none. `scripts/ide/python_cadence_test.py` is the regression test, and it refuses to
> run under the basic REPL on purpose.
>
> Both simpler answers were measured and are dead under PyREPL.
> `readline.get_current_history_length()` — the direct analogue of the `numel(history())` trick
> the Octave side leans on — returns 0, because PyREPL keeps its own history. And an audit hook
> that does not check the filename sees 52 execs for 4 statements. Installing the hook costs
> nothing measurable: numeric work, pure loops and two thousand `open`+`write` come out
> identical to three decimals with it and without.
>
> So the comparison with Octave still holds, and by a wider margin than it looked: Octave polls
> at 10 Hz and needs a fingerprint to tell whether anything moved, while Python gets an exact
> per-statement callback for nothing. It simply takes two mechanisms to build, one for *which*
> statement and one for *when it finished* — which is also why neither alone was enough.

**This removes the hardest part of the Octave design.** Octave's hook polls at ~10 Hz, so
it needs change detection — the `history()` counter, the `whos` fingerprint, and the
vectorised `sprintf` that took it from 7.46 ms to 0.54 ms per tick (`ide-mode-octave.md` §3). None
of that is needed here. The prompt hook fires when, and only when, something has been run.
Idle cost is exactly zero.

## Installing it

`PYTHONSTARTUP` is the direct analogue of `~/.octaverc`, and better: it is already an
environment variable, so CleeCode sets it per pane and nothing on the user's machine is
edited at all. No `.octaverc` block to write, no gating needed.

```
PYTHONSTARTUP=<lib>/pythonstartup.py
PYTHONPATH=<lib>
CLEECODE_PY_WS=<per-pane snapshot path>
CLEECODE_PY_FIGS=<per-pane figure dir>
```

All of it goes next to `cmd.env("CLEECODE", "1")` at `terminal_panel.rs:365`. `PYTHONPATH`
is what makes the module importable regardless of which venv is active — relevant since
CleeCode already has venv handling (`apply_venv`).

**Namespace pollution is the one trap.** `PYTHONSTARTUP` executes inside the user's
`__main__`, so a careless startup file puts its own imports into the workspace panel. The
startup file is therefore two lines, and everything it leaves behind is
underscore-prefixed and filtered out:

```python
import cleecode_pyws as _cleecode_pyws
_cleecode_pyws.install()
```

Octave has no equivalent problem — its functions live in files on the path.

## Snapshot format

Same shape as the Octave one (`ide-mode-octave.md` §4) so a single Rust type reads both, plus
`"lang": "python"` and a `figures` array. Verified end to end in a PTY: in-place edit
(`arr[0,0] = 999` → `max` 999), NaN counted and excluded, complex arrays summarised as
`|z|`, `del s` removed from the panel, modules filtered out, list/dict/scalar handled.

`size` is the native shape (`[10, 10]`, or `[3]` for 1-D) rather than Octave's
always-2-D convention — don't force them to match, they mean different things.

Note that `fig` and `ax` show up as ordinary variables here. In Octave figures are not
variables; in Python they are, and hiding them would be lying about the namespace.

## Plots

Everything from `ide-mode-octave.md` §6 applies, and every number is better:

| | Octave | matplotlib |
|---|---|---|
| headless | `octave` (qt), **not** `octave-cli` | `MPLBACKEND=Agg`, one env var |
| first render | 813 ms (Qt init) | 319 ms |
| re-render after zoom | 32 ms | **22 ms** |
| 3-D rotate + render | 60 ms | **19 ms** |
| exact PNG size | needs the `paperposition` fix, else 800×600 silently becomes 709×532 | exact by construction: `figsize × dpi` |
| pixel → data | reconstruct from the axes rect | `ax.transData.inverted()`, exact |

The nav round trip is `ax.set_xlim(...)` / `ax.view_init(...)` written to the prompt, same
as Octave. The geometry sidecar (`pos`, `xlim`, `ylim`, `xscale`, `is3d`, `view`) is
already emitted per axes by `_figures()`.

`transData.inverted()` deserves attention: it handles log axes, insets and shared axes
correctly, where hand-rolled linear interpolation from the axes rectangle would be quietly
wrong. It is not usable per mouse-move (that would be a round trip per pixel), but it is
the right thing to validate the local mapping against.

### The part Octave cannot match: a real backend

matplotlib lets a backend be an arbitrary module, chosen by env var. *Measured*: with
`MPLBACKEND=module://cleecode_mpl` and `PYTHONPATH` set, `plt.show()` writes the PNGs and
returns, no window is ever created, and **the user's code is unchanged** — no
`matplotlib.use()` call, no import order to get right.

`assets/python/cleecode_mpl.py` is a working 20-line backend. This is the clean integration
point: `plt.show()` means "show it in CleeCode", and `draw_idle` could later give live
updates during long computations, which the prompt hook alone cannot do because it only
fires between statements.

## Caveats

- **IPython does not use `sys.ps1`.** If the user runs `ipython`, the prompt hook never
  fires. IPython's own mechanism is `ip.events.register("post_run_cell", fn)` — more
  official and more robust than the `ps1` trick. Both paths should exist; detect which
  REPL is running rather than guessing.
- Interactive sessions only, exactly as with Octave. `python script.py` never draws a
  prompt.
- Code that reassigns `sys.ps1` breaks the hook. Rare, and it degrades to "panel stops
  updating" rather than anything worse.
- `MPLBACKEND` must be set before `matplotlib.pyplot` is first imported. The env var
  handles this by construction; a `matplotlib.use()` call would not.

## Verdict for CleeCode

Python is the easier of the two, and the design is close enough to share the Rust side:
one snapshot reader, one plot-tab implementation with the nav round trip, and two small
adapters differing in how the hook is installed and what command re-renders a figure. Build
the Rust panel against the `"lang"` field from the start and Octave and Python are the same
feature, not two.
