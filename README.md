# CleeCode 🐢

A terminal IDE written in Rust: a `micro`-style editor with a file tree sidebar,
integrated terminals, syntax highlighting, and a classic drop-down menu bar —
all inside your terminal. No mouse required, but it's there if you want it.

![CleeCode main view](docs/screenshots/main.png)

## Features

- **Editor** — syntax highlighting (via [syntect](https://github.com/trevorfulton/syntect)), line numbers, multi-file tabs with a click-to-close `×`, mouse and keyboard text selection, copy/cut/paste with the real system clipboard, indent/outdent, auto-indent, word wrap, whitespace display, code folding (`F7`)
- **Split editor** — `Alt+P` or Layout → Split editor divides the editor column into two independent panes, each with its own tab strip; click either pane (or `Alt+Left`/`Alt+Right`) to move focus between them
- **File tree sidebar** — a Nerd Font icon and color per file type, folder icons, a right-aligned git status dot (modified/added/deleted/renamed/untracked, rolled up to parent folders), live refresh when files change on disk (even from another process), `H` toggles hidden files, double-click or `Enter` opens a file / makes a folder the new root, `..` walks back up, `Delete` asks for confirmation
- **Run button** — a `▶ Run` button above the editor (also `F10`) pastes a run command for the current file into the first idle terminal; commands are configurable per extension in `settings.toml` (defaults cover Python, Bash, Ruby, Node, Go, PHP, Perl, Octave...)
- **Venv selector** — next to Run, cycles through Python virtualenvs found at the project root and swaps in that venv's interpreter when running a `.py` file
- **Multiple terminals** — starts with two side-by-side embedded terminals (real ptys, so `ssh`, `vim`, `claude`, etc. all work); open/close more, auto-clears startup banners, auto-collapses a pane when its shell exits
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

## Getting started

```bash
cargo build --release
./target/release/cleecode              # opens the current directory (or resumes the last workspace)
./target/release/cleecode src/main.rs  # opens the current directory with a file pre-opened
./target/release/cleecode ./some-dir   # opens that directory as the project root
```

Launching with a file or folder argument skips the startup splash and goes straight in;
the splash only shows on a bare `cleecode` (and any key dismisses it early).

### Nerd Font icons

The file tree's per-file-type icons need a [Nerd Font](https://www.nerdfonts.com/) to
render as icons instead of blank boxes. CleeCode bundles one (JetBrainsMono Nerd Font
Mono, SIL OFL) and can install it for you:

```bash
./target/release/cleecode --install-font
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

Cross-platform: builds and runs on macOS, Linux and Windows. System-clipboard access goes
through [arboard](https://github.com/1Password/arboard) (native clipboard on each OS),
process inspection for the `scp`-on-drop and Run features uses
[sysinfo](https://github.com/GuillaumeGomez/sysinfo), and config/font paths resolve via
[dirs](https://github.com/dirs-dev/directories-rs); the UI itself is plain
[ratatui](https://github.com/ratatui/ratatui)/[crossterm](https://github.com/crossterm-rs/crossterm).
The terminal pane launches `$SHELL` (falling back to `/bin/bash`) on Unix and `%ComSpec%`
(`cmd.exe`) on Windows. Developed primarily on macOS.

## Status

Personal project, actively evolving.
