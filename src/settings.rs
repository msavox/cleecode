use crate::i18n::{self, Key, Lang};
use serde::{Deserialize, Serialize};

pub const SETTINGS_COUNT: usize = 9;

pub const SIDEBAR_WIDTH_RANGE: (u16, u16) = (15, 60);
pub const TERMINAL_PCT_RANGE: (u16, u16) = (15, 70);

#[derive(Serialize, Deserialize)]
pub struct Settings {
    pub show_line_numbers: bool,
    pub syntax_highlighting: bool,
    pub word_wrap: bool,
    pub tab_size: usize,
    pub insert_spaces: bool,
    pub show_whitespace: bool,
    pub auto_indent: bool,
    pub mouse_enabled: bool,
    pub lang: Lang,
    // Layout: persisted alongside the rest so a preferred workspace shape survives restarts.
    pub show_sidebar: bool,
    pub show_terminal: bool,
    pub sidebar_width: u16,
    pub terminal_pct: u16,
    pub terminal_on_right: bool,
    // Extension (no dot) -> shell command template; "{file}" is replaced with the
    // active file's shell-quoted absolute path. Hand-editable in settings.toml.
    #[serde(default = "default_run_commands")]
    pub run_commands: std::collections::HashMap<String, String>,
    // Folder name (relative to the project root) of the venv to use when running
    // Python scripts, e.g. ".venv". None means "system python".
    #[serde(default)]
    pub active_venv: Option<String>,
    #[serde(default = "default_true")]
    pub show_hidden_files: bool,
    // Auto-close brackets/quotes and expand pairs on Enter. Hand-editable; on by default.
    #[serde(default = "default_true")]
    pub auto_pairs: bool,
    // Workspace resume: which project folder and which of its files were open, so
    // launching cleecode with no arguments picks up where the last session left off.
    #[serde(default)]
    pub last_root: Option<std::path::PathBuf>,
    #[serde(default)]
    pub last_open_files: Vec<std::path::PathBuf>,
    #[serde(default)]
    pub last_active_file: Option<std::path::PathBuf>,
}

fn default_true() -> bool {
    true
}

fn default_run_commands() -> std::collections::HashMap<String, String> {
    [
        ("py", "python3 {file}"),
        ("sh", "bash {file}"),
        ("bash", "bash {file}"),
        ("rb", "ruby {file}"),
        ("js", "node {file}"),
        ("ts", "ts-node {file}"),
        ("m", "octave {file}"),
        ("pl", "perl {file}"),
        ("go", "go run {file}"),
        ("php", "php {file}"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            show_line_numbers: true,
            syntax_highlighting: true,
            word_wrap: false,
            tab_size: 4,
            insert_spaces: true,
            show_whitespace: false,
            auto_indent: true,
            mouse_enabled: true,
            lang: Lang::default(),
            show_sidebar: true,
            show_terminal: true,
            sidebar_width: 30,
            terminal_pct: 35,
            terminal_on_right: false,
            run_commands: default_run_commands(),
            active_venv: None,
            show_hidden_files: true,
            auto_pairs: true,
            last_root: None,
            last_open_files: Vec::new(),
            last_active_file: None,
        }
    }
}

fn config_dir() -> Option<std::path::PathBuf> {
    // Keep the long-standing ~/.config/cleecode location on Unix (macOS included, where it
    // predates this cross-platform work) rather than moving to ~/Library/Application
    // Support, so existing settings are still found. On Windows fall back to %APPDATA%.
    #[cfg(unix)]
    {
        match std::env::var_os("XDG_CONFIG_HOME") {
            Some(xdg) => Some(std::path::PathBuf::from(xdg).join("cleecode")),
            None => dirs::home_dir().map(|h| h.join(".config").join("cleecode")),
        }
    }
    #[cfg(not(unix))]
    {
        dirs::config_dir().map(|d| d.join("cleecode"))
    }
}

fn config_path() -> Option<std::path::PathBuf> {
    config_dir().map(|d| d.join("settings.toml"))
}

impl Settings {
    /// Loads persisted settings from disk, falling back to defaults if there's nothing
    /// saved yet or the file can't be read/parsed.
    pub fn load() -> Self {
        config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Best-effort save; silently does nothing if the config dir can't be created/written.
    pub fn save(&self) {
        let Some(path) = config_path() else { return };
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        if let Ok(text) = toml::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }

    pub fn clamp_layout(&mut self) {
        self.sidebar_width = self.sidebar_width.clamp(SIDEBAR_WIDTH_RANGE.0, SIDEBAR_WIDTH_RANGE.1);
        self.terminal_pct = self.terminal_pct.clamp(TERMINAL_PCT_RANGE.0, TERMINAL_PCT_RANGE.1);
    }
}

pub struct SettingRow {
    pub label: &'static str,
    pub value: String,
}

impl Settings {
    pub fn rows(&self) -> Vec<SettingRow> {
        let lang = self.lang;
        let b = |v: bool| i18n::t(lang, if v { Key::On } else { Key::Off }).to_string();
        vec![
            SettingRow { label: i18n::t(lang, Key::SettingLineNumbers), value: b(self.show_line_numbers) },
            SettingRow { label: i18n::t(lang, Key::SettingSyntaxHighlighting), value: b(self.syntax_highlighting) },
            SettingRow { label: i18n::t(lang, Key::SettingWordWrap), value: b(self.word_wrap) },
            SettingRow { label: i18n::t(lang, Key::SettingTabSize), value: self.tab_size.to_string() },
            SettingRow { label: i18n::t(lang, Key::SettingInsertSpaces), value: b(self.insert_spaces) },
            SettingRow { label: i18n::t(lang, Key::SettingShowWhitespace), value: b(self.show_whitespace) },
            SettingRow { label: i18n::t(lang, Key::SettingAutoIndent), value: b(self.auto_indent) },
            SettingRow { label: i18n::t(lang, Key::SettingMouseEnabled), value: b(self.mouse_enabled) },
            SettingRow { label: i18n::t(lang, Key::SettingLanguage), value: self.lang.label().to_string() },
        ]
    }

    /// Enter/Space on a row: toggles booleans, bumps numbers, cycles enums.
    pub fn activate(&mut self, idx: usize) {
        match idx {
            0 => self.show_line_numbers = !self.show_line_numbers,
            1 => self.syntax_highlighting = !self.syntax_highlighting,
            2 => self.word_wrap = !self.word_wrap,
            3 => self.adjust_tab_size(1),
            4 => self.insert_spaces = !self.insert_spaces,
            5 => self.show_whitespace = !self.show_whitespace,
            6 => self.auto_indent = !self.auto_indent,
            7 => self.mouse_enabled = !self.mouse_enabled,
            8 => self.lang = self.lang.next(),
            _ => {}
        }
    }

    /// Left/Right on a row: only meaningful for numeric/enum rows, otherwise same as activate.
    pub fn adjust(&mut self, idx: usize, delta: i32) {
        if idx == 3 {
            self.adjust_tab_size(delta);
        } else {
            self.activate(idx);
        }
    }

    fn adjust_tab_size(&mut self, delta: i32) {
        let new_val = (self.tab_size as i32 + delta).clamp(1, 8);
        self.tab_size = new_val as usize;
    }
}
