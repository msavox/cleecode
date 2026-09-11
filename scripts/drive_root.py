#!/usr/bin/env python3
"""Which folder a session opens in, when a workspace has an opinion about it.

    python3 scripts/drive_root.py [path/to/clee]

A saved workspace pins the project it was saved in, and `clee -w work` from anywhere is how you
get back to it. A directory typed on the same line says otherwise — open this set-up *here* — and
the two must not be the same command: the shape comes over either way, the root does not.

Needs nothing installed. The fixture is two throwaway folders with one file each, so which root
won is readable straight off the file tree.
"""

import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_drive import Report, Session, binary_from_argv  # noqa: E402

WORKSPACE = """
name = "work"
root = "{root}"
open_files = ["{file}"]
active_terminal = 0

[layout]
show_sidebar = true
show_terminal = true
show_menubar = true
sidebar_width = 30
terminal_pct = 20
terminal_on_right = false
split_view = false
split_pct = 50
"""


def fixture():
    """Two projects and a workspace saved in the first, ready to be opened from the second.

    The workspace file goes under the *second* folder's config, since that is the one a session
    launched there reads: `Session` puts XDG_CONFIG_HOME inside the directory it is given."""
    tmp = tempfile.mkdtemp(prefix="clee-root-")
    saved, here = os.path.join(tmp, "saved-project"), os.path.join(tmp, "where-i-stand")
    os.makedirs(saved)
    os.makedirs(here)
    with open(os.path.join(saved, "theirs.rs"), "w") as f:
        f.write("// the project the workspace was saved in\n")
    with open(os.path.join(here, "mine.rs"), "w") as f:
        f.write("// the project I am standing in\n")
    workspaces = os.path.join(here, ".config", "cleecode", "workspaces")
    os.makedirs(workspaces, exist_ok=True)
    with open(os.path.join(workspaces, "work.toml"), "w") as f:
        f.write(WORKSPACE.format(root=saved, file=os.path.join(saved, "theirs.rs")))
    return saved, here


def tree_settled(session):
    """Either project's file on screen: whichever it is, the root has been decided."""
    screen = "\n".join(session.lines())
    return "mine.rs" in screen or "theirs.rs" in screen


def main():
    binary = binary_from_argv(sys.argv)
    report = Report()

    # `clee -w work .` — the set-up, applied where I am.
    saved, here = fixture()
    session = Session(binary, here, args=["-w", "work"])
    try:
        session.wait(tree_settled, timeout=20)
        screen = "\n".join(session.lines())
        report.check("a directory next to -w is the project", "mine.rs" in screen, session)
        # The workspace's own files live in a folder this session is not in. Opening them would
        # put somebody else's work in front of you under this project's name.
        report.check("the saved project's files stay behind", "theirs.rs" not in screen, session)
        report.check("the workspace was still applied", "work" in screen, session)
    finally:
        session.close()

    # `clee -w work` — no directory said, so the file's own root stands.
    saved, here = fixture()
    session = Session(binary, here, args=["-w", "work"], project=None)
    try:
        session.wait(tree_settled, timeout=20)
        screen = "\n".join(session.lines())
        report.check("with no directory the saved root stands", "theirs.rs" in screen, session)
        report.check("and the folder it was launched from does not",
                     "mine.rs" not in screen, session)
    finally:
        session.close()

    return report.finish()


if __name__ == "__main__":
    sys.exit(main())
