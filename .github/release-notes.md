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
