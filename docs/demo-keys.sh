#!/usr/bin/env bash
# Key script for the README recording, played into the tmux session that docs/demo.tape
# attaches to. Run by the tape, not by hand.
#
# Two things it has to work around:
#
#  - vhs cannot send function keys, and its Alt+<key> arrives as ESC then the letter, far enough
#    apart to read as two events. So the keys come from here, through `tmux send-keys`.
#  - tmux cannot deliver Ctrl+Shift+<letter> either — the same fact that makes those chords safe
#    inside a terminal (no terminal can encode them) stops them being injected. So every action
#    that lives on Ctrl+Shift is reached from the command palette instead, which is honest: the
#    palette is how most people find an action they have not memorised.
#
# Frame navigation *is* driven by its real keys, Ctrl+Alt with an arrow, because tmux sends those
# perfectly well and it is the headline of this release.
#
# The tape's total Sleep must cover TOTAL below; keep the two in step by hand when editing.

set -u
S="${1:-demo}"
k() { tmux send-keys -t "$S" "$@"; }    # a key or chord
t() { tmux send-keys -t "$S" -l "$1"; } # literal text, never read as key names

# Runs an action by name through the command palette.
palette() {
    k C-p; sleep 0.7
    t "$1"; sleep 1.1
    k Enter; sleep "${2:-1.5}"
}

# ---- The splash, on a bare `clee` with no argument. It clears itself after 1.8s, so this
#      just waits for it and for the first shells to settle. ----------------------- ~5s
sleep 5.0

# ---- File tree: walk it, expand a folder, open a file ---------------------------- ~9s
k Down; sleep 0.6                 # assets
k Down; sleep 0.6                 # docs
k Down; sleep 0.5                 # examples
k Right; sleep 1.4                # expand it
k Down; sleep 0.6                 # hello.m
k Down; sleep 0.8                 # hello.py
k Enter; sleep 2.2                # open it — syntax highlighting, line numbers

# ---- Pick the interpreter from the venv drop-down, so Run below uses the project's
#      own .venv ------------------------------------------------------------------- ~6s
palette "venv" 1.4
k Down; sleep 1.0
k Enter; sleep 1.6

# ---- Run it. The command goes to the first idle terminal. ------------------------- ~8s
palette "run current" 6.0

# ---- The same for Octave. `--persist` leaves the interpreter running afterwards. -- ~10s
k C-o; sleep 0.8                  # quick open
t "hello.m"; sleep 1.2
k Enter; sleep 1.6
palette "run current" 6.0

# ---- Frame navigation, the new part: Ctrl+Alt and an arrow goes to whatever frame
#      lies that way, whether it is the tree, an editor pane or a shell. ----------- ~10s
k C-M-Down; sleep 1.8             # into the terminals
k C-M-Right; sleep 1.5            # the next terminal window along
k C-M-Up; sleep 1.5               # back to the editor
k C-M-Left; sleep 1.8             # out to the file tree
k C-M-Right; sleep 1.5            # and back

# ---- Split the editor: two panes, each with its own tab strip and Run button ------ ~7s
k C-l; sleep 2.2
k C-o; sleep 0.7
t "main.rs"; sleep 1.0
k Enter; sleep 2.0
k C-M-Left; sleep 1.2             # the split halves are frames too

# ---- Terminals: a second tab inside the focused window --------------------------- ~6s
palette "new terminal tab" 2.0
palette "new terminal tab" 2.2

# ---- Named workspaces: save the whole set-up — root, open files, frame sizes, and
#      the terminals with their names and startup commands — under one name -------- ~7s
palette "save workspace" 1.4
t "demo"; sleep 1.4
k Enter; sleep 2.6

# ---- The menu bar, and the manual ------------------------------------------------ ~14s
palette "open the menu bar" 1.4
k Right; sleep 0.7                # File
k Right; sleep 0.7                # Edit
k Right; sleep 0.9                # View
k Down Down; sleep 1.5            # its entries
k Escape; sleep 1.0

palette "manual" 2.5
k Down Down Down; sleep 1.5       # scroll the section
k Tab; sleep 2.0                  # next section
k Escape; sleep 1.0

# ---- End on the split editor rather than an empty shell: the GIF loops. ---------- ~3s
sleep 2.5

# TOTAL ~ 86s
