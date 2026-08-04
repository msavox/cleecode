# CleeCode 🐢

A terminal IDE written in Rust: a `micro`-style editor with a file tree sidebar, integrated
terminals, syntax highlighting, and a classic drop-down menu bar. No mouse required, but it's
there if you want it.

By **Matteo Savoia** ([msavox](https://github.com/msavox)).

![CleeCode in action](docs/demo.gif)

*Recorded from [`docs/demo.tape`](docs/demo.tape); the stills come from
[`docs/shots.tape`](docs/shots.tape). Both replay a fixed script, so they can be re-made after a
UI change instead of re-shot by hand.*

![CleeCode main view](docs/screenshots/main.png)

## Features

- **Editor** — [syntect](https://github.com/trishume/syntect) highlighting, line numbers,
  multi-file tabs, mouse and keyboard selection, system-clipboard copy/cut/paste, auto-indent,
  word wrap, whitespace display, code folding (`Ctrl+Shift+F`)
- **Split editor** — `Ctrl+L` splits the editor column into two panes, each with its own tab
  strip; drag the divider (or use `Ctrl+Shift+U`) to rebalance them
- **File tree sidebar** — per-type Nerd Font icons, git status dots (rolled up to folders), live
  refresh on external changes, create/rename/delete, hidden-file toggle (`H`), reroot on a folder
- **Run button** — `▶ Run` / `Ctrl+Shift+R` pastes a run command for the current file into an idle
  terminal; configurable per extension in `settings.toml` (Python, Bash, Ruby, Node, Go, PHP,
  Perl, Octave…). Interpreters off `PATH` go under `[interpreter_paths]`
- **Quick open** — `Ctrl+O` fuzzy-searches the project; start the query with `/`, `~`, `./` or
  `../` and it becomes a filesystem browser, so files outside the root are reachable too
- **Venv selector** — drop-down of the Python virtualenvs at the project root, plus registered
  venvs from elsewhere on disk — typed in by path or picked with a folder browser (saved to
  `settings.toml`, available in every project)
- **Tabbed terminals** — real ptys, so `ssh`, `vim`, `claude` all work; each terminal window (a
  tiled pane) holds one or more tabs. New window (`Ctrl+Shift+N`) vs new tab (`Ctrl+Shift+T`); close a tab or a
  whole window from its `✕`; rename a tab or window; drag the seam between windows to rebalance
  them; auto-collapse on shell exit
- **Terminal selection** — drag with the mouse or `Shift`+arrows; the selection goes straight to
  the system clipboard, `Esc` clears it
- **Menu bar** — macOS-style app menu plus File / Edit / View / Layout / Run / Terminal, their
  entries grouped into sections; `Ctrl+Shift+B` opens it, then the underlined letter jumps to a menu. `Ctrl+B` hides it without making
  anything unreachable
- **Context menus** — right-click or `Ctrl+Shift+G` on the file tree, editor, or a terminal for its
  common actions
- **Resizable layout** — `Ctrl+Shift+U` resize mode (arrows grow the focused frame, `Shift`+arrow shrinks)
  or drag any inner border with the mouse — sidebar, editor/terminal, the split divider, between
  terminals; three presets and a terminal-side toggle, persisted across restarts
- **Settings panel** — line numbers, highlighting, word wrap, tab size, tabs vs spaces,
  whitespace, auto-indent, mouse, language, all live
- **Named workspaces** — save a whole set-up under a name (`Ctrl+Shift+W`): project root, open
  files, frame sizes, and the terminal windows with their tab names and startup commands, so
  reopening one brings the shells back already running `claude`, `octave`, `npm run dev`… Open one
  from the Workspace menu or straight from the shell with `clee -w NAME`; the name it is running
  under sits in the corner of the menu bar. One hand-editable TOML file per workspace under
  `~/.config/cleecode/workspaces/`, plus a built-in **Default layout** that puts the frames back
  the way they ship and cannot be deleted. A bare `clee` resumes the last one
- **Built-in manual** — `Ctrl+Shift+M`, in English or Italian, so the key bindings are reachable
  from inside the editor rather than only from this file
- **Also** — `Ctrl+Shift+S` saves all, drag & drop into the tree (or `scp` onto an `ssh` session),
  auto-reload of externally changed files, English and Italian

![Layout and Run menus](docs/screenshots/menu.png)

![Split editor view](docs/screenshots/split.png)

Nothing needs to be memorised: `Ctrl+P` fuzzy-searches every action in the app, with the key
that would have done it shown alongside.

![Command palette](docs/screenshots/palette.png)

And the manual travels with the binary — `Ctrl+Shift+M`, English or Italian.

![Built-in manual](docs/screenshots/manual.png)

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
./target/release/clee              # current directory (or resume the last workspace)
./target/release/clee src/main.rs  # current directory, with a file pre-opened
./target/release/clee ./some-dir   # that directory as the project root
./target/release/clee -w work      # open the saved workspace called "work"
./target/release/clee -w           # list the saved workspaces
./target/release/clee --help       # usage, --version, --install-font
```

An argument skips the startup splash; the splash only shows on a bare `clee`.

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


In the file tree: `↑↓` move, `→` expand, `←` collapse or jump to parent, `Enter` / double-click
opens a file or reroots a folder (`..` walks up), `n` / `N` create a file / folder, `E` renames,
`Delete` removes with confirmation, `H` toggles hidden files.

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
