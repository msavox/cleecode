use crate::i18n::{self, Key, Lang};
use serde::{Deserialize, Serialize};

pub const SETTINGS_COUNT: usize = 9;

pub const SIDEBAR_WIDTH_RANGE: (u16, u16) = (15, 60);
pub const TERMINAL_PCT_RANGE: (u16, u16) = (15, 70);
/// Left pane's share of the editor region when split. Kept away from the extremes so neither
/// pane can be squeezed to nothing.
pub const SPLIT_PCT_RANGE: (u16, u16) = (20, 80);

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
    // Left pane's percentage of the editor region in split view. Defaulted so configs written
    // before split resizing existed still load, landing on the old fixed 50/50.
    #[serde(default = "default_split_pct")]
    pub split_pct: u16,
    // Menu bar visibility. On by default so newcomers keep the discoverable drop-down bar;
    // power users can hide it (Ctrl+B / View menu) and still reach menus via Ctrl+Shift+B.
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
    // Extension (no dot) -> the command line of a language server, overriding the built-in
    // table and adding to it. An entry set to the empty string turns a built-in one off.
    //
    // This is what keeps the built-in list from being the limit. A release should not be the way
    // to reach a new language server, or the fork of one somebody keeps in ~/bin, or a language
    // nobody here thought of. Hand-editable in settings.toml, like run_commands beside it.
    // Whether Run hands a file to an interpreter that is already at a prompt, instead of
    // starting a fresh one in a shell.
    //
    // On by default, because the alternative is what CleeCode used to do for Python and it was
    // the wrong answer three ways at once: a script that ran in a process that exited took its
    // variables with it, so the workspace panel stayed empty, and the figures were drawn by
    // something that no longer existed. The run-target drop-down turns it off — choosing a venv
    // is choosing to start an interpreter, which is the same choice said from the other end.
    #[serde(default = "default_true")]
    pub run_in_session: bool,
    #[serde(default)]
    pub language_servers: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub show_hidden_files: bool,
    // Lines of scrolled-off output each shell keeps, for scrolling back through. Costs
    // lines x columns x 32 bytes per shell, so it is worth setting deliberately rather than
    // to a huge number. vt100 fixes the length when a shell starts, so a change here reaches
    // the shells opened afterwards, not the ones already running.
    #[serde(default = "default_terminal_scrollback")]
    pub terminal_scrollback: usize,
    // Whether documents are shown inverted — a light page turned dark. A preference about how
    // you read rather than about one file, so it belongs here and outlives the tab: turning it
    // on for one PDF and having the next one open bright again is exactly the annoyance a dark
    // mode exists to remove.
    //
    // Kept apart from the markdown one on purpose. A paper and a README are read in different
    // places for different reasons, and one answer for both meant that setting either changed
    // the other. Pictures are in neither: inverting a photograph is a negative, not a mode, so
    // it stays with the tab it was asked for and is never remembered.
    #[serde(default)]
    pub preview_dark: bool,
    #[serde(default)]
    pub preview_dark_markdown: bool,
    // Whether a markdown preview opens as styled text rather than as a rendered document. Only
    // ever a choice where pandoc can make the document at all; without it there is nothing to
    // choose between and the text view is all there is.
    #[serde(default)]
    pub preview_markdown_text: bool,
    // Auto-close brackets/quotes and expand pairs on Enter. Hand-editable; on by default.
    #[serde(default = "default_true")]
    pub auto_pairs: bool,
    // Whether the word-completion popup appears while typing. On by default, and off is a real
    // answer: a list that opens by itself is the kind of help some people would rather not have,
    // and a feature that cannot be turned off is one they have to work around instead.
    #[serde(default = "default_true")]
    pub completion: bool,
    // Whether a language server is started for files that have one. It underlines what the
    // server finds wrong and feeds its names into the completion popup — two things, which is
    // why this is no longer called `diagnostics`; the old spelling is still read, so a
    // settings.toml written before the rename keeps working.
    //
    // On by default, and it costs nothing where no server is installed — that is simply an
    // editor without underlines and a popup with only the words of the file in it, which is what
    // CleeCode was until this existed.
    #[serde(default = "default_true", alias = "diagnostics")]
    pub language_server: bool,
    // Where a plot drawn in a live Octave or Python session goes. On — the default — it is
    // captured and opens as a tab, beside the code that drew it. Off, the interpreter keeps its
    // own windows: qt for Octave, matplotlib's usual backend for Python, exactly as they behave
    // outside CleeCode.
    //
    // Read as a *preference*, not as an instruction: a session with nowhere to put a window
    // captures anyway. See `wsnap::can_open_a_window` — over ssh, or on a Linux box with no
    // DISPLAY, "windows" means no plot at all, and silently drawing nothing is not a setting
    // anybody wants. The old spelling of this key is still read, so a settings.toml written
    // before the rename keeps working.
    #[serde(default = "default_true", alias = "diagnostics_figures")]
    pub plots_in_tabs: bool,
    // Whether the editor paints its own background instead of letting the terminal's show
    // through. Off by default, because a terminal's background is the user's choice and taking
    // it over uninvited is rude — but a translucent one with a bright window behind it turns
    // the text unreadable, and then this is the only way back.
    #[serde(default)]
    pub opaque_background: bool,
    // Workspace resume: which project folder and which of its files were open, so
    // launching cleecode with no arguments picks up where the last session left off.
    #[serde(default)]
    pub last_root: Option<std::path::PathBuf>,
    #[serde(default)]
    pub last_open_files: Vec<std::path::PathBuf>,
    #[serde(default)]
    pub last_active_file: Option<std::path::PathBuf>,
    // The named workspace in use, if any. Reopened on a bare `clee` (taking precedence over
    // the plain last_root/last_open_files resume) and kept up to date on exit, so a saved
    // layout — terminal names and startup commands included — survives the session.
    #[serde(default)]
    pub last_workspace: Option<String>,
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

fn default_split_pct() -> u16 {
    50
}

fn default_terminal_scrollback() -> usize {
    crate::terminal_panel::DEFAULT_SCROLLBACK
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
        // LaTeX writes its .aux/.log/.pdf next to wherever it is told to, so it needs the
        // file's own folder rather than the shell's current directory, which is the project
        // root. `-interaction=nonstopmode` keeps an error from parking the terminal at
        // LaTeX's interactive `?` prompt.
        ("tex", "pdflatex -interaction=nonstopmode -output-directory {dir} {file}"),
        // Pictures, shown in a terminal pane as coloured text.
        //
        // `-f symbols` is not optional. Left to itself chafa asks the *host* terminal what it
        // can do, and on a Ghostty or kitty it answers with the graphics protocol — which a
        // pane never passes on: the pty is parsed into a grid of cells and repainted, and
        // vt100 drops the graphics escapes on the floor. The result is a megabyte of output
        // and a blank pane. Symbols are half-blocks and box glyphs with RGB colour, which are
        // just cells, so they survive the trip intact.
        ("png", "chafa -f symbols {file}"),
        ("jpg", "chafa -f symbols {file}"),
        ("jpeg", "chafa -f symbols {file}"),
        ("gif", "chafa -f symbols {file}"),
        ("webp", "chafa -f symbols {file}"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

/// The command an extension ships with, if any. Emptying the run-command box restores this
/// rather than removing the entry, since `merge_run_command_defaults` would put it back at the
/// next start anyway.
pub fn default_run_command(ext: &str) -> Option<String> {
    default_run_commands().get(ext).cloned()
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
            split_pct: default_split_pct(),
            show_menubar: true,
            run_commands: default_run_commands(),
            active_venv: None,
            registered_venvs: Vec::new(),
            interpreter_paths: std::collections::HashMap::new(),
            run_in_session: true,
            language_servers: std::collections::BTreeMap::new(),
            show_hidden_files: false,
            preview_dark: false,
            preview_dark_markdown: false,
            preview_markdown_text: false,
            terminal_scrollback: default_terminal_scrollback(),
            auto_pairs: true,
            completion: true,
            language_server: true,
            plots_in_tabs: true,
            opaque_background: false,
            last_root: None,
            last_open_files: Vec::new(),
            last_active_file: None,
            last_workspace: None,
        }
    }
}

pub fn config_dir() -> Option<std::path::PathBuf> {
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
        self.split_pct = self.split_pct.clamp(SPLIT_PCT_RANGE.0, SPLIT_PCT_RANGE.1);
    }
}

/// The file a project keeps its own settings in, in its root.
pub const PROJECT_FILE: &str = ".cleecode.toml";

/// What a project says about itself, overriding the global settings for as long as it is open.
///
/// Only run commands so far, because that is where a global answer is actually wrong rather
/// than merely coarse: one `[run_commands]` entry for `.tex` cannot compile this project's
/// master file and the next project's too. Preferences — theme, layout, keys — are about the
/// person, not the project, and stay in settings.toml where changing them once is the point.
///
/// It lives in the project, so it is meant to be committed and shared: hand-written entries and
/// comments survive untouched unless the same extension is edited from inside CleeCode.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ProjectSettings {
    #[serde(default)]
    pub run_commands: std::collections::HashMap<String, String>,
}

impl ProjectSettings {
    pub fn path(root: &std::path::Path) -> std::path::PathBuf {
        root.join(PROJECT_FILE)
    }

    /// Reads a project's file, or an empty set when it has none. A malformed file is also read
    /// as empty: a typo in it should cost the overrides, not the ability to run anything.
    pub fn load(root: &std::path::Path) -> Self {
        std::fs::read_to_string(Self::path(root))
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Whether anything is actually overridden. An empty set is written out as an empty file,
    /// so this is what decides between keeping the file and removing it.
    pub fn is_empty(&self) -> bool {
        self.run_commands.is_empty()
    }

    /// Best-effort save into the project root. Emptied of every override, the file is deleted
    /// rather than left behind as an empty stub in someone's repository.
    pub fn save(&self, root: &std::path::Path) {
        let path = Self::path(root);
        if self.is_empty() {
            let _ = std::fs::remove_file(&path);
            return;
        }
        if let Ok(text) = toml::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The plot destination is a preference on a desktop and a fact on a server. It names both
    /// of its answers rather than reading on/off: "off" meant the interpreter's own windows,
    /// which is a different way of working and not a feature being switched off. Where the
    /// choice is not the user's, the row says which way it went and why — a switch that reads
    /// "off" while the tabs keep arriving is a broken switch.
    #[test]
    fn the_plot_row_says_when_the_choice_is_not_the_users_to_make() {
        use i18n::Lang;
        assert_eq!(plots_value(Lang::En, true, true), i18n::t(Lang::En, Key::SettingPlotsTabs));
        assert_eq!(plots_value(Lang::En, false, true), i18n::t(Lang::En, Key::SettingPlotsWindows));
        // Neither answer is a bare "on" or "off": the row has to say what it chose.
        for asked in [true, false] {
            assert_ne!(plots_value(Lang::En, asked, true), i18n::t(Lang::En, Key::On));
            assert_ne!(plots_value(Lang::En, asked, true), i18n::t(Lang::En, Key::Off));
        }
        // No screen: what the file says stops mattering, and both answers read the same.
        for asked in [true, false] {
            for lang in [Lang::En, Lang::It] {
                assert_eq!(plots_value(lang, asked, false), i18n::t(lang, Key::SettingPlotsNoDisplay));
            }
        }
    }

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
        // How documents are read: one answer per kind, and each has to come back on its own.
        settings.preview_dark = true;
        settings.preview_markdown_text = true;

        let text = toml::to_string_pretty(&settings).expect("settings must serialize");
        let back: Settings = toml::from_str(&text).expect("settings must parse back");

        assert_eq!(back.registered_venvs, settings.registered_venvs);
        assert_eq!(back.interpreter_paths, settings.interpreter_paths);
        assert_eq!(back.active_venv, settings.active_venv);
        assert_eq!(back.tab_size, 2);
        assert_eq!(back.run_commands.get("m"), settings.run_commands.get("m"));
        assert!(back.preview_dark, "a PDF read dark stays dark");
        assert!(back.preview_markdown_text, "markdown left as text opens as text");
        assert!(!back.preview_dark_markdown, "markdown keeps its own answer, untouched by the PDF one");
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

    /// A project's file is meant to be committed and shared, so what it does to somebody's
    /// repository matters: it appears when there is something to say and goes away again when
    /// there is not, rather than leaving an empty stub behind.
    #[test]
    fn a_project_file_round_trips_and_removes_itself_when_emptied() {
        let dir = std::env::temp_dir().join(format!("clee_proj_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = ProjectSettings::path(&dir);

        // A project with no file of its own overrides nothing.
        let loaded = ProjectSettings::load(&dir);
        assert!(loaded.is_empty());
        assert!(!path.exists());

        let mut project = ProjectSettings::default();
        project.run_commands.insert("tex".to_string(), "latexmk -pdf main.tex".to_string());
        project.save(&dir);
        assert!(path.exists());
        assert_eq!(ProjectSettings::load(&dir), project);

        // A file nobody can parse costs the overrides, not the ability to run anything.
        std::fs::write(&path, "this is not toml {{{").unwrap();
        assert!(ProjectSettings::load(&dir).is_empty());

        // Emptied of its last override, the file is removed rather than left as a stub.
        ProjectSettings::default().save(&dir);
        assert!(!path.exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// What the plot row reads, which is the state the *session* will be in and not only what the
/// file says.
///
/// On a machine with no screen — a remote one over ssh, a Linux server with no DISPLAY — the
/// destination is not a choice: the interpreter's own window has nowhere to open, so asking for
/// one means no plot at all. The row says so and stops taking Enter, rather than flipping to
/// "off" while the tabs keep arriving, which reads as a broken switch.
fn plots_value(lang: i18n::Lang, in_tabs: bool, can_open_a_window: bool) -> String {
    if !can_open_a_window {
        return i18n::t(lang, Key::SettingPlotsNoDisplay).to_string();
    }
    i18n::t(lang, if in_tabs { Key::SettingPlotsTabs } else { Key::SettingPlotsWindows }).to_string()
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
            SettingRow { label: i18n::t(lang, Key::SettingCompletion), value: b(self.completion) },
            SettingRow {
                label: i18n::t(lang, Key::SettingLanguageServer),
                value: b(self.language_server),
            },
            SettingRow {
                label: i18n::t(lang, Key::SettingPlotsInTabs),
                value: plots_value(lang, self.plots_in_tabs, crate::wsnap::can_open_a_window()),
            },
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
            7 => self.completion = !self.completion,
            8 => self.language_server = !self.language_server,
            // Refused where it would mean nothing: see `plots_value`. The row still moves under
            // the cursor and still reads out the state — it is disabled, not hidden, because
            // "why can I not turn this off" is a question the value answers and an absence
            // does not.
            9 => {
                if crate::wsnap::can_open_a_window() {
                    self.plots_in_tabs = !self.plots_in_tabs;
                }
            }
            10 => self.mouse_enabled = !self.mouse_enabled,
            11 => self.lang = self.lang.next(),
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
