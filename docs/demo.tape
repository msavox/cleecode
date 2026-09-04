# The demo for the README and the site's hero, played by vhs (https://github.com/charmbracelet/vhs):
#
#   cargo build --release && vhs docs/demo.tape
#
# Then shrink it, because vhs writes a full-colour GIF and this one lands north of 3 MB:
#
#   ffmpeg -y -i docs/demo.gif -vf palettegen=max_colors=64:stats_mode=diff /tmp/p.png
#   ffmpeg -y -i docs/demo.gif -i /tmp/p.png -lavfi paletteuse=dither=none /tmp/small.gif
#   mv /tmp/small.gif docs/demo.gif
#
# The third line is not decoration. ffmpeg 9 refuses an output file it is also reading —
# "Output docs/demo.gif same as Input #0" — where older ffmpegs let the in-place form through, so
# the second pass writes somewhere else and the file is moved back. `dither=none` is the
# load-bearing half of the recipe: dithering adds noise, noise defeats the GIF's run-length
# compression, and the "optimised" file comes out *larger* than the one it replaced. Sixty-four
# colours is plenty for a terminal, which is flat colour and nothing else — the Turbo beat, the
# one place this film goes loud, survives it without a band anywhere.
#
# ---- Why this cut, and not a tour --------------------------------------------------------------
#
# The recording this replaces walked the app: tree, picture, markdown, split, focus, workspaces,
# menu bar, manual — seventy seconds of somebody else's mouse. It showed more and said less.
# This one is six beats, one feature each, in the order a keynote would put them: what it *is*
# (the splash), what it does for work (runs code, renders a document, reads the repository's
# history), then the two that are simply nice to look at (a whole-screen theme change, and the
# split the film ends composed on). Nothing is demonstrated twice, and every sleep below is the
# length a beat needs to be read — not the length it takes to perform. Fifty-six seconds, all in.
#
# ---- What vhs can and cannot send --------------------------------------------------------------
#
# It used to run CleeCode inside tmux, with the keystrokes in a companion script, because vhs
# cannot send function keys or real Alt chords while `tmux send-keys` can. That stopped working —
# keys sent through tmux no longer reach the application — so tmux is gone and the keys are
# pressed here. What survives the loss is everything reachable through plain Ctrl chords, the
# arrows, and the command palette; that turns out to be the whole demo, because the palette is
# how most people find an action they have not memorised anyway. Ctrl+Tab and every Ctrl+Shift
# chord are beyond vhs too, so the git panel and the theme list are opened from the palette rather
# than from the menu bar a real user would pull down — the same action, reached the way this
# recorder can reach it.
#
# The palette is driven by typing a few words and pressing Enter on the top match, which is what a
# person does with it. It is worth knowing that this makes the tape sensitive to the *labels*:
# rename "Git panel" or "Theme…" in src/i18n.rs and this film quietly opens something else. The
# frames are the check — see below.
#
# A focused shell, and a focused agent, are handed every Ctrl chord deliberately: they need their
# own Ctrl+C and Ctrl+R. So Ctrl+P stops opening the palette the moment focus lands in a terminal,
# and a tape that lets that happen types the rest of itself into zsh. This one never focuses one:
# the Run output is *watched* in the terminal below, never typed into.
#
# ---- What is not here, and why -----------------------------------------------------------------
#
# No picture, no PDF, no typeset page. vhs answers the capability query claiming sixel and then
# draws none of it, so anything CleeCode hands over as pixels records as an empty rectangle —
# which reads as a broken program rather than as a limitation of the recorder. The real image and
# PDF previews are photographed by hand in docs/screenshots/ for exactly that reason.
#
# The markdown beat survives this only because the preview has a second rendering that is made of
# characters, and the tape asks for it: `preview_markdown_text = true`, written into the throwaway
# config off camera. Without that line the take looks right for about a second — the styled text
# appears, then pandoc finishes, the document replaces it, and the pane goes blank for the rest of
# the film. Two takes were lost to that before the pattern was visible. The styled view is not a
# stand-in invented for the recorder: it is what the preview *is* on any machine without pandoc,
# and it is one keypress (`t`) away in the preview's own toolbar.
#
# That setting once exposed a wrong sentence: the status line claimed "rendered as a document"
# over plainly styled text, because the message was built from what the machine *could* do
# rather than from what the tab was asked for. This recording is the reason
# `msg_markdown_preview` now knows the difference — the film caught what no unit test had.
#
# The git beat shows this repository's own history, because the tape is recorded from the
# checkout: real branches, real subjects, no fixture pretending to be a project. It reads
# git and never writes it — no stage, no commit, no checkout is pressed on camera — so recording
# the demo cannot touch the tree it is recorded in.
#
# ---- Sizing and fonts --------------------------------------------------------------------------
#
# 1800x1000 gives 175x46 cells, and the site's <img> carries those numbers, so changing them here
# means changing them there. Measure before you do: a tape that prints `stty size` is two minutes
# of work and settles the question.
#
# ---- Verifying a take --------------------------------------------------------------------------
#
# Deterministic — no model, no network, the same tape gives the same GIF — so a take is checked
# once and trusted after. Check it by looking:
#
#   ffmpeg -i docs/demo.gif -vf fps=1 /tmp/f/%03d.png
#
# and read the frames for the six things. In the take that shipped, at one frame a second, they
# are 002 (the turtle on the splash), 013 (Hello from CleeCode in the terminal), 025 (the guide
# rendered beside its source), 033 (the commit graph in lanes), 041 (Turbo's blue filling the
# screen) and 056 (the two editors side by side). Those numbers move by a second when a sleep
# does; the six pictures are what has to be there.

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
# the recording cannot overwrite it; the one setting this recorder needs (see above); and `clee`
# on PATH pointing at the build under test, so the only command typed on screen is the one anyone
# would type.
Hide
Type "export XDG_CONFIG_HOME=$(mktemp -d) BIN=$(mktemp -d)"
Enter
Type "mkdir -p $XDG_CONFIG_HOME/cleecode"
Enter
Type `echo 'preview_markdown_text = true' > $XDG_CONFIG_HOME/cleecode/settings.toml`
Enter
Type "ln -sf $PWD/target/release/clee $BIN/clee && export PATH=$BIN:$PATH && clear"
Enter
Sleep 1s
Show

# ---- 1. The name, on the splash --------------------------------------------------------- ~4s
# `clee` with no argument opens the directory it was invoked from, which is also the only way to
# see the splash: a path argument skips it. It holds for 1.8s of its own accord, so the sleep
# below is that plus the beat it takes to land on the editor behind it.
Type "clee"
Enter
Sleep 3.5s

# ---- 2. It runs the file you are looking at ---------------------------------------------- ~11s
# Ctrl+O is a fuzzy finder over the tree, so a name is enough — no walking the folders on camera.
# hello.py prints which interpreter actually ran it, which is the honest answer to the question
# people have about a Run button.
Ctrl+o
Sleep 1s
Type "hello.py"
Sleep 1.2s
Enter
Sleep 2s

Ctrl+p
Sleep 0.8s
Type "run current"
Sleep 1.1s
Enter
Sleep 4.5s

# ---- 3. It renders the document beside its source ----------------------------------------- ~12s
# One file, two tabs, one copy of the text: the preview owns none of it and cannot drift from it.
Ctrl+o
Sleep 1s
Type "guida.md"
Sleep 1.2s
Enter
Sleep 2s

Ctrl+p
Sleep 0.8s
Type "run current"
Sleep 1.1s
Enter
Sleep 4s

# Then the source is closed, and the rendered tab goes with it — it owns no text of its own, so
# there is nothing left for it to be a view of. This is not tidying for its own sake: asking for
# a preview splits the editor, and beat 6 is about splitting the editor on purpose. Leaving this
# split standing would have Ctrl+L *closing* a split instead of opening one, which is the same
# key doing the opposite thing in front of somebody seeing it for the first time.
Ctrl+w
Sleep 1.5s

# ---- 4. It knows what the repository has been doing ---------------------------------------- ~8s
# The panel opens on Status; Tab twice reaches Graph, which is the tab worth two seconds of
# anyone's attention — every branch at once, drawn in lanes, with the commit you are standing on
# among them. Escape closes it and gives the keyboard back to the editor.
Ctrl+p
Sleep 0.8s
Type "git panel"
Sleep 1.1s
Enter
Sleep 1.5s
Tab
Sleep 0.6s
Tab
Sleep 2.6s
Escape
Sleep 0.8s

# ---- 5. The whole screen changes colour ---------------------------------------------------- ~12s
# The list opens on the theme in use, which in a fresh config is CleeCode, one row below Auto.
# One Down lands on Turbo — the six bright EGA colours on that blue — and Enter repaints
# everything at once: chrome, syntax, terminal, tabs. Then back, because the film ends in the
# colours the README is about.
Ctrl+p
Sleep 0.8s
Type "theme"
Sleep 1.2s
Enter
Sleep 1.2s
Down
Sleep 1s
Enter
Sleep 2.5s

Ctrl+p
Sleep 0.8s
Type "theme"
Sleep 1.2s
Enter
Sleep 1s
Up
Sleep 0.8s
Enter
Sleep 1.5s

# ---- 6. Two editors, and the layout the film ends on --------------------------------------- ~10s
# Ctrl+L splits the editor into two independent halves, each with its own tabs and its own Run.
# A Rust file on the right, and the last frame is the whole thing composed: tree, two editors,
# the terminal underneath. The GIF loops, so this is also the first thing anyone sees.
#
# Ctrl+Alt+Right is the step between, and it is not optional: the new half opens empty and *not*
# focused, so opening a file straight after the split puts it back in the half it came from and
# leaves the right one blank. Ctrl with an arrow moves the focus one frame over — Ctrl+Alt
# because macOS takes plain Ctrl+arrow for Mission Control before any terminal sees it, and
# because it is one of the few frame-moving chords vhs can actually send.
Ctrl+l
Sleep 1.6s
Ctrl+Alt+Right
Sleep 1s
Ctrl+o
Sleep 0.8s
Type "main.rs"
Sleep 1.1s
Enter
Sleep 4s
