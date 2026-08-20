# Design notes

The working record behind the numeric side, kept because the measurements are the argument.

Each of these was written against a real interpreter driven in a PTY, not reasoned about — the
value is in what the measurements *ruled out*, which is the part that would otherwise be
rediscovered the hard way.

  · [One feature, two backends](ide-mode.md) — the seam, and why both languages share it
  · [CleeCode + Octave](ide-mode-octave.md) — the idle hook, figures, the debugger
  · [The same thing for Python](ide-mode-python.md) — two mechanisms where Octave needs one

For what any of it does rather than how, see [the numeric guide](../numeric.md).
