# Demo recording for the README, played by vhs (https://github.com/charmbracelet/vhs):
#
#   cargo build --release && vhs docs/demo.tape
#
# Deterministic: the same tape always produces the same GIF, so the demo can be re-recorded
# after a UI change rather than re-shot by hand.
#
# Three constraints shape the script, all of them vhs limits rather than choices:
#  - it runs ./target/release/clee, not whatever `clee` is on PATH, because recording against an
#    installed older release quietly demos features that no longer match the code;
#  - no function keys and no mouse, so the menu bar (F9), focus switching (F1/F2/F3) and the Run
#    button (F10) are unreachable. Everything goes through the command palette instead, which
#    lists the same actions. The drop-down menus are shown by docs/screenshots/menu.png;
#  - Alt+<letter> mnemonics are not usable either: vhs sends them as ESC then the letter, far
#    enough apart that they arrive as two events, so Alt+r typed an "r" into the buffer.
#
# Because focus cannot be moved back to the sidebar, the tree browsing has to come first, while
# focus is still there.

Output docs/demo.gif

Set Shell "bash"
# The font's own family name. The file is named for this project but its name table still says
# JetBrainsMono NFM, and a family vhs can't resolve silently falls back to a serif face whose
# advance widths don't line up with the terminal grid.
Set FontFamily "JetBrainsMono NFM"
Set FontSize 16
Set Width 1400
Set Height 800
Set Padding 12
Set Framerate 24

Type "./target/release/clee ."
Enter
Sleep 4s

# The splash gives way to the workspace.
Escape
Sleep 800ms

# Sidebar browsing: move down the tree, expand a folder with Right, collapse it with Left.
Down
Sleep 500ms
Down
Sleep 500ms
Right
Sleep 1500ms
Left
Sleep 800ms
Down
Sleep 500ms
Right
Sleep 1500ms
Down
Sleep 400ms
Down
Sleep 600ms
# Enter on a file opens it in a tab.
Enter
Sleep 2s

# Run it. The command goes to the first idle embedded terminal, and the venv selector decides
# which interpreter that is.
Ctrl+p
Sleep 800ms
Type "run current"
Sleep 1200ms
Enter
Sleep 3500ms

# The same for an Octave script. `octave --persist` leaves the interpreter running afterwards,
# so the session (and any plot window) survives the script.
Ctrl+o
Sleep 700ms
Type "hello.m"
Sleep 1000ms
Enter
Sleep 1500ms
Ctrl+p
Sleep 700ms
Type "run current"
Sleep 1000ms
Enter
Sleep 6s

# Workspace layout presets, all three of them.
Ctrl+p
Sleep 700ms
Type "wide"
Sleep 1000ms
Enter
Sleep 2500ms
Ctrl+p
Sleep 700ms
Type "triple"
Sleep 1000ms
Enter
Sleep 2500ms
Ctrl+p
Sleep 700ms
Type "classic"
Sleep 1000ms
Enter
Sleep 2000ms

# The venv drop-down, hanging under its toolbar button.
Ctrl+p
Sleep 700ms
Type "venv"
Sleep 1000ms
Enter
Sleep 1500ms
Down
Sleep 1200ms
Escape
Sleep 600ms

# Quick open, first as a fuzzy search over the project, then as a filesystem browser: a query
# starting with a path lists that directory instead.
Ctrl+o
Sleep 700ms
Type "settings"
Sleep 1200ms
Escape
Sleep 500ms
Ctrl+o
Sleep 600ms
Type "~/"
Sleep 2s
Escape
Sleep 600ms

# Tabbed terminals: Ctrl+T opens tabs inside the focused terminal window. They ride its top
# border, in green, so they never read as editor tabs.
Ctrl+t
Sleep 1500ms
Ctrl+t
Sleep 2s

# Two independent editor panes, each with its own tab strip. The recording ends here rather than
# quitting: a looping GIF should not spend its last seconds on an empty shell.
Ctrl+l
Sleep 3s
