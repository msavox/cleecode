# Demo recording for the README, played by vhs (https://github.com/charmbracelet/vhs):
#
#   cargo build --release && vhs docs/demo.tape
#
# Deterministic: the same tape produces the same GIF, so the demo can be re-recorded after a UI
# change rather than re-shot by hand.
#
# It used to run CleeCode inside tmux, with the keystrokes in a companion script, because vhs
# cannot send function keys or real Alt chords while `tmux send-keys` can. That stopped working —
# keys sent through tmux no longer reach the application — so tmux is gone and the keys are
# pressed here. What survives the loss is everything reachable through plain Ctrl chords, the
# arrows, and the command palette; that turns out to be the whole demo, because the palette is
# how most people find an action they have not memorised anyway. Ctrl+Tab is not among what vhs
# can send either, so moving between frames goes through the palette's Focus entries rather than
# the chord a real user would press.
#
# No Wait+Screen here, unlike docs/shots.tape. A screenshot taken at the wrong moment is a wrong
# picture and worth guarding against; a demo only needs pacing, and one wait that never matches
# takes the whole recording down with it.
#
# 1800x1000 gives 175x46 cells. The old recording was 1400x800 — 135x36 — while the tmux session
# inside it demanded 172x48, which is why it always looked cramped. Measure before changing this:
# a tape that prints `stty size` is two minutes of work and settles the question.
#
# Nothing here shows a picture or a PDF as real pixels. vhs speaks no graphics protocol, so those
# tabs fall back to half-blocks, and a recording that claimed otherwise would be a lie told in a
# README. What it *can* show honestly is chafa putting a photograph into a terminal pane, which
# is symbols all the way down and looks the same everywhere.

Output docs/demo.gif

Set Shell "bash"
# The font's own family name. The file is named for this project but its name table still says
# JetBrainsMono NFM, and a family vhs can't resolve silently falls back to a serif face whose
# advance widths don't line up with the terminal grid.
Set FontFamily "JetBrainsMono NFM"
Set FontSize 16
Set Width 1800
Set Height 1000
Set Padding 12
Set Framerate 20

# Off camera: a throwaway config, so the author's own layout cannot leak into the recording and
# the recording cannot overwrite it; and `clee` on PATH pointing at the build under test, so the
# only command typed on screen is the one anyone would type.
Hide
Type "export XDG_CONFIG_HOME=$(mktemp -d) BIN=$(mktemp -d)"
Enter
Type "ln -sf $PWD/target/release/clee $BIN/clee && export PATH=$BIN:$PATH && clear"
Enter
Sleep 1s
Show

# `clee` with no argument opens the directory it was invoked from, which is also the only way to
# see the splash: a path argument skips it.
Type "clee"
Enter
Sleep 5s

# ---- The file tree: walk it, open a Python file, run it -------------------------- ~16s
Down
Sleep 0.5s
Down
Sleep 0.5s
Down
Sleep 0.6s
Right
Sleep 1.4s
Down
Sleep 0.4s
Down
Sleep 0.4s
Down
Sleep 0.4s
Down
Sleep 0.9s
Enter
Sleep 2.5s

Ctrl+p
Sleep 1s
Type "run current"
Sleep 1.2s
Enter
Sleep 7s

# ---- A photograph through chafa. Run on a picture defaults to it, and the result is symbols,
#      so it records the same everywhere.
#
#      The picture *tab* is opened and closed again rather than dwelt on, because here it draws
#      nothing at all: vhs answers the capability query claiming sixel and then renders none of
#      it, so the pane comes out black. That is a fact about this recorder, not about CleeCode —
#      but a viewer cannot tell those apart, and an empty pane reads as a broken program. The
#      real thing is photographed by hand in docs/screenshots/preview-image.jpg.
Ctrl+o
Sleep 1s
Type "schermata.png"
Sleep 1.2s
Enter
Sleep 1s

Ctrl+p
Sleep 0.9s
Type "run current"
Sleep 1.1s
Enter
Sleep 1.2s
Ctrl+w
Sleep 6s

# ---- Markdown beside its source. One file, two tabs, one copy of the text. Without a graphics
#      protocol this is the styled-text fallback rather than a typeset document — which is what
#      this terminal would give you too.
Ctrl+o
Sleep 1s
Type "guida.md"
Sleep 1.5s
Enter
Sleep 2.5s

Ctrl+p
Sleep 1s
Type "run current"
Sleep 1.2s
Enter
Sleep 4s

# ---- Two independent editors, each with its own tabs and its own Run button ------- ~8s
Ctrl+l
Sleep 3s
Ctrl+o
Sleep 1s
Type "main.rs"
Sleep 1.2s
Enter
Sleep 3.5s

# ---- Frames: the tree, each editor half and every shell is one, and focus moves between them.
#      Ctrl+Tab cycles them in real use — vhs cannot send that chord, so this goes through the
#      palette, which is the other honest route to the same thing.
Ctrl+p
Sleep 1s
Type "focus terminal"
Sleep 1.2s
Enter
Sleep 1.8s

Ctrl+p
Sleep 1s
Type "focus file tree"
Sleep 1.2s
Enter
Sleep 1.8s

Ctrl+p
Sleep 1s
Type "focus editor"
Sleep 1.2s
Enter
Sleep 1.8s

# ---- A named workspace: the whole set-up — root, open files, frame sizes, terminals with their
#      names and startup commands — saved under one name.
Ctrl+p
Sleep 1s
Type "save workspace"
Sleep 1.2s
Enter
Sleep 1.4s
Type "demo"
Sleep 1.2s
Enter
Sleep 2.5s

# ---- The menu bar, then the built-in manual -------------------------------------- ~13s
Ctrl+p
Sleep 1s
Type "open the menu bar"
Sleep 1.2s
Enter
Sleep 1.2s
Right
Sleep 0.6s
Right
Sleep 0.6s
Right
Sleep 0.9s
Down
Sleep 0.5s
Down
Sleep 1.4s
Escape
Sleep 1s

Ctrl+p
Sleep 1s
Type "manual"
Sleep 1.2s
Enter
Sleep 3s
Down
Sleep 0.5s
Down
Sleep 1.2s
Tab
Sleep 2s
Escape
Sleep 1.5s

# End on the split editor rather than an empty shell: the GIF loops.
Sleep 2s
