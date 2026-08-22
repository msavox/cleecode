#!/usr/bin/env python3
"""Open `clee -w octave` and `clee -w pylab` and check what you actually get.

    python3 scripts/drive_presets.py [path/to/clee]

A preset is a promise about what appears when you type its name, and the only way to check a
promise like that is to type it. Every check here is about the screen: is the interpreter at its
prompt, is there a shell beside it, did the frames land where the width says they should, and —
running the same preset in a narrow window — did they move.

Skips a language whose interpreter is not installed rather than passing quietly.
"""

import os
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_drive import Report, Session, binary_from_argv  # noqa: E402

PRESETS = [
    {"name": "octave", "needs": "octave", "prompt": ">>", "tab": "octave"},
    {"name": "pylab", "needs": "python3", "prompt": ">>>", "tab": "python"},
]


def open_preset(binary, name, root, cols):
    """A session started as `clee -w <name> .`, in a window `cols` wide."""
    return Session(binary, root, args=["-w", name], cols=cols)


def check_preset(binary, spec, report):
    root = tempfile.mkdtemp(prefix="clee_preset_")
    # A script to work on, so the preset opens over something realistic.
    with open(os.path.join(root, "demo." + ("m" if spec["name"] == "octave" else "py")), "w") as h:
        h.write("%% one\na = 1;\n" if spec["name"] == "octave" else "# %% one\na = 1\n")

    wide = open_preset(binary, spec["name"], root, 190)
    try:
        started = wide.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20)
        report.check(f"{spec['name']}: the preset opens", started, wide)
        if not started:
            return
        wide.send(" ")
        wide.wait(lambda s: "Files" in s.text(), 8)

        # The interpreter starts itself. Nothing was typed at it.
        at_prompt = wide.wait(lambda s: spec["prompt"] in s.text(), 40)
        report.check(f"{spec['name']}: the interpreter is already at its prompt", at_prompt, wide,
                     note="nothing was typed to start it")

        # On the tab, not merely on screen. The name is written in the menu bar's workspace
        # label, in Octave's own banner (three times, in URLs) and in the shell echo that
        # started it — so "somewhere on screen" was satisfied for both presets with the tab
        # strip removed entirely.
        strip = next((line for line in wide.lines() if "shell ✕" in line), "")
        report.check(f"{spec['name']}: its tab carries the interpreter's name",
                     spec["tab"] in strip, wide, note=repr(strip[:60]))
        report.check(f"{spec['name']}: a plain shell sits beside it in the same window",
                     "shell" in wide.text(), wide)

        # Underneath, and at this width on purpose. This check used to require the prompt to
        # be *beside* the editor on a wide window, and it had been failing since 0.9.2 without
        # anyone reading it: the presets put the prompt underneath at every width, because the
        # editor splits to put a figure next to the code that drew it and a third window
        # alongside makes each half a third of the screen. A plot a third of a window wide is a
        # thumbnail. The decision is in docs/ROADMAP.md under 0.9.2; the check now holds it.
        term_col = wide.column_of("shell")
        term_row = wide.row_of("shell")
        report.check(f"{spec['name']}: at 190 columns the prompt is underneath, not beside",
                     term_col is not None and term_col < 95 and term_row is not None and term_row > 10,
                     wide, note=f"the terminal starts at column {term_col} of 190, row {term_row}")
    finally:
        wide.close()

    narrow = open_preset(binary, spec["name"], root, 92)
    try:
        if not narrow.wait(lambda s: sum(1 for l in s.lines() if l.strip()) > 3, timeout=20):
            report.check(f"{spec['name']}: the preset opens narrow", False, narrow)
            return
        narrow.send(" ")
        narrow.wait(lambda s: "Files" in s.text(), 8)
        narrow.wait(lambda s: spec["prompt"] in s.text(), 40)
        term_col = narrow.column_of("shell")
        term_row = narrow.row_of("shell")
        # And underneath here too — the same arrangement, which is the point. This used to be
        # written as "it *moves* underneath instead", implying the wide window put it elsewhere;
        # it passed against a layout identical to the wide one and so demonstrated nothing.
        report.check(f"{spec['name']}: at 92 columns it is underneath as well",
                     term_col is not None and term_col < 46 and term_row is not None and term_row > 10,
                     narrow, note=f"the terminal starts at column {term_col}, row {term_row}")
        report.check(f"{spec['name']}: the file tree survives the narrow window",
                     "Files" in narrow.text(), narrow)
    finally:
        narrow.close()
        shutil.rmtree(root, ignore_errors=True)


def main():
    binary = binary_from_argv(sys.argv)
    report = Report()
    for spec in PRESETS:
        if shutil.which(spec["needs"]) is None:
            print(f"  SKIP  {spec['name']}: {spec['needs']} not installed")
            continue
        check_preset(binary, spec, report)
    return report.finish()


if __name__ == "__main__":
    try:
        sys.exit(main())
    except BrokenPipeError:
        os._exit(0)
