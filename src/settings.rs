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
    // Menu bar visibility. On by default so newcomers keep the discoverable drop-down bar;
    // power users can hide it (Alt+B / View menu) and still reach menus via F9 / Alt+<letter>.
    #[serde(default = "default_true")]
    pub show_menubar: bool,
    // Extension (no dot) -> shell command template; "{file}" is replaced with the
    // active file's shell-quoted absolute path. Hand-editable in settings.toml.
    #[serde(default = "default_run_commands")]
    pub run_commands: std::collections::HashMap<String, String>,
    // The selected venv when running Python. Either a folder name relative to the project
    // root (an auto-discovered venv, e.g. ".venv") or an absolute path (a registered one).
    // None means "system python".
    #[serde(default)]
    pub active_venv: Option<String>,
    // Venvs the user registered by absolute path, available in every project on top of the
    // ones auto-discovered in the project root. Hand-editable, or via the venv manager.
    #[serde(default)]
    pub registered_venvs: Vec<RegisteredVenv>,
    // Program name as written in a run_commands template (e.g. "octave-cli") -> absolute path
    // to that executable. For interpreters installed outside PATH; the common case is Octave
    // on Windows, which lives in a versioned Program Files directory. Hand-editable.
    #[serde(default)]
    pub interpreter_paths: std::collections::HashMap<String, String>,
    // Off by default: a project root is usually full of dot-directories (.git, .venv, caches)
    // that bury the actual source files. `H` (or the View menu) brings them back.
    #[serde(default)]
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

/// A venv registered by absolute path, offered in every project. Accepts either form in
/// settings.toml, so a plain list of paths keeps working:
///
/// ```toml
/// registered_venvs = ["/opt/venvs/ml-3.12"]        # path only
///
/// [[registered_venvs]]                             # with a short nickname
/// name = "ml"
/// path = "/opt/venvs/ml-3.12"
/// ```
///
/// The nickname is what the selector shows; without one it falls back to the folder name.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum RegisteredVenv {
    Path(String),
    Named { name: String, path: String },
}

impl RegisteredVenv {
    pub fn path(&self) -> &str {
        match self {
            RegisteredVenv::Path(path) => path,
            RegisteredVenv::Named { path, .. } => path,
        }
    }

    pub fn nickname(&self) -> Option<&str> {
        match self {
            RegisteredVenv::Path(_) => None,
            RegisteredVenv::Named { name, .. } => Some(name),
        }
    }
}

fn default_true() -> bool {
    true
}

/// Octave's plot windows only live as long as the interpreter does, so a plain
/// `octave script.m` draws the figures and closes them the instant the script ends.
/// `--persist` stays in the interactive prompt afterwards, leaving the plots on screen and
/// the variables inspectable; `exit` at the prompt closes them.
///
/// On Windows plain `octave` is the GUI launcher, which would detach from the embedded
/// terminal; `octave-cli` is the console interpreter that runs the script in place.
fn default_octave_command() -> &'static str {
    if cfg!(windows) { "octave-cli --persist {file}" } else { "octave --persist {file}" }
}

fn default_run_commands() -> std::collections::HashMap<String, String> {
    [
        ("py", "python3 {file}"),
        ("sh", "bash {file}"),
        ("bash", "bash {file}"),
        ("rb", "ruby {file}"),
        ("js", "node {file}"),
        ("ts", "ts-node {file}"),
        ("m", default_octave_command()),
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
            show_menubar: true,
            run_commands: default_run_commands(),
            active_venv: None,
            registered_venvs: Vec::new(),
            interpreter_paths: std::collections::HashMap::new(),
            show_hidden_files: false,
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
        let mut settings: Settings = config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default();
        settings.merge_run_command_defaults();
        settings
    }

    /// Fills in run commands for extensions a saved file doesn't mention, so defaults added in
    /// a later version reach existing users instead of only new installs — `run_commands` is
    /// persisted whole, so without this it would be frozen at whatever the first run wrote.
    /// Entries the user actually edited are left alone.
    fn merge_run_command_defaults(&mut self) {
        for (ext, command) in default_run_commands() {
            self.run_commands.entry(ext).or_insert(command);
        }
        // The pre-`--persist` Octave commands closed every plot the moment a script ended.
        // Nobody would pick that deliberately, so upgrade those exact entries in place.
        let stale = matches!(self.run_commands.get("m").map(String::as_str), Some("octave {file}" | "octave-cli {file}"));
        if stale {
            self.run_commands.insert("m".to_string(), default_octave_command().to_string());
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `save()` swallows serialization errors, so a field ordering that TOML rejects (a
    /// scalar emitted after a table) would silently stop settings from persisting at all.
    #[test]
    fn settings_survive_a_toml_round_trip() {
        let mut settings = Settings::default();
        settings.registered_venvs = vec![
            RegisteredVenv::Path("/opt/venvs/central".to_string()),
            RegisteredVenv::Named { name: "ml".to_string(), path: "/opt/venvs/ml-3.12".to_string() },
        ];
        settings.interpreter_paths =
            [("octave-cli".to_string(), "/opt/homebrew/bin/octave-cli".to_string())].into_iter().collect();
        settings.active_venv = Some(".venv".to_string());
        settings.tab_size = 2;

        let text = toml::to_string_pretty(&settings).expect("settings must serialize");
        let back: Settings = toml::from_str(&text).expect("settings must parse back");

        assert_eq!(back.registered_venvs, settings.registered_venvs);
        assert_eq!(back.interpreter_paths, settings.interpreter_paths);
        assert_eq!(back.active_venv, settings.active_venv);
        assert_eq!(back.tab_size, 2);
        assert_eq!(back.run_commands.get("m"), settings.run_commands.get("m"));
    }

    /// Both hand-written forms must parse. A parse failure is silent (`load()` falls back to
    /// defaults), which would look like every preference being reset, so this is worth pinning.
    #[test]
    fn registered_venvs_accept_bare_paths_and_nicknamed_tables() {
        // Only the one key matters here; the surrounding Settings fields are covered elsewhere.
        #[derive(Deserialize)]
        struct OnlyVenvs {
            #[serde(default)]
            registered_venvs: Vec<RegisteredVenv>,
        }

        // The two forms are alternatives for the same key, so each is checked on its own.
        let bare: OnlyVenvs =
            toml::from_str("registered_venvs = [\"/opt/venvs/plain\"]").expect("bare list must parse");
        assert_eq!(bare.registered_venvs, vec![RegisteredVenv::Path("/opt/venvs/plain".to_string())]);
        assert_eq!(bare.registered_venvs[0].nickname(), None);
        assert_eq!(bare.registered_venvs[0].path(), "/opt/venvs/plain");

        let named: OnlyVenvs =
            toml::from_str("[[registered_venvs]]\nname = \"ml\"\npath = \"/opt/venvs/ml-3.12\"\n")
                .expect("nicknamed table must parse");
        assert_eq!(named.registered_venvs[0].nickname(), Some("ml"));
        assert_eq!(named.registered_venvs[0].path(), "/opt/venvs/ml-3.12");

        // Mixed in one list: the untagged enum must pick the right variant per entry.
        let mixed: OnlyVenvs = toml::from_str(
            "registered_venvs = [\"/opt/venvs/plain\", { name = \"ml\", path = \"/opt/venvs/ml\" }]",
        )
        .expect("mixed list must parse");
        assert_eq!(mixed.registered_venvs[0].nickname(), None);
        assert_eq!(mixed.registered_venvs[1].nickname(), Some("ml"));
    }

    /// The new fields are hand-editable, so an older settings.toml lacking them must still
    /// load instead of resetting every preference to default.
    #[test]
    fn older_settings_file_without_new_fields_still_loads() {
        let text = "show_line_numbers = false\nsyntax_highlighting = true\nword_wrap = false\n\
                    tab_size = 8\ninsert_spaces = true\nshow_whitespace = false\nauto_indent = true\n\
                    mouse_enabled = true\nlang = \"It\"\nshow_sidebar = true\nshow_terminal = true\n\
                    sidebar_width = 30\nterminal_pct = 35\nterminal_on_right = false\n";
        let s: Settings = toml::from_str(text).expect("legacy file must parse");
        assert_eq!(s.tab_size, 8);
        assert!(!s.show_line_numbers);
        assert!(s.registered_venvs.is_empty());
        assert!(s.interpreter_paths.is_empty());
        // Defaults fill in for what the old file never knew about.
        assert!(s.auto_pairs);
        assert!(s.run_commands.contains_key("m"));
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
