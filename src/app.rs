use crate::clipboard::Clipboard;
use crate::dnd;
use crate::editor::Editor;
use crate::file_tree::{Activation, FileTree};
use crate::highlight::Highlighter;
use crate::i18n::{self, Key, Lang};
use crate::menu::{MenuAction, MenuBar};
use crate::settings::{self, Settings};
use crate::terminal_panel::{key_to_bytes, TerminalPanel};
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

/// One row of the venv drop-down.
pub struct VenvRow {
    pub label: String,
    /// Full path, shown dimmed after the label when the label alone is ambiguous.
    pub detail: Option<String>,
    /// Whether this is the venv currently in use, marked in the list.
    pub active: bool,
    pub action: VenvRowAction,
}

/// The drop-down's rows: "no venv" first, then every available venv, then the register entry.
/// A free function so the index-to-action mapping a click relies on can be tested without
/// standing up an App (which would need real ptys).
fn venv_rows(
    active: Option<&str>,
    available: &[String],
    registered: &[settings::RegisteredVenv],
    lang: Lang,
) -> Vec<VenvRow> {
    let mut rows = vec![VenvRow {
        label: i18n::t(lang, Key::ToolbarVenvNone).to_string(),
        detail: None,
        active: active.is_none(),
        action: VenvRowAction::Select(None),
    }];
    for venv in available {
        let label = ui::venv_display_name(venv, registered);
        rows.push(VenvRow {
            // The full path, dimmed, so two venvs with the same folder name stay tellable
            // apart — but not when it would just repeat the label, as it does for the plain
            // project-root venvs.
            detail: (*venv != label).then(|| venv.clone()),
            label,
            active: active == Some(venv.as_str()),
            action: VenvRowAction::Select(Some(venv.clone())),
        });
    }
    rows.push(VenvRow {
        label: i18n::t(lang, Key::VenvRegisterItem).to_string(),
        detail: None,
        active: false,
        action: VenvRowAction::Register,
    });
    rows
}

pub enum VenvRowAction {
    /// Use this venv, or the system python for `None`.
    Select(Option<String>),
    /// Open the box that registers a venv from elsewhere on disk.
    Register,
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
    pub tab_offsets: [usize; 2],
    /// The active tab each pane's offset was last reconciled against, so the strip is only
    /// scrolled to reveal the active tab when that tab actually changes.
    tab_revealed: [Option<usize>; 2],
    pub terminals: Vec<TerminalPanel>,
    pub active_terminal: usize,
    pub focus: Focus,
    pub should_quit: bool,
    pub status_message: String,
    pub editor_viewport: (usize, usize),
    pub settings: Settings,
    pub show_settings: bool,
    pub settings_selected: usize,
    pub highlighter: Highlighter,
    pub menu: MenuBar,
    pub show_about: bool,
    pub clipboard: Clipboard,
    pub show_splash: bool,
    pub splash_started: Instant,
    pub show_delete_confirm: bool,
    pub delete_target: Option<PathBuf>,
    pub show_rename: bool,
    pub rename_target: Option<PathBuf>,
    pub rename_input: String,
    /// Selected row while the venv drop-down is open under its toolbar button.
    pub venv_dropdown: Option<usize>,
    /// Which step the "register a venv" box is on, when it's open.
    pub venv_register: Option<VenvRegisterStep>,
    pub venv_register_input: String,
    /// The path accepted in step one, waiting for its nickname in step two.
    venv_register_path: Option<PathBuf>,
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
    last_tree_click: Option<(usize, Instant)>,
    pub git_status: std::collections::HashMap<PathBuf, crate::git_status::FileStatus>,
    git_status_tx: Sender<std::collections::HashMap<PathBuf, crate::git_status::FileStatus>>,
    git_status_rx: Receiver<std::collections::HashMap<PathBuf, crate::git_status::FileStatus>>,
    git_status_pending: Arc<AtomicBool>,
    bg_tx: Sender<String>,
    bg_rx: Receiver<String>,
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
    TextSelection,
    /// Selecting text inside an embedded terminal, in the pane the drag started in.
    TerminalSelection(usize),
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
        let t1 = TerminalPanel::new(term_rows, half_cols, &root)?;
        let t2 = TerminalPanel::new(term_rows, half_cols, &root)?;
        let (bg_tx, bg_rx) = mpsc::channel();
        let (git_status_tx, git_status_rx) = mpsc::channel();
        let git_status_pending = Arc::new(AtomicBool::new(false));
        spawn_git_status_refresh(root.clone(), git_status_tx.clone(), git_status_pending.clone());
        let settings = Settings::load();
        let available_venvs = available_venvs(&root, &settings.registered_venvs);
        let file_tree = FileTree::new(root.clone(), settings.show_hidden_files);
        Ok(App {
            file_tree,
            root,
            editors: vec![Editor::empty()],
            active_editor: 0,
            split_view: false,
            active_editor_right: 0,
            editor_pane_focus: EditorPane::Left,
            tab_offsets: [0, 0],
            tab_revealed: [None, None],
            terminals: vec![t1, t2],
            active_terminal: 0,
            focus: Focus::FileTree,
            should_quit: false,
            status_message: i18n::t(Lang::default(), Key::StatusHelp).to_string(),
            editor_viewport: (0, 0),
            settings,
            show_settings: false,
            settings_selected: 0,
            highlighter: Highlighter::new(),
            menu: MenuBar::new(),
            show_about: false,
            clipboard: Clipboard::new(),
            show_splash: true,
            splash_started: Instant::now(),
            show_delete_confirm: false,
            delete_target: None,
            show_rename: false,
            rename_target: None,
            rename_input: String::new(),
            venv_dropdown: None,
            venv_register: None,
            venv_register_input: String::new(),
            venv_register_path: None,
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
            last_tree_click: None,
            git_status: std::collections::HashMap::new(),
            git_status_tx,
            git_status_rx,
            git_status_pending,
            bg_tx,
            bg_rx,
        })
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

    pub fn poll_terminal_exits(&mut self) {
        let before = self.terminals.len();
        self.terminals.retain(|t| !t.exited.load(Ordering::Relaxed));
        if self.terminals.is_empty() {
            if let Ok(t) = TerminalPanel::new(24, 80, &self.root) {
                self.terminals.push(t);
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
            self.terminals
                .get(self.active_terminal)
                .and_then(|t| t.child_pid())
                .and_then(dnd::detect_ssh_target)
        };
        if let Some(target) = ssh_target {
            self.scp_paths_background(target, paths);
        } else if let Some(term) = self.terminals.get_mut(self.active_terminal) {
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
    fn active_editor_index(&self) -> usize {
        if self.split_view && self.editor_pane_focus == EditorPane::Right {
            self.active_editor_right.min(self.editors.len().saturating_sub(1))
        } else {
            self.active_editor
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
            if self.editors.len() > 1 && self.active_editor_right == self.active_editor {
                self.active_editor_right = (self.active_editor + 1) % self.editors.len();
            }
        } else {
            self.editor_pane_focus = EditorPane::Left;
        }
    }

    pub fn poll_external_changes(&mut self) {
        let lang = self.settings.lang;
        if let Some(msg) = self.editor_mut().check_external_changes(lang) {
            self.status_message = msg;
        }
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
        if let Some(idx) = self.editors.iter().position(|e| e.path.as_deref() == Some(path.as_path())) {
            self.active_editor = idx;
            self.focus = Focus::Editor;
            self.status_message = i18n::msg_opened(lang, &self.editors[idx].title(lang));
            return;
        }
        match Editor::open(path) {
            Ok(editor) => {
                if self.editors.len() == 1 && self.editors[0].path.is_none() && !self.editors[0].dirty {
                    self.editors[0] = editor;
                    self.active_editor = 0;
                } else {
                    self.editors.push(editor);
                    self.active_editor = self.editors.len() - 1;
                }
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

    fn open_command_palette(&mut self) {
        let lang = self.settings.lang;
        let mut items = Vec::new();
        for def in crate::menu::menu_defs() {
            let menu_title = i18n::t(lang, def.title_key);
            for it in def.items {
                let label = format!("{}: {}", menu_title, i18n::t(lang, it.label_key));
                items.push(crate::picker::PickItem {
                    label,
                    shortcut: it.shortcut.map(|s| s.to_string()),
                    action: crate::picker::PickAction::Command(it.action),
                });
            }
        }
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
                self.refresh_file_picker();
            }
            KeyCode::Char(c) if !ctrl => {
                if let Some(p) = self.picker.as_mut() {
                    p.push_char(c);
                }
                self.refresh_file_picker();
            }
            _ => {}
        }
    }

    fn execute_picker_selection(&mut self) {
        let mut cmd = None;
        let mut file = None;
        if let Some(action) = self.picker.as_ref().and_then(|p| p.selected_action()) {
            match action {
                crate::picker::PickAction::Command(a) => cmd = Some(*a),
                crate::picker::PickAction::OpenFile(p) => file = Some(p.clone()),
            }
        }
        if let Some(a) = cmd {
            self.picker = None;
            self.run_menu_action(a);
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
        if self.editors.len() <= 1 {
            self.editors[0] = Editor::empty();
            self.active_editor = 0;
            self.active_editor_right = 0;
            return;
        }
        self.editors.remove(idx);
        if idx < self.active_editor {
            self.active_editor -= 1;
        }
        if self.active_editor >= self.editors.len() {
            self.active_editor = self.editors.len() - 1;
        }
        if idx < self.active_editor_right {
            self.active_editor_right -= 1;
        }
        if self.active_editor_right >= self.editors.len() {
            self.active_editor_right = self.editors.len() - 1;
        }
        // Closing tabs shortens the strip, so an offset past the end would blank it out.
        let last = self.editors.len() - 1;
        for offset in &mut self.tab_offsets {
            *offset = (*offset).min(last);
        }
    }

    fn cycle_editor(&mut self, forward: bool) {
        if self.editors.is_empty() {
            return;
        }
        let len = self.editors.len();
        self.active_editor = if forward {
            (self.active_editor + 1) % len
        } else {
            (self.active_editor + len - 1) % len
        };
    }

    fn set_root(&mut self, new_root: PathBuf) {
        self.file_tree = FileTree::new(new_root.clone(), self.settings.show_hidden_files);
        self.root = new_root;
        self.available_venvs = available_venvs(&self.root, &self.settings.registered_venvs);
        spawn_git_status_refresh(self.root.clone(), self.git_status_tx.clone(), self.git_status_pending.clone());
        self.status_message = i18n::msg_project_folder(self.settings.lang, &self.root.display().to_string());
    }

    fn toggle_hidden_files(&mut self) {
        // The setting is the single source of truth; the tree follows it. Flipping both
        // independently let them drift apart.
        self.settings.show_hidden_files = !self.settings.show_hidden_files;
        self.file_tree.set_show_hidden(self.settings.show_hidden_files);
    }

    pub fn new_terminal(&mut self) {
        let lang = self.settings.lang;
        match TerminalPanel::new(24, 80, &self.root) {
            Ok(t) => {
                self.terminals.push(t);
                self.active_terminal = self.terminals.len() - 1;
                self.settings.show_terminal = true;
                self.focus = Focus::Terminal;
                self.status_message = i18n::msg_new_terminal(lang, self.terminals.len());
            }
            Err(e) => self.status_message = i18n::msg_terminal_create_error(lang, &e.to_string()),
        }
    }

    pub fn close_active_terminal(&mut self) {
        if self.terminals.len() <= 1 {
            self.status_message = i18n::msg_min_one_terminal(self.settings.lang);
            return;
        }
        self.terminals.remove(self.active_terminal);
        if self.active_terminal >= self.terminals.len() {
            self.active_terminal = self.terminals.len() - 1;
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
        let Some(path) = self.editor().path.clone() else {
            self.status_message = i18n::msg_run_no_file(lang);
            return;
        };
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let Some(template) = self.settings.run_commands.get(&ext).cloned() else {
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
            let pids: Vec<Option<u32>> = self.terminals.iter().map(|t| t.child_pid()).collect();
            if let Some(idx) = dnd::shell_running_octave(&pids) {
                let command = format!("run({})", dnd::octave_quote(&path.to_string_lossy()));
                self.terminals[idx].write_input(command.as_bytes());
                self.terminals[idx].write_input(b"\r");
                self.status_message = i18n::msg_run_started(lang, idx, &command);
                return;
            }
        }
        let quoted = shell_words::quote(&path.to_string_lossy()).into_owned();
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
        let command = template.replace("{file}", &quoted);
        let idx = self
            .terminals
            .iter()
            .position(|t| t.child_pid().map(|pid| !dnd::shell_is_busy(pid)).unwrap_or(false))
            .unwrap_or(self.active_terminal.min(self.terminals.len().saturating_sub(1)));

        self.terminals[idx].write_input(command.as_bytes());
        self.terminals[idx].write_input(b"\r");
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

    /// Opens the venv drop-down under the toolbar button. Replaces cycling blindly to the next
    /// venv, which with more than two meant clicking until the right one appeared.
    pub fn open_venv_dropdown(&mut self) {
        // Start on the entry that is currently active, so Enter alone changes nothing.
        let selected = match &self.settings.active_venv {
            None => 0,
            Some(active) => {
                self.available_venvs.iter().position(|v| v == active).map(|i| i + 1).unwrap_or(0)
            }
        };
        self.venv_dropdown = Some(selected);
    }

    /// The drop-down's rows: "no venv", every available venv, then the register entry. Built in
    /// one place so what is drawn and what a click resolves to can't disagree.
    pub fn venv_dropdown_rows(&self) -> Vec<VenvRow> {
        venv_rows(
            self.settings.active_venv.as_deref(),
            &self.available_venvs,
            &self.settings.registered_venvs,
            self.settings.lang,
        )
    }

    fn handle_venv_dropdown_key(&mut self, key: KeyEvent) {
        let Some(selected) = self.venv_dropdown else { return };
        let len = self.venv_dropdown_rows().len();
        match key.code {
            KeyCode::Esc => self.venv_dropdown = None,
            KeyCode::Up => self.venv_dropdown = Some((selected + len - 1) % len),
            KeyCode::Down => self.venv_dropdown = Some((selected + 1) % len),
            KeyCode::Enter => self.activate_venv_row(selected),
            _ => {}
        }
    }

    fn activate_venv_row(&mut self, index: usize) {
        self.venv_dropdown = None;
        let mut rows = self.venv_dropdown_rows();
        if index >= rows.len() {
            return;
        }
        match rows.swap_remove(index).action {
            VenvRowAction::Select(venv) => self.select_venv(venv),
            VenvRowAction::Register => self.begin_venv_register(),
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
        if self.venv_dropdown.is_some() {
            self.handle_venv_dropdown_key(key);
            return;
        }
        if self.venv_register.is_some() {
            self.handle_venv_register_key(key);
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

        if alt {
            if let KeyCode::Char(c) = key.code {
                if let Some(idx) = self.menu_index_for_mnemonic(c) {
                    self.menu.open_at(idx);
                    return;
                }
            }
        }

        match key.code {
            KeyCode::F(9) => {
                self.menu.open();
                return;
            }
            KeyCode::F(1) => {
                self.focus = Focus::FileTree;
                return;
            }
            KeyCode::F(2) => {
                self.focus = Focus::Editor;
                return;
            }
            KeyCode::F(3) => {
                if self.settings.show_terminal {
                    self.focus = Focus::Terminal;
                }
                return;
            }
            KeyCode::F(4) => {
                self.show_settings = true;
                return;
            }
            KeyCode::F(5) => {
                self.new_terminal();
                return;
            }
            KeyCode::F(6) => {
                self.close_active_terminal();
                return;
            }
            KeyCode::F(7) => {
                self.editor_mut().toggle_fold();
                return;
            }
            KeyCode::F(8) => {
                self.resize_mode = !self.resize_mode;
                return;
            }
            KeyCode::F(10) => {
                self.run_active_file();
                return;
            }
            KeyCode::PageDown if ctrl => {
                self.cycle_terminal(true);
                return;
            }
            KeyCode::PageUp if ctrl => {
                self.cycle_terminal(false);
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
            KeyCode::Char('t') if ctrl => {
                self.settings.show_terminal = !self.settings.show_terminal;
                if !self.settings.show_terminal && self.focus == Focus::Terminal {
                    self.cycle_focus(true);
                }
                return;
            }
            KeyCode::Tab if ctrl => {
                self.cycle_focus(true);
                return;
            }
            // Ctrl+\ (0x1C, ASCII FS) isn't reliably delivered by every terminal, unlike
            // Alt+letter which already works via the ESC-prefix mechanism (menu mnemonics).
            KeyCode::Char('p') | KeyCode::Char('P') if alt => {
                self.toggle_split_view();
                return;
            }
            // Show/hide the menu bar. Alt+B works where the terminal sends Option as Meta;
            // 'B' is no menu's mnemonic in either language, so it never clashes with the
            // Alt+<letter> menu-open shortcuts. Ctrl+B is a macOS-friendly alias, since
            // Terminal.app/iTerm don't send Option as Meta by default — but it's only
            // claimed outside the terminal pane, so a focused shell/tmux still gets Ctrl+B.
            KeyCode::Char('b') | KeyCode::Char('B') if alt => {
                self.settings.show_menubar = !self.settings.show_menubar;
                return;
            }
            KeyCode::Char('b') if ctrl && self.focus != Focus::Terminal => {
                self.settings.show_menubar = !self.settings.show_menubar;
                return;
            }
            // Ctrl+L: macOS-friendly alias for Alt+P (toggle split editor), since Option
            // isn't Meta by default there. Claimed only outside the terminal pane.
            KeyCode::Char('l') if ctrl && self.focus != Focus::Terminal => {
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
            KeyCode::Esc | KeyCode::F(9) => self.menu.close(),
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
            MenuAction::ToggleMenuBar => self.settings.show_menubar = !self.settings.show_menubar,
            MenuAction::OpenSettings => self.show_settings = true,
            MenuAction::NewTerminal => self.new_terminal(),
            MenuAction::CloseTerminal => self.close_active_terminal(),
            MenuAction::Save => self.save_active_file(),
            // Deliberately available for a named buffer too, to save a copy under a new name.
            MenuAction::SaveAs => self.begin_save_as(self.active_editor, None),
            MenuAction::SaveAll => self.save_all(),
            MenuAction::SelectVenv => self.open_venv_dropdown(),
            MenuAction::Quit => self.request_quit(),
            MenuAction::ShowAbout => self.show_about = true,
            MenuAction::Copy => self.copy_selection(),
            MenuAction::Cut => self.cut_selection(),
            MenuAction::Paste => self.paste_clipboard(),
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
            MenuAction::CommandPalette => self.open_command_palette(),
            MenuAction::OpenFilePicker => self.open_file_picker(),
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
        match key.code {
            KeyCode::Esc | KeyCode::F(8) | KeyCode::Enter => self.resize_mode = false,
            KeyCode::Left => {
                self.settings.sidebar_width = self.settings.sidebar_width.saturating_sub(2);
                self.settings.clamp_layout();
            }
            KeyCode::Right => {
                self.settings.sidebar_width = self.settings.sidebar_width.saturating_add(2);
                self.settings.clamp_layout();
            }
            KeyCode::Up => {
                self.settings.terminal_pct = self.settings.terminal_pct.saturating_sub(5);
                self.settings.clamp_layout();
            }
            KeyCode::Down => {
                self.settings.terminal_pct = self.settings.terminal_pct.saturating_add(5);
                self.settings.clamp_layout();
            }
            _ => {}
        }
    }

    fn apply_layout_preset(&mut self, preset: LayoutPreset) {
        self.settings.show_sidebar = preset.show_sidebar;
        self.settings.show_terminal = preset.show_terminal;
        self.settings.sidebar_width = preset.sidebar_width;
        self.settings.terminal_pct = preset.terminal_pct;
        self.settings.terminal_on_right = preset.terminal_on_right;
        self.settings.clamp_layout();
    }

    fn try_start_drag(&mut self, col: u16, row: u16, areas: &ui::Areas) -> bool {
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
            // Both are handled where the drag happens, against the pane they started in.
            Some(DragTarget::TextSelection) | Some(DragTarget::TerminalSelection(_)) | None => {}
        }
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
            KeyCode::Esc | KeyCode::F(4) => self.show_settings = false,
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
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            // Ctrl+Shift+S is indistinguishable from plain Ctrl+S in standard terminal
            // input (no Kitty keyboard protocol), so Save All uses Alt+S instead — Alt
            // combos already work reliably via the ESC-prefix menu mnemonics.
            KeyCode::Char('s') | KeyCode::Char('S') if alt => self.save_all(),
            // Split-pane focus (only meaningful when split); left unchanged on Alt+←/→.
            KeyCode::Left if alt && self.split_view => self.editor_pane_focus = EditorPane::Left,
            KeyCode::Right if alt && self.split_view => self.editor_pane_focus = EditorPane::Right,
            // Move the current line up/down; Alt+Shift+↓ duplicates it.
            KeyCode::Down if alt && shift => self.editor_mut().duplicate_line(),
            KeyCode::Up if alt => self.editor_mut().move_line_up(),
            KeyCode::Down if alt => self.editor_mut().move_line_down(),
            // Editor-tab cycling moved off Ctrl+←/→ (now word motion) to Alt+, / Alt+.
            KeyCode::Char(',') if alt => self.cycle_editor(false),
            KeyCode::Char('.') if alt => self.cycle_editor(true),
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
            KeyCode::Char('/') if ctrl => self.toggle_comment(),
            KeyCode::Char('f') if ctrl => self.open_find(false),
            KeyCode::Char('g') if ctrl => self.open_goto(),
            // Word-wise motion (Ctrl+←/→, Shift extends) and deletion (Ctrl+Backspace/Delete).
            KeyCode::Left if ctrl => self.move_with_selection(shift, |e| e.move_word_left()),
            KeyCode::Right if ctrl => self.move_with_selection(shift, |e| e.move_word_right()),
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
        }
        let index = self.active_terminal;
        if key.code == KeyCode::Esc && self.terminals.get(index).is_some_and(|t| t.selection.is_some()) {
            if let Some(term) = self.terminals.get_mut(index) {
                term.clear_selection();
            }
            return;
        }
        let bytes = key_to_bytes(key);
        if !bytes.is_empty() {
            if let Some(term) = self.terminals.get_mut(self.active_terminal) {
                term.write_input(&bytes);
            }
        }
    }

    /// Extends the active pane's selection by one cell, anchoring it at the terminal's own
    /// cursor the first time, then copies it — same rule as finishing a mouse drag.
    fn move_terminal_selection(&mut self, d_row: i16, d_col: i16) {
        let index = self.active_terminal;
        let Some(term) = self.terminals.get_mut(index) else { return };
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
        let single = self.terminals.get(index).and_then(|t| t.selection).is_some_and(|s| s.is_single_cell());
        if single {
            if let Some(term) = self.terminals.get_mut(index) {
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
        let Some(text) = self.terminals.get(index).and_then(|t| t.selection_text()) else { return };
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
                if self.show_save_as {
                    self.cancel_save_as();
                    return;
                }
                if self.venv_register.is_some() {
                    self.cancel_venv_register();
                    return;
                }
                if self.venv_dropdown.is_some() {
                    // Inside the list picks a row; anywhere else dismisses it, like the menus.
                    let rect = ui::venv_dropdown_rect(self, areas.editor, full);
                    match rect.map(ui::inner_rect).filter(|inner| within(*inner, col, row)) {
                        Some(inner) => self.activate_venv_row((row - inner.y) as usize),
                        None => self.venv_dropdown = None,
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
                let panes = ui::editor_pane_rects(areas.editor, self.split_view);
                if let Some((pane_idx, pane_rect)) = panes.iter().enumerate().find(|(_, r)| within(**r, col, row)) {
                    let pane_rect = *pane_rect;
                    self.focus = Focus::Editor;
                    self.editor_pane_focus = if pane_idx == 0 { EditorPane::Left } else { EditorPane::Right };
                    let (tab_bar, content) = ui::split_editor_area(pane_rect);
                    if within(tab_bar, col, row) {
                        self.mouse_tab_click(col, tab_bar, self.editor_pane_focus);
                    } else {
                        self.editor_mut().clear_selection();
                        self.position_cursor_from_click(content, col, row);
                        let anchor = (self.editor().cursor_line, self.editor().cursor_col);
                        self.editor_mut().selection_anchor = Some(anchor);
                        self.dragging = Some(DragTarget::TextSelection);
                    }
                    return;
                }
                if let Some(term_areas) = &areas.terminals {
                    for (i, rect) in term_areas.iter().enumerate() {
                        if within(*rect, col, row) {
                            self.focus = Focus::Terminal;
                            self.active_terminal = i;
                            // Start a selection: cleecode captures the mouse, so the host
                            // terminal's own selection can't be used while it runs.
                            let inner = ui::inner_rect(*rect);
                            if let Some(cell) = cell_at(inner, col, row) {
                                if let Some(term) = self.terminals.get_mut(i) {
                                    term.begin_selection(cell);
                                }
                                self.dragging = Some(DragTarget::TerminalSelection(i));
                            }
                            return;
                        }
                    }
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => match self.dragging {
                Some(DragTarget::Sidebar) | Some(DragTarget::TerminalHeight) => {
                    self.continue_drag(col, row, full);
                }
                Some(DragTarget::TextSelection) => {
                    if within(areas.editor, col, row) {
                        // Stay within the pane the drag started in, regardless of which
                        // pane the pointer is currently over.
                        let panes = ui::editor_pane_rects(areas.editor, self.split_view);
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
                        if let Some(cell) = cell_at(ui::inner_rect(rect), col, row) {
                            if let Some(term) = self.terminals.get_mut(index) {
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
        let panes = ui::editor_pane_rects(areas.editor, self.split_view);
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
                self.scroll_tabs(pane, step, tab_bar.width, pane == EditorPane::Left);
                return;
            }
            if within(content, col, row) {
                // Scroll whichever pane the pointer is over, independent of focus.
                let idx = if pane_idx == 0 { self.active_editor } else { self.active_editor_right };
                if delta < 0 {
                    self.editors[idx].top_line = self.editors[idx].top_line.saturating_sub((-delta) as usize);
                } else {
                    let max_top = self.editors[idx].rope.len_lines().saturating_sub(1);
                    self.editors[idx].top_line = (self.editors[idx].top_line + delta as usize).min(max_top);
                }
            }
            return;
        }
    }

    fn mouse_tab_click(&mut self, col: u16, tab_bar: Rect, pane: EditorPane) {
        let rel_col = col.saturating_sub(tab_bar.x);
        // Both panes show a Run button; the venv selector only renders in the left/only pane.
        let with_venv = pane == EditorPane::Left;
        let strip = self.tab_strip(tab_bar.width, with_venv, pane);
        if let Some((start, end)) = strip.left_arrow {
            if rel_col >= start && rel_col < end {
                self.scroll_tabs(pane, -1, tab_bar.width, with_venv);
                return;
            }
        }
        if let Some((start, end)) = strip.right_arrow {
            if rel_col >= start && rel_col < end {
                self.scroll_tabs(pane, 1, tab_bar.width, with_venv);
                return;
            }
        }
        if let Some((i, layout)) = strip.tab_at(rel_col) {
            if rel_col >= layout.close.0 && rel_col < layout.close.1 {
                self.close_editor_at(i);
            } else {
                match pane {
                    EditorPane::Left => self.active_editor = i,
                    EditorPane::Right => self.active_editor_right = i,
                }
                self.focus = Focus::Editor;
                self.editor_pane_focus = pane;
            }
            return;
        }
        let (venv_range, run_range) = ui::toolbar_button_ranges(self, tab_bar.width, with_venv);
        if let Some((start, end)) = venv_range {
            if rel_col >= start && rel_col < end {
                self.open_venv_dropdown();
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
    fn tab_strip(&self, tab_bar_width: u16, with_venv: bool, pane: EditorPane) -> ui::TabStrip {
        ui::tab_strip_layout(
            &ui::tab_widths(self),
            ui::tab_strip_width(self, tab_bar_width, with_venv),
            self.tab_offsets[pane.index()],
        )
    }

    /// Brings a pane's active tab into view, but only when the active tab has changed since the
    /// last time this ran. Doing it every frame is what broke scrolling left: the strip snapped
    /// back to the active tab immediately, so the `‹` arrow looked dead.
    pub fn reveal_active_tab(&mut self, pane: EditorPane, tab_bar_width: u16, with_venv: bool) {
        let active = match pane {
            EditorPane::Left => self.active_editor,
            EditorPane::Right => self.active_editor_right,
        };
        let slot = pane.index();
        if self.tab_revealed[slot] == Some(active) {
            return;
        }
        self.tab_offsets[slot] = ui::offset_revealing(
            &ui::tab_widths(self),
            ui::tab_strip_width(self, tab_bar_width, with_venv),
            self.tab_offsets[slot],
            active,
        );
        self.tab_revealed[slot] = Some(active);
    }

    /// Scrolls a pane's tab strip by `delta` tabs, starting from what is on screen rather
    /// than from the stored offset, so the first step after an auto-scroll doesn't jump.
    fn scroll_tabs(&mut self, pane: EditorPane, delta: isize, tab_bar_width: u16, with_venv: bool) {
        let first = self.tab_strip(tab_bar_width, with_venv, pane).first as isize;
        let last = self.editors.len().saturating_sub(1) as isize;
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

    fn mouse_menu_bar_click(&mut self, col: u16) {
        let ranges = ui::menu_title_ranges(&self.menu, self.settings.lang);
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
                let idx = (row - inner.y) as usize;
                if idx < self.menu.defs[self.menu.menu_index].items.len() {
                    self.menu.item_index = idx;
                    if let Some(action) = self.menu.selected_action() {
                        self.menu.close();
                        self.run_menu_action(action);
                    }
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
    fn venv_dropdown_rows_map_positions_to_actions() {
        let registered = vec![crate::settings::RegisteredVenv::Named {
            name: "ml".to_string(),
            path: "/opt/venvs/ml-3.12".to_string(),
        }];
        let available = vec![".venv".to_string(), "/opt/venvs/ml-3.12".to_string()];
        let rows = venv_rows(Some("/opt/venvs/ml-3.12"), &available, &registered, Lang::En);

        // "no venv", one row per venv, then the register entry.
        assert_eq!(rows.len(), 4);
        assert!(matches!(rows[0].action, VenvRowAction::Select(None)));
        assert!(matches!(rows[1].action, VenvRowAction::Select(Some(ref v)) if v == ".venv"));
        assert!(matches!(rows[3].action, VenvRowAction::Register));

        // The nickname is the label; the path stays as the dimmed detail.
        assert_eq!(rows[2].label, "ml");
        assert_eq!(rows[2].detail.as_deref(), Some("/opt/venvs/ml-3.12"));
        // A project-root venv's label already *is* its path, so it carries no detail to repeat.
        assert_eq!(rows[1].label, ".venv");
        assert_eq!(rows[1].detail, None);
        // Exactly the venv in use is marked.
        assert!(rows[2].active);
        assert!(!rows[0].active && !rows[1].active && !rows[3].active);

        // With no venv selected, the marker moves to the first row.
        let rows = venv_rows(None, &available, &registered, Lang::En);
        assert!(rows[0].active);
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
