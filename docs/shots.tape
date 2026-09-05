# The README's still screenshots, taken by vhs (https://github.com/charmbracelet/vhs):
#
#   cargo build --release && vhs docs/shots.tape
#
# These used to be shot by hand, which meant they quietly went stale — menus gained entries and
# key bindings moved while the pictures did not. Regenerating them is now one command, and it
# produces the same images every time.
#
# They used to run CleeCode inside tmux, for a fixed pane size. That stopped working: keys sent
# through tmux no longer reached the application, so every shot timed out waiting for a state
# that never arrived. vhs sets the size by itself, so tmux was buying nothing and costing the
# whole pipeline — it is gone, and the keys are pressed here rather than by a companion script.
#
# Each shot waits for text that only appears once the state is up (`Wait+Screen`), so a slow
# machine delays a screenshot rather than mis-taking it. The GIF output is a by-product vhs
# insists on; only the .png files are kept.

Output docs/.shots-scratch.gif

Set Shell "bash"
Set FontFamily "JetBrainsMono NFM"
Set FontSize 16
Set Width 1400
Set Height 800
Set Padding 12

# A throwaway config, so whatever layout the author happens to be using cannot leak into the
# pictures — and so taking them cannot overwrite it either.
Hide
Type "export XDG_CONFIG_HOME=$(mktemp -d) && clear"
Enter
Sleep 1s
Show

Type "./target/release/clee ."
Enter
Sleep 4s

# ---- main.png: the whole window doing something. A Python file open and highlighted, and its
#      output sitting in the first terminal. The wait is on the script's own arithmetic, which
#      can only be on screen once the run has finished.
Ctrl+o
Sleep 1s
Type "hello.py"
Sleep 1.5s
Enter
Sleep 2s
Ctrl+p
Sleep 1s
Type "run current"
Sleep 1.5s
Enter
Wait+Screen@40s /= 385/
Sleep 1s
Screenshot docs/screenshots/main.png

# ---- menu.png: the menu bar open on Layout, where the presets live. The initials are
#      underlined only while it is open, which is the only time pressing one does anything.
Ctrl+p
Sleep 1s
Type "open the menu bar"
Sleep 1.5s
Enter
Sleep 1s
Right
Sleep 0.4s
Right
Sleep 0.4s
Right
Sleep 0.4s
Right
Wait+Screen@30s /Triple/
Sleep 1s
Screenshot docs/screenshots/menu.png
Escape
Sleep 1s

# ---- split.png: two independent editors, each with its own tabs and its own Run button.
#
#      The second file is opened *before* the split, so the split has a real one to move across:
#      it hands the other half a tab rather than showing the same file twice, and with only one
#      file open that other half would get an empty buffer — true to the design, but a dull
#      picture of it.
Ctrl+o
Sleep 1s
Type "main.rs"
Sleep 1.5s
Enter
Wait+Screen@30s /mod workspace/
Sleep 1.5s
Ctrl+l
Sleep 2.5s
Screenshot docs/screenshots/split.png

# No shot of a picture or a PDF preview here. vhs has no graphics protocol, so those tabs can
# only fall back to half-blocks, and the picture would show the fallback while claiming to show
# the feature. Worse, a modal closing over an image pane currently leaves its old cells behind —
# the image widget marks them as not-to-be-repainted — so the shot came out with a stale command
# palette drawn across it. Those pictures are taken by hand, on a terminal that can draw them.

# ---- manual.png: the built-in guide, left on Overview, which carries the diagram of the whole
#      window and shows the colouring — chords yellow, headings cyan, rules dimmed.
Ctrl+p
Sleep 1s
Type "manual"
Sleep 1.5s
Enter
Wait+Screen@30s /CleeCode manual/
Sleep 2s
Screenshot docs/screenshots/manual.png
Escape
Sleep 1s

# ---- palette.png: the command palette, which is how anything is reached without a shortcut.
Ctrl+p
Sleep 1s
Type "term"
Wait+Screen@30s /Command palette/
Sleep 1s
Screenshot docs/screenshots/palette.png
Escape
Sleep 1s

# ---- gitpanel.png: the git panel on History — the graph of this repository's own commits,
#      tags and branch heads coloured. Reached through the palette so the shot does not depend
#      on which tab the panel remembers; the wait is on the Graph tab's key line, which only
#      that tab draws.
Ctrl+p
Sleep 1s
Type "history and branches"
Sleep 1.5s
Enter
Wait+Screen@40s /cherry-pick/
Sleep 2s
Screenshot docs/screenshots/gitpanel.png
Escape
Sleep 1s
