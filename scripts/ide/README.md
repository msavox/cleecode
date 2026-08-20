# Probes for the Octave and Python IDE mode

Harnesses for the prototypes in `assets/octave/` and `assets/python/`, described in
`docs/ide-mode.md` and its two companions. None of them is wired into `cargo test`: each
drives a real interpreter in a pseudo-terminal, which is the only way to exercise a hook that
by definition only fires at an interactive prompt.

They expect to be run from a directory holding the library files, because that is how both
interpreters find them — `addpath` for Octave, `PYTHONPATH` for Python:

    cd assets/octave  && python3 ../../scripts/ide/octave_ws_test.py
    cd assets/python  && python3 ../../scripts/ide/python_ws_test.py

| file | what it is for |
|---|---|
| `octave_ws_test.py` | The regression test. Eleven commands, one at a time, checking the snapshot after each: an in-place edit that changes no metadata, a command too fast to leave a timing gap, and the same command run twice in a row — the cases the change detection exists for. |
| `octave_ws_e2e.py` | Burst behaviour, and a look at what the user's transcript ends up containing. |
| `python_ws_test.py` | The Python side, end to end: what a snapshot *contains*. **Pins `TERM="dumb"`,** which makes CPython fall back to the basic REPL — see the correction in `docs/ide-mode-python.md`, since that pin is why the prompt hook was once measured as firing per statement. Anything checking *cadence* must not use it. |
| `python_cadence_test.py` | How *often* the hook fires, under real PyREPL, which it insists on. One snapshot per statement however many times the prompt is drawn, the snapshot showing the statement's result rather than the state before it, and nothing at all from typing that was never run. |
| `octave_plot_probe.m` | Prints a figure to PNG from inside the idle hook. |

One thing to know before believing a failure: the harness must keep draining the master fd
between sends. An early version stopped reading in between, the interpreter blocked on a full
pipe, and it looked exactly like a lost update. If snapshots seem to go missing, suspect the
harness first.
