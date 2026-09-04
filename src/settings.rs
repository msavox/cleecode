use crate::i18n::{self, Key, Lang};
use serde::{Deserialize, Serialize};

/// How many rows the settings modal draws and the arrows can reach. Kept honest by
/// `every_setting_row_is_reachable`: it sat at 9 while `rows()` grew to a dozen, and the three
/// newest settings — where plots open, the mouse, the language — were drawn off the bottom of a
/// box sized from this number and skipped by a cursor that wrapped on it. A setting nobody can
/// see is a setting that does not exist.
pub const SETTINGS_COUNT: usize = 17;

pub const SIDEBAR_WIDTH_RANGE: (u16, u16) = (15, 60);
pub const TERMINAL_PCT_RANGE: (u16, u16) = (15, 70);
/// Left pane's share of the editor region when split. Kept away from the extremes so neither
/// pane can be squeezed to nothing.
pub const SPLIT_PCT_RANGE: (u16, u16) = (20, 80);
/// The agent drawer's share of the window. The floor is where an agent's own output starts
/// wrapping into nonsense; the ceiling leaves the editor more than half the screen, which is
/// the arrangement the drawer exists to sit beside rather than to replace.
pub const DRAWER_PCT_RANGE: (u16, u16) = (20, 60);
/// Wide enough for an agent to be readable beside the code it is reading, narrow enough that the
/// editor is still the thing you are looking at. Shared with the workspace file, whose `[drawer]`
/// block defaults to the same number when it says nothing.
pub const DRAWER_PCT_DEFAULT: u16 = 40;

/// Missing keys fall back to the default for that key alone, rather than throwing the file away.
///
/// Most fields here already said `#[serde(default)]` one at a time — every one added after the
/// first release had to, or that release's settings.toml would have stopped parsing. Saying it
/// once for the struct covers the handful from the first release that never did, and those were
/// the ones that made a hand-written file fail: the file is documented as hand-editable, and a
/// file holding just the one line the user meant to change would be set aside as broken and
/// silently replaced by the defaults.
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub show_line_numbers: bool,
    pub syntax_highlighting: bool,
    pub word_wrap: bool,
    pub tab_size: usize,
    pub insert_spaces: bool,
    pub show_whitespace: bool,
    pub auto_indent: bool,
    pub mouse_enabled: bool,
    #[serde(default, deserialize_with = "lang_however_it_is_spelled")]
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
    /// The agent drawer's percentage of the window. A percentage rather than a column count —
    /// the name would have been `drawer_width` — because it sits beside `terminal_pct` in every
    /// piece of arithmetic there is: the same seam drag, the same clamp, the same carve out of
    /// the main area. A fixed column count that is right at 200 columns crushes the editor at 80.
    #[serde(default = "default_drawer_pct")]
    pub drawer_pct: u16,
    /// Which agent the drawer's launcher opens highlighted — the last one started in it. Empty
    /// until one has been, and read through `Agent::of_program`, so a hand-edited name that no
    /// longer means anything costs the highlight and nothing else.
    #[serde(default)]
    pub drawer_agent: String,
    /// Whether the drawer is pinned — a column of the layout, with every other frame making room
    /// for it — or autocollapses, painted over the frames and gone again the moment the focus
    /// leaves it.
    ///
    /// The mode and the compositing are one setting rather than two because the driver is
    /// technical and not decorative: carving a column resizes every frame, and a resized frame is
    /// a `SIGWINCH` to the pty inside it — an interpreter and every TUI in a pane redraw whole,
    /// twice per summoning. Painting cells over the top is what the git panel already does, and
    /// it touches no pty at all. So pinned reflows, because it is part of the layout and the
    /// layout is what everything else is measured against; autocollapse overlays, because a
    /// question asked in passing must not cost half the screen a repaint on the way out and
    /// another on the way back.
    ///
    /// Pinned by default: a panel that moves by itself is a surprise until it is the one you
    /// asked for.
    #[serde(default = "default_drawer_pinned")]
    pub drawer_pinned: bool,
    /// Whether an agent started from the drawer is handed CleeCode's own MCP server already
    /// registered — a flag for claude and codex, a name in the environment for opencode and
    /// gemini. See `mcp.rs`, where all four mechanisms and their failure modes are written down.
    ///
    /// On, because the alternative is what the drawer used to do: start an agent that cannot see
    /// the editor it is sitting in until the user has gone and configured it by hand. Off is
    /// still a real answer and is the reason this key exists — nothing here touches the user's own
    /// configuration, but somebody who wants the agent in the drawer to be exactly the agent they
    /// get in a terminal, argument for argument, should be able to say so in one line. Agents
    /// typed by hand into an ordinary pane are unaffected either way; this only ever describes
    /// the one the drawer starts.
    #[serde(default = "default_true")]
    pub agent_mcp: bool,
    /// Whether an agent may change a buffer that has unsaved work in it: `"ask"`, `"allow"` or
    /// `"deny"`. See [`AgentEdits`], which is this string as the three answers it means.
    ///
    /// `"ask"` by default, because the thing being decided is whether a program may write into
    /// text the user has typed and not yet saved — the one place in this editor where something
    /// can be lost that no undo of theirs would bring back, since they did not make the change and
    /// may not have been looking at the pane. `"allow"` is for somebody who has decided to watch
    /// an agent work and would rather not be asked every time; `"deny"` turns the door off without
    /// turning off everything else the MCP server does, which is all reading.
    ///
    /// A string rather than a bool because there are three answers and a bool would have had to be
    /// two settings. Anything else written here reads as `"ask"`: an unrecognised value in a
    /// hand-edited file must not be the way to end up more permissive than the default.
    #[serde(default = "default_agent_edits")]
    pub agent_edits: String,
    // Menu bar visibility. On by default so newcomers keep the discoverable drop-down bar;
    // power users can hide it (Ctrl+B / View menu) and still reach menus via Ctrl+Shift+B.
    #[serde(default = "default_true")]
    pub show_menubar: bool,
    /// The one-row markdown formatting bar over the editor. Defaults on; hide it
    /// from the View menu once the syntax is in your fingers.
    #[serde(default = "default_true")]
    pub show_md_toolbar: bool,
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
    // Whether a file something else has just touched — an agent in one of the terminal panes, a
    // formatter, a `sed` in a shell — opens itself beside what you are reading. Off by default:
    // tabs appearing without being asked for is exactly the kind of help that has to be opted
    // into. It never takes the keyboard, and outside a git repository it does nothing at all and
    // says so, because the detection is the difference between two `git status` sweeps.
    #[serde(default)]
    pub follow_agent_edits: bool,
    // Whether a copy of every unsaved buffer is kept in the config directory while you type, and
    // offered back if CleeCode was not the one who decided to stop.
    //
    // On by default, because the alternative is the state CleeCode was in before it existed: a
    // `kill -9`, a stack overflow, a laptop losing power, and everything unsaved was gone with
    // no record that it had ever been there. Off is still a real answer — the copies are a
    // second version of your file sitting in ~/.config, and somebody working on something they
    // would rather not have two of on disk should be able to say no. See `recovery.rs` for what
    // it does and does not protect against. Deliberately without an interval knob: the tick is
    // five seconds, which is short enough not to matter and long enough to cost nothing.
    #[serde(default = "default_true")]
    pub autosave_recovery: bool,
    // Whether the title card appears at startup. On, because it is where a named workspace
    // announces itself and it costs under two seconds — but it is still two seconds in front of
    // the file you opened the editor to change, and somebody starting CleeCode twenty times a
    // day has seen the turtle. Off, the first frame is the editor.
    //
    // The command line already skips it when it was given a file to open: this is the same
    // decision made once instead of per invocation.
    #[serde(default = "default_true")]
    pub show_splash: bool,
    // Whether the terminal's own background is left showing through instead of the editor
    // painting its own. Off by default, because a theme is a set of colours *and* the surface
    // they were chosen against: handing it only the colours is how the text ends up on somebody
    // else's paper. Asking for it back is one press of the button on the menu bar, and a
    // translucent window with the desktop behind it is a good enough reason to.
    //
    // It used to read the other way round — `opaque_background`, off by default, with the dark
    // themes leaning on whatever the terminal happened to be. That default cost the switch its
    // meaning the moment the theme changed: `set_theme` repainted nothing, so a theme chosen to
    // fix an unreadable screen arrived without the surface it was drawn against and fixed
    // nothing, and the way out was a setting nobody thinks to look for. Inverting it puts the
    // choice where it belongs — transparency is the thing you ask for, and the next theme you
    // choose takes it back. An older settings.toml carrying `opaque_background` still loads:
    // the struct has no `deny_unknown_fields`, so the key it no longer knows is read as nothing
    // at all, this field takes its default, and the dead key goes on the next write out.
    #[serde(default)]
    pub transparent_background: bool,
    // Which set of colours the interface is drawn in. Defaults to the one the editor has always
    // used, so an existing settings.toml — which has no such key — loads into exactly the editor
    // it was written by.
    //
    // A *choice* rather than a theme: `theme = "auto"` is an instruction to ask the terminal
    // what colour it is, which is a question with a different answer on a different machine and
    // so cannot be stored as its answer. What the editor draws in is `App::theme`, resolved from
    // this once at startup.
    #[serde(default)]
    pub theme: crate::theme::ThemeChoice,
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
    /// Chords the user has moved: action name -> chord, overriding the built-in table one entry
    /// at a time. See `keymap.rs` for what the names and the chords are; everything not named
    /// here keeps the key CleeCode ships with, and a name or a chord this does not understand is
    /// a warning on the status line rather than a file that fails to load.
    ///
    /// Not written back out while it is empty, which is what keeps the Keybindings menu entry
    /// useful: the block it seeds is comments, and comments do not survive the exit that rewrites
    /// this file. Leaving an empty `[keys]` behind would mean the entry saw a section already
    /// there and never offered the list again.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub keys: std::collections::BTreeMap<String, String>,
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

/// Reads the language the way a person writes it by hand: `it`, `It` and `IT` all mean Italian.
///
/// serde matches enum variants by their exact Rust spelling, so `lang = "it"` — the obvious
/// thing to type, and the spelling every other program uses — failed to parse. It did not fail
/// alone: one unparsable key sinks the whole file, so every other preference in it was replaced
/// by a default and then written back over the original on exit. The tolerance costs a function;
/// the strictness cost the file.
fn lang_however_it_is_spelled<'de, D>(deserializer: D) -> Result<Lang, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let written = String::deserialize(deserializer)?;
    match written.trim().to_ascii_lowercase().as_str() {
        "en" => Ok(Lang::En),
        "it" => Ok(Lang::It),
        other => Err(serde::de::Error::custom(format!("unknown language {other:?}"))),
    }
}

fn default_true() -> bool {
    true
}

fn default_split_pct() -> u16 {
    50
}

fn default_drawer_pct() -> u16 {
    DRAWER_PCT_DEFAULT
}

fn default_drawer_pinned() -> bool {
    true
}

fn default_agent_edits() -> String {
    AgentEdits::Ask.word().to_string()
}

/// What an agent is allowed to do to a buffer with unsaved work in it.
///
/// The three answers `agent_edits` can be, kept as a type so that the decision is made in one
/// place: the string is what the file holds and what a person hand-edits, and every reader of it
/// goes through [`AgentEdits::of`] — which answers `Ask` to anything it does not recognise, so a
/// typo in a settings file is never the reason an agent is allowed to write.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AgentEdits {
    Ask,
    Allow,
    Deny,
}

impl AgentEdits {
    pub fn of(text: &str) -> AgentEdits {
        match text {
            "allow" => AgentEdits::Allow,
            "deny" => AgentEdits::Deny,
            _ => AgentEdits::Ask,
        }
    }

    /// How it is spelled in settings.toml.
    pub fn word(self) -> &'static str {
        match self {
            AgentEdits::Ask => "ask",
            AgentEdits::Allow => "allow",
            AgentEdits::Deny => "deny",
        }
    }

    /// The next one round, which is what picking the settings row does. Ordered from the most
    /// cautious answer outwards, so the first press of Enter on the default row is the one that
    /// asks for *less* interruption rather than for less care.
    pub fn next(self) -> AgentEdits {
        match self {
            AgentEdits::Ask => AgentEdits::Allow,
            AgentEdits::Allow => AgentEdits::Deny,
            AgentEdits::Deny => AgentEdits::Ask,
        }
    }

    fn label(self) -> Key {
        match self {
            AgentEdits::Ask => Key::SettingAgentEditsAsk,
            AgentEdits::Allow => Key::SettingAgentEditsAllow,
            AgentEdits::Deny => Key::SettingAgentEditsDeny,
        }
    }
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
    // `--no-gui`, and it is not decoration. On a machine with a display `octave` starts the
    // *graphical* Octave — a whole IDE in a window of its own, with its own editor and its own
    // figure windows — because that is what plain `octave` means there. Run on a `.m` file
    // therefore opened a second IDE beside this one, drew the plot in its window rather than in
    // the tab, and on this Mac fell over: `octave-gui` crashed. The preset has always said
    // `--no-gui`; the Run button never did, and the two disagreed for as long as both existed.
    //
    // `octave-cli` on Windows for the same reason and by another name: there is no `--no-gui`
    // wrapper there to ask.
    if cfg!(windows) { "octave-cli --persist {file}" } else { "octave --no-gui --persist {file}" }
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
            drawer_pct: default_drawer_pct(),
            drawer_agent: String::new(),
            drawer_pinned: default_drawer_pinned(),
            agent_mcp: true,
            agent_edits: default_agent_edits(),
            show_menubar: true,
            show_md_toolbar: true,
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
            follow_agent_edits: false,
            autosave_recovery: true,
            show_splash: true,
            transparent_background: false,
            theme: crate::theme::ThemeChoice::default(),
            last_root: None,
            last_open_files: Vec::new(),
            last_active_file: None,
            last_workspace: None,
            keys: std::collections::BTreeMap::new(),
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

/// The settings file itself. Public because two things outside this module need to recognise it
/// by path rather than by name: the menu entry that opens it for editing, and the save that has
/// to notice you have just saved *this* file and reload the keymap from it.
pub fn config_path() -> Option<std::path::PathBuf> {
    config_dir().map(|d| d.join("settings.toml"))
}

/// Re-reads just the `[keys]` table from disk, for a settings.toml that was edited in the editor
/// and saved a moment ago.
///
/// Only the chords, deliberately. Everything else in this file is also live state — the layout
/// the user has been dragging, the theme they picked from the menu — and reloading the whole
/// struct would throw the session's own changes away in favour of whatever was last written.
/// `None` means the file does not parse, which is worth saying out loud rather than reading as
/// "no overrides".
pub fn read_keys_from_disk() -> Option<std::collections::BTreeMap<String, String>> {
    let path = config_path()?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let parsed: Settings = toml::from_str(&text).ok()?;
    Some(parsed.keys)
}

/// Where a write to `path` actually belongs, following a symlink to the thing it points at.
///
/// Config files and source files alike are routinely symlinks into a dotfiles repository or a
/// shared checkout, and such a link is meant to be written *through*. Renaming onto the link
/// itself would replace it with an ordinary file and quietly detach it from the place the user
/// arranged for it to live — which they would only discover much later, when the repository
/// stopped seeing their edits.
fn resolve_link(path: &std::path::Path) -> std::path::PathBuf {
    if let Ok(real) = std::fs::canonicalize(path) {
        return real;
    }
    // A link whose target does not exist yet still says where the write belongs.
    match std::fs::read_link(path) {
        Ok(dest) if dest.is_absolute() => dest,
        Ok(dest) => path.parent().map(|parent| parent.join(&dest)).unwrap_or(dest),
        Err(_) => path.to_path_buf(),
    }
}

/// Writes `contents` to `path` so that the file on disk is either wholly the old content or
/// wholly the new one, never a truncated mixture.
///
/// A plain `fs::write` opens the target and truncates it before the first byte is written, so a
/// crash, a kill, or a full disk in the middle of a save leaves the user with a half a file and
/// no copy of the other half. Writing a sibling temp file and renaming it into place makes the
/// swap a single filesystem operation: readers see one version or the other.
///
/// The temp file must be a *sibling*, because rename is only atomic within one filesystem and
/// the system temp directory is routinely a different one. Its name carries the pid so two
/// CleeCode instances saving the same file cannot land on the same scratch file and interleave.
///
/// Permissions are copied from the file being replaced rather than left to the umask: a script
/// that was executable has to stay executable, and a private file must not come back
/// world-readable.
pub fn write_atomic(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    let target = resolve_link(path);
    let dir = match target.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    };
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temp = dir.join(format!(".{name}.clee{}.tmp", std::process::id()));

    let existing = std::fs::metadata(&target).ok();
    let written = std::fs::write(&temp, contents)
        .and_then(|()| match &existing {
            Some(meta) => std::fs::set_permissions(&temp, meta.permissions()),
            None => Ok(()),
        })
        .and_then(|()| std::fs::rename(&temp, &target));
    if written.is_err() {
        // Nothing else will ever look at it, and a scratch file left beside somebody's source
        // is litter that outlives the failure that made it.
        let _ = std::fs::remove_file(&temp);
    }
    written
}

impl Settings {
    /// Loads persisted settings from disk, falling back to defaults if there's nothing
    /// saved yet or the file can't be read/parsed.
    pub fn load() -> Self {
        let mut settings = match config_path() {
            Some(path) => Self::read_or_set_aside(&path),
            None => Settings::default(),
        };
        settings.merge_run_command_defaults();
        settings
    }

    /// Reads the settings file, and when it exists but cannot be parsed, moves it aside to
    /// `settings.toml.broken` before starting from the defaults.
    ///
    /// Falling back to defaults is not on its own enough, and used to be actively destructive:
    /// the settings are written back out on exit, so a single typo in a hand-edited file cost the
    /// entire file — parsed as nothing, started from defaults, saved over the evidence before the
    /// user had any idea something was wrong. Keeping their own text where they can look at it
    /// and fix it turns that into a recoverable mistake. A previous `.broken` is worth less than
    /// the file that just failed, so it is overwritten rather than left to accumulate.
    fn read_or_set_aside(path: &std::path::Path) -> Settings {
        let Ok(text) = std::fs::read_to_string(path) else { return Settings::default() };
        match toml::from_str(&text) {
            Ok(settings) => settings,
            Err(_) => {
                let _ = std::fs::rename(path, path.with_extension("toml.broken"));
                Settings::default()
            }
        }
    }

    /// Fills in run commands for extensions a saved file doesn't mention, so defaults added in
    /// a later version reach existing users instead of only new installs — `run_commands` is
    /// persisted whole, so without this it would be frozen at whatever the first run wrote.
    /// Entries the user actually edited are left alone.
    fn merge_run_command_defaults(&mut self) {
        for (ext, command) in default_run_commands() {
            self.run_commands.entry(ext).or_insert(command);
        }
        // Octave commands nobody would have chosen deliberately, upgraded in place.
        //
        // The first two are from before `--persist`, and closed every plot the moment a script
        // ended. The third is the one that shipped as a default until 0.11.1: without
        // `--no-gui`, `octave` starts the graphical Octave, so Run opened a second IDE in its
        // own window — and that is where the plot went instead of into the tab. Anybody who ran
        // a `.m` file has that string saved in their settings.toml, so leaving it would leave
        // the bug fixed only for new installs.
        //
        // Matched exactly, and only against the strings CleeCode itself wrote. A command the
        // user has edited is theirs, whatever it says.
        let stale = matches!(
            self.run_commands.get("m").map(String::as_str),
            Some("octave {file}" | "octave-cli {file}" | "octave --persist {file}")
        );
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
            let _ = write_atomic(&path, text.as_bytes());
        }
    }

    /// Which way the transparency switch goes from here, or `None` when it has nothing to move.
    ///
    /// Split out from the app so the rule can be read — and tested — without a running editor. A
    /// theme with dark text has no terminal colours left to reveal: handing the background back
    /// would not make it translucent, it would leave the text on whatever the terminal happens
    /// to be, which for most people is dark on dark. Those themes refuse the switch instead of
    /// accepting it and changing nothing.
    pub fn next_transparent_background(&self, theme: crate::theme::Theme) -> Option<bool> {
        if theme.paints_its_own_background() {
            return None;
        }
        Some(!self.transparent_background)
    }

    /// A chosen theme owns its background: choosing one gives back whatever transparency was in
    /// force, so the theme arrives with the surface it was drawn against rather than with the
    /// last one's. Answers whether there was any to give back, which is what tells the caller
    /// the file has to be written even when nothing else about the choice changed.
    ///
    /// Here rather than inside `set_theme` so the rule can be tested without a running editor,
    /// which is the only way an App-level method gets to be read by a test at all.
    pub fn reclaim_background_for_the_theme(&mut self) -> bool {
        std::mem::take(&mut self.transparent_background)
    }

    pub fn clamp_layout(&mut self) {
        self.sidebar_width = self.sidebar_width.clamp(SIDEBAR_WIDTH_RANGE.0, SIDEBAR_WIDTH_RANGE.1);
        self.terminal_pct = self.terminal_pct.clamp(TERMINAL_PCT_RANGE.0, TERMINAL_PCT_RANGE.1);
        self.split_pct = self.split_pct.clamp(SPLIT_PCT_RANGE.0, SPLIT_PCT_RANGE.1);
        self.drawer_pct = self.drawer_pct.clamp(DRAWER_PCT_RANGE.0, DRAWER_PCT_RANGE.1);
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
            let _ = write_atomic(&path, text.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    /// A settings file holding one key is the file somebody hand-edited, and it has to load as
    /// that key on top of the defaults. It used to be set aside as broken and replaced, which
    /// took the rest of their settings with it — the file is written back out on exit, so the
    /// defaults were saved over the evidence before anyone could see what happened.
    #[test]
    fn a_file_with_one_key_keeps_that_key_and_defaults_the_rest() {
        let one_line: Settings = toml::from_str("theme = \"turbo\"").expect("a partial file loads");
        assert_eq!(one_line.theme, crate::theme::ThemeChoice::Fixed(crate::theme::Theme::Turbo));
        let fresh = Settings::default();
        assert_eq!(one_line.tab_size, fresh.tab_size, "the rest is the defaults, not nothing");
        assert_eq!(one_line.show_line_numbers, fresh.show_line_numbers);
        // And an empty file is every default, rather than an error.
        let empty: Settings = toml::from_str("").expect("an empty file loads");
        assert_eq!(empty.theme, fresh.theme);
    }

    /// `auto` is a theme name like any other as far as the file is concerned: it loads on top of
    /// the defaults and takes nothing else with it. Written down here as well as in `theme.rs`
    /// because this is the file somebody hand-edits, and the failure it guards against — one
    /// unknown key setting the whole file aside — is this module's to prevent.
    #[test]
    fn a_file_asking_for_the_automatic_theme_keeps_the_rest() {
        let auto: Settings = toml::from_str("theme = \"auto\"\ntab_size = 3\n")
            .expect("a partial file asking for auto loads");
        assert_eq!(auto.theme, crate::theme::ThemeChoice::Auto);
        assert_eq!(auto.tab_size, 3, "the key beside it is still read");
        let fresh = Settings::default();
        assert_eq!(auto.show_line_numbers, fresh.show_line_numbers, "and the rest defaults");
        assert_eq!(auto.lang, fresh.lang);
        // And it survives being written back out on exit, which is what would otherwise quietly
        // turn the choice into whichever theme it happened to resolve to.
        let round_trip: Settings =
            toml::from_str(&toml::to_string_pretty(&auto).expect("settings serialise"))
                .expect("what was written loads");
        assert_eq!(round_trip.theme, crate::theme::ThemeChoice::Auto);
    }

    use super::*;

    /// A file holding nothing but `[keys]` is the file somebody wrote after using the
    /// Keybindings menu entry, and it has to load as those chords on top of every other default
    /// — not as a file that quietly redefines the editor around them.
    ///
    /// The reverse matters just as much and is the easier one to get wrong: a settings.toml
    /// written before this feature existed has no `[keys]` at all, and it has to keep loading.
    /// That is what `#[serde(default)]` on the field is for, and it is the lesson the 0.14
    /// themes left behind.
    #[test]
    fn a_file_of_nothing_but_keys_keeps_every_other_default() {
        let only_keys: Settings =
            toml::from_str("[keys]\nfind-in-project = \"Ctrl+Alt+F\"\n").expect("a [keys]-only file loads");
        assert_eq!(only_keys.keys.get("find-in-project").map(String::as_str), Some("Ctrl+Alt+F"));
        let fresh = Settings::default();
        assert_eq!(only_keys.tab_size, fresh.tab_size);
        assert_eq!(only_keys.theme, fresh.theme);
        assert_eq!(only_keys.terminal_scrollback, fresh.terminal_scrollback);
        assert!(!only_keys.run_commands.is_empty() || fresh.run_commands.is_empty());

        // A file from before `[keys]` existed has none, and that is not an error.
        let older: Settings = toml::from_str("theme = \"turbo\"").expect("an older file loads");
        assert!(older.keys.is_empty());

        // Round trip: what is written out is what comes back.
        let mut mine = Settings::default();
        mine.keys.insert("manual".to_string(), "F1".to_string());
        let text = toml::to_string_pretty(&mine).expect("settings serialise");
        let back: Settings = toml::from_str(&text).expect("and parse again");
        assert_eq!(back.keys.get("manual").map(String::as_str), Some("F1"));

        // An empty table is not written at all, so the commented block the Keybindings entry
        // seeds is offered again after an exit rather than suppressed by an empty `[keys]`.
        let text = toml::to_string_pretty(&Settings::default()).expect("settings serialise");
        assert!(!text.contains("[keys]"), "an empty keys table should not be written out:\n{text}");
    }

    /// The plot destination is a preference on a desktop and a fact on a server. It names both
    /// of its answers rather than reading on/off: "off" meant the interpreter's own windows,
    /// which is a different way of working and not a feature being switched off. Where the
    /// choice is not the user's, the row says which way it went and why — a switch that reads
    /// "off" while the tabs keep arriving is a broken switch.
    /// Run must not start the *graphical* Octave.
    ///
    /// Plain `octave` on a machine with a display is a whole IDE in a window of its own, with
    /// its own editor and its own figure windows — so Run opened a second one beside this, the
    /// plot went there instead of into the tab, and on one Mac `octave-gui` crashed outright.
    /// The preset always said `--no-gui`; the button did not.
    #[test]
    fn run_never_asks_for_the_graphical_octave() {
        let default = default_octave_command();
        assert!(default.contains("--no-gui") || default.starts_with("octave-cli"), "{default}");

        // And the string that shipped as the default until 0.11.1 is upgraded in place: anybody
        // who ran a .m file has it saved, so fixing only the default fixes only new installs.
        let mut settings = Settings::default();
        settings.run_commands.insert("m".to_string(), "octave --persist {file}".to_string());
        settings.merge_run_command_defaults();
        assert_eq!(settings.run_commands.get("m").map(String::as_str), Some(default));

        // A command the user wrote is theirs, whatever it says.
        let mine = "octave --persist --eval \"disp(1)\" {file}".to_string();
        let mut settings = Settings::default();
        settings.run_commands.insert("m".to_string(), mine.clone());
        settings.merge_run_command_defaults();
        assert_eq!(settings.run_commands.get("m"), Some(&mine));
    }

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

    /// The drawer's row reads out which of its two modes it is in, in both languages, and
    /// picking it moves between them. Pinned is the default, and that is the decision: a panel
    /// that moves by itself is a surprise until it is the one you asked for.
    #[test]
    fn the_drawer_row_says_which_mode_it_is_in() {
        assert!(Settings::default().drawer_pinned, "pinned until asked otherwise");
        for lang in [Lang::En, Lang::It] {
            let mut settings = Settings { lang, ..Settings::default() };
            // Found by its label rather than by a number written here: `activate` is a match on
            // the index, and the point of the test is that the row and the arm are the same row.
            let label = i18n::t(lang, Key::SettingDrawerMode);
            let idx = settings
                .rows()
                .iter()
                .position(|r| r.label == label)
                .expect("the drawer's mode has a row");
            let value = |s: &Settings| s.rows()[idx].value.clone();
            assert_eq!(value(&settings), i18n::t(lang, Key::SettingDrawerPinned));
            settings.activate(idx);
            assert!(!settings.drawer_pinned);
            assert_eq!(value(&settings), i18n::t(lang, Key::SettingDrawerAutocollapse));
            settings.activate(idx);
            assert_eq!(value(&settings), i18n::t(lang, Key::SettingDrawerPinned), "two states, a ring");
        }
    }

    /// `save()` swallows serialization errors, so a field ordering that TOML rejects (a
    /// scalar emitted after a table) would silently stop settings from persisting at all.
    /// The modal is sized from `SETTINGS_COUNT` and the cursor wraps on it, so a row past that
    /// number is drawn nowhere and reachable by nothing. This is how three settings — where
    /// plots open, the mouse, the language — went missing while the constant stayed at 9.
    #[test]
    fn every_setting_row_is_reachable() {
        let settings = Settings::default();
        assert_eq!(settings.rows().len(), SETTINGS_COUNT);
    }

    /// `activate` is a match on the row's index, so a row inserted in the middle without
    /// renumbering below it silently flips its neighbour instead. Every row has to *do*
    /// something: toggled twice, each one comes back to where it started, and toggled once, at
    /// least one thing about the settings is different.
    #[test]
    fn every_row_changes_something_of_its_own() {
        for idx in 0..SETTINGS_COUNT {
            let mut settings = Settings::default();
            let before = settings.rows();
            settings.activate(idx);
            let after = settings.rows();
            // On a machine with no display, the plots row has one honest value regardless of
            // what's asked for, and `activate` refuses to change it — a documented 0.12.1
            // decision (an off that reads while tabs keep arriving is a broken switch), already
            // pinned by `plots_value`'s own test above. The row says so itself, so skip the
            // "it changed" assertion here rather than fail on a refusal that is the feature.
            let plots_row_declares_nothing_to_choose =
                before[idx].value == i18n::t(settings.lang, Key::SettingPlotsNoDisplay);
            if !plots_row_declares_nothing_to_choose {
                assert_ne!(
                    before[idx].value, after[idx].value,
                    "row {idx} ({}) did not change when it was picked", before[idx].label
                );
            }
            // The language row is the one exception, and not a leak: it repaints every other
            // row in the other language, which is the whole of what it does.
            if settings.lang == Settings::default().lang {
                for (other, (was, now)) in before.iter().zip(after.iter()).enumerate() {
                    assert!(
                        other == idx || was.value == now.value,
                        "picking row {idx} changed row {other} ({})", now.label
                    );
                }
            }
        }
    }

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

    /// The switch that hands the background back, and the one theme that will not take it: a
    /// dark theme flips, a light one answers `None` so the caller can say why rather than moving
    /// a control that changes nothing.
    #[test]
    fn the_transparency_switch_flips_on_a_dark_theme_and_is_refused_by_a_light_one() {
        use crate::theme::Theme;
        let mut settings = Settings::default();
        assert!(!settings.transparent_background, "every theme paints its own surface by default");

        // On a dark theme it goes both ways, from wherever it is.
        assert_eq!(settings.next_transparent_background(Theme::CleeCode), Some(true));
        settings.transparent_background = true;
        assert_eq!(settings.next_transparent_background(Theme::CleeCode), Some(false));

        // A light theme refuses either way round: its dark text has no terminal colours left to
        // reveal, so there is nothing for the switch to turn.
        for asked in [true, false] {
            settings.transparent_background = asked;
            assert_eq!(settings.next_transparent_background(Theme::CleeCodeLight), None);
        }

        // And what the switch writes survives the file it is written to, which is the whole
        // reason it is saved at once rather than at exit.
        settings.transparent_background = true;
        let text = toml::to_string_pretty(&settings).expect("settings serialise");
        let back: Settings = toml::from_str(&text).expect("and parse again");
        assert!(back.transparent_background, "the choice comes back next session");
    }

    /// The bug the inversion was decided by, as a rule: choosing a theme takes the background
    /// back, so the theme that arrives is the one that was chosen and not its colours over the
    /// last theme's surface. The answer tells `set_theme` whether the file has to be written
    /// even when the theme itself did not change, which is what a user re-choosing the theme
    /// they are already on is asking for.
    #[test]
    fn choosing_a_theme_takes_its_background_back() {
        let mut settings = Settings::default();
        assert!(!settings.transparent_background, "which is where a session starts");
        // The switch has been pressed since: the terminal is showing through the editor.
        settings.transparent_background = true;
        assert!(settings.reclaim_background_for_the_theme(), "there was transparency to give back");
        assert!(!settings.transparent_background, "and the theme arrives with its own surface");

        // Nothing to reclaim the second time, and nothing for the caller to write out.
        assert!(!settings.reclaim_background_for_the_theme());
        assert!(!settings.transparent_background);

        // What that leaves the switch meaning: whichever dark theme was chosen, the next press
        // is a request for transparency rather than the one that hands it back.
        for theme in [crate::theme::Theme::CleeCode, crate::theme::Theme::Turbo] {
            assert_eq!(settings.next_transparent_background(theme), Some(true), "{}", theme.name());
        }
    }

    /// A settings.toml written before the inversion carries `opaque_background`, which no longer
    /// exists. It has to load as the file it is rather than be set aside as broken — the struct
    /// takes unknown keys as nothing at all — and the editor it loads into is the new default:
    /// the theme paints its own surface, whichever way the dead key was set.
    #[test]
    fn a_file_naming_the_old_opaque_key_still_loads_and_gets_the_new_default() {
        let older: Settings = toml::from_str("opaque_background = true\ntab_size = 3\n")
            .expect("a file with the retired key loads");
        assert!(!older.transparent_background, "the retired key decides nothing");
        assert_eq!(older.tab_size, 3, "and the keys beside it are still read");

        // Off in the old file meant a dark theme leaning on the terminal; it reads the same way,
        // because the answer now comes from the theme rather than from this key.
        let off: Settings = toml::from_str("opaque_background = false").expect("either way round");
        assert!(!off.transparent_background);

        // Written back out, the dead key is simply gone.
        let text = toml::to_string_pretty(&older).expect("settings serialise");
        assert!(!text.contains("opaque_background"), "the retired key is not written again:\n{text}");
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

    /// Every key that has no default of its own, with values chosen to differ from the defaults
    /// so a test can tell "the file was read" from "the file was thrown away".
    const HAND_WRITTEN: &str = "show_line_numbers = false\nsyntax_highlighting = true\n\
        word_wrap = true\ntab_size = 3\ninsert_spaces = false\nshow_whitespace = true\n\
        auto_indent = true\nmouse_enabled = false\nshow_sidebar = true\nshow_terminal = true\n\
        sidebar_width = 42\nterminal_pct = 41\nterminal_on_right = true\n";

    /// `lang = "it"` is the obvious thing to type and the spelling every other program uses, and
    /// it used to sink the whole file: one unparsable key means no settings at all, so every
    /// other preference reverted to a default and was written back over the original on exit.
    /// The tolerance is only worth having if the rest of the file survives with it, so that is
    /// what is checked.
    #[test]
    fn a_hand_written_language_parses_however_it_was_typed_and_costs_nothing_else() {
        for (written, expected) in
            [("it", Lang::It), ("It", Lang::It), ("IT", Lang::It), ("en", Lang::En), ("EN", Lang::En)]
        {
            let text = format!("{HAND_WRITTEN}lang = \"{written}\"\n");
            let s: Settings = toml::from_str(&text)
                .unwrap_or_else(|e| panic!("lang = \"{written}\" must parse: {e}"));
            assert_eq!(s.lang, expected);
            assert_eq!(s.tab_size, 3);
            assert_eq!(s.sidebar_width, 42);
            assert_eq!(s.terminal_pct, 41);
            assert!(s.word_wrap && s.show_whitespace && s.terminal_on_right);
            assert!(!s.show_line_numbers && !s.insert_spaces && !s.mouse_enabled);
        }

        // A language nobody speaks is still an error rather than a silent English: the file is
        // then set aside intact, which is a question the user can answer.
        assert!(toml::from_str::<Settings>(&format!("{HAND_WRITTEN}lang = \"martian\"\n")).is_err());
        // Absent entirely it simply falls back, the way every other optional key does.
        let s: Settings = toml::from_str(HAND_WRITTEN).expect("a file with no language must load");
        assert_eq!(s.lang, Lang::default());
    }

    /// The old fallback-to-defaults was quietly destructive: settings are written back out on
    /// exit, so a typo in a hand-edited file cost the whole file before anyone noticed anything
    /// was wrong. The defaults still come back — the app has to start — but the user's own text
    /// is kept where they can read it.
    #[test]
    fn a_broken_settings_file_is_set_aside_rather_than_written_over() {
        let dir = std::env::temp_dir().join(format!("clee_broken_settings_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.toml");
        let broken = dir.join("settings.toml.broken");

        // A file that parses is read, and nothing is moved anywhere.
        std::fs::write(&path, HAND_WRITTEN).unwrap();
        assert_eq!(Settings::read_or_set_aside(&path).tab_size, 3);
        assert!(path.exists() && !broken.exists());

        let mangled = format!("{HAND_WRITTEN}this is not toml {{{{{{\n");
        std::fs::write(&path, &mangled).unwrap();
        assert_eq!(Settings::read_or_set_aside(&path).tab_size, Settings::default().tab_size);
        assert!(!path.exists(), "the unparsable file moves aside instead of waiting to be overwritten");
        assert_eq!(std::fs::read_to_string(&broken).unwrap(), mangled);

        // A second failure replaces the first one: the file that just broke is the interesting
        // one, and a directory of numbered wreckage helps nobody.
        std::fs::write(&path, "@@@\n").unwrap();
        Settings::read_or_set_aside(&path);
        assert_eq!(std::fs::read_to_string(&broken).unwrap(), "@@@\n");

        // Nothing on disk at all is not a breakage, and leaves no .broken behind.
        std::fs::remove_file(&broken).unwrap();
        assert_eq!(Settings::read_or_set_aside(&path).tab_size, Settings::default().tab_size);
        assert!(!broken.exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Config files live in dotfiles repositories as often as not, reached through a symlink.
    /// Renaming onto the link would replace it with an ordinary file and detach it from the
    /// repository — silently, and for good.
    #[cfg(unix)]
    #[test]
    fn an_atomic_write_goes_through_a_symlink_rather_than_over_it() {
        let dir = std::env::temp_dir().join(format!("clee_atomic_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let real = dir.join("real.toml");
        let link = dir.join("link.toml");
        std::fs::write(&real, "old\n").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        write_atomic(&link, b"new\n").expect("writing through a link must work");
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new\n");
        assert!(
            std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(),
            "the link is still a link"
        );

        // A file that does not exist yet is created just as readily, and neither path leaves a
        // scratch file beside its target.
        let fresh = dir.join("fresh.toml");
        write_atomic(&fresh, b"hello\n").expect("a new file must be created");
        assert_eq!(std::fs::read_to_string(&fresh).unwrap(), "hello\n");
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, vec!["fresh.toml".to_string(), "link.toml".to_string(), "real.toml".to_string()]);

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

/// What the agent drawer's row reads: which of its two modes it is in.
///
/// A value row rather than an on/off one, for the reason the plot row is: neither state is the
/// absence of the other, so a switch labelled with one of the two names would leave the reader
/// guessing what "off" is. Each name says what it does to the rest of the screen, which is the
/// only difference between them a user can see.
fn drawer_mode_value(lang: i18n::Lang, pinned: bool) -> String {
    i18n::t(lang, if pinned { Key::SettingDrawerPinned } else { Key::SettingDrawerAutocollapse })
        .to_string()
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
                label: i18n::t(lang, Key::SettingFollowAgentEdits),
                value: b(self.follow_agent_edits),
            },
            SettingRow {
                label: i18n::t(lang, Key::SettingAgentEdits),
                value: i18n::t(lang, AgentEdits::of(&self.agent_edits).label()).to_string(),
            },
            SettingRow {
                label: i18n::t(lang, Key::SettingAutosaveRecovery),
                value: b(self.autosave_recovery),
            },
            SettingRow {
                label: i18n::t(lang, Key::SettingPlotsInTabs),
                value: plots_value(lang, self.plots_in_tabs, crate::wsnap::can_open_a_window()),
            },
            SettingRow {
                label: i18n::t(lang, Key::SettingDrawerMode),
                value: drawer_mode_value(lang, self.drawer_pinned),
            },
            SettingRow { label: i18n::t(lang, Key::SettingSplash), value: b(self.show_splash) },
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
            9 => self.follow_agent_edits = !self.follow_agent_edits,
            // Three states, so picking the row is walking round them. See `AgentEdits::next`.
            10 => self.agent_edits = AgentEdits::of(&self.agent_edits).next().word().to_string(),
            11 => self.autosave_recovery = !self.autosave_recovery,
            // Refused where it would mean nothing: see `plots_value`. The row still moves under
            // the cursor and still reads out the state — it is disabled, not hidden, because
            // "why can I not turn this off" is a question the value answers and an absence
            // does not.
            12 => {
                if crate::wsnap::can_open_a_window() {
                    self.plots_in_tabs = !self.plots_in_tabs;
                }
            }
            // Two states, so picking the row is cycling the pair. Nothing is touched but the
            // flag: the mode changes what the next frame is laid out as, and the pty in the
            // drawer never hears about it.
            13 => self.drawer_pinned = !self.drawer_pinned,
            14 => self.show_splash = !self.show_splash,
            15 => self.mouse_enabled = !self.mouse_enabled,
            16 => self.lang = self.lang.next(),
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
