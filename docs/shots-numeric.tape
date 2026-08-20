# The numeric side's still screenshots, taken by vhs (https://github.com/charmbracelet/vhs):
#
#   cargo build --release && vhs docs/shots-numeric.tape
#
# Separate from shots.tape because these need a real Octave on the machine: the point of every
# picture here is a live session, and each shot waits for text only a real interpreter can have
# produced — the variable table filling in, Octave's own debug prompt. On a machine with no
# Octave these time out rather than quietly photographing an empty panel.
#
# It runs in a temporary folder of its own rather than in examples/, and that is not tidiness.
# Octave puts the current directory on its load path, and examples/ holds a plot.m — so a script
# calling plot() from there calls *that file* instead of Octave's own, and the shot would show a
# figure that is wrong for a reason no reader could guess. The two files it needs are copied in.
#
# Everything is reached through the command palette rather than by its shortcut. Ctrl+Shift is
# the application's own key layer and needs the kitty keyboard protocol to be encoded at all;
# vhs does not send it, so a tape pressing Ctrl+Shift+X presses nothing and then waits sixty
# seconds for a cell that never ran.
#
# There is no shot of a figure tab, for the same reason shots.tape has none of a picture: vhs has
# no graphics protocol, so that pane comes out blank, and a photograph of an empty pane claiming
# to be a plot is worse than no photograph. That one is taken by hand, on a terminal that draws.

Output docs/.shots-numeric-scratch.gif

Set Shell "bash"
Set FontFamily "JetBrainsMono NFM"
Set FontSize 16
Set Width 1600
Set Height 900
Set Padding 12

Hide
Type "export XDG_CONFIG_HOME=$(mktemp -d) && CLEE=$PWD/target/release/clee && SHOTS=$(mktemp -d) && cp examples/celle.m examples/calcola.m $SHOTS/ && cd $SHOTS && clear"
Enter
Sleep 1s
Show

Type "$CLEE -w octave"
Enter
Wait+Screen@60s />>/
Sleep 3s

# ---- workspace.png: the panel that fills in by itself. Two cells are sent, so the table has
#      something worth reading in it — a scalar and three vectors, with their ranges.
Ctrl+o
Sleep 1s
Type "celle.m"
Sleep 1.5s
Enter
Wait+Screen@40s /ampiezza/
Sleep 1s
Ctrl+g
Sleep 1s
Type "4"
Sleep 0.8s
Enter
Sleep 0.8s
Ctrl+p
Sleep 1s
Type "run selection"
Sleep 1.5s
Enter
Sleep 5s
Ctrl+g
Sleep 1s
Type "8"
Sleep 0.8s
Enter
Sleep 0.8s
Ctrl+p
Sleep 1s
Type "run selection"
Sleep 1.5s
Enter
Wait+Screen@60s /misura/
Sleep 2s
Screenshot docs/screenshots/workspace.png

# ---- debug.png: stopped inside a function, with the frame's own variables in the panel rather
#      than the session's. The wait is on Octave's own debug prompt, so this cannot be taken
#      before the session has actually stopped.
Ctrl+o
Sleep 1s
Type "calcola.m"
Sleep 1.5s
Enter
Wait+Screen@30s /b = a . 10/
Sleep 1s
Ctrl+g
Sleep 1s
Type "5"
Sleep 0.8s
Enter
Sleep 0.8s
Ctrl+p
Sleep 1s
Type "breakpoint"
Sleep 1.5s
Enter
Sleep 2s
Ctrl+p
Sleep 1s
Type "focus term"
Sleep 1.5s
Enter
Sleep 2s
Type "x = calcola(5)"
Enter
Wait+Screen@40s /debug>/
Sleep 3s
Screenshot docs/screenshots/debug.png
