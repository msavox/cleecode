# What CleeCode does

The tour. Every key here is also in the built-in manual (`Ctrl+Shift+M`), which ships inside
the binary and so cannot fall out of step with it; this page is the version you can read
before installing anything.

Two more pages go deeper where a paragraph is not enough:

  · [The numeric side](numeric.md) — Octave and Python as an IDE
  · [How the interpreter side was built](design/ide-mode.md) — the measurements behind it


![CleeCode main view](screenshots/main.png)

*The demo and most stills are replayed from [`docs/demo.tape`](demo.tape) and
[`docs/shots.tape`](shots.tape), so they are re-made after a UI change rather than left to
go stale. The preview shots below are taken by hand: they need a terminal that can draw
pictures, and the recorder has none.*

### Editing

[syntect](https://github.com/trishume/syntect) highlighting over
[two-face](https://github.com/CosmicHorrorDev/two-face)'s grammars — 200-odd languages, the
ones written this decade included — line numbers, multi-file tabs,
undo with coalescing, find and replace, go-to-line, code folding, auto-indent and auto-closing
brackets. Selection works with the mouse or the keyboard, goes to the system clipboard, and can
be **rectangular** — `Alt`+drag for a column selection over ragged text.

`Ctrl+L` splits the editor into two independent editors sharing one pool of buffers: each half
has its own tabs, no file is in both strips at once, and closing the last tab of a half closes
the split rather than leaving it empty. Closing the last tab of all leaves nothing open — an
empty frame that says how to open something — rather than the untitled buffer that used to take
its place, which made that tab the one tab you could not close. Files changed underneath you are reloaded when they are
not dirty, and a binary or non-UTF-8 file opens
read-only rather than being corrupted on save. Scrollbars appear inside the frame while the view
moves or the pointer rests on them, and they are working controls: drag the thumb, click the
groove to jump, click the end arrows to step a line.

Pictures, PDFs and Markdown open as themselves. A `.png` gets a tab that draws it — real pixels
on a terminal with a graphics protocol (kitty, iTerm2, sixel), coloured half-blocks elsewhere —
instead of the blank read-only buffer a binary file used to give. A PDF opens as pages, turned
with the plain arrow keys, and re-renders in place when the file changes: edit the `.tex`, press
Run, and the page beside it is the one you just typeset.

Every preview carries a navigation bar along its bottom edge: the page arrows, `go` to jump to a
page by number, `-` and `+` for zoom, and `fit` or `wide` to size the page to the pane or to its
width. Each control is labelled with its own key, so the bar is also the reminder of how to work
without the mouse — though the wheel zooms as well, and the scrollbars drag. Documents get one
control pictures do not, `dark`, which inverts the page for reading at night: inverting a
photograph is not a reading aid, it is just a wrong photograph. The setting is remembered
between sessions.

![A picture in a tab, and the same picture through chafa in a terminal](screenshots/preview-image.jpg)

*The same file twice: real pixels in the tab, and `chafa` putting it into a terminal pane with
the Run button beside it.*

Markdown gets a live preview beside the source — one file, two tabs, one copy of the text, so
the two can never disagree about what it says. Where `pandoc` is installed it is a real
document, pictures the text refers to included; elsewhere it falls back to styled terminal text.
CleeCode draws all of it itself, so it works over `ssh` too.

![A LaTeX source and its typeset PDF side by side](screenshots/preview-pdf.png)

*Edit the `.tex`, press Run, and the page beside it is the one you just typeset. On a preview
tab the button says Refresh instead, because there the file is generated and can come out
different.*

![Markdown source and its rendered document](screenshots/preview-md.png)

*Two tabs onto one file. The glyph in the strip tells them apart, and typing in the source moves
the document beside it without a save.*

Two letters into a word, a short list drops under the cursor: the words already written in the
open files, and the language's keywords. It is not a box you have to answer — `↑↓` walk the list,
`Tab` or `Enter` take a word, `Esc` dismisses, and every other key goes on typing while the list
narrows. The nearest words come first and keywords last, because a keyword is quicker to type
than a list is to walk. Where an interpreter is open, the names *that session* is holding are
offered too, in green: a variable you made at the prompt exists in no file, so nothing that reads
one could suggest it.

Where a language server is installed it feeds the same list, in magenta — and those are the names
no amount of reading the file could have found, because after a dot they are whatever that *type*
has. The list never waits for them: it opens on the words in the file and the server's names drop
into it a moment later, without moving a row you have already arrowed down to.

The same server underlines what it finds wrong, where it is — red for an error, yellow for a
warning — the line number takes the same colour, and the message for the line you are on sits at
the right of the status bar. When the line is clean, that spot says what the thing under the
cursor **is** instead: the type, or the signature, one line of it. Nothing is pressed for it — it
arrives when the cursor stops on a word, and a diagnostic wins the space when there is one,
because an error on this line is news and a type is not.

`Ctrl+Shift+J` goes to the definition of what is under the cursor and `Ctrl+Shift+L` comes back.
A stack of them, so following a name into a name into a name still leads home.

It speaks LSP over stdio directly rather than through a framework, and is told about an edit once
you stop typing rather than on every key. Names are known for rust-analyzer, pyright, tsserver,
gopls, clangd, lua-language-server, zls, solargraph, bash-language-server, texlab and a couple
more; one process per program, started the first time a file it serves is open, so a project with
Rust and Python in it ends up with both and each is told only about its own files. For anything
else — a language nobody put in that list, or your own build of a server — `settings.toml` takes a
`[language_servers]` table of `extension = "command line"`, which wins over the built-in names, and
an entry set to `""` turns a built-in one off. A server that is not installed is not an error to
report: nothing is underlined, the list has the file's own words in it, and everything else carries
on.

`Ctrl+Shift+D` opens the git panel — or the **Git** menu, which opens it already on the tab you
came for, and a right-click on a changed file in the tree, which offers the same actions for that
file under a heading of their own.

**Status** is the tab you act on: every changed file with git's own two letters in front of it —
the index's and the working tree's — because `MM` is a file that was added and then changed again,
and one letter would lose half of that. `S` stages the file under the cursor, `U` takes it back
out, `A` stages everything, `C` asks for a message and commits, `E` rewrites the last commit, `Z`
puts the working tree away as a stash, `Enter` opens the file. **Changes** is the diff.
**Branches** lists them with the current one marked and how far each is from its upstream —
`Enter` moves to one and git refuses that itself if it would write over uncommitted work, `N`
makes one, `D` deletes one, `M` merges it into where you are. **Stashes** is what you have put
away: `Enter` applies, `O` pops, `D` drops.

**History** is a graph — every branch at once, in lanes, drawn with `git log --graph`'s own six
ASCII characters. That is deliberate: box-drawing and braille make a prettier picture where the
font has them and come out as boxes or half-column offsets over `ssh` to whatever console is
there, and a graph that is wrong about which line joins which is worse than no graph. `[main]` is
where you are standing, `(spike)` a branch, `<v1>` a tag. `Enter` opens the commit in full — its
message, what it touched and the patch; `B` starts a branch at it, `T` tags it, `K` copies it onto
your branch, `V` undoes it in a new commit, `H` moves the branch back to it.

Unlike `git log --graph`, lanes never shuffle left: a lane that empties stays empty until
something reuses it. That costs a column or two and buys lines that stay in their column, so the
only diagonals left are the two that mean something — a branch leaving and a branch coming back.

`Q` gets out of a merge, a pick, a revert or a rebase that stopped part-way, and is offered only
while there is one to get out of.

`F`, `L` and `P` — fetch, pull and push — are typed into one of your shells rather than run behind
the panel, and that is the point rather than a shortcut: they can stop to ask for a passphrase, a
two-factor code or a host key, and a modal panel has nowhere to put such a question. A terminal is
exactly the thing that can ask it. The panel closes and the shell takes the focus.

`X` throws away every change to a file, and it is the only thing in CleeCode that destroys work:
what it removes is in no commit and no stash. So it asks first, and the question takes one letter
and reads every other key as no — including the ones that do something to the list behind it. A
file git has never been told about is refused rather than deleted; there is nothing to put back,
and `rm` belongs in the terminal where it reads as what it is.

`X` and the other questions that cannot be taken back are drawn in red only where saying yes
destroys something that is in no commit, no stash and no reflog — throwing a file's changes away,
`reset --hard`, dropping a stash. Deleting a branch asks in the same shape and not in the same
colour, because its commits stay in the reflog for ninety days and red on every question is red on
none of them.

Everything goes through `git` on PATH rather than a library, so what the panel does is what the
terminal beside it would do: hooks run, commits are signed, credential helpers are asked. The
spellings are the old ones — `reset HEAD --`, `checkout HEAD --`, `stash save` — because a
long-lived server reached over `ssh` is exactly where a terminal editor earns its keep, and it is
also where the newer commands are missing.

For a one-off edit there is `clee -e FILE`: the editor and nothing else, leaving your saved
layout and session untouched.

![Split editor view](screenshots/split.png)

### Terminals that are real

Each terminal window is a tiled pane holding one or more tabbed shells, on proper ptys — `ssh`,
`vim`, `claude` all work. Panes can be renamed, given a startup command, resized by dragging the
seam, and they collapse when their shell exits.

The keys respect that: a focused terminal keeps every `Ctrl` chord for the program running in it.
`Ctrl+J` is Enter to a shell, `Ctrl+E` is end-of-line, and the editor does not steal either.

Each shell keeps its scrolled-off output. The wheel walks back through it, typing returns to the
live end, and output arriving while you read back does not drag the page away. The same
scrollbar the editor has shows where in the history you are.

`▶ Run` runs the current file in an idle terminal. The button beside it says what Run will use
on *this* file and changes it: the venv selector on a `.py` file, and on any file type the run
command for its extension — `{file}`, `{dir}`, `{name}` and `{stem}` to build it, so a `.tex`
file can typeset and open its own PDF, and `chafa` will put a `.png` in a terminal pane beside
its output. A command can be shared by every project or kept in the project's own `.cleecode.toml`,
which wins and is meant to be committed with it. Interpreters off `PATH` go under
`[interpreter_paths]`.

### Workspaces

Save a whole set-up under a name: project root, open files, frame sizes, and the terminal windows
with their tab names and startup commands. Reopening one brings the shells back already running
`claude`, `octave`, `npm run dev`. Open it from the Workspace menu or straight from the shell
with `clee -w NAME`; the name it is running under sits in the corner of the menu bar.

Each is one hand-editable TOML file under `~/.config/cleecode/workspaces/`, so they travel
between machines. A built-in **Default layout** is always there and cannot be deleted or
overwritten. A bare `clee` never reopens a named workspace — that stays a deliberate act — but it
does restore the project, its open files and the layout you left.

### Finding your way

Nothing needs to be memorised. `Ctrl+P` fuzzy-searches every action in the app and shows the key
that would have done it; `Ctrl+O` does the same for files, and a query starting `/`, `~`, `./` or
`../` turns it into a filesystem browser.

![Command palette](screenshots/palette.png)

There is a full menu bar behind `Ctrl+Shift+B`, context menus on right-click, and a manual that
travels with the binary — `Ctrl+Shift+M`, English or Italian, with diagrams.

![Built-in manual](screenshots/manual.png)

There is also a `man clee`.

### The frame around it

A file tree with per-type Nerd Font icons and git status dots, live refresh, create/rename/delete
and drag & drop (dropped onto a terminal inside an `ssh` session, files go up with `scp`).
Three layout presets, a resizable everything, and a settings panel that applies changes live.
English and Italian throughout, including the manual. The `◐` at the right-hand end of the menu
bar fills in the background, for a translucent terminal with something bright behind it — which
is worth knowing about the other way round too: by default CleeCode paints no background of its
own, so a terminal with a translucent window shows your desktop through the editor.

![Layout and Run menus](screenshots/menu.png)

### Octave and Python, as an IDE

`clee -w octave` or `clee -w pylab` starts the interpreter you already have — in a terminal you
can type into — and puts a second window beside it showing what that session holds: every
variable with its size, class and range, filling in by itself whenever a command finishes.

![The workspace window filling in from a cell](screenshots/workspace.png)

Nothing CleeCode wants to know is typed at your prompt. Your transcript stays a record of what
*you* did, and a busy interpreter is never interrupted mid-computation to answer a question about
itself. `Ctrl+Shift+X` sends a cell to the running session, plots arrive as tabs beside the script
that drew them, `Ctrl+Shift+I` looks inside a variable, and `Ctrl+Shift+P` sets a breakpoint —
where the panel shows the *frame's* variables rather than the session's.

![Stopped at a breakpoint](screenshots/debug.png)

Everything works in both languages and almost none of it works the same way underneath.
**[The full walkthrough is its own page](numeric.md)**, including what is different between the
two and what is not built yet.

### It does not close on you

CleeCode hosts long-running shells, so a crash costing you an `ssh` session or a build would be
the worst thing it could do. An internal failure is contained and reported in the status line
rather than ending the process: a broken terminal costs you that terminal, at most. Details go to
`~/.config/cleecode/panic.log`.

## Key bindings


| Key | Action |
|---|---|
| `Ctrl+Alt+←` `↑` `↓` `→` | Go to the frame that lies in that direction — sidebar, either half of a split editor, or a tiled terminal, whichever is there. `Ctrl+Alt` rather than plain `Ctrl` because macOS keeps `Ctrl`+arrow for Mission Control and Spaces |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Or cycle the frames, the way `Cmd+Tab` cycles windows |
| `Ctrl+Shift+←` / `→` | Previous / next tab *inside* the focused frame |
| `Ctrl+Shift+↑` / `↓` | Previous / next terminal window, whatever the layout |
| `Ctrl+Shift+M` | The built-in manual |
| `Ctrl+Shift+B` | Open the menu bar (then arrows and Enter) |
| `Ctrl+Shift+O` | Settings |
| `Ctrl+Shift+G` / right-click | Context menu for the focused frame |
| `Ctrl+Shift+R` | Run the current file |
| `Ctrl+Shift+T` / `Ctrl+Shift+K` | New terminal tab / close this shell |
| `Ctrl+Shift+N` | New terminal window |
| `Ctrl+Shift+U` | Resize mode (arrows grow the focused frame, `Shift`+arrow shrinks) |
| `Ctrl+Shift+F` | Fold/unfold the block under the cursor |
| `Ctrl+L` | Toggle split editor (`Ctrl+Alt+←`/`→` moves between the panes) |
| `Ctrl+S` / `Ctrl+Shift+S` | Save / save all (an unnamed buffer asks for a name; Save As is in the File menu) |
| `Ctrl+Shift+W` | Save the current workspace (open and delete are in the View menu) |
| `Ctrl+Shift+E` | Name the focused terminal and give it a startup command |
| `Ctrl+E` / `Ctrl+J` | Toggle sidebar / terminal panel |
| `Ctrl+B` | Show/hide the menu bar |
| `Ctrl+W` / `Ctrl+D` | Close current tab (prompts if unsaved) |
| `Ctrl+Q` | Quit (prompts if any file is unsaved) |
| `Ctrl+C/X/V/A` | Copy / cut / paste / select all (in the editor) |
| `Ctrl+Z` / `Ctrl+Y` | Undo / redo (`Ctrl+Shift+Z` also redoes) |
| `Alt+Left` / `Alt+Right` | Move by word (`Ctrl`+arrow too, where the OS allows it) |
| `Ctrl+Backspace` / `Ctrl+Delete` | Delete the word before / after the cursor |
| `Ctrl+P` / `Ctrl+O` | Command palette / quick open (both fuzzy) |
| `Ctrl+F` / `Ctrl+G` | Find and replace / go to line |
| `Ctrl+U` / `Ctrl+N` | Inside Find: case sensitivity / read the query as a regex |
| `Ctrl+Shift+H` | Search the project; results are a list, `Enter` opens one at its line |
| `Ctrl+Shift+D` | Git panel: status, changes, history, branches — stage, commit, switch, straight from `git` |
| `Ctrl+K` | Toggle line comment |
| `Alt+Up` / `Alt+Down` | Move the current line up / down |
| `Alt+Shift+Down` | Duplicate the current line |
| `Tab` / `Shift+Tab` | Indent / outdent |
| `Alt`+drag | Column selection (also in the Edit menu, then `Shift`+arrows) |


In the file tree: `↑↓` move, `→` expand, `←` collapse or jump to parent, `Enter` / double-click
opens a file or reroots a folder (`..` walks up), `n` / `N` create a file / folder, `e` renames,
`Delete` removes with confirmation, `H` toggles hidden files.

There are deliberately **no function keys and no `PageUp`/`PageDown`** — on a laptop both need
`Fn` — and **no `Alt`+letter chords**, because macOS only sends Option as Meta on US keyboard
layouts, so on any other one they never arrived at all. `Ctrl+Shift` is the application's layer,
and it is safe inside a terminal for a structural reason: no terminal can encode `Ctrl+Shift` for
the program running in a pane, so nothing there is listening for it.

The same list, with more detail, is in the built-in manual (`Ctrl+Shift+M`) and in `man clee`.
