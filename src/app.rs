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
use std::path::{Path, PathBuf};
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
#[allow(clippy::too_many_arguments, reason = "one row list, and every argument is a source of rows")]
fn run_rows(
    ext: &str,
    active: Option<&str>,
    available: &[String],
    registered: &[settings::RegisteredVenv],
    run_commands: &std::collections::HashMap<String, String>,
    project_commands: &std::collections::HashMap<String, String>,
    session: SessionTarget,
    lang: Lang,
) -> Vec<RunRow> {
    let mut rows = Vec::new();
    // The session comes first, and only for a language that can hold one. It is the top of the
    // list because when there *is* a prompt open it is nearly always the answer: the point of
    // `clee -w pylab` is that the session is where the work is, and a Run that started a fresh
    // interpreter every time threw away everything the last one held.
    if session.possible {
        rows.push(RunRow {
            label: i18n::msg_run_session_row(lang).to_string(),
            // Says whether there is one right now, because the row means different things
            // either way — with nothing open it is a preference, not a destination.
            detail: Some(i18n::msg_run_session_detail(lang, session.open).to_string()),
            // Marked only when it is what Run would really do. With the preference on and no
            // prompt open, Run falls back to a shell, and the tick belongs on the venv it
            // would use — which is the whole reason this marker exists.
            active: session.wanted && session.open,
            action: RunRowAction::UseSession,
        });
    }
    if is_python_ext(ext) {
        rows.push(RunRow {
            label: i18n::t(lang, Key::ToolbarVenvNone).to_string(),
            detail: None,
            active: active.is_none() && !(session.wanted && session.open),
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
                active: active == Some(venv.as_str()) && !(session.wanted && session.open),
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

/// What the drop-down needs to know about running in a live session, rather than working it out
/// itself — the answer needs the process table and the settings, and neither belongs in a
/// function whose job is to lay out rows.
#[derive(Clone, Copy, Default)]
pub struct SessionTarget {
    /// Whether this language can hold a session at all: Octave and Python can, LaTeX cannot.
    pub possible: bool,
    /// Whether one is running in a pane right now.
    pub open: bool,
    /// Whether the user has asked for it, which is a preference and survives there being none.
    pub wanted: bool,
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
    /// Hand the file to the interpreter that is already running, instead of starting one.
    UseSession,
    /// Use this venv, or the system python for `None`. Turns the session off: they are two
    /// answers to one question, and a list where both can be ticked answers neither.
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

/// A completion question out to the language server, and the popup it was asked on behalf of.
///
/// The answer comes back frames later, into an editor that may have moved on. These three fields
/// are what makes the reply refusable: the id says it is the answer to *this* question, and the
/// buffer and the word's start say the popup is still the one that asked.
struct PendingCompletion {
    id: i64,
    editor: usize,
    start: usize,
}

/// A file handed to a live session, watched until the prompt comes back.
///
/// What it is for: knowing which figures that run opened, so running the same file again can
/// close them and land in the same tabs. Anything the session gains while this is set belongs to
/// the run — which is why it is *closed* the moment the prompt returns rather than left open
/// until the next run. A plot typed by hand at the prompt afterwards is not part of any run, and
/// a rerun must not take it away.
struct RunWatch {
    /// The file that was run, which is what the figures are remembered against.
    file: PathBuf,
    /// Which language's sessions to read. A `.m` cannot open a matplotlib figure.
    language: crate::session::Language,
    /// Which window the command went to, so the right pane is asked whether it is done.
    terminal: usize,
    /// The figures that were already open, and are therefore somebody else's.
    before: Vec<i64>,
    /// The figures that have appeared since. Grown on every tick, because a script that draws
    /// four figures is only holding one of them when the first snapshot arrives.
    opened: Vec<i64>,
    /// When the command was typed. The prompt is still reading keys for the moment after that —
    /// the shell has not started the command yet — so "the prompt is back" only counts once the
    /// pane has been seen busy, or once this has run out for a script too quick to catch at it.
    started: std::time::Instant,
    busy_seen: bool,
    /// When the sessions were last read for their figures. A run can last minutes, and asking
    /// every frame would read every snapshot on disk thirty times a second for the whole of it.
    looked: std::time::Instant,
    /// Since when the prompt has been back. The figures are not in the snapshot at that instant:
    /// the hook writes it as the prompt is drawn (Python) or once the interpreter is already
    /// waiting for input (Octave), so a run finalised on the tick the prompt returns records
    /// nothing at all — which is what made the *second* run of a script open a second set of
    /// tabs and only the third behave. Held open a moment longer, and read every tick meanwhile.
    settled: Option<std::time::Instant>,
}

/// How often a watched run's session is asked what it is holding.
const RUN_WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

/// How long the prompt has to have been back before a run counts as finished. Long enough for
/// the hook to have written the snapshot that says what the run drew — measured at a few tens of
/// milliseconds for both languages — and short enough that nothing typed afterwards is caught.
const RUN_SETTLE: std::time::Duration = std::time::Duration::from_millis(500);

/// How long a run that was never caught mid-command is watched before it is called finished.
/// A script that draws and returns inside one frame is over long before this; what the wait
/// protects against is the opposite mistake, of calling a run finished while it is still
/// starting up and attributing its figures to nobody.
const RUN_WATCH_MAX: std::time::Duration = std::time::Duration::from_secs(2);

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
    /// The word-completion popup, when it is up. The one overlay in this list that does not take
    /// the keyboard: it claims five keys and lets every other one through to the editor.
    pub completion: Option<crate::complete::Popup>,
    /// The terminal's width when CleeCode started. Stands in for the window width before the
    /// first frame has been drawn, which is when a workspace named on the command line is
    /// applied — and a preset that shaped itself for a zero-width window would be no preset.
    startup_cols: u16,
    /// Where the cursor was drawn last frame, so the popup can hang under it. Written by the
    /// renderer, which is the only thing that knows where a buffer line landed on screen.
    pub completion_anchor: (u16, u16),
    /// The language server, once a file it knows about has been opened. `None` is the ordinary
    /// state, not a failure: most machines do not have one installed.
    /// The running language servers, by program name. One process per program rather than per
    /// language: `clangd` serves seven extensions, and one of it per extension would be seven
    /// clangds indexing the same project.
    lsp: std::collections::HashMap<String, crate::lsp::Client>,
    /// Why there is no server, said once. Kept so the attempt is not repeated at every keystroke
    /// — spawning a process that is not there, sixty times a second, is its own kind of bug.
    /// Why a server did not start, by program name — so a machine with `gopls` and without
    /// `clangd` still gets Go, and neither is tried twice.
    lsp_error: std::collections::HashMap<String, String>,
    /// What the server says about each file. Replaced wholesale per file, because that is what
    /// the protocol sends: a list, not a diff.
    pub diagnostics: std::collections::HashMap<PathBuf, Vec<crate::lsp::Mark>>,
    /// The buffer revision last sent for each file, and the revision seen with the moment it
    /// appeared — together they are "has it changed, and has the typing stopped".
    lsp_sent: std::collections::HashMap<PathBuf, u64>,
    lsp_seen: std::collections::HashMap<PathBuf, (u64, Instant)>,
    /// The absolute path each open file was announced under, and the path the editor holds it
    /// by. A tab opened from the tree of a project started as `.` is `./src/main.rs`, which has
    /// no `file:` URI at all, and what comes back from the server is neither of those but the
    /// resolved one — so the translation is kept rather than recomputed, and the file is asked
    /// about the disk once instead of sixty times a second.
    lsp_paths: std::collections::HashMap<PathBuf, PathBuf>,
    /// The one completion question currently out to the server, if any.
    ///
    /// One, not a queue: the popup that would receive an older answer is gone by the time a
    /// newer question is asked, so keeping the earlier ones would only be keeping answers
    /// nobody can use.
    lsp_completion: Option<PendingCompletion>,
    /// The one definition or hover request still out.
    ///
    /// One at a time, and the newest wins: both are questions about where the cursor is now, so
    /// an answer to where it was is not a late answer, it is an answer about somewhere else.
    lsp_asked: Option<PendingAsk>,
    /// Where the cursor was when the last hover was asked, so the same question is not asked
    /// again every frame while nothing moves.
    lsp_hovered: Option<(PathBuf, usize, usize)>,
    /// The one line the server had to say about what is under the cursor.
    lsp_what_it_is: Option<String>,
    /// Where you were before each jump to a definition, newest last.
    ///
    /// A stack rather than one remembered place: following a definition into a definition into a
    /// definition is the ordinary way of reading unfamiliar code, and a single slot would strand
    /// you two files from where you started.
    jumps: Vec<(PathBuf, usize, usize)>,
    /// The snapshot watched for figures. The editor reads it for pictures; the workspace window
    /// reads the same file for variables. Two readers of one file, which is why the producers
    /// write by rename — neither can ever see half of one.
    figures: Option<crate::wsnap::Watch>,
    /// When each figure's picture was last written, so a snapshot can be read for *what changed*
    /// rather than for what exists. Keyed by the PNG's path, which is what identifies a figure
    /// everywhere else here. Kept on the app and not on the watch above: which snapshot file is
    /// the newest changes as panes take turns ticking, and a figure does not stop being the one
    /// already on screen because a different session wrote last.
    figure_drawn: std::collections::HashMap<PathBuf, std::time::SystemTime>,
    /// What the last run of each file left open in its session, by figure number. Read when the
    /// same file is run again: those figures are closed first, so the rerun draws into the tabs
    /// that are already there instead of opening a second set beside them.
    run_figures: std::collections::HashMap<PathBuf, Vec<i64>>,
    /// The run being watched, while it runs. See [`RunWatch`].
    run_watch: Option<RunWatch>,
    /// The variable inspector, when it is open: which name, where in it we are looking, and the
    /// file the session writes its answer to.
    pub inspector: Option<Inspector>,
    /// Which lines have a breakpoint, by file. Kept here rather than on the editor because they
    /// belong to the session, not to the buffer: closing a tab does not clear them.
    pub breakpoints: std::collections::HashMap<PathBuf, std::collections::BTreeSet<usize>>,
    /// Where the session was last seen stopped, so the editor can stop marking the line once it
    /// runs on.
    pub stopped_at: Option<(PathBuf, usize)>,
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
    /// Which terminal row was last clicked, and when, so the second click on the same one can
    /// mean "take me there".
    last_terminal_click: Option<(usize, u16, Instant)>,
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
    /// The box asking what to look for across the project, when open.
    pub show_search: bool,
    pub search_input: String,
    /// How the project search reads its query. Kept on the app rather than in the Find box: a
    /// search across files outlives the box it was typed in, and asking for a pattern twice —
    /// once per place you can search — would be asking the same question twice.
    pub search_regex: bool,
    pub search_case_sensitive: bool,
    search_tx: Sender<crate::search::Outcome>,
    search_rx: Receiver<crate::search::Outcome>,
    search_pending: Arc<AtomicBool>,
    /// Where "the current editor" points when there is no current editor.
    ///
    /// Closing the last tab used to put an untitled buffer back, because the eighty-odd places
    /// that reach for the open file cannot each grow an arm for there not being one. This is the
    /// other way of keeping that promise: a real buffer that no tab points at and nothing draws.
    /// Nothing reaches it in normal use — the editor stops taking keys when its pane is empty,
    /// and the renderer draws the empty state instead of a buffer — so it stays empty. It is
    /// here so that a caller which asks anyway gets a buffer rather than a panic.
    scratch: Editor,
    /// The read-only git panel, when open.
    pub git_panel: Option<GitPanel>,
    git_panel_tx: Sender<GitMessage>,
    git_panel_rx: Receiver<GitMessage>,
    /// A question asked from outside the panel, waiting for the panel to be able to ask it.
    ///
    /// Neither can go up until the snapshot is in — one names a row of a list that does not
    /// exist yet, the other has to know how many files are staged — so the request is held here
    /// and tried again when the answer lands.
    git_wanted: Option<GitWanted>,
    /// How many times the panel has asked git what it looks like.
    ///
    /// Each ask runs on a thread of its own, and threads finish in whatever order they finish
    /// in: two asks in flight — pressing `R` twice, or pressing it while a write is landing —
    /// could deliver the older answer last and leave the panel showing the state from before the
    /// action it had just taken, with nothing on screen to say so. The answer carries the number
    /// it was asked under and anything but the latest is dropped.
    git_asked: u64,
}

/// Which of the five questions the git panel is answering.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GitTab {
    /// What is changed, file by file. First, because it is the one you came for: the diff tells
    /// you what you did, this tells you what to do next.
    Status,
    Diff,
    /// The history with its shape: every branch at once, drawn in lanes. This is the tab the
    /// flat list of the last fifty commits used to be — the graph says everything that list said
    /// and also which of those commits are on the branch you are standing on.
    Graph,
    Branches,
    Stashes,
}

impl GitTab {
    pub const ALL: [GitTab; 5] =
        [GitTab::Status, GitTab::Diff, GitTab::Graph, GitTab::Branches, GitTab::Stashes];

    /// Whether the tab is a list you move a cursor through, as opposed to text you scroll.
    ///
    /// The difference is whether there is anything to *do* to a row. A diff line is not a thing
    /// you can act on, so a highlight on one would be a promise the panel does not keep.
    pub fn picks_a_row(self) -> bool {
        !matches!(self, GitTab::Diff)
    }

    fn cycle(self, delta: isize) -> GitTab {
        let at = Self::ALL.iter().position(|t| *t == self).unwrap_or(0) as isize;
        let len = Self::ALL.len() as isize;
        Self::ALL[(((at + delta) % len) + len) as usize % Self::ALL.len()]
    }
}

/// The git panel: which tab, how far down it, and the answers — `None` until they arrive, since
/// three `git` invocations on a large repository are not instant and the frame loop does not
/// wait for anything.
pub struct GitPanel {
    pub tab: GitTab,
    pub scroll: usize,
    /// The row the cursor is on, in the tabs that have one. Kept across a refresh — staging a
    /// file redraws the list, and being put back at the top after every action would make
    /// staging five files an exercise in counting down again.
    pub selected: usize,
    pub snap: Option<crate::git::Snapshot>,
    /// The graph laid out into rows, worked out once when a snapshot arrives rather than once a
    /// frame: the lane assignment is a walk over every commit, and the answer only changes when
    /// the commits do.
    pub rows: Vec<crate::git_graph::Row>,
    /// One commit opened in full, drawn over the graph. Esc closes this before it closes the
    /// panel, because leaving a reader means going back to what you were reading.
    pub detail: Option<GitDetail>,
    /// A question that has to be answered before anything else happens.
    pub prompt: Option<GitPrompt>,
    /// Whether a `git` that writes is still running. One at a time: two `git add`s racing for
    /// the index lock is a failure that reads as the panel ignoring a keystroke.
    pub busy: bool,
    /// What git said about the last thing it was asked to do, and whether it was a complaint.
    pub notice: Option<(String, bool)>,
    /// How many rows the list had room for when it was last drawn. Written by the renderer,
    /// which is the only thing that knows — the same arrangement as the completion popup's
    /// anchor, and for the same reason: keeping a selection on screen needs the height.
    pub body_rows: usize,
}

impl GitPanel {
    /// How many rows the tab in front of you has.
    pub fn len(&self) -> usize {
        // The graph is measured in drawn rows and not in commits: a merge costs two or three
        // rows, and a cursor counting commits would slide out of step with the picture it is
        // moving over. They are the panel's own rows rather than the snapshot's, which is why
        // this is asked before the snapshot is.
        if self.tab == GitTab::Graph {
            return self.rows.len();
        }
        let Some(snap) = self.snap.as_ref() else { return 0 };
        match self.tab {
            GitTab::Status => snap.changes.len(),
            GitTab::Diff => snap.diff.len(),
            GitTab::Branches => snap.branches.len(),
            GitTab::Stashes => snap.stashes.len(),
            GitTab::Graph => self.rows.len(),
        }
    }

    /// The commit the cursor is on, when it is on one. Rows that are only lines have none, and
    /// the cursor never rests on those.
    pub fn selected_commit(&self) -> Option<&crate::git::GraphCommit> {
        if self.tab != GitTab::Graph {
            return None;
        }
        let at = self.rows.get(self.selected)?.commit?;
        self.snap.as_ref()?.graph.get(at)
    }

    pub fn selected_stash(&self) -> Option<&crate::git::Stash> {
        if self.tab != GitTab::Stashes {
            return None;
        }
        self.snap.as_ref()?.stashes.get(self.selected)
    }

    /// The nearest row at or past `from` that draws a commit, walking in `delta`'s direction.
    ///
    /// The cursor lands on commits and never on the lines between them: a highlight on a row of
    /// `|/` would be offering an action on a piece of the drawing.
    fn commit_row(&self, from: isize, delta: isize) -> Option<usize> {
        let mut at = from;
        while at >= 0 && (at as usize) < self.rows.len() {
            if self.rows[at as usize].commit.is_some() {
                return Some(at as usize);
            }
            at += if delta == 0 { 1 } else { delta.signum() };
        }
        None
    }

    pub fn selected_change(&self) -> Option<&crate::git::Change> {
        if self.tab != GitTab::Status {
            return None;
        }
        self.snap.as_ref()?.changes.get(self.selected)
    }

    pub fn selected_branch(&self) -> Option<&crate::git::Branch> {
        if self.tab != GitTab::Branches {
            return None;
        }
        self.snap.as_ref()?.branches.get(self.selected)
    }

    /// How many files are staged — the number a commit is about to be made of.
    pub fn staged_count(&self) -> usize {
        self.snap.as_ref().map(|s| s.changes.iter().filter(|c| c.staged()).count()).unwrap_or(0)
    }

    /// The absolute path of the row the cursor is on. Absolute because git's own paths here are
    /// relative to the top of the working tree, and the panel is running wherever CleeCode was
    /// opened — which is not always the same place.
    pub fn selected_path(&self) -> Option<PathBuf> {
        let change = self.selected_change()?;
        let top = self.snap.as_ref()?.top.as_ref()?;
        Some(top.join(&change.path))
    }

    fn clamp_to_list(&mut self) {
        let max = self.len().saturating_sub(1);
        self.selected = self.selected.min(max);
        self.scroll = self.scroll.min(max);
        if self.tab == GitTab::Graph {
            // A refresh redraws the graph, and a row that was a commit can become a line: a new
            // commit above shifts everything down, and a merge adds two rows where there was one.
            let at = self.selected as isize;
            self.selected = self.commit_row(at, 1).or_else(|| self.commit_row(at, -1)).unwrap_or(0);
        }
        self.reveal();
    }

    /// Moves the cursor, or the view, depending on which the tab has.
    fn move_by(&mut self, delta: isize) {
        let max = self.len().saturating_sub(1) as isize;
        if !self.tab.picks_a_row() {
            self.scroll = (self.scroll as isize + delta).clamp(0, max) as usize;
            return;
        }
        let wanted = (self.selected as isize + delta).clamp(0, max);
        self.selected = if self.tab == GitTab::Graph {
            // Past the end of the graph in either direction, the cursor stays on the last commit
            // there is rather than sliding onto the lines below it.
            self.commit_row(wanted, delta)
                .or_else(|| self.commit_row(wanted, -delta.signum()))
                .unwrap_or(self.selected)
        } else {
            wanted as usize
        };
        self.reveal();
    }

    /// Scrolls just enough to keep the cursor on screen, and no further: a list that jumped to
    /// centre the selection would move rows the eye was using to keep its place.
    fn reveal(&mut self) {
        if !self.tab.picks_a_row() {
            return;
        }
        let rows = self.body_rows.max(1);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + rows {
            self.scroll = self.selected + 1 - rows;
        }
    }
}

/// A question asked from outside the panel and waiting on it.
#[derive(Clone)]
pub enum GitWanted {
    /// Throw away the changes to this file, once the panel can find the row it is on.
    Discard(PathBuf),
    /// Commit what is staged, once the panel knows how much that is.
    Commit,
}

/// A definition or hover request that has not been answered yet.
///
/// `from` is where the cursor was when it went out — checked when the answer arrives, since by
/// then it may be somewhere else entirely, and used as the place to come back to after a jump.
pub struct PendingAsk {
    pub id: i64,
    pub from: (PathBuf, usize, usize),
}

/// A question the panel is holding everything else for.
///
/// Two kinds, and the difference is the whole of the safety here. Something you *type* can be
/// abandoned by pressing Esc and costs nothing if you change your mind halfway. Something you
/// *agree to* takes a single letter and reads every other key as no — which is why the actions
/// that cannot be undone are all on that side of the line.
pub enum GitPrompt {
    Text { kind: GitText, typed: String },
    Confirm(GitConfirm),
}

/// The boxes you type into.
pub enum GitText {
    Commit,
    /// Replacing the last commit. Opens holding the message it already has: retyping a sentence
    /// to add one forgotten file is how a commit loses the sentence that explained it.
    Amend,
    /// A new branch, starting at a commit picked out of the graph or at HEAD when `at` is none.
    Branch { at: Option<String> },
    Tag { at: String },
    Stash,
}

/// The questions that take one letter.
pub enum GitConfirm {
    /// About to throw away the changes to a file. Holds the whole change rather than the path,
    /// because whether it is refused depends on what git knows about the file.
    Discard(crate::git::Change),
    DeleteBranch(String),
    /// Moving the branch to an older commit and making the working tree match it.
    ResetHard { hash: String, subject: String },
    DropStash(String),
}

impl GitConfirm {
    /// Whether the question is drawn in red, which here means one thing exactly: saying yes
    /// destroys something that is in no commit, no stash and no reflog.
    ///
    /// Deleting a branch is not in this list, and that is the distinction rather than an
    /// oversight — its commits stay in the reflog for ninety days, so it is a question worth
    /// asking and not a warning worth shouting. Red on everything is red on nothing.
    pub fn destroys_work(&self) -> bool {
        match self {
            GitConfirm::Discard(_) => true,
            // The commits it leaves behind are in the reflog; anything uncommitted it stepped on
            // is nowhere at all, and that is the half that decides the colour.
            GitConfirm::ResetHard { .. } => true,
            GitConfirm::DropStash(_) => true,
            GitConfirm::DeleteBranch(_) => false,
        }
    }
}

/// One commit read in full, over the top of the graph.
pub struct GitDetail {
    pub hash: String,
    pub subject: String,
    /// `None` while `git show` is still running: a large commit is not instant, and a reader
    /// that opened empty and filled in later would be read as an empty commit.
    pub lines: Option<Result<Vec<String>, String>>,
    pub scroll: usize,
}

/// What comes back from a thread doing something with git.
enum GitMessage {
    /// The panel's state, and which ask it is the answer to.
    Snapshot(u64, Box<crate::git::Snapshot>),
    /// The outcome of a command that wrote — git's own words, either way.
    Wrote(Result<String, String>),
    /// One commit in full. Carries the hash it was asked about so a slow answer cannot land in a
    /// reader that has since been opened on a different commit.
    Detail(String, Result<Vec<String>, String>),
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

/// Which workspace, if any, survives a change of project folder. A saved one does not: its file
/// describes the project being left, and staying attached to it meant the next exit wrote the new
/// folder's files and shells over it. The built-in layout does travel — it belongs to no project
/// and is never written to disk, so carrying it along costs nothing and keeps the badge honest.
pub fn workspace_after_root_change(active: Option<&str>) -> Option<String> {
    match active {
        Some(name) if crate::workspace::is_built_in(name) => Some(name.to_string()),
        _ => None,
    }
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
    let quoted = shell_quote(&path.to_string_lossy());
    if rest.is_empty() { quoted } else { format!("{quoted} {rest}") }
}

/// Quotes a path for the shell the command is about to be typed at.
///
/// POSIX quoting is single quotes with backslash escapes, and on Windows both halves are wrong:
/// cmd.exe has no single quotes at all, and a Windows path is mostly backslashes. Every run
/// command there came out as `'C:\Users\me\octave-cli.exe' script.m`, which cmd looks for
/// verbatim and never finds — the interpreter was resolved correctly and then handed over
/// unusable.
fn shell_quote(text: &str) -> String {
    if cfg!(windows) {
        quote_for_cmd(text)
    } else {
        shell_words::quote(text).into_owned()
    }
}

/// Double quotes are what cmd.exe understands, and inside them a backslash is just a backslash
/// and the operators lose their meaning. A path with nothing to protect is left bare, which is
/// what keeps a command line readable when it is echoed into a pane. `"` cannot appear in a
/// Windows path at all, so there is nothing to escape inside the quotes.
fn quote_for_cmd(text: &str) -> String {
    if text.is_empty() {
        return "\"\"".to_string();
    }
    if text.contains([' ', '\t', '&', '|', '<', '>', '^', '(', ')', ',', ';', '=']) {
        return format!("\"{text}\"");
    }
    text.to_string()
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
    let quote = |s: std::borrow::Cow<'_, str>| shell_quote(&s);
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
pub fn collect_project_files(root: &std::path::Path, out: &mut Vec<PathBuf>, show_hidden: bool) {
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

/// Below this a figure joins the tab strip of the pane it is in rather than splitting the
/// editor: two panes of thirty columns each are two panes nobody can read.
const SPLIT_FOR_FIGURES_COLS: u16 = 120;

/// One variable, being looked at a screenful at a time.
///
/// The values are not in the snapshot — a large matrix is millions of numbers — so each
/// screenful is asked for and answered through a file. Paging types a new question at the
/// prompt; the answer arrives the way everything else from a session does.
pub struct Inspector {
    pub name: String,
    /// Zero-based corner of what is being looked at. The interpreter clamps it, and what comes
    /// back says where it actually landed.
    pub row: usize,
    pub col: usize,
    pub watch: crate::wsnap::SliceWatch,
    /// Set while an answer is expected, so the panel can say it is waiting rather than showing
    /// the previous variable's numbers under the new one's name.
    pub asked: bool,
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

/// How much of a variable is asked for at a time. A screenful and a little, so paging by a
/// screen never leaves a gap, and small enough that the question is cheap to answer.
const INSPECT_ROWS: usize = 40;
const INSPECT_COLS: usize = 24;

/// The slice file that belongs beside a pane's snapshot: `ws-3.json` and `slice-3.json` are the
/// same session's two channels.
fn break_path_beside(snapshot: &Path) -> PathBuf {
    let name = snapshot.file_name().and_then(|n| n.to_str()).unwrap_or("ws-0.json");
    let suffix = name.strip_prefix("ws-").unwrap_or("0.json");
    snapshot.with_file_name(format!("break-{suffix}"))
}

fn request_path_beside(slice: &Path) -> PathBuf {
    let name = slice.file_name().and_then(|n| n.to_str()).unwrap_or("slice-0.json");
    let suffix = name.strip_prefix("slice-").unwrap_or("0.json");
    slice.with_file_name(format!("slicereq-{suffix}"))
}

fn slice_path_beside(snapshot: &Path) -> PathBuf {
    let name = snapshot.file_name().and_then(|n| n.to_str()).unwrap_or("ws-0.json");
    let suffix = name.strip_prefix("ws-").unwrap_or("0.json");
    snapshot.with_file_name(format!("slice-{suffix}"))
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
        // Read before `settings` is moved into the struct below, where the field that holds it
        // is initialised ahead of this one.
        let show_splash = settings.show_splash;
        crate::terminal_panel::set_scrollback_len(settings.terminal_scrollback);
        crate::wsnap::set_plots_in_tabs(settings.plots_in_tabs);
        // Two windows side by side to start, each with a single tab — the familiar two-pane view.
        let t1 = TerminalWindow::new(term_rows, half_cols, &root)?;
        let t2 = TerminalWindow::new(term_rows, half_cols, &root)?;
        let (bg_tx, bg_rx) = mpsc::channel();
        let (preview_tx, preview_rx) = mpsc::channel();
        let (git_status_tx, git_status_rx) = mpsc::channel();
        let (search_tx, search_rx) = mpsc::channel();
        let (git_panel_tx, git_panel_rx) = mpsc::channel();
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
            show_splash,
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
            completion: None,
            completion_anchor: (0, 0),
            startup_cols: term_cols,
            lsp: std::collections::HashMap::new(),
            lsp_error: std::collections::HashMap::new(),
            diagnostics: std::collections::HashMap::new(),
            lsp_sent: std::collections::HashMap::new(),
            lsp_seen: std::collections::HashMap::new(),
            lsp_paths: std::collections::HashMap::new(),
            lsp_completion: None,
            lsp_asked: None,
            lsp_hovered: None,
            lsp_what_it_is: None,
            jumps: Vec::new(),
            figures: None,
            figure_drawn: std::collections::HashMap::new(),
            run_figures: std::collections::HashMap::new(),
            run_watch: None,
            inspector: None,
            breakpoints: std::collections::HashMap::new(),
            stopped_at: None,
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
            last_terminal_click: None,
            git_status: std::collections::HashMap::new(),
            git_status_tx,
            git_status_rx,
            git_status_pending,
            bg_tx,
            bg_rx,
            preview_tx,
            preview_rx,
            show_search: false,
            search_input: String::new(),
            search_regex: false,
            search_case_sensitive: false,
            search_tx,
            search_rx,
            search_pending: Arc::new(AtomicBool::new(false)),
            scratch: Editor::empty(),
            git_panel: None,
            git_panel_tx,
            git_panel_rx,
            git_asked: 0,
            git_wanted: None,
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
                // Put back only something worth putting back. Restoring an *empty* line was
                // indistinguishable from the joke having eaten it: the bar simply went blank and
                // stayed blank, which is the one outcome a joke on the status line must not have.
                if self.status_message == tagline && !displaced.trim().is_empty() {
                    self.status_message = displaced;
                }
            }
            self.turtle = None;
        }
    }

    /// What the menu items that hold a setting read out beside themselves.
    ///
    /// The plot destination is the *effective* one and not the stored preference: a machine with
    /// no screen captures whatever the setting says, so reading the setting out would have the
    /// menu claim "windows" while every figure kept arriving as a tab.
    pub fn menu_states(&self) -> crate::menu::MenuStates {
        crate::menu::MenuStates {
            plots_in_tabs: self.settings.plots_in_tabs || !crate::wsnap::can_open_a_window(),
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
            // Cleared before the page check below, not after: a reply that is thrown away is
            // still the end of that read, and leaving the mark set would stop the tab ever
            // asking for another one.
            preview.reloading = false;
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
                Ok(image) => {
                    let (cols, rows) = (preview.area_cols, preview.area_rows);
                    // A picture asked for before the pane had ever been drawn comes back at its
                    // own size, since there was no box to scale it into then. There is one now,
                    // so it is fitted here rather than left at whatever a camera produced — a
                    // 4000-pixel photograph would otherwise show as a pane-sized crop of itself.
                    let image = match preview.kind() {
                        crate::preview::Kind::Picture => {
                            crate::preview::scale_picture(image, preview.picture_box(), preview.fit)
                        }
                        _ => image,
                    };
                    // The whole page is kept and only the window being looked at is handed to
                    // the terminal. That is what makes zoom and scrolling cost a crop instead
                    // of a rasteriser — and what makes zoom visible at all, since the widget
                    // would otherwise shrink a larger page straight back to the pane.
                    let window = crate::preview::visible_window(
                        &image,
                        cols,
                        rows,
                        preview.scroll_x,
                        preview.scroll_px,
                    );
                    preview.full = Some(image);
                    preview.show(window);
                }
                // A document that could not be made is not the end of a markdown preview: it
                // still has styled text to offer, which is better than a red line where the
                // document should be. The reason is said once, in the status line, and the tab
                // stops trying until Refresh asks it to.
                Err(message) if rendered_view.is_some() => {
                    preview.document_failed = true;
                    preview.shown_revision = u64::MAX;
                    self.status_message = i18n::msg_preview_failed(self.settings.lang, &message);
                }
                // A picture that could not be read is not a reason to take down the one the
                // tab is already showing. It is nearly always the newest frame of something
                // being animated, caught mid-write; the tab that answered it with a red line
                // was reporting a file that reads perfectly a millisecond later. What is up
                // stays up, the reason is said once in the status line, and the next frame
                // settles it. With nothing up there is nothing to keep, and the tab says so.
                Err(message) => match preview.state {
                    crate::preview::State::Ready(_) => {
                        self.status_message = i18n::msg_preview_failed(self.settings.lang, &message)
                    }
                    _ => preview.state = crate::preview::State::Failed(message),
                },
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
        let lang = self.settings.lang;
        if self.show_splash {
            self.show_splash = false;
            return;
        }
        if self.show_about {
            return;
        }
        // A paste is keys, and while a box is up the keys are its. Into the one kind of box that
        // takes text it arrives as text; over any other it does nothing at all — which is the
        // point, because doing nothing is what a box that has no use for it should do, and
        // falling through to the editor underneath is what it used to do instead.
        if self.a_modal_owns_the_keyboard() {
            self.paste_into_a_modal(&text);
            return;
        }
        match self.focus {
            Focus::FileTree => {
                let paths = dnd::parse_dropped_paths(&text);
                if !paths.is_empty() {
                    self.copy_dropped_paths(paths);
                } else if dnd::looks_like_dropped_paths(&text) {
                    // Dropped files that are not on this machine. Copying them is not something
                    // this side can do — the file is on the other end of the connection, and
                    // nothing here can read it — so it says so instead of doing nothing, which
                    // is what it used to do.
                    self.status_message = i18n::msg_drop_not_here(lang, dnd::running_over_ssh());
                }
            }
            Focus::Editor => self.editor_mut().insert_multiline(&text),
            Focus::Terminal => self.handle_terminal_paste(&text),
        }
    }

    /// A paste while a box is up.
    ///
    /// Only the boxes that take typing take it, and today that is the git panel's. The rest
    /// ignore it rather than passing it on: a paste over a question that wants one letter is not
    /// an answer to it, and a paste over a list is not anything.
    ///
    /// Newlines become spaces because every one of these boxes is a single line. A pasted commit
    /// message with a blank line in it would otherwise arrive as a message with two invisible
    /// characters in the middle of it.
    fn paste_into_a_modal(&mut self, text: &str) {
        let Some(GitPrompt::Text { typed, .. }) =
            self.git_panel.as_mut().and_then(|p| p.prompt.as_mut())
        else {
            return;
        };
        typed.push_str(&text.replace(['\n', '\r'], " "));
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

    /// What a built-in workspace needs to know about this machine: where the project is, how
    /// wide the window is, and which Python a selected virtualenv means.
    ///
    /// `last_full` is the window as it was drawn last frame and is zero before the first one,
    /// which is exactly when a workspace named on the command line is applied — so the terminal's
    /// own size stands in until there is a frame to measure.
    pub fn workspace_shape(&self) -> crate::workspace::Shape {
        crate::workspace::Shape {
            root: self.root.clone(),
            cols: if self.last_full.width > 0 { self.last_full.width } else { self.startup_cols },
            python: self.apply_venv("python3"),
            // `current_exe` rather than the word "clee": the preset must open *this* CleeCode's
            // viewer, not whichever one is on the PATH — which during development is usually an
            // older one installed by Homebrew.
            workspace_view: std::env::current_exe().ok().map(|exe| {
                format!(
                    "{} --watch-workspace {}",
                    crate::session::Language::Octave.quote(&exe.to_string_lossy()),
                    crate::session::Language::Octave
                        .quote(&crate::wsnap::snapshot_dir().to_string_lossy())
                )
            }),
        }
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
        self.editors.get(self.active_editor_index()).unwrap_or(&self.scratch)
    }

    pub fn editor_mut(&mut self) -> &mut Editor {
        let idx = self.active_editor_index();
        // `get_mut` rather than an index, because with every tab closed there is no buffer to
        // index. See `scratch`: the answer to "which file am I editing" can be "none", and that
        // is a state to draw rather than one to avoid by keeping a file open you closed.
        self.editors.get_mut(idx).unwrap_or(&mut self.scratch)
    }

    /// Puts the window into the state with no file in it: no buffers, no tabs, no split, and
    /// the keyboard somewhere that can use it.
    ///
    /// One place rather than two, because the second way to get here is easy to miss: closing a
    /// file also closes the preview that was a view of it, so the last *two* tabs can go on one
    /// keystroke.
    fn nothing_open(&mut self) {
        self.editors.clear();
        self.tabs = [Vec::new(), Vec::new()];
        self.active_editor = 0;
        self.active_editor_right = 0;
        // A split of two empty halves is two of the same nothing. One frame says it once.
        self.split_view = false;
        // And the keyboard leaves with the file: an editor with no buffer that still held the
        // focus would swallow every keystroke into a buffer nobody can see.
        if self.focus == Focus::Editor {
            self.focus =
                if self.settings.show_sidebar { Focus::FileTree } else { Focus::Terminal };
        }
    }

    /// Whether anything is open at all. The window with no file in it is a real state — the one
    /// you get by closing your last tab — and several things have to be drawn differently in it.
    pub fn any_tabs_open(&self) -> bool {
        self.tabs.iter().any(|strip| !strip.is_empty())
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

    // ---- Project search -------------------------------------------------------------------

    /// Asks what to look for across the project, starting from whatever you were already
    /// looking for: the selection, or the last thing typed into the Find box. Searching for the
    /// word under the cursor is the common case and this is the cheapest way to serve it.
    fn begin_project_search(&mut self) {
        self.search_input = self
            .editor()
            .selected_text()
            .filter(|s| !s.contains('\n') && !s.trim().is_empty())
            .or_else(|| self.find.as_ref().map(|f| f.query.clone()).filter(|q| !q.is_empty()))
            .unwrap_or_default();
        self.show_search = true;
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                self.show_search = false;
                self.search_input.clear();
            }
            KeyCode::Enter => self.start_project_search(),
            // The same two switches as the Find box, by the same two keys: one idea, one pair
            // of keys, wherever a query is typed.
            KeyCode::Char('u') if ctrl => self.search_case_sensitive = !self.search_case_sensitive,
            KeyCode::Char('n') if ctrl => self.search_regex = !self.search_regex,
            KeyCode::Backspace => {
                self.search_input.pop();
            }
            KeyCode::Char(c) if !ctrl => self.search_input.push(c),
            _ => {}
        }
    }

    fn start_project_search(&mut self) {
        let query = self.search_input.trim().to_string();
        if query.is_empty() {
            return;
        }
        self.show_search = false;
        let lang = self.settings.lang;
        self.status_message = i18n::msg_search_running(lang, &query);
        crate::search::spawn(
            self.root.clone(),
            query,
            self.search_regex,
            self.search_case_sensitive,
            self.settings.show_hidden_files,
            self.search_tx.clone(),
            self.search_pending.clone(),
        );
    }

    // ---- Breakpoints, and being stopped -----------------------------------------------------

    /// Puts a breakpoint on the cursor's line, or takes it off.
    ///
    /// Written to a file for the session's hook to apply, never typed at the prompt: `dbstop`
    /// works through `evalin` from inside the hook — measured — so setting a breakpoint leaves
    /// no line in the transcript that the user did not write.
    fn toggle_breakpoint(&mut self) {
        let lang = self.settings.lang;
        let editor = self.editor();
        let Some(path) = editor.path.clone() else {
            self.status_message = i18n::msg_break_unsaved(lang);
            return;
        };
        if crate::session::Language::of_path(&path).is_none() {
            self.status_message = i18n::msg_break_no_language(lang, &file_ext(&path));
            return;
        }
        let line = editor.cursor_line + 1;
        let lines = self.breakpoints.entry(path.clone()).or_default();
        let on = if lines.remove(&line) {
            false
        } else {
            lines.insert(line);
            true
        };
        if lines.is_empty() {
            self.breakpoints.remove(&path);
        }
        self.publish_breakpoints();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.status_message = i18n::msg_breakpoint(lang, on, &name, line);
    }

    /// Leaves the whole set where the hook will find it. The whole set rather than a change,
    /// because the hook clears and re-applies: a session that missed one message would otherwise
    /// disagree with the editor about where the breakpoints are, silently and forever.
    fn publish_breakpoints(&mut self) {
        let Some(watch) = self.figures.as_ref() else { return };
        let path = break_path_beside(&watch.path);
        // By function name, which is what `dbstop` takes and what a `.m` file is known by, and
        // by path, which is what pdb takes. Each language reads the field it can use.
        let wanted: Vec<serde_json::Value> = self
            .breakpoints
            .iter()
            .flat_map(|(file, lines)| {
                let name = file
                    .file_stem()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let full = file.to_string_lossy().into_owned();
                lines.iter().map(move |line| {
                    serde_json::json!({"name": name, "path": full, "line": line})
                })
            })
            .collect();
        let temp = path.with_extension("tmp");
        let _ = std::fs::write(&temp, serde_json::Value::Array(wanted).to_string())
            .and_then(|_| std::fs::rename(&temp, &path));
    }

    /// Follows the session into the file it stopped in.
    ///
    /// Called when a snapshot says the stop moved. Opens the file if it is not already open and
    /// puts the cursor on the line — the same thing a double-clicked traceback does, and for the
    /// same reason: the place to be is where the program is.
    fn follow_stop(&mut self, debug: &crate::wsnap::Debug) {
        let lang = self.settings.lang;
        if !debug.stopped {
            if self.stopped_at.take().is_some() {
                self.status_message = i18n::msg_debug_running(lang);
            }
            return;
        }
        let path = PathBuf::from(&debug.file);
        let path = if path.exists() { path } else { self.root.join(&debug.file) };
        let here = (path.clone(), debug.line);
        if self.stopped_at.as_ref() == Some(&here) {
            return;
        }
        self.stopped_at = Some(here);
        if path.exists() {
            // Shown, not focused. You are at the prompt with `dbstep` half typed when this
            // fires, and taking the keyboard to point at a line would put the next word you
            // type into the file you are debugging. Everything about where the keyboard was is
            // put back — the same rule a figure follows when it opens.
            let was = (self.focus, self.editor_pane_focus);
            self.open_file_at(path, debug.line.saturating_sub(1), 0);
            self.focus = was.0;
            self.editor_pane_focus = was.1;
        }
        // Named by the snapshot's own `lang`, the same field the workspace window is titled
        // from — so the words offered are the ones that work at the prompt in front of you.
        let python = self
            .figures
            .as_ref()
            .and_then(|watch| watch.snapshot.as_ref())
            .is_some_and(|snapshot| snapshot.lang == "python");
        let steps = if python { "n / c" } else { "dbstep / dbcont" };
        self.status_message = i18n::msg_debug_stopped(lang, &debug.name, debug.line, steps);
    }

    /// The breakpoints on a file, for the renderer.
    pub fn breakpoints_in(&self, path: Option<&Path>) -> Option<&std::collections::BTreeSet<usize>> {
        self.breakpoints.get(path?)
    }

    // ---- Looking inside a variable ----------------------------------------------------------

    /// Offers the session's variables, and opens the one picked.
    ///
    /// A picker rather than a key on the workspace window, because that window is a separate
    /// program with no keyboard of its own — and because the names are already known here.
    fn open_inspector_picker(&mut self) {
        let lang = self.settings.lang;
        let names = self.session_names();
        if names.is_empty() {
            self.status_message = i18n::msg_inspect_no_session(lang);
            return;
        }
        let items = names
            .into_iter()
            .map(|name| crate::picker::PickItem {
                label: name.clone(),
                shortcut: None,
                action: crate::picker::PickAction::Inspect(name),
            })
            .collect();
        self.picker = Some(crate::picker::Picker::new(
            "Variables",
            crate::picker::PickerKind::Variables,
            items,
        ));
    }

    /// Opens the inspector on `name` and asks the session for the first screenful.
    pub fn inspect(&mut self, name: String) {
        let path = self
            .figures
            .as_ref()
            .map(|w| slice_path_beside(&w.path))
            .unwrap_or_else(|| crate::wsnap::snapshot_dir().join("slice-0.json"));
        self.inspector = Some(Inspector {
            name,
            row: 0,
            col: 0,
            watch: crate::wsnap::SliceWatch::new(path),
            asked: false,
        });
        self.ask_for_slice();
    }

    /// Leaves the question where the session's own hook will find it.
    ///
    /// Written to a file rather than typed at the prompt, and that is not a detail. Typing there
    /// puts a line in the user's transcript that they did not write, fights whatever they are
    /// half-way through, and only works if the line editor happens to be listening — which, for
    /// this particular command, it reliably was not: byte-identical writes to the same terminal
    /// were acted on the second time and ignored the first. The hook already runs at every idle
    /// moment and already reads and writes files. Asking it is quieter and cannot miss.
    fn ask_for_slice(&mut self) {
        let Some(inspector) = self.inspector.as_ref() else { return };
        let request = serde_json::json!({
            "name": inspector.name,
            "r0": inspector.row + 1,
            "r1": inspector.row + INSPECT_ROWS,
            "c0": inspector.col + 1,
            "c1": inspector.col + INSPECT_COLS,
        });
        let path = request_path_beside(&inspector.watch.path);
        // Written beside and renamed, like everything else on this channel, so the reader on the
        // other side never sees half a question.
        let temp = path.with_extension("tmp");
        let written = std::fs::write(&temp, request.to_string()).and_then(|_| std::fs::rename(&temp, &path));
        if written.is_err() {
            self.status_message = i18n::msg_inspect_no_session(self.settings.lang);
            return;
        }
        if let Some(inspector) = self.inspector.as_mut() {
            inspector.asked = true;
        }
    }

    pub fn poll_inspector(&mut self) {
        if let Some(inspector) = self.inspector.as_mut()
            && inspector.watch.poll()
        {
            inspector.asked = false;
        }
    }

    fn handle_inspector_key(&mut self, key: KeyEvent) {
        let Some(inspector) = self.inspector.as_ref() else { return };
        let (rows, cols) = inspector
            .watch
            .slice
            .as_ref()
            .map(|s| (s.rows, s.cols))
            .unwrap_or((0, 0));
        let (mut row, mut col) = (inspector.row, inspector.col);
        match key.code {
            KeyCode::Esc => {
                self.inspector = None;
                return;
            }
            _ if is_ctrl_shift(key, 'i') => {
                self.inspector = None;
                return;
            }
            KeyCode::Down => row = (row + INSPECT_ROWS).min(rows.saturating_sub(1)),
            KeyCode::Up => row = row.saturating_sub(INSPECT_ROWS),
            KeyCode::Right => col = (col + INSPECT_COLS).min(cols.saturating_sub(1)),
            KeyCode::Left => col = col.saturating_sub(INSPECT_COLS),
            KeyCode::Home => {
                row = 0;
                col = 0;
            }
            // Asking again is the only action a read-only panel needs: the answer went stale the
            // moment the session ran anything.
            KeyCode::Char('r') | KeyCode::Char('R') => {}
            _ => return,
        }
        if let Some(inspector) = self.inspector.as_mut() {
            inspector.row = row;
            inspector.col = col;
        }
        self.ask_for_slice();
    }

    // ---- Going where the output points -----------------------------------------------------

    /// Whether this click is the second one on the same terminal row, quickly enough to be a
    /// double-click. Tracked here rather than asked of the terminal, which has no idea it is
    /// being clicked twice.
    fn second_click_on(&mut self, pane: usize, row: u16) -> bool {
        let now = Instant::now();
        let again = self
            .last_terminal_click
            .map(|(was_pane, was_row, when)| {
                was_pane == pane && was_row == row && now.duration_since(when) < DOUBLE_CLICK_THRESHOLD
            })
            .unwrap_or(false);
        self.last_terminal_click = Some((pane, row, now));
        again
    }

    /// Opens whatever file a terminal row is pointing at. `true` when it found one.
    ///
    /// Works on anything that prints `path:line:column`, which is every compiler, linter and
    /// grep — not only on the tracebacks it was written for.
    fn open_location_at(&mut self, pane: usize, row: u16) -> bool {
        let Some(text) = self.window_tab_mut(pane).and_then(|t| t.row_text(row)) else {
            return false;
        };
        let lang = self.settings.lang;
        let Some(location) = crate::locate::find(&text) else { return false };
        match crate::locate::resolve(&location, &self.root) {
            Some(path) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.open_file_at(path, location.line.saturating_sub(1), location.column.saturating_sub(1));
                self.status_message = i18n::msg_jumped_to(lang, &name, location.line);
                true
            }
            // It named something, and the something is not here. Saying so beats a double-click
            // that silently does nothing, and beats opening a file of that name from elsewhere.
            None => {
                self.status_message = i18n::msg_jump_not_found(lang, &location.path);
                true
            }
        }
    }

    // ---- Figures from a live session ------------------------------------------------------

    /// Opens a tab for each figure the session has drawn, and re-reads one whose picture has
    /// changed underneath it.
    ///
    /// A figure arrives as a picture because a live Qt window cannot be reparented into a
    /// terminal — but a raster hand-off can, and CleeCode already draws PNGs. The interpreter is
    /// told not to open a window at all, so nothing appears behind the terminal.
    ///
    /// The tab is opened but **not focused**: `plot` in the middle of a script should not take
    /// the keyboard away from what you were writing. It appears in the strip, which is where you
    /// look for it.
    pub fn poll_figures(&mut self) {
        if !self.settings.plots_in_tabs {
            return;
        }
        let Some(path) = crate::wsnap::newest_in(&crate::wsnap::snapshot_dir()) else { return };
        if self.figures.as_ref().map(|w| w.path != path).unwrap_or(true) {
            self.figures = Some(crate::wsnap::Watch::new(path));
        }
        let Some(watch) = self.figures.as_mut() else { return };
        // A new snapshot is news about *which* figures the session holds, and about where a
        // debugger stopped. What a figure looks like is a different question, and it is asked
        // below whether or not the snapshot moved — because during a loop it does not.
        //
        // The snapshot is written when the interpreter is between commands: Octave's hook runs
        // from `add_input_event_hook`, which fires while it waits for input, and a loop never
        // waits. `cleecode_frame` exists for exactly that case and it reprints the figures — it
        // does not, and should not, rebuild the whole snapshot, which means walking every
        // variable in the session sixty times a second. Gated on the snapshot, an animation in
        // Octave therefore moved only when the loop happened to let the hook in, which is what
        // "it jumps a frame now and then" was. The picture's own timestamp is what says a
        // figure was redrawn, and it costs one stat per open figure per tick to ask.
        let fresh = watch.poll();
        let (debug, figures): (crate::wsnap::Debug, Vec<PathBuf>) = watch
            .snapshot
            .as_ref()
            .map(|s| {
                (s.debug.clone(), s.figures.iter().map(|f| PathBuf::from(&f.path)).collect())
            })
            .unwrap_or_default();
        if fresh {
            self.follow_stop(&debug);
        }
        for path in figures {
            // A snapshot lists every figure the session is holding, not the one that just moved.
            // Followed literally that reopens all of them on every tick: plot into figure 3 and
            // figure 1's tab — closed a minute ago, because you were done with it — comes back
            // with it. Closing a tab is an instruction, and a session that still happens to hold
            // the figure is not a reason to overrule it.
            //
            // The picture's own timestamp is what says which figure was redrawn. It is on disk
            // already, it costs a stat, and it needs nothing added to the snapshot contract —
            // whose `figures` list means "these exist", which is a different question.
            let Ok(drawn) = std::fs::metadata(&path).and_then(|m| m.modified()) else { continue };
            if !redrawn(&mut self.figure_drawn, &path, drawn) {
                continue;
            }
            match self.editors.iter().position(|e| e.path.as_deref() == Some(path.as_path())) {
                // Already a tab: the picture on disk is new, and nothing about a decoded image
                // knows its file moved on.
                //
                // Unless the previous frame is still being decoded. An animation can write
                // faster than a decode returns, and starting a thread per write piles up
                // threads that each produce a picture already out of date by the time it
                // arrives. Marked as drawn only once the read has begun, so the frame skipped
                // here is picked up on the next tick — the newest one on disk, which is the
                // only one worth having.
                Some(idx) => {
                    if self.editors[idx].preview.as_ref().is_some_and(|p| p.reading()) {
                        // Put back what `redrawn` just recorded: this frame has not been shown,
                        // and the next tick must be free to see it again.
                        self.figure_drawn.remove(&path);
                        continue;
                    }
                    self.reread_preview(idx, None);
                }
                None => self.open_figure_tab(path),
            }
        }
    }

    /// Watches a file handed to a live session until its prompt comes back, remembering which
    /// figures it opened along the way. See [`RunWatch`] for why the watch closes there and not
    /// at the next run.
    pub fn poll_run_watch(&mut self) {
        let Some((language, terminal, looked)) =
            self.run_watch.as_ref().map(|w| (w.language, w.terminal, w.looked))
        else {
            return;
        };
        // Every tick once the prompt is back, throttled while the script is still running.
        let settling = self.run_watch.as_ref().is_some_and(|w| w.settled.is_some());
        let open = (settling || looked.elapsed() >= RUN_WATCH_INTERVAL)
            .then(|| crate::wsnap::open_figures(&crate::wsnap::snapshot_dir(), language.snapshot_lang()));
        // A pane that is gone takes its run with it: there is nothing left to be at a prompt,
        // and nothing left to close figures in either.
        let at_prompt =
            self.terminals.get(terminal).map(|w| w.active_tab().is_at_prompt()).unwrap_or(true);
        let Some(watch) = self.run_watch.as_mut() else { return };
        if let Some(open) = open {
            watch.looked = std::time::Instant::now();
            for number in open {
                if !watch.before.contains(&number) && !watch.opened.contains(&number) {
                    watch.opened.push(number);
                }
            }
        }
        if !at_prompt {
            watch.busy_seen = true;
            watch.settled = None;
            return;
        }
        // Still at the prompt because the command has not started yet, rather than because it
        // has finished. A script quick enough never to be caught running is over by the time
        // the wait runs out, and its figures are in the snapshot by then too.
        if !watch.busy_seen && watch.started.elapsed() < RUN_WATCH_MAX {
            return;
        }
        if watch.settled.get_or_insert_with(std::time::Instant::now).elapsed() < RUN_SETTLE {
            return;
        }
        let Some(watch) = self.run_watch.take() else { return };
        if !watch.opened.is_empty() {
            self.run_figures.insert(watch.file, watch.opened);
        }
    }

    /// The figure a preview tab is showing, and the session that drew it.
    ///
    /// A figure tab is an ordinary picture tab in every way but this: it has an interpreter
    /// behind it that can be asked to draw it again differently. That is what the keys below
    /// need to know, and it is the only thing that distinguishes the two.
    fn figure_for(&self, path: Option<&Path>) -> Option<(crate::wsnap::Figure, crate::session::Language)> {
        let path = path?.to_string_lossy().into_owned();
        // The session the panel is showing first, because it is already in memory and it is the
        // right answer nearly always.
        let held = self.figures.as_ref().and_then(|w| w.snapshot.as_ref()).and_then(|snapshot| {
            snapshot
                .figures
                .iter()
                .find(|f| f.path == path)
                .cloned()
                .map(|f| (f, snapshot.lang.clone()))
        });
        // And then every other session's, off disk. Two prompts write two snapshots and the
        // panel follows whichever ticked last — so a figure drawn by the other one belonged to
        // no session as far as this was concerned, and its keys fell through to the picture and
        // scrolled it. Which is indistinguishable, from the outside, from controls that do not
        // exist. A handful of small files, read only when a key is pressed on a figure tab.
        let (figure, lang) = held
            .or_else(|| {
                crate::wsnap::figure_owner(&crate::wsnap::snapshot_dir(), &path)
                    .map(|(figure, snapshot)| (figure, snapshot.lang))
            })?;
        let language = match lang.as_str() {
            "python" => crate::session::Language::Python,
            _ => crate::session::Language::Octave,
        };
        Some((figure, language))
    }

    /// What the arrow keys do on the figure in this tab, or `None` when it is not one.
    ///
    /// A 3-D axes turns and a 2-D one slides, and the same four keys do both — so the bar has to
    /// say which, or the hint is only right half the time.
    pub fn figure_nav_hint(&self, idx: usize) -> Option<String> {
        let path = self.editors.get(idx).and_then(|e| e.path.clone());
        let (figure, _) = self.figure_for(path.as_deref())?;
        let is3d = figure.axes.first().map(|a| a.is3d).unwrap_or(false);
        Some(i18n::msg_figure_keys(self.settings.lang, is3d).to_string())
    }

    /// A click on one of the figure bar's buttons, run through the same code the key runs.
    ///
    /// One path, so a button and its key cannot come to mean different things — which is the
    /// same reason `git_tab_slots` is one function and not two.
    fn figure_nav_click(&mut self, code: KeyCode) {
        self.figure_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    /// A key pressed on a figure tab. `true` when it was one of the ones that moves the plot.
    ///
    /// The move is sent to the interpreter, which redraws and writes a new picture; the tab
    /// picks that up the way it picks up any change to its file. Nothing is done to the pixels
    /// on screen — magnifying those would leave the axis labels describing a range that is no
    /// longer shown, and a plot whose numbers are wrong is worse than one that is small.
    fn figure_key(&mut self, key: KeyEvent) -> bool {
        use crate::session::Nav;
        if !key.modifiers.is_empty() {
            return false;
        }
        let idx = self.active_editor_index();
        let path = self.editors.get(idx).and_then(|e| e.path.clone());
        let Some((figure, language)) = self.figure_for(path.as_deref()) else { return false };
        let axes = figure.axes.first();
        let is3d = axes.map(|a| a.is3d).unwrap_or(false);
        let view = axes
            .map(|a| (a.view.first().copied().unwrap_or(0.0), a.view.get(1).copied().unwrap_or(90.0)))
            .unwrap_or((0.0, 90.0));
        let nav = match key.code {
            KeyCode::Char('+') | KeyCode::Char('=') => Nav::In,
            KeyCode::Char('-') | KeyCode::Char('_') => Nav::Out,
            KeyCode::Left => Nav::Left,
            KeyCode::Right => Nav::Right,
            KeyCode::Up => Nav::Up,
            KeyCode::Down => Nav::Down,
            // Not `f`, which fits a page: there is no page here, and "back to how it was drawn"
            // is the thing a plot actually wants.
            KeyCode::Char('r') | KeyCode::Char('0') => Nav::Reset,
            // Out of the editor and into a document. Handled here rather than as a nav, because
            // it is the one figure key that produces something rather than changing something.
            KeyCode::Char('e') => {
                self.export_figure(&figure, language);
                return true;
            }
            _ => return false,
        };
        let command = language.nav_command(nav, figure.fig, is3d, view);
        let lang = self.settings.lang;
        self.status_message = match self.send_to_session(language, &command) {
            Some(_) => i18n::msg_figure_nav(lang, nav, is3d),
            // The session that drew it is gone. Saying so beats a key that does nothing, and the
            // picture is still a picture — it is simply the last one that session made.
            None => i18n::msg_figure_no_session(lang, language.label()),
        };
        true
    }

    /// Writes a figure out as a PDF beside the project, and says where it went.
    ///
    /// Asked of the session rather than converted from the PNG on screen: the interpreter still
    /// has the figure, so it can draw it at any size, and a PDF made from a bitmap would be a
    /// bitmap in a wrapper.
    fn export_figure(&mut self, figure: &crate::wsnap::Figure, language: crate::session::Language) {
        let lang = self.settings.lang;
        let file = self.root.join(format!("fig{}.pdf", figure.fig));
        let command = language.export_command(figure.fig, &file.to_string_lossy());
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.status_message = match self.send_to_session(language, &command) {
            Some(_) => i18n::msg_figure_exported(lang, &name),
            None => i18n::msg_figure_no_session(lang, language.label()),
        };
    }

    /// Adds a figure as a preview tab in the pane that is not being typed in, without taking the
    /// focus. Split view is opened for it when there is room: a plot beside the script that drew
    /// it is the arrangement the whole feature is for.
    fn open_figure_tab(&mut self, path: PathBuf) {
        if !path.exists() {
            return;
        }
        let was = (self.focus, self.editor_pane_focus, self.active_editor, self.active_editor_right);
        if !self.split_view && self.last_full.width >= SPLIT_FOR_FIGURES_COLS {
            self.toggle_split_view();
        }
        if self.split_view {
            self.editor_pane_focus = match was.1 {
                EditorPane::Left => EditorPane::Right,
                EditorPane::Right => EditorPane::Left,
            };
        }
        self.open_preview_tab(path, false);
        // Everything about where the keyboard was, put back. Opening a tab is the whole of what
        // a figure is allowed to do to a window somebody is working in.
        self.focus = was.0;
        self.editor_pane_focus = was.1;
        if self.split_view {
            match was.1 {
                EditorPane::Left => self.active_editor = was.2,
                EditorPane::Right => self.active_editor_right = was.3,
            }
        }
    }

    // ---- Language server -----------------------------------------------------------------

    /// One call per frame, doing the three things the server needs: exist, be told what is open,
    /// and be listened to.
    ///
    /// Everything here is a *check* rather than a notification from elsewhere in the app. A file
    /// can be opened from the tree, the quick-open, the command line, a workspace, a search
    /// result or a drop — six places that would each have to remember to tell the server, and
    /// the seventh, added later, would not.
    pub fn poll_lsp(&mut self) {
        self.lsp_start_if_wanted();
        self.lsp_take_events();
        self.lsp_sync_open_files();
        self.lsp_ask_what_this_is();
    }

    /// The command line for a file, and `None` for a language nothing here can serve.
    fn lsp_argv_for(&self, path: &Path) -> Option<Vec<String>> {
        if !self.settings.language_server {
            return None;
        }
        crate::lsp::server_for(path, &self.settings.language_servers)
    }

    /// The running server for a file.
    ///
    /// Keyed on the program name rather than on the language: `typescript-language-server` serves
    /// six extensions and `clangd` seven, and one process for each of those would be seven
    /// clangds indexing the same project at once.
    fn lsp_client_for(&mut self, path: &Path) -> Option<&mut crate::lsp::Client> {
        let program = self.lsp_argv_for(path)?.first()?.clone();
        self.lsp.get_mut(&program)
    }

    /// Starts a server the first time a file it knows about is open, and never twice.
    ///
    /// One process per program, started on demand: opening a Rust file in a project that also
    /// has Python in it should not start pyright, and a project with both open ends up with both
    /// — each told only about the files it serves.
    fn lsp_start_if_wanted(&mut self) {
        let wanted: Vec<Vec<String>> = self
            .editors
            .iter()
            .filter(|e| e.preview.is_none())
            .filter_map(|e| e.path.as_deref())
            .filter_map(|path| self.lsp_argv_for(path))
            .filter(|argv| {
                argv.first().is_some_and(|program| {
                    !self.lsp.contains_key(program) && !self.lsp_error.contains_key(program)
                })
            })
            .collect();
        for argv in wanted {
            let program = argv[0].clone();
            if self.lsp.contains_key(&program) || self.lsp_error.contains_key(&program) {
                continue;
            }
            let borrowed: Vec<&str> = argv.iter().map(String::as_str).collect();
            match crate::lsp::Client::start_with(&borrowed, &self.root) {
                Ok(client) => {
                    self.lsp.insert(program, client);
                }
                // Said once, in the status bar, and then remembered rather than retried. Not
                // having the server installed is the normal case, and CleeCode has to be exactly
                // as useful there as it was before any of this existed. Remembered *per program*,
                // so a machine with gopls and without clangd still gets Go.
                Err(detail) => {
                    self.status_message = i18n::msg_lsp_missing(self.settings.lang, &program);
                    self.lsp_error.insert(program, detail);
                }
            }
        }
    }

    fn lsp_take_events(&mut self) {
        let lang = self.settings.lang;
        // Drained first and handled after, because handling one needs `&mut self` — and which
        // server said it has to travel with it, since two of them are entitled to disagree about
        // how positions are counted.
        let mut arrived: Vec<(String, crate::lsp::Event)> = Vec::new();
        for (program, client) in self.lsp.iter() {
            while let Some(event) = client.try_recv() {
                arrived.push((program.clone(), event));
            }
        }
        for (program, event) in arrived {
            match event {
                crate::lsp::Event::Ready { utf16 } => {
                    let Some(client) = self.lsp.get_mut(&program) else { continue };
                    client.confirm_ready(utf16);
                    let name = client.name.clone();
                    self.status_message = i18n::msg_lsp_ready(lang, &name);
                }
                crate::lsp::Event::Diagnostics { path, raw } => {
                    // The server answers about the file it was told about, which is the resolved
                    // path — not the one the tab holds. The translation was recorded when it was
                    // announced; without it, a diagnostic for a project opened as `.` matches no
                    // tab and is silently dropped.
                    let Some(path) = self.lsp_paths.get(&path).cloned() else { continue };
                    let utf16 = self.lsp.get(&program).map(|c| c.utf16()).unwrap_or(true);
                    // Converted here because this is where the buffer is. A diagnostic for a file
                    // that is not open has nothing to be measured against, so it is dropped
                    // rather than stored against a guess at the text.
                    let Some(editor) =
                        self.editors.iter().find(|e| e.path.as_deref() == Some(path.as_path()))
                    else {
                        self.diagnostics.remove(&path);
                        continue;
                    };
                    let lines: Vec<String> = editor
                        .rope
                        .lines()
                        .map(|l| l.to_string().trim_end_matches('\n').to_string())
                        .collect();
                    let marks = crate::lsp::marks_from(&raw, &lines, utf16);
                    // An empty list is not nothing to do: it is how a fixed error stops being
                    // drawn, so it replaces the old list rather than being skipped.
                    self.diagnostics.insert(path, marks);
                }
                crate::lsp::Event::Completion { id, words } => {
                    self.absorb_lsp_completion(id, words);
                }
                crate::lsp::Event::Definition { id, target } => self.lsp_go_there(id, target),
                crate::lsp::Event::Hover { id, text } => self.lsp_show_what_it_is(id, text),
                crate::lsp::Event::Stopped { detail } => {
                    // Only this one. Another server that is still running goes on underlining
                    // its own files, and the marks that came from the one that died are the only
                    // ones that have to go.
                    self.lsp.remove(&program);
                    self.lsp_error.insert(program.clone(), detail);
                    self.lsp_completion = None;
                    self.lsp_asked = None;
                    self.lsp_forget(&program);
                    self.status_message = i18n::msg_lsp_stopped(lang);
                }
            }
        }
    }

    /// Drops everything remembered about the files a dead server was serving, and leaves the
    /// rest alone.
    fn lsp_forget(&mut self, program: &str) {
        let served: Vec<PathBuf> = self
            .lsp_sent
            .keys()
            .filter(|path| {
                self.lsp_argv_for(path).and_then(|argv| argv.first().cloned()).as_deref()
                    == Some(program)
            })
            .cloned()
            .collect();
        for path in served {
            self.lsp_sent.remove(&path);
            self.lsp_seen.remove(&path);
            self.diagnostics.remove(&path);
        }
    }


    /// The path the server knows a file by.
    ///
    /// Asked of the disk once per file and then remembered, because the server resolves symlinks:
    /// `/tmp/x` comes back as `/private/tmp/x` on macOS, and no amount of lexical tidying would
    /// have matched them. `None` means the file is not on disk yet — an unsaved buffer with a
    /// name — and there is nothing for a server to look at.
    ///
    /// Takes the map rather than `&self` so it can be called while the editors are being walked.
    fn lsp_absolute_for(
        paths: &std::collections::HashMap<PathBuf, PathBuf>,
        path: &Path,
    ) -> Option<PathBuf> {
        match paths.iter().find(|(_, held)| held.as_path() == path) {
            Some((absolute, _)) => Some(absolute.clone()),
            None => std::fs::canonicalize(path).ok(),
        }
    }

    /// Asks the server what could be typed where the popup just opened.
    ///
    /// The file is sent first, and this is the one place that goes round [`lsp::QUIET`]. The
    /// debounce is there so a server is not made to re-analyse a file that is still being
    /// written; a completion request is a question about *this* text, and an answer about the
    /// text of four hundred milliseconds ago is not a slower right answer, it is a wrong one.
    /// It costs one extra message per word typed, which is still far less than the editors that
    /// send one per keystroke.
    fn lsp_ask_completion(&mut self, editor_index: usize, start: usize) {
        self.lsp_completion = None;
        if self.lsp.is_empty() {
            return;
        }
        let Some(editor) = self.editors.get(editor_index) else { return };
        let Some(path) = editor.path.clone() else { return };
        if self.lsp_argv_for(&path).is_none() {
            return;
        }
        let (line, col) = (editor.cursor_line, editor.cursor_col);
        let line_text = editor.rope.line(line).to_string();
        let text = editor.rope.to_string();
        let revision = editor.revision();
        let Some(absolute) = Self::lsp_absolute_for(&self.lsp_paths, &path) else { return };
        self.lsp_paths.insert(absolute.clone(), path.clone());
        let Some(client) = self.lsp_client_for(&path) else { return };
        client.did_change(&absolute, &text);
        let asked = client.completion(&absolute, line, &line_text, col);
        // Recorded as sent, so the debounce does not turn round and send the same revision again.
        self.lsp_sent.insert(path, revision);
        self.lsp_completion =
            asked.map(|id| PendingCompletion { id, editor: editor_index, start });
    }

    /// Folds a server's answer into the popup that asked for it, or drops it.
    ///
    /// Three ways it is dropped, and none of them is an error: it answers a question we are no
    /// longer waiting for, the popup has closed or moved on, or the server had nothing to say.
    /// The popup carries on with the words from the buffer in every one of those cases, which is
    /// the property worth protecting — the list was never waiting on this.
    fn absorb_lsp_completion(&mut self, id: i64, words: Vec<String>) {
        let Some(pending) = self.lsp_completion.as_ref().filter(|p| p.id == id) else { return };
        let (editor, start) = (pending.editor, pending.start);
        self.lsp_completion = None;
        if words.is_empty() || !self.completion_live() {
            return;
        }
        let Some(popup) = self.completion.as_mut() else { return };
        // The popup can have closed and a new one opened on another word since the question went
        // out; the id alone would not tell them apart.
        if popup.editor != editor || popup.start != start {
            return;
        }
        popup.absorb(crate::complete::lsp_candidates(&words));
    }

    /// Tells the server what is open, what has changed once the typing has stopped, and what has
    /// gone away.
    fn lsp_sync_open_files(&mut self) {
        if self.lsp.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut to_send: Vec<(PathBuf, PathBuf, String, u64)> = Vec::new();
        let mut live: Vec<PathBuf> = Vec::new();
        let mut resolved: Vec<(PathBuf, PathBuf)> = Vec::new();
        for editor in self.editors.iter().filter(|e| e.preview.is_none()) {
            let Some(path) = editor.path.as_deref() else { continue };
            if self.lsp_argv_for(path).is_none() {
                continue;
            }
            live.push(path.to_path_buf());
            let revision = editor.revision();
            // When the revision first differs from what was sent, note the moment; the send
            // happens once that moment is old enough.
            let entry = self.lsp_seen.entry(path.to_path_buf()).or_insert((revision, now));
            if entry.0 != revision {
                *entry = (revision, now);
            }
            let since = now.saturating_duration_since(entry.1);
            if crate::lsp::should_send(self.lsp_sent.get(path).copied(), revision, since) {
                let Some(absolute) = Self::lsp_absolute_for(&self.lsp_paths, path) else {
                    continue;
                };
                resolved.push((absolute.clone(), path.to_path_buf()));
                to_send.push((absolute, path.to_path_buf(), editor.rope.to_string(), revision));
            }
        }
        let gone: Vec<PathBuf> =
            self.lsp_sent.keys().filter(|p| !live.contains(p)).cloned().collect();

        for (absolute, held) in resolved {
            self.lsp_paths.insert(absolute, held);
        }
        // Each file goes to the one server that serves it. A `.py` announced to rust-analyzer
        // is a file it will parse as Rust and report a page of errors about.
        for (absolute, held, text, revision) in to_send {
            if let Some(client) = self.lsp_client_for(&held) {
                client.did_open(&absolute, &text);
            }
            self.lsp_sent.insert(held, revision);
        }
        for path in gone {
            if let Some((absolute, _)) =
                self.lsp_paths.iter().find(|(_, held)| held.as_path() == path.as_path())
            {
                let absolute = absolute.clone();
                if let Some(client) = self.lsp_client_for(&path) {
                    client.did_close(&absolute);
                }
                self.lsp_paths.remove(&absolute);
            }
            self.lsp_sent.remove(&path);
            self.lsp_seen.remove(&path);
            self.diagnostics.remove(&path);
        }
    }

    /// Asks the server where the thing under the cursor is defined.
    ///
    /// The file is sent first, for the same reason a completion sends it first: the question is
    /// about *this* text, and an answer about the text of four hundred milliseconds ago would
    /// point into a file that has since moved under it.
    pub fn lsp_go_to_definition(&mut self) {
        let lang = self.settings.lang;
        let index = self.active_editor_index();
        let Some(editor) = self.editors.get(index) else { return };
        let Some(path) = editor.path.clone() else { return };
        let (line, col) = (editor.cursor_line, editor.cursor_col);
        let line_text = editor.rope.line(line).to_string();
        let text = editor.rope.to_string();
        let Some(absolute) = Self::lsp_absolute_for(&self.lsp_paths, &path) else {
            self.status_message = i18n::msg_lsp_needs_saving(lang).to_string();
            return;
        };
        self.lsp_paths.insert(absolute.clone(), path.clone());
        let from = (path.clone(), line, col);
        let Some(client) = self.lsp_client_for(&path) else {
            self.status_message = i18n::msg_lsp_none_here(lang).to_string();
            return;
        };
        client.did_change(&absolute, &text);
        let asked = client.definition(&absolute, line, &line_text, col);
        match asked {
            Some(id) => {
                self.lsp_asked = Some(PendingAsk { id, from });
                self.status_message = i18n::msg_lsp_looking(lang).to_string();
            }
            None => self.status_message = i18n::msg_lsp_none_here(lang).to_string(),
        }
    }

    /// Opens what the server pointed at, and remembers where you were.
    fn lsp_go_there(&mut self, id: i64, target: Option<crate::lsp::Jump>) {
        let lang = self.settings.lang;
        let Some(asked) = self.lsp_asked.take().filter(|a| a.id == id) else { return };
        let Some(target) = target else {
            // An answer, and worth saying. A key that does nothing and says nothing is a key you
            // press again harder, and "there is no definition of that" is usually the news:
            // the cursor is on a keyword, a comment, or a name the server has not indexed yet.
            self.status_message = i18n::msg_lsp_no_definition(lang).to_string();
            return;
        };
        // Written down before the jump rather than after it, so the place remembered is where
        // the key was pressed and not wherever the file that opened happened to be scrolled to.
        self.jumps.push(asked.from);
        // Back to the spelling the tabs use. The server answers in resolved paths — symlinks
        // followed, `.` gone — and opening a second tab on a file that is already open under
        // another name is how one file ends up with two buffers and one of them silently stale.
        let path = self.lsp_paths.get(&target.path).cloned().unwrap_or(target.path);
        self.open_file_in_tab(path);
        // The column arrives in the server's units and is turned into characters here, which is
        // the first place that has the target file's text to measure it against.
        let index = self.active_editor_index();
        let utf16 = self
            .editors
            .get(index)
            .and_then(|e| e.path.clone())
            .and_then(|p| self.lsp_argv_for(&p))
            .and_then(|argv| argv.first().cloned())
            .and_then(|program| self.lsp.get(&program))
            .map(crate::lsp::Client::utf16)
            .unwrap_or(true);
        let line_text = self
            .editors
            .get(index)
            .map(|e| e.rope.line(target.line.min(e.rope.len_lines().saturating_sub(1))).to_string())
            .unwrap_or_default();
        let column = if utf16 {
            crate::lsp::utf16_to_chars(&line_text, target.column)
        } else {
            target.column
        };
        // The protocol counts lines from zero and `goto_line` counts them the way a person does.
        self.editor_mut().goto_line(target.line + 1);
        let index = self.active_editor_index();
        let len = self.editors[index].line_char_len(self.editors[index].cursor_line);
        self.editors[index].cursor_col = column.min(len);
        self.status_message = String::new();
    }

    /// Back to where the last jump started from.
    pub fn lsp_jump_back(&mut self) {
        let lang = self.settings.lang;
        let Some((path, line, col)) = self.jumps.pop() else {
            self.status_message = i18n::msg_lsp_nowhere_back(lang).to_string();
            return;
        };
        // Counted as a person counts them, which is what `goto_line` takes; the stack holds the
        // editor's own zero-based line.
        self.open_file_at(path, line + 1, col);
    }

    /// Asks what the thing under the cursor is, once the cursor has stopped moving.
    ///
    /// No key of its own, and that is the design rather than a shortage of keys — though there
    /// is one of those too. A hover is the answer to a question you did not quite ask: what is
    /// this, what does it return. Anything that has to be asked for is asked for by people who
    /// already know. So it arrives on its own, in the one line of the status bar that already
    /// carries what the server has to say about the line you are on.
    ///
    /// The diagnostic wins that space when there is one. An error on this line is news; the type
    /// of the name under the cursor is not, while there is something wrong with it.
    fn lsp_ask_what_this_is(&mut self) {
        if self.lsp.is_empty() || self.completion.is_some() || self.a_modal_owns_the_keyboard() {
            return;
        }
        let index = self.active_editor_index();
        let Some(editor) = self.editors.get(index) else { return };
        let Some(path) = editor.path.clone() else { return };
        // Nothing is asked about a file the server has not been told about yet. It would answer
        // — about a document it has never seen, which is to say about nothing — and the answer
        // would be remembered as this file's. Announcing the file happens a frame or two after
        // it opens, and this is the window in between.
        //
        // Unlike the definition, which sends the file with the question, a hover is asked so
        // often that sending the whole buffer each time would be a copy of the file per word
        // read. So it waits for the announcement instead of forcing one. And it waits without
        // writing anything down, so the next frame asks again.
        if !self.lsp_sent.contains_key(&path) {
            return;
        }
        let (line, col) = (editor.cursor_line, editor.cursor_col);
        // Asked once per place. Without this the same question goes out every frame for as long
        // as the cursor sits still, which is most of the time a file is open.
        let here = (path.clone(), line, col);
        if self.lsp_hovered.as_ref() == Some(&here) {
            return;
        }
        // Only on a word. A hover over a bracket is a request the server answers with nothing,
        // sent thirty times a second while somebody reads.
        let line_text = editor.rope.line(line).to_string();
        if !crate::complete::prefix_at(&editor.rope, line, col)
            .is_some_and(|(_, word)| !word.is_empty())
        {
            self.lsp_hovered = Some(here);
            self.lsp_what_it_is = None;
            return;
        }
        let Some(absolute) = Self::lsp_absolute_for(&self.lsp_paths, &path) else { return };
        // Not while a definition is still out: one question at a time, and that one was asked
        // on purpose.
        if self.lsp_asked.is_some() {
            return;
        }
        self.lsp_hovered = Some(here.clone());
        self.lsp_what_it_is = None;
        let Some(client) = self.lsp_client_for(&path) else { return };
        if let Some(id) = client.hover(&absolute, line, &line_text, col) {
            self.lsp_asked = Some(PendingAsk { id, from: here });
        }
    }

    /// Holds on to what the server said, if the cursor is still where it was asked about.
    fn lsp_show_what_it_is(&mut self, id: i64, text: Option<String>) {
        let Some(asked) = self.lsp_asked.take().filter(|a| a.id == id) else { return };
        if self.lsp_hovered.as_ref() != Some(&asked.from) {
            return;
        }
        self.lsp_what_it_is = text;
    }

    /// What the server says the thing under the cursor is, for the status bar to draw when it
    /// has nothing more pressing to say there.
    pub fn what_it_is(&self) -> Option<&str> {
        self.lsp_what_it_is.as_deref()
    }

    /// The diagnostics for a file, or an empty slice — so the renderer can ask without checking.
    pub fn marks_for(&self, path: Option<&Path>) -> &[crate::lsp::Mark] {
        path.and_then(|p| self.diagnostics.get(p)).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Picks up a finished search. The results become an ordinary picker, so the list is
    /// navigated, filtered and clicked with everything already built for the palette — and the
    /// query typed there narrows a search that found too much without running it again.
    pub fn poll_search(&mut self) {
        let lang = self.settings.lang;
        while let Ok(outcome) = self.search_rx.try_recv() {
            if let Some(detail) = &outcome.error {
                self.status_message = i18n::msg_find_pattern_error(lang, detail);
                continue;
            }
            if outcome.hits.is_empty() {
                self.status_message = i18n::msg_search_none(lang, &outcome.query, outcome.files_searched);
                continue;
            }
            self.status_message = i18n::msg_search_done(
                lang,
                outcome.hits.len(),
                outcome.files_searched,
                outcome.truncated,
            );
            let root = self.root.clone();
            let items = outcome
                .hits
                .iter()
                .map(|hit| crate::picker::PickItem {
                    label: crate::search::label(hit, &root),
                    shortcut: None,
                    action: crate::picker::PickAction::FileLine(hit.path.clone(), hit.line, hit.col),
                })
                .collect();
            self.picker = Some(crate::picker::Picker::new(
                "Search results",
                crate::picker::PickerKind::SearchResults,
                items,
            ));
        }
    }

    // ---- Git panel ------------------------------------------------------------------------

    /// Opens the panel and asks the repository the three questions at once. Fetched together so
    /// switching tabs never waits, and on a thread so a slow repository — a big diff, a network
    /// filesystem — costs a moment of "…" rather than a frozen editor.
    fn toggle_git_panel(&mut self) {
        if self.git_panel.is_some() {
            self.git_panel = None;
            return;
        }
        self.git_panel = Some(GitPanel {
            tab: GitTab::Status,
            scroll: 0,
            selected: 0,
            snap: None,
            rows: Vec::new(),
            detail: None,
            prompt: None,
            busy: false,
            notice: None,
            body_rows: 1,
        });
        self.refresh_git_panel();
    }

    fn refresh_git_panel(&mut self) {
        self.git_asked += 1;
        let asked = self.git_asked;
        let root = self.root.clone();
        // The diff is of the file in front of you when there is one: that is what "what have I
        // changed" means while you are looking at it. With no file open it is the whole tree.
        let file = self.editor().path.clone();
        let tx = self.git_panel_tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(GitMessage::Snapshot(asked, Box::new(crate::git::snapshot(&root, file))));
        });
    }

    pub fn poll_git_panel(&mut self) {
        let lang = self.settings.lang;
        while let Ok(message) = self.git_panel_rx.try_recv() {
            match message {
                GitMessage::Snapshot(asked, snap) => {
                    // Anything but the answer to the latest ask is thrown away: it describes a
                    // repository that has since been written to, and drawing it would put the
                    // panel back to before the action that finished.
                    if asked != self.git_asked {
                        continue;
                    }
                    let Some(panel) = self.git_panel.as_mut() else { continue };
                    // Laid out here rather than while drawing: the walk is over every commit in
                    // the repository up to the limit, and it only has a new answer when the
                    // commits change.
                    panel.rows = crate::git_graph::lay_out(&snap.graph);
                    panel.snap = Some(*snap);
                    let wanted = self.git_wanted.is_some();
                    // Clamped rather than reset. A refresh follows every action, and being put
                    // back at the top after each one would make staging five files an exercise
                    // in counting down to the sixth again.
                    panel.clamp_to_list();
                    if wanted {
                        self.git_put_up_the_wanted_question();
                    }
                }
                GitMessage::Wrote(outcome) => {
                    if let Some(panel) = self.git_panel.as_mut() {
                        panel.busy = false;
                        panel.notice = Some(match &outcome {
                            Ok(said) if said.is_empty() => {
                                (i18n::msg_git_done(lang).to_string(), false)
                            }
                            Ok(said) => (said.clone(), false),
                            Err(complaint) => (complaint.clone(), true),
                        });
                    }
                    // Whatever happened, what is on screen is now the state from before it.
                    self.refresh_git_panel();
                    // And so are the dots in the file tree, which are the same facts drawn in
                    // the frame behind the panel. They have their own sweep every 700 ms, which
                    // is soon enough for a file changed by something else and far too slow for a
                    // commit made from here: the panel would say the tree was clean while the
                    // tree beside it still had a row marked.
                    spawn_git_status_refresh(
                        self.root.clone(),
                        self.git_status_tx.clone(),
                        self.git_status_pending.clone(),
                    );
                }
                GitMessage::Detail(hash, lines) => {
                    // Only into the reader that asked. A `git show` on a large commit takes long
                    // enough for the cursor to have moved on, and an answer landing in the wrong
                    // reader is a commit shown under another commit's name.
                    if let Some(detail) = self.git_panel.as_mut().and_then(|p| p.detail.as_mut())
                        && detail.hash == hash
                    {
                        detail.lines = Some(lines);
                    }
                }
            }
        }
    }

    /// Hands a writing `git` to a thread and takes the answer back on the same channel the
    /// reads come in on.
    ///
    /// On a thread for the reason every other slow thing here is: `git commit` runs the hooks,
    /// and a pre-commit hook that runs a test suite would otherwise stop the frame loop — the
    /// editor, the terminals and the clock along with it.
    fn git_write<F>(&mut self, action: F)
    where
        F: FnOnce(&Path) -> Result<String, String> + Send + 'static,
    {
        let Some(panel) = self.git_panel.as_mut() else { return };
        // Two `git add`s racing for the index lock is a failure that reads as a dropped
        // keystroke, so the second one is not started.
        if panel.busy {
            return;
        }
        panel.busy = true;
        panel.notice = Some((i18n::msg_git_working(self.settings.lang).to_string(), false));
        let root = self.root.clone();
        let tx = self.git_panel_tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(GitMessage::Wrote(action(&root)));
        });
    }

    fn handle_git_panel_key(&mut self, key: KeyEvent) {
        // A question that is up takes the whole keyboard until it is answered. It is the only
        // nesting in this panel that changes what a key means, and it is here because the boxes
        // it puts up — a message, a name, and whether to throw work away — must not be answered
        // by a keystroke meant for the list behind them.
        if self.git_panel.as_ref().is_some_and(|p| p.prompt.is_some()) {
            self.handle_git_prompt_key(key);
            return;
        }
        // A commit opened in full is a reader, not a list: it scrolls and it closes, and the
        // letters that act on the graph underneath are not offered on top of it. Esc goes back
        // to the graph rather than out of the panel, because leaving a reader is going back to
        // what you were reading.
        if self.git_panel.as_ref().is_some_and(|p| p.detail.is_some()) {
            self.handle_git_detail_key(key);
            return;
        }
        match key.code {
            KeyCode::Esc => self.git_panel = None,
            KeyCode::Tab | KeyCode::Right => self.switch_git_tab(1),
            KeyCode::BackTab | KeyCode::Left => self.switch_git_tab(-1),
            KeyCode::Down => self.scroll_git_panel(1),
            KeyCode::Up => self.scroll_git_panel(-1),
            KeyCode::PageDown => self.scroll_git_panel(10),
            KeyCode::PageUp => self.scroll_git_panel(-10),
            KeyCode::Home => {
                if let Some(p) = self.git_panel.as_mut() {
                    p.scroll = 0;
                    p.selected = 0;
                    p.clamp_to_list();
                }
            }
            // The answer goes stale the moment you type in the shell next to it, so asking again
            // is a first-class action rather than something to close and reopen for.
            KeyCode::Char('r') | KeyCode::Char('R') => {
                if let Some(p) = self.git_panel.as_mut() {
                    p.snap = None;
                    p.rows.clear();
                    p.notice = None;
                }
                self.refresh_git_panel();
            }
            // The way out of a merge, pick, revert or rebase that stopped on a conflict. Offered
            // from every tab, and only while there is one to get out of: the state it undoes is
            // one you did not choose to be in, and hunting for the right tab to leave it from
            // would be a puzzle on top of a problem.
            KeyCode::Char('q') | KeyCode::Char('Q') => self.git_abort(),
            KeyCode::Enter => self.git_enter(),
            KeyCode::Char(c) => self.git_letter(c.to_ascii_lowercase()),
            _ => {}
        }
    }

    /// The single letters, which mean what the tab you are on says they mean.
    ///
    /// A letter does one thing per tab and nothing on the others, rather than one thing
    /// everywhere. `d` deletes a branch on the branch list and drops a stash on the stash list,
    /// and neither of those is a thing you can do to the other — a key that guessed which list
    /// you meant would be guessing about deleting something.
    fn git_letter(&mut self, c: char) {
        let Some(tab) = self.git_panel.as_ref().map(|p| p.tab) else { return };
        match (tab, c) {
            (GitTab::Status, 's') => self.git_stage_selected(),
            (GitTab::Status, 'u') => self.git_unstage_selected(),
            (GitTab::Status, 'a') => self.git_stage_everything(),
            (GitTab::Status, 'c') => self.git_ask_for_a_message(),
            (GitTab::Status, 'e') => self.git_ask_to_amend(),
            (GitTab::Status, 'x') => self.git_ask_about_discarding(),
            (GitTab::Status, 'z') | (GitTab::Stashes, 'z') => self.git_ask_for_a_stash(),

            (GitTab::Graph, 'b') => self.git_ask_for_a_branch(true),
            (GitTab::Graph, 't') => self.git_ask_for_a_tag(),
            (GitTab::Graph, 'k') => self.git_cherry_pick(),
            (GitTab::Graph, 'v') => self.git_revert(),
            (GitTab::Graph, 'h') => self.git_ask_about_resetting(),

            (GitTab::Branches, 'n') => self.git_ask_for_a_branch(false),
            (GitTab::Branches, 'd') => self.git_ask_about_deleting_a_branch(),
            (GitTab::Branches, 'm') => self.git_merge_selected(),
            (GitTab::Branches, 'f') => self.git_remote(crate::git::Remote::Fetch),
            (GitTab::Branches, 'l') => self.git_remote(crate::git::Remote::Pull),
            (GitTab::Branches, 'p') => self.git_remote(crate::git::Remote::Push),

            (GitTab::Stashes, 'o') => self.git_stash_pop(),
            (GitTab::Stashes, 'd') => self.git_ask_about_dropping_a_stash(),
            _ => {}
        }
    }

    /// Enter: the obvious thing for whichever list is in front of you.
    fn git_enter(&mut self) {
        let Some(tab) = self.git_panel.as_ref().map(|p| p.tab) else { return };
        match tab {
            GitTab::Status | GitTab::Branches => self.git_open_or_switch(),
            GitTab::Graph => self.git_show_commit(),
            // Apply and not pop, because apply is the one that can be tried again. A stash that
            // does not go back cleanly is exactly when you want it still to be there.
            GitTab::Stashes => self.git_stash_apply(),
            GitTab::Diff => {}
        }
    }

    fn handle_git_detail_key(&mut self, key: KeyEvent) {
        let Some(panel) = self.git_panel.as_mut() else { return };
        let Some(detail) = panel.detail.as_mut() else { return };
        let last = detail.lines.as_ref().and_then(|l| l.as_ref().ok()).map_or(0, |l| l.len());
        let max = last.saturating_sub(1) as isize;
        let step = |at: usize, delta: isize| (at as isize + delta).clamp(0, max) as usize;
        match key.code {
            KeyCode::Esc => panel.detail = None,
            KeyCode::Down => detail.scroll = step(detail.scroll, 1),
            KeyCode::Up => detail.scroll = step(detail.scroll, -1),
            KeyCode::PageDown => detail.scroll = step(detail.scroll, panel.body_rows as isize),
            KeyCode::PageUp => detail.scroll = step(detail.scroll, -(panel.body_rows as isize)),
            KeyCode::Home => detail.scroll = 0,
            KeyCode::End => detail.scroll = max.max(0) as usize,
            _ => {}
        }
    }

    fn switch_git_tab(&mut self, delta: isize) {
        let Some(panel) = self.git_panel.as_mut() else { return };
        panel.tab = panel.tab.cycle(delta);
        // Each tab is a list of its own length, and carrying an offset across would land in the
        // middle of a shorter one.
        panel.scroll = 0;
        panel.selected = 0;
        panel.clamp_to_list();
    }

    /// The boxes the panel puts up. Everything not spelled out here says no — which is the safe
    /// answer to the questions that destroy something and a harmless one to the rest.
    fn handle_git_prompt_key(&mut self, key: KeyEvent) {
        let lang = self.settings.lang;
        let Some(panel) = self.git_panel.as_mut() else { return };
        match panel.prompt.as_mut() {
            Some(GitPrompt::Text { kind, typed }) => match key.code {
                KeyCode::Esc => panel.prompt = None,
                KeyCode::Backspace => {
                    typed.pop();
                }
                // The modifiers are checked, and that is not pedantry: without it `Ctrl+V` puts a
                // `v` in the commit message, which is what a person pressing it least wants and
                // has no way to tell has happened until the commit is made.
                KeyCode::Char(c)
                    if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    typed.push(c)
                }
                KeyCode::Enter => {
                    let typed = typed.clone();
                    let kind = std::mem::replace(kind, GitText::Commit);
                    panel.prompt = None;
                    self.run_git_text(kind, typed);
                }
                _ => {}
            },
            Some(GitPrompt::Confirm(confirm)) => {
                // Only the one letter, and only the one the question was asked in. Every other
                // key means no — including the ones that do something on the list underneath,
                // which is the whole point of asking.
                let yes = key.code == KeyCode::Char(i18n::yes_key(lang))
                    || key.code == KeyCode::Char(i18n::yes_key(lang).to_ascii_uppercase());
                let confirm = std::mem::replace(
                    confirm,
                    GitConfirm::DeleteBranch(String::new()),
                );
                panel.prompt = None;
                if yes {
                    self.run_git_confirm(confirm);
                }
            }
            None => {}
        }
    }

    /// What a typed box does once it is answered.
    fn run_git_text(&mut self, kind: GitText, typed: String) {
        let lang = self.settings.lang;
        // A name that is only spaces would become a branch git refuses and a tag it accepts,
        // which are two bad answers to one slip.
        let typed = typed.trim().to_string();
        match kind {
            GitText::Commit => self.git_write(move |root| crate::git::commit(root, &typed)),
            GitText::Amend => self.git_write(move |root| crate::git::amend(root, &typed)),
            GitText::Branch { at } => {
                if typed.is_empty() {
                    self.git_say(i18n::msg_git_needs_a_name(lang).to_string(), true);
                    return;
                }
                self.git_write(move |root| crate::git::create_branch(root, &typed, at.as_deref()));
            }
            GitText::Tag { at } => {
                if typed.is_empty() {
                    self.git_say(i18n::msg_git_needs_a_name(lang).to_string(), true);
                    return;
                }
                self.git_write(move |root| crate::git::tag(root, &typed, &at));
            }
            // An unnamed stash is fine: git writes "WIP on main: …" itself, and it is a better
            // sentence than most of the ones a person types into that box.
            GitText::Stash => self.git_write(move |root| crate::git::stash_push(root, &typed)),
        }
    }

    /// What a one-letter question does once it is agreed to.
    fn run_git_confirm(&mut self, confirm: GitConfirm) {
        match confirm {
            GitConfirm::Discard(change) => {
                let top = self.git_panel.as_ref().and_then(|p| p.snap.as_ref()).and_then(|s| s.top.clone());
                let Some(top) = top else { return };
                let absolute = crate::git::Change { path: top.join(&change.path), ..change };
                self.git_write(move |root| crate::git::discard(root, &absolute));
            }
            GitConfirm::DeleteBranch(name) => {
                self.git_write(move |root| crate::git::delete_branch(root, &name))
            }
            GitConfirm::ResetHard { hash, .. } => {
                self.git_write(move |root| crate::git::reset_hard(root, &hash))
            }
            GitConfirm::DropStash(name) => {
                self.git_write(move |root| crate::git::stash_drop(root, &name))
            }
        }
    }

    fn git_stage_selected(&mut self) {
        let Some(path) = self.git_panel.as_ref().and_then(GitPanel::selected_path) else { return };
        self.git_write(move |root| crate::git::stage(root, &path));
    }

    fn git_unstage_selected(&mut self) {
        let Some(path) = self.git_panel.as_ref().and_then(GitPanel::selected_path) else { return };
        self.git_write(move |root| crate::git::unstage(root, &path));
    }

    fn git_stage_everything(&mut self) {
        if self.git_panel.as_ref().is_none_or(|p| p.tab != GitTab::Status) {
            return;
        }
        self.git_write(crate::git::stage_all)
    }

    /// Opens the commit box, unless there is nothing to commit — in which case it says what to
    /// do instead. An empty commit box that then fails is a longer way of saying the same thing.
    fn git_ask_for_a_message(&mut self) {
        let lang = self.settings.lang;
        let Some(panel) = self.git_panel.as_mut() else { return };
        if panel.tab != GitTab::Status {
            return;
        }
        if panel.staged_count() == 0 {
            panel.notice = Some((i18n::msg_git_nothing_staged(lang).to_string(), true));
            return;
        }
        panel.prompt = Some(GitPrompt::Text { kind: GitText::Commit, typed: String::new() });
    }

    fn git_ask_about_discarding(&mut self) {
        let Some(panel) = self.git_panel.as_mut() else { return };
        let Some(change) = panel.selected_change().cloned() else { return };
        // Said before the question rather than after the answer: a file git has never seen has
        // no earlier version to go back to, and finding that out by confirming would be finding
        // it out from an error.
        if change.untracked() {
            if let Err(why) = crate::git::discard(Path::new("."), &change) {
                panel.notice = Some((why, true));
            }
            return;
        }
        panel.prompt = Some(GitPrompt::Confirm(GitConfirm::Discard(change)));
    }

    /// Says something in the panel's own notice line without going near git.
    ///
    /// The refusals belong here rather than in `git.rs`: "there is nothing staged" is a fact
    /// about what the panel is showing, and answering it by running a command that fails would
    /// be asking git to write the message.
    fn git_say(&mut self, text: String, complaint: bool) {
        if let Some(panel) = self.git_panel.as_mut() {
            panel.notice = Some((text, complaint));
        }
    }

    /// Replaces the last commit. Opens holding the message that commit already has.
    fn git_ask_to_amend(&mut self) {
        let lang = self.settings.lang;
        let root = self.root.clone();
        let Some(panel) = self.git_panel.as_mut() else { return };
        if panel.tab != GitTab::Status {
            return;
        }
        // Read here rather than on a thread: it is one `git log -1` on a commit that is already
        // in memory on any repository, and a box that opened empty and filled in a frame later
        // is a box you have started typing into by then.
        let Some(message) = crate::git::head_message(&root) else {
            panel.notice = Some((i18n::msg_git_nothing_to_amend(lang).to_string(), true));
            return;
        };
        panel.prompt = Some(GitPrompt::Text { kind: GitText::Amend, typed: message });
    }

    fn git_ask_for_a_stash(&mut self) {
        let lang = self.settings.lang;
        let Some(panel) = self.git_panel.as_mut() else { return };
        // Nothing to put away is worth saying rather than letting git say "No local changes to
        // save", which reads as a failure when it is the tree being clean.
        if panel.snap.as_ref().is_some_and(|s| s.changes.iter().all(crate::git::Change::untracked)) {
            panel.notice = Some((i18n::msg_git_nothing_to_stash(lang).to_string(), true));
            return;
        }
        panel.prompt = Some(GitPrompt::Text { kind: GitText::Stash, typed: String::new() });
    }

    /// A new branch. From the graph it starts at the commit under the cursor; from the branch
    /// list it starts where you are.
    fn git_ask_for_a_branch(&mut self, from_graph: bool) {
        let Some(panel) = self.git_panel.as_mut() else { return };
        let at = if from_graph {
            let Some(commit) = panel.selected_commit() else { return };
            Some(commit.hash.clone())
        } else {
            None
        };
        panel.prompt = Some(GitPrompt::Text { kind: GitText::Branch { at }, typed: String::new() });
    }

    fn git_ask_for_a_tag(&mut self) {
        let Some(panel) = self.git_panel.as_mut() else { return };
        let Some(commit) = panel.selected_commit() else { return };
        let at = commit.hash.clone();
        panel.prompt = Some(GitPrompt::Text { kind: GitText::Tag { at }, typed: String::new() });
    }

    /// Copies the commit under the cursor onto the branch you are on.
    ///
    /// No question in front of it, deliberately. It makes a commit and takes nothing away, and
    /// if it stops on a conflict the way out is the same `Q` that gets out of a merge — which is
    /// why that key is offered from every tab rather than from the one that started it.
    fn git_cherry_pick(&mut self) {
        let Some(commit) = self.git_panel.as_ref().and_then(GitPanel::selected_commit) else {
            return;
        };
        let hash = commit.hash.clone();
        self.git_write(move |root| crate::git::cherry_pick(root, &hash));
    }

    /// Makes a new commit that undoes an old one. The commit being undone stays exactly where it
    /// is, which is why this needs no question either.
    fn git_revert(&mut self) {
        let Some(commit) = self.git_panel.as_ref().and_then(GitPanel::selected_commit) else {
            return;
        };
        let hash = commit.hash.clone();
        self.git_write(move |root| crate::git::revert(root, &hash));
    }

    fn git_ask_about_resetting(&mut self) {
        let Some(panel) = self.git_panel.as_mut() else { return };
        let Some(commit) = panel.selected_commit() else { return };
        let hash = commit.hash.clone();
        let subject = commit.subject.clone();
        panel.prompt = Some(GitPrompt::Confirm(GitConfirm::ResetHard { hash, subject }));
    }

    fn git_ask_about_deleting_a_branch(&mut self) {
        let lang = self.settings.lang;
        let Some(panel) = self.git_panel.as_mut() else { return };
        let Some(branch) = panel.selected_branch() else { return };
        // The branch you are standing on cannot be deleted, and git's refusal names the branch
        // rather than the reason. Said before the question, for the same reason discarding an
        // untracked file is: finding out by agreeing to something is finding out too late.
        if branch.current {
            let name = branch.name.clone();
            panel.notice = Some((i18n::msg_git_branch_is_current(lang, &name), true));
            return;
        }
        let name = branch.name.clone();
        panel.prompt = Some(GitPrompt::Confirm(GitConfirm::DeleteBranch(name)));
    }

    /// Merges the branch under the cursor into the one you are on.
    fn git_merge_selected(&mut self) {
        let lang = self.settings.lang;
        let Some(panel) = self.git_panel.as_mut() else { return };
        let Some(branch) = panel.selected_branch() else { return };
        if branch.current {
            panel.notice = Some((i18n::msg_git_merge_into_itself(lang).to_string(), true));
            return;
        }
        let name = branch.name.clone();
        self.git_write(move |root| crate::git::merge(root, &name));
    }

    fn git_stash_apply(&mut self) {
        let Some(stash) = self.git_panel.as_ref().and_then(GitPanel::selected_stash) else {
            return;
        };
        let name = stash.name.clone();
        self.git_write(move |root| crate::git::stash_apply(root, &name));
    }

    fn git_stash_pop(&mut self) {
        let Some(stash) = self.git_panel.as_ref().and_then(GitPanel::selected_stash) else {
            return;
        };
        let name = stash.name.clone();
        self.git_write(move |root| crate::git::stash_pop(root, &name));
    }

    fn git_ask_about_dropping_a_stash(&mut self) {
        let Some(panel) = self.git_panel.as_mut() else { return };
        let Some(stash) = panel.selected_stash() else { return };
        let name = stash.name.clone();
        panel.prompt = Some(GitPrompt::Confirm(GitConfirm::DropStash(name)));
    }

    /// Opens the commit under the cursor in full.
    ///
    /// On a thread, because `git show` on a commit that touched two hundred files is not
    /// instant, and the reader opens straight away saying so rather than after it.
    fn git_show_commit(&mut self) {
        let Some(commit) = self.git_panel.as_ref().and_then(GitPanel::selected_commit) else {
            return;
        };
        let hash = commit.hash.clone();
        let subject = commit.subject.clone();
        if let Some(panel) = self.git_panel.as_mut() {
            panel.detail =
                Some(GitDetail { hash: hash.clone(), subject, lines: None, scroll: 0 });
        }
        let root = self.root.clone();
        let tx = self.git_panel_tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(GitMessage::Detail(hash.clone(), crate::git::show(&root, &hash)));
        });
    }

    /// Puts back whatever a half-finished merge, pick, revert or rebase was in the middle of.
    fn git_abort(&mut self) {
        let lang = self.settings.lang;
        let unfinished = self.git_panel.as_ref().and_then(|p| p.snap.as_ref()).and_then(|s| s.unfinished);
        let Some(unfinished) = unfinished else {
            // Nothing to get out of. Said rather than ignored: a key that does nothing and says
            // nothing is a key you press again harder.
            self.git_say(i18n::msg_git_nothing_to_abort(lang).to_string(), false);
            return;
        };
        self.git_write(move |root| crate::git::abort(root, unfinished));
    }

    /// Whether git has anything to say about the file the tree has selected.
    ///
    /// Read off the map the sidebar's own dots are drawn from, so the menu and the mark beside
    /// the name always agree — a row with a dot offers the git items and a row without does not,
    /// which is a rule you can see rather than one you have to learn. A file git has never been
    /// told about counts: it has a dot too, and staging it is the obvious thing to want.
    fn selected_file_is_versioned(&self) -> bool {
        self.file_tree
            .selected_path()
            .is_some_and(|path| self.git_status.contains_key(&path) && path.is_file())
    }

    /// Stage or unstage the file the tree has selected, without going near the panel.
    ///
    /// Both are commands that change nothing you cannot change back, which is the whole reason
    /// they can happen straight from a right-click. The answer lands in the status bar rather
    /// than in the panel's notice line, because the panel is very likely not open.
    fn git_file_action(&mut self, stage: bool) {
        let lang = self.settings.lang;
        let Some(path) = self.file_tree.selected_path() else { return };
        let root = self.root.clone();
        // On the frame thread: `git add` on one path is a few milliseconds, and unlike a commit
        // it runs no hooks. The panel's own writes go to a thread because a pre-commit hook can
        // run a test suite; nothing here can.
        let outcome = if stage {
            crate::git::stage(&root, &path)
        } else {
            crate::git::unstage(&root, &path)
        };
        self.status_message = match outcome {
            Ok(said) if said.is_empty() => i18n::msg_git_done(lang).to_string(),
            Ok(said) => said,
            Err(complaint) => complaint,
        };
        // The dot beside the name is the thing that just changed, so it is asked again now
        // rather than at the next sweep 700 ms away.
        spawn_git_status_refresh(
            self.root.clone(),
            self.git_status_tx.clone(),
            self.git_status_pending.clone(),
        );
        if self.git_panel.is_some() {
            self.refresh_git_panel();
        }
    }

    /// Opens the panel on the diff of the file the tree has selected.
    fn git_show_file_in_panel(&mut self) {
        self.open_git_panel_on(GitTab::Diff);
    }

    /// Opens the panel on the file the tree has selected, with the discard question already up.
    ///
    /// The question is not re-asked out here, and that is deliberate. Its rules — one letter,
    /// every other key a no, and a flat refusal for a file git has never been told about — are
    /// the most carefully written thing in the panel and the only ones guarding an action
    /// nothing undoes. A second copy of them on the right-click would be a second copy to keep
    /// right, and the one that went wrong would be the one nobody was watching.
    ///
    /// The panel may still be waiting on git when this runs, so the file is remembered and the
    /// question goes up when the answer arrives.
    fn git_ask_to_discard_the_tree_selection(&mut self) {
        let Some(path) = self.file_tree.selected_path() else { return };
        self.open_git_panel_on(GitTab::Status);
        self.git_wanted = Some(GitWanted::Discard(path));
        self.git_put_up_the_wanted_question();
    }

    /// Opens the panel on Status with the commit box up — the end of the sentence that starts by
    /// staging a file from the same pop-up.
    fn git_ask_for_a_message_in_the_panel(&mut self) {
        self.open_git_panel_on(GitTab::Status);
        self.git_wanted = Some(GitWanted::Commit);
        self.git_put_up_the_wanted_question();
    }

    /// Moves the cursor to the file a right-click asked about and puts its question up.
    ///
    /// Does nothing until the snapshot is in: the list it has to find the file in does not exist
    /// before then. Called both when the request is made and when the snapshot lands, so it
    /// happens on whichever comes second.
    fn git_put_up_the_wanted_question(&mut self) {
        let lang = self.settings.lang;
        let Some(wanted) = self.git_wanted.clone() else { return };
        let Some(panel) = self.git_panel.as_mut() else {
            self.git_wanted = None;
            return;
        };
        if panel.snap.is_none() {
            return;
        }
        let wanted = match wanted {
            GitWanted::Commit => {
                self.git_wanted = None;
                self.git_ask_for_a_message();
                return;
            }
            GitWanted::Discard(path) => path,
        };
        let Some(panel) = self.git_panel.as_mut() else { return };
        let Some(snap) = panel.snap.as_ref() else { return };
        let Some(top) = snap.top.clone() else {
            self.git_wanted = None;
            return;
        };
        let found = snap
            .changes
            .iter()
            .position(|c| top.join(&c.path) == wanted)
            .map(|at| (at, snap.changes[at].clone()));
        self.git_wanted = None;
        let Some((at, change)) = found else {
            // Nothing to throw away. Said rather than opening a question about a file that is
            // already what the last commit says it is.
            panel.notice = Some((i18n::msg_git_file_unchanged(lang).to_string(), false));
            return;
        };
        panel.selected = at;
        panel.reveal();
        if change.untracked() {
            // The same refusal the panel gives, given here for the same reason: there is no
            // earlier version to go back to, and finding that out by agreeing is too late.
            if let Err(why) = crate::git::discard(std::path::Path::new("."), &change) {
                panel.notice = Some((why, true));
            }
            return;
        }
        panel.prompt = Some(GitPrompt::Confirm(GitConfirm::Discard(change)));
    }

    /// Opens the panel already on a tab, which is how the Git menu reaches all five of them.
    ///
    /// Which tab you want is the question you arrive with — "what have I changed", "where am I"
    /// — so answering it from the menu is better than opening on Status and asking you to press
    /// Tab three times to get to the one you meant.
    fn open_git_panel_on(&mut self, tab: GitTab) {
        if self.git_panel.is_none() {
            self.toggle_git_panel();
        }
        if let Some(panel) = self.git_panel.as_mut() {
            panel.tab = tab;
            panel.scroll = 0;
            panel.selected = 0;
            panel.detail = None;
            panel.clamp_to_list();
        }
    }

    /// Fetch, pull and push — the three that talk to another machine, and the reason they were
    /// out of this panel for three releases.
    ///
    /// They are typed into a shell rather than run behind the panel, and that *is* the feature.
    /// Any of them can stop to ask for a passphrase, a two-factor code or a host key, and a
    /// modal box has nowhere to put such a question: run from here they would hang with it on a
    /// pipe nobody can see, which is exactly the failure the original decision was avoiding.
    /// A terminal is the thing that can ask it, and CleeCode has real ones a keypress away.
    ///
    /// So the panel closes and the shell takes the focus. Watching git talk to a server is
    /// watching a terminal, and doing it from behind a box that covers the window is not
    /// watching it at all.
    fn git_remote(&mut self, op: crate::git::Remote) {
        let lang = self.settings.lang;
        // Asked of git rather than read off the panel's snapshot, and reachable with no panel
        // open at all — the Git menu offers these three whether or not you have been in it. The
        // snapshot would be free and can be wrong by the time it is used: the shell next door
        // can change branch, and a push built on a stale answer pushes a different one.
        let (branch, upstream) = crate::git::head_branch(&self.root);
        let command = crate::git::remote_command(op, branch.as_deref(), upstream);
        self.git_panel = None;
        self.type_into_a_shell(&command);
        self.status_message = i18n::msg_git_in_terminal(lang, &command);
    }

    /// Types a command at a shell prompt and puts the focus there.
    ///
    fn type_into_a_shell(&mut self, command: &str) {
        let Some(at) = self.a_shell_to_type_into() else { return };
        if let Some(term) = self.window_tab_mut(at) {
            term.type_line(command);
        }
        self.active_terminal = at;
        self.settings.show_terminal = true;
        self.focus = Focus::Terminal;
    }

    /// A pane a shell command can be typed into, opening one if there is none free.
    ///
    /// Free means free, not merely quiet. A prompt with something running under it takes a typed
    /// line as *input to that thing*, and when the thing is an interpreter the line lands in the
    /// user's own transcript as their own mistake:
    ///
    ///     >>> python3 /home/ada/hello.py
    ///     NameError: name 'python3' is not defined
    ///
    /// which is what pressing Run with a Python prompt open used to do. The rule that produced
    /// it read "if every shell is busy, use the one you were last in, which is at least the one
    /// you are looking at" — and the one you were last in is precisely the interpreter you were
    /// working in. It is the third outing of the same mistake: `octave-cli-11.3.0` in 0.9.1 and
    /// a capital `Python` in the 0.10 audit were both this, wearing a different hat.
    ///
    /// So when nothing is free a new terminal is opened instead. Run has to run, and a command
    /// typed where it cannot run is worse than a pane nobody asked for. `None` only when a
    /// terminal could not be started at all, and `new_terminal` has already said why.
    fn a_shell_to_type_into(&mut self) -> Option<usize> {
        if self.terminals.is_empty() {
            self.new_terminal();
        }
        let free = self.terminals.iter().position(|w| {
            w.active_tab().child_pid().map(|pid| !dnd::shell_is_busy(pid)).unwrap_or(false)
        });
        if free.is_some() {
            return free;
        }
        let before = self.terminals.len();
        self.new_terminal();
        (self.terminals.len() > before).then(|| self.terminals.len() - 1)
    }

    /// Enter, which means the obvious thing for whichever list is in front of you: open the file,
    /// or move to the branch.
    fn git_open_or_switch(&mut self) {
        let Some(panel) = self.git_panel.as_ref() else { return };
        match panel.tab {
            GitTab::Status => {
                let Some(path) = panel.selected_path() else { return };
                // The panel closes: you asked for the file, and reading it behind a full-window
                // box is not reading it.
                self.git_panel = None;
                self.open_file_in_tab(path);
            }
            GitTab::Branches => {
                let Some(branch) = panel.selected_branch() else { return };
                if branch.current {
                    return;
                }
                let name = branch.name.clone();
                self.git_write(move |root| crate::git::switch(root, &name));
            }
            _ => {}
        }
    }

    /// A click inside the git panel: the tab row switches tabs, and a row of a list the cursor
    /// can be on takes the cursor.
    ///
    /// The body used to do nothing, and the reason written here was that a panel which cannot
    /// write has nothing for a click on a line of diff to mean. That is still true of the diff
    /// and the log — and false of the two lists whose rows have actions attached, where being
    /// able to reach a row only by pressing Down is a worse answer than the one it replaced.
    fn click_git_panel(&mut self, rect: Rect, col: u16, row: u16) {
        let inner = ui::inner_rect(rect);
        if row != inner.y {
            self.click_git_row(inner, row);
            return;
        }
        let header = Rect { height: 1, ..inner };
        let Some(tab) = ui::git_tab_at(self.settings.lang, header, col) else { return };
        let Some(panel) = self.git_panel.as_mut() else { return };
        if panel.tab != tab {
            panel.tab = tab;
            // Same as switching with the keyboard: each tab is a list of its own length, and
            // carrying the offset across would land in the middle of a shorter one.
            panel.scroll = 0;
            panel.selected = 0;
        }
    }

    /// A click on the body. Lands on the row under the pointer, worked out from the same two
    /// numbers the drawing used: where the list starts, and how far it has been scrolled.
    fn click_git_row(&mut self, inner: Rect, row: u16) {
        let Some(panel) = self.git_panel.as_mut() else { return };
        if !panel.tab.picks_a_row() {
            return;
        }
        // The list starts one row below the tabs and runs for as many rows as the renderer last
        // gave it. Below that is the footer, which is not a row of anything.
        let top = inner.y + 1;
        if row < top || (row - top) as usize >= panel.body_rows {
            return;
        }
        let landed = panel.scroll + (row - top) as usize;
        if landed < panel.len() {
            panel.selected = landed;
        }
    }

    /// Moves through the panel, stopping at the end of what it has rather than running off it.
    ///
    /// What moves depends on the tab: a cursor where there are rows to act on, the view where
    /// there is only text. Both stop at the tab's own length, so switching tabs cannot leave
    /// either past the bottom of a shorter list.
    pub fn scroll_git_panel(&mut self, delta: isize) {
        if let Some(panel) = self.git_panel.as_mut() {
            panel.move_by(delta);
        }
    }

    /// Opens a file at a line and column, for a result chosen out of the search list.
    fn open_file_at(&mut self, path: PathBuf, line: usize, col: usize) {
        self.open_file_in_tab(path);
        self.editor_mut().goto_line(line);
        // The column is clamped by the same rule as any other cursor move: a file edited since
        // the search ran may have a shorter line there now, or none.
        let idx = self.pane_editor_index(self.editor_pane_focus);
        let len = self.editors[idx].line_char_len(self.editors[idx].cursor_line);
        self.editors[idx].cursor_col = col.min(len);
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
        // Never drawn yet, so nothing is known about the pane it will land in; the preview's
        // own default stands in until the first frame records a real width.
        let width_px = crate::preview::Preview::picture().render_width();
        let mut preview =
            if paged { crate::preview::Preview::document(1) } else { crate::preview::Preview::picture() };
        // A document opens the way documents were last read. A picture never does: inverting one
        // is a negative rather than a dark mode, so it is per-tab and starts off.
        preview.inverted = paged && self.settings.preview_dark;
        preview.state = crate::preview::start_loading(
            match first {
                Some(page) => crate::preview::Job::Page { path: path.clone(), page, width_px },
                None => crate::preview::Job::Picture {
                    path: path.clone(),
                    box_px: preview.picture_box(),
                    fit: preview.fit,
                },
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

    /// Runs one of the navigation bar's controls. The bar and the keyboard both come through
    /// here, so a button and its key can never drift apart — the bar even writes the key on
    /// itself, and that promise has to hold.
    pub fn preview_control(&mut self, control: ui::NavControl) {
        let idx = self.pane_editor_index(self.editor_pane_focus);
        let Some(preview) = self.editors[idx].preview.as_mut() else { return };
        match control {
            // The figure's own controls go to the session, not to the picture. Handled before
            // everything else because a live figure tab is a preview *and* a figure, and on one
            // the arrows mean the plot rather than the page.
            ui::NavControl::FigLeft => return self.figure_nav_click(KeyCode::Left),
            ui::NavControl::FigRight => return self.figure_nav_click(KeyCode::Right),
            ui::NavControl::FigUp => return self.figure_nav_click(KeyCode::Up),
            ui::NavControl::FigDown => return self.figure_nav_click(KeyCode::Down),
            ui::NavControl::FigReset => return self.figure_nav_click(KeyCode::Char('r')),
            ui::NavControl::FigExport => return self.figure_nav_click(KeyCode::Char('e')),
            ui::NavControl::PageBack => return self.turn_page(-1),
            ui::NavControl::PageForward => return self.turn_page(1),
            ui::NavControl::GoToPage => return self.begin_goto_page(),
            ui::NavControl::ZoomOut if !preview.zoom_by(-1) => return,
            ui::NavControl::ZoomIn if !preview.zoom_by(1) => return,
            ui::NavControl::ZoomOut | ui::NavControl::ZoomIn => {}
            ui::NavControl::FitPage => {
                preview.fit = crate::preview::Fit::Page;
                preview.zoom = 1.0;
            }
            ui::NavControl::FitWidth => {
                preview.fit = crate::preview::Fit::Width;
                preview.zoom = 1.0;
            }
            ui::NavControl::Invert => {
                preview.inverted = !preview.inverted;
                let (dark, kind) = (preview.inverted, preview.kind());
                // A dark mode is a way of reading, so it is remembered and every other document
                // of the same kind follows: turning it on for one PDF and having the next open
                // bright again is the annoyance a dark mode exists to remove. PDFs and markdown
                // keep separate answers — a rendered README and a paper are not read alike — and
                // a picture keeps none at all, its inversion being a negative of that one image.
                match kind {
                    crate::preview::Kind::Document => self.settings.preview_dark = dark,
                    crate::preview::Kind::Markdown => self.settings.preview_dark_markdown = dark,
                    crate::preview::Kind::Picture => {}
                }
                if kind != crate::preview::Kind::Picture {
                    // Written out now rather than at exit, so a session that ends badly still
                    // remembers how its documents were being read.
                    self.settings.save();
                    for editor in self.editors.iter_mut() {
                        if let Some(other) = editor.preview.as_mut() {
                            if other.kind() == kind {
                                other.inverted = dark;
                            }
                        }
                    }
                }
            }
            // Markdown only: the rendered document and the styled text are two ways of reading
            // the same buffer, and which is wanted changes with what is being done to it.
            ui::NavControl::TextMode if preview.kind() == crate::preview::Kind::Markdown => {
                let text_only = !preview.text_only;
                self.settings.preview_markdown_text = text_only;
                self.settings.save();
                for editor in self.editors.iter_mut() {
                    if let Some(other) = editor.preview.as_mut() {
                        if other.kind() == crate::preview::Kind::Markdown {
                            other.set_text_only(text_only);
                        }
                    }
                }
                // The next pass over the buffers makes the other rendering; nothing here does.
                return;
            }
            ui::NavControl::TextMode => return,
        }
        // Everything that reaches here changes how the page must be made, not merely where it
        // is looked at, so it is made again.
        self.rerender_preview(idx);
    }

    /// Moves a width-fitted page up or down within itself, by a fraction of the pane. Cuts a
    /// new band out of the page already in hand rather than asking for the page again.
    /// Zooms the preview the pointer is over, if it is over one. Answers whether it did, so the
    /// wheel can fall back to scrolling when it is not.
    fn zoom_preview_under(&mut self, col: u16, row: u16, areas: &ui::Areas, in_: bool) -> bool {
        let panes = ui::editor_pane_rects(areas.editor, self.split_view, self.settings.split_pct);
        let Some((pane_idx, rect)) = panes.iter().enumerate().find(|(_, r)| within(**r, col, row))
        else {
            return false;
        };
        let (_, content) = ui::split_editor_area(*rect);
        if !within(content, col, row) {
            return false;
        }
        let pane = if pane_idx == 0 { EditorPane::Left } else { EditorPane::Right };
        if self.editors[self.pane_editor_index(pane)].preview.is_none() {
            return false;
        }
        self.editor_pane_focus = pane;
        self.preview_control(if in_ { ui::NavControl::ZoomIn } else { ui::NavControl::ZoomOut });
        true
    }

    /// Drops the window on a rendered page at an absolute position, for a scrollbar dragged
    /// there rather than a key pressed.
    fn set_preview_scroll(&mut self, idx: usize, axis: ui::Axis, position: u32) {
        let Some(preview) = self.editors[idx].preview.as_mut() else { return };
        let (cols, rows) = (preview.area_cols, preview.area_rows);
        let Some(full) = preview.full.as_ref() else { return };
        let (max_x, max_y) = crate::preview::max_scroll(full, cols, rows);
        match axis {
            ui::Axis::Vertical => preview.scroll_px = position.min(max_y),
            ui::Axis::Horizontal => preview.scroll_x = position.min(max_x),
        }
        let window =
            crate::preview::visible_window(full, cols, rows, preview.scroll_x, preview.scroll_px);
        preview.show(window);
        self.editors[idx].mark_scrolled();
    }

    /// Moves the window over a zoomed or width-fitted page. `true` when it handled the gesture,
    /// so a caller can fall through to scrolling text when there is no page to move.
    fn scroll_page(&mut self, dx: isize, dy: isize) -> bool {
        let idx = self.pane_editor_index(self.editor_pane_focus);
        let Some(preview) = self.editors[idx].preview.as_mut() else { return false };
        let (cols, rows) = (preview.area_cols, preview.area_rows);
        let Some(full) = preview.full.as_ref() else { return false };
        let (max_x, max_y) = crate::preview::max_scroll(full, cols, rows);
        if max_x == 0 && max_y == 0 {
            return false;
        }
        // A step is a fraction of the pane, so the gesture feels the same whatever the zoom.
        let (pane_w, pane_h) = crate::preview::pane_pixels(cols, rows);
        let step_x = (pane_w / 6).max(20) as isize;
        let step_y = (pane_h / 6).max(20) as isize;
        let x = (preview.scroll_x as isize + dx * step_x).clamp(0, max_x as isize) as u32;
        let y = (preview.scroll_px as isize + dy * step_y).clamp(0, max_y as isize) as u32;
        if (x, y) == (preview.scroll_x, preview.scroll_px) {
            return true;
        }
        preview.scroll_x = x;
        preview.scroll_px = y;
        let window = crate::preview::visible_window(full, cols, rows, x, y);
        preview.show(window);
        // The bars fade on idleness, and a page being moved is not idle.
        self.editors[idx].mark_scrolled();
        true
    }

    /// Rebuilds a preview at its current zoom, fit and colour. A picture is decoded again, which
    /// costs milliseconds; a document is rasterised again, which costs a fraction of a second —
    /// either way less than keeping a second copy of it around against the chance of a change.
    fn rerender_preview(&mut self, idx: usize) {
        let Some(path) = self.editors[idx].path.clone() else { return };
        let Some(preview) = self.editors[idx].preview.as_ref() else { return };
        // Styled text is not made out here — it is parsed from the buffer a frame at a time —
        // and asking pandoc for a document this tab has been told not to show would be work
        // thrown away.
        if preview.text_view() {
            return;
        }
        let (page, width_px, source) = (preview.page(), preview.render_width(), preview.source.clone());
        let (box_px, fit) = (preview.picture_box(), preview.fit);
        let job = match (source, page) {
            (Some(source), page) => {
                let text = self
                    .editors
                    .iter()
                    .find(|e| e.preview.is_none() && e.path.as_deref() == Some(source.as_path()))
                    .map(|e| e.rope.to_string())
                    .unwrap_or_default();
                crate::preview::Job::Markdown { path: source, text, page: page.unwrap_or(1), width_px }
            }
            (None, Some(page)) => crate::preview::Job::Page { path, page, width_px },
            (None, None) => crate::preview::Job::Picture { path, box_px, fit },
        };
        let started = crate::preview::start_loading(job, self.preview_tx.clone());
        if let Some(preview) = self.editors[idx].preview.as_mut() {
            // What is up stays up while the new one is made, rather than blanking the pane.
            if !matches!(preview.state, crate::preview::State::Ready(_)) {
                preview.state = started;
            }
        }
    }

    /// Jumps a document preview to a page, clamped to the ones it has. Past a known last page
    /// it lands on that instead of asking a rasteriser for something that is not there.
    fn goto_page(&mut self, page: usize) {
        let idx = self.pane_editor_index(self.editor_pane_focus);
        let Some(preview) = self.editors[idx].preview.as_mut() else { return };
        let Some(pages) = preview.pages.as_mut() else { return };
        let wanted = pages.total.map_or(page, |total| page.min(total)).max(1);
        if wanted == pages.current {
            return;
        }
        let delta = wanted as isize - pages.current as isize;
        self.turn_page(delta);
    }

    /// Asks which page to jump to. Reuses the Go-to-line box, which is the same question.
    fn begin_goto_page(&mut self) {
        self.show_goto = true;
        self.goto_input.clear();
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
        let width_px = preview.render_width();
        let job = match rendered_from {
            Some(source) => {
                let text = self
                    .editors
                    .iter()
                    .find(|e| e.preview.is_none() && e.path.as_deref() == Some(source.as_path()))
                    .map(|e| e.rope.to_string())
                    .unwrap_or_default();
                crate::preview::Job::Markdown { path: source, text, page: wanted, width_px }
            }
            None => crate::preview::Job::Page { path, page: wanted, width_px },
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
            // A figure being animated is a file changing on disk ten times a second, and the
            // figure poll is already reading it. Left to also fire here, this asked for the same
            // picture a second time and — worse — blanked the pane to do it, on its own 700 ms
            // rhythm: a flicker with a beat of its own, twice a second, laid over a smooth one.
            // The timestamp is deliberately not recorded when we skip, so an edit that lands
            // while a read is in flight is still noticed once that read is done.
            if self.editors[i].preview.as_ref().is_some_and(|p| p.reading()) {
                continue;
            }
            self.editors[i].disk_mtime = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());
            let width_px =
                self.editors[i].preview.as_ref().map(|p| p.render_width()).unwrap_or_default();
            let (box_px, fit) = self.editors[i]
                .preview
                .as_ref()
                .map(|p| (p.picture_box(), p.fit))
                .unwrap_or(((0, 0), crate::preview::Fit::Page));
            if let Some(preview) = self.editors[i].preview.as_mut() {
                let started = crate::preview::start_loading(
                    match page {
                        Some(page) => crate::preview::Job::Page { path: path.clone(), page, width_px },
                        None => crate::preview::Job::Picture { path: path.clone(), box_px, fit },
                    },
                    self.preview_tx.clone(),
                );
                // What is up stays up while the new one is made — the same rule the rest of the
                // preview paths follow. A typeset page being rebuilt should not take the old one
                // off the screen for half a second.
                match preview.state {
                    crate::preview::State::Ready(_) => preview.reloading = true,
                    _ => preview.state = started,
                }
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
        // Nothing open anywhere is a state of its own, not a half that needs filling: handing it
        // a buffer here would put back the tab that was just closed.
        if !self.any_tabs_open() {
            self.split_view = false;
            return;
        }
        for pane in [EditorPane::Left, EditorPane::Right] {
            if self.tabs[pane.index()].is_empty() {
                // One half of an open split with nothing in it does get a fresh empty buffer —
                // the same one a brand-new window starts with. The half is there because you
                // asked for it, and an empty frame beside a full one reads as a bug.
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

    /// The text a match covers, for a replacement that wants to quote parts of it back.
    fn matched_text(&self, m: (usize, usize)) -> String {
        self.editor().rope.slice(m.0..m.1).to_string()
    }

    fn replace_current(&mut self) {
        let Some(m) = self.find.as_ref().and_then(|f| f.current_match()) else { return };
        let matched = self.matched_text(m);
        let Some(replace) = self.find.as_ref().map(|f| f.replacement_for(&matched)) else { return };
        self.editor_mut().replace_char_range(m.0, m.1, &replace);
        // Matches shifted; recompute and land on the next one from the edit point.
        self.recompute_find();
    }

    fn replace_all(&mut self) {
        let Some(f) = self.find.as_ref() else { return };
        if f.query.is_empty() || f.matches.is_empty() {
            return;
        }
        // Replace from the last match backwards so earlier char indices stay valid. Each
        // replacement is worked out from the text it covers, since a pattern's groups differ
        // from match to match.
        let matches: Vec<(usize, usize)> = f.matches.clone();
        let count = matches.len();
        for &(s, e) in matches.iter().rev() {
            let matched = self.matched_text((s, e));
            let Some(replace) = self.find.as_ref().map(|f| f.replacement_for(&matched)) else { break };
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
            // The two readings of the query. Both recompute straight away, so the match count
            // answers before anything else is typed — which is how you find out what they do.
            //
            // U and N because they are what is left: every other letter is spoken for somewhere
            // (Ctrl+D closes a tab, Ctrl+T is the terminal), and h/i/m are backspace, tab and
            // return by the time a terminal has finished with them. Both are printed in the box
            // for exactly that reason — they are not going to be guessed.
            KeyCode::Char('u') if ctrl => {
                if let Some(f) = self.find.as_mut() {
                    f.case_sensitive = !f.case_sensitive;
                }
                self.recompute_find();
            }
            KeyCode::Char('n') if ctrl => {
                if let Some(f) = self.find.as_mut() {
                    f.regex = !f.regex;
                }
                self.recompute_find();
            }
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
                if let Ok(number) = self.goto_input.trim().parse::<usize>() {
                    if number > 0 {
                        // The same question means a page on a document and a line in a buffer.
                        if self.editor().preview.as_ref().is_some_and(|p| p.pages.is_some()) {
                            self.goto_page(number);
                        } else {
                            self.editor_mut().goto_line(number);
                        }
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
                shortcut: it.shortcut.map(|s| i18n::shortcut_label(lang, s).to_string()),
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
        let mut file_line = None;
        let mut inspect = None;
        if let Some(action) = self.picker.as_ref().and_then(|p| p.selected_action()) {
            match action {
                crate::picker::PickAction::Command(a) => cmd = Some(*a),
                crate::picker::PickAction::OpenFile(p) => file = Some(p.clone()),
                crate::picker::PickAction::VenvDir(p) => venv_dir = Some(p.clone()),
                crate::picker::PickAction::Workspace(name) => workspace = Some(name.clone()),
                crate::picker::PickAction::FileLine(p, line, col) => {
                    file_line = Some((p.clone(), *line, *col))
                }
                crate::picker::PickAction::Inspect(name) => inspect = Some(name.clone()),
            }
        }
        if let Some(name) = inspect {
            self.picker = None;
            self.inspect(name);
            return;
        }
        if let Some((path, line, col)) = file_line {
            self.picker = None;
            self.open_file_at(path, line, col);
            return;
        }
        if let Some(name) = workspace {
            // Which of the two workspace pickers is open decides what Enter means.
            if self.picker.as_ref().map(|p| p.kind) == Some(crate::picker::PickerKind::WorkspaceDelete) {
                self.delete_workspace(&name);
            } else {
                self.picker = None;
                let (found, shadowed) = crate::workspace::resolve(&name, &self.workspace_shape());
                match found {
                    Some(ws) => {
                        self.apply_workspace(ws);
                        if let Some(built_in) = shadowed {
                            self.status_message =
                                i18n::msg_workspace_shadows(self.settings.lang, built_in);
                        }
                    }
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
        // Closing the only tab leaves nothing open, and that is the whole point of it. It used
        // to put a fresh untitled buffer in its place, which made the last tab the one tab you
        // could not close: you asked for it to go and something identical took its seat. What is
        // left is an empty frame that says how to open a file — see `any_tabs_open`.
        if self.editors.len() <= 1 {
            self.nothing_open();
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
        // Two can go at once — a file and the preview that was a view of it — so the list can
        // empty here even though it had more than one in it a moment ago.
        if self.editors.is_empty() {
            self.nothing_open();
            return;
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
        // Changing folder steps *out* of the workspace rather than dragging it along. A saved
        // workspace is the set-up of its own project, and staying attached meant exit wrote this
        // folder's files and shells over it — silently, so the workspace was gone before anyone
        // could notice. Reopening it is one trip through the Workspace menu; getting the
        // overwritten one back was impossible. The built-in layout is exempt: it belongs to no
        // project, so it travels.
        let lang = self.settings.lang;
        let path = self.root.display().to_string();
        let kept = workspace_after_root_change(self.active_workspace.as_deref());
        let left = if kept.is_none() { self.active_workspace.take() } else { None };
        self.active_workspace = kept;
        self.status_message = match left {
            Some(name) => {
                self.settings.last_workspace = None;
                i18n::msg_workspace_left(lang, &name, &path)
            }
            None => i18n::msg_project_folder(lang, &path),
        };
    }

    /// Takes the background over, or hands it back. Written out at once rather than at exit:
    /// this is reached for when the screen has become unreadable, and having to do it again
    /// after every session would be its own small misery.
    fn toggle_opaque_background(&mut self) {
        self.settings.opaque_background = !self.settings.opaque_background;
        self.settings.save();
        self.status_message =
            i18n::msg_opaque_background(self.settings.lang, self.settings.opaque_background);
    }

    /// Plots as tabs, or plots in the interpreter's own windows.
    ///
    /// The shells already running keep the setting they were started with: their interpreter
    /// read it once, at startup, and a figure window cannot be talked back into a picture. So
    /// the message says *next session* rather than letting the user wonder why the plot they
    /// just drew ignored the menu.
    fn toggle_plots_in_tabs(&mut self) {
        // Nothing to choose between on a machine with no screen: the interpreter's own window
        // has nowhere to open, so the setting is left alone and the reason is said out loud.
        if !crate::wsnap::can_open_a_window() {
            self.status_message = i18n::msg_plots_in_tabs(self.settings.lang, true, false);
            return;
        }
        self.settings.plots_in_tabs = !self.settings.plots_in_tabs;
        crate::wsnap::set_plots_in_tabs(self.settings.plots_in_tabs);
        self.settings.save();
        self.status_message = i18n::msg_plots_in_tabs(
            self.settings.lang,
            self.settings.plots_in_tabs,
            crate::wsnap::can_open_a_window(),
        );
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

    /// Open the variables panel as a window of its own, in whatever layout is up.
    ///
    /// It used to exist only inside the two built-in presets, which meant a saved workspace of
    /// your own — the thing anybody who uses CleeCode for a while is actually in — had no panel
    /// and no way to ask for one. The feature was reachable only by abandoning your layout.
    ///
    /// Already-open ones are focused rather than duplicated: the panel follows whichever session
    /// last ran something, so a second one would show the same thing beside the first.
    pub fn show_workspace_panel(&mut self) {
        let lang = self.settings.lang;
        let Some(command) = self.workspace_shape().workspace_view else {
            return;
        };
        if let Some(idx) = self
            .terminals
            .iter()
            .position(|w| w.tabs.iter().any(|t| t.name.as_deref() == Some("workspace")))
        {
            self.active_terminal = idx;
            self.settings.show_terminal = true;
            self.focus = Focus::Terminal;
            self.status_message = i18n::msg_workspace_panel(lang);
            return;
        }
        match crate::terminal_panel::TerminalPanel::with_startup(24, 80, &self.root, Some(&command))
        {
            Ok(mut panel) => {
                panel.name = Some("workspace".to_string());
                panel.startup_command = Some(command);
                self.terminals.push(crate::terminal_panel::TerminalWindow {
                    tabs: vec![panel],
                    active: 0,
                    weight: crate::terminal_panel::TERMINAL_WEIGHT_DEFAULT,
                });
                self.active_terminal = self.terminals.len() - 1;
                self.settings.show_terminal = true;
                self.status_message = i18n::msg_workspace_panel(lang);
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
        if crate::workspace::is_built_in(&name) {
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
                // A shell spawned for this tab is handed the command; a reused one is told to
                // run it. Both end up in the same place — held until the shell is at a prompt,
                // then typed onto an empty line.
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
            // The built-ins go on top, in the order they are declared, and only where they are
            // not already a file of the user's own — otherwise the same name would be offered
            // twice with two different meanings.
            let shape = self.workspace_shape();
            for name in crate::workspace::BUILT_INS.iter().rev() {
                if saved.iter().any(|w| crate::workspace::slug(&w.name) == crate::workspace::slug(name)) {
                    continue;
                }
                if let Some(ws) = crate::workspace::built_in(name, &shape) {
                    saved.insert(0, ws);
                }
            }
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
        // Indexed through `get`, because with every tab closed there is no buffer to index and
        // `pane_editor_index` answers 0 for an empty list — which is the honest answer to "which
        // tab is active" and a panic to `editors[0]`. Run with no tab open did exactly that:
        // caught by the shield, so the session survived with a line in the status bar, but ▶ on
        // an empty window is an ordinary thing to press and it must simply say there is nothing
        // to run. `editor()` has taken this care since the last tab became closeable.
        let Some(editor) = self.editors.get(idx) else { return false };
        if let Some(preview) = editor.preview.as_ref().filter(|p| p.refreshable()) {
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
            } else if path.is_some() {
                self.reread_preview(idx, page);
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

    /// Reads a preview tab's file again, keeping the zoom and fit it is being shown at.
    ///
    /// What ▶ Refresh does by hand, and what a figure does by itself when the interpreter draws
    /// over it: the picture on screen was made from a file that has since changed, and the tab
    /// has to be told, because nothing about a decoded image knows its source moved on.
    fn reread_preview(&mut self, idx: usize, page: Option<usize>) {
        let Some(path) = self.editors[idx].path.clone() else { return };
        let width_px = self.editors[idx].preview.as_ref().map(|p| p.render_width()).unwrap_or_default();
        let (box_px, fit) = self.editors[idx]
            .preview
            .as_ref()
            .map(|p| (p.picture_box(), p.fit))
            .unwrap_or(((0, 0), crate::preview::Fit::Page));
        if let Some(preview) = self.editors[idx].preview.as_mut() {
            let started = crate::preview::start_loading(
                match page {
                    Some(page) => crate::preview::Job::Page { path, page, width_px },
                    None => crate::preview::Job::Picture { path, box_px, fit },
                },
                self.preview_tx.clone(),
            );
            // A tab with a picture in it keeps that picture while the next one is decoded. This
            // is the path a figure being animated comes down — the file on disk has changed and
            // is read again, ten times a second — and blanking it to `Loading` in between is a
            // pane that spends half its life empty. `rerender_preview` has said the same thing
            // for its own reasons since zoom existed.
            match preview.state {
                crate::preview::State::Ready(_) => preview.reloading = true,
                _ => preview.state = started,
            }
        }
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
        let mut preview = crate::preview::Preview::rendered(source.clone());
        preview.inverted = self.settings.preview_dark_markdown;
        preview.set_text_only(self.settings.preview_markdown_text);
        let idx = self.adopt_editor(Editor::preview(source, preview));
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
            let text_only = self.editors[i].preview.as_ref().is_some_and(|p| p.text_only);
            if !as_document || failed || text_only {
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
            let width_px =
                self.editors[i].preview.as_ref().map(|p| p.render_width()).unwrap_or_default();
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
            let job = crate::preview::Job::Markdown { path, text, page, width_px };
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

    // ---- Running a piece of the file ------------------------------------------------------

    /// Sends the selection — or the `%%` cell the cursor is in — to a live interpreter.
    ///
    /// This is the thing that separates an editor with a terminal in it from somewhere you
    /// actually work: the session keeps its variables, so a script is built up a piece at a time
    /// with the data already loaded, instead of being re-run from the top after every change.
    pub fn run_selection(&mut self) {
        let lang = self.settings.lang;
        let (path, text, selection, cursor_line) = {
            let editor = self.editor();
            (editor.path.clone(), editor.rope.to_string(), editor.selection_range(), editor.cursor_line)
        };
        let Some(path) = path else {
            self.status_message = i18n::msg_run_piece_unsaved(lang);
            return;
        };
        let Some(language) = crate::session::Language::of_path(&path) else {
            self.status_message = i18n::msg_run_piece_no_language(lang, &file_ext(&path));
            return;
        };

        // A selection is an explicit answer to "which piece"; the cell is what to do when nobody
        // said. Whole lines either way — a fragment of one is not a statement, and sending half
        // an expression to a prompt produces a syntax error about code the user never wrote.
        let lines: Vec<&str> = text.lines().collect();
        let (from, to, what) = match selection {
            Some(((start_line, _), (end_line, _))) => {
                (start_line, (end_line + 1).min(lines.len()), crate::session::Piece::Selection)
            }
            None => {
                let (from, to) = crate::session::cell_at(&lines, cursor_line);
                (from, to, crate::session::Piece::Cell)
            }
        };
        let piece = lines[from.min(lines.len())..to.min(lines.len())].join("\n");
        if piece.trim().is_empty() {
            self.status_message = i18n::msg_run_piece_empty(lang);
            return;
        }

        let Some(scratch) = self.write_scratch(language, &piece) else {
            self.status_message = i18n::msg_run_piece_no_scratch(lang);
            return;
        };
        let command = language.run_file(&scratch.to_string_lossy());

        // The interpreter that is already open, or none. Only the on-screen tab of each window
        // counts: running something in a hidden tab would be invisible.
        match self.send_to_session(language, &command) {
            Some(idx) => {
                self.status_message =
                    i18n::msg_run_piece(lang, what, language.label(), to - from, idx);
            }
            // Nothing to send it to. Running the scratch file the ordinary way starts an
            // interpreter, which is the same answer the Run button gives and leaves a session
            // open for the next piece — so pressing it twice does what it looks like it should.
            None => {
                self.settings.show_terminal = true;
                self.run_path(&scratch);
            }
        }
    }

    /// Types one line at the prompt of a live interpreter, and says which terminal took it.
    ///
    /// Only the on-screen tab of each window is a candidate: sending something into a hidden tab
    /// would be invisible, and the whole point of talking to a session that is already open is
    /// that you can see what it says back.
    fn send_to_session(&mut self, language: crate::session::Language, command: &str) -> Option<usize> {
        let pids: Vec<Option<u32>> =
            self.terminals.iter().map(|w| w.active_tab().child_pid()).collect();
        let idx = dnd::shell_running(language, &pids)?;
        if let Some(term) = self.window_tab_mut(idx) {
            term.type_line(command);
        }
        self.settings.show_terminal = true;
        Some(idx)
    }

    /// Writes a piece of a buffer where an interpreter can be pointed at it.
    ///
    /// One file per buffer rather than one per run, so a session's history stays readable —
    /// `run('/tmp/…/plot.m')` twice means the same file was run twice, which is what happened.
    /// The extension matters: Octave refuses to `run` anything that is not `.m`.
    fn write_scratch(&self, language: crate::session::Language, piece: &str) -> Option<PathBuf> {
        let dir = std::env::temp_dir().join(format!("cleecode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok()?;
        let stem = self
            .editor()
            .path
            .as_deref()
            .and_then(|p| p.file_stem().and_then(|s| s.to_str()))
            .unwrap_or("piece");
        // The name is the user's, sanitised: it shows up in their transcript and in any
        // traceback, and `cell_3f9a.m` tells them nothing about which file it came from.
        let stem: String = stem.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
        let file = dir.join(format!("{stem}.{}", language.scratch_extension()));
        std::fs::write(&file, format!("{}\n", piece.trim_end())).ok()?;
        Some(file)
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
        // A file of a language whose prompt is already open in one of the panes: hand the script
        // to *that* session — `run(...)` in Octave, `exec(open(...).read())` in Python, both of
        // which run in the prompt's own namespace. Starting a second interpreter would be slow
        // and would lose everything the session is holding.
        //
        // Octave has worked this way since 0.9. Python deliberately did not, and the reason
        // written here was that a Python REPL open in a side terminal while you edit a web
        // application is not where `manage.py` should run. That reasoning describes a real
        // session and misses the one CleeCode ships a preset for. In `clee -w pylab` the prompt
        // *is* where the work is happening, and Run there did the only thing worse than running
        // in the wrong place: it ran in a fresh shell that exited immediately, so the variables
        // were gone before the panel could see them and the figures were drawn by a process that
        // no longer existed. Three symptoms — no plot, no variables, an empty panel — one cause,
        // and none of them said so.
        //
        // The narrow reading is still available and is the one that asks for it: Ctrl+Shift+X
        // sends a cell or a selection and nothing else.
        let program = template.split_once(' ').map(|(p, _)| p).unwrap_or(&template);
        // Absolute, because the session's own working directory is not necessarily the one the
        // project was opened in — a `cd` at the prompt is an ordinary thing to have typed.
        let named = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        for language in [crate::session::Language::Octave, crate::session::Language::Python] {
            if !self.settings.run_in_session || !language.is_interpreter(program) {
                continue;
            }
            // Only the on-screen tab of each window is a candidate: running a script in a hidden
            // tab would be invisible and confusing.
            let pids: Vec<Option<u32>> = self.terminals.iter().map(|w| w.active_tab().child_pid()).collect();
            if let Some(idx) = dnd::shell_running(language, &pids) {
                let command = language.run_file(&named.to_string_lossy());
                // The figures this file's last run left behind, closed before it runs again.
                //
                // Both languages hand out the next free figure number, so a script that does not
                // name its figures — `plt.subplots()`, or a bare `figure()` — draws a new one
                // every time it runs: two plots became four tabs on the second run and six on
                // the third, all showing the same two pictures. Closing the previous set frees
                // the numbers, so the rerun draws into the tabs that are already open. Only that
                // set: a plot made by hand at the prompt belongs to nobody's run and stays.
                let previous = self.run_figures.remove(&named).unwrap_or_default();
                if !previous.is_empty() {
                    let close = language.close_figures(&previous);
                    if let Some(term) = self.window_tab_mut(idx) {
                        term.type_line(&close);
                    }
                }
                if let Some(term) = self.window_tab_mut(idx) {
                    term.type_line(&command);
                }
                // What the session is holding *now* is the baseline: everything that appears
                // between here and the prompt coming back is this run's doing. Read after the
                // close above was typed rather than before — the numbers just closed are gone,
                // and counting them as somebody else's would make them immortal.
                let before = crate::wsnap::open_figures(&crate::wsnap::snapshot_dir(), language.snapshot_lang())
                    .into_iter()
                    .filter(|n| !previous.contains(n))
                    .collect();
                self.run_watch = Some(RunWatch {
                    file: named.clone(),
                    language,
                    terminal: idx,
                    before,
                    opened: Vec::new(),
                    started: std::time::Instant::now(),
                    busy_seen: false,
                    looked: std::time::Instant::now(),
                    settled: None,
                });
                self.active_terminal = idx;
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
        // Into a shell, and a new one if every pane is an interpreter's prompt. See
        // `a_shell_to_type_into`: typing `python3 hello.py` at a `>>>` is a NameError with the
        // user's name on it, and it is what this did.
        let Some(idx) = self.a_shell_to_type_into() else { return };
        if let Some(term) = self.window_tab_mut(idx) {
            term.type_line(&command);
        }
        self.active_terminal = idx;
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
        let quoted = shell_quote(&venv_bin.to_string_lossy());
        format!("{quoted} {rest}")
    }

    /// Which buffer a pane is showing. The toolbar button describes the file under it, so each
    /// pane asks about its own.
    /// Which buffer a pane is showing, clamped to one that exists.
    ///
    /// Clamped for the same reason `active_editor_index` is: a pane's index is written from a
    /// dozen places — opening, closing, splitting, merging, restoring a workspace — and about
    /// thirty callers index straight into `editors` with what this returns. One of those writes
    /// lagging by a frame would be a panic, and a panic here takes the window down with every
    /// shell running in it. Showing the last tab for one frame is the cheaper wrong answer.
    pub fn pane_editor_index(&self, pane: EditorPane) -> usize {
        let last = self.editors.len().saturating_sub(1);
        match pane {
            EditorPane::Left => self.active_editor.min(last),
            EditorPane::Right => self.active_editor_right.min(last),
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
            self.run_session_target(&menu.ext),
            self.settings.lang,
        )
    }

    /// Whether a file of this extension could run in a session, and whether one is open.
    ///
    /// The process table is read here rather than in `run_rows`, which is a pure layout function
    /// with tests that would otherwise need a running interpreter. It is read once, when the
    /// drop-down opens, which is the same cost Run itself pays.
    fn run_session_target(&self, ext: &str) -> SessionTarget {
        let Some(language) = crate::session::Language::of_path(std::path::Path::new(&format!("x.{ext}")))
        else {
            return SessionTarget::default();
        };
        let pids: Vec<Option<u32>> =
            self.terminals.iter().map(|w| w.active_tab().child_pid()).collect();
        SessionTarget {
            possible: true,
            open: dnd::shell_running(language, &pids).is_some(),
            wanted: self.settings.run_in_session,
        }
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
            RunRowAction::UseSession => {
                self.settings.run_in_session = true;
                self.status_message = i18n::msg_run_session_chosen(self.settings.lang).to_string();
                let _ = self.settings.save();
            }
            RunRowAction::SelectVenv(venv) => {
                // Choosing an interpreter is choosing *not* to use the session: they are two
                // answers to the same question, and a list where both look chosen answers
                // neither.
                self.settings.run_in_session = false;
                self.select_venv(venv);
            }
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

    /// Whether a box is up that owns the keyboard.
    ///
    /// The gate in front of [`Self::dispatch_modal_key`], and the same question a paste has to
    /// ask: see the comment at its one use in [`Self::handle_key`] for why they are one function
    /// and not two lists.
    fn a_modal_owns_the_keyboard(&self) -> bool {
        self.context_menu.is_some()
            || self.unsaved_prompt.is_some()
            || self.show_save_as
            || self.run_menu.is_some()
            || self.venv_register.is_some()
            || self.run_command_edit.is_some()
            || self.picker.is_some()
            || self.find.is_some()
            || self.show_goto
            || self.show_search
            || self.show_new_entry
            || self.show_delete_confirm
            || self.show_rename
            || self.show_terminal_rename
            || self.show_workspace_save
            || self.git_panel.is_some()
            || self.inspector.is_some()
            || self.manual.is_some()
            || self.resize_mode
            || self.show_settings
            || self.menu.active
    }

    /// Hands the key to whichever box is up. In the order the boxes can appear over one another,
    /// which is why it is a chain and not a match.
    fn dispatch_modal_key(&mut self, key: KeyEvent) {
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
        if self.show_search {
            self.handle_search_key(key);
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
        if self.git_panel.is_some() {
            self.handle_git_panel_key(key);
            return;
        }
        if self.inspector.is_some() {
            self.handle_inspector_key(key);
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
        // Every box that owns the keyboard, behind one question.
        //
        // The chain is the dispatch and this is the gate in front of it, rather than the chain
        // being both. Two things need this answer — a key goes to the box, and so does a
        // *paste*, which is keys arriving by another route — and while they worked it out
        // separately the paste knew about four boxes out of twenty: pasting a commit message
        // into the git panel typed it into the file behind the panel instead, silently, with the
        // box still sitting there asking for one.
        //
        // Making it the gate is what keeps them in step from here on. A box added to the chain
        // and not to the predicate never gets a key at all; one added to the predicate and not
        // to the chain swallows every key. Either way it is wrong the first time it is opened,
        // rather than wrong only for a paste, which is the failure that lasted.
        if self.a_modal_owns_the_keyboard() {
            self.dispatch_modal_key(key);
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
            // eXecute this much of it. R runs the file, X runs the piece — next to each other in
            // meaning, and X is one of the letters still free. Not Shift+Enter, which is what
            // every notebook uses and what a terminal cannot deliver: the encoding has had no
            // room for the Shift since VT100, so it would work in two emulators and silently do
            // nothing in the rest. Ctrl+X is still cut; this is Ctrl+Shift+X.
            KeyCode::Char('x') | KeyCode::Char('X') if ctrl && shift => {
                self.run_selection();
                return;
            }
            // Put a breakpoint on this line, or take it off.
            KeyCode::Char('p') | KeyCode::Char('P') if ctrl && shift => {
                self.toggle_breakpoint();
                return;
            }
            // Inspect: what a variable actually contains, a screenful at a time.
            KeyCode::Char('i') | KeyCode::Char('I') if ctrl && shift => {
                self.open_inspector_picker();
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
            // J and L, next to each other, for a pair that is used as one: go and come back.
            // Neither is a letter anyone would guess, and neither had a better claim — the
            // mnemonic keys are spent, and F12 is not available here for the reason no feature
            // in CleeCode uses a function key.
            KeyCode::Char('j') | KeyCode::Char('J') if ctrl && shift => {
                self.lsp_go_to_definition();
                return;
            }
            KeyCode::Char('l') | KeyCode::Char('L') if ctrl && shift => {
                self.lsp_jump_back();
                return;
            }
            KeyCode::Char('d') | KeyCode::Char('D') if ctrl && shift => {
                self.toggle_git_panel();
                return;
            }
            // H rather than the F that VS Code uses for this: Ctrl+Shift+F already folds, and a
            // key that does two things is a key that does the wrong one.
            KeyCode::Char('h') | KeyCode::Char('H') if ctrl && shift => {
                self.begin_project_search();
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
            i18n::menu_title(lang, d.title_key)
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
            MenuAction::ToggleOpaqueBackground => self.toggle_opaque_background(),
            MenuAction::TogglePlotsInTabs => self.toggle_plots_in_tabs(),
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
            MenuAction::RunSelection => self.run_selection(),
            MenuAction::ToggleBreakpoint => self.toggle_breakpoint(),
            MenuAction::ShowWorkspacePanel => self.show_workspace_panel(),
            MenuAction::InspectVariable => self.open_inspector_picker(),
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
            MenuAction::SearchProject => self.begin_project_search(),
            MenuAction::ToggleGitPanel => self.toggle_git_panel(),
            MenuAction::GitStatus => self.open_git_panel_on(GitTab::Status),
            MenuAction::GitChanges => self.open_git_panel_on(GitTab::Diff),
            MenuAction::GitHistory => self.open_git_panel_on(GitTab::Graph),
            MenuAction::GitBranches => self.open_git_panel_on(GitTab::Branches),
            MenuAction::GitStashes => self.open_git_panel_on(GitTab::Stashes),
            MenuAction::GitFetch => self.git_remote(crate::git::Remote::Fetch),
            MenuAction::GitPull => self.git_remote(crate::git::Remote::Pull),
            MenuAction::GitPush => self.git_remote(crate::git::Remote::Push),
            MenuAction::GitStageFile => self.git_file_action(true),
            MenuAction::GitUnstageFile => self.git_file_action(false),
            MenuAction::GitFileDiff => self.git_show_file_in_panel(),
            MenuAction::GitDiscardFile => self.git_ask_to_discard_the_tree_selection(),
            MenuAction::GitCommit => self.git_ask_for_a_message_in_the_panel(),
            MenuAction::GoToDefinition => self.lsp_go_to_definition(),
            MenuAction::JumpBack => self.lsp_jump_back(),
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
                // A preview measures in the rendered page's pixels, not in lines: the bar shows
                // where the window sits on a page that zoom may have made larger than the pane.
                if let Some(metrics) = self.preview_scroll_view(idx, axis) {
                    return Some(metrics);
                }
                let (_, height, width) = ui::editor_viewport(self, idx, rect);
                ui::editor_scroll_metrics(self, idx, axis, height, width)
            }
            ScrollbarId::Terminal(i) => ui::terminal_scroll_metrics(self.window_tab(i)?),
        }
    }

    /// What a preview's scrollbar describes: the rendered page, the window on it, and how much
    /// of it fits. `None` when the tab is not a preview, or the page fits whole.
    pub fn preview_scroll_view(&self, idx: usize, axis: ui::Axis) -> Option<(usize, usize, usize)> {
        let preview = self.editors.get(idx)?.preview.as_ref()?;
        let full = preview.full.as_ref()?;
        let (pane_w, pane_h) = crate::preview::pane_pixels(preview.area_cols, preview.area_rows);
        let (total, position, viewport) = match axis {
            ui::Axis::Vertical => (full.height(), preview.scroll_px, pane_h),
            ui::Axis::Horizontal => (full.width(), preview.scroll_x, pane_w),
        };
        // A page fitted to the width lands within a few pixels of the pane, not exactly on it:
        // the resolution is worked back from a pixel count and rounded, so the raster overshoots
        // or undershoots by a hair. Comparing exactly made the bar appear, vanish and reappear
        // as pages were re-made. A page has to be more than one cell too big to be worth a bar.
        let slack = crate::preview::cell_size().map_or(8, |(w, h)| {
            u32::from(if axis == ui::Axis::Horizontal { w } else { h })
        });
        (total > viewport.saturating_add(slack))
            .then_some((total as usize, position as usize, viewport as usize))
    }

    /// Whether a scrollbar should show itself in full rather than as a hint: the pointer is
    /// resting on it, or it is the one being dragged. Both are the moment its arrows and groove
    /// have to be aimable instead of merely suggestive.
    pub fn scrollbar_engaged(&self, id: ScrollbarId, frame: Rect, axis: ui::Axis) -> bool {
        if self.dragging == Some(DragTarget::Scrollbar(id)) {
            return true;
        }
        let Some((col, row)) = self.pointer else { return false };
        ui::scrollbar_reveal_zone(ui::inner_rect(frame), axis).is_some_and(|zone| within(zone, col, row))
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
            let Some(strip) = ui::scrollbar_strip(self.scrollbar_box(frame, axis), axis) else { continue };
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

    /// The box a frame's scrollbars ride, which on a preview stops short of its navigation bar.
    fn scrollbar_box(&self, frame: Rect, _axis: ui::Axis) -> Rect {
        let panes = [EditorPane::Left, EditorPane::Right];
        for pane in panes {
            let idx = self.pane_editor_index(pane);
            if self.editors.get(idx).is_some_and(|e| e.preview.is_some()) {
                return ui::scrollbar_area(self, idx, frame);
            }
        }
        ui::inner_rect(frame)
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
        let Some(strip) = ui::scrollbar_strip(self.scrollbar_box(frame, axis), axis) else { return };
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
                if self.editors[idx].preview.is_some() {
                    self.set_preview_scroll(idx, axis, position as u32);
                    return;
                }
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
                self.settings_changed();
            }
            KeyCode::Left => {
                self.settings.adjust(self.settings_selected, -1);
                self.settings_changed();
            }
            KeyCode::Right => {
                self.settings.adjust(self.settings_selected, 1);
                self.settings_changed();
            }
            _ => {}
        }
    }

    /// What a row of the settings modal does beyond changing the struct.
    ///
    /// Two things the modal used to leave undone. The plot destination lives in a second place —
    /// an atomic the shells are started from, since a shell is spawned off the main thread and
    /// cannot reach into the app — so a row that only wrote the struct was a switch the next
    /// session ignored. And nothing here was written to disk until a clean quit, so a change
    /// made in the modal and a terminal closed by its own X button cancelled each other out.
    /// The menu's own toggle has always done both; these are the same settings.
    fn settings_changed(&mut self) {
        crate::wsnap::set_plots_in_tabs(self.settings.plots_in_tabs);
        self.settings.save();
        self.editor_mut().syntax_dirty = true;
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
        // With every tab closed there is no buffer to type into. Said here rather than left to
        // `editor_mut` finding the scratch: keystrokes disappearing into a buffer nobody can see
        // is the kind of thing that works for a year and then explains a lost paragraph.
        if !self.any_tabs_open() {
            return;
        }
        // The completion popup goes first, and it is the one overlay in this file that does not
        // swallow the keyboard. Every other one early-returns out of `handle_key` until it is
        // dismissed; this one claims five keys — two to walk the list, two to accept, one to
        // dismiss — and lets every other key fall through to the editor below, re-filtering
        // afterwards against what the edit left behind. A popup you have to close before you can
        // carry on typing interrupts the typing it was there to help.
        if self.completion_key(key) {
            return;
        }
        // A preview tab has no text to move a cursor through, so the plain arrows are free and
        // mean the only thing they could mean here: the page before and the page after. No
        // chord had to be found for it, which on a keyboard this crowded is worth something.
        // A preview tab holds no text, so plain keys are free here and mean the one thing they
        // could mean. Every one of them is also a button on the bar, which writes the key beside
        // the label so neither has to be discovered twice.
        // A figure gets the keys first: on a plot, + and the arrows mean "draw it again, closer"
        // rather than "magnify what is already drawn", and the difference is whether the axis
        // labels still describe what is on screen.
        if self.editor().preview.is_some() && self.figure_key(key) {
            return;
        }
        if self.editor().preview.is_some() && key.modifiers.is_empty() {
            let paged = self.editor().preview.as_ref().is_some_and(|p| p.pages.is_some());
            let kind = self.editor().preview.as_ref().map(|p| p.kind());
            // Styled text has no page to zoom, fit or darken, so those keys stay unbound over it
            // — the bar does not offer them there either.
            let text_view = self.editor().preview.as_ref().is_some_and(|p| p.text_view());
            let idx = self.pane_editor_index(self.editor_pane_focus);
            let scroll_limit = self.rendered_len(idx).map(|len| len.saturating_sub(1));
            let control = match key.code {
                KeyCode::Left if paged => Some(ui::NavControl::PageBack),
                KeyCode::Right if paged => Some(ui::NavControl::PageForward),
                KeyCode::Up if paged => Some(ui::NavControl::PageBack),
                KeyCode::Down if paged => Some(ui::NavControl::PageForward),
                KeyCode::Char('g') if paged => Some(ui::NavControl::GoToPage),
                KeyCode::Char('-') | KeyCode::Char('_') if !text_view => Some(ui::NavControl::ZoomOut),
                KeyCode::Char('+') | KeyCode::Char('=') if !text_view => Some(ui::NavControl::ZoomIn),
                KeyCode::Char('f') if !text_view => Some(ui::NavControl::FitPage),
                KeyCode::Char('w') if !text_view => Some(ui::NavControl::FitWidth),
                // `d` for a document's dark mode, `i` for a picture's negative: two different
                // things, so two keys rather than one word stretched over both.
                KeyCode::Char('d') if !text_view && kind != Some(crate::preview::Kind::Picture) => {
                    Some(ui::NavControl::Invert)
                }
                KeyCode::Char('i') if kind == Some(crate::preview::Kind::Picture) => {
                    Some(ui::NavControl::Invert)
                }
                KeyCode::Char('t') if kind == Some(crate::preview::Kind::Markdown) => {
                    Some(ui::NavControl::TextMode)
                }
                _ => None,
            };
            if let Some(control) = control {
                self.preview_control(control);
                return;
            }
            // What is left of the arrows scrolls a rendered markdown view, which is one long
            // page rather than a set of them.
            let scroll = |app: &mut Self, delta: isize| {
                // A width-fitted page scrolls inside itself; a rendered markdown view scrolls
                // its lines. Both answer the same keys.
                if app.scroll_page(0, delta.signum()) {
                    return;
                }
                let Some(max) = scroll_limit else { return };
                let top = &mut app.editors[idx].top_line;
                *top = top.saturating_add_signed(delta).min(max);
            };
            match key.code {
                KeyCode::Up => scroll(self, -1),
                KeyCode::Down => scroll(self, 1),
                KeyCode::PageUp => scroll(self, -20),
                KeyCode::PageDown => scroll(self, 20),
                KeyCode::Home => self.editors[idx].top_line = 0,
                _ => {}
            }
            if matches!(
                key.code,
                KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home
            ) {
                return;
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
        self.follow_completion(key, ctrl);
    }

    // ---- Word completion ----------------------------------------------------------------

    /// Whether the popup still describes the word under the cursor.
    ///
    /// Asked rather than answered: the cursor can move from a tab switch, a click, a
    /// find-and-replace or a menu action, and clearing the popup from each of those places would
    /// mean remembering to do it in the next one too. A check that runs before every use cannot
    /// be forgotten in one place.
    pub fn completion_live(&self) -> bool {
        let Some(popup) = self.completion.as_ref() else { return false };
        if self.focus != Focus::Editor {
            return false;
        }
        // `active_editor_index`, not `pane_editor_index`: this has to name the same buffer that
        // `editor_mut` will write into when the word is accepted, and the two disagree when the
        // focus is on the right pane with the split closed. An accept that took its offsets from
        // one buffer and applied them to another would corrupt a file, so the question is asked
        // the same way in both places rather than kept in step by hand.
        let idx = self.active_editor_index();
        if idx != popup.editor {
            return false;
        }
        let Some(ed) = self.editors.get(idx) else { return false };
        match crate::complete::prefix_at(&ed.rope, ed.cursor_line, ed.cursor_col) {
            Some((start, prefix)) => start == popup.start && prefix == popup.prefix,
            None => false,
        }
    }

    /// The five keys the popup claims. `true` when the editor should not also see the key.
    fn completion_key(&mut self, key: KeyEvent) -> bool {
        if self.completion.is_some() && !self.completion_live() {
            self.completion = None;
        }
        if self.completion.is_none() {
            return false;
        }
        match crate::complete::key_action(key.code, key.modifiers) {
            crate::complete::KeyAction::Fall => false,
            crate::complete::KeyAction::Close => {
                self.completion = None;
                true
            }
            crate::complete::KeyAction::Up => {
                if let Some(popup) = self.completion.as_mut() {
                    popup.move_selection(-1);
                }
                true
            }
            crate::complete::KeyAction::Down => {
                if let Some(popup) = self.completion.as_mut() {
                    popup.move_selection(1);
                }
                true
            }
            crate::complete::KeyAction::Accept => {
                self.accept_completion();
                true
            }
        }
    }

    fn accept_completion(&mut self) {
        let Some(popup) = self.completion.as_ref() else { return };
        let Some(text) = popup.selected().map(|c| c.text.clone()) else { return };
        let start = popup.start;
        let end = start + popup.prefix.chars().count();
        self.completion = None;
        // One undo step: undoing an accepted word puts back what was typed, rather than unpicking
        // the insertion a character at a time.
        self.editor_mut().replace_char_range(start, end, &text);
    }

    /// Called after the editor has seen the key, which is the point: the word to filter against
    /// is the one now in the buffer, not the one that was there before the edit.
    fn follow_completion(&mut self, key: KeyEvent, ctrl: bool) {
        let idx = self.active_editor_index();
        let Some(ed) = self.editors.get(idx) else { return };
        let here = crate::complete::prefix_at(&ed.rope, ed.cursor_line, ed.cursor_col);
        if let Some(popup) = self.completion.as_mut() {
            let alive = match &here {
                Some((start, prefix)) if *start == popup.start && idx == popup.editor => {
                    popup.refilter(prefix)
                }
                _ => false,
            };
            if !alive {
                self.completion = None;
            }
            return;
        }
        if !self.settings.completion {
            return;
        }
        if crate::complete::opens_on(key.code, ctrl, here.as_ref().map(|(_, p)| p.as_str())) {
            self.open_completion();
        }
    }

    /// The names the live session is holding, from the snapshot it publishes. Empty when there
    /// is no session, which is most of the time and costs nothing.
    fn session_names(&self) -> Vec<String> {
        self.figures
            .as_ref()
            .and_then(|w| w.snapshot.as_ref())
            .map(|s| s.vars.iter().map(|v| v.name.clone()).collect())
            .unwrap_or_default()
    }

    /// Builds the candidate list and puts the popup up.
    ///
    /// The index is a snapshot, scanned once here and only filtered afterwards. An index kept up
    /// to date as you type would be worse than a stale one: it would go on offering words that
    /// have since been deleted, and there is no keystroke at which that is easy to explain.
    fn open_completion(&mut self) {
        let idx = self.active_editor_index();
        let Some(ed) = self.editors.get(idx) else { return };
        let Some((start, prefix)) =
            crate::complete::prefix_at(&ed.rope, ed.cursor_line, ed.cursor_col)
        else {
            return;
        };
        let mut index = crate::complete::Index::new();
        index.add_buffer(&ed.rope, Some(ed.cursor_line));
        index.add_keywords(ed.path.as_deref());
        // What the interpreter is holding right now, for a file in a language it speaks. This is
        // the third source the seam was built for in 0.7, and it offers what no buffer can: a
        // name made at the prompt exists nowhere in the file.
        let speaks = ed.path.as_deref().and_then(crate::session::Language::of_path).is_some();
        if speaks {
            index.add_session(&self.session_names());
        }
        // The other tabs count too: a name you are about to write is more often in the file you
        // were just in than nowhere at all. A preview holds no text, so it holds no words.
        for (i, other) in self.editors.iter().enumerate() {
            if i != idx && other.preview.is_none() {
                index.add_buffer(&other.rope, None);
            }
        }
        self.completion = crate::complete::Popup::open(idx, start, prefix, index.into_candidates());
        // Asked only once the popup is actually up. A question whose answer has nowhere to land
        // is a question not worth putting to a server that has to think about it.
        if self.completion.is_some() {
            self.lsp_ask_completion(idx, start);
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
                let bytes = key_to_bytes(key);
                if let Some(term) = self.window_tab_mut(self.active_terminal) {
                    if term.alternate_screen() {
                        // A full-screen program has no history of ours to page through, and it
                        // is the one with something to scroll — so the key goes to it rather
                        // than being swallowed on the way, which is what used to happen.
                        term.write_input(&bytes);
                    } else {
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

        // The git panel is a reader too: the wheel moves through it, a click on a tab switches
        // to it, a click outside puts it away.
        if self.git_panel.is_some() {
            match mouse.kind {
                MouseEventKind::ScrollUp => self.scroll_git_panel(-3),
                MouseEventKind::ScrollDown => self.scroll_git_panel(3),
                MouseEventKind::Down(MouseButton::Left) => {
                    let rect = ui::git_panel_rect(full);
                    if !within(rect, col, row) {
                        self.git_panel = None;
                    } else {
                        self.click_git_panel(rect, col, row);
                    }
                }
                _ => {}
            }
            return;
        }

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
                if self.show_search {
                    self.show_search = false;
                    self.search_input.clear();
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
                    self.mouse_menu_bar_click(col, areas.menu_bar.width);
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
                    // A preview's controls sit inside its frame, so they are claimed before the
                    // click can reach the picture behind them.
                    let idx = self.pane_editor_index(self.editor_pane_focus);
                    if let Some((control, _)) = ui::nav_bar_layout(self, idx, content)
                        .into_iter()
                        .find(|(_, r)| within(*r, col, row))
                    {
                        self.preview_control(control);
                        return;
                    }
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
                                // A second click on the same row is asking to *go* there: a
                                // traceback names a file and a line, and retyping it into the
                                // editor is work a double-click can do for you.
                                if self.second_click_on(i, cell.0) && self.open_location_at(i, cell.0) {
                                    return;
                                }
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
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let down = matches!(mouse.kind, MouseEventKind::ScrollDown);
                let delta = if down { 3 } else { -3 };
                // Ctrl and the wheel is zoom everywhere else; without it the wheel keeps
                // meaning "move", which is what it means over every other frame here.
                if mouse.modifiers.contains(KeyModifiers::CONTROL) && self.zoom_preview_under(col, row, areas, !down) {
                    return;
                }
                self.scroll(col, row, areas, delta)
            }
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
                let cell = cell_at(ui::terminal_content_rect(term_areas[i]), col, row);
                self.wheel_over_terminal(i, delta, cell);
            }
        }
    }

    /// Spends a wheel notch on the pane it is over: on the program running there if it asked to
    /// hear about the mouse, otherwise on our own history.
    ///
    /// The first case is what a terminal emulator does and what this did not. Claude Code, htop,
    /// a mouse-mode vim all turn mouse reporting on and scroll a view of their own; the notch was
    /// being dropped instead, so they could not be scrolled at all — while our own history stayed
    /// empty, because a program on the alternate screen never puts anything in one.
    fn wheel_over_terminal(&mut self, index: usize, delta: isize, cell: Option<(u16, u16)>) {
        let Some(term) = self.window_tab_mut(index) else { return };
        let up = delta < 0;
        if let Some((row, col)) = cell {
            // One notch, one report: the lines-per-notch that `delta` carries is our own idea of
            // how far a notch goes through a scrollback, and a program that handles the wheel
            // has its own.
            if let Some(report) = term.wheel_report(up, row, col) {
                term.write_input(&report);
                return;
            }
        }
        // Nothing asked for the mouse. A full-screen program still owns the screen and has no
        // scrollback of its own, so there the notch has nowhere to go.
        if !term.alternate_screen() {
            term.scroll_by(delta);
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
        let versioned = self.selected_file_is_versioned();
        self.context_menu = Some(ContextMenu::new(target, (rect.x + 2, rect.y + 1), versioned));
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
                // Asked after the row under the pointer has been selected, above: the git
                // half of the menu is about *that* file, and asking first would answer for
                // whichever row the cursor happened to be on before the click.
                let versioned = self.selected_file_is_versioned();
                self.context_menu =
                    Some(ContextMenu::new(ContextTarget::Sidebar, (col, row), versioned));
                return;
            }
        }
        if within(areas.editor, col, row) {
            self.focus = Focus::Editor;
            self.context_menu = Some(ContextMenu::new(ContextTarget::Editor, (col, row), false));
            return;
        }
        if let Some(term_areas) = &areas.terminals {
            if let Some(i) = term_areas.iter().position(|r| within(*r, col, row)) {
                self.focus = Focus::Terminal;
                self.active_terminal = i;
                self.context_menu =
                    Some(ContextMenu::new(ContextTarget::Terminal, (col, row), false));
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
                    // A caption is a row you can click on and nothing happens, which is what it
                    // looks like: no highlight follows the pointer over one either.
                    return (!item.header).then_some(item.action);
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

    fn mouse_menu_bar_click(&mut self, col: u16, width: u16) {
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
        // The background button, over at the other end. Tested after the titles, which own their
        // columns outright; the range is empty when the bar is too narrow to show it.
        let button = ui::menu_bar_button_range(self, width);
        if !button.is_empty() && button.contains(&col) {
            self.toggle_opaque_background();
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
            // The overlay bar drawn while a menu is open spans the window, so that is the width
            // its buttons are placed against.
            self.mouse_menu_bar_click(col, full.width);
            return;
        }
        self.menu.close();
    }

    fn mouse_settings(&mut self, col: u16, row: u16, full: Rect) {
        let modal = ui::settings_modal_rect(self, full);
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
            self.settings_changed();
        }
    }

}

/// Whether a figure's picture is one CleeCode has not put on screen yet — a figure it has never
/// seen, or one the session has drawn again since — recording the time either way.
///
/// Split out of the poll so the rule can be tested on its own: everything else in that loop
/// needs a running interpreter and a pty behind it, and this is the part that was wrong.
fn redrawn(
    seen: &mut std::collections::HashMap<PathBuf, std::time::SystemTime>,
    path: &Path,
    drawn: std::time::SystemTime,
) -> bool {
    if seen.get(path) == Some(&drawn) {
        return false;
    }
    seen.insert(path.to_path_buf(), drawn);
    true
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

    /// A snapshot says which figures a session *holds*, and every tick lists all of them. Read
    /// as "show these", plotting into figure 3 reopened figure 1's tab as well — the one closed
    /// a minute ago because that plot was finished with.
    #[test]
    fn a_figure_is_shown_when_it_is_drawn_and_not_because_it_still_exists() {
        use std::time::{Duration, SystemTime};
        let mut seen = std::collections::HashMap::new();
        let one = PathBuf::from("/figs/fig1.png");
        let three = PathBuf::from("/figs/fig3.png");
        let (drawn, later) = (SystemTime::UNIX_EPOCH, SystemTime::UNIX_EPOCH + Duration::from_secs(1));

        assert!(redrawn(&mut seen, &one, drawn), "a figure never seen before is shown");
        assert!(!redrawn(&mut seen, &one, drawn), "and not again on the next tick");
        assert!(!redrawn(&mut seen, &one, drawn));
        // Plotting into another figure says nothing about this one.
        assert!(redrawn(&mut seen, &three, drawn));
        assert!(!redrawn(&mut seen, &one, drawn));
        // Drawing into it again does, which is what makes a closed tab come back when it
        // should: because the plot changed, not because the session still has it.
        assert!(redrawn(&mut seen, &one, later));
        assert!(!redrawn(&mut seen, &one, later));
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

    /// Changing project folder used to keep the workspace attached, and exit then wrote the new
    /// folder's root, files and shells into its file — so a workspace was destroyed by walking
    /// into another project, without a word on screen. Leaving it is what keeps the file intact.
    #[test]
    fn changing_folder_leaves_a_saved_workspace_but_not_the_built_in_one() {
        assert_eq!(workspace_after_root_change(Some("Marunja")), None);
        assert_eq!(workspace_after_root_change(None), None);
        // A built-in is not a file and belongs to no project, so it travels — all of them,
        // not just the layout one: `clee -w octave` in one folder is the same preset in the next.
        for built_in in crate::workspace::BUILT_INS {
            assert_eq!(
                workspace_after_root_change(Some(built_in)),
                Some(built_in.to_string()),
                "{built_in} should survive a change of folder"
            );
        }
        // Matched by slug, like everywhere else the built-in is recognised.
        assert_eq!(workspace_after_root_change(Some("default  LAYOUT")), Some("default  LAYOUT".to_string()));
        // Someone's own workspace called "default" is an ordinary one and is left behind.
        assert_eq!(workspace_after_root_change(Some("default")), None);
    }

    /// Five tabs on one key: Tab has to come back round in both directions, or the last tab is
    /// reachable only by passing through all the others.
    #[test]
    fn the_git_panel_tabs_cycle_both_ways() {
        assert_eq!(GitTab::Status.cycle(1), GitTab::Diff);
        assert_eq!(GitTab::Diff.cycle(1), GitTab::Graph);
        assert_eq!(GitTab::Graph.cycle(1), GitTab::Branches);
        assert_eq!(GitTab::Branches.cycle(1), GitTab::Stashes);
        assert_eq!(GitTab::Stashes.cycle(1), GitTab::Status, "round, not stuck at the end");
        assert_eq!(GitTab::Status.cycle(-1), GitTab::Stashes, "and round the other way");
        assert_eq!(GitTab::Graph.cycle(-1), GitTab::Diff);

        // Every list whose rows can be acted on carries a cursor, and the diff — which is text
        // rather than a list of things — does not. A highlight on a diff line would be a promise
        // the panel does not keep.
        for tab in GitTab::ALL {
            assert_eq!(tab.picks_a_row(), tab != GitTab::Diff, "{tab:?}");
        }
    }

    /// The cursor lands on commits and never on the `|/` between them, in both directions and at
    /// both ends. A highlight on a piece of the drawing would be offering an action on a line.
    #[test]
    fn the_graph_cursor_steps_over_the_rows_that_are_only_lines() {
        use crate::git::GraphCommit;
        let commit = |hash: &str, parents: &[&str]| GraphCommit {
            hash: hash.to_string(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            refs: Vec::new(),
            author: String::new(),
            when: String::new(),
            subject: String::new(),
        };
        // The shape with a link row at both ends of the middle: a merge, a branch, and the join.
        let graph = vec![
            commit("m", &["a", "b"]),
            commit("a", &["c"]),
            commit("b", &["c"]),
            commit("c", &[]),
        ];
        let rows = crate::git_graph::lay_out(&graph);
        let art: Vec<String> = rows.iter().map(crate::git_graph::Row::art).collect();
        assert_eq!(art, vec!["*", "|\\", "* |", "| *", "|/", "*"], "the shape this is checked on");

        let mut panel = GitPanel {
            tab: GitTab::Graph,
            scroll: 0,
            selected: 0,
            snap: None,
            rows,
            detail: None,
            prompt: None,
            busy: false,
            notice: None,
            body_rows: 10,
        };
        // Down from the merge skips the `|\` and lands on the next commit.
        panel.move_by(1);
        assert_eq!(panel.selected, 2);
        panel.move_by(1);
        assert_eq!(panel.selected, 3);
        // Down again crosses the `|/` to the root.
        panel.move_by(1);
        assert_eq!(panel.selected, 5);
        // Past the bottom it stays on the last commit rather than sliding off it.
        panel.move_by(1);
        assert_eq!(panel.selected, 5);
        // And back up, over the same link row.
        panel.move_by(-1);
        assert_eq!(panel.selected, 3);
        panel.move_by(-10);
        assert_eq!(panel.selected, 0, "the top is a commit, not the row above one");
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
            SessionTarget::default(),
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
        let rows = run_rows("py", None, &available, &registered, &commands, &none, SessionTarget::default(), Lang::En);
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
        let rows = run_rows("tex", Some(".venv"), &available, &[], &commands, &none, SessionTarget::default(), Lang::En);
        assert_eq!(rows.len(), 2);
        assert!(matches!(rows[0].action, RunRowAction::EditCommand(RunScope::Global)));
        assert_eq!(rows[0].detail.as_deref(), Some("pdflatex {file}"));
        assert!(matches!(rows[1].action, RunRowAction::EditCommand(RunScope::Project)));

        // An extension with no command still gets both rows — that is how one is set.
        let rows = run_rows("md", None, &available, &[], &commands, &none, SessionTarget::default(), Lang::En);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].detail, None);
        assert!(matches!(rows[0].action, RunRowAction::EditCommand(RunScope::Global)));
    }

    /// The marker is the only thing in the menu that answers "which of these two wins", and
    /// getting it wrong is how you typeset the wrong master file and blame the editor.
    /// The session row, and the one thing about it that is easy to get wrong: the tick means
    /// "what Run would do right now", so wanting the session and not having one leaves the tick
    /// on the interpreter Run will actually start.
    #[test]
    fn the_session_row_is_ticked_only_when_there_is_a_session() {
        let commands = std::collections::HashMap::from([("py".to_string(), "python3 {file}".to_string())]);
        let none = std::collections::HashMap::new();
        let available = vec![".venv".to_string()];
        let rows = |session: SessionTarget| {
            run_rows("py", Some(".venv"), &available, &[], &commands, &none, session, Lang::En)
        };

        // A language with no session at all — LaTeX — does not get the row.
        let tex = run_rows("tex", None, &[], &[], &none, &none, SessionTarget::default(), Lang::En);
        assert!(!matches!(tex[0].action, RunRowAction::UseSession));

        // Wanted and open: the session is what Run would use, and nothing else is ticked.
        let ticked = rows(SessionTarget { possible: true, open: true, wanted: true });
        assert!(matches!(ticked[0].action, RunRowAction::UseSession));
        assert!(ticked[0].active);
        // Counted over the interpreter rows alone. The two command rows below carry a tick of
        // their own for a different question — which of the two files the command comes from —
        // and one answer each is exactly right.
        let interpreters = |rows: &[RunRow]| {
            rows.iter()
                .filter(|r| matches!(r.action, RunRowAction::UseSession | RunRowAction::SelectVenv(_)))
                .filter(|r| r.active)
                .count()
        };
        assert_eq!(interpreters(&ticked), 1, "one answer, not two");

        // Wanted and none open: Run falls back to the chosen venv, so the tick goes there.
        let fallen = rows(SessionTarget { possible: true, open: true, wanted: false });
        assert!(!fallen[0].active);
        assert!(fallen.iter().any(|r| r.active && matches!(r.action, RunRowAction::SelectVenv(Some(_)))));
        assert_eq!(interpreters(&fallen), 1);
        let no_prompt = rows(SessionTarget { possible: true, open: false, wanted: true });
        assert!(!no_prompt[0].active, "there is nothing to hand the file to");
        assert!(no_prompt.iter().any(|r| r.active && matches!(r.action, RunRowAction::SelectVenv(Some(_)))));

        assert_eq!(interpreters(&no_prompt), 1);
        // And it says which of the two it is, because the row means different things either way.
        assert_ne!(no_prompt[0].detail, ticked[0].detail);
    }

    #[test]
    fn the_marker_follows_the_command_that_would_actually_run() {
        let global =
            std::collections::HashMap::from([("tex".to_string(), "pdflatex {file}".to_string())]);
        let overridden =
            std::collections::HashMap::from([("tex".to_string(), "latexmk main.tex".to_string())]);
        let none = std::collections::HashMap::new();

        // No override: the shared command is in force.
        let rows = run_rows("tex", None, &[], &[], &global, &none, SessionTarget::default(), Lang::En);
        assert!(rows[0].active && !rows[1].active);
        assert_eq!(rows[1].detail, None, "nothing to show for a project that overrides nothing");

        // Overridden: the marker moves, and both commands stay visible so the one being
        // shadowed is not a mystery.
        let rows = run_rows("tex", None, &[], &[], &global, &overridden, SessionTarget::default(), Lang::En);
        assert!(!rows[0].active && rows[1].active);
        assert_eq!(rows[0].detail.as_deref(), Some("pdflatex {file}"));
        assert_eq!(rows[1].detail.as_deref(), Some("latexmk main.tex"));

        // An override with no global command behind it still wins, and nothing is marked as
        // shared because there is nothing shared to mark.
        let rows = run_rows("tex", None, &[], &[], &none, &overridden, SessionTarget::default(), Lang::En);
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
        // Which placeholder becomes which part of the path, and that a space in one survives as
        // a single argument. *How* it is quoted belongs to the shell the line is typed at —
        // single quotes on a Unix shell, double on cmd.exe — and has its own tests, so the
        // expectation is built with the same helper rather than spelling one platform's out.
        assert_eq!(
            expanded,
            format!(
                "pdflatex -output-directory {dir} {file} && open {dir}/report.pdf",
                dir = shell_quote("/work/my papers"),
                file = shell_quote("/work/my papers/report.tex"),
            )
        );
        // Whatever the platform, the space is protected rather than left to split the argument.
        assert!(expanded.contains(&shell_quote("/work/my papers")));
        assert!(!expanded.contains("-output-directory /work/my papers"));

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

    /// cmd.exe has no single quotes, and a Windows path is mostly backslashes — the two things
    /// POSIX quoting uses. Every run command on Windows came out as `'C:\...\octave-cli.exe'`,
    /// which cmd looks for verbatim and never finds. Tested from any platform, since the branch
    /// that is wrong here is the one that runs there.
    #[test]
    fn a_windows_command_is_quoted_the_way_cmd_reads_it() {
        // Nothing to protect: left bare, so an echoed command line stays readable.
        assert_eq!(quote_for_cmd(r"C:\Users\me\octave-cli.exe"), r"C:\Users\me\octave-cli.exe");
        // A space, or anything cmd would otherwise act on, gets double quotes — inside which a
        // backslash is just a backslash.
        assert_eq!(
            quote_for_cmd(r"C:\Program Files\GNU Octave\octave-cli.exe"),
            "\"C:\\Program Files\\GNU Octave\\octave-cli.exe\""
        );
        assert_eq!(quote_for_cmd("a&b"), "\"a&b\"");
        assert_eq!(quote_for_cmd(""), "\"\"");
        // What POSIX quoting does to the same path, and why it is not used there.
        assert_eq!(shell_words::quote(r"C:\Users\me\octave-cli.exe"), r"'C:\Users\me\octave-cli.exe'");
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
