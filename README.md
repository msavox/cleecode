# CleeCode 🐢

An editor, a file tree and real terminals in one window. Written in Rust, driven from the
keyboard, with the mouse as an alternative rather than the only way.

By **Matteo Savoia** ([msavox](https://github.com/msavox)).

![CleeCode in action](docs/demo.gif)

## Installing

### macOS and Linux — Homebrew

```bash
brew tap msavox/clee
brew trust msavox/clee
brew install clee
```

The `brew trust` step is required, not a formality: a formula is Ruby code Homebrew executes
locally, so recent versions refuse to load one from a third-party tap until you trust its
source — tapping alone doesn't grant that. Without it you get `Refusing to load formula
msavox/clee/clee from untrusted tap`.

The formula builds from source (well under a minute on macOS; longer on Linux, where it also
pulls `libxcb`). [Homebrew on Linux](https://docs.brew.sh/Homebrew-on-Linux) uses the same tap
and CI verifies that install on Ubuntu, but only the *install* is tested.

### Prebuilt binaries

macOS arm64/x86_64 and x86_64 Linux and Windows builds are attached to each
[release](https://github.com/msavox/cleecode/releases). Outside macOS they're experimental: CI
checks they start, nothing more. The Linux binary needs glibc and libxcb — install `libxcb1`
(Debian/Ubuntu) or `libxcb` (Fedora/Arch) if it fails to start. For Alpine/musl, build from
source.

### Optional extras

Previews reach for a few outside tools. None is required — without them CleeCode shows less
rather than failing, and says so in the tab instead of leaving it blank.

```bash
brew install poppler        # PDF pages (ghostscript works too)
brew install pandoc typst   # Markdown as a real document, pictures and all
brew install chafa          # a picture inside a terminal pane
```

Real pixels also need a terminal with a graphics protocol — kitty, iTerm2, Ghostty, WezTerm.
Anywhere else, pictures fall back to coloured half-blocks and Markdown to styled text.

### From source

Needs a [Rust toolchain](https://rustup.rs) 1.85+ (edition 2024). On Linux the clipboard also
needs the X11/xcb headers; on Windows, the MSVC toolchain plus *Desktop development with C++*.

```bash
# Debian/Ubuntu
sudo apt install build-essential pkg-config libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
# Fedora
sudo dnf install gcc pkgconf-pkg-config libxcb-devel
# Arch
sudo pacman -S base-devel libxcb

cargo install --locked --git https://github.com/msavox/cleecode
```

That puts `clee` in `~/.cargo/bin` (`%USERPROFILE%\.cargo\bin`), so make sure it's on your
`PATH`.

### From a clone

```bash
cargo build --release
./target/release/clee              # the last project, its open files and your layout
./target/release/clee src/main.rs  # current directory, with a file pre-opened
./target/release/clee ./some-dir   # that directory as the project root
./target/release/clee -w work      # open the saved workspace called "work"
./target/release/clee -w           # list the saved workspaces
./target/release/clee -e notes.md  # just that file, everything else hidden
./target/release/clee --help       # usage, --version, --install-font
```

An argument skips the startup splash; the splash only shows on a bare `clee` or with `-w`,
where it names the workspace being opened.

### Nerd Font icons

The file tree's icons need a [Nerd Font](https://www.nerdfonts.com/). CleeCode bundles
JetBrainsMono Nerd Font Mono and can install it:

```bash
./target/release/clee --install-font
```

It copies the font into your per-user font directory (`~/Library/Fonts`,
`~/.local/share/fonts`, or `%LOCALAPPDATA%\Microsoft\Windows\Fonts`), points Ghostty at it if
present on macOS/Linux, and registers it on Windows. Restart your terminal afterwards — or
just point your terminal at a Nerd Font you already have.

## What it does

![CleeCode main view](docs/screenshots/main.png)

*The demo and the stills are replayed from [`docs/demo.tape`](docs/demo.tape) and
[`docs/shots.tape`](docs/shots.tape), so they are re-made after a UI change rather than re-shot
by hand and left to go stale.*

### Editing

[syntect](https://github.com/trishume/syntect) highlighting, line numbers, multi-file tabs,
undo with coalescing, find and replace, go-to-line, code folding, auto-indent and auto-closing
brackets. Selection works with the mouse or the keyboard, goes to the system clipboard, and can
be **rectangular** — `Alt`+drag for a column selection over ragged text.

`Ctrl+L` splits the editor into two independent editors sharing one pool of buffers: each half
has its own tabs, no file is in both strips at once, and closing the last tab of a half closes
the split rather than leaving it empty. Files changed underneath you are reloaded when they are
not dirty, and a binary or non-UTF-8 file opens
read-only rather than being corrupted on save. Scrollbars appear inside the frame while the view
moves or the pointer rests on them, and they are working controls: drag the thumb, click the
groove to jump, click the end arrows to step a line.

Pictures, PDFs and Markdown open as themselves. A `.png` gets a tab that draws it — real pixels
on a terminal with a graphics protocol (kitty, iTerm2, sixel), coloured half-blocks elsewhere —
instead of the blank read-only buffer a binary file used to give. A PDF opens as pages, turned
with the plain arrow keys, and re-renders in place when the file changes: edit the `.tex`, press
Run, and the page beside it is the one you just typeset.

Markdown gets a live preview beside the source — one file, two tabs, one copy of the text.
Where `pandoc` is installed it is a real document, so pictures the text refers to appear in the
flow of it; elsewhere it falls back to styled terminal text. CleeCode draws all of it itself, so
it works over `ssh` too.

For a one-off edit there is `clee -e FILE`: the editor and nothing else, leaving your saved
layout and session untouched.

![Split editor view](docs/screenshots/split.png)

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

![Command palette](docs/screenshots/palette.png)

There is a full menu bar behind `Ctrl+Shift+B`, context menus on right-click, and a manual that
travels with the binary — `Ctrl+Shift+M`, English or Italian, with diagrams.

![Built-in manual](docs/screenshots/manual.png)

There is also a `man clee`.

### The frame around it

A file tree with per-type Nerd Font icons and git status dots, live refresh, create/rename/delete
and drag & drop (dropped onto a terminal inside an `ssh` session, files go up with `scp`).
Three layout presets, a resizable everything, and a settings panel that applies changes live.
English and Italian throughout, including the manual.

![Layout and Run menus](docs/screenshots/menu.png)

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

## Requirements

**macOS** is the supported platform — where CleeCode is developed, tested and released.
**Linux and Windows** are written for throughout (paths, clipboard, shell, fonts, venvs) and
compile in CI, where the binary is also started to confirm it links and launches. Beyond that
nobody has used the editor there, so those builds are experimental and bug reports are welcome.

Built on [ratatui](https://github.com/ratatui/ratatui)/[crossterm](https://github.com/crossterm-rs/crossterm),
with [arboard](https://github.com/1Password/arboard) for the clipboard,
[sysinfo](https://github.com/GuillaumeGomez/sysinfo) for the process inspection behind Run and
`scp`-on-drop, and [dirs](https://github.com/dirs-dev/directories-rs) for config and font paths.
Terminal panes launch `$SHELL` (falling back to `/bin/bash`) on Unix and `%ComSpec%` on Windows.

## Status

Personal project, actively evolving.

## License

[MIT](LICENSE). The bundled font (`assets/fonts/`) is a Nerd Font-patched build of JetBrains
Mono under the [SIL Open Font License 1.1](assets/fonts/OFL.txt) and keeps its own terms.
