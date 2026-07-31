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

pub struct App {
    pub root: PathBuf,
    pub file_tree: FileTree,
    pub editors: Vec<Editor>,
    pub active_editor: usize,
    pub split_view: bool,
    pub active_editor_right: usize,
    pub editor_pane_focus: EditorPane,
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

/// Top-level subdirectories of `root` that look like Python virtualenvs (they carry a
/// `bin/activate` script). Non-recursive: only scans the project root itself.
fn discover_venvs(root: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut venvs: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("bin").join("activate").exists())
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .collect();
    venvs.sort();
    venvs
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DragTarget {
    Sidebar,
    TerminalHeight,
    TextSelection,
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

impl App {
    pub fn new(root: PathBuf, term_rows: u16, term_cols: u16) -> Result<Self> {
        let half_cols = (term_cols / 2).max(10);
        let t1 = TerminalPanel::new(term_rows, half_cols, &root)?;
        let t2 = TerminalPanel::new(term_rows, half_cols, &root)?;
        let (bg_tx, bg_rx) = mpsc::channel();
        let available_venvs = discover_venvs(&root);
        let (git_status_tx, git_status_rx) = mpsc::channel();
        let git_status_pending = Arc::new(AtomicBool::new(false));
        spawn_git_status_refresh(root.clone(), git_status_tx.clone(), git_status_pending.clone());
        let settings = Settings::load();
        let mut file_tree = FileTree::new(root.clone());
        file_tree.show_hidden = settings.show_hidden_files;
        file_tree.rebuild_visible();
        Ok(App {
            file_tree,
            root,
            editors: vec![Editor::empty()],
            active_editor: 0,
            split_view: false,
            active_editor_right: 0,
            editor_pane_focus: EditorPane::Left,
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
            match std::process::Command::new("cp").arg("-R").arg(path).arg(&dest).status() {
                Ok(s) if s.success() => ok += 1,
                Ok(s) => last_err = Some(format!("exit {}", s.code().unwrap_or(-1))),
                Err(e) => last_err = Some(e.to_string()),
            }
        }
        if ok > 0 {
            self.file_tree = FileTree::new(self.root.clone());
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
                self.status_message = i18n::msg_opened(lang, &self.editor().title(lang));
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
        self.close_editor_at(self.active_editor);
    }

    fn save_active_file(&mut self) {
        let lang = self.settings.lang;
        match self.editor_mut().save() {
            Ok(()) => self.status_message = i18n::msg_saved(lang, &self.editor().title(lang)),
            Err(e) => self.status_message = i18n::msg_save_error(lang, &e.to_string()),
        }
    }

    fn save_all(&mut self) {
        let lang = self.settings.lang;
        let mut saved = 0usize;
        let mut errors = Vec::new();
        for editor in &mut self.editors {
            if !editor.dirty || editor.path.is_none() {
                continue;
            }
            match editor.save() {
                Ok(()) => saved += 1,
                Err(e) => errors.push(format!("{}: {}", editor.title(lang), e)),
            }
        }
        self.status_message = if errors.is_empty() {
            i18n::msg_saved_all(lang, saved)
        } else {
            i18n::msg_save_all_errors(lang, saved, &errors.join("; "))
        };
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
        self.file_tree = FileTree::new(new_root.clone());
        self.file_tree.show_hidden = self.settings.show_hidden_files;
        self.file_tree.rebuild_visible();
        self.root = new_root;
        self.available_venvs = discover_venvs(&self.root);
        spawn_git_status_refresh(self.root.clone(), self.git_status_tx.clone(), self.git_status_pending.clone());
        self.status_message = i18n::msg_project_folder(self.settings.lang, &self.root.display().to_string());
    }

    fn toggle_hidden_files(&mut self) {
        self.settings.show_hidden_files = !self.settings.show_hidden_files;
        self.file_tree.toggle_hidden();
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
        let quoted = shell_words::quote(&path.to_string_lossy()).into_owned();
        let template = self.apply_venv(&template);
        let command = template.replace("{file}", &quoted);

        if self.terminals.is_empty() {
            self.new_terminal();
        }
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
        let venv_bin = self.root.join(venv).join("bin").join(program);
        if !venv_bin.exists() {
            return template.to_string();
        }
        let quoted = shell_words::quote(&venv_bin.to_string_lossy()).into_owned();
        format!("{quoted} {rest}")
    }

    pub fn cycle_venv(&mut self) {
        if self.available_venvs.is_empty() {
            return;
        }
        let next = match &self.settings.active_venv {
            None => Some(self.available_venvs[0].clone()),
            Some(current) => {
                let pos = self.available_venvs.iter().position(|v| v == current);
                match pos {
                    Some(i) if i + 1 < self.available_venvs.len() => Some(self.available_venvs[i + 1].clone()),
                    _ => None,
                }
            }
        };
        self.settings.active_venv = next;
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
        if self.show_delete_confirm {
            self.handle_delete_confirm_key(key);
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
                self.should_quit = true;
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
            MenuAction::OpenSettings => self.show_settings = true,
            MenuAction::NewTerminal => self.new_terminal(),
            MenuAction::CloseTerminal => self.close_active_terminal(),
            MenuAction::Save => self.save_active_file(),
            MenuAction::SaveAll => self.save_all(),
            MenuAction::Quit => self.should_quit = true,
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
            Some(DragTarget::TextSelection) | None => {}
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
                self.file_tree = FileTree::new(self.root.clone());
                self.status_message = i18n::msg_deleted(lang, &name);
            }
            Err(e) => self.status_message = i18n::msg_delete_failed(lang, &name, &e.to_string()),
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
            KeyCode::Left if alt && self.split_view => self.editor_pane_focus = EditorPane::Left,
            KeyCode::Right if alt && self.split_view => self.editor_pane_focus = EditorPane::Right,
            KeyCode::Char('s') if ctrl => self.save_active_file(),
            KeyCode::Char('w') if ctrl => self.close_active_editor(),
            KeyCode::Char('d') if ctrl => self.close_active_editor(),
            KeyCode::Char('c') if ctrl => self.copy_selection(),
            KeyCode::Char('x') if ctrl => self.cut_selection(),
            KeyCode::Char('v') if ctrl => self.paste_clipboard(),
            KeyCode::Char('a') if ctrl => self.select_all(),
            KeyCode::Right if ctrl => self.cycle_editor(true),
            KeyCode::Left if ctrl => self.cycle_editor(false),
            KeyCode::Char(c) if !ctrl => self.editor_mut().insert_char(c),
            KeyCode::Enter => {
                let auto_indent = self.settings.auto_indent;
                self.editor_mut().insert_newline(auto_indent);
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
    }

    fn handle_terminal_key(&mut self, key: KeyEvent) {
        let bytes = key_to_bytes(key);
        if !bytes.is_empty() {
            if let Some(term) = self.terminals.get_mut(self.active_terminal) {
                term.write_input(&bytes);
            }
        }
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
                if self.show_settings {
                    self.mouse_settings(col, row, full);
                    return;
                }
                if self.menu.active {
                    self.mouse_menu(col, row, full);
                    return;
                }
                if row == areas.menu_bar.y {
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
                None => {}
            },
            MouseEventKind::Up(MouseButton::Left) => {
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
            let (_, content) = ui::split_editor_area(*pane_rect);
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
        let layouts = ui::tab_layouts(self);
        for (i, layout) in layouts.iter().enumerate() {
            if rel_col >= layout.close.0 && rel_col < layout.close.1 {
                self.close_editor_at(i);
                return;
            }
            if rel_col >= layout.full.0 && rel_col < layout.full.1 {
                match pane {
                    EditorPane::Left => self.active_editor = i,
                    EditorPane::Right => self.active_editor_right = i,
                }
                self.focus = Focus::Editor;
                self.editor_pane_focus = pane;
                return;
            }
        }
        // The Run/venv toolbar only renders in the left (or only) pane.
        if pane != EditorPane::Left {
            return;
        }
        let (venv_range, run_range) = ui::toolbar_button_ranges(self, tab_bar.width);
        if let Some((start, end)) = venv_range {
            if rel_col >= start && rel_col < end {
                self.cycle_venv();
                return;
            }
        }
        if let Some((start, end)) = run_range {
            if rel_col >= start && rel_col < end {
                self.run_active_file();
            }
        }
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
