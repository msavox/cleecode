## What's new in 0.23.0

**A safety fix worth the release on its own.** Clicking an agent that is not installed used to
type its install command *and run it* — for two of the four that meant a script downloaded from
the web and piped straight into a shell, executed because a mouse landed on a row. Now the
command is only placed at the prompt; the Enter that runs it is yours, as it always should have
been. The same discipline the rest of the drawer already followed.

**Zoomed pictures pan every way.** A picture zoomed past the pane now moves sideways on the
left and right arrows, not only up and down — and its body can be grabbed and dragged with the
mouse, the way every image viewer moves one.

**The close box is easier to see, and calmer.** The little square that closes a tab or a pane
is filled now instead of a hollow outline that all but vanished on the focused tab, and it is
drawn in the accent colour rather than a red that read as "this will destroy something" on a
control that only ever asks first.

**Steadier on a bare shell.** On a shell without readline — `/bin/sh` is one on many Linux
boxes — a command the editor typed for you could arrive mangled, a stray form feed dropped into
the middle of it. The editor now waits for the prompt and holds that form feed back, so what it
types lands whole, on that kind of shell and every other.

**A Debian package.** `clee` now ships a `.deb` alongside the archives, for `apt install
./clee.deb` on Debian and Ubuntu — the binary, its man page and a desktop entry, with its
dependencies read off the binary itself.

Under the hood, the interactive test drivers are now a required check on every push, on macOS
and Linux both, rather than an advisory one — the interface they exercise is held to the same
bar as the rest of the build.

## What's new in 0.22.0

Four releases in one — the roadmap's 0.19 through 0.22, landed together.

**The agent can point, and asks before it writes.** `clee --mcp` grows from four read-only
tools to seven. `open_file` takes a line range the editor highlights, so "look here" is a
gesture, not a sentence; `preview` renders a file beside your work — Markdown as a document,
pictures and PDFs as pages; `say` puts one line in the status bar, so the agent can narrate
without owning a pane. `open_files` now flags buffers with unsaved edits, and for exactly
those there is `edit_buffer`: the agent proposes a change and the editor asks you — once, for
the whole session, or no — before one character moves. The edit lands as a single undo, in
the buffer, never on disk: saving stays yours. An agent started from the drawer gets all of
it with zero configuration — claude, codex, opencode and gemini each registered by whatever
mechanism that CLI has, in session files that vanish with the editor, never in your own
config. And the lines any outside program writes into your open files now carry a quiet tint
until you take the keyboard back, so what changed while you looked away is findable when you
return.

**Code actions.** The diagnostic under the cursor can fix itself: "add the import", "apply
the compiler's suggestion", offered in a picker and applied with the discipline rename and
format already follow — a diff first where more than one file is touched, one undo step
where one buffer is. rust-analyzer's lazily-resolved actions included.

**Structural selection, and a new keyboard layer.** Widen the selection outward — identifier,
expression, function — and narrow it back, one `textDocument/selectionRange` rung per press.
It lives on `Ctrl+Cmd+↑/↓` (`Ctrl+Super` off macOS): a whole new chord layer, delivered on
terminals speaking the kitty keyboard protocol — Ghostty, kitty, WezTerm, iTerm2, foot — and
absent elsewhere, where the actions stay in the menu and `[keys]` rebinds them. Code folding
now takes its boundaries from the language server where one is attached, falling back to the
brace-and-indent heuristic on dirty buffers and everywhere else.

**A debugger for compiled programs.** C, C++, Rust — anything built with symbols. *Debug ▸
Start debugging* asks what to run with the guess prefilled (Cargo projects guess their own
binary), and runs it under `lldb-dap` or a gdb 14+, found on their own or named in
settings.toml. One set of breakpoints — the same `Ctrl+Shift+P`, the same gutter, any file
now — and stopping marks the line and opens a panel: the stack, the frame's variables, your
watch expressions, the debuggee's output. With the panel focused, `c`, `n`, `s`, `o` and `x`
do what gdb taught your fingers; Continue, Pause, the steps and Stop live in the new Debug
menu too. The whole flow is driven end-to-end in CI against a real adapter — a run that found
and fixed a real bug before release: breakpoints in projects reached through a symlink (every
`/tmp` on a Mac) were accepted, drawn, and never hit.

**The close box wears System 7.** Every ✕ became `□`, moved to the top-left with a cell of
air on each side — `┌ □ ─ Terminal 1` — on windows, tabs and terminal chips alike.

**The turtle is the icon's.** The About drawing and the splash now carry the same top-view
turtle as the app icon, resampled onto the half-block grid in the icon's own greens.

**A chosen theme owns its background.** Every theme now paints its own surface, so switching
themes on a translucent terminal no longer leaves the new theme sitting on the old ground.
Transparency became the explicit choice: *View ▸ Transparent background* (the `●`/`◐` button)
hands the background back to the terminal, and picking a theme takes it whole again.

**Windows installs with two lines.** `scoop bucket add clee
https://github.com/msavox/scoop-clee`, then `scoop install clee` — the Windows twin of the
Homebrew tap.

Also in this release: the status line no longer claims a Markdown document while showing
styled text by request; the interpreter debug keeps its breakpoints to the files it can run
now that the gutter is shared; and both demo recordings on the site and README were reshot —
the six-beat feature reel, and the agent drawer with a real claude editing a real buffer.
## What's new in 0.18.0

**The agent drawer.** `Ctrl+Shift+A` opens a panel down the right of the window, in whatever
workspace you were already in. Empty, it is the launcher: the four agents drawn as their own
marks in half-block pixels — mascot and name side by side — a frame around the one Enter would
start, and the ones you do not have shown dimmed with the honest phrase under them. Choosing a
missing one types its documented install command at a shell prompt and stops there: the line is
never run, Enter stays yours. The drawer has two modes — pin, a column of the layout, and
autocollapse, painted over the frames and withdrawing when the keyboard goes back to your work —
and closing it never kills the agent: the pty runs on, the conversation is where you left it.
Handles on the window's edge open and close it with the mouse, striped in six colours each theme
signs its own way.

**gemini is the fourth agent.** Same drawer, same chord, same rules as claude, codex and
opencode — including being found when npm dresses it up as `node`.

**The agent presets are retiring, and say so.** With the drawer there is nothing left for
`clee -w claude` and friends to do that a plain workspace plus one keypress does not do better.
This release they still open something sensible and the status line says they are deprecated and
what replaces them; the next one removes them. A `minimal` preset — one editor, one shell —
arrives in their place.

**Rename, references, format: the language server's second half.** Rename the symbol under the
cursor with a grouped preview and a single undo; list references and document symbols; format
the whole file with `Ctrl+Shift+Q` as one edit. After a `.` the completion popup now asks the
server — what follows a dot is what that type has, which the buffer's own words cannot know —
and opens only when the answer arrives, never empty.

**Search learned to write.** `Ctrl+Shift+H` grows a second field: empty, it is the search it
always was; filled, Enter opens a preview grouped by file — the rename discipline applied to
project-wide replace. The field empties on every opening, so a forgotten leftover can never
silently turn the next search into a replace.

**Autosave and recovery.** Every five seconds, dirty buffers are copied to
`~/.config/cleecode/recovery` — unnamed ones included, which used to die without a trace. A
SIGKILL, a stack overflow or the power going out now costs you at most five seconds of typing.

**A block selection types.** With a column block active, every key inserts on every line of the
block at the same column, Backspace eats one column back, and each keystroke is a single undo
step. Lines shorter than the column are skipped, not padded.

**A file over fifty megabytes is told the truth.** Above the documented threshold the editor
declares its limits — no highlighting, no buffer words in completion, a shorter undo — instead
of discovering them by freezing.

**The status bar says how the file is built.** Encoding and line endings are read out loud, and
a command converts them.

**Animated GIFs animate.** In the preview tab, at the pace the file itself asks for.

**A new coat of paint.** CleeCode has an icon — in the Dock via `clee --install-app`, in the
browser tab, and beside the name on its new home page: <https://cleecode.pages.dev>.

## What's new in 0.15.1

**Ctrl+Shift+A now finds the `claude` most people actually have.** Installed from npm,
`claude` is a script run by `node`, so the process table says `node` and a search by name came
up empty — an agent sitting at its prompt two panes away, and the key said there was none. A
pane opened by the preset was covered by its startup command; a `claude` typed by hand into an
ordinary shell was covered by nothing. Now each process is read by name *and* by the arguments
it was started with: where the name is an interpreter's, the script it was handed answers, and
the npm-installed agent is found like any other.

**"Send where you are to the agent" is in the Run menu.** The chord was the only way to it,
and a feature reachable only by a shortcut you were told about once is a feature most people
never learn they have. The menu row is also what `Ctrl+P` finds.

**Running a script again lands in the same figure tabs again.** Run closes the figures the
file's previous run opened, so an unnumbered `figure()` redraws into the tab that is already
there instead of opening a new one — but the watch that remembers what a run drew was closing
half a second after the prompt returned, and the session publishes its figures up to two
seconds later. Every rerun recorded nothing, closed nothing, and stacked fresh tabs. The watch
now waits for the session to actually say what the run drew, with a cap for sessions that
never say anything.

## What's new in 0.15.0

**Your coding agent, inside the editor.** `clee -w claude`, `clee -w opencode` and
`clee -w codex` open the editor with the agent running in one terminal tab and a plain shell
in the other. The agent is the TUI you already have, with the subscription you already pay
for — CleeCode holds no API key and rebuilds no chat; it is the place where the agent runs
best, with the editor, the clickable compiler output and the Git panel around it.

**Ctrl+Shift+A tells the agent where you are.** The selection, the diagnostic under the
cursor, or just the cursor's line, written to the agent's prompt as `path:line` — and never
submitted: Enter stays yours. The way back has always been there: agents print `path:line`
constantly, and a double-click opens it.

**The agent's edits are live.** A file it rewrites reloads on its own and the new lines light
up in the gutter until you type again or press Esc. Follow mode — off by default — opens the
files it touches beside your work, without ever taking the keyboard; it needs a git
repository and says so when there is none. A buffer with your unsaved edits never reloads
itself: your work wins over the agent's, always.

**`clee --mcp` makes the editor an MCP server.** Four tools tell the agent what only the
editor knows — the open files, the active one, the selection, the diagnostics — and
`open_file` shows a file beside your work without touching your keyboard. One implementation,
three agents: the wiring is one line in each agent's config, spelled out in the manual.

Along for the ride, the 0.14 tail: `theme = "auto"` asks the terminal for its background
colour (OSC 11, with a timeout) and picks the dark or light house theme; keybindings can be
remapped one at a time from a `[keys]` table in settings.toml, seeded as a commented catalogue
by the new menu entry; and the macOS app's window now closes when the editor quits, whatever
the Ghostty config says (rebuild the launcher with `clee --install-app` to pick it up).

## What's new in 0.14.0

**Nine themes, and one place the colours come from.** Five dark, four light. Six are built on
the values syntect already carried in the binary — it shipped seven themes and CleeCode used
one — and Turbo is the blue screen, in the set because it stresses the palette like no other
dark theme would. The theme picker sits beside the background button, and in View → Theme for
whoever has their hands on the keyboard.

A settings.toml with a single key now loads as that key over the defaults instead of being
set aside as broken: the first release's fields lacked `#[serde(default)]`, so a hand-written
partial file lost everything else — and the file is rewritten on exit, so the defaults papered
over the evidence before you could see it.

## What's new in 0.13.3

**A URL in terminal output opens on a double-click.** The same double-click that already takes
a `path:line` to the editor now takes a `https://…` to the browser — the hand-off *Open outside
CleeCode* uses, refused over ssh for the same reason, where the desktop it would open on
belongs to whoever is sitting at the far machine.

The address it takes is the one that was printed. It stops where a URL cannot go on, so
`<https://…>` and `href="https://…"` give up their markup and an ANSI escape after an address
is not swallowed into it; a closing bracket goes only when nothing inside the URL opened it, so
`…/wiki/Rust_(programming_language)` and `http://[::1]:8080` keep theirs while `(see https://…)`
gives its up; and `…/wiki/Perù` stays as printed. A row naming a file that is not here *and* a
URL opens the URL rather than complaining about the file.

Only `http` and `https`: `file:` and `javascript:` are handlers nobody asked a double-click to
reach, and are refused at the door to the desktop rather than trusted to the caller. A row of
terminal output is whatever a build log, a `cat` or a `curl` printed, so on Windows the URL goes
to `explorer` and never to `cmd /C start` — a shell would read the `&` of a query string as the
end of one command and the start of another.

First outside contribution: [#1](https://github.com/msavox/cleecode/pull/1) by
[@rikhza](https://github.com/rikhza).

## What's new in 0.13.2

**Markdown files get a formatting bar.** One row above the text, only when the buffer is an
editable Markdown file: bold, italic, strikethrough, inline code, a heading cycle, the three
kinds of list, link, quote and code fence. The buttons are semantic toggles, not blind
insertion — bold on a bold word makes it plain again, the selection survives the press, and
every action is a single undo step. Each button shows the syntax it writes beside its label,
so the bar teaches the thing it is for; once `**` is in your fingers, *View → Formatting bar*
switches it off, and the choice is remembered across sessions.

The same eleven actions sit in a new *Format* menu — offered whatever file is open, so they
can be found, and saying so in the status bar on a file they are not for rather than doing
nothing silently — and, through the menu, in the command palette. No new keyboard shortcuts:
five comfortable chords remain free in the whole application, and eleven actions would not
fit in them.

## What's new in 0.13.1

**Quit gives the terminal back.** Reported minutes after 0.13.0: quitting with an interactive
fish in a pane froze the editor. Two causes, one on top of the other. portable-pty's `kill`
only sends SIGHUP on unix — `/bin/sh` dies of that, which is why every lifecycle test passed,
and an interactive fish ignores it — so now the hangup is the offer and SIGKILL the deadline,
six tenths of a second later. And the pty reader thread exited on the stop flag instead of
draining: the kernel does not let a process finish exiting while output sits unread in its
pty, and fish repaints its prompt as it dies. The flag now silences the reader instead of
stopping it — bytes drain and are dropped until EOF, like a real terminal reading to the
hangup.

## What's new in 0.13.0

**The review debt is paid.** Six waves of fixes out of a line-by-line review of the whole
codebase: the four paths that could lose unsaved changes, atomic writes everywhere a file is
written, a terminal Drop that really ends the shell, the stack overflow in the directory
walk, incremental highlighting so the cost of a keystroke stops growing with the file, an
event loop that redraws only when something changed, LSP positions in the units the protocol
means, a language server that restarts when it dies, a Git panel that works on a repository
born a minute ago, bracketed paste and mouse reporting inside the panes, and paste in every
input box. The pty drivers now run in CI against the real binary, and a release checks that
the tag and `Cargo.toml` agree before publishing binaries that would say the wrong version.

## What's new in 0.12.5

**The Octave frame speedup from 0.12.3 is reverted.** It wrote gnuplot's own chatter into the
transcript of a session over ssh — `multiplot> set style increment default; line 0: warning:
deprecated command`, once per frame of an animation — and, on that machine, a figure window
where there should have been a tab. `drawnow (TERM, FILE)` draws through the figure's *live*
gnuplot stream, whose stderr is the shell's and whose terminal is whatever the display offers;
`print` runs a gnuplot of its own and says nothing. On the machine it was developed on that
stream never opened at all, which is precisely why every measurement looked clean.

Frames are back to where they were before 0.12.3 — slower, quiet, and windowless. The route is
not wrong in principle and the note in `cleecode_figs.m` records what it cost and what would have
to be proved before it comes back: a Linux box with a display forwarded over ssh, which is the
case that broke.

## What's new in 0.12.4

**Where plots go now takes effect without restarting CleeCode.** Reported as "the switch is
ignored unless I restart clee", and it was half true: a shell opened *after* the toggle did get
the new answer — but a shell that was already open could not, because a process's environment is
a copy taken when it started and nothing outside can change it. So the `octave` or `python3` you
typed next, at the prompt you had been working in all day, went on doing what the old setting
said. CleeCode now also writes the answer to a file both hooks read *as the interpreter starts*,
so the next one honours the switch wherever it is started from. A session already running keeps
what it was born with, which is not a policy but a fact about processes — and the status line now
says "from the next Octave or Python you start" rather than the vaguer "next session".

**A figure's buttons say when they cannot work.** The six on the left of a figure's bar — pan,
rotate, reset, save — do not touch the picture: they ask the session that drew it to redraw. With
that session gone, which is the normal state of a figure that came from `▶ Run`, they could do
nothing, and they were drawn exactly like the buttons that still worked. Their refusal was a line
in the status bar, the easiest thing on screen to miss. They are now dimmed when there is no
interpreter behind the figure.

**And the bar is easier to hit.** No button is narrower than five cells — `+` was three, which is
a target the size of a full stop — and the single column of space between two buttons now belongs
to one of them instead of to neither, so a click a column wide of the mark still lands. Nothing
moved on screen; the targets grew.

## What's new in 0.12.3

**Octave animations are four to five times faster.** Reported from a session over ssh, where
Python animated smoothly and Octave crawled — and it was not ssh, and not the toolkit. It was
`print`: measured at 155 ms a frame for a 500-point line at 800x600, and 150 ms at 400x300, and
159 ms at half the resolution. The cost is not in the pixels, it is print's own machinery, which
copies the figure into a hidden one and takes the toolkit right round again every frame.
matplotlib's `savefig` does the same job in 11 ms.

Under gnuplot — which is what a session with no display gets, so every ssh session — there is a
way round it: `drawnow (TERM, FILE)` hands the figure to the toolkit's png terminal with none of
that in between. Through the real hook, on the same machine and the same figures:

| | before | now |
|---|---|---|
| animated line, 800x600 | 243 ms/frame | 55 ms/frame |
| 3-D surface with a colorbar, 640x480 | 345 ms | 74 ms |
| image, 500x400 | 233 ms | 47 ms |

The picture is the same picture: the grid is still drawn by CleeCode as real line objects, so it
survives the change of route. Only under gnuplot, and only when what lands on disk is the PNG
that was asked for — under qt the terminal argument is ignored and the file written is PostScript
with a `.png` name, so the magic bytes and the width and height are read back before the file is
accepted. Either check failing is a `print`, exactly as before.

## What's new in 0.12.2

**A markdown file with a picture in it becomes a document again.** Reported as "the README of
CleeCode only previews as text, the small notes render fine", and the difference between the two
was a picture. pandoc extracts a document's images into a temporary directory and hands typst
absolute paths to them — and typst resolves an absolute path against its *root*, not against the
filesystem, so it looked for `/private/var/…/media/docs/demo.gif` under the working directory and
reported it missing. The engine is now told where its root is. Every markdown file with an image
was affected, and the fallback to styled text is silent by design, which is why it read as
"the graphical preview stopped working".

**And when a preview does fail, the status line says why.** It showed pandoc's last line —
"Error producing PDF." — which is true and useless. The engine's own report is above it: typst
opens with `error:`, TeX with `!`, and either now reaches the status line.

**Open outside CleeCode.** New first entry in the file tree's right-click menu (and in the
command palette): it hands the file to whatever the desktop opens that kind with — a PDF to a
reader, a `.md` to a browser, anything CleeCode can only show. `open` on macOS, `xdg-open` on
Linux, `start` on Windows. Over ssh it refuses and says so, rather than opening a window on the
machine at the other end.

## What's new in 0.12.1

**Three settings were drawn off the bottom of the box.** The settings modal is sized from one
number and the cursor wraps on it, and that number had said nine since the first commit while the
list grew to twelve. *Where plots open*, *Mouse enabled* and *Language* were below the fold:
invisible, unreachable by the arrows, and not clickable either — which is why the only way to
change the plot destination was the Run menu or the file on disk. The box now takes its size from
the rows it has, a test fails if the two ever disagree again, and each row's value is set against
the right-hand edge instead of a fixed column that the language-server label had outgrown.

**A change made in the settings modal now takes effect, and survives.** Picking a row wrote the
struct and nothing else: the plot destination also lives in an atomic the shells are started
from, so the row nobody could reach would not have worked if they had. Nothing was written to
disk until a clean quit either, so a change made there and a terminal closed by its own button
cancelled each other out. Both are fixed for every row at once.

**The plot switch says which way it is set.** *Plots: tabs or windows* in the Run menu asked the
question and answered neither — you had to flip it and read the status line, which is a question
you cannot ask without also answering it. It now reads out `tabs` or `windows` on the right, in
the column the shortcuts use, and it reads out the *effective* destination: on a machine with no
screen it says `tabs`, because that is where the figures are going whatever the setting says.

**The splash screen is a setting.** An argument on the command line has always skipped it, so
`clee src/main.rs` went straight to work while a bare `clee` showed the turtle for one and
eight tenths of a second. *Splash screen at startup*, off, skips it for the bare `clee` too.

## What's new in 0.12.0

**Animations do not flicker any more, and it was four separate things.** The pane emptied
between frames, because re-reading a figure put the tab back into its loading state while the
new picture was decoded on a thread — a picture, a blank, a picture, ten times a second. Every
frame was also a *new image* to the terminal: a fresh kitty protocol per frame means a fresh
image id, the id is written into the cells as a colour, so every cell changed and the terminal
was told to drop one image and place another over the whole area. Nothing held the screen still
while a frame was written, which synchronised update (DEC 2026) now does — the vertical retrace
a TUI never had. And the picture was being read while it was still being written: both hooks
print beside the name and rename onto it now, which within a directory is atomic.

**Octave animations move.** A figure tab was re-read only when the *snapshot* changed, and the
snapshot is written by a hook that runs while the interpreter waits for a command — a loop never
waits. `cleecode_frame` reprints the figures without rebuilding a snapshot, so an Octave
animation moved only when the loop happened to let the hook in. The picture's own timestamp is
what says a figure was redrawn.

**A figure is the same figure next time.** Running a script again used to leave a second set of
tabs beside the first, because both languages hand out the next free number and neither
`plt.subplots()` nor a bare `figure()` names one. Run now closes the figures that file's previous
run opened, and only those — a plot made by hand at the prompt survives a rerun. Octave and
matplotlib also stopped writing `fig1.png` over each other: one directory per language.

**Plots reach a tab without the presets.** `python3 plot.py` — what Run does when no prompt is
open — installed no hook, because PYTHONSTARTUP is read only by an interactive interpreter, while
matplotlib was already pointed at CleeCode's windowless backend. The plot existed nowhere at all.
It works from any Python CleeCode starts now, session or script, which is what Octave has done
since its hook moved onto the load path.

**Smaller things.** The three built-in workspaces are coloured in the chooser, so it is visible
that they cannot be deleted rather than only true. The first menu is called CleeCode again under
every workspace. `examples/plot.m` is `examples/grafico.m`: Octave looks for a function among the
files of the directory it is working in before its own library, so shipping a `plot.m` renamed
Octave's `plot` for every example beside it. And Run with no tab open says there is nothing to
run instead of hitting the panic shield.

## What's new in 0.10.2

**`grid minor` draws again on a machine with no display.** A plot made over ssh came back
without a grid — not a faint one, none at all — while the same script on a desktop drew it.
The difference is not the operating system, it is the toolkit: with no display to open a
window on, CleeCode starts Octave on gnuplot, and Octave's gnuplot backend states its axis
ticks as an explicit list, which is precisely the case where gnuplot will not place minor
ticks between them. Nothing settable from Octave could have changed that. CleeCode now draws
those grid lines itself at the positions Octave would have used, prints, and removes them
again — under gnuplot only, and leaving the session's figure exactly as it found it.

## What's new in 0.10.1

Three bugs, all of them silent, all of them found in a real session rather than in a test.

**`clear` did nothing in a terminal pane, and neither did the startup-banner scrub.** A pane
inherited `TERM` from whatever terminal CleeCode was displayed in — and the pane is not that
terminal, it is CleeCode's own parser. Reached over ssh from Ghostty, an Ubuntu box was told it
was an `xterm-ghostty`, an entry it has never heard of, so everything that goes through terminfo
stopped working inside the panes. They are told `xterm-256color` now, which is what the parser
actually implements, plus `COLORTERM=truecolor` for the 24-bit colour it does carry.

**Figures could lose their titles and axis labels.** Not in the plot — the picture on disk was
always right. CleeCode asks the terminal at startup whether it can draw real pixels, and that
question reads the terminal's reply off stdin. It was asked *after* mouse reporting was switched
on, so a hand resting on the trackpad during startup could bury the answer under mouse events;
CleeCode then fell back to half-blocks, where a pane twenty rows tall is forty pixels of vertical
resolution and every label on a plot disappears. The question is asked first now.

**A session over `ssh -X` could die outright, leaving an unusable terminal.** CleeCode opened a
system clipboard at startup, always; on Linux that holds an X11 connection for the whole session.
Under `ssh -X` the display is forwarded, and forwarding expires — after twenty minutes, by
default. When it goes, libxcb ends the process without unwinding, so none of the terminal
teardown runs. Over ssh that clipboard was never usable anyway (it is the *server's*, which is
why copying already went out through OSC 52), so it is no longer opened there.

## What's new in 0.10.0

**The completion popup gains its second source.** Where a language server is installed, its
suggestions drop into the list already on screen — in magenta, ranked by the server's own
judgement of what belongs at that position. After a dot those are the only names that mean
anything, and no amount of reading the file could have found them. The list never waits for
them: it opens on the words in your buffers and gets better a moment later, and a row you have
already arrowed down to stays where it is. The `diagnostics` setting is now `language_server` —
it governs the underlines and the list both — and the old spelling is still read.

**The git panel writes.** A new Status tab lists every changed file with git's own two letters in
front of it, the index's and the working tree's. `S` stages the file under the cursor, `U` takes
it back out, `A` stages everything, `C` asks for a message and commits, `Enter` opens the file;
on Branches, `Enter` moves to one. `X` throws away a file's changes behind a question that takes
one letter and reads every other key as no — it is the only thing here that destroys work.
Everything goes through `git` on PATH, so hooks run, commits are signed and credential helpers
are asked, exactly as in the terminal beside it. Push and pull are deliberately absent: they can
stop to ask for a password, and a panel has no terminal to ask it in.

**The last tab can be closed.** It used to be replaced by an identical untitled buffer, which
made it the one tab you could not close. What is left is an empty frame that says how to open
something.

**Figures redraw again on a desktop.** Zoom, pan and reset sent their command and the session
really did move the plot, but the picture never changed: Octave only marks a figure modified
under gnuplot, and every machine with a display uses qt. Anyone not on a headless box had a plot
they could not move. Its Python counterpart had the mirror of the same fault — matplotlib's
staleness flag was never cleared, so every figure was re-rendered at every prompt.

**A Python that calls itself Python.** Homebrew's Python on macOS is a framework build and the
process is named with a capital P, so CleeCode did not recognise it as a session: sending a cell
typed a shell command at the Python prompt instead. Every brew `python3` on macOS was affected.

## What's new in 0.9.2

**Plots can go back to their own windows.** Capture was the only way plotting worked: every
Octave and matplotlib session was told to open no window and hand over a picture. That is still
the default, and still the right one — a live Qt window cannot be moved into a terminal, so
without it the figure appears behind the terminal, which is the worst of both. But on a desktop a
real figure window zooms, pans and rotates with the toolkit's own tools, and wanting that was not
something you could ask for. *Run ▸ Plots as tabs* turns capture off, and the settings panel
carries the same row: off, Octave keeps its qt windows and matplotlib its usual backend, exactly
as they behave outside CleeCode. It applies to sessions started afterwards, because an
interpreter picks its backend once, at startup.

Where there is no screen, the choice is not offered. Over ssh without X forwarding, or on a Linux
box with no `DISPLAY`, asking for a window means no plot at all. The row stays visible and reads
"on — no display" rather than flipping to "off" while the tabs keep arriving, which is what a
broken switch looks like. `ssh -X` with an X server at your end is a display like any other, so
there the choice is yours again — the question is whether a window can open, never whether the
connection is remote.

**A figure tab you closed came back.** The snapshot lists every figure the session is *holding*,
and the poll read that as "show these" — so plotting into figure 3 reopened figure 1's tab, closed
a minute ago because you were done with it. A tab now stays closed until that figure is drawn
again.

**The Run button typed a shell command at a live Octave prompt.** On a headless Linux build
`/usr/bin/octave` execs `octave-cli-11.3.0`, and Linux shows that name cut to fifteen characters.
Neither spelling was recognised as an Octave, so Run decided no prompt was open and sent
`octave --persist file.m`, which at an Octave prompt is only `error: 'octave' undefined`.

**Octave's advice about gnuplot arrived at your first plot.** Nine lines recommending qt instead,
printed the first time the toolkit is *used* — so on a machine whose only toolkit is gnuplot they
landed in the middle of your own output rather than at startup. They are about a choice CleeCode
made on your behalf, recommending a toolkit that needs the display the machine does not have, so
that one warning is off for the session. Everything else it wants to tell you still gets through.

**The presets put the prompt underneath at every width.** It used to move beside the editor on a
wide window. That reads well until a figure opens: the editor splits to put the plot next to the
code that drew it, and in three columns each half is about a third of the window — a plot that
size is a thumbnail. Underneath, the editor keeps the whole width to divide and the prompt keeps
it too, which is what a matrix row wanted in the first place.

A `settings.toml` written before this keeps working: `diagnostics_figures` is still read under its
new name, `plots_in_tabs`.

## What's new in 0.9.1

Four bugs in the numeric side, all of them reported from real use and none of them visible from
the machine it was built on.

**The workspace panel worked sometimes.** It followed "the newest `.json` in the snapshot
directory", and four kinds of JSON live there: the snapshot, the inspector's question, its answer,
and the breakpoints. So setting a breakpoint or opening the inspector made a request file the
newest thing there; the panel read it as a snapshot, found it was not one, and went blank until
the next tick wrote the real file. The same watcher is where the breakpoint file's path comes
from, so this was worse than a flicker — with the watch pointed at `break-0.json`, the next
breakpoint went to `break-break-0.json`, a file no session reads. Breakpoints that intermittently,
silently stopped being set.

**Plots opened in real windows.** The figure capture, and the setting that stops a window opening
at all, were installed by the `octave` preset's own startup command — so an Octave started any
other way got none of it: one typed at a shell tab, or the one the Run button starts when no
prompt is open. Octave's own mechanism fits better: the library directory now carries a `PKG_ADD`,
which Octave runs when the directory joins the load path, and `OCTAVE_PATH` is set on every shell.
Any Octave you start in a CleeCode terminal now reports its workspace and hands over its plots.
It prepends to your own `OCTAVE_PATH` rather than replacing it, and outside CleeCode it stays
inert.

**A machine with no display had no way to plot, and no way to find that out.** On a remote server
over ssh there is no display, qt cannot load, and with gnuplot not installed either Octave has no
graphics toolkit at all — which you discover from inside whichever line of your own script first
calls `figure()`, as `error: no graphics toolkits are available!`. A session CleeCode drives never
shows a window, so its toolkit only has to be able to *print*: gnuplot can do that with no display,
and is now preferred when there is none. With no toolkit at all, the panel says so from the first
snapshot — install gnuplot, it needs no display.

**The variables panel existed only inside the two built-in workspaces.** A saved workspace of your
own had none, and no way to ask for one: no menu entry, no command, no key. *Workspace ▸ Show
session variables* opens one wherever you are, and it follows whichever session last ran
something — so it picks up an interpreter you started yourself, in any terminal.

Also: `scripts/doctor.sh` reports what CleeCode set, what the interpreters see, and which graphics
toolkits exist. Two of these bugs were things that could only be described over a chat, which is a
slow way to find out that a load path is empty.

## What's new in 0.9.0

An IDE for Octave and Python, a language server, and completion — three releases in one.

### The numeric side

**A workspace window that fills in by itself.** Start `clee -w octave` or `clee -w pylab` and
there is a second terminal window beside your session showing what it holds: every variable, its
shape, its class, its range, and enough of its value to recognise. It is a *window*, not a tab,
so it stays in sight while you work. Nothing is typed at your prompt to produce it — the session
reports its own state from an idle hook, and your transcript stays a record of what you did.

**Run a piece of a file.** `Ctrl+Shift+X` sends the selection, or the cell the cursor is in, to
the interpreter that is already open — a cell being everything between `%%` markers in Octave or
`# %%` in Python, which is what both worlds already write. It goes through a scratch file rather
than a paste, because a pasted indented block makes Python answer `IndentationError`.

**Plots open as tabs beside the script that drew them**, resize with the pane by asking the
session to draw them again at the new size, and can be written out to PNG, PDF or SVG with a
command you can read. Octave figures are only re-printed when the figure says it changed;
matplotlib's are only re-rendered when it says they are stale.

**Look inside a variable** with `Ctrl+Shift+I`: pick one and a screenful of values arrives —
paged, so a 2000×2000 matrix does not have to be written to disk on the chance somebody looks. It
shows and does not yet edit.

**A debugger.** `Ctrl+Shift+P` puts a breakpoint on the cursor's line. When the session reaches
one it stops, the editor opens that file and marks the line, and the workspace window shows the
*frame's* variables rather than the ones outside it — which is the whole difference between
watching a program run and looking at what it left behind. Stepping is typed at the prompt where
the session is waiting (`dbstep`/`dbcont` in Octave, `n`/`c` at Python's `(Pdb)`), and the status
line says which. Setting a breakpoint leaves no line in your transcript.

**Double-click an error and land on it.** A traceback, a `grep` hit, a compiler message: the file
opens at the line with the cursor on the column, for Python's `File "…", line N`, Octave's
`name at line N column M`, and the ordinary `path:line:col`.

All of it works in both languages, and almost none of it works the same way underneath — Octave's
history comes from `history()` while Python's readline reports none at all and its own reader has
to be asked; Octave's breakpoints go through `dbstop` from inside the hook while Python's go
through pdb with tracing switched on for the length of exactly one statement, so that typing at
the prompt costs nothing.

Python figures need matplotlib installed in the same python your terminal runs.

### A language server

**`Ctrl+Shift+E` asks whichever language server is installed what is wrong**, and the answers are
underlined where they are, listed in a panel, and counted in the status line. It speaks LSP over
stdio directly rather than through a framework. A server that is not installed is not an error to
report: nothing is underlined and everything else carries on.

### Completion

**The words already in your buffers are offered as you type**, ranked exact-prefix first, then
case-insensitively, then fuzzily, with language keywords last. It claims exactly five keys and
gives them back the moment the popup closes, so it never interrupts the writing. When an
interpreter is open it also offers the names *that session* is holding — a variable you made at
the prompt and that exists in no file.

### Also

The editor is now driven in a real terminal by a test harness that reads back what lands on the
screen, which is how every claim above was checked rather than argued about.

## What's new in 0.6.2

Tools that are installed, found; a background that stays readable.

**PDF and Markdown previews work when CleeCode is started from the Dock.** An app launched from
a launcher inherits macOS's own environment, not a shell's — no Homebrew, no `/Library/TeX/texbin`,
nothing `/etc/paths.d` contributes, because all of that reaches the `PATH` through a shell's
startup files. Every outside tool the preview uses then looked uninstalled, so a PDF that opened
perfectly from a terminal said there was no rasteriser on the machine. `pdftoppm`, `gs`, `pdfinfo`
and `pandoc` are now looked for where they are actually installed, the same way the TeX engine
already was, and the message when there really is none says what to install.

**A solid background, one button away.** At the right-hand end of the menu bar, next to the
workspace badge: it fills in the background CleeCode was letting the terminal show through. A
translucent terminal with a bright window behind it leaves the text barely readable, and this is
the way out without going to change the terminal's own settings. *View ▸ Solid background* is the
same switch, and the choice is remembered. It is painted over the finished frame, so dialogs —
which clear the cells they cover — stay opaque too.

Also: the menu bar is measured in screen columns rather than characters. The turtle in the corner
is two columns wide and one character long, which put everything to the right of it, and every
click mapped onto a menu title, one column out.

## What's new in 0.6.1

A Dock icon, for free.

**`clee --install-app` puts CleeCode in /Applications (macOS).** Clicking it comes back to the
project you were last in. Drop a file or a folder on it — or pick CleeCode under *Open with* in
Finder's Get Info, then *Change All…* — and it opens that instead, with its folder as the project
root. To keep it in the Dock, open it once and choose *Options ▸ Keep in Dock*; to uninstall,
drag the app to the Bin.

The bundle is built on your machine rather than downloaded, and that is the whole reason this
costs nothing. A `.app` fetched from the internet arrives quarantined and needs an Apple
Developer signature — the yearly fee — or the user has to unblock it by hand through a dialog
that claims the app is damaged. One written locally by a program you just ran carries no
quarantine and simply opens. It is a command rather than something a normal launch does for the
same reason `--install-font` is: putting an icon in your Applications folder is a decision.

It needs [Ghostty](https://ghostty.org) to host the editor, and asks the Ghostty already running
for a new window instead of starting a second one — which is automation, so macOS asks permission
the first time. Refusing costs only that: it falls back to opening a separate instance and still
works. The path to `clee` is compiled into the launcher, so run the command again after moving or
reinstalling the binary.

Linux is not covered yet; the equivalent is a `.desktop` file, and the command says so rather
than pretending.

**`--resume` starts in the last project, wherever it was run from.** This is what the launcher
uses, because an icon has no current directory — inheriting whatever folder Finder happened to be
in is not an answer. A bare `clee` is unchanged and still opens where you are standing.

## What's new in 0.6.0

Finding things, and seeing what changed.

**Find reads patterns, and stops minding case unless asked.** It was a case-sensitive substring
scan, which could not answer "where is this word, however it is spelled" — the search anyone
actually types. `Ctrl+U` makes case matter; `Ctrl+N` reads the query as a regular expression, and
with it on the replacement can quote the groups back: search `(\w+)@(\w+)`, replace `$2.$1`. A
literal search has no groups, so there a `$` stays a dollar. Both switches, and which way they
are set, are printed in the box. A pattern that will not compile says so instead of reporting no
matches — half-typed and no-results want different fixes.

**Search the whole project with `Ctrl+Shift+H`.** The results are an ordinary list: type to narrow
them further, Enter or a click to open one at its line with the cursor on the word. It starts
from the selection, or from the last thing in the Find box. It walks the project itself rather
than shelling out to ripgrep — one engine means a pattern means the same thing in a file and
across the project, and it works where rg is not installed. The walk runs on a thread, skips
`.git`, `target` and `node_modules`, ignores anything over 2 MB or not text, and takes one hit
per line.

**A read-only git panel on `Ctrl+Shift+D`.** Three tabs. *Changes* is the diff of the file you
are looking at — the whole tree with none open — against the last commit, so staged and unstaged
work both show. *History* is the last fifty commits. *Branches* lists them with the current one
marked and how far each is from its upstream. `Tab` or `←→` switch, `↑↓` scroll, `R` asks again,
`Esc` closes. It reads through `git` itself rather than a linked library, so what it says is what
the terminal beside it would say — hooks, signing and credential helpers included. Nothing here
writes: stage and commit in a shell, where you can see what happened.

### Fixed

**The wheel no longer stops after one screen.** Scrolling with a trackpad worked until the cursor
line left the view, and from there every notch was undone before it was drawn — the only way on
was to click into the text, which moved the cursor and so moved the wall with it. The view now
goes where it is sent and stays there; the next arrow key or keystroke brings it back to the
cursor, as everywhere else.

**Scrollbars can be aimed at.** They are one cell wide and invisible until reached, so hitting
one was luck. Approaching now brings the bar up while the click still has to land on it, and it
stays 2.5s after the last scroll instead of 1.2 — long enough for a hand to leave the trackpad
and go for it.

**Changing the project folder no longer destroys the workspace you were in.** It stayed attached,
and on exit the workspace file was rewritten to describe the folder you had wandered into, with
its files and its shells — silently. Changing folder now steps out of the workspace, leaving its
file exactly as it was. Reopen it from the Workspace menu, from any folder. The built-in layout
travels with you, since it belongs to no project.

## What's new in 0.5.2

**The wheel reaches the program in the pane.** A pane running something full-screen could not be
scrolled at all: the notch was dropped, on the reasoning that a program owning the screen has no
scrollback of ours to move through. True, and beside the point — it has one of its own, and it
asked to be told about the mouse so it could move it. htop, `less --mouse`, a mouse-mode vim,
Claude Code all turn mouse reporting on and were sitting there unscrollable. A notch now becomes
the report the program asked for (SGR, UTF-8 or the old single-byte encoding) at the cell under
the pointer, and Shift+PageUp goes to it too instead of being swallowed. Only the wheel: click and
drag stay with the pane, or selecting text out of it would stop working.

## What's new in 0.5.1

Windows only, and all of it the same mistake: POSIX rules applied to a platform that does not
use them. macOS and Linux are unaffected.

**Run commands reach the shell intact.** Paths were quoted the way a POSIX shell wants them —
single quotes, backslash escapes — and cmd.exe has neither, so a resolved interpreter arrived as
`'C:\Users\me\octave-cli.exe' script.m` and was looked for under that name, verbatim. Quoting now
answers for the shell the line is typed at: double quotes, and only where there is something to
protect.

**Dragging a file in works.** The drop was split by a POSIX splitter, where a backslash escapes
the character after it, so `C:\Users\me\notes.txt` became `C:Usersmenotes.txt` — a path that
exists nowhere, which is why a drop there did nothing at all and said nothing about it. A line
that yields no file is now also tried whole, so a path with spaces pasted from an address bar is
one file rather than four words.

**A drop from a Windows machine over ssh is recognised** as one, and says where the files are
instead of going quiet. It wanted a leading slash before, which no Windows path has.

**The toolbar names the program, not a mangled path.** An unquoted Windows path in a run command
went through the same POSIX splitter, so `C:\Octave\bin\octave-cli.exe` arrived as
`C:Octavebinoctave-cli.exe` — with no separator left to cut the name at, the whole thing sat on
the button.

**The venv label is a name again, not a whole path**, for a venv registered on one platform and
read on the other: the check for "is this a path" was `is_absolute`, which on Windows is false for
`/opt/venvs/ml-3.12` — no drive letter — and a settings.toml does get carried between machines.

## What's new in 0.5.0

**Two hundred languages, highlighted.** The grammars used to be syntect's own, which stop at the
Sublime Text packages: no TypeScript, no TOML, no Kotlin, no Swift, no Zig, no Dockerfile, no Vue.
They now come from [two-face](https://github.com/CosmicHorrorDev/two-face), which collects bat's
set — 213 of them, the ones written this decade included. A file name is also asked before its
extension, so `CMakeLists.txt` is CMake rather than prose, and spellings without a grammar of
their own (`.cjs`, `.mts`, `.jsonc`, `.astro`, a bare `Gemfile`) are read as the language they
are. `Ctrl+/` knows the comment syntax of about a hundred and twenty file types now, up from
fifty, `Makefile` and `Dockerfile` among them.

**The startup command no longer collides with the `clear`.** It came back: a shell inherited from
the previous layout was handed its command while the queued `clear` was still sitting in its line
editor, and the two arrived as `clearclaude`. Nothing is written into a pty before the shell has
read from it any more — the command waits for the prompt, the line is emptied first, and exactly
one line is ever typed. Tested against bash, zsh and fish rather than argued about.

**A picture is inverted, a document is darkened.** They shared one button, one name and one
remembered preference, so reading a PDF at night meant the next photograph opened as a negative.
Now a picture has its own **invert** (`i`), which belongs to that tab and is never carried to the
next one, while **dark** (`d`) is for documents and is remembered *separately for PDFs and for
markdown* — a paper and a README are read in different places, and setting one no longer changes
the other. Both are written to disk when you press them rather than at exit.

**The zoom buttons work on pictures.** They did nothing: a photograph was decoded at whatever size
it was saved at, and the widget shrank it to the pane no matter how far you had zoomed. The zoom
now reaches the pixels, which also gives `fit` and `wide` something to mean on an image.

**Markdown renders both ways, on demand.** A new **text** (`t`) button switches between the
rendered document and the styled terminal text — the first is prettier, the second follows your
keystrokes and needs neither pandoc nor a graphics protocol — and which you chose is remembered.
Over the text view the bar carries nothing else, since there are no pixels there to zoom, fit or
darken.

## What's new in 0.3.2

**Column selection.** `Alt`+drag makes a rectangular selection, or turn it on from the Edit menu
and draw it with `Shift`+arrows. Over ragged text the columns are clipped, not padded: a
rectangle selects the text that is actually there.

**The startup command no longer collides with the `clear`.** A shell opened with a command to run
gets that command *instead of* the startup `clear`, so only ever one line is queued in the pty —
which is what makes `clearclaude` on one line impossible rather than unlikely.

**Workspaces.** `clee -w NAME` opens one from the shell and `clee -w` lists them; the name it is
running under sits in the corner of the menu bar. A built-in **Default layout** is always offered
and cannot be deleted or overwritten. A bare `clee` no longer reopens a named workspace — that
stays a deliberate act — while the project, its files and your layout still come back. Loading one
restores which terminal had focus, which was being written to the file and then ignored.

**Reachable with the mouse.** The command palette, quick open, the venv browser and the workspace
lists were keyboard-only: no click reached them. Clicking a result now takes it, the wheel moves
the selection, and a click outside dismisses. In the *delete* list a click only selects, because a
list that refolds under the pointer should not delete on a single click.

**`clee -e FILE`** opens just that file — the editor and nothing else — and leaves your saved
layout and session alone, so a quick edit does not become the state you come back to.

**A man page**, `man clee`, installed by Homebrew and shipped in the archives. The manual now
navigates with `↑↓` between sections and `Space` to page, since the contents list is a column and
`PageUp`/`PageDown` want the `Fn` key.

And a turtle. You will find it.

## What's new in 0.3.1

- **`clee -w NAME`** opens a saved workspace straight from the shell, and `clee -w` on its own
  lists the ones you have. The name is announced on the splash while the shells start.
- **The workspace you are in is now visible**, in the corner of the menu bar. It was tracked all
  along but never shown, so the only clue was a status message that had scrolled away.
- **A built-in "Default layout" workspace**, always in the Workspace menu and impossible to
  delete, for putting the frames back the way they ship without editing settings by hand. A
  workspace of your own called `default` is untouched and keeps its place in the list.
- **Loading a workspace restores which terminal had focus.** It was being written to the file and
  then ignored on the way back in.

## What's new in 0.3.0

### It no longer closes on you

A panic used to take the whole editor down, and with it every shell running inside — an ssh
session, a long-running build, a `claude` in a pane, gone with no way back. Three real crashes
are fixed at the source (a terminal opened at zero height, a split editor in a very narrow
window, a stale tab index), and a safety net now contains anything left: a panic is reported in
the status line and written to `panic.log` instead of ending the session. A broken terminal
costs you that terminal, at most.

### Keys you can actually press

The bindings were rebuilt around two facts. Function keys and PageUp/PageDown need Fn on a
laptop, so they are gone — every one of them. And macOS only sends Option as Meta on US keyboard
layouts, so `Alt`+letter never arrived at all on an Italian, German or French keyboard; those are
gone too. What replaces them:

- **`Ctrl+Alt`+arrow** moves to the frame in that direction — sidebar, either half of a split
  editor, or a tiled terminal, whichever is actually there. (`Ctrl`+arrow alone belongs to
  macOS, which uses it for Mission Control and Spaces.)
- **`Ctrl+Shift`+letter** is the application's layer: `M` manual, `B` menu bar, `O` settings,
  `R` run, `T`/`K` terminal tab open/close, `N` new terminal window, `W` save workspace.
  It is safe inside a terminal because no terminal can encode `Ctrl+Shift` for the program
  running in a pane — so nothing there is listening for it.
- **`Ctrl+Shift+←/→`** moves between the tabs of the focused frame; `Ctrl+Tab` cycles the frames.

A focused terminal now gets every other `Ctrl` chord. `Ctrl+J` is Enter to a shell, `Ctrl+E` is
end-of-line, `Ctrl+T` is transpose — the editor no longer eats any of them.

### Named workspaces

Save a whole set-up under a name (`Ctrl+Shift+W`): project root, open files, frame sizes, and the
terminal windows with their tab names and startup commands. Reopening one brings the shells back
already running `claude`, `octave`, `npm run dev`. One hand-editable TOML file per workspace under
`~/.config/cleecode/workspaces/`.

### Also

A built-in manual on `Ctrl+Shift+M`, in English or Italian, syntax-coloured and reachable without
leaving the editor. Save As for buffers that never had a name. And the demo and screenshots are
now generated from scripts in `docs/`, so they cannot quietly go stale again.

## Install

**macOS** (Homebrew):

```bash
brew tap msavox/clee
brew trust msavox/clee
brew install clee
```

All three steps are needed: Homebrew executes a formula's Ruby on your machine, so it refuses
to load one from a third-party tap until you trust the source, and tapping does not imply
trusting.

Or download the archive for your platform from the assets below, unpack it, and put `clee`
somewhere on your `PATH`.

## Platform status

**macOS** is the supported platform: developed, tested and used there daily, on both Apple
Silicon and Intel.

The **Linux** and **Windows** binaries are compiled and started by CI — they build, launch,
and print `--version` — but they have had **no interactive testing at all**. Treat them as
experimental, and please open an issue for whatever breaks. Building from source works too;
see the README for the system dependencies.

The Linux build links against glibc and libxcb: on a minimal or headless system install
`libxcb1` (Debian/Ubuntu) or `libxcb` (Fedora/Arch) if it won't start. Alpine/musl and arm64
aren't covered yet — build from source there.

## First run

The file-tree icons need a Nerd Font. The bundled one installs with:

```bash
clee --install-font
```

Then restart your terminal. `clee --help` lists the rest.
