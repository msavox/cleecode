#!/usr/bin/env bash
# Key script for the README screenshots, played into the tmux session docs/shots.tape attaches
# to. Run by the tape, not by hand. See docs/demo-keys.sh for why the keys live in a script, and
# why the ones on Ctrl+Shift are reached from the command palette instead.
#
# Each shot is set up and then held still for a few seconds. The tape does not count those
# seconds: it waits for text that only appears once the state is on screen (`Wait+Screen`), so a
# slow machine delays a screenshot rather than mis-taking it.

set -u
S="${1:-shots}"
k() { tmux send-keys -t "$S" "$@"; }
t() { tmux send-keys -t "$S" -l "$1"; }

palette() {
    k C-p; sleep 0.7
    t "$1"; sleep 1.1
    k Enter; sleep "${2:-1.5}"
}

sleep 3.5

# ---- main.png: the whole window doing something. A Python file open and highlighted, and its
#      output sitting in the first terminal.
#
#      The venv was once picked here too, through a palette entry called "venv". That entry is
#      now "How this file runs...", and its menu offers whatever venvs the project actually has
#      — none, in a Rust repository — so the old blind "down, enter" landed on the disk browser
#      and the shot never happened. What the picture is for is the window working, so it now
#      does the one thing that always works.
k C-o; sleep 0.8
t "hello.py"; sleep 1.2
k Enter; sleep 1.5
palette "run current" 6.0
sleep 4                            # held for the shot

# ---- menu.png: the menu bar open on Layout, where the presets live. The initials are
#      underlined only while it is open, which is the only time pressing one does anything.
palette "open the menu bar" 1.2
k Right; sleep 0.4
k Right; sleep 0.4
k Right; sleep 0.4
k Right; sleep 1.0
sleep 4                            # held for the shot
k Escape; sleep 1.0

# ---- split.png: two independent editors, each with its own tabs and its own Run button.
k C-l; sleep 1.8
k C-o; sleep 0.8
t "main.rs"; sleep 1.2
k Enter; sleep 2.0
sleep 4                            # held for the shot

# ---- manual.png: the built-in guide, left on Overview, which carries the diagram of the
#      whole window and now shows the colouring — chords yellow, headings cyan, rules dimmed.
palette "manual" 2.0
sleep 5                            # held for the shot

# ---- palette.png: the command palette, which is how anything is reached without a shortcut.
k Escape; sleep 0.8
k C-p; sleep 0.8
t "term"; sleep 1.5
sleep 3                            # held for the shot
k Escape; sleep 0.8
