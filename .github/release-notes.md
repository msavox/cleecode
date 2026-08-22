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
