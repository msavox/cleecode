# CleeCode 🐢

A terminal IDE written in Rust: a `micro`-style editor with a file tree sidebar,
integrated terminals, syntax highlighting, and a classic drop-down menu bar —
all inside your terminal. No mouse required, but it's there if you want it.

![CleeCode main view](docs/screenshots/main.png)

## Features

- **Editor** — syntax highlighting (via [syntect](https://github.com/trevorfulton/syntect)), line numbers, multi-file tabs with a click-to-close `×`, mouse and keyboard text selection, copy/cut/paste with the real system clipboard, indent/outdent, auto-indent, word wrap, whitespace display, code folding (`F7`)
- **Split editor** — `Alt+P` or Layout → Split editor divides the editor column into two independent panes, each with its own tab strip; click either pane (or `Alt+Left`/`Alt+Right`) to move focus between them
- **File tree sidebar** — a Nerd Font icon and color per file type, folder icons, a right-aligned git status dot (modified/added/deleted/renamed/untracked, rolled up to parent folders), live refresh when files change on disk (even from another process), `H` toggles hidden files, double-click or `Enter` opens a file / makes a folder the new root, `..` walks back up, `Delete` asks for confirmation
- **Run button** — a `▶ Run` button above the editor (also `F10`) pastes a run command for the current file into the first idle terminal; commands are configurable per extension in `settings.toml` (defaults cover Python, Bash, Ruby, Node, Go, PHP, Perl, Octave...). For interpreters that aren't on `PATH`, map the program to an absolute path under `[interpreter_paths]` (e.g. `octave-cli = "C:\\Program Files\\GNU Octave\\Octave-10.1.0\\mingw64\\bin\\octave-cli.exe"`) — Octave on Windows is auto-detected there even without the entry
- **Venv selector** — next to Run, cycles through Python virtualenvs found at the project root and swaps in that venv's interpreter when running a `.py` file; venvs kept outside the project can be added to `registered_venvs` (`settings.toml`) and are then offered in every project, either as a bare path or with a short nickname to show in the selector (`[[registered_venvs]]` / `name = "ml"` / `path = "/opt/venvs/ml-3.12"`)
- **Multiple terminals** — starts with two side-by-side embedded terminals (real ptys, so `ssh`, `vim`, `claude`, etc. all work); open/close more, auto-clears startup banners, auto-collapses a pane when its shell exits
- **Terminal text selection** — drag with the mouse, or hold `Shift` with the arrow keys, to select terminal output; the selection goes to the system clipboard as soon as it's made (a terminal pane has no free key for an explicit copy — `Ctrl+C` has to reach the shell as an interrupt), and `Esc` clears it. Necessary because cleecode captures the mouse, which suppresses the host terminal's own selection
- **Menu bar** — a macOS-style `CleeCode` app menu (About / Settings / Quit) plus File / Edit / View / Layout / Run / Terminal; `Alt+<letter>` jumps straight to a menu (underlined mnemonic, Borland/Turbo-Vision style), or open with `F9` and type the letter — a fallback that works even when a terminal swallows Alt combos. Prefer a cleaner screen? `Ctrl+B` (or `Alt+B`, or View → Menu bar) hides the bar entirely; `F9` still opens the menus (the bar reappears while a menu is open), so nothing becomes unreachable
- **Resizable workspace layout** — `F8` (arrows to resize) or drag panel borders with the mouse; three built-in presets (Classic, Wide 2-column, Triple 3-column) and a terminal-on-left/right toggle, all persisted across restarts
- **Settings panel** — line numbers, syntax highlighting, word wrap, tab size, spaces-vs-tabs, whitespace, auto-indent, mouse, language — all live-toggleable
- **Workspace persistence** — launching with no arguments resumes the last project folder and every file that was open, including which tab was active
- **Save All** — `Alt+S` saves every dirty file at once, alongside per-file `Ctrl+S`
- **Drag & drop** — drop a file onto the file tree to copy it in; drop it onto a terminal running an active `ssh` session and CleeCode attempts an `scp` upload
- **Auto-reload** — picks up external changes to the open file without asking, as long as you have no unsaved edits
- **English by default**, Italian available in Settings → Language (small `i18n` layer, easy to extend)

![Layout and Run menus](docs/screenshots/menu.png)

![Split editor view](docs/screenshots/split.png)

## Installing

### macOS — Homebrew

Add the tap once, then install by name:

```bash
brew tap msavox/clee
brew install clee
```

A tap is just a GitHub repository Homebrew reads formulae from — here
[msavox/homebrew-clee](https://github.com/msavox/homebrew-clee). Adding it teaches your
`brew` about `clee`; after that it behaves like any other formula:

```bash
brew upgrade clee     # update to a newer release
brew uninstall clee   # remove it
brew untap msavox/clee
```

You can also skip the tap step with the fully qualified name, which works in one command:

```bash
brew install msavox/clee/clee
```

Those three parts are *user* / *tap* / *formula* — `clee` appears twice only because the tap
and the command happen to share a name.

The formula builds from source (it pulls Rust as a build dependency), which takes well under
a minute. Prebuilt macOS binaries for arm64 and x86_64 are attached to each
[release](https://github.com/msavox/cleecode/releases) if you'd rather not build at all.

### Linux and Windows — experimental binaries, or build it yourself

x86_64 builds for both are attached to each
[release](https://github.com/msavox/cleecode/releases). CI compiles them and checks that they
start, but nothing interactive has been tested on either platform yet — so they're
experimental, and bug reports are welcome.

The Linux binary links dynamically against glibc and libxcb, so a desktop distribution has
what it needs, but a minimal or headless system may not: install `libxcb1` (Debian/Ubuntu) or
`libxcb` (Fedora/Arch) if it fails to start. Alpine/musl isn't covered — build from source
there.

To build instead, you need a [Rust toolchain](https://rustup.rs) (1.85 or newer, for edition
2024).

**Linux.** The clipboard integration needs the X11/xcb development headers:

```bash
# Debian/Ubuntu
sudo apt install build-essential pkg-config libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
# Fedora
sudo dnf install gcc pkgconf-pkg-config libxcb-devel
# Arch
sudo pacman -S base-devel libxcb

cargo install --locked --git https://github.com/msavox/cleecode
```

**Windows.** Install Rust with the MSVC toolchain (rustup's default) plus the *Desktop
development with C++* workload from the Visual Studio Build Tools, then:

```powershell
cargo install --locked --git https://github.com/msavox/cleecode
```

Either way `cargo install` puts `clee` in `~/.cargo/bin` (`%USERPROFILE%\.cargo\bin`), so
make sure that's on your `PATH`. Note that CleeCode is developed and tested on macOS: the
Linux and Windows code paths are written but not yet exercised on those platforms, so treat
them as experimental and please report what breaks.

### From a clone

```bash
cargo build --release
./target/release/clee              # opens the current directory (or resumes the last workspace)
./target/release/clee src/main.rs  # opens the current directory with a file pre-opened
./target/release/clee ./some-dir   # opens that directory as the project root
./target/release/clee --help       # usage, --version, --install-font
```

Launching with a file or folder argument skips the startup splash and goes straight in;
the splash only shows on a bare `clee` (and any key dismisses it early).

### Nerd Font icons

The file tree's per-file-type icons need a [Nerd Font](https://www.nerdfonts.com/) to
render as icons instead of blank boxes. CleeCode bundles one (JetBrainsMono Nerd Font
Mono, SIL OFL) and can install it for you:

```bash
./target/release/clee --install-font
```

This copies the font into your per-user font directory (`~/Library/Fonts` on macOS,
`~/.local/share/fonts` on Linux, `%LOCALAPPDATA%\Microsoft\Windows\Fonts` on Windows). On
macOS/Linux it also points Ghostty's config at the font if one is present; on Windows it
registers the font so it's usable right away. Restart your terminal afterwards. If you use
a different terminal, or already have a Nerd Font configured, just point your terminal's
font setting at it manually.

## Key bindings

| Key | Action |
|---|---|
| `F9` | Open/close the menu bar (then type a letter to jump to a menu) |
| `Alt+<letter>` | Jump straight to a menu (see the underlined letter) |
| `F1` / `F2` / `F3` | Focus file tree / editor / terminal |
| `F4` | Settings |
| `F5` / `F6` | New / close terminal |
| `F7` | Fold/unfold the block under the cursor |
| `F8` | Resize mode (arrows to resize, `Esc`/`Enter` to exit) |
| `F10` | Run the current file |
| `Ctrl+L` / `Alt+P` | Toggle split editor view (`Ctrl+L` outside the terminal; `Alt+P` needs Option-as-Meta on macOS) |
| `Alt+Left` / `Alt+Right` | Switch focus between split panes |
| `Alt+S` | Save all files |
| `Ctrl+E` / `Ctrl+T` | Toggle sidebar / terminal panel |
| `Ctrl+B` / `Alt+B` | Show/hide the menu bar (`Ctrl+B` outside the terminal; `Alt+B` needs Option-as-Meta on macOS) |
| `H` (sidebar focused) | Toggle hidden files |
| `Ctrl+S` | Save |
| `Ctrl+W` / `Ctrl+D` | Close current tab (prompts if unsaved) |
| `Ctrl+Q` | Quit (prompts if any file is unsaved) |
| `Ctrl+C/X/V/A` | Copy / cut / paste / select all (in the editor) |
| `Ctrl+Z` / `Ctrl+Y` | Undo / redo (`Ctrl+Shift+Z` also redoes) |
| `Ctrl+Left/Right` | Move by word (`Shift` extends the selection) |
| `Ctrl+Backspace` / `Ctrl+Delete` | Delete the word before / after the cursor |
| `Ctrl+P` | Command palette (fuzzy) |
| `Ctrl+O` | Quick-open a file (fuzzy) |
| `Ctrl+F` | Find / replace in the current file |
| `Ctrl+G` | Go to line |
| `Ctrl+/` | Toggle line comment on the line/selection |
| `Alt+Up` / `Alt+Down` | Move the current line up / down |
| `Alt+Shift+Down` | Duplicate the current line |
| `Tab` / `Shift+Tab` | Indent / outdent |
| `Alt+,` / `Alt+.` | Switch editor tab |
| `Ctrl+PageUp/Down` | Switch terminal |

In the file tree: `↑↓` move, `→` expand, `←` collapse (or jump to parent), `Enter` or a
double-click opens a file / makes a folder the new root / walks up via `..`, `n` / `N`
create a new file / folder in the selected directory, `E` renames, `Delete` removes (with
confirmation), `H` toggles hidden files.

## Requirements

**macOS** is the supported platform — it's where CleeCode is developed, tested and released.
**Linux and Windows** are written for throughout (paths, clipboard, shell, fonts and venvs
all have per-OS handling) and both compile in CI, where the built binary is also
started to confirm it launches and links correctly. Beyond that they are untested: nobody
has actually used the editor on either platform. Binaries ship as experimental, and bug
reports are welcome.

System-clipboard access goes
through [arboard](https://github.com/1Password/arboard) (native clipboard on each OS),
process inspection for the `scp`-on-drop and Run features uses
[sysinfo](https://github.com/GuillaumeGomez/sysinfo), and config/font paths resolve via
[dirs](https://github.com/dirs-dev/directories-rs); the UI itself is plain
[ratatui](https://github.com/ratatui/ratatui)/[crossterm](https://github.com/crossterm-rs/crossterm).
The terminal pane launches `$SHELL` (falling back to `/bin/bash`) on Unix and `%ComSpec%`
(`cmd.exe`) on Windows. Developed primarily on macOS.

## Status

Personal project, actively evolving.

## License

[MIT](LICENSE). The bundled font (`assets/fonts/`) is a renamed Nerd Font-patched build of
JetBrains Mono under the [SIL Open Font License 1.1](assets/fonts/OFL.txt) and keeps its own
terms.
