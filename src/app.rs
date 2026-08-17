use crate::clipboard::Clipboard;
use crate::dnd;
use crate::editor::Editor;
use crate::file_tree::{Activation, FileTree};
use crate::highlight::Highlighter;
use crate::i18n::{self, Key, Lang};
use crate::menu::{ContextMenu, ContextTarget, MenuAction, MenuBar};
use crate::settings::{self, Settings};
use crate::terminal_panel::{key_to_bytes, TerminalPanel, TerminalWindow};
use crate::ui;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const SPLASH_DURATION: Duration = Duration::from_millis(1800);
const DOUBLE_CLICK_THRESHOLD: Duration = Duration::from_millis(400);

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum Focus {
    FileTree,
    Editor,
    Terminal,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum EditorPane {
    Left,
    Right,
}

/// One row of the run-target drop-down.
pub struct RunRow {
    pub label: String,
    /// Full path or command line, shown dimmed after the label when the label alone is
    /// ambiguous.
    pub detail: Option<String>,
    /// Whether this is what Run would use right now, marked in the list.
    pub active: bool,
    pub action: RunRowAction,
}

/// Extensions whose run command is a python interpreter, and which therefore get the venv
/// selector. Everything else runs through its `run_commands` entry alone, where a venv would
/// mean nothing — `apply_venv` has always refused to touch a non-python program.
pub fn is_python_ext(ext: &str) -> bool {
    matches!(ext, "py" | "pyw")
}

/// The run-target drop-down's rows for a file of extension `ext`. Python files get the venv
/// list — "no venv" first, then every available venv, then browse/register — because for them
/// "which interpreter" is a choice; every extension, python included, then gets the row that
/// edits its run command. So the button answers one question for every file type: what will
/// Run use here.
///
/// A free function so the index-to-action mapping a click relies on can be tested without
/// standing up an App (which would need real ptys).
fn run_rows(
    ext: &str,
    active: Option<&str>,
    available: &[String],
    registered: &[settings::RegisteredVenv],
    run_commands: &std::collections::HashMap<String, String>,
    project_commands: &std::collections::HashMap<String, String>,
    lang: Lang,
) -> Vec<RunRow> {
    let mut rows = Vec::new();
    if is_python_ext(ext) {
        rows.push(RunRow {
            label: i18n::t(lang, Key::ToolbarVenvNone).to_string(),
            detail: None,
            active: active.is_none(),
            action: RunRowAction::SelectVenv(None),
        });
        for venv in available {
            let label = ui::venv_display_name(venv, registered);
            rows.push(RunRow {
                // The full path, dimmed, so two venvs with the same folder name stay tellable
                // apart — but not when it would just repeat the label, as it does for the plain
                // project-root venvs.
                detail: (*venv != label).then(|| venv.clone()),
                label,
                active: active == Some(venv.as_str()),
                action: RunRowAction::SelectVenv(Some(venv.clone())),
            });
        }
        rows.push(RunRow {
            label: i18n::t(lang, Key::VenvBrowseItem).to_string(),
            detail: None,
            active: false,
            action: RunRowAction::Browse,
        });
        rows.push(RunRow {
            label: i18n::t(lang, Key::VenvRegisterItem).to_string(),
            detail: None,
            active: false,
            action: RunRowAction::Register,
        });
    }
    // Two places a command can live, always both offered, with the marker on whichever one Run
    // would actually use. That marker is the whole point of showing them together: "which of
    // these two wins" is otherwise invisible, and getting it wrong is how you compile the wrong
    // master file and blame the editor.
    let global = run_commands.get(ext);
    let project = project_commands.get(ext);
    rows.push(RunRow {
        label: match global {
            Some(_) => i18n::msg_run_command_row(lang, ext),
            None => i18n::msg_run_command_unset_row(lang, ext),
        },
        // The command itself, so the button's one-word label ("octave") can be checked
        // against what will actually be typed at the shell.
        detail: global.cloned(),
        active: global.is_some() && project.is_none(),
        action: RunRowAction::EditCommand(RunScope::Global),
    });
    rows.push(RunRow {
        label: i18n::msg_run_command_project_row(lang),
        detail: project.cloned(),
        active: project.is_some(),
        action: RunRowAction::EditCommand(RunScope::Project),
    });
    rows
}

/// Which file a run command is written to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RunScope {
    /// settings.toml: every project, unless one of them overrides it.
    Global,
    /// .cleecode.toml in the project root: this project alone, and shareable with it.
    Project,
}

pub enum RunRowAction {
    /// Use this venv, or the system python for `None`.
    SelectVenv(Option<String>),
    /// Browse the disk for a venv folder, rather than typing its path by hand.
    Browse,
    /// Open the box that registers a venv from elsewhere on disk, path typed by hand.
    Register,
    /// Type the run command for the extension the menu was opened on, into one of the two
    /// files that can hold one.
    EditCommand(RunScope),
}

/// The run-target drop-down while it is open.
pub struct RunMenu {
    /// Which pane's toolbar button it hangs from.
    pub pane: EditorPane,
    /// The extension the rows were built for, frozen when the menu opened. Rebuilding them
    /// from the live active file would let a row change meaning under the pointer.
    pub ext: String,
    pub selected: usize,
}

/// Which field the terminal name/startup-command box is typing into.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum TerminalField {
    Name,
    Startup,
}

impl TerminalField {
    fn other(self) -> Self {
        match self {
            TerminalField::Name => TerminalField::Startup,
            TerminalField::Startup => TerminalField::Name,
        }
    }
}

/// Registering a venv asks for two things in turn: where it is, then what to call it.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum VenvRegisterStep {
    Path,
    Nickname,
}

impl EditorPane {
    /// Index into per-pane state kept side by side, such as the tab strip's scroll offset.
    pub fn index(self) -> usize {
        match self {
            EditorPane::Left => 0,
            EditorPane::Right => 1,
        }
    }
}

/// A pending action being held back by the unsaved-changes prompt, so it can be carried
/// out (or abandoned) once the user decides what to do with the dirty buffer(s).
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum UnsavedPrompt {
    Quit,
    CloseTab(usize),
}

/// The turtle from the splash tagline, out for a walk along the status line. Clicking the logo
/// in the menu bar sets it off; clicking again while it walks hurries it, which is the joke —
/// the tagline has been saying "chi va piano va lontano" since the first launch, and this is
/// where it finally answers back.
pub struct Turtle {
    started: Instant,
    /// Columns of extra ground granted by impatient clicking.
    nudged: u16,
    /// How often it has been hurried. It replies once, then lets it go.
    hurried: u8,
    /// Whatever the status line was saying before, put back when the walk is over. A joke that
    /// leaves your status line occupied afterwards has outstayed its welcome.
    displaced: Option<String>,
}

/// How long a crossing takes when nobody hurries it. Slow on purpose: it should be something
/// you notice out of the corner of your eye while you carry on working, not a thing to watch.
const TURTLE_CROSSING: Duration = Duration::from_secs(14);
/// Ground one impatient click buys.
const TURTLE_NUDGE: u16 = 4;
/// Hurryings before the tagline answers back.
const TURTLE_PATIENCE: u8 = 3;

/// Which column the turtle has reached, or `None` once it has walked off the end and the walk
/// is over. Pure so the pace can be checked without running a terminal.
///
/// It walks right to left because that is the way the glyph faces — a turtle strolling backwards
/// across the screen is a worse joke than no turtle. Starting on the right also keeps it away
/// from the status message, which is written from the left, for most of the crossing.
pub fn turtle_column(elapsed: Duration, nudged: u16, width: u16) -> Option<u16> {
    if width == 0 {
        return None;
    }
    let progress = elapsed.as_secs_f32() / TURTLE_CROSSING.as_secs_f32();
    let walked = (progress * width as f32) as u32 + nudged as u32;
    // The glyph is two cells wide, so it leaves the screen when its left edge passes the start.
    (walked + 2 <= width as u32).then(|| (width as u32 - 2 - walked) as u16)
}

pub struct App {
    pub root: PathBuf,
    pub file_tree: FileTree,
    pub editors: Vec<Editor>,
    pub active_editor: usize,
    pub split_view: bool,
    pub active_editor_right: usize,
    pub editor_pane_focus: EditorPane,
    /// Leftmost tab rendered in each pane's strip (indexed by `EditorPane::index`), so a long
    /// list of open files scrolls horizontally instead of running off the edge. Rendering
    /// still pulls the active tab back into view, so this is a starting point, not a promise.
    /// Which buffers each half of the editor has open, in strip order. The two lists are
    /// disjoint and together cover every buffer: a split is two independent editors sharing one
    /// pool, not two windows onto one list, so no file is ever in both strips at once.
    ///
    /// Only `tabs[0]` is used while the split is closed; opening the split moves a tab across,
    /// closing it pours the right list back onto the end of the left one.
    pub tabs: [Vec<usize>; 2],
    pub tab_offsets: [usize; 2],
    /// The active tab each pane's offset was last reconciled against, so the strip is only
    /// scrolled to reveal the active tab when that tab actually changes.
    tab_revealed: [Option<usize>; 2],
    /// Terminal windows: each is a tiled pane in the layout, holding one or more tabbed shells.
    pub terminals: Vec<TerminalWindow>,
    /// Index of the focused window within `terminals`.
    pub active_terminal: usize,
    pub focus: Focus,
    pub should_quit: bool,
    pub status_message: String,
    pub editor_viewport: (usize, usize),
    /// Where the pointer last was. Only used to light up the scrollbar it is resting on, which
    /// is the one bit of the interface that has to react to the mouse merely being somewhere.
    pointer: Option<(u16, u16)>,
    pub settings: Settings,
    pub show_settings: bool,
    pub settings_selected: usize,
    pub highlighter: Highlighter,
    pub menu: MenuBar,
    /// Right-click / Ctrl+Shift+G pop-up, when open.
    pub context_menu: Option<ContextMenu>,
    /// Name + startup-command box for the focused terminal tab/window, when open.
    pub show_terminal_rename: bool,
    pub terminal_rename_input: String,
    pub terminal_startup_input: String,
    pub terminal_rename_field: TerminalField,
    /// Name box for saving the current set-up as a workspace, when open.
    pub show_workspace_save: bool,
    pub workspace_save_input: String,
    /// The named workspace in use, if any: what Save overwrites by default, and what gets
    /// written back on exit so a session's layout changes aren't lost.
    pub active_workspace: Option<String>,
    /// The built-in manual (Help ▸ Manual / F1), when open.
    pub manual: Option<crate::manual::ManualState>,
    /// The last full frame rect seen at draw time, so keyboard-opened pop-ups (which don't get
    /// passed the layout) can still anchor themselves against the current geometry.
    pub last_full: Rect,
    pub show_about: bool,
    pub clipboard: Clipboard,
    pub show_splash: bool,
    /// The easter egg, while it is walking. See `Turtle`.
    pub turtle: Option<Turtle>,
    pub splash_started: Instant,
    pub show_delete_confirm: bool,
    pub delete_target: Option<PathBuf>,
    pub show_rename: bool,
    pub rename_target: Option<PathBuf>,
    pub rename_input: String,
    /// The run-target drop-down, while it is open under its toolbar button.
    pub run_menu: Option<RunMenu>,
    /// Which step the "register a venv" box is on, when it's open.
    pub venv_register: Option<VenvRegisterStep>,
    pub venv_register_input: String,
    /// The path accepted in step one, waiting for its nickname in step two.
    venv_register_path: Option<PathBuf>,
    /// The extension whose run command is being typed and where it will be written, while
    /// that box is open.
    pub run_command_edit: Option<(String, RunScope)>,
    pub run_command_input: String,
    /// Save As box, for a buffer that has never been written to disk.
    pub show_save_as: bool,
    pub save_as_input: String,
    /// Which buffer is being named, and the action that was waiting on the save (quitting, or
    /// closing the tab) so it can go ahead once the file exists.
    save_as_target: Option<usize>,
    save_as_then: Option<UnsavedPrompt>,
    /// When set, an unsaved-changes prompt is up, holding back the given action.
    pub unsaved_prompt: Option<UnsavedPrompt>,
    /// In-file find / find-and-replace overlay state, when open.
    pub find: Option<crate::find::FindState>,
    /// Command palette / file quick-open overlay, when open.
    pub picker: Option<crate::picker::Picker>,
    /// Go-to-line prompt state.
    pub show_goto: bool,
    pub goto_input: String,
    /// New file/folder prompt state (created in the tree's selected directory).
    pub show_new_entry: bool,
    pub new_entry_is_dir: bool,
    pub new_entry_input: String,
    pub resize_mode: bool,
    pub dragging: Option<DragTarget>,
    pub available_venvs: Vec<String>,
    /// What the open project says about itself, from its `.cleecode.toml`. Reloaded whenever
    /// the root changes, since it belongs to the folder rather than to the session.
    pub project_settings: settings::ProjectSettings,
    last_tree_click: Option<(usize, Instant)>,
    pub git_status: std::collections::HashMap<PathBuf, crate::git_status::FileStatus>,
    git_status_tx: Sender<std::collections::HashMap<PathBuf, crate::git_status::FileStatus>>,
    git_status_rx: Receiver<std::collections::HashMap<PathBuf, crate::git_status::FileStatus>>,
    git_status_pending: Arc<AtomicBool>,
    bg_tx: Sender<String>,
    bg_rx: Receiver<String>,
    /// Pictures being decoded off the main thread, answering by path: a tab's index can change
    /// while a decode is still running, but the file it was asked about cannot.
    preview_tx: Sender<crate::preview::Decoded>,
    preview_rx: Receiver<crate::preview::Decoded>,
}

/// Computes git status on a background thread so a slow (or merely process-spawn-heavy)
/// `git status` never blocks the render loop — most visibly, never delays the very first
/// frame (and thus the embedded terminals) from appearing at startup. `pending` bounds
/// this to at most one in-flight computation, so a `git status` slower than the poll
/// interval that drives refreshes doesn't pile up background threads.
fn spawn_git_status_refresh(
    root: PathBuf,
    tx: Sender<std::collections::HashMap<PathBuf, crate::git_status::FileStatus>>,
    pending: Arc<AtomicBool>,
) {
    if pending.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let result = crate::git_status::compute(&root);
        let _ = tx.send(result);
        pending.store(false, Ordering::SeqCst);
    });
}

/// Name of the directory holding a virtualenv's executables: `Scripts` on Windows,
/// `bin` everywhere else. Keeps venv discovery and the interpreter swap portable.
pub fn venv_bin_dir() -> &'static str {
    if cfg!(windows) {
        "Scripts"
    } else {
        "bin"
    }
}

/// Top-level subdirectories of `root` that look like Python virtualenvs (they carry an
/// `activate` script in their bin/Scripts dir). Non-recursive: only scans the project
/// root itself.
/// The venv that would actually be used: the remembered one, but only while it is among those
/// available here. `active_venv` is global, so a project without that venv would otherwise show
/// it in the toolbar while runs silently used system python.
pub fn effective_venv<'a>(active: Option<&'a str>, available: &[String]) -> Option<&'a str> {
    let active = active?;
    available.iter().any(|v| v == active).then_some(active)
}

/// What makes a directory a virtualenv: an activate script in its executables directory.
/// Shared by auto-discovery and by the box that registers one by hand, so both agree on what
/// counts — a path that merely exists is not a venv.
pub fn is_venv_dir(path: &std::path::Path) -> bool {
    path.is_dir() && path.join(venv_bin_dir()).join("activate").exists()
}

fn discover_venvs(root: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut venvs: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| is_venv_dir(p))
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .collect();
    venvs.sort();
    venvs
}

/// The venvs offered by the selector: those auto-discovered in `root`, plus every
/// still-existing user-registered absolute path (deduplicated, registered ones last).
fn available_venvs(root: &std::path::Path, registered: &[crate::settings::RegisteredVenv]) -> Vec<String> {
    let mut venvs = discover_venvs(root);
    for r in registered {
        let path = r.path().to_string();
        if std::path::Path::new(&path).is_dir() && !venvs.contains(&path) {
            venvs.push(path);
        }
    }
    venvs
}

/// Orders version-bearing directory names numerically, so `Octave-10.1.0` ranks above
/// `Octave-9.2.0` — plain string ordering would pick the older one.
fn version_key(name: &str) -> Vec<u64> {
    name.split(|c: char| !c.is_ascii_digit())
        .filter_map(|s| s.parse().ok())
        .collect()
}

/// Finds the Octave console binary under a Windows `Program Files` directory, where it sits
/// in a versioned folder (`GNU Octave\Octave-9.2.0\mingw64\bin\octave-cli.exe`) that the
/// installer does not add to PATH — so a bare `octave-cli` would not resolve. Picks the
/// newest install. Returns `None` when `program_files` is absent (i.e. off Windows, where
/// Octave comes from a package manager and is already on PATH).
fn discover_octave(program_files: Option<&std::path::Path>) -> Option<PathBuf> {
    let base = program_files?.join("GNU Octave");
    let mut installs: Vec<PathBuf> = std::fs::read_dir(&base)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    installs.sort_by_key(|p| version_key(&p.file_name().unwrap_or_default().to_string_lossy()));
    installs.reverse();
    installs.into_iter().find_map(|install| {
        // Layout differs across releases: newer installs nest the toolchain under mingw64/.
        ["mingw64", "."]
            .iter()
            .map(|mid| install.join(mid).join("bin").join("octave-cli.exe"))
            .find(|exe| exe.exists())
    })
}

/// Swaps a run command's program for an explicitly configured absolute path
/// (`interpreter_paths` in settings.toml), so interpreters installed outside PATH still run.
/// Falls back to auto-detecting Octave, the case where this bites by default on Windows.
/// The template is returned untouched when nothing applies.
fn resolve_interpreter(
    template: &str,
    interpreter_paths: &std::collections::HashMap<String, String>,
    program_files: Option<&std::path::Path>,
) -> String {
    let (program, rest) = template.split_once(' ').unwrap_or((template, ""));
    let resolved = interpreter_paths
        .get(program)
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .or_else(|| {
            matches!(program, "octave" | "octave-cli")
                .then(|| discover_octave(program_files))
                .flatten()
        });
    let Some(path) = resolved else { return template.to_string() };
    let quoted = shell_words::quote(&path.to_string_lossy()).into_owned();
    if rest.is_empty() { quoted } else { format!("{quoted} {rest}") }
}

/// Drops buffer `idx` from both strips and renumbers what is left, since removing it shifts
/// every later buffer down one.
///
/// A free function so the renumbering can be tested on its own. It is the part of closing a tab
/// where an off-by-one does not show up as a wrong tab but as a stale index surviving into a
/// later draw, which in an app that hosts live shells is the expensive kind of bug.
fn forget_buffer(tabs: &mut [Vec<usize>; 2], idx: usize) {
    for strip in tabs.iter_mut() {
        strip.retain(|&i| i != idx);
        for i in strip.iter_mut() {
            if *i > idx {
                *i -= 1;
            }
        }
    }
}

/// A path's extension, lowercased — the key everything about running a file is looked up by.
fn file_ext(path: &std::path::Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Fills a run command's placeholders from the file being run, each shell-quoted so paths with
/// spaces survive: `{file}` is the whole path, `{dir}` its folder, `{name}` the file name,
/// `{stem}` that name without its extension.
///
/// `{file}` alone only ever covers "interpreter, then script". The rest are what let a command
/// work where the file is rather than where the shell is (LaTeX's `-output-directory`) or name
/// its own output (`{dir}/{stem}.pdf`). Nothing else is needed to chain steps: the command is
/// typed at a real shell, so `&&` works as it reads.
fn expand_placeholders(template: &str, path: &std::path::Path) -> String {
    let quote = |s: std::borrow::Cow<'_, str>| shell_words::quote(&s).into_owned();
    // A bare relative path has an empty parent, which as a directory means "here".
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    template
        .replace("{file}", &quote(path.to_string_lossy()))
        .replace("{dir}", &quote(dir.to_string_lossy()))
        .replace("{name}", &quote(path.file_name().unwrap_or_default().to_string_lossy()))
        .replace("{stem}", &quote(path.file_stem().unwrap_or_default().to_string_lossy()))
}

/// Collects files under `root` for the quick-open picker, capped so a huge tree can't
/// stall the UI. Always skips VCS/build dirs; skips dotfiles unless `show_hidden`.
fn collect_project_files(root: &std::path::Path, out: &mut Vec<PathBuf>, show_hidden: bool) {
    const LIMIT: usize = 8000;
    if out.len() >= LIMIT {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == ".git" || name == "target" || name == "node_modules" {
            continue;
        }
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_project_files(&path, out, show_hidden);
        } else if path.is_file() {
            out.push(path);
        }
        if out.len() >= LIMIT {
            return;
        }
    }
}

/// Recursively copies `src` to `dest` (file, directory tree, or symlink target),
/// replacing `cp -R` so drag-and-drop copies work identically on every platform.
fn copy_recursive(src: &std::path::Path, dest: &std::path::Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(src)?;
    if meta.is_dir() {
        std::fs::create_dir_all(dest)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dest.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dest)?;
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DragTarget {
    Sidebar,
    TerminalHeight,
    /// Dragging the vertical seam between the two editor panes in split view.
    EditorSplit,
    /// Dragging the seam between terminal window `i` and window `i + 1` to redistribute their
    /// space (horizontally when tiled side by side, vertically when stacked).
    TerminalSplit(usize),
    TextSelection,
    /// Selecting text inside an embedded terminal, in the pane the drag started in.
    TerminalSelection(usize),
    /// Dragging a scrollbar's thumb along its track.
    Scrollbar(ScrollbarId),
}

/// Which scrollbar a click, a drag or the pointer is on. A frame plus an axis is enough to name
/// one, and both halves are cheap to compare, so nothing about a bar has to be remembered
/// between frames.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScrollbarId {
    Editor(EditorPane, ui::Axis),
    /// The vertical bar of terminal window `i`. Terminals have no horizontal one: the pty is
    /// sized to its pane, so output wraps rather than running off the side.
    Terminal(usize),
}

/// Which part of a scrollbar a point falls on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScrollbarPart {
    /// An end arrow: one line back, or one line on. The mouse's answer to an arrow key.
    Step(isize),
    /// The groove, `offset` cells along a track `len` cells long. Clicking jumps there and
    /// dragging keeps following, which is the same gesture either way.
    Track { offset: u16, len: u16 },
}

impl DragTarget {
    /// Whether this drag is resizing a layout seam (as opposed to selecting text), so the
    /// focused frame can show its resize highlight while the drag is under way.
    fn is_layout(self) -> bool {
        matches!(
            self,
            DragTarget::Sidebar
                | DragTarget::TerminalHeight
                | DragTarget::EditorSplit
                | DragTarget::TerminalSplit(_)
        )
    }
}

/// A side of the focused frame, named by the arrow key that selects it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ResizeSide {
    Left,
    Right,
    Up,
    Down,
}

/// The layout scalar a resize nudge moves, with a signed step already folded in (grow/shrink and
/// terminal orientation accounted for). Applied by the caller, then clamped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ResizeCmd {
    /// Delta in columns for `sidebar_width`.
    Sidebar(i16),
    /// Delta in percent for `terminal_pct`.
    Terminal(i16),
    /// Delta in percent for `split_pct` (the left pane's share).
    Split(i16),
    /// The seam between terminal windows `seam` and `seam + 1`: `delta` is added to the first
    /// window's weight and taken from the second, so only the pair resizes.
    TerminalWeight { seam: usize, delta: i16 },
}

/// The layout facts a resize nudge depends on, gathered so the resolver stays a pure, testable
/// function rather than reaching into `App`.
pub struct ResizeLayout {
    pub focus: Focus,
    pub editor_pane: EditorPane,
    pub split_view: bool,
    pub show_sidebar: bool,
    pub show_terminal: bool,
    pub terminal_on_right: bool,
    /// Which terminal window has focus, and how many there are — the seams between them are
    /// movable too, and until now only with the mouse.
    pub terminal_index: usize,
    pub terminal_count: usize,
}

/// Where a directional move lands. A "frame" for this purpose is finer-grained than `Focus`:
/// the two halves of a split editor and each tiled terminal window are places you can be, and
/// an arrow should reach them the same way it reaches the sidebar.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum FocusTarget {
    Tree,
    Editor(EditorPane),
    Terminal(usize),
}

/// The frame that lies in the given direction, or `None` at the edge of the window.
///
/// Navigation is spatial rather than by category: you press the direction the thing you want is
/// in, and it does not matter whether that thing is a file tree, an editor pane or a shell. The
/// layout has only two arrangements, so the whole map is small:
///
/// ```text
///   terminals below (classic)        terminals on the right
///   ┌──────┬──────────────┐          ┌──────┬─────────┬──────┐
///   │ tree │ editor       │          │ tree │ editor  │ term │
///   ├──────┴──────────────┤          │      │         ├──────┤
///   │ term │ term         │          │      │         │ term │
///   └──────┴──────────────┘          └──────┴─────────┴──────┘
///   windows side by side             windows stacked
/// ```
///
/// The terminal strip spans the full width in the classic layout, which is why its windows are
/// walked with left/right there and with up/down when the panel is a column instead.
pub fn focus_neighbour(l: &ResizeLayout, side: ResizeSide) -> Option<FocusTarget> {
    use ResizeSide::*;
    let last_window = l.terminal_count.checked_sub(1)?;
    match l.focus {
        Focus::FileTree => match side {
            Right => Some(FocusTarget::Editor(EditorPane::Left)),
            Down if l.show_terminal && !l.terminal_on_right => Some(FocusTarget::Terminal(0)),
            _ => None,
        },
        Focus::Editor => match side {
            Left if l.split_view && l.editor_pane == EditorPane::Right => {
                Some(FocusTarget::Editor(EditorPane::Left))
            }
            Left if l.show_sidebar => Some(FocusTarget::Tree),
            Right if l.split_view && l.editor_pane == EditorPane::Left => {
                Some(FocusTarget::Editor(EditorPane::Right))
            }
            Right if l.show_terminal && l.terminal_on_right => Some(FocusTarget::Terminal(0)),
            Down if l.show_terminal && !l.terminal_on_right => Some(FocusTarget::Terminal(0)),
            _ => None,
        },
        Focus::Terminal => {
            // Along the axis the windows tile on, the arrows walk between them; across it, they
            // leave the panel for whatever is next to it.
            let (prev, next) = if l.terminal_on_right { (Up, Down) } else { (Left, Right) };
            let leave = if l.terminal_on_right { Left } else { Up };
            let back_to_editor =
                Some(FocusTarget::Editor(if l.split_view { EditorPane::Right } else { EditorPane::Left }));
            match side {
                s if s == prev => l.terminal_index.checked_sub(1).map(FocusTarget::Terminal),
                s if s == next => (l.terminal_index < last_window).then_some(FocusTarget::Terminal(l.terminal_index + 1)),
                s if s == leave => back_to_editor,
                _ => None,
            }
        }
    }
}

const SIDEBAR_STEP: i16 = 2;
const TERMINAL_STEP: i16 = 5;
const SPLIT_STEP: i16 = 5;
/// A tenth of the default weight: ten nudges take a window from its share to a neighbour's.
const WEIGHT_STEP: i16 = 100;

/// Resolves an arrow nudge on the focused frame to the seam it moves. `None` when the named
/// border coincides with the window edge — there is nothing there to drag. `grow` pushes the
/// border outward (the frame gets bigger); `!grow` pulls it inward.
///
/// The whole layout has only three movable seams — sidebar↔editor, editor↔terminal, and (in
/// split view) editor-left↔editor-right — so every frame has at most two of them, always on
/// sides that the arrow keys can tell apart.
pub fn resize_command(l: &ResizeLayout, side: ResizeSide, grow: bool) -> Option<ResizeCmd> {
    let s: i16 = if grow { 1 } else { -1 };
    use ResizeSide::*;
    match l.focus {
        Focus::FileTree => match side {
            // The sidebar's right edge is the sidebar↔editor seam; growing widens the sidebar.
            Right => Some(ResizeCmd::Sidebar(s * SIDEBAR_STEP)),
            // Its bottom edge only meets a seam when the terminal is a full-width strip below.
            Down if l.show_terminal && !l.terminal_on_right => Some(ResizeCmd::Terminal(-s * TERMINAL_STEP)),
            _ => None,
        },
        Focus::Terminal => {
            if !l.show_terminal {
                return None;
            }
            // Windows tile across the panel: side by side when it is a strip below the editor,
            // stacked when it is a column beside it. Along that axis the focused window's
            // borders are seams with its neighbours; across it, with the editor.
            let along_axis = if l.terminal_on_right {
                matches!(side, Up | Down)
            } else {
                matches!(side, Left | Right)
            };
            if along_axis {
                let toward_next = matches!(side, Right | Down);
                // The seam is named by the window on its left/top, so which one that is
                // depends on the direction — and which way its weight has to move for the
                // *focused* window to grow.
                let seam = if toward_next { l.terminal_index } else { l.terminal_index.checked_sub(1)? };
                if seam + 1 >= l.terminal_count {
                    return None;
                }
                let delta = if toward_next { s * WEIGHT_STEP } else { -s * WEIGHT_STEP };
                return Some(ResizeCmd::TerminalWeight { seam, delta });
            }
            // The terminal touches the editor on exactly one side, set by its orientation.
            match (l.terminal_on_right, side) {
                (true, Left) => Some(ResizeCmd::Terminal(s * TERMINAL_STEP)),
                (false, Up) => Some(ResizeCmd::Terminal(s * TERMINAL_STEP)),
                _ => None,
            }
        }
        Focus::Editor => {
            // Which seams the focused editor region touches depends on whether it is split, and
            // on which pane holds focus.
            let (sidebar_left, split_left, split_right, terminal_far) = if l.split_view {
                match l.editor_pane {
                    // Left pane: sidebar on its left, the split seam on its right.
                    EditorPane::Left => (true, false, true, false),
                    // Right pane: the split seam on its left, the terminal on its far side.
                    EditorPane::Right => (false, true, false, true),
                }
            } else {
                // Unsplit: from the sidebar seam on the left to the terminal seam on the far side.
                (true, false, false, true)
            };
            match side {
                Left if sidebar_left && l.show_sidebar => Some(ResizeCmd::Sidebar(-s * SIDEBAR_STEP)),
                Left if split_left => Some(ResizeCmd::Split(-s * SPLIT_STEP)),
                Right if split_right => Some(ResizeCmd::Split(s * SPLIT_STEP)),
                Right if terminal_far && l.show_terminal && l.terminal_on_right => {
                    Some(ResizeCmd::Terminal(-s * TERMINAL_STEP))
                }
                Down if terminal_far && l.show_terminal && !l.terminal_on_right => {
                    Some(ResizeCmd::Terminal(-s * TERMINAL_STEP))
                }
                _ => None,
            }
        }
    }
}

/// Adds a signed delta to a layout scalar without wrapping; `clamp_layout` then bounds it.
fn nudge_u16(v: u16, delta: i16) -> u16 {
    (v as i32 + delta as i32).clamp(0, u16::MAX as i32) as u16
}

#[derive(Clone, Copy)]
pub struct LayoutPreset {
    pub show_sidebar: bool,
    pub show_terminal: bool,
    pub sidebar_width: u16,
    pub terminal_pct: u16,
    pub terminal_on_right: bool,
}

pub const PRESET_CLASSIC: LayoutPreset = LayoutPreset {
    show_sidebar: true,
    show_terminal: true,
    sidebar_width: 30,
    terminal_pct: 35,
    terminal_on_right: false,
};

pub const PRESET_WIDE: LayoutPreset = LayoutPreset {
    show_sidebar: false,
    show_terminal: true,
    sidebar_width: 30,
    terminal_pct: 45,
    terminal_on_right: true,
};

pub const PRESET_TRIPLE: LayoutPreset = LayoutPreset {
    show_sidebar: true,
    show_terminal: true,
    sidebar_width: 26,
    terminal_pct: 35,
    terminal_on_right: true,
};

fn within(r: Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
}

/// Reads a quick-open query as a filesystem path, returning the directory to list and the
/// fragment to filter its entries by. `None` when the query is an ordinary project-file search:
/// only a leading `/`, `~`, `./` or `../` means "browse the disk", so typing a plain name still
/// searches the project.
///
/// A query ending in a separator lists that directory whole; otherwise the last component is
/// treated as what the user is partway through typing.
fn path_query(query: &str, root: &std::path::Path, home: Option<&std::path::Path>) -> Option<(PathBuf, String)> {
    let trimmed = query.trim_start();
    let base: PathBuf = if let Some(rest) = trimmed.strip_prefix("~/") {
        home?.join(rest)
    } else if trimmed == "~" || trimmed == "~/" {
        home?.to_path_buf()
    } else if trimmed.starts_with('/') {
        PathBuf::from(trimmed)
    } else if trimmed.starts_with("./") || trimmed.starts_with("../") {
        root.join(trimmed)
    } else {
        return None;
    };

    // A trailing separator means the whole thing is the directory to list.
    if trimmed.ends_with('/') || trimmed == "~" {
        return Some((base, String::new()));
    }
    let fragment = base.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let dir = base.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("/"));
    Some((dir, fragment))
}

/// Rows for the venv browser: the sub-directories of `dir` only — a file can never be a venv —
/// each flagged when it is itself a venv, so Enter's meaning (register vs. descend) is visible.
/// Hidden folders are always included, since the commonest venv of all, `.venv`, is one. A free
/// function so the listing can be tested without standing up an App.
fn venv_browse_items(dir: &std::path::Path) -> Vec<crate::picker::PickItem> {
    list_dir_entries(dir, true)
        .into_iter()
        .filter(|p| p.is_dir())
        .map(|path| {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            crate::picker::PickItem {
                label: format!("{name}/"),
                shortcut: is_venv_dir(&path).then(|| "venv".to_string()),
                action: crate::picker::PickAction::VenvDir(path),
            }
        })
        .collect()
}

/// Directory entries for the quick-open browser: directories first, then files, each
/// alphabetically, with dotfiles omitted unless `show_hidden`. An unreadable directory yields
/// nothing rather than an error, since the user may still be typing its name.
fn list_dir_entries(dir: &std::path::Path, show_hidden: bool) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            show_hidden
                || !p.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(false)
        })
        .collect();
    paths.sort_by_key(|p| {
        let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        (!p.is_dir(), name)
    });
    paths
}

/// Turns what was typed in the Save As box into a path. A bare name or a relative path hangs
/// off the project root, an absolute one is taken as it is, and `~` is expanded — the box is
/// typed by hand, so a home-relative path is a reasonable thing to write. `None` for a name
/// that is only whitespace.
fn resolve_save_as_path(input: &str, root: &std::path::Path, home: Option<&std::path::Path>) -> Option<PathBuf> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let expanded = match trimmed.strip_prefix("~/").or(trimmed.strip_prefix("~\\")) {
        Some(rest) => home?.join(rest),
        None if trimmed == "~" => home?.to_path_buf(),
        None => PathBuf::from(trimmed),
    };
    Some(if expanded.is_absolute() { expanded } else { root.join(expanded) })
}

/// Screen cell `(row, col)` under a mouse position, relative to a pane's inner area. Positions
/// outside the area are clamped to its edges, so a drag that wanders off the pane keeps
/// selecting up to the border instead of being dropped.
/// Whether `key` is a particular Ctrl+Shift+<letter>. Each overlay closes on the chord that
/// opened it, so the two have to agree; naming the letter here keeps that pairing readable at
/// both ends instead of spelling out the modifier test four times.
fn is_ctrl_shift(key: KeyEvent, letter: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && key.modifiers.contains(KeyModifiers::SHIFT)
        && matches!(key.code, KeyCode::Char(c) if c.eq_ignore_ascii_case(&letter))
}

fn cell_at(inner: Rect, col: u16, row: u16) -> Option<(u16, u16)> {
    if inner.width == 0 || inner.height == 0 {
        return None;
    }
    let clamp = |v: u16, min: u16, len: u16| v.clamp(min, min + len - 1) - min;
    Some((clamp(row, inner.y, inner.height), clamp(col, inner.x, inner.width)))
}

impl App {
    pub fn new(root: PathBuf, term_rows: u16, term_cols: u16) -> Result<Self> {
        let half_cols = (term_cols / 2).max(10);
        // Loaded before the first shell is spawned: vt100 fixes a terminal's scrollback length
        // at construction, so a shell started ahead of the preference would be stuck without one
        // for the whole session.
        let settings = Settings::load();
        crate::terminal_panel::set_scrollback_len(settings.terminal_scrollback);
        // Two windows side by side to start, each with a single tab — the familiar two-pane view.
        let t1 = TerminalWindow::new(term_rows, half_cols, &root)?;
        let t2 = TerminalWindow::new(term_rows, half_cols, &root)?;
        let (bg_tx, bg_rx) = mpsc::channel();
        let (preview_tx, preview_rx) = mpsc::channel();
        let (git_status_tx, git_status_rx) = mpsc::channel();
        let git_status_pending = Arc::new(AtomicBool::new(false));
        spawn_git_status_refresh(root.clone(), git_status_tx.clone(), git_status_pending.clone());
        let available_venvs = available_venvs(&root, &settings.registered_venvs);
        let project_settings = settings::ProjectSettings::load(&root);
        let file_tree = FileTree::new(root.clone(), settings.show_hidden_files);
        Ok(App {
            file_tree,
            root,
            editors: vec![Editor::empty()],
            active_editor: 0,
            split_view: false,
            active_editor_right: 0,
            editor_pane_focus: EditorPane::Left,
            tabs: [vec![0], Vec::new()],
            tab_offsets: [0, 0],
            tab_revealed: [None, None],
            terminals: vec![t1, t2],
            active_terminal: 0,
            context_menu: None,
            show_terminal_rename: false,
            terminal_rename_input: String::new(),
            terminal_startup_input: String::new(),
            terminal_rename_field: TerminalField::Name,
            show_workspace_save: false,
            workspace_save_input: String::new(),
            active_workspace: None,
            manual: None,
            last_full: Rect::new(0, 0, 0, 0),
            focus: Focus::FileTree,
            should_quit: false,
            status_message: i18n::t(Lang::default(), Key::StatusHelp).to_string(),
            editor_viewport: (0, 0),
            pointer: None,
            settings,
            show_settings: false,
            settings_selected: 0,
            highlighter: Highlighter::new(),
            menu: MenuBar::new(),
            show_about: false,
            clipboard: Clipboard::new(),
            show_splash: true,
            turtle: None,
            splash_started: Instant::now(),
            show_delete_confirm: false,
            delete_target: None,
            show_rename: false,
            rename_target: None,
            rename_input: String::new(),
            run_menu: None,
            venv_register: None,
            venv_register_input: String::new(),
            venv_register_path: None,
            run_command_edit: None,
            run_command_input: String::new(),
            show_save_as: false,
            save_as_input: String::new(),
            save_as_target: None,
            save_as_then: None,
            unsaved_prompt: None,
            find: None,
            picker: None,
            show_goto: false,
            goto_input: String::new(),
            show_new_entry: false,
            new_entry_is_dir: false,
            new_entry_input: String::new(),
            resize_mode: false,
            dragging: None,
            available_venvs,
            project_settings,
            last_tree_click: None,
            git_status: std::collections::HashMap::new(),
            git_status_tx,
            git_status_rx,
            git_status_pending,
            bg_tx,
            bg_rx,
            preview_tx,
            preview_rx,
        })
    }

    /// A click on the logo. The first sets the turtle off; the rest hurry it along, and once
    /// it has been hurried enough the status line quotes the tagline back at you.
    fn poke_turtle(&mut self) {
        let mut scold = false;
        if let Some(t) = self.turtle.as_mut() {
            t.nudged = t.nudged.saturating_add(TURTLE_NUDGE);
            t.hurried = t.hurried.saturating_add(1);
            scold = t.hurried == TURTLE_PATIENCE;
        } else {
            self.turtle = Some(Turtle { started: Instant::now(), nudged: 0, hurried: 0, displaced: None });
        }
        if scold {
            let was = std::mem::replace(
                &mut self.status_message,
                i18n::t(self.settings.lang, Key::SplashTagline).to_string(),
            );
            if let Some(t) = self.turtle.as_mut() {
                t.displaced = Some(was);
            }
        }
    }

    /// Where the turtle is right now, for the status line to draw. `None` when it is not out.
    pub fn turtle_at(&self, width: u16) -> Option<u16> {
        let t = self.turtle.as_ref()?;
        turtle_column(t.started.elapsed(), t.nudged, width)
    }

    pub fn poll_turtle(&mut self) {
        let width = self.last_full.width;
        if self.turtle.is_some() && self.turtle_at(width).is_none() {
            // Hand the status line back exactly as it was found — but only if the reply is still
            // the thing on it. Anything the user has done since has more right to be there.
            let tagline = i18n::t(self.settings.lang, Key::SplashTagline);
            if let Some(displaced) = self.turtle.as_mut().and_then(|t| t.displaced.take()) {
                if self.status_message == tagline {
                    self.status_message = displaced;
                }
            }
            self.turtle = None;
        }
    }

    pub fn poll_splash(&mut self) {
        if self.show_splash && self.splash_started.elapsed() >= SPLASH_DURATION {
            self.show_splash = false;
        }
    }

    pub fn poll_background_messages(&mut self) {
        while let Ok(msg) = self.bg_rx.try_recv() {
            self.status_message = msg;
        }
    }

    /// Hands decoded pictures to the tabs that asked for them. Matched by path rather than by
    /// index: tabs can be closed or reordered while a decode is in flight, and a stale index
    /// would put a picture in somebody else's tab.
    pub fn poll_previews(&mut self) {
        while let Ok(done) = self.preview_rx.try_recv() {
            let Some(preview) = self.editors.iter_mut().find_map(|e| {
                e.preview.as_mut().filter(|_| e.path.as_deref() == Some(done.path.as_path()))
            }) else {
                continue;
            };
            // A reply for a page that is no longer the one being looked at: the reader paged on
            // while it was rendering, and putting it up would yank them back a page.
            if done.page != preview.page() {
                continue;
            }
            if let (Some(pages), Some(total)) = (preview.pages.as_mut(), done.total) {
                pages.total = Some(total);
            }
            let rendered_view = preview.source.clone();
            match done.result {
                Ok(image) => preview.state = crate::preview::State::ready(image),
                // A document that could not be made is not the end of a markdown preview: it
                // still has styled text to offer, which is better than a red line where the
                // document should be. The reason is said once, in the status line, and the tab
                // stops trying until Refresh asks it to.
                Err(message) if rendered_view.is_some() => {
                    preview.document_failed = true;
                    preview.shown_revision = u64::MAX;
                    self.status_message = i18n::msg_preview_failed(self.settings.lang, &message);
                }
                Err(message) => preview.state = crate::preview::State::Failed(message),
            }
        }
    }

    /// The focused window's on-screen tab, if any.
    pub fn focused_panel(&self) -> Option<&TerminalPanel> {
        self.terminals.get(self.active_terminal).map(|w| w.active_tab())
    }

    pub fn focused_panel_mut(&mut self) -> Option<&mut TerminalPanel> {
        self.terminals.get_mut(self.active_terminal).map(|w| w.active_tab_mut())
    }

    /// Window `i`'s on-screen tab — the one a click or run targets.
    fn window_tab(&self, i: usize) -> Option<&TerminalPanel> {
        self.terminals.get(i).map(|w| w.active_tab())
    }

    fn window_tab_mut(&mut self, i: usize) -> Option<&mut TerminalPanel> {
        self.terminals.get_mut(i).map(|w| w.active_tab_mut())
    }

    pub fn poll_terminal_exits(&mut self) {
        // A workspace's startup commands are typed here rather than at spawn time, once each
        // shell is actually at a prompt.
        for window in &mut self.terminals {
            for tab in &mut window.tabs {
                tab.flush_pending();
            }
        }
        let before: usize = self.terminals.len();
        // Reap exited tabs within each window, then drop any window left with no tabs.
        for window in &mut self.terminals {
            window.reap_exited();
        }
        self.terminals.retain(|w| !w.tabs.is_empty());
        // Never leave the workspace with no terminal at all.
        if self.terminals.is_empty() {
            if let Ok(w) = TerminalWindow::new(24, 80, &self.root) {
                self.terminals.push(w);
            }
        }
        if self.terminals.len() != before {
            self.active_terminal = self.active_terminal.min(self.terminals.len().saturating_sub(1));
        }
    }

    pub fn handle_paste(&mut self, text: String) {
        if self.show_splash {
            self.show_splash = false;
            return;
        }
        if self.show_about || self.show_settings || self.menu.active {
            return;
        }
        match self.focus {
            Focus::FileTree => {
                let paths = dnd::parse_dropped_paths(&text);
                if !paths.is_empty() {
                    self.copy_dropped_paths(paths);
                }
            }
            Focus::Editor => self.editor_mut().insert_multiline(&text),
            Focus::Terminal => self.handle_terminal_paste(&text),
        }
    }

    fn handle_terminal_paste(&mut self, text: &str) {
        let paths = dnd::parse_dropped_paths(text);
        let ssh_target = if paths.is_empty() {
            None
        } else {
            self.focused_panel()
                .and_then(|t| t.child_pid())
                .and_then(dnd::detect_ssh_target)
        };
        if let Some(target) = ssh_target {
            self.scp_paths_background(target, paths);
        } else if let Some(term) = self.focused_panel_mut() {
            term.write_input(text.as_bytes());
        }
    }

    fn copy_dropped_paths(&mut self, paths: Vec<PathBuf>) {
        let lang = self.settings.lang;
        let dest_dir = self.file_tree.selected_dir();
        let mut ok = 0usize;
        let mut last_err = None;
        for path in &paths {
            let Some(file_name) = path.file_name() else { continue };
            let dest = dest_dir.join(file_name);
            match copy_recursive(path, &dest) {
                Ok(()) => ok += 1,
                Err(e) => last_err = Some(e.to_string()),
            }
        }
        if ok > 0 {
            self.file_tree = FileTree::new(self.root.clone(), self.settings.show_hidden_files);
        }
        let dest_display = dest_dir.display().to_string();
        self.status_message = match last_err {
            Some(err) if ok == 0 => i18n::msg_copy_failed(lang, &dest_display, &err),
            _ => i18n::msg_copied_files(lang, ok, &dest_display),
        };
    }

    fn scp_paths_background(&mut self, target: String, paths: Vec<PathBuf>) {
        let lang = self.settings.lang;
        self.status_message = i18n::msg_scp_started(lang, paths.len(), &target);
        let tx = self.bg_tx.clone();
        std::thread::spawn(move || {
            let mut ok = 0usize;
            let mut failed = 0usize;
            for path in &paths {
                let dest = format!("{target}:~/");
                let status = std::process::Command::new("scp")
                    .args(["-r", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10"])
                    .arg(path)
                    .arg(&dest)
                    .status();
                match status {
                    Ok(s) if s.success() => ok += 1,
                    _ => failed += 1,
                }
            }
            let _ = tx.send(i18n::msg_scp_result(lang, ok, failed, &target));
        });
    }

    /// Which editor the keyboard (and anything else routed through `editor()`/
    /// `editor_mut()`) currently acts on: the right pane's active tab when split and
    /// focused there, the left/only pane's otherwise.
    /// Both panes are clamped, not just the right one. `editor()` is indexed on every frame and
    /// every keystroke, so an index that has fallen behind the tab list is not a wrong buffer —
    /// it is a panic in the hottest path in the program, which used to close CleeCode outright.
    /// Showing the last tab is a far better answer than that.
    fn active_editor_index(&self) -> usize {
        let last = self.editors.len().saturating_sub(1);
        if self.split_view && self.editor_pane_focus == EditorPane::Right {
            self.active_editor_right.min(last)
        } else {
            self.active_editor.min(last)
        }
    }

    pub fn editor(&self) -> &Editor {
        &self.editors[self.active_editor_index()]
    }

    pub fn editor_mut(&mut self) -> &mut Editor {
        let idx = self.active_editor_index();
        &mut self.editors[idx]
    }

    pub fn toggle_split_view(&mut self) {
        self.split_view = !self.split_view;
        if self.split_view {
            self.open_split();
        } else {
            self.close_split();
        }
    }

    pub fn poll_external_changes(&mut self) {
        let lang = self.settings.lang;
        if let Some(msg) = self.editor_mut().check_external_changes(lang) {
            self.status_message = msg;
        }
        self.reload_changed_previews();
        self.file_tree.refresh();
        spawn_git_status_refresh(self.root.clone(), self.git_status_tx.clone(), self.git_status_pending.clone());
    }

    pub fn poll_git_status(&mut self) {
        while let Ok(status) = self.git_status_rx.try_recv() {
            self.git_status = status;
        }
    }

    pub fn open_file_in_tab(&mut self, path: PathBuf) {
        let lang = self.settings.lang;
        // A file with nothing in it to edit is shown rather than opened.
        //
        // Opening a PNG used to give a blank read-only tab — the binary guard had already
        // emptied the buffer — and left you to work out that ▶ Run was the way to see it.
        // Handled here rather than at the double click, so the tree's Enter and the quick-open
        // take the same route: one way in, one behaviour.
        let ext = file_ext(&path);
        if crate::preview::is_previewable(&ext) || crate::preview::is_document(&ext) {
            self.open_preview_tab(path, crate::preview::is_document(&ext));
            return;
        }
        // Not a picture, but still not text, and we know a command that displays it: run that.
        // A PDF with a viewer configured lands here, as does anything else somebody has taught
        // the run commands about.
        if Editor::looks_binary(&path) && self.run_command_for(&ext).is_some() {
            // It lands in a terminal, so there had better be one on screen. Opening a file and
            // having it appear nowhere visible would be worse than the blank tab this replaces
            // — at least that was on screen.
            self.settings.show_terminal = true;
            self.run_path(&path);
            return;
        }
        if let Some(idx) = self.editors.iter().position(|e| e.path.as_deref() == Some(path.as_path())) {
            self.focus_existing_tab(idx);
            self.status_message = i18n::msg_opened(lang, &self.editors[idx].title(lang));
            return;
        }
        match Editor::open(path) {
            Ok(editor) => {
                // Into the focused pane, leaving the other one on whatever it was showing.
                let idx = self.adopt_editor(editor);
                self.place_in_pane(self.editor_pane_focus, idx);
                self.focus = Focus::Editor;
                self.status_message = if self.editor().is_read_only() {
                    i18n::msg_opened_read_only(lang, &self.editor().title(lang))
                } else {
                    i18n::msg_opened(lang, &self.editor().title(lang))
                };
            }
            Err(e) => self.status_message = i18n::msg_open_error(lang, &e.to_string()),
        }
    }

    /// Puts a new tab on screen, taking over the untouched scratch buffer when that is all
    /// there is rather than leaving an empty tab beside the real one.
    /// Returns where the tab landed without deciding which pane looks at it. Deliberately: a
    /// caller that also chose a pane would otherwise be pointing *both* panes at the new tab —
    /// one here and one of its own — and a split whose halves show the same file has stopped
    /// being a split.
    fn adopt_editor(&mut self, editor: Editor) -> usize {
        if self.editors.len() == 1 && self.editors[0].path.is_none() && !self.editors[0].dirty {
            self.editors[0] = editor;
            0
        } else {
            self.editors.push(editor);
            self.editors.len() - 1
        }
    }

    /// Opens a picture in its own tab, decoding it on a background thread. A photo takes long
    /// enough to decode that doing it here would stall the window, terminals included.
    ///
    /// In split view it opens in the pane that *isn't* focused, so the file being worked on
    /// stays visible beside it. The layout itself is never changed: it is yours to shape, and
    /// an opened file rearranging your frames would be a surprise every time it was not wanted.
    fn open_preview_tab(&mut self, path: PathBuf, paged: bool) {
        let lang = self.settings.lang;
        if let Some(idx) = self.editors.iter().position(|e| e.path.as_deref() == Some(path.as_path())) {
            self.focus_existing_tab(idx);
            self.status_message = i18n::msg_opened(lang, &self.editors[idx].title(lang));
            return;
        }
        let first = paged.then_some(1);
        let mut preview =
            if paged { crate::preview::Preview::document(1) } else { crate::preview::Preview::picture() };
        preview.state = crate::preview::start_loading(
            match first {
                Some(page) => crate::preview::Job::Page { path: path.clone(), page },
                None => crate::preview::Job::Picture(path.clone()),
            },
            self.preview_tx.clone(),
        );
        let idx = self.adopt_editor(Editor::preview(path, preview));
        // A preview goes to the half you are *not* working in when there is one, so the file it
        // belongs to stays in front of you.
        let pane = if self.split_view && self.editor_pane_focus == EditorPane::Left {
            EditorPane::Right
        } else {
            self.editor_pane_focus
        };
        self.place_in_pane(pane, idx);
        self.focus = Focus::Editor;
        self.status_message = i18n::msg_preview_opened(lang, crate::preview::protocol_name());
    }

    /// Moves a document preview a page. The first page is the near end; the far one announces
    /// itself by the render failing, since no page count is needed to reach it and none may be
    /// available. The page number changes straight away so the label and the arrows keep up
    /// even while the picture is still being made.
    fn turn_page(&mut self, delta: isize) {
        let idx = self.pane_editor_index(self.editor_pane_focus);
        let path = match self.editors.get(idx).and_then(|e| e.path.clone()) {
            Some(path) => path,
            None => return,
        };
        let Some(preview) = self.editors[idx].preview.as_mut() else { return };
        let Some(pages) = preview.pages.as_mut() else { return };
        let wanted = (pages.current as isize + delta).max(1) as usize;
        // Past a known last page there is nothing to ask for, so the key simply does nothing
        // rather than flashing an error at the end of every document.
        if pages.total.is_some_and(|total| wanted > total) || wanted == pages.current {
            return;
        }
        pages.current = wanted;
        // A markdown preview has no PDF on disk to page through: its document is made from the
        // buffer, so another page means making it again. Asking a rasteriser for page two of a
        // .md file is what the previous version did, and it failed exactly as it deserved to.
        let rendered_from = preview.source.clone();
        let job = match rendered_from {
            Some(source) => {
                let text = self
                    .editors
                    .iter()
                    .find(|e| e.preview.is_none() && e.path.as_deref() == Some(source.as_path()))
                    .map(|e| e.rope.to_string())
                    .unwrap_or_default();
                crate::preview::Job::Markdown { path: source, text, page: wanted }
            }
            None => crate::preview::Job::Page { path, page: wanted },
        };
        let started = crate::preview::start_loading(job, self.preview_tx.clone());
        if let Some(preview) = self.editors[idx].preview.as_mut() {
            // The page on screen stays up while the next one is made, rather than blanking.
            if !matches!(preview.state, crate::preview::State::Rendered { .. }) {
                preview.state = started;
            }
        }
    }

    /// Re-renders a preview whose file changed under it. This is the LaTeX loop: edit the
    /// source, press Run, and the page beside it becomes the page that was just typeset —
    /// without closing and reopening the tab to notice.
    fn reload_changed_previews(&mut self) {
        let stale: Vec<(usize, PathBuf, Option<usize>)> = self
            .editors
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let preview = e.preview.as_ref()?;
                let path = e.path.clone()?;
                let mtime = std::fs::metadata(&path).ok()?.modified().ok()?;
                (Some(mtime) != e.disk_mtime).then_some((i, path, preview.page()))
            })
            .collect();
        for (i, path, page) in stale {
            self.editors[i].disk_mtime = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());
            if let Some(preview) = self.editors[i].preview.as_mut() {
                preview.state = crate::preview::start_loading(
                    match page {
                        Some(page) => crate::preview::Job::Page { path: path.clone(), page },
                        None => crate::preview::Job::Picture(path.clone()),
                    },
                    self.preview_tx.clone(),
                );
            }
        }
    }

    /// Brings tab `idx` on screen, in the pane that keeps the most work visible: the unfocused
    /// half when the editor is split, the only pane otherwise.
    /// The buffers a pane has open, in the order its strip shows them. With the split closed
    /// the right pane has none, and everything belongs to the left one.
    /// How many lines a rendered preview drew, or `None` when the tab is not one. Scrolling a
    /// preview is scrolling those lines; there is no rope behind it to measure instead.
    fn rendered_len(&self, idx: usize) -> Option<usize> {
        match &self.editors.get(idx)?.preview.as_ref()?.state {
            crate::preview::State::Rendered { lines, .. } => Some(lines.len()),
            _ => None,
        }
    }

    pub fn pane_tabs(&self, pane: EditorPane) -> &[usize] {
        &self.tabs[pane.index()]
    }

    /// Where the pane's current buffer sits in its own strip. Clamped rather than optional: the
    /// strip is drawn every frame, and a pane briefly out of step should show its first tab, not
    /// take the window down.
    pub fn pane_tab_position(&self, pane: EditorPane) -> usize {
        let wanted = self.pane_editor_index(pane);
        self.tabs[pane.index()].iter().position(|&i| i == wanted).unwrap_or(0)
    }

    /// Which pane holds a buffer, if any. The lists are disjoint, so there is at most one.
    fn pane_holding(&self, editor: usize) -> Option<EditorPane> {
        [EditorPane::Left, EditorPane::Right]
            .into_iter()
            .find(|&pane| self.tabs[pane.index()].contains(&editor))
    }

    /// Puts a buffer in a pane and shows it there, taking it away from the other pane if that is
    /// where it was. Moving rather than copying is what keeps the two strips disjoint, which is
    /// the whole of "a split does not duplicate tabs".
    fn place_in_pane(&mut self, pane: EditorPane, editor: usize) {
        for other in [EditorPane::Left, EditorPane::Right] {
            if other != pane {
                self.tabs[other.index()].retain(|&i| i != editor);
            }
        }
        if !self.tabs[pane.index()].contains(&editor) {
            self.tabs[pane.index()].push(editor);
        }
        self.set_pane_editor(pane, editor);
        // A pane that just gave up the buffer it was showing has to land somewhere real.
        self.settle_panes();
    }

    /// Points every pane at a buffer it actually holds, and makes sure no pane is left with an
    /// empty strip. Called after anything that moves buffers between panes or removes one, so
    /// that the invariants are restored in one place instead of at each call site.
    fn settle_panes(&mut self) {
        // The split closed: the left pane owns everything, and nothing is drawn from the right.
        if !self.split_view {
            return;
        }
        for pane in [EditorPane::Left, EditorPane::Right] {
            if self.tabs[pane.index()].is_empty() {
                // An editor with no tabs is not a thing this app can draw, so it gets a fresh
                // empty buffer — the same one a brand-new window starts with.
                self.editors.push(Editor::empty());
                let idx = self.editors.len() - 1;
                self.tabs[pane.index()].push(idx);
            }
            let held = self.tabs[pane.index()].clone();
            if !held.contains(&self.pane_editor_index(pane)) {
                self.set_pane_editor(pane, held[0]);
            }
        }
    }

    /// Splits the editor in two. The right half takes one tab from the left — the one after the
    /// current file, since that is the one you were most likely reaching for — leaving the rest
    /// where they are. Nothing is copied: with only one tab open the right half gets a new empty
    /// buffer rather than a second view of the same file.
    fn open_split(&mut self) {
        let left = &self.tabs[0];
        let taken = left
            .iter()
            .position(|&i| i == self.active_editor)
            .and_then(|pos| left.get(pos + 1).or_else(|| left.first().filter(|_| left.len() > 1)))
            .copied();
        match taken {
            Some(idx) => self.place_in_pane(EditorPane::Right, idx),
            None => self.settle_panes(),
        }
    }

    /// Closes the split, pouring the right half's tabs onto the end of the left half's. Nothing
    /// is thrown away: the buffers were only ever parked in two lists, and closing the split is
    /// about the frames, not about what is open.
    fn close_split(&mut self) {
        let right = std::mem::take(&mut self.tabs[1]);
        let showing = self.active_editor_index();
        for idx in right {
            if !self.tabs[0].contains(&idx) {
                self.tabs[0].push(idx);
            }
        }
        // Whichever half you were reading stays on screen, so closing the split never also
        // changes the file in front of you — unless that half is the one that just emptied,
        // in which case its index means nothing and the merged strip decides.
        let showing = if self.tabs[0].contains(&showing) {
            showing
        } else {
            self.tabs[0].first().copied().unwrap_or(0)
        };
        self.active_editor = showing;
        self.active_editor_right = showing;
        self.editor_pane_focus = EditorPane::Left;
    }

    /// Brings a tab that is already open to the front.
    ///
    /// When a pane is already showing it, that pane is simply focused rather than the other one
    /// being moved onto it too. Pointing both halves of a split at the same file is the one
    /// thing a split must never do — it stops being a split — and "open something already on
    /// screen" is precisely when it would otherwise happen.
    fn focus_existing_tab(&mut self, idx: usize) {
        self.focus = Focus::Editor;
        // Already open somewhere: go to it rather than dragging it into this half. Moving it
        // would take it out of the strip the reader put it in, which is not what "open" means.
        if let Some(pane) = self.pane_holding(idx) {
            self.editor_pane_focus = pane;
            self.set_pane_editor(pane, idx);
            return;
        }
        self.place_in_pane(self.editor_pane_focus, idx);
    }

    /// Points one pane at a tab, leaving the other one alone. The one place either index is
    /// assigned from an "open this" path, so no caller can set both by accident.
    fn set_pane_editor(&mut self, pane: EditorPane, idx: usize) {
        match pane {
            EditorPane::Left => self.active_editor = idx,
            EditorPane::Right => self.active_editor_right = idx,
        }
    }

    fn copy_selection(&mut self) {
        if let Some(text) = self.editor().selected_text() {
            let count = text.chars().count();
            self.clipboard.set(&text);
            self.status_message = i18n::msg_copied(self.settings.lang, count);
        }
    }

    fn cut_selection(&mut self) {
        if let Some(text) = self.editor().selected_text() {
            let count = text.chars().count();
            self.clipboard.set(&text);
            self.editor_mut().delete_selection();
            self.status_message = i18n::msg_cut(self.settings.lang, count);
        }
    }

    fn paste_clipboard(&mut self) {
        let text = self.clipboard.get();
        if !text.is_empty() {
            self.editor_mut().insert_multiline(&text);
        }
    }

    fn select_all(&mut self) {
        let ed = self.editor_mut();
        ed.selection_anchor = Some((0, 0));
        let last_line = ed.rope.len_lines().saturating_sub(1);
        ed.cursor_line = last_line;
        ed.cursor_col = ed.line_char_len(last_line);
    }

    fn close_active_editor(&mut self) {
        let idx = self.active_editor;
        // Guard against silently dropping unsaved edits: prompt first if the tab is dirty.
        if self.editors.get(idx).map(|e| e.dirty).unwrap_or(false) {
            self.unsaved_prompt = Some(UnsavedPrompt::CloseTab(idx));
        } else {
            self.close_editor_at(idx);
        }
    }

    /// Quit request from Ctrl+Q or the menu. Holds back the quit behind a prompt if any
    /// buffer has unsaved changes, so exiting never silently discards work.
    fn request_quit(&mut self) {
        if self.editors.iter().any(|e| e.dirty) {
            self.unsaved_prompt = Some(UnsavedPrompt::Quit);
        } else {
            self.should_quit = true;
        }
    }

    /// Keys for the unsaved-changes prompt: `s` saves then proceeds, `y`/Enter discards and
    /// proceeds, anything else cancels.
    fn handle_unsaved_prompt_key(&mut self, key: KeyEvent) {
        let Some(action) = self.unsaved_prompt else { return };
        match key.code {
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.unsaved_prompt = None;
                // An unnamed buffer can't be written yet: ask for a name and let the quit or
                // the tab close resume from there. Going ahead would throw the work away,
                // which is exactly what the prompt exists to prevent.
                let unnamed = match action {
                    UnsavedPrompt::Quit => self.editors.iter().position(|e| e.dirty && e.path.is_none()),
                    UnsavedPrompt::CloseTab(idx) => {
                        Some(idx).filter(|&i| self.editors.get(i).is_some_and(|e| e.path.is_none()))
                    }
                };
                if let Some(idx) = unnamed {
                    self.begin_save_as(idx, Some(action));
                    return;
                }
                match action {
                    UnsavedPrompt::Quit => self.save_all(),
                    UnsavedPrompt::CloseTab(idx) => {
                        if let Some(ed) = self.editors.get_mut(idx) {
                            let _ = ed.save();
                        }
                    }
                }
                self.perform_unsaved_action(action);
            }
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.unsaved_prompt = None;
                self.perform_unsaved_action(action);
            }
            _ => {
                self.unsaved_prompt = None;
            }
        }
    }

    fn perform_unsaved_action(&mut self, action: UnsavedPrompt) {
        match action {
            UnsavedPrompt::Quit => self.should_quit = true,
            UnsavedPrompt::CloseTab(idx) => self.close_editor_at(idx),
        }
    }

    // ---- Find / replace -------------------------------------------------------------

    fn editor_cursor_char_idx(&self) -> usize {
        let ed = self.editor();
        ed.rope.line_to_char(ed.cursor_line) + ed.cursor_col
    }

    fn open_find(&mut self, _replace: bool) {
        let mut fs = crate::find::FindState::new();
        // Seed the query from a single-line selection, the way most editors do.
        if let Some(sel) = self.editor().selected_text() {
            if !sel.is_empty() && !sel.contains('\n') {
                fs.query = sel;
            }
        }
        let text = self.editor().rope.to_string();
        let from = self.editor_cursor_char_idx();
        fs.recompute(&text, from);
        self.find = Some(fs);
        self.apply_find_selection();
    }

    /// Recomputes matches after the query changed, biasing the current match to the cursor.
    fn recompute_find(&mut self) {
        let text = self.editor().rope.to_string();
        let from = self.editor_cursor_char_idx();
        if let Some(f) = self.find.as_mut() {
            f.recompute(&text, from);
        }
        self.apply_find_selection();
    }

    /// Selects the current match so it's visible via the normal selection highlight.
    fn apply_find_selection(&mut self) {
        let Some(m) = self.find.as_ref().and_then(|f| f.current_match()) else { return };
        self.editor_mut().select_char_range(m.0, m.1);
    }

    fn replace_current(&mut self) {
        let (Some(m), replace) = (
            self.find.as_ref().and_then(|f| f.current_match()),
            self.find.as_ref().map(|f| f.replace.clone()).unwrap_or_default(),
        ) else {
            return;
        };
        self.editor_mut().replace_char_range(m.0, m.1, &replace);
        // Matches shifted; recompute and land on the next one from the edit point.
        self.recompute_find();
    }

    fn replace_all(&mut self) {
        let Some(f) = self.find.as_ref() else { return };
        if f.query.is_empty() || f.matches.is_empty() {
            return;
        }
        let replace = f.replace.clone();
        // Replace from the last match backwards so earlier char indices stay valid.
        let matches: Vec<(usize, usize)> = f.matches.clone();
        let count = matches.len();
        for &(s, e) in matches.iter().rev() {
            self.editor_mut().replace_char_range(s, e, &replace);
        }
        let lang = self.settings.lang;
        self.status_message = i18n::msg_replaced_all(lang, count);
        self.recompute_find();
    }

    fn handle_find_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                self.find = None;
                self.editor_mut().clear_selection();
            }
            KeyCode::Char('a') if ctrl => self.replace_all(),
            KeyCode::Char('r') if ctrl => self.replace_current(),
            KeyCode::Enter | KeyCode::Down => {
                if let Some(f) = self.find.as_mut() {
                    f.next();
                }
                self.apply_find_selection();
            }
            KeyCode::Up => {
                if let Some(f) = self.find.as_mut() {
                    f.prev();
                }
                self.apply_find_selection();
            }
            KeyCode::Tab => {
                if let Some(f) = self.find.as_mut() {
                    f.focus_replace = !f.focus_replace;
                }
            }
            KeyCode::Backspace => {
                if let Some(f) = self.find.as_mut() {
                    if f.focus_replace {
                        f.replace.pop();
                    } else {
                        f.query.pop();
                    }
                }
                self.recompute_find();
            }
            KeyCode::Char(c) if !ctrl => {
                let mut changed_query = false;
                if let Some(f) = self.find.as_mut() {
                    if f.focus_replace {
                        f.replace.push(c);
                    } else {
                        f.query.push(c);
                        changed_query = true;
                    }
                }
                if changed_query {
                    self.recompute_find();
                }
            }
            _ => {}
        }
    }

    // ---- Go to line -----------------------------------------------------------------

    fn open_goto(&mut self) {
        self.show_goto = true;
        self.goto_input.clear();
    }

    fn handle_goto_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.show_goto = false;
            }
            KeyCode::Enter => {
                if let Ok(line) = self.goto_input.trim().parse::<usize>() {
                    if line > 0 {
                        self.editor_mut().goto_line(line);
                    }
                }
                self.show_goto = false;
            }
            KeyCode::Backspace => {
                self.goto_input.pop();
            }
            KeyCode::Char(c) if c.is_ascii_digit() => self.goto_input.push(c),
            _ => {}
        }
    }

    // ---- New file / folder ----------------------------------------------------------

    fn open_new_entry(&mut self, is_dir: bool) {
        self.show_new_entry = true;
        self.new_entry_is_dir = is_dir;
        self.new_entry_input.clear();
    }

    fn handle_new_entry_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.show_new_entry = false;
            }
            KeyCode::Enter => {
                self.confirm_new_entry();
                self.show_new_entry = false;
            }
            KeyCode::Backspace => {
                self.new_entry_input.pop();
            }
            KeyCode::Char(c) => self.new_entry_input.push(c),
            _ => {}
        }
    }

    fn confirm_new_entry(&mut self) {
        let lang = self.settings.lang;
        let name = self.new_entry_input.trim();
        if name.is_empty() {
            return;
        }
        let dest = self.file_tree.selected_dir().join(name);
        let is_dir = self.new_entry_is_dir;
        let result = if is_dir {
            std::fs::create_dir_all(&dest)
        } else {
            if let Some(parent) = dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // Don't clobber an existing file.
            if dest.exists() {
                Ok(())
            } else {
                std::fs::write(&dest, "")
            }
        };
        match result {
            Ok(()) => {
                self.file_tree = FileTree::new(self.root.clone(), self.settings.show_hidden_files);
                self.status_message = i18n::msg_created_entry(lang, &dest.display().to_string());
                if !is_dir {
                    self.open_file_in_tab(dest);
                }
            }
            Err(e) => self.status_message = i18n::msg_create_entry_error(lang, &e.to_string()),
        }
    }

    // ---- Command palette / file quick-open ------------------------------------------

    /// Every action in the app, menu bar and context menus alike — see `menu::command_entries`.
    /// Built from that one list so an action can't be offered by a right-click and nowhere else.
    fn open_command_palette(&mut self) {
        let lang = self.settings.lang;
        let items: Vec<crate::picker::PickItem> = crate::menu::command_entries()
            .into_iter()
            .map(|(group_key, it)| crate::picker::PickItem {
                label: format!("{}: {}", i18n::t(lang, group_key), i18n::t(lang, it.label_key)),
                shortcut: it.shortcut.map(|s| s.to_string()),
                action: crate::picker::PickAction::Command(it.action),
            })
            .collect();
        self.picker = Some(crate::picker::Picker::new("Command palette", crate::picker::PickerKind::Commands, items));
    }

    fn open_file_picker(&mut self) {
        let items = self.project_file_items();
        self.picker =
            Some(crate::picker::Picker::new("Open file (type / or ~ to browse)", crate::picker::PickerKind::Files, items));
    }

    /// Every file under the project root, the quick-open default: type a few characters to jump
    /// to one without walking the tree.
    fn project_file_items(&self) -> Vec<crate::picker::PickItem> {
        let mut files = Vec::new();
        collect_project_files(&self.root, &mut files, self.settings.show_hidden_files);
        files.sort();
        let root = self.root.clone();
        files
            .into_iter()
            .map(|p| {
                let label = p.strip_prefix(&root).unwrap_or(&p).to_string_lossy().to_string();
                crate::picker::PickItem { label, shortcut: None, action: crate::picker::PickAction::OpenFile(p) }
            })
            .collect()
    }

    /// Keeps the file picker's list in step with what has been typed. A query starting with `/`,
    /// `~`, `./` or `../` browses the disk — the project-file list can only ever offer what is
    /// under the root, which is why opening anything outside it used to be impossible from here.
    /// Rebuilds whichever picker is open as its query changes. The command palette is a fixed
    /// list, so it needs no rebuild.
    fn refresh_picker(&mut self) {
        match self.picker.as_ref().map(|p| p.kind) {
            Some(crate::picker::PickerKind::Files) => self.refresh_file_picker(),
            Some(crate::picker::PickerKind::VenvBrowse) => self.refresh_venv_browser(),
            _ => {}
        }
    }

    fn refresh_file_picker(&mut self) {
        let Some(query) = self
            .picker
            .as_ref()
            .filter(|p| p.kind == crate::picker::PickerKind::Files)
            .map(|p| p.query.clone())
        else {
            return;
        };
        let home = dirs::home_dir();
        match path_query(&query, &self.root, home.as_deref()) {
            Some((dir, fragment)) => {
                let show_hidden = self.settings.show_hidden_files;
                let items: Vec<crate::picker::PickItem> = list_dir_entries(&dir, show_hidden)
                    .into_iter()
                    .map(|path| {
                        let is_dir = path.is_dir();
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        crate::picker::PickItem {
                            // The trailing slash is the only cue that Enter will descend rather
                            // than open.
                            label: if is_dir { format!("{name}/") } else { name },
                            shortcut: None,
                            action: crate::picker::PickAction::OpenFile(path),
                        }
                    })
                    .collect();
                if let Some(picker) = self.picker.as_mut() {
                    picker.path_mode = true;
                    picker.filter_override = Some(fragment);
                    picker.set_items(items);
                }
            }
            None => {
                // Back from browsing to searching the project. Rebuilt only on the transition,
                // since walking the tree on every keystroke would be wasteful.
                if self.picker.as_ref().is_some_and(|p| p.path_mode) {
                    let items = self.project_file_items();
                    if let Some(picker) = self.picker.as_mut() {
                        picker.path_mode = false;
                        picker.filter_override = None;
                        picker.set_items(items);
                    }
                }
            }
        }
    }

    fn handle_picker_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.picker = None,
            KeyCode::Up => {
                if let Some(p) = self.picker.as_mut() {
                    p.move_selection(-1);
                }
            }
            KeyCode::Down => {
                if let Some(p) = self.picker.as_mut() {
                    p.move_selection(1);
                }
            }
            KeyCode::Enter => self.execute_picker_selection(),
            KeyCode::Backspace => {
                if let Some(p) = self.picker.as_mut() {
                    p.pop_char();
                }
                self.refresh_picker();
            }
            KeyCode::Char(c) if !ctrl => {
                if let Some(p) = self.picker.as_mut() {
                    p.push_char(c);
                }
                self.refresh_picker();
            }
            _ => {}
        }
    }

    /// A click inside the picker. On a result it takes it there and then, which is what makes a
    /// double click unnecessary; anywhere else inside is ignored so a stray click on the query
    /// line does not throw the list away; outside, it dismisses.
    fn mouse_picker(&mut self, col: u16, row: u16, full: Rect) {
        let Some(p) = self.picker.as_ref() else {
            return;
        };
        // A click runs the thing it lands on — except where that thing destroys something. The
        // delete list refolds under the pointer after each removal, so one click executing
        // immediately means a second click in the same spot takes the *next* workspace with it.
        // There it selects only, and Enter is the deliberate second step.
        let destructive = p.kind == crate::picker::PickerKind::WorkspaceDelete;
        match ui::picker_row_at(p, full, col, row) {
            Some(index) => {
                if let Some(p) = self.picker.as_mut() {
                    p.selected = index;
                }
                if !destructive {
                    self.execute_picker_selection();
                }
            }
            None => {
                let rect = ui::picker_rect(full);
                let inside = col >= rect.x
                    && col < rect.x + rect.width
                    && row >= rect.y
                    && row < rect.y + rect.height;
                if !inside {
                    self.picker = None;
                }
            }
        }
    }

    fn execute_picker_selection(&mut self) {
        let mut cmd = None;
        let mut file = None;
        let mut venv_dir = None;
        let mut workspace = None;
        if let Some(action) = self.picker.as_ref().and_then(|p| p.selected_action()) {
            match action {
                crate::picker::PickAction::Command(a) => cmd = Some(*a),
                crate::picker::PickAction::OpenFile(p) => file = Some(p.clone()),
                crate::picker::PickAction::VenvDir(p) => venv_dir = Some(p.clone()),
                crate::picker::PickAction::Workspace(name) => workspace = Some(name.clone()),
            }
        }
        if let Some(name) = workspace {
            // Which of the two workspace pickers is open decides what Enter means.
            if self.picker.as_ref().map(|p| p.kind) == Some(crate::picker::PickerKind::WorkspaceDelete) {
                self.delete_workspace(&name);
            } else {
                self.picker = None;
                let found = if crate::workspace::is_default(&name) {
                    Some(crate::workspace::default_workspace(self.root.clone()))
                } else {
                    crate::workspace::load(&name)
                };
                match found {
                    Some(ws) => self.apply_workspace(ws),
                    None => self.status_message = i18n::t(self.settings.lang, Key::MsgNoWorkspaces).to_string(),
                }
            }
        } else if let Some(a) = cmd {
            self.picker = None;
            self.run_menu_action(a);
        } else if let Some(p) = venv_dir {
            // A venv folder is the target: register it (then ask for a nickname). Any other
            // directory is just somewhere to go, so descend and keep browsing.
            if is_venv_dir(&p) {
                self.picker = None;
                self.begin_venv_nickname(p);
            } else if let Some(picker) = self.picker.as_mut() {
                picker.query = format!("{}/", p.to_string_lossy().trim_end_matches('/'));
                self.refresh_venv_browser();
            }
        } else if let Some(p) = file {
            // A directory is somewhere to go, not something to open: descend into it and keep
            // browsing, which is what makes typing a path a usable way to walk the disk.
            if p.is_dir() {
                if let Some(picker) = self.picker.as_mut() {
                    picker.query = format!("{}/", p.to_string_lossy().trim_end_matches('/'));
                }
                self.refresh_file_picker();
            } else {
                self.picker = None;
                self.open_file_in_tab(p);
            }
        } else {
            self.picker = None;
        }
    }

    /// A buffer that has never been written has nowhere to save to, so this asks for a name
    /// rather than reporting a save that didn't happen.
    fn save_active_file(&mut self) {
        if self.editor().path.is_none() {
            self.begin_save_as(self.active_editor, None);
            return;
        }
        let lang = self.settings.lang;
        match self.editor_mut().save() {
            Ok(()) => self.status_message = i18n::msg_saved(lang, &self.editor().title(lang)),
            Err(e) => self.status_message = i18n::msg_save_error(lang, &e.to_string()),
        }
    }

    fn save_all(&mut self) {
        let lang = self.settings.lang;
        let mut saved = 0usize;
        let mut unnamed = 0usize;
        let mut errors = Vec::new();
        for editor in &mut self.editors {
            if !editor.dirty {
                continue;
            }
            // Each unnamed buffer needs its own name, which a batch save can't ask for. They
            // are reported rather than skipped in silence, as they used to be.
            if editor.path.is_none() {
                unnamed += 1;
                continue;
            }
            match editor.save() {
                Ok(()) => saved += 1,
                Err(e) => errors.push(format!("{}: {}", editor.title(lang), e)),
            }
        }
        self.status_message = if !errors.is_empty() {
            i18n::msg_save_all_errors(lang, saved, &errors.join("; "))
        } else if unnamed > 0 {
            i18n::msg_saved_all_unnamed(lang, saved, unnamed)
        } else {
            i18n::msg_saved_all(lang, saved)
        };
    }

    /// Opens the Save As box for buffer `idx`. `then` carries an action that was waiting on
    /// this save so it can resume afterwards.
    fn begin_save_as(&mut self, idx: usize, then: Option<UnsavedPrompt>) {
        self.save_as_target = Some(idx);
        self.save_as_then = then;
        self.save_as_input.clear();
        self.show_save_as = true;
    }

    fn handle_save_as_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.confirm_save_as(),
            KeyCode::Esc => self.cancel_save_as(),
            KeyCode::Backspace => {
                self.save_as_input.pop();
            }
            KeyCode::Char(c) => self.save_as_input.push(c),
            _ => {}
        }
    }

    /// Dismissing the box abandons whatever was waiting on it too: a quit must not go through
    /// on a buffer the user just declined to name.
    pub fn cancel_save_as(&mut self) {
        self.show_save_as = false;
        self.save_as_input.clear();
        self.save_as_target = None;
        self.save_as_then = None;
    }

    fn confirm_save_as(&mut self) {
        let lang = self.settings.lang;
        let Some(idx) = self.save_as_target else {
            self.cancel_save_as();
            return;
        };
        let home = dirs::home_dir();
        let Some(path) = resolve_save_as_path(&self.save_as_input, &self.root, home.as_deref()) else {
            return;
        };
        // Never overwrite an existing file from a hand-typed name — one typo would destroy it.
        // The box stays open so the name can be corrected.
        if path.exists() {
            self.status_message = i18n::msg_save_as_exists(lang, &path.display().to_string());
            return;
        }
        let then = self.save_as_then;
        self.show_save_as = false;
        self.save_as_input.clear();
        self.save_as_target = None;
        self.save_as_then = None;

        let Some(editor) = self.editors.get_mut(idx) else { return };
        editor.path = Some(path.clone());
        // The name decides the language, so the buffer is re-highlighted on the next frame.
        editor.syntax_dirty = true;
        match editor.save() {
            Ok(()) => {
                self.status_message = i18n::msg_saved(lang, &path.display().to_string());
                self.file_tree.refresh();
                if let Some(then) = then {
                    self.resume_after_save_as(then);
                }
            }
            Err(e) => {
                // Leave the buffer unnamed rather than pointing it at a file that isn't there,
                // and drop the pending action so nothing is discarded.
                if let Some(editor) = self.editors.get_mut(idx) {
                    editor.path = None;
                }
                self.status_message = i18n::msg_save_error(lang, &e.to_string());
            }
        }
    }

    /// Resumes the action that was waiting on a Save As. Further unnamed dirty buffers are
    /// asked about one at a time, so quitting only proceeds once nothing is left unwritten.
    fn resume_after_save_as(&mut self, action: UnsavedPrompt) {
        if matches!(action, UnsavedPrompt::Quit) {
            if let Some(next) = self.editors.iter().position(|e| e.dirty && e.path.is_none()) {
                self.begin_save_as(next, Some(action));
                return;
            }
            self.save_all();
        }
        self.perform_unsaved_action(action);
    }

    fn close_editor_at(&mut self, idx: usize) {
        if idx >= self.editors.len() {
            return;
        }
        // Closing the only tab leaves an empty buffer rather than no buffer: the rest of the app
        // assumes there is always one to show. Assigning into slot 0 would panic on an already
        // empty list, so the list is rebuilt instead of indexed.
        if self.editors.len() <= 1 {
            self.editors.clear();
            self.editors.push(Editor::empty());
            self.tabs = [vec![0], Vec::new()];
            self.active_editor = 0;
            self.active_editor_right = 0;
            self.settle_panes();
            return;
        }
        // A rendered preview is a view of a buffer, not a copy of one: with the buffer gone it
        // has nothing left to show, so it goes with it rather than reopening the file behind
        // your back.
        let closed = self.editors[idx].path.clone();
        self.editors.remove(idx);
        forget_buffer(&mut self.tabs, idx);
        if let Some(path) = closed {
            if let Some(orphan) = self.editors.iter().position(|e| {
                e.preview.as_ref().and_then(|p| p.source.as_deref()) == Some(path.as_path())
            }) {
                self.editors.remove(orphan);
                forget_buffer(&mut self.tabs, orphan);
            }
        }
        for active in [&mut self.active_editor, &mut self.active_editor_right] {
            if *active > idx {
                *active -= 1;
            }
            *active = (*active).min(self.editors.len() - 1);
        }
        // A half left with nothing in it gives the split up rather than being handed a blank
        // buffer to justify its own existence. Closing your last tab on one side is a way of
        // saying you are done with that side.
        if self.split_view && self.tabs.iter().any(|t| t.is_empty()) {
            self.split_view = false;
            self.close_split();
        }
        self.settle_panes();
        // Closing tabs shortens the strip, so an offset past the end would blank it out.
        for (pane, offset) in self.tab_offsets.iter_mut().enumerate() {
            *offset = (*offset).min(self.tabs[pane].len().saturating_sub(1));
        }
    }

    /// Sideways through the tabs of whichever frame has focus — one key for both strips instead
    /// of one idiom per frame. The file tree has no tabs of its own, so from there it moves the
    /// editor's: the strip is the only one on screen, and doing nothing would be the stranger
    /// answer.
    fn cycle_focused_tab(&mut self, forward: bool) {
        match self.focus {
            Focus::Terminal => self.cycle_terminal_tab(forward),
            _ => self.cycle_editor(forward),
        }
    }

    /// Sideways through the focused half's own strip. In a split each half cycles its own tabs
    /// and never reaches across, which is the point of them being separate strips at all.
    fn cycle_editor(&mut self, forward: bool) {
        let pane = if self.split_view { self.editor_pane_focus } else { EditorPane::Left };
        let tabs = self.pane_tabs(pane);
        if tabs.is_empty() {
            return;
        }
        let len = tabs.len();
        let at = self.pane_tab_position(pane);
        let next = if forward { (at + 1) % len } else { (at + len - 1) % len };
        let idx = tabs[next];
        self.set_pane_editor(pane, idx);
    }

    fn set_root(&mut self, new_root: PathBuf) {
        self.file_tree = FileTree::new(new_root.clone(), self.settings.show_hidden_files);
        self.root = new_root;
        self.available_venvs = available_venvs(&self.root, &self.settings.registered_venvs);
        self.project_settings = settings::ProjectSettings::load(&self.root);
        spawn_git_status_refresh(self.root.clone(), self.git_status_tx.clone(), self.git_status_pending.clone());
        self.status_message = i18n::msg_project_folder(self.settings.lang, &self.root.display().to_string());
    }

    fn toggle_hidden_files(&mut self) {
        // The setting is the single source of truth; the tree follows it. Flipping both
        // independently let them drift apart.
        self.settings.show_hidden_files = !self.settings.show_hidden_files;
        self.file_tree.set_show_hidden(self.settings.show_hidden_files);
    }

    /// New terminal *window*: another tiled pane, focused.
    pub fn new_terminal(&mut self) {
        let lang = self.settings.lang;
        match TerminalWindow::new(24, 80, &self.root) {
            Ok(w) => {
                self.terminals.push(w);
                self.active_terminal = self.terminals.len() - 1;
                self.settings.show_terminal = true;
                self.focus = Focus::Terminal;
                self.status_message = i18n::msg_new_terminal(lang, self.terminals.len());
            }
            Err(e) => self.status_message = i18n::msg_terminal_create_error(lang, &e.to_string()),
        }
    }

    /// New terminal *tab*: another shell inside the focused window, sharing its pane. With no
    /// window open yet, there's nothing to tab into, so this opens a window instead.
    pub fn new_terminal_tab(&mut self) {
        let lang = self.settings.lang;
        if self.terminals.is_empty() {
            self.new_terminal();
            return;
        }
        match TerminalPanel::new(24, 80, &self.root) {
            Ok(panel) => {
                let window = &mut self.terminals[self.active_terminal];
                window.add_tab(panel);
                self.settings.show_terminal = true;
                self.focus = Focus::Terminal;
                self.status_message = i18n::msg_new_terminal_tab(lang, window.tabs.len());
            }
            Err(e) => self.status_message = i18n::msg_terminal_create_error(lang, &e.to_string()),
        }
    }

    /// Cycles the tabs of the focused window (Ctrl+Shift+←/→).
    pub fn cycle_terminal_tab(&mut self, forward: bool) {
        if let Some(window) = self.terminals.get_mut(self.active_terminal) {
            window.cycle_tab(forward);
        }
    }

    pub fn close_active_terminal(&mut self) {
        self.close_terminal(self.active_terminal);
    }

    /// Closes the whole terminal window at `index` — every tab in it — keeping the active index
    /// valid. The last window stays: there is always at least one terminal.
    pub fn close_terminal(&mut self, index: usize) {
        if self.terminals.len() <= 1 {
            self.status_message = i18n::msg_min_one_terminal(self.settings.lang);
            return;
        }
        if index >= self.terminals.len() {
            return;
        }
        self.terminals.remove(index);
        // Keep the active index pointing at the same window where possible: shift it left when
        // a window before it went, and never let it fall off the end.
        if self.active_terminal > index || self.active_terminal >= self.terminals.len() {
            self.active_terminal = self.active_terminal.saturating_sub(1);
        }
    }

    /// Opens the name/startup-command box for the focused window's on-screen tab, prefilled
    /// with what it has now. For a single-tab window the name is what shows as the window title.
    pub fn start_terminal_rename(&mut self) {
        let Some((name, startup)) = self
            .focused_panel()
            .map(|p| (p.name.clone().unwrap_or_default(), p.startup_command.clone().unwrap_or_default()))
        else {
            return;
        };
        self.terminal_rename_input = name;
        self.terminal_startup_input = startup;
        self.terminal_rename_field = TerminalField::Name;
        self.show_terminal_rename = true;
    }

    fn terminal_field_mut(&mut self) -> &mut String {
        match self.terminal_rename_field {
            TerminalField::Name => &mut self.terminal_rename_input,
            TerminalField::Startup => &mut self.terminal_startup_input,
        }
    }

    fn handle_terminal_rename_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.confirm_terminal_rename(),
            KeyCode::Esc => self.cancel_terminal_rename(),
            // Both fields are one line, so the vertical keys move between them too — nobody
            // should have to guess that only Tab works.
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                self.terminal_rename_field = self.terminal_rename_field.other();
            }
            KeyCode::Backspace => {
                self.terminal_field_mut().pop();
            }
            KeyCode::Char(c) => self.terminal_field_mut().push(c),
            _ => {}
        }
    }

    fn confirm_terminal_rename(&mut self) {
        let lang = self.settings.lang;
        let name = self.terminal_rename_input.trim().to_string();
        // The command keeps its inner spacing — it is a shell command line, not a label.
        let startup = self.terminal_startup_input.trim().to_string();
        if let Some(panel) = self.focused_panel_mut() {
            // An empty name clears it, falling back to the default "Terminal N"; an empty
            // command clears it, so a workspace stops running it.
            panel.name = (!name.is_empty()).then(|| name.clone());
            panel.startup_command = (!startup.is_empty()).then(|| startup.clone());
        }
        // Deliberately not run here: it belongs to opening the workspace, and re-running
        // `claude` (or a dev server) just for renaming its tab would be a nasty surprise.
        self.status_message = i18n::msg_terminal_renamed(lang, &name, (!startup.is_empty()).then_some(&startup));
        self.cancel_terminal_rename();
    }

    pub fn cancel_terminal_rename(&mut self) {
        self.show_terminal_rename = false;
        self.terminal_rename_input.clear();
        self.terminal_startup_input.clear();
        self.terminal_rename_field = TerminalField::Name;
    }

    /// Closes the focused window's on-screen tab (context menu entry).
    pub fn close_active_terminal_tab(&mut self) {
        if let Some(window) = self.terminals.get(self.active_terminal) {
            let tab = window.active;
            self.close_terminal_tab(self.active_terminal, tab);
        }
    }

    /// Closes a single tab within a window. If it was the window's last tab, the window goes too.
    /// The very last terminal in the workspace is kept — there is always at least one.
    pub fn close_terminal_tab(&mut self, window_idx: usize, tab_idx: usize) {
        let total: usize = self.terminals.iter().map(|w| w.tabs.len()).sum();
        if total <= 1 {
            self.status_message = i18n::msg_min_one_terminal(self.settings.lang);
            return;
        }
        let Some(window) = self.terminals.get_mut(window_idx) else { return };
        if tab_idx >= window.tabs.len() {
            return;
        }
        window.tabs.remove(tab_idx);
        if window.active >= window.tabs.len() {
            window.active = window.tabs.len().saturating_sub(1);
        }
        // A window with no tabs left disappears, like closing it outright.
        if window.tabs.is_empty() {
            self.terminals.remove(window_idx);
            if self.active_terminal > window_idx || self.active_terminal >= self.terminals.len() {
                self.active_terminal = self.active_terminal.saturating_sub(1);
            }
        }
    }

    // ---- Workspaces -------------------------------------------------------------------

    /// Asks for a name to save the current set-up under, defaulting to the workspace already in
    /// use (so saving again just updates it) or the project folder's name.
    fn begin_save_workspace(&mut self) {
        self.workspace_save_input = self.active_workspace.clone().unwrap_or_else(|| {
            self.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        });
        self.show_workspace_save = true;
    }

    fn handle_workspace_save_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.confirm_save_workspace(),
            KeyCode::Esc => self.cancel_save_workspace(),
            KeyCode::Backspace => {
                self.workspace_save_input.pop();
            }
            KeyCode::Char(c) => self.workspace_save_input.push(c),
            _ => {}
        }
    }

    pub fn cancel_save_workspace(&mut self) {
        self.show_workspace_save = false;
        self.workspace_save_input.clear();
    }

    fn confirm_save_workspace(&mut self) {
        let lang = self.settings.lang;
        let name = self.workspace_save_input.trim().to_string();
        // An empty name has nothing to save under, so the box stays open rather than
        // silently doing nothing.
        if name.is_empty() {
            return;
        }
        // Refused here rather than deep in the writer, so the message is the user's language and
        // names what to do instead.
        if crate::workspace::is_default(&name) {
            self.cancel_save_workspace();
            self.status_message = i18n::msg_workspace_readonly(lang, &name);
            return;
        }
        self.cancel_save_workspace();
        let ws = self.capture_workspace(name.clone());
        let terminals = ws.terminals.len();
        match crate::workspace::save(&ws) {
            Ok(_) => {
                self.active_workspace = Some(name.clone());
                self.settings.last_workspace = Some(name.clone());
                // Written now rather than at exit, so a crash can't lose the workspace.
                self.settings.save();
                self.status_message = i18n::msg_workspace_saved(lang, &name, terminals);
            }
            Err(e) => self.status_message = i18n::msg_workspace_error(lang, &e),
        }
    }

    /// The current set-up as a workspace. Paths are canonicalized, as they are for the plain
    /// session resume: a workspace is opened from wherever the next `clee` happens to start.
    pub fn capture_workspace(&self, name: String) -> crate::workspace::Workspace {
        let canonical = |p: &PathBuf| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
        crate::workspace::Workspace {
            name,
            root: canonical(&self.root),
            open_files: self.editors.iter().filter_map(|e| e.path.as_ref()).map(canonical).collect(),
            active_file: self.editors.get(self.active_editor).and_then(|e| e.path.as_ref()).map(canonical),
            active_venv: self.settings.active_venv.clone(),
            active_terminal: self.active_terminal,
            layout: crate::workspace::WorkspaceLayout {
                show_sidebar: self.settings.show_sidebar,
                show_terminal: self.settings.show_terminal,
                show_menubar: self.settings.show_menubar,
                sidebar_width: self.settings.sidebar_width,
                terminal_pct: self.settings.terminal_pct,
                terminal_on_right: self.settings.terminal_on_right,
                split_view: self.split_view,
                split_pct: self.settings.split_pct,
            },
            terminals: self
                .terminals
                .iter()
                .map(|w| crate::workspace::WorkspaceTerminal {
                    weight: w.weight,
                    active: w.active,
                    tabs: w
                        .tabs
                        .iter()
                        .map(|t| crate::workspace::WorkspaceTab {
                            name: t.name.clone(),
                            startup_command: t.startup_command.clone(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    /// Restores a saved set-up: root, files, frame sizes, and the terminal windows/tabs with
    /// their names — running each tab's startup command in its own shell.
    pub fn apply_workspace(&mut self, ws: crate::workspace::Workspace) {
        let lang = self.settings.lang;
        let name = ws.name.clone();
        // Shells already running in the right directory are reused rather than replaced, which
        // is what makes opening the last workspace at startup cost nothing extra. A workspace
        // for a *different* project gets fresh shells, since a reused one would still sit in
        // the old project's directory.
        let same_root = self.root == ws.root;
        if ws.root.is_dir() && !same_root {
            self.set_root(ws.root.clone());
        }

        self.settings.show_sidebar = ws.layout.show_sidebar;
        self.settings.show_terminal = ws.layout.show_terminal;
        self.settings.show_menubar = ws.layout.show_menubar;
        self.settings.sidebar_width = ws.layout.sidebar_width;
        self.settings.terminal_pct = ws.layout.terminal_pct;
        self.settings.terminal_on_right = ws.layout.terminal_on_right;
        self.settings.split_pct = ws.layout.split_pct;
        self.settings.clamp_layout();
        self.split_view = ws.layout.split_view;
        self.settings.active_venv = ws.active_venv.clone();

        // Unsaved work outlives a workspace switch: dirty buffers stay open alongside the
        // workspace's own files. Everything else makes way.
        self.editors.retain(|e| e.dirty);
        if self.editors.is_empty() {
            self.editors.push(Editor::empty());
        }
        // The buffers that survived are renumbered from scratch, so the strips are rebuilt from
        // them rather than left pointing at what used to be there. Everything lands in the left
        // half; `settle_panes` gives the right one a buffer if the workspace was split.
        self.tabs = [(0..self.editors.len()).collect(), Vec::new()];
        self.active_editor = 0;
        self.active_editor_right = 0;
        self.tab_offsets = [0, 0];
        self.tab_revealed = [None, None];
        for path in &ws.open_files {
            if path.is_file() {
                self.open_file_in_tab(path.clone());
            }
        }
        if let Some(active) = &ws.active_file {
            if let Some(idx) = self.editors.iter().position(|e| e.path.as_deref() == Some(active.as_path())) {
                let pane = self.pane_holding(idx).unwrap_or(EditorPane::Left);
                self.set_pane_editor(pane, idx);
                self.editor_pane_focus = pane;
            }
        }
        self.settle_panes();

        self.rebuild_terminals(&ws, same_root);
        // Which shell you were looking at is part of the layout too, and it was being written to
        // the file and then ignored on the way back in.
        self.active_terminal = ws.active_terminal.min(self.terminals.len().saturating_sub(1));
        self.active_workspace = Some(name.clone());
        self.settings.last_workspace = Some(name.clone());
        self.status_message = i18n::msg_workspace_loaded(lang, &name);
    }

    /// Rebuilds the terminal windows a workspace describes. Existing shells are handed out in
    /// order when `reuse` allows; the rest are spawned. A window whose shells all failed to
    /// start is skipped rather than left empty, and the workspace never ends up with none.
    fn rebuild_terminals(&mut self, ws: &crate::workspace::Workspace, reuse: bool) {
        let mut spare: std::collections::VecDeque<TerminalPanel> = if reuse {
            self.terminals.drain(..).flat_map(|w| w.tabs).collect()
        } else {
            std::collections::VecDeque::new()
        };
        let root = self.root.clone();
        let mut windows = Vec::new();
        for wt in &ws.terminals {
            let mut tabs = Vec::new();
            for tab in &wt.tabs {
                // A shell spawned for this tab gets the command instead of the startup `clear`,
                // so only one line is ever queued. A reused one is already past that, and takes
                // the command the patient way — held until it is back at a prompt.
                let startup = tab.startup_command.as_deref();
                let (panel, reused) = match spare.pop_front() {
                    Some(p) => (Some(p), true),
                    None => (
                        TerminalPanel::with_startup(24, 80, &root, startup).ok(),
                        false,
                    ),
                };
                let Some(mut panel) = panel else { continue };
                panel.name = tab.name.clone();
                panel.startup_command = tab.startup_command.clone();
                if let (true, Some(command)) = (reused, &tab.startup_command) {
                    panel.run_command(command);
                }
                tabs.push(panel);
            }
            if tabs.is_empty() {
                continue;
            }
            let active = wt.active.min(tabs.len() - 1);
            windows.push(TerminalWindow { tabs, active, weight: wt.weight.max(1) });
        }
        if windows.is_empty() {
            // A workspace saved with no terminals, or one whose shells all failed to spawn:
            // fall back to the invariant the rest of the app relies on.
            match spare.pop_front() {
                Some(panel) => windows.push(TerminalWindow {
                    tabs: vec![panel],
                    active: 0,
                    weight: crate::terminal_panel::TERMINAL_WEIGHT_DEFAULT,
                }),
                None => {
                    if let Ok(w) = TerminalWindow::new(24, 80, &root) {
                        windows.push(w);
                    }
                }
            }
        }
        self.terminals = windows;
        self.active_terminal = ws.active_terminal.min(self.terminals.len().saturating_sub(1));
    }

    const WORKSPACE_OPEN_TITLE: &'static str = "Open workspace (Enter opens)";
    const WORKSPACE_DELETE_TITLE: &'static str = "Delete workspace (Enter deletes)";

    /// The saved workspaces as picker rows, each with its project folder as the dimmed detail
    /// so two workspaces over different projects stay tellable apart.
    fn open_workspace_picker(&mut self, delete: bool) {
        let mut saved = crate::workspace::list();
        // Deleting offers only the files; the built-in has nothing on disk to remove, so it is
        // simply not among the things you can pick there.
        if !delete {
            saved.insert(0, crate::workspace::default_workspace(self.root.clone()));
        }
        if saved.is_empty() {
            self.status_message = i18n::t(self.settings.lang, Key::MsgNoWorkspaces).to_string();
            return;
        }
        let items: Vec<crate::picker::PickItem> = saved
            .into_iter()
            .map(|ws| crate::picker::PickItem {
                shortcut: ws
                    .root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .or_else(|| Some(ws.root.to_string_lossy().into_owned())),
                action: crate::picker::PickAction::Workspace(ws.name.clone()),
                label: ws.name,
            })
            .collect();
        let (title, kind) = if delete {
            (Self::WORKSPACE_DELETE_TITLE, crate::picker::PickerKind::WorkspaceDelete)
        } else {
            (Self::WORKSPACE_OPEN_TITLE, crate::picker::PickerKind::Workspaces)
        };
        self.picker = Some(crate::picker::Picker::new(title, kind, items));
    }

    /// Deletes a saved workspace and refreshes the list in place, so several can go in one
    /// visit. The picker closes once nothing is left to delete.
    fn delete_workspace(&mut self, name: &str) {
        let lang = self.settings.lang;
        if crate::workspace::delete(name) {
            self.status_message = i18n::msg_workspace_deleted(lang, name);
        }
        if self.active_workspace.as_deref() == Some(name) {
            self.active_workspace = None;
            self.settings.last_workspace = None;
        }
        self.picker = None;
        if !crate::workspace::list().is_empty() {
            self.open_workspace_picker(true);
        }
    }

    pub fn cycle_terminal(&mut self, forward: bool) {
        if self.terminals.is_empty() {
            return;
        }
        let len = self.terminals.len();
        self.active_terminal = if forward {
            (self.active_terminal + 1) % len
        } else {
            (self.active_terminal + len - 1) % len
        };
    }

    pub fn run_active_file(&mut self) {
        let lang = self.settings.lang;
        // A preview tab, or a markdown source: ▶ means "show me", not "run a command".
        if self.run_as_preview() {
            return;
        }
        let Some(path) = self.editor().path.clone() else {
            self.status_message = i18n::msg_run_no_file(lang);
            return;
        };
        self.run_path(&path);
    }

    /// Runs one file's run command in a terminal. Split out from `run_active_file` because the
    /// ▶ button is no longer the only way in: a file that can only be *shown* runs the moment it
    /// is opened, with no buffer in between.
    /// Re-renders whatever a preview tab is showing, or opens the markdown preview for the
    /// source you are editing. `true` when it took the action, so Run can fall through to the
    /// shell for everything else.
    fn run_as_preview(&mut self) -> bool {
        let idx = self.pane_editor_index(self.editor_pane_focus);
        // Already a preview: ▶ means refresh. A rendered tab re-reads its buffer, a document
        // re-rasterises the page in front of you.
        if let Some(preview) = self.editors[idx].preview.as_ref().filter(|p| p.refreshable()) {
            let page = preview.page();
            let rendered = preview.source.is_some();
            let path = self.editors[idx].path.clone();
            if rendered {
                // Forcing a mismatch is the whole refresh: the next frame sees stale lines.
                // A document that failed before is worth one more try — that is what asking
                // for a refresh by hand means.
                if let Some(preview) = self.editors[idx].preview.as_mut() {
                    preview.document_failed = false;
                    preview.settled = None;
                    preview.shown_revision = u64::MAX;
                    preview.state = crate::preview::State::Rendered { lines: Vec::new(), revision: u64::MAX };
                }
            } else if let Some(path) = path {
                if let Some(preview) = self.editors[idx].preview.as_mut() {
                    preview.state = crate::preview::start_loading(
                    match page {
                        Some(page) => crate::preview::Job::Page { path: path.clone(), page },
                        None => crate::preview::Job::Picture(path.clone()),
                    },
                    self.preview_tx.clone(),
                );
                }
            }
            self.status_message = i18n::msg_preview_refreshed(self.settings.lang);
            return true;
        }
        // Markdown source: show it rendered beside itself.
        let Some(path) = self.editors[idx].path.clone() else { return false };
        if !crate::preview::is_renderable(&file_ext(&path)) {
            return false;
        }
        self.open_rendered_preview(path);
        true
    }

    /// Opens the rendered view of a markdown buffer in the other half, splitting the editor if
    /// it is not already. Opening a file leaves the layout alone — that is passive — but asking
    /// for a preview is an explicit request to see two things at once, and refusing to make room
    /// for it would be answering a different question.
    fn open_rendered_preview(&mut self, source: PathBuf) {
        let lang = self.settings.lang;
        // The same file now has two tabs, so a tab is found by path *and* kind: looking by path
        // alone would hand back the source and the preview would never open.
        if let Some(idx) = self
            .editors
            .iter()
            .position(|e| e.path.as_deref() == Some(source.as_path()) && e.preview.is_some())
        {
            self.focus_existing_tab(idx);
            return;
        }
        if !self.split_view {
            self.split_view = true;
            self.open_split();
        }
        let pane = match self.editor_pane_focus {
            EditorPane::Left => EditorPane::Right,
            EditorPane::Right => EditorPane::Left,
        };
        // The tab is *for* the source file and is a view *of* it: same path both times.
        let idx =
            self.adopt_editor(Editor::preview(source.clone(), crate::preview::Preview::rendered(source)));
        self.place_in_pane(pane, idx);
        // Says which of the two renderings you got: a document and styled text look very
        // different, and knowing which is which is the difference between "my machine cannot do
        // better" and "something is wrong".
        self.status_message = i18n::msg_markdown_preview(lang, crate::preview::markdown_as_document());
    }

    /// How long the text must sit still before a document preview is made from it. Long enough
    /// that a burst of typing is one render rather than dozens, short enough that a pause reads
    /// as "done" — the render itself takes about half a second, so anything shorter would just
    /// queue work behind work.
    const PREVIEW_SETTLE: std::time::Duration = std::time::Duration::from_millis(500);

    /// Brings every live preview up to date with the buffer it is a view of. Called once a
    /// frame; nothing happens at all while the text is not moving.
    ///
    /// Styled text is made here and now — it is only parsing, and it can follow the keystrokes.
    /// A document goes out to pandoc and a rasteriser and takes about half a second, so it waits
    /// for the typing to stop first.
    pub fn refresh_rendered_previews(&mut self) {
        let now = std::time::Instant::now();
        let as_document = crate::preview::markdown_as_document();
        // Gathered first: each preview needs a look at another editor, which cannot be done
        // while holding a mutable borrow of the list.
        let sources: Vec<(usize, Option<(u64, String)>)> = self
            .editors
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let source = e.preview.as_ref()?.source.as_ref()?;
                let src = self
                    .editors
                    .iter()
                    .find(|s| s.preview.is_none() && s.path.as_deref() == Some(source.as_path()));
                Some((i, src.map(|s| (s.revision(), s.rope.to_string()))))
            })
            .collect();

        for (i, source) in sources {
            let Some((revision, text)) = source else { continue };
            let Some(preview) = self.editors[i].preview.as_ref() else { continue };
            if !preview.stale(revision) {
                continue;
            }
            let failed = self.editors[i].preview.as_ref().is_some_and(|p| p.document_failed);
            if !as_document || failed {
                // Parsing is cheap enough to do on the spot, so the text view keeps up with the
                // keys — which on a terminal without graphics is the whole of what it can offer.
                let lines = crate::preview::render_markdown(&text);
                if let Some(preview) = self.editors[i].preview.as_mut() {
                    preview.state = crate::preview::State::Rendered { lines, revision };
                    preview.shown_revision = revision;
                }
                continue;
            }
            // Wait for the text to stop moving before spending half a second on it.
            let settled_at = match self.editors[i].preview.as_ref().and_then(|p| p.settled) {
                Some((seen, at)) if seen == revision => at,
                _ => {
                    if let Some(preview) = self.editors[i].preview.as_mut() {
                        preview.settled = Some((revision, now));
                    }
                    continue;
                }
            };
            if now.duration_since(settled_at) < Self::PREVIEW_SETTLE {
                continue;
            }
            let path = self.editors[i].path.clone().unwrap_or_default();
            let page = self.editors[i].preview.as_ref().and_then(|p| p.page()).unwrap_or(1);
            // Nothing on screen yet: put the styled-text rendering up as the interim, so the
            // first half second shows the document rather than an empty frame. Later renders
            // keep the previous page up instead, which is steadier than flashing back to text.
            let first = matches!(
                self.editors[i].preview.as_ref().map(|p| &p.state),
                Some(crate::preview::State::Rendered { lines, .. }) if lines.is_empty()
            );
            if first {
                let lines = crate::preview::render_markdown(&text);
                if let Some(preview) = self.editors[i].preview.as_mut() {
                    preview.state = crate::preview::State::Rendered { lines, revision };
                }
            }
            let job = crate::preview::Job::Markdown { path, text, page };
            if let Some(preview) = self.editors[i].preview.as_mut() {
                // Marked as shown *now*, before the render finishes: what is on screen stays
                // there meanwhile, and the same revision must not start a second render.
                preview.shown_revision = revision;
                let started = crate::preview::start_loading(job, self.preview_tx.clone());
                // Only blank the pane when there is nothing better to leave up.
                if !first {
                    preview.state = started;
                }
            }
        }
    }

    fn run_path(&mut self, path: &std::path::Path) {
        let lang = self.settings.lang;
        let path = path.to_path_buf();
        let ext = file_ext(&path);
        let Some(template) = self.run_command_for(&ext).cloned() else {
            self.status_message = i18n::msg_run_no_command(lang, &ext);
            return;
        };
        if self.terminals.is_empty() {
            self.new_terminal();
        }
        // A `.m` file when an Octave prompt is already open in one of the terminals: hand the
        // script to that interpreter with `run(...)`. Starting a second Octave would be slow,
        // would lose the session's variables, and (with --persist) would leave two sets of
        // plot windows around. Typing the shell command at an Octave prompt is also just a
        // syntax error, which is what used to happen.
        let program = template.split_once(' ').map(|(p, _)| p).unwrap_or(&template);
        if dnd::is_octave_program(program) {
            // Only the on-screen tab of each window is a candidate: running a script in a hidden
            // tab would be invisible and confusing.
            let pids: Vec<Option<u32>> = self.terminals.iter().map(|w| w.active_tab().child_pid()).collect();
            if let Some(idx) = dnd::shell_running_octave(&pids) {
                let command = format!("run({})", dnd::octave_quote(&path.to_string_lossy()));
                if let Some(term) = self.window_tab_mut(idx) {
                    term.write_input(command.as_bytes());
                    term.write_input(b"\r");
                }
                self.status_message = i18n::msg_run_started(lang, idx, &command);
                return;
            }
        }
        // An active venv wins over a configured interpreter path: it is the more specific
        // choice, and only ever rewrites python programs.
        let venved = self.apply_venv(&template);
        let template = if venved == template {
            resolve_interpreter(
                &template,
                &self.settings.interpreter_paths,
                std::env::var_os("ProgramFiles").map(PathBuf::from).as_deref(),
            )
        } else {
            venved
        };
        let command = expand_placeholders(&template, &path);
        let idx = self
            .terminals
            .iter()
            .position(|w| w.active_tab().child_pid().map(|pid| !dnd::shell_is_busy(pid)).unwrap_or(false))
            .unwrap_or(self.active_terminal.min(self.terminals.len().saturating_sub(1)));

        if let Some(term) = self.window_tab_mut(idx) {
            term.write_input(command.as_bytes());
            term.write_input(b"\r");
        }
        self.status_message = i18n::msg_run_started(lang, idx, &command);
    }

    /// If a venv is selected and the command's program is a python interpreter,
    /// swaps in the venv's own binary (e.g. "python3" -> ".venv/bin/python3").
    /// Left untouched for every other program, so custom run_commands entries for
    /// other languages are unaffected.
    fn apply_venv(&self, template: &str) -> String {
        let Some(venv) = &self.settings.active_venv else { return template.to_string() };
        let Some((program, rest)) = template.split_once(' ') else { return template.to_string() };
        if !matches!(program, "python" | "python3" | "python2") {
            return template.to_string();
        }
        // On Windows the venv ships python.exe under Scripts\; elsewhere a bare name under bin/.
        let bin_name = if cfg!(windows) { "python.exe" } else { program };
        // A registered venv is stored as an absolute path; an auto-discovered one is a
        // folder name relative to the project root.
        let venv_path = std::path::Path::new(venv);
        let venv_dir = if venv_path.is_absolute() { venv_path.to_path_buf() } else { self.root.join(venv) };
        let venv_bin = venv_dir.join(venv_bin_dir()).join(bin_name);
        if !venv_bin.exists() {
            return template.to_string();
        }
        let quoted = shell_words::quote(&venv_bin.to_string_lossy()).into_owned();
        format!("{quoted} {rest}")
    }

    /// Which buffer a pane is showing. The toolbar button describes the file under it, so each
    /// pane asks about its own.
    pub fn pane_editor_index(&self, pane: EditorPane) -> usize {
        match pane {
            EditorPane::Left => self.active_editor,
            EditorPane::Right => self.active_editor_right,
        }
    }

    /// The extension that decides how a buffer runs, lowercased. Empty for a file with no
    /// extension and for one that has never been saved — neither can be keyed on.
    pub fn editor_ext(&self, idx: usize) -> String {
        self.editors.get(idx).and_then(|e| e.path.as_deref()).map(file_ext).unwrap_or_default()
    }

    /// Opens the run-target drop-down under a pane's toolbar button: the venv list for python
    /// files, and for every file type the run command behind the Run button. Replaces cycling
    /// blindly to the next venv, which with more than two meant clicking until the right one
    /// appeared.
    pub fn open_run_menu(&mut self, pane: EditorPane) {
        let ext = self.editor_ext(self.pane_editor_index(pane));
        // Nothing to configure without an extension to key the command on; say so rather than
        // opening a menu whose one row couldn't be saved anywhere.
        if ext.is_empty() {
            self.status_message = i18n::msg_run_no_ext(self.settings.lang);
            return;
        }
        // Start on the entry that is currently active, so Enter alone changes nothing.
        let selected = if is_python_ext(&ext) {
            match &self.settings.active_venv {
                None => 0,
                Some(active) => {
                    self.available_venvs.iter().position(|v| v == active).map(|i| i + 1).unwrap_or(0)
                }
            }
        } else {
            0
        };
        self.run_menu = Some(RunMenu { pane, ext, selected });
    }

    /// The open menu's rows. Built in one place so what is drawn and what a click resolves to
    /// can't disagree.
    pub fn run_menu_rows(&self) -> Vec<RunRow> {
        let Some(menu) = &self.run_menu else { return Vec::new() };
        run_rows(
            &menu.ext,
            self.settings.active_venv.as_deref(),
            &self.available_venvs,
            &self.settings.registered_venvs,
            &self.settings.run_commands,
            &self.project_settings.run_commands,
            self.settings.lang,
        )
    }

    fn handle_run_menu_key(&mut self, key: KeyEvent) {
        let Some(menu) = &self.run_menu else { return };
        let selected = menu.selected;
        let len = self.run_menu_rows().len();
        match key.code {
            KeyCode::Esc => self.run_menu = None,
            KeyCode::Up => {
                if let Some(menu) = self.run_menu.as_mut() {
                    menu.selected = (selected + len - 1) % len;
                }
            }
            KeyCode::Down => {
                if let Some(menu) = self.run_menu.as_mut() {
                    menu.selected = (selected + 1) % len;
                }
            }
            KeyCode::Enter => self.activate_run_row(selected),
            _ => {}
        }
    }

    fn activate_run_row(&mut self, index: usize) {
        let mut rows = self.run_menu_rows();
        let ext = self.run_menu.as_ref().map(|m| m.ext.clone()).unwrap_or_default();
        self.run_menu = None;
        if index >= rows.len() {
            return;
        }
        match rows.swap_remove(index).action {
            RunRowAction::SelectVenv(venv) => self.select_venv(venv),
            RunRowAction::Browse => self.begin_venv_browse(),
            RunRowAction::Register => self.begin_venv_register(),
            RunRowAction::EditCommand(scope) => self.begin_run_command_edit(ext, scope),
        }
    }

    /// The command a file of this extension runs with: the project's own if it has one, and the
    /// global one otherwise. The single place that precedence is decided, so the toolbar label,
    /// the menu's marker and Run itself cannot disagree about which command is in force.
    pub fn run_command_for(&self, ext: &str) -> Option<&String> {
        self.project_settings.run_commands.get(ext).or_else(|| self.settings.run_commands.get(ext))
    }

    /// Opens the box that types the run command for `ext`, pre-filled with what that scope
    /// already holds, so the common edit is a tweak rather than a retype.
    ///
    /// Pre-filled from the scope being edited and not from what is in force: opening the
    /// project row on a project with no override starts empty, which is the truth about the
    /// file being written, and typing nothing then leaves it that way.
    fn begin_run_command_edit(&mut self, ext: String, scope: RunScope) {
        if ext.is_empty() {
            return;
        }
        let existing = match scope {
            RunScope::Global => self.settings.run_commands.get(&ext),
            RunScope::Project => self.project_settings.run_commands.get(&ext),
        };
        self.run_command_input = existing.cloned().unwrap_or_default();
        self.run_command_edit = Some((ext, scope));
    }

    fn handle_run_command_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.confirm_run_command_edit(),
            KeyCode::Esc => self.cancel_run_command_edit(),
            KeyCode::Backspace => {
                self.run_command_input.pop();
            }
            KeyCode::Char(c) => self.run_command_input.push(c),
            _ => {}
        }
    }

    pub fn cancel_run_command_edit(&mut self) {
        self.run_command_edit = None;
        self.run_command_input.clear();
    }

    fn confirm_run_command_edit(&mut self) {
        let lang = self.settings.lang;
        let Some((ext, scope)) = self.run_command_edit.take() else { return };
        let command = self.run_command_input.trim().to_string();
        self.run_command_input.clear();
        match scope {
            RunScope::Project => {
                if command.is_empty() {
                    // Dropping the override hands the extension back to the global command,
                    // which is a complete answer on its own — so there is nothing to restore
                    // and the entry simply goes.
                    self.project_settings.run_commands.remove(&ext);
                    self.status_message = i18n::msg_run_command_project_cleared(lang, &ext);
                } else {
                    self.status_message = i18n::msg_run_command_project_set(lang, &ext, &command);
                    self.project_settings.run_commands.insert(ext, command);
                }
                self.project_settings.save(&self.root);
            }
            RunScope::Global => {
                if command.is_empty() {
                    // An emptied box means "undo my customisation". For an extension that ships
                    // with a default that is the default coming back, not the entry vanishing —
                    // the defaults are re-merged at every start, so removing it would only look
                    // like it worked until the next launch.
                    match settings::default_run_command(&ext) {
                        Some(default) => {
                            self.status_message = i18n::msg_run_command_set(lang, &ext, &default);
                            self.settings.run_commands.insert(ext, default);
                        }
                        None => {
                            self.settings.run_commands.remove(&ext);
                            self.status_message = i18n::msg_run_command_cleared(lang, &ext);
                        }
                    }
                } else {
                    self.status_message = i18n::msg_run_command_set(lang, &ext, &command);
                    self.settings.run_commands.insert(ext, command);
                }
                // Persisted now rather than at exit, so a crash can't lose what was just typed.
                self.settings.save();
            }
        }
    }

    /// Opens the disk browser for picking a venv folder. Starts in the project root — where a
    /// per-project venv usually lives — and reuses the quick-open path machinery, so typing
    /// `/` or `~` walks off elsewhere just as it does there.
    fn begin_venv_browse(&mut self) {
        let mut picker = crate::picker::Picker::new(
            "Browse for a venv (type / or ~ to go elsewhere)",
            crate::picker::PickerKind::VenvBrowse,
            Vec::new(),
        );
        // A relative marker lists the project root while keeping the query box readable.
        picker.query = "./".to_string();
        self.picker = Some(picker);
        self.refresh_venv_browser();
    }

    /// Rebuilds the venv browser's listing from what has been typed. Only directories are
    /// offered — a file can't be a venv — and the ones that actually are venvs are flagged.
    fn refresh_venv_browser(&mut self) {
        let Some(query) = self
            .picker
            .as_ref()
            .filter(|p| p.kind == crate::picker::PickerKind::VenvBrowse)
            .map(|p| p.query.clone())
        else {
            return;
        };
        let home = dirs::home_dir();
        // The browser always reads its query as a path, so fall back to the root when what has
        // been typed doesn't parse as one (e.g. a bare fragment after backspacing the "./").
        let (dir, fragment) = path_query(&query, &self.root, home.as_deref())
            .unwrap_or_else(|| (self.root.clone(), String::new()));
        let items = venv_browse_items(&dir);
        if let Some(picker) = self.picker.as_mut() {
            picker.path_mode = true;
            picker.filter_override = Some(fragment);
            picker.set_items(items);
        }
    }

    fn select_venv(&mut self, venv: Option<String>) {
        let lang = self.settings.lang;
        self.status_message = match &venv {
            Some(v) => i18n::msg_venv_selected(lang, &ui::venv_display_name(v, &self.settings.registered_venvs)),
            None => i18n::msg_venv_cleared(lang),
        };
        self.settings.active_venv = venv;
    }

    /// Step one of registering a venv: ask for its path.
    fn begin_venv_register(&mut self) {
        self.venv_register = Some(VenvRegisterStep::Path);
        self.venv_register_input.clear();
        self.venv_register_path = None;
    }

    /// The path is already known (picked in the browser and confirmed a venv), so skip step one
    /// and go straight to naming it — the same second step as the typed-by-hand flow.
    fn begin_venv_nickname(&mut self, path: PathBuf) {
        self.venv_register_path = Some(path);
        self.venv_register_input.clear();
        self.venv_register = Some(VenvRegisterStep::Nickname);
    }

    fn handle_venv_register_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.confirm_venv_register(),
            KeyCode::Esc => self.cancel_venv_register(),
            KeyCode::Backspace => {
                self.venv_register_input.pop();
            }
            KeyCode::Char(c) => self.venv_register_input.push(c),
            _ => {}
        }
    }

    pub fn cancel_venv_register(&mut self) {
        self.venv_register = None;
        self.venv_register_input.clear();
        self.venv_register_path = None;
    }

    fn confirm_venv_register(&mut self) {
        let lang = self.settings.lang;
        match self.venv_register {
            Some(VenvRegisterStep::Path) => {
                // Same resolution as Save As: absolute, or relative to the project root, with
                // ~ expanded — this box is typed by hand.
                let home = dirs::home_dir();
                let Some(path) = resolve_save_as_path(&self.venv_register_input, &self.root, home.as_deref())
                else {
                    return;
                };
                // Refuse anything that isn't actually a venv, rather than registering a dead
                // entry that would silently never appear in the list.
                if !is_venv_dir(&path) {
                    self.status_message = i18n::msg_not_a_venv(lang, &path.display().to_string());
                    return;
                }
                self.venv_register_path = Some(path);
                self.venv_register_input.clear();
                self.venv_register = Some(VenvRegisterStep::Nickname);
            }
            Some(VenvRegisterStep::Nickname) => {
                let Some(path) = self.venv_register_path.take() else {
                    self.cancel_venv_register();
                    return;
                };
                let path = path.to_string_lossy().into_owned();
                let nickname = self.venv_register_input.trim().to_string();
                self.cancel_venv_register();

                let entry = if nickname.is_empty() {
                    settings::RegisteredVenv::Path(path.clone())
                } else {
                    settings::RegisteredVenv::Named { name: nickname, path: path.clone() }
                };
                // Registering the same path twice would show it twice in the list.
                self.settings.registered_venvs.retain(|r| r.path() != path);
                self.settings.registered_venvs.push(entry);
                self.available_venvs = available_venvs(&self.root, &self.settings.registered_venvs);
                // Selecting it is almost certainly why it was just added.
                self.select_venv(Some(path));
                // Persisted now rather than at exit, so a crash can't lose the registration.
                self.settings.save();
            }
            None => {}
        }
    }

    fn cycle_focus(&mut self, forward: bool) {
        let mut order = vec![Focus::FileTree, Focus::Editor, Focus::Terminal];
        if !self.settings.show_sidebar {
            order.retain(|f| *f != Focus::FileTree);
        }
        if !self.settings.show_terminal {
            order.retain(|f| *f != Focus::Terminal);
        }
        if order.is_empty() {
            return;
        }
        let pos = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        let len = order.len();
        let new_pos = if forward { (pos + 1) % len } else { (pos + len - 1) % len };
        self.focus = order[new_pos];
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.show_splash {
            self.show_splash = false;
            return;
        }
        if self.show_about {
            self.show_about = false;
            return;
        }
        // The context menu grabs keys ahead of everything else while it's up.
        if self.context_menu.is_some() {
            self.handle_context_menu_key(key);
            return;
        }
        if self.unsaved_prompt.is_some() {
            self.handle_unsaved_prompt_key(key);
            return;
        }
        // Ahead of the other modals: it can be opened *by* the unsaved-changes prompt, and
        // until a name is given there is nothing else worth typing at.
        if self.show_save_as {
            self.handle_save_as_key(key);
            return;
        }
        if self.run_menu.is_some() {
            self.handle_run_menu_key(key);
            return;
        }
        if self.venv_register.is_some() {
            self.handle_venv_register_key(key);
            return;
        }
        if self.run_command_edit.is_some() {
            self.handle_run_command_key(key);
            return;
        }
        if self.picker.is_some() {
            self.handle_picker_key(key);
            return;
        }
        if self.find.is_some() {
            self.handle_find_key(key);
            return;
        }
        if self.show_goto {
            self.handle_goto_key(key);
            return;
        }
        if self.show_new_entry {
            self.handle_new_entry_key(key);
            return;
        }
        if self.show_delete_confirm {
            self.handle_delete_confirm_key(key);
            return;
        }
        if self.show_rename {
            self.handle_rename_key(key);
            return;
        }
        if self.show_terminal_rename {
            self.handle_terminal_rename_key(key);
            return;
        }
        if self.show_workspace_save {
            self.handle_workspace_save_key(key);
            return;
        }
        if self.manual.is_some() {
            self.handle_manual_key(key);
            return;
        }
        if self.resize_mode {
            self.handle_resize_key(key);
            return;
        }
        if self.show_settings {
            self.handle_settings_key(key);
            return;
        }
        if self.menu.active {
            self.handle_menu_key(key);
            return;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);

        // A focused terminal has first claim on Ctrl. These panes run vim, tmux, ssh and
        // readline, where Ctrl+E is end-of-line, Ctrl+T is transpose, Ctrl+P is the previous
        // command and Ctrl+J *is* Enter — an editor that eats them is an editor you cannot work
        // in. So every Ctrl chord goes straight to the child, and CleeCode keeps exactly one:
        // Ctrl+Tab, its way back out of the pane. That is the bargain a terminal multiplexer
        // strikes, and Ctrl+Tab is chosen because no common terminal program binds it.
        //
        // Alt chords and the frame's own keys are unaffected: they never had a conflict to lose.
        //
        // Two exceptions stay with CleeCode. Ctrl+Tab is its way back out of the pane. And every
        // Ctrl+Shift chord is safe to keep, because no terminal can deliver one to a child in the
        // first place: the encoding terminals have used since VT100 has no room for the Shift, so
        // Ctrl+Shift+M and Ctrl+M are the same byte. Nothing running in the shell can be listening
        // for these, which is exactly what makes them a good home for the application's own keys.
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let reserved = matches!(
            key.code,
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down
        ) || shift;
        if self.focus == Focus::Terminal && ctrl && !reserved {
            self.handle_terminal_key(key);
            return;
        }

        // No Alt+<letter> and no Alt+<digit> anywhere in CleeCode. macOS only sends Option as
        // Meta on US keyboard layouts, so on any other one — Italian, German, French — those
        // chords never arrived at all, and a shortcut that silently does nothing is worse than
        // no shortcut. Alt with an *arrow* is a different matter and is still used: an Option
        // arrow produces no printable character, so the terminal forwards it as Meta whatever
        // the layout, which is also why editors have settled on Alt+↑/↓ for moving a line.
        match key.code {
            // ---- The application layer: Ctrl+Shift+<letter> -----------------------------------
            //
            // Function keys used to live here, and they are gone. On a laptop they are a second-
            // class row — on a Mac they need Fn — and PageUp/PageDown need Fn too, which ruled out
            // the tab and window keys as well. Alt was not an option either: it only reaches an
            // application when the terminal sends Option as Meta, which it does not on non-US
            // keyboard layouts, so half the old Alt bindings never arrived at all.
            //
            // Letters and arrows only, never a symbol: on an Italian layout `/`, `<`, `[` and
            // friends already need Shift or Option to type, so a chord built on one of them would
            // ask for the same modifier twice.
            KeyCode::Char('m') | KeyCode::Char('M') if ctrl && shift => {
                self.manual = Some(crate::manual::ManualState::new());
                return;
            }
            KeyCode::Char('o') | KeyCode::Char('O') if ctrl && shift => {
                self.show_settings = true;
                return;
            }
            KeyCode::Char('r') | KeyCode::Char('R') if ctrl && shift => {
                self.run_active_file();
                return;
            }
            KeyCode::Char('n') | KeyCode::Char('N') if ctrl && shift => {
                self.new_terminal();
                return;
            }
            KeyCode::Char('t') | KeyCode::Char('T') if ctrl && shift => {
                self.new_terminal_tab();
                return;
            }
            // One key closes the shell you are looking at. It takes the window with it when that
            // was its last tab, so there is nothing to remember about which of the two you meant.
            KeyCode::Char('k') | KeyCode::Char('K') if ctrl && shift => {
                self.close_active_terminal_tab();
                return;
            }
            KeyCode::Char('f') | KeyCode::Char('F') if ctrl && shift => {
                self.editor_mut().toggle_fold();
                return;
            }
            KeyCode::Char('u') | KeyCode::Char('U') if ctrl && shift => {
                self.resize_mode = !self.resize_mode;
                return;
            }
            KeyCode::Char('b') | KeyCode::Char('B') if ctrl && shift => {
                self.menu.open();
                return;
            }
            KeyCode::Char('g') | KeyCode::Char('G') if ctrl && shift => {
                self.open_context_menu_for_focus();
                return;
            }
            // Navigation lives on the arrows: the same physical keys on every layout, and no Fn
            // needed. Ctrl+<direction> moves to the frame that lies that way — sidebar, either
            // half of a split editor, or a tiled terminal window, without caring which kind it
            // is. Ctrl+Shift+←/→ is the one exception, moving between the tabs *inside* the
            // frame you are already in.
            KeyCode::Right if ctrl && shift => {
                self.cycle_focused_tab(true);
                return;
            }
            KeyCode::Left if ctrl && shift => {
                self.cycle_focused_tab(false);
                return;
            }
            // Walks the terminal windows without having to know how they happen to be tiled —
            // the spatial arrows do it too, but which one depends on the layout.
            KeyCode::Down if ctrl && shift => {
                self.cycle_terminal(true);
                return;
            }
            KeyCode::Up if ctrl && shift => {
                self.cycle_terminal(false);
                return;
            }
            // Ctrl+Alt, not plain Ctrl: macOS binds Ctrl with each arrow to Mission Control and
            // to switching Spaces, and the system takes them before any terminal sees them, so
            // a plain Ctrl+arrow here would never arrive on the platform CleeCode is developed
            // on. Adding Alt steps out of the way of all four, and costs nothing elsewhere —
            // nothing in a shell, an editor or a multiplexer binds Ctrl+Alt with an arrow.
            KeyCode::Left if ctrl && alt => {
                self.focus_in_direction(ResizeSide::Left);
                return;
            }
            KeyCode::Right if ctrl && alt => {
                self.focus_in_direction(ResizeSide::Right);
                return;
            }
            KeyCode::Up if ctrl && alt => {
                self.focus_in_direction(ResizeSide::Up);
                return;
            }
            KeyCode::Down if ctrl && alt => {
                self.focus_in_direction(ResizeSide::Down);
                return;
            }
            // Name the focused terminal and give it a startup command; save the workspace under
            // a name; save every dirty buffer. All three used to be Alt chords.
            KeyCode::Char('e') | KeyCode::Char('E') if ctrl && shift => {
                self.start_terminal_rename();
                return;
            }
            KeyCode::Char('w') | KeyCode::Char('W') if ctrl && shift => {
                self.begin_save_workspace();
                return;
            }
            KeyCode::Char('s') | KeyCode::Char('S') if ctrl && shift => {
                self.save_all();
                return;
            }
            KeyCode::Char('q') if ctrl => {
                self.request_quit();
                return;
            }
            KeyCode::Char('p') if ctrl => {
                self.open_command_palette();
                return;
            }
            KeyCode::Char('o') if ctrl => {
                self.open_file_picker();
                return;
            }
            KeyCode::Char('e') if ctrl => {
                self.settings.show_sidebar = !self.settings.show_sidebar;
                if !self.settings.show_sidebar && self.focus == Focus::FileTree {
                    self.cycle_focus(true);
                }
                return;
            }
            // Ctrl+T opens a new terminal tab in the focused window (the shell never sees it —
            // it's an app shortcut). Ctrl+J took over toggling the terminal panel.
            KeyCode::Char('t') if ctrl => {
                self.new_terminal_tab();
                return;
            }
            KeyCode::Char('j') if ctrl => {
                self.settings.show_terminal = !self.settings.show_terminal;
                if !self.settings.show_terminal && self.focus == Focus::Terminal {
                    self.cycle_focus(true);
                }
                return;
            }
            // Cycle the three frames, the way Cmd+Tab cycles windows. Reaching this from a
            // focused terminal is the whole point, so it is the one Ctrl chord the gate above
            // holds back from the shell.
            KeyCode::Tab if ctrl => {
                self.cycle_focus(!key.modifiers.contains(KeyModifiers::SHIFT));
                return;
            }
            KeyCode::BackTab if ctrl => {
                self.cycle_focus(false);
                return;
            }
            // No `focus != Terminal` guard on these two: the gate above already handed every
            // Ctrl chord to a focused shell before we got here.
            KeyCode::Char('b') if ctrl => {
                self.settings.show_menubar = !self.settings.show_menubar;
                return;
            }
            // Split the editor into two panes.
            KeyCode::Char('l') if ctrl => {
                self.toggle_split_view();
                return;
            }
            _ => {}
        }

        match self.focus {
            Focus::FileTree => self.handle_file_tree_key(key),
            Focus::Editor => self.handle_editor_key(key),
            Focus::Terminal => self.handle_terminal_key(key),
        }
    }

    fn menu_index_for_mnemonic(&self, c: char) -> Option<usize> {
        let target = c.to_ascii_lowercase();
        let lang = self.settings.lang;
        self.menu.defs.iter().position(|d| {
            i18n::t(lang, d.title_key)
                .chars()
                .next()
                .map(|first| first.to_ascii_lowercase() == target)
                .unwrap_or(false)
        })
    }

    fn handle_menu_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.menu.close(),
            _ if is_ctrl_shift(key, 'b') => self.menu.close(),
            KeyCode::Left => self.menu.move_menu(-1),
            KeyCode::Right => self.menu.move_menu(1),
            KeyCode::Up => self.menu.move_item(-1),
            KeyCode::Down => self.menu.move_item(1),
            KeyCode::Enter => {
                if let Some(action) = self.menu.selected_action() {
                    self.menu.close();
                    self.run_menu_action(action);
                }
            }
            KeyCode::Char(c) => {
                if let Some(idx) = self.menu_index_for_mnemonic(c) {
                    self.menu.menu_index = idx;
                    self.menu.item_index = 0;
                }
            }
            _ => {}
        }
    }

    fn run_menu_action(&mut self, action: MenuAction) {
        match action {
            MenuAction::ToggleSidebar => {
                self.settings.show_sidebar = !self.settings.show_sidebar;
                if !self.settings.show_sidebar && self.focus == Focus::FileTree {
                    self.cycle_focus(true);
                }
            }
            MenuAction::ToggleTerminal => {
                self.settings.show_terminal = !self.settings.show_terminal;
                if !self.settings.show_terminal && self.focus == Focus::Terminal {
                    self.cycle_focus(true);
                }
            }
            MenuAction::OpenMenuBar => self.menu.open(),
            MenuAction::ColumnSelection => {
                let lang = self.settings.lang;
                let ed = self.editor_mut();
                ed.selection_block = !ed.selection_block;
                // Turning it on with nothing selected drops an anchor where the cursor is, so
                // Shift+arrows have a corner to grow the rectangle from.
                if ed.selection_block && ed.selection_anchor.is_none() {
                    ed.selection_anchor = Some((ed.cursor_line, ed.cursor_col));
                }
                let on = self.editor().selection_block;
                self.status_message = i18n::msg_column_selection(lang, on);
            }
            MenuAction::ToggleMenuBar => self.settings.show_menubar = !self.settings.show_menubar,
            MenuAction::OpenSettings => self.show_settings = true,
            MenuAction::NewTerminal => self.new_terminal(),
            MenuAction::NewTerminalTab => self.new_terminal_tab(),
            MenuAction::CloseTerminalTab => self.close_active_terminal_tab(),
            MenuAction::RenameTerminal => self.start_terminal_rename(),
            MenuAction::CloseTerminal => self.close_active_terminal(),
            MenuAction::Save => self.save_active_file(),
            // Deliberately available for a named buffer too, to save a copy under a new name.
            MenuAction::SaveAs => self.begin_save_as(self.active_editor, None),
            MenuAction::SaveAll => self.save_all(),
            MenuAction::RunTarget => self.open_run_menu(self.editor_pane_focus),
            MenuAction::Quit => self.request_quit(),
            MenuAction::ShowAbout => self.show_about = true,
            // Copy/Paste follow the focus: from a terminal they act on its selection and input,
            // so the same menu entries make sense whichever frame raised the context menu.
            MenuAction::Copy => {
                if self.focus == Focus::Terminal {
                    self.copy_terminal_selection(self.active_terminal);
                } else {
                    self.copy_selection();
                }
            }
            MenuAction::Cut => self.cut_selection(),
            MenuAction::Paste => {
                if self.focus == Focus::Terminal {
                    let text = self.clipboard.get();
                    if !text.is_empty() {
                        if let Some(term) = self.focused_panel_mut() {
                            term.write_input(text.as_bytes());
                        }
                    }
                } else {
                    self.paste_clipboard();
                }
            }
            MenuAction::SelectAll => self.select_all(),
            MenuAction::Indent => {
                let tab_size = self.settings.tab_size;
                self.editor_mut().indent_selection(tab_size);
            }
            MenuAction::Outdent => {
                let tab_size = self.settings.tab_size;
                self.editor_mut().outdent_selection(tab_size);
            }
            MenuAction::ToggleFold => self.editor_mut().toggle_fold(),
            MenuAction::CloseFile => self.close_active_editor(),
            MenuAction::NextTab => self.cycle_editor(true),
            MenuAction::PrevTab => self.cycle_editor(false),
            MenuAction::NextTerminal => self.cycle_terminal(true),
            MenuAction::PrevTerminal => self.cycle_terminal(false),
            MenuAction::LayoutClassic => self.apply_layout_preset(PRESET_CLASSIC),
            MenuAction::LayoutWide => self.apply_layout_preset(PRESET_WIDE),
            MenuAction::LayoutTriple => self.apply_layout_preset(PRESET_TRIPLE),
            MenuAction::ToggleTerminalSide => self.settings.terminal_on_right = !self.settings.terminal_on_right,
            MenuAction::ToggleResizeMode => self.resize_mode = !self.resize_mode,
            MenuAction::RunFile => self.run_active_file(),
            MenuAction::ToggleSplitView => self.toggle_split_view(),
            MenuAction::ToggleHiddenFiles => self.toggle_hidden_files(),
            MenuAction::Undo => self.editor_undo(),
            MenuAction::Redo => self.editor_redo(),
            MenuAction::ToggleComment => self.toggle_comment(),
            MenuAction::DuplicateLine => self.editor_mut().duplicate_line(),
            MenuAction::MoveLineUp => self.editor_mut().move_line_up(),
            MenuAction::MoveLineDown => self.editor_mut().move_line_down(),
            MenuAction::Find => self.open_find(false),
            MenuAction::GotoLine => self.open_goto(),
            MenuAction::NewFile => self.open_new_entry(false),
            MenuAction::NewFolder => self.open_new_entry(true),
            MenuAction::Rename => self.start_rename(),
            MenuAction::Delete => self.start_delete(),
            MenuAction::CommandPalette => self.open_command_palette(),
            MenuAction::OpenFilePicker => self.open_file_picker(),
            MenuAction::NextTerminalTab => self.cycle_terminal_tab(true),
            MenuAction::PrevTerminalTab => self.cycle_terminal_tab(false),
            MenuAction::SaveWorkspace => self.begin_save_workspace(),
            MenuAction::OpenWorkspace => self.open_workspace_picker(false),
            MenuAction::DeleteWorkspace => self.open_workspace_picker(true),
            MenuAction::ShowManual => self.manual = Some(crate::manual::ManualState::new()),
            MenuAction::FocusFileTree => {
                // Focusing a hidden frame would leave the keyboard talking to something
                // invisible, so show it first — the intent is clearly to work there.
                self.settings.show_sidebar = true;
                self.focus = Focus::FileTree;
            }
            MenuAction::FocusEditor => self.focus = Focus::Editor,
            MenuAction::FocusTerminal => {
                self.settings.show_terminal = true;
                self.focus = Focus::Terminal;
            }
        }
    }

    // ---- Manual -----------------------------------------------------------------------

    /// Rows of manual text on screen, kept in step with what `ui::draw_manual` lays out so
    /// Space and Shift+Space move by exactly one screenful.
    fn manual_page(&self) -> usize {
        crate::ui::manual_body_height(self.last_full) as usize
    }

    fn handle_manual_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc || is_ctrl_shift(key, 'm') {
            self.manual = None;
            return;
        }
        let sections = crate::manual::sections(self.settings.lang);
        let page = self.manual_page();
        let Some(state) = self.manual.as_mut() else { return };
        let len = sections.get(state.section).map(|s| s.body.len()).unwrap_or(0);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            // Up and down walk the contents list, because that list is drawn as a column and
            // that is what an arrow pointing down at it should do. Reading position moves a
            // screenful at a time on Space, the way every pager since `more` has done it —
            // and unlike PageUp/PageDown it does not want the Fn key on a laptop.
            KeyCode::Up => state.cycle(-1, sections.len()),
            KeyCode::Down => state.cycle(1, sections.len()),
            KeyCode::Char(' ') if shift => state.scroll_by(-(page as isize), len, page),
            KeyCode::Char(' ') => state.scroll_by(page as isize, len, page),
            KeyCode::Backspace => state.scroll_by(-(page as isize), len, page),
            KeyCode::PageUp => state.scroll_by(-(page as isize), len, page),
            KeyCode::PageDown => state.scroll_by(page as isize, len, page),
            KeyCode::Home => state.scroll = 0,
            KeyCode::End => state.scroll_by(len as isize, len, page),
            KeyCode::Left | KeyCode::BackTab => state.cycle(-1, sections.len()),
            KeyCode::Right | KeyCode::Tab => state.cycle(1, sections.len()),
            // A digit jumps straight to a section, the way a table of contents invites.
            KeyCode::Char(c) if c.is_ascii_digit() => {
                let n = c.to_digit(10).unwrap_or(0) as usize;
                if n > 0 {
                    state.select(n - 1, sections.len());
                }
            }
            _ => {}
        }
    }

    fn handle_delete_confirm_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => self.confirm_delete(),
            _ => {
                self.show_delete_confirm = false;
                self.delete_target = None;
                self.status_message = i18n::msg_delete_cancelled(self.settings.lang);
            }
        }
    }

    fn handle_resize_key(&mut self, key: KeyEvent) {
        // Shift inverts the gesture: a plain arrow grows the focused frame on that side, Shift
        // shrinks it. The window-edge sides simply do nothing.
        let grow = !key.modifiers.contains(KeyModifiers::SHIFT);
        // Ctrl+Shift+U toggles back out, so the key that enters the mode also leaves it. Tested
        // before the arrows, which inside the mode belong to resizing.
        if is_ctrl_shift(key, 'u') {
            self.resize_mode = false;
            self.settings.save();
            return;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.resize_mode = false;
                self.settings.save();
            }
            KeyCode::Left => self.apply_resize(ResizeSide::Left, grow),
            KeyCode::Right => self.apply_resize(ResizeSide::Right, grow),
            KeyCode::Up => self.apply_resize(ResizeSide::Up, grow),
            KeyCode::Down => self.apply_resize(ResizeSide::Down, grow),
            _ => {}
        }
    }

    /// Moves the seam on `side` of the focused frame. A no-op (with a brief note) when that
    /// border is the outer window edge, which can't move.
    /// Moves the focus one frame in the given direction, doing nothing at the edge of the window
    /// rather than wrapping — an arrow that quietly jumped to the far side would be worse than
    /// one that stops.
    fn focus_in_direction(&mut self, side: ResizeSide) {
        let Some(target) = focus_neighbour(&self.resize_layout(), side) else { return };
        match target {
            FocusTarget::Tree => self.focus = Focus::FileTree,
            FocusTarget::Editor(pane) => {
                self.focus = Focus::Editor;
                self.editor_pane_focus = pane;
            }
            FocusTarget::Terminal(index) => {
                self.focus = Focus::Terminal;
                self.active_terminal = index.min(self.terminals.len().saturating_sub(1));
            }
        }
    }

    /// The layout facts the pure resolvers work from.
    fn resize_layout(&self) -> ResizeLayout {
        ResizeLayout {
            focus: self.focus,
            editor_pane: self.editor_pane_focus,
            split_view: self.split_view,
            show_sidebar: self.settings.show_sidebar,
            show_terminal: self.settings.show_terminal,
            terminal_on_right: self.settings.terminal_on_right,
            terminal_index: self.active_terminal,
            terminal_count: self.terminals.len(),
        }
    }

    fn apply_resize(&mut self, side: ResizeSide, grow: bool) {
        let layout = ResizeLayout {
            focus: self.focus,
            editor_pane: self.editor_pane_focus,
            split_view: self.split_view,
            show_sidebar: self.settings.show_sidebar,
            show_terminal: self.settings.show_terminal,
            terminal_on_right: self.settings.terminal_on_right,
            terminal_index: self.active_terminal,
            terminal_count: self.terminals.len(),
        };
        match resize_command(&layout, side, grow) {
            Some(ResizeCmd::Sidebar(d)) => {
                self.settings.sidebar_width = nudge_u16(self.settings.sidebar_width, d);
            }
            Some(ResizeCmd::Terminal(d)) => {
                self.settings.terminal_pct = nudge_u16(self.settings.terminal_pct, d);
            }
            Some(ResizeCmd::Split(d)) => {
                self.settings.split_pct = nudge_u16(self.settings.split_pct, d);
            }
            Some(ResizeCmd::TerminalWeight { seam, delta }) => {
                self.nudge_terminal_weight(seam, delta);
                return;
            }
            None => {
                self.status_message = i18n::msg_resize_edge(self.settings.lang);
                return;
            }
        }
        self.settings.clamp_layout();
    }

    /// Moves weight across the seam between terminal windows `seam` and `seam + 1`, keeping
    /// their combined share fixed — the keyboard twin of dragging that seam — with the same
    /// floor of a tenth each, so neither collapses to nothing.
    fn nudge_terminal_weight(&mut self, seam: usize, delta: i16) {
        let (Some(a), Some(b)) = (
            self.terminals.get(seam).map(|w| w.weight),
            self.terminals.get(seam + 1).map(|w| w.weight),
        ) else {
            return;
        };
        let total = a as i32 + b as i32;
        let floor = (total / 10).max(1);
        let new_a = (a as i32 + delta as i32).clamp(floor, total - floor);
        self.terminals[seam].weight = new_a as u16;
        self.terminals[seam + 1].weight = (total - new_a) as u16;
    }

    /// True while a layout resize is in play — Ctrl+Shift+U, or a border drag — so the focused
    /// frame can switch its border to the resize colour.
    pub fn layout_resize_active(&self) -> bool {
        self.resize_mode || self.dragging.map(DragTarget::is_layout).unwrap_or(false)
    }

    fn apply_layout_preset(&mut self, preset: LayoutPreset) {
        self.settings.show_sidebar = preset.show_sidebar;
        self.settings.show_terminal = preset.show_terminal;
        self.settings.sidebar_width = preset.sidebar_width;
        self.settings.terminal_pct = preset.terminal_pct;
        self.settings.terminal_on_right = preset.terminal_on_right;
        self.settings.clamp_layout();
    }

    /// Handles a click on a terminal window's top border: its close ✕, or (with several tabs) a
    /// tab's ✕ / tab switch. Returns whether the click was consumed, so the caller can stop before
    /// the resize seam that shares this row claims it.
    fn handle_terminal_titlebar_click(&mut self, col: u16, row: u16, areas: &ui::Areas) -> bool {
        let Some(term_areas) = &areas.terminals else { return false };
        let window_close = self.terminals.len() > 1;
        let lang = self.settings.lang;
        for (i, rect) in term_areas.iter().enumerate() {
            if row != rect.y || col < rect.x || col >= rect.x + rect.width {
                continue;
            }
            // The whole-window close button.
            if window_close && ui::terminal_close_cell(*rect) == Some((col, row)) {
                self.close_terminal(i);
                return true;
            }
            // A tab, when the window has more than one: its ✕ closes it, elsewhere switches to it.
            let tab_count = self.terminals[i].tabs.len();
            if tab_count > 1 {
                let strip = ui::terminal_tab_strip_rect(*rect, window_close);
                let labels = ui::terminal_tab_labels(&self.terminals[i], i, lang);
                if let Some((t, tab)) = ui::terminal_tab_ranges(strip, &labels)
                    .into_iter()
                    .enumerate()
                    .find(|(_, tab)| col >= tab.full.0 && col < tab.full.1)
                {
                    self.focus = Focus::Terminal;
                    self.active_terminal = i;
                    if tab.close == Some(col) {
                        self.close_terminal_tab(i, t);
                    } else {
                        self.terminals[i].active = t;
                    }
                    return true;
                }
            }
        }
        false
    }

    /// Every scrollbar that could be on screen, with the frame it rides and the way it runs.
    /// One list, walked by hit testing and by the pointer test alike, so a bar can never be
    /// clickable somewhere it isn't drawn.
    fn scrollbar_frames(&self, areas: &ui::Areas) -> Vec<(ScrollbarId, Rect, ui::Axis)> {
        let mut out = Vec::new();
        let panes = ui::editor_pane_rects(areas.editor, self.split_view, self.settings.split_pct);
        for (i, pane_rect) in panes.iter().enumerate() {
            let pane = if i == 0 { EditorPane::Left } else { EditorPane::Right };
            // The bars ride the *content* frame, below the tab strip, which is the box the
            // renderer draws them on.
            let (_, content) = ui::split_editor_area(*pane_rect);
            for axis in [ui::Axis::Vertical, ui::Axis::Horizontal] {
                out.push((ScrollbarId::Editor(pane, axis), content, axis));
            }
        }
        if let Some(terminals) = &areas.terminals {
            for (i, rect) in terminals.iter().enumerate() {
                out.push((ScrollbarId::Terminal(i), *rect, ui::Axis::Vertical));
            }
        }
        out
    }

    /// What a scrollbar describes right now: the whole content, where the view sits in it, and
    /// how much is on screen. `None` when it all fits and there is therefore no bar.
    fn scrollbar_metrics(&self, id: ScrollbarId, areas: &ui::Areas) -> Option<(usize, usize, usize)> {
        match id {
            ScrollbarId::Editor(pane, axis) => {
                let panes =
                    ui::editor_pane_rects(areas.editor, self.split_view, self.settings.split_pct);
                let rect = *panes.get(pane.index())?;
                let idx = self.pane_editor_index(pane);
                let (_, height, width) = ui::editor_viewport(self, idx, rect);
                ui::editor_scroll_metrics(self, idx, axis, height, width)
            }
            ScrollbarId::Terminal(i) => ui::terminal_scroll_metrics(self.window_tab(i)?),
        }
    }

    /// Whether a scrollbar should show itself in full rather than as a hint: the pointer is
    /// resting on it, or it is the one being dragged. Both are the moment its arrows and groove
    /// have to be aimable instead of merely suggestive.
    pub fn scrollbar_engaged(&self, id: ScrollbarId, frame: Rect, axis: ui::Axis) -> bool {
        if self.dragging == Some(DragTarget::Scrollbar(id)) {
            return true;
        }
        let Some((col, row)) = self.pointer else { return false };
        ui::scrollbar_strip(frame, axis).is_some_and(|strip| within(strip, col, row))
    }

    /// The scrollbar under a point, and which part of it.
    ///
    /// Only bars with somewhere to scroll are offered. The bars sit inside their frames, over
    /// the last column or row of the contents, so this is what keeps that column ordinary text
    /// — clickable, selectable — in every buffer that fits on screen.
    fn scrollbar_at(
        &self,
        col: u16,
        row: u16,
        areas: &ui::Areas,
    ) -> Option<(ScrollbarId, ScrollbarPart)> {
        for (id, frame, axis) in self.scrollbar_frames(areas) {
            let Some(strip) = ui::scrollbar_strip(frame, axis) else { continue };
            if !within(strip, col, row) || self.scrollbar_metrics(id, areas).is_none() {
                continue;
            }
            let layout = ui::scrollbar_layout(strip, axis);
            if layout.back.is_some_and(|r| within(r, col, row)) {
                return Some((id, ScrollbarPart::Step(-1)));
            }
            if layout.forward.is_some_and(|r| within(r, col, row)) {
                return Some((id, ScrollbarPart::Step(1)));
            }
            let (start, len, at) = Self::track_axis(layout.track, axis, col, row);
            return Some((id, ScrollbarPart::Track { offset: at.saturating_sub(start), len }));
        }
        None
    }

    /// A track reduced to the one dimension it runs along: where it starts, how long it is, and
    /// where the pointer falls on it.
    fn track_axis(track: Rect, axis: ui::Axis, col: u16, row: u16) -> (u16, u16, u16) {
        match axis {
            ui::Axis::Vertical => (track.y, track.height, row),
            ui::Axis::Horizontal => (track.x, track.width, col),
        }
    }

    fn apply_scrollbar(&mut self, id: ScrollbarId, part: ScrollbarPart, areas: &ui::Areas) {
        match part {
            ScrollbarPart::Step(delta) => self.nudge_scroll(id, delta),
            ScrollbarPart::Track { offset, len } => {
                let Some((total, _, viewport)) = self.scrollbar_metrics(id, areas) else { return };
                let position = ui::scroll_position_from_track(offset, len, total, viewport);
                self.set_scroll_position(id, position);
            }
        }
    }

    /// Keeps following the pointer while the thumb is held. The position is clamped to the
    /// track rather than abandoned when the pointer wanders off it, so a drag that strays
    /// sideways still scrolls — which is what every scrollbar does.
    fn drag_scrollbar(&mut self, id: ScrollbarId, col: u16, row: u16, areas: &ui::Areas) {
        let Some((_, frame, axis)) = self.scrollbar_frames(areas).into_iter().find(|(i, ..)| *i == id)
        else {
            return;
        };
        let Some(strip) = ui::scrollbar_strip(frame, axis) else { return };
        let layout = ui::scrollbar_layout(strip, axis);
        let (start, len, at) = Self::track_axis(layout.track, axis, col, row);
        let offset = at.saturating_sub(start).min(len.saturating_sub(1));
        self.apply_scrollbar(id, ScrollbarPart::Track { offset, len }, areas);
    }

    /// One line back or on — the mouse's version of pressing an arrow key.
    fn nudge_scroll(&mut self, id: ScrollbarId, delta: isize) {
        match id {
            ScrollbarId::Editor(pane, axis) => {
                let idx = self.pane_editor_index(pane);
                let current = match axis {
                    ui::Axis::Vertical => self.editors[idx].top_line,
                    ui::Axis::Horizontal => self.editors[idx].left_col,
                };
                self.set_scroll_position(id, current.saturating_add_signed(delta));
            }
            // The terminal counts its offset backwards from the live output, and `scroll_by`
            // already speaks the wheel's sign, so this needs no flipping of its own.
            ScrollbarId::Terminal(i) => {
                if let Some(term) = self.window_tab_mut(i) {
                    term.scroll_by(delta);
                }
            }
        }
    }

    fn set_scroll_position(&mut self, id: ScrollbarId, position: usize) {
        match id {
            ScrollbarId::Editor(pane, axis) => {
                let idx = self.pane_editor_index(pane);
                match axis {
                    ui::Axis::Vertical => {
                        let max = self.editors[idx].rope.len_lines().saturating_sub(1);
                        self.editors[idx].top_line = position.min(max);
                    }
                    ui::Axis::Horizontal => self.editors[idx].left_col = position,
                }
            }
            ScrollbarId::Terminal(i) => {
                if let Some(term) = self.window_tab_mut(i) {
                    // The bar counts from the start of the history; vt100 counts back from the
                    // live end, so the two are mirror images of each other.
                    let held = term.scrollback_lines();
                    term.scroll_to_offset(held.saturating_sub(position));
                }
            }
        }
    }

    fn try_start_drag(&mut self, col: u16, row: u16, areas: &ui::Areas) -> bool {
        // The scrollbars live inside their frames, so they no longer compete with the seams on
        // the borders — but they do sit over the contents, and this runs before the click
        // reaches the text underneath.
        if let Some((id, part)) = self.scrollbar_at(col, row, areas) {
            self.apply_scrollbar(id, part, areas);
            // An arrow is a click, not a grab; only the groove keeps following the pointer.
            if matches!(part, ScrollbarPart::Track { .. }) {
                self.dragging = Some(DragTarget::Scrollbar(id));
            }
            return true;
        }
        self.try_start_seam_drag(col, row, areas)
    }

    fn try_start_seam_drag(&mut self, col: u16, row: u16, areas: &ui::Areas) -> bool {
        if let Some(sidebar) = areas.sidebar {
            let border_x = sidebar.x + sidebar.width;
            if row >= sidebar.y && row < sidebar.y + sidebar.height && (col == border_x.saturating_sub(1) || col == border_x) {
                self.dragging = Some(DragTarget::Sidebar);
                return true;
            }
        }
        if let Some(term_areas) = &areas.terminals {
            if let Some(first) = term_areas.first() {
                if self.settings.terminal_on_right {
                    let border_x = first.x;
                    if row >= first.y && col + 1 >= border_x && col <= border_x + 1 {
                        self.dragging = Some(DragTarget::TerminalHeight);
                        return true;
                    }
                } else {
                    let border_y = first.y;
                    if col >= first.x && (row + 1 == border_y || row == border_y) {
                        self.dragging = Some(DragTarget::TerminalHeight);
                        return true;
                    }
                }
            }
            // The seams between adjacent terminal windows: vertical when tiled side by side,
            // horizontal when stacked. Dragging one redistributes the two windows' space.
            for i in 0..term_areas.len().saturating_sub(1) {
                let next = term_areas[i + 1];
                let hit = if self.settings.terminal_on_right {
                    let seam_y = next.y; // stacked: horizontal seam at the next window's top edge
                    col >= next.x && col < next.x + next.width && (row == seam_y || row + 1 == seam_y)
                } else {
                    let seam_x = next.x; // side by side: vertical seam at the next window's left edge
                    row >= next.y && row < next.y + next.height && (col == seam_x || col + 1 == seam_x)
                };
                if hit {
                    self.dragging = Some(DragTarget::TerminalSplit(i));
                    return true;
                }
            }
        }
        // The seam between the two editor panes, when split. It's the right pane's left edge;
        // grabbing it (or the column just left of it) starts a split-ratio drag.
        if self.split_view {
            let panes = ui::editor_pane_rects(areas.editor, true, self.settings.split_pct);
            if let Some(right) = panes.get(1) {
                let seam_x = right.x;
                // Skip the tab-bar row (the panes' top row): a click there is a tab click, not a
                // seam grab, or the split drag would swallow the editor tabs sitting on it.
                let (tab_bar, _) = ui::split_editor_area(*right);
                if row >= areas.editor.y + tab_bar.height
                    && row < areas.editor.y + areas.editor.height
                    && (col == seam_x || col + 1 == seam_x)
                {
                    self.dragging = Some(DragTarget::EditorSplit);
                    return true;
                }
            }
        }
        false
    }

    fn continue_drag(&mut self, col: u16, row: u16, full: Rect) {
        match self.dragging {
            Some(DragTarget::Sidebar) => {
                self.settings.sidebar_width = col;
                self.settings.clamp_layout();
            }
            Some(DragTarget::TerminalHeight) => {
                let main_top = 1u16;
                let main_bottom = full.height.saturating_sub(1);
                let main_height = main_bottom.saturating_sub(main_top).max(1);
                if self.settings.terminal_on_right {
                    let main_right = full.width;
                    let main_left = if self.settings.show_sidebar { self.settings.sidebar_width } else { 0 };
                    let main_width = main_right.saturating_sub(main_left).max(1);
                    let term_cols_from_right = main_right.saturating_sub(col);
                    self.settings.terminal_pct = ((term_cols_from_right as u32 * 100) / main_width as u32) as u16;
                } else {
                    let term_rows_from_bottom = main_bottom.saturating_sub(row);
                    self.settings.terminal_pct = ((term_rows_from_bottom as u32 * 100) / main_height as u32) as u16;
                }
                self.settings.clamp_layout();
            }
            Some(DragTarget::EditorSplit) => {
                // Turn the cursor's column within the editor region into the left pane's share.
                let areas = ui::compute_layout(full, &ui::LayoutParams::from_app(self));
                let editor = areas.editor;
                if editor.width > 1 && col > editor.x {
                    let offset = (col - editor.x).min(editor.width);
                    self.settings.split_pct = ((offset as u32 * 100) / editor.width as u32) as u16;
                    self.settings.clamp_layout();
                }
            }
            Some(DragTarget::TerminalSplit(i)) => self.drag_terminal_split(i, col, row, full),
            // All handled where the drag happens, against the frame it started in.
            Some(DragTarget::TextSelection)
            | Some(DragTarget::TerminalSelection(_))
            | Some(DragTarget::Scrollbar(_))
            | None => {}
        }
    }

    /// Redistributes the space between terminal windows `i` and `i + 1` as their seam is dragged.
    /// Their combined weight is preserved, so only these two resize, and a floor keeps either from
    /// collapsing to nothing.
    fn drag_terminal_split(&mut self, i: usize, col: u16, row: u16, full: Rect) {
        let areas = ui::compute_layout(full, &ui::LayoutParams::from_app(self));
        let Some(term_areas) = areas.terminals else { return };
        let (Some(&a), Some(&b)) = (term_areas.get(i), term_areas.get(i + 1)) else { return };
        // The pair's combined span along the tiling axis, and the cursor's position within it.
        let (start, end, pos) = if self.settings.terminal_on_right {
            (a.y, b.y + b.height, row) // stacked: drag vertically
        } else {
            (a.x, b.x + b.width, col) // side by side: drag horizontally
        };
        let span = end.saturating_sub(start) as u32;
        let (Some(wi), Some(wj)) = (
            self.terminals.get(i).map(|w| w.weight),
            self.terminals.get(i + 1).map(|w| w.weight),
        ) else {
            return;
        };
        let total = wi as u32 + wj as u32;
        if span < 2 || total == 0 {
            return;
        }
        let frac = pos.clamp(start + 1, end - 1) as u32 - start as u32;
        let floor = (total / 10).max(1); // at least a tenth of the pair each side
        let new_i = (total * frac / span).clamp(floor, total - floor) as u16;
        self.terminals[i].weight = new_i;
        self.terminals[i + 1].weight = (total - new_i as u32) as u16;
    }

    fn confirm_delete(&mut self) {
        let lang = self.settings.lang;
        self.show_delete_confirm = false;
        let Some(path) = self.delete_target.take() else { return };
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let result = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        match result {
            Ok(()) => {
                self.file_tree = FileTree::new(self.root.clone(), self.settings.show_hidden_files);
                self.status_message = i18n::msg_deleted(lang, &name);
            }
            Err(e) => self.status_message = i18n::msg_delete_failed(lang, &name, &e.to_string()),
        }
    }

    fn start_rename(&mut self) {
        let Some(path) = self.file_tree.selected_path() else { return };
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.rename_target = Some(path);
        self.rename_input = name;
        self.show_rename = true;
    }

    /// Opens the delete-confirmation prompt for the file-tree selection — the same flow the Delete
    /// key triggers, reached here from the context menu.
    fn start_delete(&mut self) {
        if let Some(path) = self.file_tree.selected_path() {
            self.delete_target = Some(path);
            self.show_delete_confirm = true;
        }
    }

    fn handle_rename_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.confirm_rename(),
            KeyCode::Esc => {
                self.show_rename = false;
                self.rename_target = None;
                self.rename_input.clear();
            }
            KeyCode::Backspace => {
                self.rename_input.pop();
            }
            KeyCode::Char(c) => self.rename_input.push(c),
            _ => {}
        }
    }

    fn confirm_rename(&mut self) {
        let lang = self.settings.lang;
        self.show_rename = false;
        let Some(old_path) = self.rename_target.take() else { return };
        let new_name = self.rename_input.trim().to_string();
        self.rename_input.clear();
        let old_name = old_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if new_name.is_empty() || new_name == old_name {
            return;
        }
        let Some(parent) = old_path.parent() else { return };
        let new_path = parent.join(&new_name);
        match std::fs::rename(&old_path, &new_path) {
            Ok(()) => {
                self.file_tree.refresh();
                for editor in &mut self.editors {
                    if editor.path.as_deref() == Some(old_path.as_path()) {
                        editor.path = Some(new_path.clone());
                    }
                }
                self.status_message = i18n::msg_renamed(lang, &old_name, &new_name);
            }
            Err(e) => self.status_message = i18n::msg_rename_failed(lang, &old_name, &e.to_string()),
        }
    }

    fn handle_settings_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.show_settings = false,
            _ if is_ctrl_shift(key, 'o') => self.show_settings = false,
            KeyCode::Up => {
                self.settings_selected = if self.settings_selected == 0 {
                    settings::SETTINGS_COUNT - 1
                } else {
                    self.settings_selected - 1
                };
            }
            KeyCode::Down => {
                self.settings_selected = (self.settings_selected + 1) % settings::SETTINGS_COUNT;
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.settings.activate(self.settings_selected);
                self.editor_mut().syntax_dirty = true;
            }
            KeyCode::Left => {
                self.settings.adjust(self.settings_selected, -1);
                self.editor_mut().syntax_dirty = true;
            }
            KeyCode::Right => {
                self.settings.adjust(self.settings_selected, 1);
                self.editor_mut().syntax_dirty = true;
            }
            _ => {}
        }
    }

    fn activate_file_tree_selection(&mut self) {
        match self.file_tree.activate_selected() {
            Some(Activation::OpenFile(path)) => self.open_file_in_tab(path),
            Some(Activation::SetRoot(path)) => self.set_root(path),
            Some(Activation::NavigateUp) => {
                if let Some(parent) = self.file_tree.parent_dir() {
                    self.set_root(parent);
                }
            }
            None => {}
        }
    }

    fn handle_file_tree_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => self.file_tree.move_selection(-1),
            KeyCode::Down => self.file_tree.move_selection(1),
            KeyCode::Left => self.file_tree.collapse_selected(),
            KeyCode::Right => self.file_tree.expand_selected(),
            KeyCode::Enter => self.activate_file_tree_selection(),
            KeyCode::Delete => {
                if let Some(path) = self.file_tree.selected_path() {
                    self.delete_target = Some(path);
                    self.show_delete_confirm = true;
                }
            }
            KeyCode::Char('h') | KeyCode::Char('H') => self.toggle_hidden_files(),
            KeyCode::Char('e') | KeyCode::Char('E') => self.start_rename(),
            KeyCode::Char('n') => self.open_new_entry(false),
            KeyCode::Char('N') => self.open_new_entry(true),
            _ => {}
        }
    }

    fn handle_editor_key(&mut self, key: KeyEvent) {
        // A preview tab has no text to move a cursor through, so the plain arrows are free and
        // mean the only thing they could mean here: the page before and the page after. No
        // chord had to be found for it, which on a keyboard this crowded is worth something.
        if self.editor().preview.is_some() && key.modifiers.is_empty() {
            let paged = self.editor().preview.as_ref().is_some_and(|p| p.pages.is_some());
            let idx = self.pane_editor_index(self.editor_pane_focus);
            let page = self.rendered_len(idx).map(|len| len.saturating_sub(1));
            let scroll = |app: &mut Self, delta: isize| {
                let Some(max) = page else { return };
                let top = &mut app.editors[idx].top_line;
                *top = top.saturating_add_signed(delta).min(max);
            };
            match key.code {
                KeyCode::Up if paged => {
                    self.turn_page(-1);
                    return;
                }
                KeyCode::Down if paged => {
                    self.turn_page(1);
                    return;
                }
                KeyCode::Left if paged => {
                    self.turn_page(-1);
                    return;
                }
                KeyCode::Right if paged => {
                    self.turn_page(1);
                    return;
                }
                // A rendered view is one long page, so the arrows scroll it instead.
                KeyCode::Up => {
                    scroll(self, -1);
                    return;
                }
                KeyCode::Down => {
                    scroll(self, 1);
                    return;
                }
                KeyCode::PageUp => {
                    scroll(self, -20);
                    return;
                }
                KeyCode::PageDown => {
                    scroll(self, 20);
                    return;
                }
                KeyCode::Home => {
                    self.editors[idx].top_line = 0;
                    return;
                }
                _ => {}
            }
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            // Ctrl+Shift+S is indistinguishable from plain Ctrl+S in standard terminal
            // input (no Kitty keyboard protocol), so Save All uses Alt+S instead — Alt
            // combos already work reliably via the ESC-prefix menu mnemonics.
            // Split-pane focus (only meaningful when split); left unchanged on Alt+←/→.
            // Move the current line up/down; Alt+Shift+↓ duplicates it.
            KeyCode::Down if alt && shift => self.editor_mut().duplicate_line(),
            KeyCode::Up if alt => self.editor_mut().move_line_up(),
            KeyCode::Down if alt => self.editor_mut().move_line_down(),
            // Editor-tab cycling moved off Ctrl+←/→ (now word motion) to Alt+, / Alt+.
            KeyCode::Char('s') if ctrl => self.save_active_file(),
            KeyCode::Char('w') if ctrl => self.close_active_editor(),
            KeyCode::Char('d') if ctrl => self.close_active_editor(),
            KeyCode::Char('c') if ctrl => self.copy_selection(),
            KeyCode::Char('x') if ctrl => self.cut_selection(),
            KeyCode::Char('v') if ctrl => self.paste_clipboard(),
            KeyCode::Char('a') if ctrl => self.select_all(),
            KeyCode::Char('z') | KeyCode::Char('Z') if ctrl && shift => self.editor_redo(),
            KeyCode::Char('z') if ctrl => self.editor_undo(),
            KeyCode::Char('y') if ctrl => self.editor_redo(),
            // Ctrl+K is the one that is documented: on an Italian layout `/` is Shift+7, so the
            // conventional Ctrl+/ asks for a modifier the chord already uses. It still works for
            // the layouts where `/` is a key of its own, since keeping it costs nothing.
            KeyCode::Char('k') if ctrl => self.toggle_comment(),
            KeyCode::Char('/') if ctrl => self.toggle_comment(),
            KeyCode::Char('f') if ctrl => self.open_find(false),
            KeyCode::Char('g') if ctrl => self.open_goto(),
            // Word-wise motion (Ctrl+←/→, Shift extends) and deletion (Ctrl+Backspace/Delete).
            // Word motion on Alt, not Ctrl: Ctrl+<direction> now leaves the frame. Alt+arrow is
            // what macOS uses for this anyway, and it survives every keyboard layout because an
            // Option arrow produces no printable character.
            KeyCode::Left if alt || ctrl => self.move_with_selection(shift, |e| e.move_word_left()),
            KeyCode::Right if alt || ctrl => self.move_with_selection(shift, |e| e.move_word_right()),
            KeyCode::Backspace if ctrl => self.editor_mut().delete_word_left(),
            KeyCode::Delete if ctrl => self.editor_mut().delete_word_right(),
            KeyCode::Char(c) if !ctrl => {
                let auto_pairs = self.settings.auto_pairs;
                self.editor_mut().insert_char_pairs(c, auto_pairs);
            }
            KeyCode::Enter => {
                let auto_indent = self.settings.auto_indent;
                let auto_pairs = self.settings.auto_pairs;
                let unit = self.indent_unit();
                self.editor_mut().newline_smart(auto_indent, auto_pairs, &unit);
            }
            KeyCode::Backspace => self.editor_mut().backspace(),
            KeyCode::Delete => self.editor_mut().delete_forward(),
            KeyCode::Left => self.move_with_selection(shift, |e| e.move_left()),
            KeyCode::Right => self.move_with_selection(shift, |e| e.move_right()),
            KeyCode::Up => self.move_with_selection(shift, |e| e.move_up()),
            KeyCode::Down => self.move_with_selection(shift, |e| e.move_down()),
            KeyCode::Home => self.move_with_selection(shift, |e| e.move_home()),
            KeyCode::End => self.move_with_selection(shift, |e| e.move_end()),
            KeyCode::PageUp => {
                let page = self.editor_viewport.0.max(1);
                self.move_with_selection(shift, |e| e.page_up(page));
            }
            KeyCode::PageDown => {
                let page = self.editor_viewport.0.max(1);
                self.move_with_selection(shift, |e| e.page_down(page));
            }
            KeyCode::Tab => {
                if self.editor().selection_range().is_some() {
                    let tab_size = self.settings.tab_size;
                    self.editor_mut().indent_selection(tab_size);
                } else if self.settings.insert_spaces {
                    let spaces = " ".repeat(self.settings.tab_size);
                    self.editor_mut().insert_str(&spaces);
                } else {
                    self.editor_mut().insert_char('\t');
                }
            }
            KeyCode::BackTab => {
                let tab_size = self.settings.tab_size;
                self.editor_mut().outdent_selection(tab_size);
            }
            _ => {}
        }
    }

    fn move_with_selection(&mut self, shift: bool, mv: impl FnOnce(&mut Editor)) {
        if shift {
            self.editor_mut().start_or_extend_selection();
        } else {
            self.editor_mut().clear_selection();
        }
        mv(self.editor_mut());
        // Moving the cursor ends the current typing run, so a later edit is its own undo step.
        self.editor_mut().break_undo_coalescing();
    }

    fn editor_undo(&mut self) {
        let lang = self.settings.lang;
        if !self.editor_mut().undo() {
            self.status_message = i18n::t(lang, Key::MsgNothingToUndo).to_string();
        }
    }

    fn editor_redo(&mut self) {
        let lang = self.settings.lang;
        if !self.editor_mut().redo() {
            self.status_message = i18n::t(lang, Key::MsgNothingToRedo).to_string();
        }
    }

    /// One indentation step as text: spaces or a tab, per settings.
    fn indent_unit(&self) -> String {
        if self.settings.insert_spaces {
            " ".repeat(self.settings.tab_size)
        } else {
            "\t".to_string()
        }
    }

    fn toggle_comment(&mut self) {
        let token = crate::editor::comment_token(self.editor().path.as_deref());
        match token {
            Some(token) => self.editor_mut().toggle_comment(token),
            None => {
                let lang = self.settings.lang;
                self.status_message = i18n::t(lang, Key::MsgNoCommentSyntax).to_string();
            }
        }
    }

    fn handle_terminal_key(&mut self, key: KeyEvent) {
        // Shift+arrows select inside the pane instead of reaching the shell, the same role a
        // terminal emulator plays for the program running in it. Esc drops the selection; every
        // other key goes through, so the child keeps its own keys.
        if key.modifiers.contains(KeyModifiers::SHIFT) {
            let step = match key.code {
                KeyCode::Left => Some((0, -1)),
                KeyCode::Right => Some((0, 1)),
                KeyCode::Up => Some((-1, 0)),
                KeyCode::Down => Some((1, 0)),
                _ => None,
            };
            if let Some((d_row, d_col)) = step {
                self.move_terminal_selection(d_row, d_col);
                return;
            }
            // Shift+PageUp/PageDown through the history: what every terminal emulator binds it
            // to, so the muscle memory already exists. It is not the only way in — both keys
            // want Fn on a laptop — which is why the wheel does the same job.
            let page = self.terminal_page(self.active_terminal);
            let paged = match key.code {
                KeyCode::PageUp => Some(-page),
                KeyCode::PageDown => Some(page),
                _ => None,
            };
            if let Some(delta) = paged {
                if let Some(term) = self.window_tab_mut(self.active_terminal) {
                    if !term.alternate_screen() {
                        term.scroll_by(delta);
                    }
                }
                return;
            }
        }
        let index = self.active_terminal;
        if key.code == KeyCode::Esc && self.window_tab(index).is_some_and(|t| t.selection.is_some()) {
            if let Some(term) = self.window_tab_mut(index) {
                term.clear_selection();
            }
            return;
        }
        let bytes = key_to_bytes(key);
        if !bytes.is_empty() {
            if let Some(term) = self.focused_panel_mut() {
                // Typing snaps back to the live output. The shell is about to answer, and an
                // answer that lands off-screen is worse than losing your place in the history —
                // which is exactly what every terminal emulator does for the same reason.
                term.scroll_to_bottom();
                term.write_input(&bytes);
            }
        }
    }

    /// One screenful of a terminal pane, for paging through its history. At least one line, so
    /// a pane collapsed to nothing still moves rather than sticking.
    fn terminal_page(&self, index: usize) -> isize {
        self.window_tab(index).map(|t| (t.rows.saturating_sub(1)).max(1) as isize).unwrap_or(1)
    }

    /// Extends the active pane's selection by one cell, anchoring it at the terminal's own
    /// cursor the first time, then copies it — same rule as finishing a mouse drag.
    fn move_terminal_selection(&mut self, d_row: i16, d_col: i16) {
        let index = self.active_terminal;
        let Some(term) = self.window_tab_mut(index) else { return };
        let from = match term.selection {
            Some(selection) => selection.cursor,
            None => {
                let cursor = term.cursor_cell();
                term.begin_selection(cursor);
                cursor
            }
        };
        let next = (
            from.0.saturating_add_signed(d_row),
            from.1.saturating_add_signed(d_col),
        );
        term.extend_selection(next);
        self.copy_terminal_selection(index);
    }

    /// Ends a mouse selection: a drag that never left its starting cell is a plain click to
    /// focus the pane, so it is dropped rather than highlighting (and copying) one character.
    fn finish_terminal_selection(&mut self, index: usize) {
        let single = self.window_tab(index).and_then(|t| t.selection).is_some_and(|s| s.is_single_cell());
        if single {
            if let Some(term) = self.window_tab_mut(index) {
                term.clear_selection();
            }
            return;
        }
        self.copy_terminal_selection(index);
    }

    /// Copies a terminal pane's selection to the system clipboard, reporting how much was
    /// taken. Silent when the selection is only blank cells, so dragging across empty space
    /// doesn't wipe the clipboard.
    fn copy_terminal_selection(&mut self, index: usize) {
        let Some(text) = self.window_tab(index).and_then(|t| t.selection_text()) else { return };
        if text.trim().is_empty() {
            return;
        }
        self.clipboard.set(&text);
        self.status_message = i18n::msg_copied_chars(self.settings.lang, text.chars().count());
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent, areas: &ui::Areas, full: Rect) {
        if !self.settings.mouse_enabled {
            return;
        }
        let col = mouse.column;
        let row = mouse.row;
        // Every event, before any of the modal early-returns: a pointer that stopped being
        // tracked while a menu was open would leave a scrollbar lit under it afterwards.
        self.pointer = Some((col, row));

        // The manual is a reader, not a frame: while it is up the mouse only picks sections,
        // scrolls, or dismisses it.
        if self.manual.is_some() {
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => self.mouse_manual(col, row),
                MouseEventKind::ScrollUp => self.scroll_manual(-3),
                MouseEventKind::ScrollDown => self.scroll_manual(3),
                _ => {}
            }
            return;
        }

        // A picker is a modal list: while one is up the mouse belongs to it. Clicking a result
        // takes it, the wheel moves the selection, and a click outside puts it away — the same
        // three things the keyboard could already do, which until now it could do alone.
        if self.picker.is_some() {
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => self.mouse_picker(col, row, full),
                MouseEventKind::ScrollUp => {
                    if let Some(p) = self.picker.as_mut() {
                        p.move_selection(-1);
                    }
                }
                MouseEventKind::ScrollDown => {
                    if let Some(p) = self.picker.as_mut() {
                        p.move_selection(1);
                    }
                }
                _ => {}
            }
            return;
        }

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if self.show_splash {
                    self.show_splash = false;
                    return;
                }
                if self.show_about {
                    self.show_about = false;
                    return;
                }
                // An open context menu intercepts the next click: on an item to run it, elsewhere
                // to dismiss.
                if self.context_menu.is_some() {
                    self.mouse_context_menu(col, row);
                    return;
                }
                if self.show_delete_confirm {
                    self.show_delete_confirm = false;
                    self.delete_target = None;
                    self.status_message = i18n::msg_delete_cancelled(self.settings.lang);
                    return;
                }
                if self.show_rename {
                    self.show_rename = false;
                    self.rename_target = None;
                    self.rename_input.clear();
                    return;
                }
                if self.show_terminal_rename {
                    self.cancel_terminal_rename();
                    return;
                }
                if self.show_workspace_save {
                    self.cancel_save_workspace();
                    return;
                }
                if self.show_save_as {
                    self.cancel_save_as();
                    return;
                }
                if self.venv_register.is_some() {
                    self.cancel_venv_register();
                    return;
                }
                if self.run_command_edit.is_some() {
                    self.cancel_run_command_edit();
                    return;
                }
                if self.run_menu.is_some() {
                    // Inside the list picks a row; anywhere else dismisses it, like the menus.
                    let rect = ui::run_menu_rect(self, areas.editor, full);
                    match rect.map(ui::inner_rect).filter(|inner| within(*inner, col, row)) {
                        Some(inner) => self.activate_run_row((row - inner.y) as usize),
                        None => self.run_menu = None,
                    }
                    return;
                }
                if self.show_settings {
                    self.mouse_settings(col, row, full);
                    return;
                }
                if self.menu.active {
                    self.mouse_menu(col, row, full);
                    return;
                }
                // A hidden menu bar collapses to a zero-height row still at y == 0, so guard
                // on its height or a click on the top editor row would open a phantom menu.
                if areas.menu_bar.height > 0 && row == areas.menu_bar.y {
                    self.mouse_menu_bar_click(col);
                    return;
                }
                // Terminal title-bar controls (window ✕, tab ✕, tab switch) live on the top
                // border, which in the bottom layout doubles as the resize seam — so claim them
                // before try_start_drag, or the drag would swallow every such click.
                if self.handle_terminal_titlebar_click(col, row, areas) {
                    return;
                }
                if self.try_start_drag(col, row, areas) {
                    return;
                }
                if let Some(sidebar) = areas.sidebar {
                    if within(sidebar, col, row) {
                        self.focus = Focus::FileTree;
                        let inner = ui::inner_rect(sidebar);
                        if row >= inner.y {
                            let idx = (row - inner.y) as usize;
                            if idx < self.file_tree.visible.len() {
                                let is_double_click = matches!(
                                    self.last_tree_click,
                                    Some((last_idx, t)) if last_idx == idx && t.elapsed() < DOUBLE_CLICK_THRESHOLD
                                );
                                self.file_tree.selected = idx;
                                if is_double_click {
                                    self.last_tree_click = None;
                                    self.activate_file_tree_selection();
                                } else {
                                    // A single click on a folder expands or collapses it, the
                                    // same as Right/Left on the keyboard. Double-click still
                                    // reroots, and that rebuilds the tree anyway, so this
                                    // intermediate toggle leaves no trace.
                                    let entry = &self.file_tree.visible[idx];
                                    if entry.is_dir && !entry.is_up {
                                        self.file_tree.toggle_selected();
                                    }
                                    self.last_tree_click = Some((idx, Instant::now()));
                                }
                            }
                        }
                        return;
                    }
                }
                let panes = ui::editor_pane_rects(areas.editor, self.split_view, self.settings.split_pct);
                if let Some((pane_idx, pane_rect)) = panes.iter().enumerate().find(|(_, r)| within(**r, col, row)) {
                    let pane_rect = *pane_rect;
                    self.focus = Focus::Editor;
                    self.editor_pane_focus = if pane_idx == 0 { EditorPane::Left } else { EditorPane::Right };
                    let (tab_bar, content) = ui::split_editor_area(pane_rect);
                    if within(tab_bar, col, row) {
                        self.mouse_tab_click(col, tab_bar, self.editor_pane_focus);
                    } else {
                        // Alt while dragging makes it a column selection, which is the gesture
                        // every editor and terminal uses for one — worth honouring precisely
                        // because there is no comfortable key combination left to spend on it.
                        let block = mouse.modifiers.contains(KeyModifiers::ALT);
                        self.editor_mut().clear_selection();
                        self.position_cursor_from_click(content, col, row);
                        let anchor = (self.editor().cursor_line, self.editor().cursor_col);
                        let ed = self.editor_mut();
                        ed.selection_anchor = Some(anchor);
                        ed.selection_block = block;
                        self.dragging = Some(DragTarget::TextSelection);
                    }
                    return;
                }
                if let Some(term_areas) = &areas.terminals {
                    // Title-bar controls were already handled above; here a click inside a pane
                    // focuses it and starts a text selection.
                    for (i, rect) in term_areas.iter().enumerate() {
                        if within(*rect, col, row) {
                            self.focus = Focus::Terminal;
                            self.active_terminal = i;
                            // cleecode captures the mouse, so the host terminal's own selection
                            // can't be used while it runs.
                            let content = ui::terminal_content_rect(*rect);
                            if let Some(cell) = cell_at(content, col, row) {
                                if let Some(term) = self.window_tab_mut(i) {
                                    term.begin_selection(cell);
                                }
                                self.dragging = Some(DragTarget::TerminalSelection(i));
                            }
                            return;
                        }
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Right) => {
                // Right-click raises the context menu for the frame under the pointer. Modals and
                // the open menu bar swallow it (nothing to act on there).
                if self.show_splash
                    || self.show_about
                    || self.show_settings
                    || self.menu.active
                    || self.context_menu.is_some()
                {
                    return;
                }
                self.open_context_menu_at(col, row, areas);
            }
            MouseEventKind::Drag(MouseButton::Left) => match self.dragging {
                Some(DragTarget::Sidebar)
                | Some(DragTarget::TerminalHeight)
                | Some(DragTarget::EditorSplit)
                | Some(DragTarget::TerminalSplit(_)) => {
                    self.continue_drag(col, row, full);
                }
                Some(DragTarget::Scrollbar(id)) => self.drag_scrollbar(id, col, row, areas),
                Some(DragTarget::TextSelection) => {
                    if within(areas.editor, col, row) {
                        // Stay within the pane the drag started in, regardless of which
                        // pane the pointer is currently over.
                        let panes = ui::editor_pane_rects(areas.editor, self.split_view, self.settings.split_pct);
                        let pane_rect = if self.split_view && self.editor_pane_focus == EditorPane::Right {
                            panes.get(1).copied().unwrap_or(areas.editor)
                        } else {
                            panes[0]
                        };
                        let (_, content) = ui::split_editor_area(pane_rect);
                        self.position_cursor_from_click(content, col, row);
                    }
                }
                Some(DragTarget::TerminalSelection(index)) => {
                    if let Some(rect) = areas.terminals.as_ref().and_then(|t| t.get(index)).copied() {
                        if let Some(cell) = cell_at(ui::terminal_content_rect(rect), col, row) {
                            if let Some(term) = self.window_tab_mut(index) {
                                term.extend_selection(cell);
                            }
                        }
                    }
                }
                None => {}
            },
            MouseEventKind::Up(MouseButton::Left) => {
                // Completing a selection puts it on the clipboard straight away: there is no
                // spare key combination in a terminal pane for an explicit copy (Ctrl+C has to
                // reach the shell as an interrupt).
                if let Some(DragTarget::TerminalSelection(index)) = self.dragging {
                    self.finish_terminal_selection(index);
                }
                self.dragging = None;
            }
            MouseEventKind::ScrollUp => self.scroll(col, row, areas, -3),
            MouseEventKind::ScrollDown => self.scroll(col, row, areas, 3),
            _ => {}
        }
    }

    fn scroll(&mut self, col: u16, row: u16, areas: &ui::Areas, delta: isize) {
        if let Some(sidebar) = areas.sidebar {
            if within(sidebar, col, row) {
                self.file_tree.move_selection(delta);
                return;
            }
        }
        let panes = ui::editor_pane_rects(areas.editor, self.split_view, self.settings.split_pct);
        for (pane_idx, pane_rect) in panes.iter().enumerate() {
            if !within(*pane_rect, col, row) {
                continue;
            }
            let (tab_bar, content) = ui::split_editor_area(*pane_rect);
            if within(tab_bar, col, row) {
                // Over the tab strip the wheel scrolls tabs sideways, one per notch, rather
                // than scrolling the text underneath.
                let pane = if pane_idx == 0 { EditorPane::Left } else { EditorPane::Right };
                let step = if delta < 0 { -1 } else { 1 };
                self.scroll_tabs(pane, step, tab_bar.width);
                return;
            }
            if within(content, col, row) {
                // Scroll whichever pane the pointer is over, independent of focus.
                let idx = if pane_idx == 0 { self.active_editor } else { self.active_editor_right };
                // A rendered preview holds no rope, so its length comes from the lines drawn.
                if let Some(len) = self.rendered_len(idx) {
                    let top = &mut self.editors[idx].top_line;
                    *top = if delta < 0 {
                        top.saturating_sub((-delta) as usize)
                    } else {
                        (*top + delta as usize).min(len.saturating_sub(1))
                    };
                    return;
                }
                if delta < 0 {
                    self.editors[idx].top_line = self.editors[idx].top_line.saturating_sub((-delta) as usize);
                } else {
                    let max_top = self.editors[idx].rope.len_lines().saturating_sub(1);
                    self.editors[idx].top_line = (self.editors[idx].top_line + delta as usize).min(max_top);
                }
            }
            return;
        }
        if let Some(term_areas) = &areas.terminals {
            if let Some(i) = term_areas.iter().position(|r| within(*r, col, row)) {
                // Like the editor panes: the wheel acts on what it is over, whether or not that
                // shell has the focus.
                if let Some(term) = self.window_tab_mut(i) {
                    // A full-screen program owns the screen and gets no scrollback of its own,
                    // so there is nothing here to scroll and the notch is simply dropped rather
                    // than moving a view that isn't there.
                    if !term.alternate_screen() {
                        term.scroll_by(delta);
                    }
                }
            }
        }
    }

    fn mouse_tab_click(&mut self, col: u16, tab_bar: Rect, pane: EditorPane) {
        let rel_col = col.saturating_sub(tab_bar.x);
        let strip = self.tab_strip(tab_bar.width, pane);
        if let Some((start, end)) = strip.left_arrow {
            if rel_col >= start && rel_col < end {
                self.scroll_tabs(pane, -1, tab_bar.width);
                return;
            }
        }
        if let Some((start, end)) = strip.right_arrow {
            if rel_col >= start && rel_col < end {
                self.scroll_tabs(pane, 1, tab_bar.width);
                return;
            }
        }
        if let Some((position, layout)) = strip.tab_at(rel_col) {
            // A click lands on a position in *this* strip; which buffer that is depends on the
            // pane, since the two halves hold different files.
            let Some(&editor_idx) = self.pane_tabs(pane).get(position) else { return };
            if rel_col >= layout.close.0 && rel_col < layout.close.1 {
                self.close_editor_at(editor_idx);
            } else {
                self.set_pane_editor(pane, editor_idx);
                self.focus = Focus::Editor;
                self.editor_pane_focus = pane;
            }
            return;
        }
        let (target_range, run_range) = ui::toolbar_button_ranges(self, tab_bar.width);
        if let Some((start, end)) = target_range {
            if rel_col >= start && rel_col < end {
                // The button describes the file in the pane it sits on, so the menu opens on
                // that pane's file even when the focus was elsewhere.
                self.open_run_menu(pane);
                return;
            }
        }
        if let Some((start, end)) = run_range {
            if rel_col >= start && rel_col < end {
                // editor_pane_focus was set to `pane` by the click, so this runs the file
                // focused in the clicked pane (left or right).
                self.run_active_file();
            }
        }
    }

    /// The tab strip exactly as rendered for `pane` — the same call the renderer makes, so a
    /// click maps to the row the user sees.
    fn tab_strip(&self, tab_bar_width: u16, pane: EditorPane) -> ui::TabStrip {
        ui::tab_strip_layout(
            &ui::tab_widths(self, pane),
            ui::tab_strip_width(self, tab_bar_width),
            self.tab_offsets[pane.index()],
        )
    }

    /// Brings a pane's active tab into view, but only when the active tab has changed since the
    /// last time this ran. Doing it every frame is what broke scrolling left: the strip snapped
    /// back to the active tab immediately, so the `‹` arrow looked dead.
    pub fn reveal_active_tab(&mut self, pane: EditorPane, tab_bar_width: u16) {
        // Tracked by position in this pane's strip, not by buffer: the same buffer can sit at a
        // different place in each half, and scrolling is about the strip.
        let active = self.pane_tab_position(pane);
        let slot = pane.index();
        if self.tab_revealed[slot] == Some(active) {
            return;
        }
        self.tab_offsets[slot] = ui::offset_revealing(
            &ui::tab_widths(self, pane),
            ui::tab_strip_width(self, tab_bar_width),
            self.tab_offsets[slot],
            active,
        );
        self.tab_revealed[slot] = Some(active);
    }

    /// Scrolls a pane's tab strip by `delta` tabs, starting from what is on screen rather
    /// than from the stored offset, so the first step after an auto-scroll doesn't jump.
    fn scroll_tabs(&mut self, pane: EditorPane, delta: isize, tab_bar_width: u16) {
        let first = self.tab_strip(tab_bar_width, pane).first as isize;
        let last = self.pane_tabs(pane).len().saturating_sub(1) as isize;
        self.tab_offsets[pane.index()] = (first + delta).clamp(0, last) as usize;
    }

    fn position_cursor_from_click(&mut self, content_area: Rect, col: u16, row: u16) {
        let inner = ui::inner_rect(content_area);
        if col < inner.x || row < inner.y {
            return;
        }
        let gutter = ui::gutter_width(self.editor().rope.len_lines(), self.settings.show_line_numbers);
        let rel_row = (row - inner.y) as usize;
        let rel_col = (col - inner.x) as i32 - gutter as i32;
        let top_line = self.editor().top_line;
        let rows = self.editor().visible_rows_from(top_line, rel_row + 1);
        let target_line = *rows.last().unwrap_or(&top_line);
        self.editor_mut().cursor_line = target_line;
        if rel_col >= 0 {
            let left_col = self.editor().left_col;
            let target_col = left_col + rel_col as usize;
            let max_col = self.editor().line_char_len(target_line);
            self.editor_mut().cursor_col = target_col.min(max_col);
        } else {
            self.editor_mut().cursor_col = 0;
        }
    }

    fn handle_context_menu_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.context_menu = None,
            KeyCode::Up => {
                if let Some(m) = self.context_menu.as_mut() {
                    m.move_selection(-1);
                }
            }
            KeyCode::Down => {
                if let Some(m) = self.context_menu.as_mut() {
                    m.move_selection(1);
                }
            }
            KeyCode::Enter => {
                if let Some(action) = self.context_menu.as_ref().and_then(|m| m.selected_action()) {
                    self.context_menu = None;
                    self.run_menu_action(action);
                }
            }
            _ => {}
        }
    }

    /// Opens the context menu for the focused frame (Ctrl+Space), anchored near its top-left —
    /// the layout isn't handed to the key path, so it's recomputed from the last drawn size.
    fn open_context_menu_for_focus(&mut self) {
        let areas = ui::compute_layout(self.last_full, &ui::LayoutParams::from_app(self));
        let (target, rect) = match self.focus {
            Focus::FileTree => (ContextTarget::Sidebar, areas.sidebar.unwrap_or(self.last_full)),
            Focus::Terminal => {
                let rect = areas
                    .terminals
                    .as_ref()
                    .and_then(|t| t.get(self.active_terminal))
                    .copied()
                    .unwrap_or(self.last_full);
                (ContextTarget::Terminal, rect)
            }
            Focus::Editor => (ContextTarget::Editor, areas.editor),
        };
        self.context_menu = Some(ContextMenu::new(target, (rect.x + 2, rect.y + 1)));
    }

    /// Right-click: focus the frame under the pointer (selecting the clicked tree row first, so
    /// Rename/Delete act on it), then raise its context menu at the click.
    fn open_context_menu_at(&mut self, col: u16, row: u16, areas: &ui::Areas) {
        if let Some(sidebar) = areas.sidebar {
            if within(sidebar, col, row) {
                self.focus = Focus::FileTree;
                let inner = ui::inner_rect(sidebar);
                if row >= inner.y {
                    let idx = (row - inner.y) as usize;
                    if idx < self.file_tree.visible.len() {
                        self.file_tree.selected = idx;
                    }
                }
                self.context_menu = Some(ContextMenu::new(ContextTarget::Sidebar, (col, row)));
                return;
            }
        }
        if within(areas.editor, col, row) {
            self.focus = Focus::Editor;
            self.context_menu = Some(ContextMenu::new(ContextTarget::Editor, (col, row)));
            return;
        }
        if let Some(term_areas) = &areas.terminals {
            if let Some(i) = term_areas.iter().position(|r| within(*r, col, row)) {
                self.focus = Focus::Terminal;
                self.active_terminal = i;
                self.context_menu = Some(ContextMenu::new(ContextTarget::Terminal, (col, row)));
            }
        }
    }

    /// A click while the context menu is open: run the item under the pointer, or dismiss it.
    fn mouse_context_menu(&mut self, col: u16, row: u16) {
        let lang = self.settings.lang;
        let rect = match self.context_menu.as_ref().map(|m| ui::context_menu_rect(m, lang, self.last_full)) {
            Some(rect) => rect,
            None => return,
        };
        let inner = ui::inner_rect(rect);
        if !within(inner, col, row) {
            self.context_menu = None;
            return;
        }
        // Rows map to items, skipping the separator rules woven between groups.
        let target = (row - inner.y) as usize;
        let action = self.context_menu.as_ref().and_then(|m| {
            let mut display_row = 0;
            for item in &m.items {
                if item.new_group {
                    display_row += 1;
                }
                if display_row == target {
                    return Some(item.action);
                }
                display_row += 1;
            }
            None
        });
        if let Some(action) = action {
            self.context_menu = None;
            self.run_menu_action(action);
        }
    }

    fn scroll_manual(&mut self, delta: isize) {
        let sections = crate::manual::sections(self.settings.lang);
        let page = self.manual_page();
        if let Some(state) = self.manual.as_mut() {
            let len = sections.get(state.section).map(|s| s.body.len()).unwrap_or(0);
            state.scroll_by(delta, len, page);
        }
    }

    /// A click in the manual: a row of the section list jumps there, anywhere outside the
    /// frame closes it, and the text itself simply absorbs the click.
    fn mouse_manual(&mut self, col: u16, row: u16) {
        let full = self.last_full;
        let rect = ui::manual_rect(full);
        if !within(rect, col, row) {
            self.manual = None;
            return;
        }
        let list = ui::manual_list_rect(rect);
        if !within(list, col, row) || row < list.y {
            return;
        }
        let count = crate::manual::sections(self.settings.lang).len();
        let index = (row - list.y) as usize;
        if index < count {
            if let Some(state) = self.manual.as_mut() {
                state.select(index, count);
            }
        }
    }

    fn mouse_menu_bar_click(&mut self, col: u16) {
        let ranges = ui::menu_title_ranges(&self.menu, self.settings.lang);
        // Anything left of the first menu title is the logo.
        if ranges.first().is_some_and(|(first, _)| col < *first) {
            self.poke_turtle();
            return;
        }
        for (i, (start, end)) in ranges.iter().enumerate() {
            if col >= *start && col < *end {
                self.menu.menu_index = i;
                self.menu.open();
                return;
            }
        }
    }

    fn mouse_menu(&mut self, col: u16, row: u16, full: Rect) {
        let dropdown = ui::menu_dropdown_rect(&self.menu, self.settings.lang, full);
        if within(dropdown, col, row) {
            let inner = ui::inner_rect(dropdown);
            if row >= inner.y {
                // Separator rules occupy display rows too, so walk the items and
                // account for the extra row each group opener adds above itself.
                // A click that lands on a separator maps to no item and is ignored.
                let target = (row - inner.y) as usize;
                let mut display_row = 0;
                for (idx, item) in self.menu.defs[self.menu.menu_index].items.iter().enumerate() {
                    if item.new_group {
                        display_row += 1;
                    }
                    if display_row == target {
                        self.menu.item_index = idx;
                        if let Some(action) = self.menu.selected_action() {
                            self.menu.close();
                            self.run_menu_action(action);
                        }
                        break;
                    }
                    display_row += 1;
                }
            }
            return;
        }
        // While a menu is open, row 0 always holds the title bar — the real one, or the
        // overlay shown when the bar is otherwise hidden — so a click there switches menus.
        if row == 0 {
            self.mouse_menu_bar_click(col);
            return;
        }
        self.menu.close();
    }

    fn mouse_settings(&mut self, col: u16, row: u16, full: Rect) {
        let modal = ui::settings_modal_rect(full);
        if !within(modal, col, row) {
            return;
        }
        let inner = ui::inner_rect(modal);
        if row < inner.y {
            return;
        }
        let idx = (row - inner.y) as usize;
        if idx < settings::SETTINGS_COUNT {
            self.settings_selected = idx;
            self.settings.activate(idx);
            self.editor_mut().syntax_dirty = true;
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cleecode_app_test_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_venv(root: &std::path::Path, name: &str) -> PathBuf {
        let venv = root.join(name);
        std::fs::create_dir_all(venv.join(venv_bin_dir())).unwrap();
        std::fs::write(venv.join(venv_bin_dir()).join("activate"), "").unwrap();
        venv
    }

    #[test]
    fn available_venvs_merges_discovered_and_registered() {
        let root = setup_dir("venvs_root");
        make_venv(&root, ".venv");
        let elsewhere = setup_dir("venvs_elsewhere");
        let path = make_venv(&elsewhere, "central").to_string_lossy().into_owned();
        let registered = crate::settings::RegisteredVenv::Path(path.clone());

        let venvs = available_venvs(&root, std::slice::from_ref(&registered));
        assert_eq!(venvs, vec![".venv".to_string(), path]);

        // A registered path that no longer exists is dropped, not offered as a dead entry.
        let _ = std::fs::remove_dir_all(&elsewhere);
        assert_eq!(available_venvs(&root, &[registered]), vec![".venv".to_string()]);
    }

    #[test]
    fn available_venvs_does_not_duplicate_a_registered_discovered_venv() {
        let root = setup_dir("venvs_dup");
        make_venv(&root, ".venv");
        // Registering the project's own venv by its relative name must not list it twice.
        let venvs = available_venvs(&root, &[crate::settings::RegisteredVenv::Path(".venv".to_string())]);
        assert_eq!(venvs, vec![".venv".to_string()]);
    }

    #[test]
    fn path_query_only_triggers_on_a_path_like_query() {
        let root = std::path::Path::new("/work/project");
        let home = std::path::Path::new("/Users/someone");
        let q = |s: &str| path_query(s, root, Some(home));

        // A plain name stays a project-file search, which is the common case.
        assert_eq!(q("main.rs"), None);
        assert_eq!(q("src/lib"), None);
        assert_eq!(q(""), None);

        // Absolute: split into the directory to list and the fragment being typed.
        assert_eq!(q("/etc/ho"), Some((PathBuf::from("/etc"), "ho".to_string())));
        // A trailing slash lists that directory whole.
        assert_eq!(q("/etc/"), Some((PathBuf::from("/etc"), String::new())));
        // Home-relative and root-relative forms.
        assert_eq!(q("~/notes"), Some((home.to_path_buf(), "notes".to_string())));
        assert_eq!(q("~/"), Some((home.to_path_buf(), String::new())));
        assert_eq!(q("./src/ma"), Some((root.join("./src"), "ma".to_string())));
        assert_eq!(q("../oth"), Some((PathBuf::from("/work/project/.."), "oth".to_string())));
        // Without a home directory, `~` can't be resolved and is not treated as a path.
        assert_eq!(path_query("~/x", root, None), None);
    }

    #[test]
    fn list_dir_entries_puts_directories_first_and_hides_dotfiles() {
        let dir = setup_dir("listing");
        std::fs::create_dir_all(dir.join("zsub")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden_dir")).unwrap();
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        std::fs::write(dir.join(".hidden"), "h").unwrap();

        let names = |show_hidden: bool| {
            list_dir_entries(&dir, show_hidden)
                .into_iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        // The directory sorts before the file even though its name is later alphabetically.
        assert_eq!(names(false), vec!["zsub".to_string(), "a.txt".to_string()]);
        let shown = names(true);
        assert!(shown.contains(&".hidden".to_string()) && shown.contains(&".hidden_dir".to_string()));
        // Still directories first when hidden entries are shown.
        assert_eq!(shown[0], ".hidden_dir");

        // An unreadable or missing directory is empty, not an error: the user may still be typing.
        assert!(list_dir_entries(&dir.join("nope"), true).is_empty());
    }

    #[test]
    fn resize_command_maps_focused_borders_to_seams() {
        use ResizeSide::*;
        // Classic layout: sidebar left, terminal a full-width strip below, editor unsplit.
        let classic = ResizeLayout { focus: Focus::Editor, ..classic_like() };
        // Editor grows left by eating the sidebar; shrinking left gives it back.
        assert_eq!(resize_command(&classic, Left, true), Some(ResizeCmd::Sidebar(-SIDEBAR_STEP)));
        assert_eq!(resize_command(&classic, Left, false), Some(ResizeCmd::Sidebar(SIDEBAR_STEP)));
        // Editor grows down by eating the terminal.
        assert_eq!(resize_command(&classic, Down, true), Some(ResizeCmd::Terminal(-TERMINAL_STEP)));
        // Right and top are window edges here — nothing to move.
        assert_eq!(resize_command(&classic, Right, true), None);
        assert_eq!(resize_command(&classic, Up, true), None);

        // Sidebar focused: its right edge is its width; its bottom meets the terminal strip.
        let sidebar = ResizeLayout { focus: Focus::FileTree, ..classic_like() };
        assert_eq!(resize_command(&sidebar, Right, true), Some(ResizeCmd::Sidebar(SIDEBAR_STEP)));
        assert_eq!(resize_command(&sidebar, Down, true), Some(ResizeCmd::Terminal(-TERMINAL_STEP)));
        assert_eq!(resize_command(&sidebar, Left, true), None);

        // Terminal on the right: its left edge is the only movable seam.
        let term_right = ResizeLayout { focus: Focus::Terminal, terminal_on_right: true, ..classic_like() };
        assert_eq!(resize_command(&term_right, Left, true), Some(ResizeCmd::Terminal(TERMINAL_STEP)));
        assert_eq!(resize_command(&term_right, Up, true), None);

        // Split view, right pane focused, terminal on the right: left edge is the split seam,
        // right edge is the terminal seam.
        let split_right = ResizeLayout {
            focus: Focus::Editor,
            editor_pane: EditorPane::Right,
            split_view: true,
            terminal_on_right: true,
            ..classic_like()
        };
        assert_eq!(resize_command(&split_right, Left, true), Some(ResizeCmd::Split(-SPLIT_STEP)));
        assert_eq!(resize_command(&split_right, Right, true), Some(ResizeCmd::Terminal(-TERMINAL_STEP)));

        // Left pane's right edge is the split seam; growing it enlarges the left pane.
        let split_left = ResizeLayout { editor_pane: EditorPane::Left, ..split_right };
        assert_eq!(resize_command(&split_left, Right, true), Some(ResizeCmd::Split(SPLIT_STEP)));
        assert_eq!(resize_command(&split_left, Left, true), Some(ResizeCmd::Sidebar(-SIDEBAR_STEP)));
    }

    fn classic_like() -> ResizeLayout {
        ResizeLayout {
            focus: Focus::Editor,
            editor_pane: EditorPane::Left,
            split_view: false,
            show_sidebar: true,
            show_terminal: true,
            terminal_on_right: false,
            terminal_index: 0,
            terminal_count: 1,
        }
    }

    /// Ctrl+<direction> is meant to read as "go to the thing over there", so what matters is that
    /// each arrow lands on whatever is actually in that direction and stops at the window edge
    /// instead of wrapping.
    /// The walk has to actually end, or the turtle would sit on the last column for the rest of
    /// the session; and hurrying it has to move it without letting it skip the finish.
    #[test]
    fn the_turtle_crosses_and_then_is_gone() {
        const W: u16 = 80;
        assert_eq!(turtle_column(Duration::ZERO, 0, W), Some(W - 2), "starts against the right edge");

        let half = turtle_column(TURTLE_CROSSING / 2, 0, W).expect("still walking half way");
        assert!((38..=42).contains(&half), "half the time is about half the way, got {half}");

        // Past the end it is over, and stays over.
        assert_eq!(turtle_column(TURTLE_CROSSING, 0, W), None);
        assert_eq!(turtle_column(TURTLE_CROSSING * 3, 0, W), None);

        // A nudge buys ground — leftward — and enough of them end the walk early.
        let nudged = turtle_column(TURTLE_CROSSING / 2, TURTLE_NUDGE, W).expect("still walking");
        assert_eq!(nudged, half - TURTLE_NUDGE, "hurrying moves it towards the left edge");
        assert_eq!(turtle_column(TURTLE_CROSSING / 2, W, W), None, "hurried right off the end");

        // A window with no room is not a walk.
        assert_eq!(turtle_column(Duration::ZERO, 0, 0), None);
    }

    #[test]
    fn a_direction_lands_on_whatever_frame_lies_that_way() {
        use FocusTarget::*;
        use ResizeSide::*;

        // Classic: tree | editor, with the terminal strip spanning the width below both.
        let tree = ResizeLayout { focus: Focus::FileTree, ..classic_like() };
        assert_eq!(focus_neighbour(&tree, Right), Some(Editor(EditorPane::Left)));
        assert_eq!(focus_neighbour(&tree, Down), Some(Terminal(0)));
        assert_eq!(focus_neighbour(&tree, Left), None, "the tree is against the window edge");
        assert_eq!(focus_neighbour(&tree, Up), None);

        let editor = classic_like();
        assert_eq!(focus_neighbour(&editor, Left), Some(Tree));
        assert_eq!(focus_neighbour(&editor, Down), Some(Terminal(0)));
        assert_eq!(focus_neighbour(&editor, Up), None);

        // Terminals below tile side by side, so they are walked left/right, and up leaves them.
        let term = ResizeLayout { focus: Focus::Terminal, terminal_count: 3, terminal_index: 1, ..classic_like() };
        assert_eq!(focus_neighbour(&term, Left), Some(Terminal(0)));
        assert_eq!(focus_neighbour(&term, Right), Some(Terminal(2)));
        assert_eq!(focus_neighbour(&term, Up), Some(Editor(EditorPane::Left)));
        let first = ResizeLayout { terminal_index: 0, ..term };
        assert_eq!(focus_neighbour(&first, Left), None, "no window to the left of the first");
        let last = ResizeLayout { terminal_index: 2, ..term };
        assert_eq!(focus_neighbour(&last, Right), None, "and none past the last");

        // With the panel on the right the windows stack, so the same walk is up/down instead.
        let right = ResizeLayout {
            focus: Focus::Terminal,
            terminal_on_right: true,
            terminal_count: 3,
            terminal_index: 1,
            ..classic_like()
        };
        assert_eq!(focus_neighbour(&right, Up), Some(Terminal(0)));
        assert_eq!(focus_neighbour(&right, Down), Some(Terminal(2)));
        assert_eq!(focus_neighbour(&right, Left), Some(Editor(EditorPane::Left)));
        assert_eq!(focus_neighbour(&right, Right), None);
    }

    /// The two halves of a split editor are frames in their own right: the same arrow that leaves
    /// the editor for the sidebar has to cross the split first.
    #[test]
    fn a_split_editor_is_two_frames_to_the_arrows() {
        use FocusTarget::*;
        use ResizeSide::*;
        let split = ResizeLayout { split_view: true, ..classic_like() };

        assert_eq!(focus_neighbour(&split, Right), Some(Editor(EditorPane::Right)));
        assert_eq!(focus_neighbour(&split, Left), Some(Tree), "from the left half, out to the tree");

        let on_right = ResizeLayout { editor_pane: EditorPane::Right, ..split };
        assert_eq!(focus_neighbour(&on_right, Left), Some(Editor(EditorPane::Left)));

        // Coming back from a terminal lands in the half nearest it.
        let term = ResizeLayout { focus: Focus::Terminal, ..split };
        assert_eq!(focus_neighbour(&term, Up), Some(Editor(EditorPane::Right)));
    }

    /// A hidden frame is not somewhere you can go.
    #[test]
    fn arrows_skip_frames_that_are_not_on_screen() {
        use ResizeSide::*;
        let no_sidebar = ResizeLayout { show_sidebar: false, ..classic_like() };
        assert_eq!(focus_neighbour(&no_sidebar, Left), None);

        let no_terminal = ResizeLayout { show_terminal: false, ..classic_like() };
        assert_eq!(focus_neighbour(&no_terminal, Down), None);
    }

    /// The seams *between* terminal windows used to be mouse-only: resize mode plus arrows did nothing
    /// along the tiling axis.
    #[test]
    fn resize_moves_the_seam_between_terminal_windows() {
        use ResizeSide::*;
        // Three windows side by side under the editor, the middle one focused.
        let strip = ResizeLayout {
            focus: Focus::Terminal,
            terminal_index: 1,
            terminal_count: 3,
            ..classic_like()
        };
        // Growing rightwards takes from the window after it; growing leftwards, from the one
        // before — in both cases the focused window ends up bigger.
        assert_eq!(resize_command(&strip, Right, true), Some(ResizeCmd::TerminalWeight { seam: 1, delta: WEIGHT_STEP }));
        assert_eq!(resize_command(&strip, Left, true), Some(ResizeCmd::TerminalWeight { seam: 0, delta: -WEIGHT_STEP }));
        assert_eq!(resize_command(&strip, Left, false), Some(ResizeCmd::TerminalWeight { seam: 0, delta: WEIGHT_STEP }));
        // Across the axis it is still the editor seam, and the outer edge is still nothing.
        assert_eq!(resize_command(&strip, Up, true), Some(ResizeCmd::Terminal(TERMINAL_STEP)));
        assert_eq!(resize_command(&strip, Down, true), None);

        // The first window has no neighbour to its left, the last none to its right.
        let first = ResizeLayout { terminal_index: 0, ..strip };
        assert_eq!(resize_command(&first, Left, true), None);
        let last = ResizeLayout { terminal_index: 2, ..strip };
        assert_eq!(resize_command(&last, Right, true), None);

        // Stacked on the right instead: the same seams, now vertical, and the editor seam
        // moves to the left border.
        let stacked = ResizeLayout { terminal_on_right: true, terminal_index: 0, terminal_count: 2, ..strip };
        assert_eq!(
            resize_command(&stacked, Down, true),
            Some(ResizeCmd::TerminalWeight { seam: 0, delta: WEIGHT_STEP })
        );
        assert_eq!(resize_command(&stacked, Left, true), Some(ResizeCmd::Terminal(TERMINAL_STEP)));
        assert_eq!(resize_command(&stacked, Up, true), None);

        // A single window has no seam of its own at all.
        let alone = ResizeLayout { terminal_count: 1, terminal_index: 0, ..strip };
        assert_eq!(resize_command(&alone, Right, true), None);
        assert_eq!(resize_command(&alone, Left, true), None);
    }

    #[test]
    fn venv_browse_lists_folders_only_and_flags_venvs() {
        let dir = setup_dir("venv_browse");
        make_venv(&dir, ".venv"); // hidden, and the whole reason for the browser
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("readme.md"), "").unwrap();

        let items = venv_browse_items(&dir);
        // The file is gone; both directories remain, each with a trailing slash.
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec![".venv/", "src/"]);

        // Only the venv carries the "venv" flag, and its action targets the real path.
        let venv = &items[0];
        assert_eq!(venv.shortcut.as_deref(), Some("venv"));
        assert!(matches!(&venv.action, crate::picker::PickAction::VenvDir(p) if p == &dir.join(".venv")));
        assert_eq!(items[1].shortcut, None);
    }

    #[test]
    fn effective_venv_ignores_one_that_is_not_here() {
        let available = vec![".venv".to_string(), "/opt/venvs/ml".to_string()];
        assert_eq!(effective_venv(Some(".venv"), &available), Some(".venv"));
        assert_eq!(effective_venv(Some("/opt/venvs/ml"), &available), Some("/opt/venvs/ml"));
        // Remembered from another project, absent here: reported as none, which is what running
        // a file would actually do.
        assert_eq!(effective_venv(Some(".venv-old"), &available), None);
        assert_eq!(effective_venv(Some(".venv"), &[]), None);
        assert_eq!(effective_venv(None, &available), None);
    }

    #[test]
    fn python_run_rows_map_positions_to_actions() {
        let registered = vec![crate::settings::RegisteredVenv::Named {
            name: "ml".to_string(),
            path: "/opt/venvs/ml-3.12".to_string(),
        }];
        let available = vec![".venv".to_string(), "/opt/venvs/ml-3.12".to_string()];
        let commands = std::collections::HashMap::new();
        let none = std::collections::HashMap::new();
        let rows = run_rows(
            "py",
            Some("/opt/venvs/ml-3.12"),
            &available,
            &registered,
            &commands,
            &none,
            Lang::En,
        );

        // "no venv", one row per venv, browse, register, then the two run-command rows.
        assert_eq!(rows.len(), 7);
        assert!(matches!(rows[0].action, RunRowAction::SelectVenv(None)));
        assert!(matches!(rows[1].action, RunRowAction::SelectVenv(Some(ref v)) if v == ".venv"));
        assert!(matches!(rows[3].action, RunRowAction::Browse));
        assert!(matches!(rows[4].action, RunRowAction::Register));
        assert!(matches!(rows[5].action, RunRowAction::EditCommand(RunScope::Global)));
        assert!(matches!(rows[6].action, RunRowAction::EditCommand(RunScope::Project)));

        // The nickname is the label; the path stays as the dimmed detail.
        assert_eq!(rows[2].label, "ml");
        assert_eq!(rows[2].detail.as_deref(), Some("/opt/venvs/ml-3.12"));
        // A project-root venv's label already *is* its path, so it carries no detail to repeat.
        assert_eq!(rows[1].label, ".venv");
        assert_eq!(rows[1].detail, None);
        // Exactly the venv in use is marked.
        assert!(rows[2].active);
        assert!(!rows[0].active && !rows[1].active && !rows[3].active && !rows[4].active);

        // With no venv selected, the marker moves to the first row.
        let rows = run_rows("py", None, &available, &registered, &commands, &none, Lang::En);
        assert!(rows[0].active);
    }

    #[test]
    fn non_python_run_rows_only_offer_the_command() {
        let available = vec![".venv".to_string()];
        let commands =
            std::collections::HashMap::from([("tex".to_string(), "pdflatex {file}".to_string())]);
        let none = std::collections::HashMap::new();

        // A venv means nothing to pdflatex, so the venv list stays out of the way entirely and
        // the two command rows are the whole of what decides how a .tex file runs.
        let rows = run_rows("tex", Some(".venv"), &available, &[], &commands, &none, Lang::En);
        assert_eq!(rows.len(), 2);
        assert!(matches!(rows[0].action, RunRowAction::EditCommand(RunScope::Global)));
        assert_eq!(rows[0].detail.as_deref(), Some("pdflatex {file}"));
        assert!(matches!(rows[1].action, RunRowAction::EditCommand(RunScope::Project)));

        // An extension with no command still gets both rows — that is how one is set.
        let rows = run_rows("md", None, &available, &[], &commands, &none, Lang::En);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].detail, None);
        assert!(matches!(rows[0].action, RunRowAction::EditCommand(RunScope::Global)));
    }

    /// The marker is the only thing in the menu that answers "which of these two wins", and
    /// getting it wrong is how you typeset the wrong master file and blame the editor.
    #[test]
    fn the_marker_follows_the_command_that_would_actually_run() {
        let global =
            std::collections::HashMap::from([("tex".to_string(), "pdflatex {file}".to_string())]);
        let overridden =
            std::collections::HashMap::from([("tex".to_string(), "latexmk main.tex".to_string())]);
        let none = std::collections::HashMap::new();

        // No override: the shared command is in force.
        let rows = run_rows("tex", None, &[], &[], &global, &none, Lang::En);
        assert!(rows[0].active && !rows[1].active);
        assert_eq!(rows[1].detail, None, "nothing to show for a project that overrides nothing");

        // Overridden: the marker moves, and both commands stay visible so the one being
        // shadowed is not a mystery.
        let rows = run_rows("tex", None, &[], &[], &global, &overridden, Lang::En);
        assert!(!rows[0].active && rows[1].active);
        assert_eq!(rows[0].detail.as_deref(), Some("pdflatex {file}"));
        assert_eq!(rows[1].detail.as_deref(), Some("latexmk main.tex"));

        // An override with no global command behind it still wins, and nothing is marked as
        // shared because there is nothing shared to mark.
        let rows = run_rows("tex", None, &[], &[], &none, &overridden, Lang::En);
        assert!(!rows[0].active && rows[1].active);
    }

    /// Closing a buffer renumbers both strips. Getting this wrong does not show up as the wrong
    /// tab being highlighted — it shows up later, as an index that no longer exists being handed
    /// to a draw, in an app whose whole promise is that it does not fall over on you.
    #[test]
    fn closing_a_buffer_renumbers_both_strips() {
        // Six buffers dealt between the halves, deliberately out of order and interleaved.
        let mut tabs = [vec![0, 3, 1], vec![4, 2, 5]];
        forget_buffer(&mut tabs, 2);
        // 2 is gone from the half that held it; 3, 4 and 5 each shift down one, wherever they
        // were, and the order within each strip survives.
        assert_eq!(tabs, [vec![0, 2, 1], vec![3, 4]]);

        // Removing the first shifts everything.
        let mut tabs = [vec![0, 1], vec![2]];
        forget_buffer(&mut tabs, 0);
        assert_eq!(tabs, [vec![0], vec![1]]);

        // Removing the last leaves the rest alone.
        let mut tabs = [vec![0, 1], vec![2]];
        forget_buffer(&mut tabs, 2);
        assert_eq!(tabs, [vec![0, 1], vec![]]);

        // A buffer no strip holds still renumbers the ones above it, and removes nothing.
        let mut tabs = [vec![0, 5], vec![7]];
        forget_buffer(&mut tabs, 3);
        assert_eq!(tabs, [vec![0, 4], vec![6]]);

        // Emptying a strip entirely is allowed here; giving that half something to show again
        // is `settle_panes`'s job, not this one's.
        let mut tabs = [vec![0], vec![1]];
        forget_buffer(&mut tabs, 1);
        assert_eq!(tabs, [vec![0], vec![]]);
    }

    #[test]
    fn placeholders_expand_to_the_files_parts_and_survive_spaces() {
        let path = std::path::Path::new("/work/my papers/report.tex");
        let expanded = expand_placeholders(
            "pdflatex -output-directory {dir} {file} && open {dir}/{stem}.pdf",
            path,
        );
        assert_eq!(
            expanded,
            "pdflatex -output-directory '/work/my papers' '/work/my papers/report.tex' \
             && open '/work/my papers'/report.pdf"
        );

        // {name} is the file name with its extension; a bare relative path has no folder of
        // its own, which as a directory means "here".
        let expanded = expand_placeholders("cd {dir} && lint {name}", std::path::Path::new("main.py"));
        assert_eq!(expanded, "cd . && lint main.py");
    }

    #[test]
    fn save_as_path_is_relative_to_the_project_root() {
        let root = std::path::Path::new("/work/project");
        let home = std::path::Path::new("/Users/someone");
        let resolve = |input: &str| resolve_save_as_path(input, root, Some(home));

        assert_eq!(resolve("notes.md"), Some(root.join("notes.md")));
        assert_eq!(resolve("src/lib.rs"), Some(root.join("src/lib.rs")));
        // Absolute and home-relative names are taken as written.
        assert_eq!(resolve("/tmp/out.txt"), Some(PathBuf::from("/tmp/out.txt")));
        assert_eq!(resolve("~/out.txt"), Some(home.join("out.txt")));
        // Surrounding whitespace is a typo, not part of the name.
        assert_eq!(resolve("  notes.md  "), Some(root.join("notes.md")));
        // Nothing to save to.
        assert_eq!(resolve(""), None);
        assert_eq!(resolve("   "), None);
        // Without a home directory `~` can't be resolved, so it is refused rather than
        // creating a file literally named "~".
        assert_eq!(resolve_save_as_path("~/out.txt", root, None), None);
    }

    #[test]
    fn saving_a_buffer_with_no_name_fails_instead_of_reporting_success() {
        // The bug this guards: save() used to return Ok(()) without writing anything, so the
        // quit prompt's "save" silently discarded the buffer.
        let mut editor = crate::editor::Editor::empty();
        editor.insert_char('x');
        assert!(editor.dirty);
        assert!(editor.save().is_err(), "an unnamed buffer must not report a successful save");
        assert!(editor.dirty, "and must stay dirty, so the work is still known to be unsaved");
    }

    #[test]
    fn cell_at_is_relative_to_the_pane_and_clamps_to_it() {
        let inner = Rect { x: 10, y: 5, width: 4, height: 3 };
        assert_eq!(cell_at(inner, 10, 5), Some((0, 0)));
        assert_eq!(cell_at(inner, 12, 6), Some((1, 2)));
        // Outside the pane clamps to its edges, so a drag that wanders off still selects up
        // to the border instead of being dropped.
        assert_eq!(cell_at(inner, 0, 0), Some((0, 0)));
        assert_eq!(cell_at(inner, 99, 99), Some((2, 3)));
        // A collapsed pane has no cells to point at.
        assert_eq!(cell_at(Rect { width: 0, ..inner }, 10, 5), None);
    }

    #[test]
    fn version_key_orders_numerically_not_lexicographically() {
        assert!(version_key("Octave-10.1.0") > version_key("Octave-9.2.0"));
        assert_eq!(version_key("Octave-9.2.0"), vec![9, 2, 0]);
        assert_eq!(version_key("no-digits"), Vec::<u64>::new());
    }

    #[test]
    fn discovers_newest_octave_under_program_files() {
        let pf = setup_dir("program_files");
        for version in ["Octave-9.2.0", "Octave-10.1.0"] {
            let bin = pf.join("GNU Octave").join(version).join("mingw64").join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::write(bin.join("octave-cli.exe"), "").unwrap();
        }
        let found = discover_octave(Some(&pf)).unwrap();
        assert!(found.to_string_lossy().contains("Octave-10.1.0"), "got {found:?}");

        // No Program Files (the non-Windows case): nothing to discover, PATH is used instead.
        assert!(discover_octave(None).is_none());
        assert!(discover_octave(Some(&setup_dir("program_files_empty"))).is_none());
    }

    #[test]
    fn resolve_interpreter_substitutes_configured_path_and_keeps_args() {
        let dir = setup_dir("interp");
        let exe = dir.join("octave-cli");
        std::fs::write(&exe, "").unwrap();
        let paths: std::collections::HashMap<String, String> =
            [("octave-cli".to_string(), exe.to_string_lossy().into_owned())].into_iter().collect();

        let out = resolve_interpreter("octave-cli {file}", &paths, None);
        assert_eq!(out, format!("{} {{file}}", exe.display()));

        // Unconfigured programs, and configured paths that no longer exist, are left alone.
        assert_eq!(resolve_interpreter("node {file}", &paths, None), "node {file}");
        let stale: std::collections::HashMap<String, String> =
            [("node".to_string(), dir.join("missing").to_string_lossy().into_owned())]
                .into_iter()
                .collect();
        assert_eq!(resolve_interpreter("node {file}", &stale, None), "node {file}");
    }

    #[test]
    fn resolve_interpreter_quotes_paths_with_spaces() {
        let dir = setup_dir("interp spaced");
        let exe = dir.join("octave-cli");
        std::fs::write(&exe, "").unwrap();
        let paths: std::collections::HashMap<String, String> =
            [("octave".to_string(), exe.to_string_lossy().into_owned())].into_iter().collect();

        let out = resolve_interpreter("octave {file}", &paths, None);
        let (program, rest) = out.rsplit_once(' ').unwrap();
        assert_eq!(rest, "{file}");
        // The space in the directory name must survive as one argument for the shell.
        assert_eq!(shell_words::split(program).unwrap(), vec![exe.to_string_lossy().into_owned()]);
    }
}
