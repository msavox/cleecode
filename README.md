# CleeCode 🐢

A terminal IDE written in Rust: a `micro`-style editor with a file tree sidebar, integrated
terminals, syntax highlighting, and a classic drop-down menu bar. No mouse required, but it's
there if you want it.

By **Matteo Savoia** ([msavox](https://github.com/msavox)).

![CleeCode in action](docs/demo.gif)

*Recorded from [`docs/demo.tape`](docs/demo.tape).*

![CleeCode main view](docs/screenshots/main.png)

## Features

- **Editor** — [syntect](https://github.com/trishume/syntect) highlighting, line numbers,
  multi-file tabs, mouse and keyboard selection, system-clipboard copy/cut/paste, auto-indent,
  word wrap, whitespace display, code folding (`F7`)
- **Split editor** — `Alt+P` splits the editor column into two panes, each with its own tab
  strip; drag the divider (or use `F8`) to rebalance them
- **File tree sidebar** — per-type Nerd Font icons, git status dots (rolled up to folders), live
  refresh on external changes, create/rename/delete, hidden-file toggle (`H`), reroot on a folder
- **Run button** — `▶ Run` / `F10` pastes a run command for the current file into an idle
  terminal; configurable per extension in `settings.toml` (Python, Bash, Ruby, Node, Go, PHP,
  Perl, Octave…). Interpreters off `PATH` go under `[interpreter_paths]`
- **Quick open** — `Ctrl+O` fuzzy-searches the project; start the query with `/`, `~`, `./` or
  `../` and it becomes a filesystem browser, so files outside the root are reachable too
- **Venv selector** — drop-down of the Python virtualenvs at the project root, plus registered
  venvs from elsewhere on disk — typed in by path or picked with a folder browser (saved to
  `settings.toml`, available in every project)
- **Tabbed terminals** — real ptys, so `ssh`, `vim`, `claude` all work; each terminal window (a
  tiled pane) holds one or more tabs. New window (`F5`) vs new tab (`Ctrl+T`); close a tab or a
  whole window from its `✕`; rename a tab or window; drag the seam between windows to rebalance
  them; auto-collapse on shell exit
- **Terminal selection** — drag with the mouse or `Shift`+arrows; the selection goes straight to
  the system clipboard, `Esc` clears it
- **Menu bar** — macOS-style app menu plus File / Edit / View / Layout / Run / Terminal, their
  entries grouped into sections; `Alt+<letter>` or `F9`. `Ctrl+B` hides the bar without making
  anything unreachable
- **Context menus** — right-click or `Shift+F10` on the file tree, editor, or a terminal for its
  common actions
- **Resizable layout** — `F8` resize mode (arrows grow the focused frame, `Shift`+arrow shrinks)
  or drag any inner border with the mouse — sidebar, editor/terminal, the split divider, between
  terminals; three presets and a terminal-side toggle, persisted across restarts
- **Settings panel** — line numbers, highlighting, word wrap, tab size, tabs vs spaces,
  whitespace, auto-indent, mouse, language, all live
- **Workspace persistence** — a bare `clee` resumes the last project and its open tabs
- **Also** — `Alt+S` saves all, drag & drop into the tree (or `scp` onto an `ssh` session),
  auto-reload of externally changed files, English and Italian

![Layout and Run menus](docs/screenshots/menu.png)

![Split editor view](docs/screenshots/split.png)

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
| `F9` | Open/close the menu bar (then a letter to jump to a menu) |
| `Alt+<letter>` | Jump straight to a menu (see the underlined letter) |
| `F1` / `F2` / `F3` | Focus file tree / editor / terminal |
| `F4` | Settings |
| `F5` / `F6` | New terminal window / close the window |
| `Ctrl+T` | New terminal tab (in the focused window) |
| `F7` | Fold/unfold the block under the cursor |
| `F8` | Resize mode (arrows grow the focused frame, `Shift`+arrow shrinks, `Esc`/`Enter` to exit) |
| `F10` | Run the current file |
| `Ctrl+L` / `Alt+P` | Toggle split editor (`Alt+P` needs Option-as-Meta on macOS) |
| `Alt+Left` / `Alt+Right` | Switch focus between split panes |
| `Ctrl+S` / `Alt+S` | Save / save all |
| `Ctrl+E` / `Ctrl+J` | Toggle sidebar / terminal panel |
| `Ctrl+B` / `Alt+B` | Show/hide the menu bar |
| `Ctrl+W` / `Ctrl+D` | Close current tab (prompts if unsaved) |
| `Ctrl+Q` | Quit (prompts if any file is unsaved) |
| `Ctrl+C/X/V/A` | Copy / cut / paste / select all (in the editor) |
| `Ctrl+Z` / `Ctrl+Y` | Undo / redo (`Ctrl+Shift+Z` also redoes) |
| `Ctrl+Left/Right` | Move by word (`Shift` extends the selection) |
| `Ctrl+Backspace` / `Ctrl+Delete` | Delete the word before / after the cursor |
| `Ctrl+P` / `Ctrl+O` | Command palette / quick open (both fuzzy) |
| `Ctrl+F` / `Ctrl+G` | Find and replace / go to line |
| `Ctrl+/` | Toggle line comment |
| `Alt+Up` / `Alt+Down` | Move the current line up / down |
| `Alt+Shift+Down` | Duplicate the current line |
| `Tab` / `Shift+Tab` | Indent / outdent |
| `Alt+,` / `Alt+.` | Switch editor tab |
| `Ctrl+PageUp/Down` | Switch terminal window |
| `Alt+PageUp/Down` | Switch terminal tab |
| `Shift+F10` / right-click | Context menu for the focused frame |

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
