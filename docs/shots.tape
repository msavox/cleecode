# The README's still screenshots, taken by vhs (https://github.com/charmbracelet/vhs):
#
#   cargo build --release && vhs docs/shots.tape
#
# Also needs tmux. These used to be shot by hand, which meant they quietly went stale — menus
# gained entries and key bindings moved while the pictures did not. Regenerating them is now the
# same one command as the demo, and it produces the same images every time.
#
# The GIF output is a by-product vhs insists on; only the .png files are kept. Keys come from
# docs/shots-keys.sh, for the reasons given at the top of docs/demo.tape.

Output docs/.shots-scratch.gif

Set Shell "bash"
Set FontFamily "JetBrainsMono NFM"
Set FontSize 16
Set Width 1400
Set Height 800
Set Padding 12

Hide
Type "tmux kill-server 2>/dev/null; export CLEE_SHOT_CONF=$(mktemp -d); clear"
Enter
Sleep 1s
Show

Type "tmux new-session -d -s shots -x 172 -y 48 -e XDG_CONFIG_HOME=$CLEE_SHOT_CONF './target/release/clee .' \; set -g status off && bash docs/shots-keys.sh shots & sleep 1; tmux attach -t shots"
Enter

# Each shot waits for text that only shows once the state is up, so the timing of the key
# script and the timing of the camera never have to agree.
# The computed total, which only exists once the program has actually run — the source line
# that prints it says "{total}", so waiting on the wording would match the editor and fire early.
Wait+Screen@40s /= 385/
Sleep 1s
Screenshot docs/screenshots/main.png

Wait+Screen@30s /Triple/
Sleep 1s
Screenshot docs/screenshots/menu.png

Wait+Screen@30s /mod workspace/
Sleep 1s
Screenshot docs/screenshots/split.png

Wait+Screen@30s /CleeCode manual/
Sleep 2s
Screenshot docs/screenshots/manual.png
Sleep 1s

Wait+Screen@30s /Command palette/
Sleep 1s
Screenshot docs/screenshots/palette.png
