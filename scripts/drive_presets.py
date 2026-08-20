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

        report.check(f"{spec['name']}: its tab carries the interpreter's name",
                     spec["tab"] in wide.text(), wide)
        report.check(f"{spec['name']}: a plain shell sits beside it in the same window",
                     "shell" in wide.text(), wide)

        # Beside or underneath is a question about columns, not rows: either way the terminal's
        # tab strip is near the top of the screen. On the right it starts past the middle.
        term_col = wide.column_of("shell")
        report.check(f"{spec['name']}: at 190 columns the prompt is beside the editor",
                     term_col is not None and term_col > 95, wide,
                     note=f"the terminal's tabs start at column {term_col} of 190")
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
        report.check(f"{spec['name']}: at 92 columns it moves underneath instead",
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
