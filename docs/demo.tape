# Demo recording for the README, played by vhs (https://github.com/charmbracelet/vhs):
#
#   cargo build --release && vhs docs/demo.tape
#
# Also needs tmux. Deterministic: the same tape produces the same GIF, so the demo can be
# re-recorded after a UI change rather than re-shot by hand.
#
# On camera the only thing typed is `clee`, with no argument — which is how anyone actually
# starts it, and what makes the splash screen appear, since a path argument skips it. Everything
# else is set up off camera:
#
#  - CleeCode runs inside tmux, because vhs cannot send function keys or real Alt chords while
#    `tmux send-keys` can; the keystrokes live in docs/demo-keys.sh.
#  - `clee` on PATH is a wrapper that starts that tmux session and the key script, so the tmux
#    invocation never appears on screen. The real binary is beside it as `clee-real`.
#  - the config directory is a throwaway: the recording saves a workspace, and would otherwise
#    leave it, and the layout it ends on, in the real ~/.config/cleecode.

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

Hide
Type "tmux kill-server 2>/dev/null; export CLEE_DEMO_REPO=$PWD CLEE_DEMO_CONF=$(mktemp -d) BIN=$(mktemp -d)"
Enter
Type "export CLEE_DEMO_KEYS=$CLEE_DEMO_REPO/docs/demo-keys.sh"
Enter
Type "echo 'set -g status off' > $CLEE_DEMO_CONF/tmux.conf"
Enter
Type "cp target/release/clee $BIN/clee-real; cp docs/demo-launch.sh $BIN/clee; chmod +x $BIN/clee"
Enter
Type "export PATH=$BIN:$PATH; clear"
Enter
Sleep 1s
Show

Type "clee"
Enter

# Covers docs/demo-keys.sh end to end — keep the two in step.
Sleep 88s
