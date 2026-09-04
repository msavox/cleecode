use crate::clipboard::Clipboard;
use crate::dnd;
use crate::editor::Editor;
use crate::file_tree::{Activation, FileTree};
use crate::highlight::Highlighter;
use crate::i18n::{self, Key, Lang};
// Renamed on the way in: `MenuAction` is already here and the two are different alphabets — one
// is everything the app can be asked to do, the other only the chords that can be moved.
use crate::keymap::Action as KeyAction;
use crate::menu::{ContextMenu, ContextTarget, MenuAction, MenuBar};
use crate::settings::{self, Settings};
use crate::terminal_panel::{self, key_to_bytes, MouseAction, TerminalPanel, TerminalWindow};
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
    /// The debug panel. A frame of its own for the same reason the drawer is one: while it holds
    /// the keyboard, single letters are the debugger's verbs — which is only safe because no
    /// other frame's keys change to make room for them.
    Debug,
    /// The agent drawer. A frame of its own rather than a fourth terminal window, because the
    /// keyboard goes somewhere different depending on what is in it: to the pty when an agent is
    /// running, to the launcher's list when one is not.
    Drawer,
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

/// Which field the project search box is typing into.
///
/// Two fields rather than two boxes, because they are two halves of one question. The second one
/// being empty is what keeps the search exactly the search it was: nothing about pressing Enter
/// on a query changes because a replacement field now exists beside it.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum SearchField {
    Query,
    Replace,
}

impl SearchField {
    fn other(self) -> Self {
        match self {
            SearchField::Query => SearchField::Replace,
            SearchField::Replace => SearchField::Query,
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

/// Files a paste into an ssh session offered to upload, held until the question on the status
/// line is answered.
///
/// Held rather than sent, because the pane cannot tell a drag from a paste and the two mean
/// opposite things. Dragging a file onto a terminal that is logged into a server is a request to
/// put it there; pasting the *text* of a path — which is what you do while looking for a file, a
/// key among them — is not a request for anything, and it used to send it anyway.
pub struct PendingUpload {
    /// The ssh destination as it was typed at the shell, handed to `scp` verbatim.
    pub target: String,
    pub paths: Vec<PathBuf>,
}

/// An agent's `edit_buffer`, waiting for the user to say yes or no to it.
///
/// Held rather than applied, and that hesitation is the whole reason the MCP bridge grew a reply
/// channel. Every other thing an agent can ask this editor for is a thing it can see afterwards;
/// this one writes into a buffer that has unsaved work in it, and the tool call on the other side
/// is blocked meanwhile precisely so that the answer can be a person's.
pub struct PendingAgentEdit {
    /// The number the answer goes back under. See [`crate::mcp::Reply`].
    pub id: u128,
    pub path: PathBuf,
    pub old: String,
    pub new: String,
}

/// How many consent questions are kept waiting.
///
/// An agent working through a file can file several before the user has looked up once, and each
/// of them is holding one of its tool calls open. Past a handful the honest answer is to refuse
/// the rest and say why: a queue longer than this is a queue whose tail will time out unanswered
/// anyway, and an agent told "not now" can ask again.
const AGENT_EDIT_QUEUE: usize = 8;

/// The most lines an agent's highlighted range may cover.
///
/// A span is a way of pointing at something. Past a few hundred lines it stops being a gesture and
/// becomes a file with its colours inverted, which points at nothing at all.
const AGENT_SPAN_LINES: usize = 400;

/// The 1-based lines an agent's `line`/`end_line` pair really names.
///
/// Clamped both ways, because both mistakes are ones a model makes: an end before the start is a
/// transposition, and a span the length of a module is a range it did not think about.
fn agent_span_lines(line: usize, end_line: usize) -> (usize, usize) {
    let start = line.max(1);
    let end = end_line.max(start).min(start + AGENT_SPAN_LINES - 1);
    (start, end)
}

/// That range as the absolute char span [`Editor::select_char_range`] takes, clamped to the file.
///
/// Through the *end* of the last line rather than to its first character: a range that stopped
/// where the text of the last line begins would leave that line looking half-marked, and the line
/// an agent named is the line it meant to point at.
fn agent_span(editor: &Editor, line: usize, end_line: usize) -> (usize, usize) {
    let (from, to) = agent_span_lines(line, end_line);
    let last = editor.rope.len_lines().saturating_sub(1);
    let from = (from - 1).min(last);
    let to = (to - 1).min(last);
    let start = editor.rope.line_to_char(from);
    let end =
        if to < last { editor.rope.line_to_char(to + 1) } else { editor.rope.len_chars() };
    (start, end)
}

/// How big an edit is, in the two numbers a diff would print: lines arriving, lines going.
///
/// Not a diff — what an agent sends is two pieces of text, not a patch — but the shape a reader
/// takes in at a glance, and the difference between "change one word" and "replace the whole
/// function" being two questions rather than the same sentence twice.
fn agent_edit_size(old: &str, new: &str) -> (usize, usize) {
    let count = |text: &str| if text.is_empty() { 0 } else { text.lines().count() };
    (count(new), count(old))
}

/// Where `old` sits in `text`, as absolute char indices, when it sits there exactly once.
///
/// `Err(n)` is how many times it was found instead, and that number is the only useful thing to
/// tell an agent about a failed match: none means the buffer has moved on since it read it, and
/// several mean it did not say enough to be unambiguous. Char indices rather than byte offsets
/// because that is what [`Editor::replace_char_range`] takes, and the difference between the two
/// is every file with an accent in it.
///
/// An empty `old` is counted as no match rather than as the infinity of matches it really is. The
/// server refuses that argument before it gets here; this is what keeps a hand-written request
/// file from being a way to insert text at position zero without saying so.
fn only_match(text: &str, old: &str) -> Result<(usize, usize), usize> {
    if old.is_empty() {
        return Err(0);
    }
    let mut found = text.match_indices(old);
    let Some((at, _)) = found.next() else { return Err(0) };
    let extra = found.count();
    if extra > 0 {
        return Err(extra + 1);
    }
    let start = text[..at].chars().count();
    Ok((start, start + old.chars().count()))
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
    /// Whether a trigger character asked this, in which case the answer may *open* a popup rather
    /// than only feed one.
    ///
    /// The two questions look identical on the wire and are opposite in what they promise. A
    /// popup that is already up wants names folded into it and carries on without them; a `.`
    /// that has just been typed has no popup at all, and the answer is the only thing that could
    /// put one there. Recorded when the question goes out because that is the only moment the
    /// difference is known — by the time the words come back there is nothing on screen to read
    /// it off.
    triggered: bool,
}

/// How long a dead server is left alone before it is started again.
///
/// Long enough that a server dying at startup — a broken project file, a version of the program
/// that will not run here — does not turn into a process started every frame, and short enough
/// that somebody who watched rust-analyzer run out of memory once gets it back without knowing
/// there was anything to get back.
const LSP_RESTART_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// How many times a program may die and be started again in one session.
///
/// A crash is usually a once-off: rust-analyzer runs out of memory on a large project and the
/// second one, started against the same files, is fine. A server that dies three times is not
/// having a bad moment, it is refusing to run here, and starting it again would be a process
/// spawned every ten seconds for as long as the editor is open.
const LSP_RESTARTS: usize = 2;

/// Why a program is not running, and whether it is worth trying again.
struct LspTrouble {
    /// What it said on the way out, kept for the same reason it always was: so the reason is
    /// available and the spawn is not repeated to find it out again.
    #[allow(dead_code)]
    detail: String,
    /// How many times it has started and then died. A program that never started at all stays
    /// at zero and is never retried — it is not installed, and it will not become installed
    /// while the editor is open.
    deaths: usize,
    /// When the last of those happened, so a restart waits rather than following it instantly.
    when: Instant,
    /// Whether starting it again is worth doing at all.
    worth_retrying: bool,
}

impl LspTrouble {
    /// Whether enough has passed, and few enough have gone wrong, to try again.
    fn may_start_again(&self, now: Instant) -> bool {
        self.worth_retrying
            && self.deaths <= LSP_RESTARTS
            && now.saturating_duration_since(self.when) >= LSP_RESTART_WAIT
    }
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
    /// What the language's sessions had written by the time the prompt came back. The wait above
    /// is a guess at how long the hook takes; this is the answer, because the snapshot naming the
    /// run's figures is a snapshot that has not been written yet at that instant. Measured
    /// against a real Octave: printing two plots to PNG puts it 1.5 to 2 seconds past the
    /// prompt, which a fixed half-second wait misses every single time — so every rerun of the
    /// script recorded nothing, closed nothing, and drew its plots into a fresh pair of tabs.
    ///
    /// Taken again whenever the prompt is lost and the settling starts over.
    generation: Option<u64>,
    /// Since when nothing new has arrived: set when the generation above is first seen to have
    /// moved, and pushed forward again by every figure that turns up after it. A script that
    /// draws four plots writes them one print at a time, and a run closed in the middle of that
    /// burst would remember half of its own figures.
    quiet: Option<std::time::Instant>,
}

/// How often a watched run's session is asked what it is holding.
const RUN_WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

/// How often the unsaved buffers are copied into the recovery directory.
///
/// Five seconds is the whole of the trade and there is no setting for it. Shorter costs a write
/// per buffer more often for work that has usually not moved — the revision gate in
/// `App::poll_autosave` catches the idle case, but a fast typist would then be writing their
/// whole file to disk several times a second. Longer widens the only gap this feature has: the
/// edits made after the last copy and before the ending. Five is short enough that what is lost
/// is a sentence, and long enough that nobody ever notices it happening.
const AUTOSAVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// How long after the last thing a run published it counts as finished. Counted from the moment
/// the session was seen to write, not from the moment the prompt came back — the two are the same
/// only for a script that draws nothing. Long enough that a script printing four figures one at a
/// time is not cut in half, and short enough that nothing typed at the prompt afterwards is caught
/// and blamed on the run.
const RUN_SETTLE: std::time::Duration = std::time::Duration::from_millis(500);

/// How long a finished run waits for its session to say anything at all before it is closed with
/// whatever it has. What it protects against is an interpreter with no CleeCode hook in it, which
/// writes no snapshot ever: without a limit that run would be watched until the editor is closed.
/// Longer than the second or two a hook takes to print its figures, since expiring here is
/// forgetting them.
const RUN_SETTLE_MAX: std::time::Duration = std::time::Duration::from_secs(5);

/// How long a run that was never caught mid-command is watched before it is called finished.
/// A script that draws and returns inside one frame is over long before this; what the wait
/// protects against is the opposite mistake, of calling a run finished while it is still
/// starting up and attributing its figures to nobody.
const RUN_WATCH_MAX: std::time::Duration = std::time::Duration::from_secs(2);

/// A buffer's text as the Find box last flattened it, and enough about where it came from to
/// tell whether it still describes that buffer. Compared, never interpreted.
struct FindText {
    editor: usize,
    revision: u64,
    chars: usize,
    text: String,
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
    /// The agent drawer, once it has been summoned. `None` until then, and never again after —
    /// closing it hides its column and leaves the struct, so the pty inside goes on running and
    /// reopening resumes the conversation instead of starting one.
    ///
    /// Outside `terminals` on purpose; the reason is written on [`crate::drawer::Drawer`], and
    /// it is the whole design. Everything in this file that walks `terminals` has to name the
    /// drawer separately or leave it alone deliberately — the polling loops do the first, the
    /// workspace rebuild and every "which shell can I type into" scan do the second.
    pub drawer: Option<crate::drawer::Drawer>,
    pub should_quit: bool,
    /// Set when something that can change what is on screen has happened, and cleared by the
    /// frame loop when it draws. A frame is a full layout plus a repaint of every pane, and an
    /// editor left open with nothing happening in it was paying that thirty times a second to
    /// produce the identical screen. Raised generously on purpose: a frame drawn for nothing
    /// costs a few milliseconds, while a frame not drawn when it was needed is a screen that
    /// disagrees with the file.
    redraw: bool,
    /// The sum of every pane's output counter as of the last look. A shell that printed since
    /// then moves the sum; nothing else does.
    terminal_generation: u64,
    pub status_message: String,
    pub editor_viewport: (usize, usize),
    /// Where the pointer last was. Only used to light up the scrollbar it is resting on, which
    /// is the one bit of the interface that has to react to the mouse merely being somewhere.
    pointer: Option<(u16, u16)>,
    pub settings: Settings,
    /// The colours being drawn in, which is the *resolved* form of `settings.theme`: the same
    /// thing for a theme chosen by name, and for `auto` whichever of the two the terminal's own
    /// background asked for. Kept beside the setting rather than worked out per frame, because
    /// resolving reads a fact about the session that only exists once (see `preview::background`)
    /// and because the setting is what gets saved while this is what gets painted.
    pub theme: crate::theme::Theme,
    /// Which chord runs which action. Built from the defaults plus whatever `[keys]` in
    /// settings.toml moved, and rebuilt when that file is saved from inside the editor.
    pub keymap: crate::keymap::Keymap,
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
    /// The box that asks what to call the name under the cursor instead, when it is open.
    ///
    /// Named apart from the three fields above, which are the file tree's rename, and the
    /// distance between the two is the point: one moves a file on disk, this one asks a language
    /// server a question about a name inside one. They can never be up at the same time, and a
    /// shared field would still have been a field two features had to agree about.
    pub symbol_rename: Option<SymbolRename>,
    /// What a rename would change, once the server has said and before any of it is a buffer.
    pub rename_preview: Option<RenamePreview>,
    /// The run-target drop-down, while it is open under its toolbar button.
    pub run_menu: Option<RunMenu>,
    /// The theme drop-down, while it is open under its button on the menu bar. Just the row the
    /// cursor is on: the list itself is `ThemeChoice::all()`, which cannot change while it is
    /// open.
    pub theme_menu: Option<usize>,
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
    /// When set, an upload is waiting on a yes from the status line. See [`PendingUpload`].
    pub pending_upload: Option<PendingUpload>,
    /// The agent's edit currently being asked about on the status line. See [`PendingAgentEdit`].
    pub agent_edit_ask: Option<PendingAgentEdit>,
    /// The ones behind it, oldest first. A question is only put up when nothing else owns the
    /// keyboard, so an agent that asks while the find box is open waits for the box to close
    /// rather than talking over it.
    agent_edit_queue: std::collections::VecDeque<PendingAgentEdit>,
    /// Set by answering a consent question with `A`: every further edit this session goes through
    /// without asking. Deliberately not persisted — "yes, while I am watching you" is a statement
    /// about this afternoon, and a setting that outlived the session would be a different promise
    /// from the one that was made. `agent_edits = "allow"` in settings.toml is how somebody says
    /// the permanent version of it on purpose.
    agent_edits_this_session: bool,
    /// In-file find / find-and-replace overlay state, when open.
    pub find: Option<crate::find::FindState>,
    /// The buffer the Find box is scanning, flattened into a string, kept between keystrokes.
    ///
    /// The regex engine reads a `&str` and the buffer is a rope, so every rescan needs a copy of
    /// the whole file. Made afresh for each character typed, that copy — not the scan — is what
    /// a search box costs in a large file. It is only ever stale in one way, so remembering
    /// which buffer it came from and at which revision is enough to know when to make it again.
    find_text: Option<FindText>,
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
    /// Why a server is not running, by program name — so a machine with `gopls` and without
    /// `clangd` still gets Go, and the missing one is not spawned again at every keystroke:
    /// starting a process that is not there, sixty times a second, is its own kind of bug.
    ///
    /// Kept for programs that are running again, too. It is the record of what has already gone
    /// wrong with each of them, and forgetting it on a successful restart is what would let a
    /// server that dies every time it starts do so for the rest of the session.
    lsp_error: std::collections::HashMap<String, LspTrouble>,
    /// What the server says about each file. Replaced wholesale per file, because that is what
    /// the protocol sends: a list, not a diff.
    pub diagnostics: std::collections::HashMap<PathBuf, Vec<crate::lsp::Mark>>,
    /// The same diagnostics as they arrived, before anything was measured against a buffer.
    ///
    /// A second copy, and not a duplicate: a [`crate::lsp::Mark`] is what a squiggle needs — a
    /// line, two character columns, a sentence — and a code action question needs what a *server*
    /// needs, which is the diagnostic whole. The `code`, the `source` and the opaque `data` a
    /// server hangs off its own diagnostics are exactly what it matches its quick fixes against,
    /// and they do not survive the conversion. Kept and dropped in step with the marks, so the two
    /// can never describe different files.
    lsp_raw: std::collections::HashMap<PathBuf, Vec<lsp_types::Diagnostic>>,
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
    /// The one request for a list still out — references, or the names in a file.
    ///
    /// A slot of its own rather than a share of [`Self::lsp_asked`], and that is the whole
    /// reason it exists: hovers fill that one on their own account, several a second, and a
    /// list asked for on purpose would either be cancelled by the next one or be blocked by it
    /// forever. Neither of those is something a key press should have to know about.
    lsp_listing: Option<PendingAsk>,
    /// The one rename still out.
    ///
    /// A third slot, and the separation from the other two is the whole reason it exists: those
    /// hold questions whose answers are things to *read*, and a hover fills one of them several
    /// times a second on its own account. An answer that is going to write into buffers cannot
    /// share a slot with that — a rename quietly displaced by a hover is a key that appeared to
    /// do nothing to a file the user believed they had changed.
    lsp_editing: Option<PendingRename>,
    /// The one format still out.
    ///
    /// A fourth slot, for the reason the third one exists and one more besides. It holds an
    /// answer that writes, so it cannot share with the hovers; and it is not the rename, because
    /// the two are asked of different things and neither cancels the other — a format asked while
    /// a rename is still out would otherwise displace it, and the buffer the user was renaming in
    /// would quietly stop changing.
    ///
    /// `from` is where the cursor was when the key was pressed, which is where it goes back
    /// afterwards. Nothing else in it is read: a format is about the whole file, so there is no
    /// position for the answer to be checked against.
    lsp_formatting: Option<PendingAsk>,
    /// The one question about what can be done here still out.
    ///
    /// A fifth slot, for the reasons the third and fourth exist: its answer opens a list that a
    /// hover must not displace, and it is neither of the other two — a code action asked for while
    /// a format is still out would otherwise cancel it, and the file the user was laying out would
    /// quietly stay as it was.
    lsp_acting: Option<PendingAsk>,
    /// The one action whose edit has been asked for and not come back.
    ///
    /// Its own slot rather than a share of the one above, because the two are out at once: the
    /// list is asked for, answered, and then one row of it is asked about again. The title travels
    /// with it for the reason [`PendingRename`]'s names do — by the time the edit lands the picker
    /// is gone, and a status line assembled from whatever is on screen then would name the wrong
    /// action.
    lsp_action_edit: Option<PendingAction>,
    /// The one question about what encloses the cursor still out.
    ///
    /// A sixth slot, for the reason the fifth exists: it is asked by a chord that people press
    /// several times in a row, and a hover arriving between two of those presses must not be able
    /// to cancel the walk half way out of an expression.
    lsp_widening: Option<PendingAsk>,
    /// The ladder of ever-wider ranges the last expand asked for, while it is still the truth.
    /// See [`SelectionWalk`], which is also where the rule for when it stops being the truth is.
    selection_walk: Option<SelectionWalk>,
    /// Which requests for a file's fold boundaries are out, and which file each is about.
    ///
    /// A map rather than a slot, because these are not asked by anybody: several files can be
    /// opened in the same frame and each is asked about on its own account, so there is no "the
    /// current one" to keep and no newer question that should cancel an older.
    lsp_folding: std::collections::HashMap<i64, PathBuf>,
    /// The buffer revision each file's cached fold boundaries were asked for at.
    ///
    /// This one number is the whole of the "ask on open, ask again on save" rule, and it is derived
    /// rather than hooked into the three places a save can happen: a clean buffer whose revision is
    /// not the one written down here is a file the server has not been asked about in its current
    /// state — which is true exactly once when it is opened, and once more each time a save leaves
    /// it clean at a revision the edits moved it to.
    lsp_folds_asked: std::collections::HashMap<PathBuf, u64>,
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
    ///
    /// One field for both debuggers. The interpreter one sets it from a snapshot and the debug
    /// adapter sets it from a `stopped` event, and they mean exactly the same thing to the
    /// renderer: this is the line the program is on. A second field would be a second highlight
    /// to keep in step with the first.
    pub stopped_at: Option<(PathBuf, usize)>,
    /// The debug adapter session, while there is one. See [`DebugSession`].
    pub debug: Option<DebugSession>,
    /// The debug panel: its column, its cursor, and the watches. See [`DebugPanel`] for why it
    /// sits beside the session rather than inside it.
    pub debug_panel: DebugPanel,
    /// The single-line question the debugger is asking, when it is asking one. See
    /// [`DebugPrompt`].
    pub debug_prompt: Option<DebugPrompt>,
    /// What this project's *Debug ▸ Start* runs, once somebody has started one.
    ///
    /// Remembered here and written into the workspace file at exit, beside the active venv and
    /// the other per-project choices — so the guess is filled in once and the answer outlives the
    /// session. A project opened without a named workspace has nowhere to write it and asks its
    /// guess again next time, which is the same bargain every other workspace field makes.
    debuggee: Option<PathBuf>,
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
    /// The previous sweep's git status, kept so the next one can be compared with it: the
    /// difference between two of them is the list of files something has just written, which is
    /// all follow mode ever knows. `None` until the first sweep lands — before that there is no
    /// "previously", and treating an empty map as one would open every changed file in the
    /// project at startup.
    ///
    /// Kept whether or not follow mode is on, so switching it on means "from now on" rather than
    /// "everything git has been holding since this morning".
    follow_seen: Option<std::collections::HashMap<PathBuf, crate::git_status::FileStatus>>,
    /// Files something has written that follow mode has not shown yet, oldest first.
    ///
    /// A queue rather than a single path, because an agent's first move often touches several
    /// files inside one 700 ms sweep and the ones after the first would otherwise be lost: git
    /// reports the same status next time, so no later sweep would ever mention them again.
    /// Drained one per sweep, which is what the throttle actually is — a burst arrives as a
    /// sequence, and the window moves once at a time.
    follow_queue: std::collections::VecDeque<PathBuf>,
    /// How many tabs follow mode has opened this session, against `FOLLOW_TAB_LIMIT`.
    follow_opened: usize,
    bg_tx: Sender<String>,
    bg_rx: Receiver<String>,
    /// Pictures being decoded off the main thread, answering by path: a tab's index can change
    /// while a decode is still running, but the file it was asked about cannot.
    preview_tx: Sender<crate::preview::Decoded>,
    preview_rx: Receiver<crate::preview::Decoded>,
    /// The box asking what to look for across the project, when open.
    pub show_search: bool,
    pub search_input: String,
    /// The box's second field: what the matches become. Empty is the whole of the difference
    /// between the two things this box does — Enter on an empty one is the search that has always
    /// been here and opens a list, Enter on a filled one opens a preview of a sweep.
    pub search_replace: String,
    /// Which of the two fields typing goes into, switched with Tab. The terminal's name-and-
    /// command box works exactly this way, and two boxes that look alike have to behave alike.
    pub search_field: SearchField,
    /// The replacement the running walk was asked for, if it was asked for one, held until the
    /// walk answers. See [`PendingReplace`] for why the flags travel with it.
    replace_asked: Option<PendingReplace>,
    /// The preview of a sweep, when one is up.
    pub replace_sweep: Option<ReplaceSweep>,
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
    /// The bridge to `clee --mcp`: the session directory this editor publishes into, and reads
    /// requests back out of. `None` on a machine with nowhere to put it, which costs the MCP
    /// server and nothing else.
    mcp: Option<crate::mcp::Session>,
    /// When the recovery copies were last taken. See [`App::poll_autosave`].
    last_autosave: Instant,
    /// Whether the user has already been told that the copies cannot be written.
    ///
    /// A latch and not a counter, because the failure it reports — a full disk, an unwritable
    /// config directory — does not go away on its own: without it the same sentence would replace
    /// the status line every five seconds for the rest of the session, which is not a warning,
    /// it is a wall. Cleared by the next tick that writes something, so a disk that was freed up
    /// can complain again if it fills a second time.
    autosave_complained: bool,
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

/// A request that has not been answered yet — a definition, a hover, or a list.
///
/// `from` is where the cursor was when it went out — checked when the answer arrives, since by
/// then it may be somewhere else entirely, and used as the place to come back to after a jump.
pub struct PendingAsk {
    pub id: i64,
    pub from: (PathBuf, usize, usize),
}

/// A rename that has gone out to a server and not come back.
///
/// The names travel with it rather than being read off the buffer when the answer lands. By then
/// the text may have moved, and a title reading `foo → bar` assembled from whatever happens to be
/// under the cursor a second later would be a preview describing the wrong rename.
pub struct PendingRename {
    pub id: i64,
    /// The buffer and the place in it the question was asked from: the position the answer is
    /// about, and the cursor to put back once the edits have landed.
    pub from: (PathBuf, usize, usize),
    pub old_name: String,
    pub new_name: String,
}

/// One code action whose edit has been asked for and not come back.
///
/// The parallel of [`PendingRename`] for the other question that writes: `from` is the buffer and
/// place the list was opened from — where the cursor goes back to — and `title` is what the server
/// called the action, kept because the answer carries no name of its own.
pub struct PendingAction {
    pub id: i64,
    pub from: (PathBuf, usize, usize),
    pub title: String,
}

/// The ladder one expansion is climbing, and the proof that it is still the reader's ladder.
///
/// The chain comes back from the server once and every later press walks it here — which is what
/// makes the second Expand instant and Shrink possible at all, since going back inwards is a step
/// down a list rather than a question anybody can ask a server.
///
/// The last three fields are the whole of how it dies. A walk is only good for the buffer it was
/// asked about (`path`), for the text it was asked about (`revision`), and for as long as the
/// selection on screen is still the one this walk put there (`selected`). Any other hand — an arrow
/// key, a click, a typed character, a file switched under it — leaves the selection or the revision
/// somewhere this cannot recognise, and the walk is dropped and asked again from wherever the
/// cursor now is. That is deliberately a check rather than a hook: hooking it would mean every one
/// of the several hundred places that move a cursor remembering to clear a field, and the one that
/// forgot would expand from a range nobody is looking at.
struct SelectionWalk {
    path: PathBuf,
    revision: u64,
    /// The chain innermost first, as absolute char ranges in this buffer — already converted out
    /// of the server's units, because that conversion needs the text and this is where the text is.
    spans: Vec<(usize, usize)>,
    /// Which rung is selected right now.
    at: usize,
    /// What that rung selects, so a selection made by anything else can be told from this one.
    selected: (usize, usize),
}

/// What one press of Expand or Shrink did to a live walk.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// It moved, and this is what is selected now.
    Moved(usize, usize),
    /// There is nothing wider in the ladder.
    Widest,
    /// There is nothing narrower: this is where the widening started.
    Narrowest,
}

impl SelectionWalk {
    /// Stands the walk on the first rung wider than what is already selected.
    ///
    /// "Wider than what is selected" and not "the outermost": the ladder starts at the token under
    /// the caret, and after a press or two of Expand the selection is already several rungs up one
    /// like it. The rung to stand on is the innermost that *strictly* contains what is on screen —
    /// with nothing selected, the innermost that contains the caret and is not itself empty.
    ///
    /// The rungs below it are kept rather than dropped, and that is what lets Shrink go further in
    /// than the selection this expansion started from: they are all real levels of the same ladder,
    /// and every one of them contains the position the question was asked about.
    fn starting_at(
        path: PathBuf,
        revision: u64,
        spans: Vec<(usize, usize)>,
        here: (usize, usize),
    ) -> Option<SelectionWalk> {
        let at = spans
            .iter()
            .position(|&(start, end)| start <= here.0 && end >= here.1 && (start, end) != here && end > start)?;
        let selected = spans[at];
        Some(SelectionWalk { path, revision, spans, at, selected })
    }

    /// Whether this walk is still about what is on screen — the three questions of the doc comment
    /// above, asked in one place so Expand and Shrink cannot answer them differently.
    fn still_true(&self, editor: &Editor) -> bool {
        editor.path.as_deref() == Some(self.path.as_path())
            && editor.revision() == self.revision
            && selected_char_range(editor) == Some(self.selected)
    }

    /// One rung outwards (`1`) or inwards (`-1`), or which end of the ladder stopped it.
    ///
    /// Running out of ladder is an answer here rather than a failure: the walk is perfectly alive,
    /// there is simply nothing beyond this rung, and the caller says so instead of sending the
    /// server a question it has already answered.
    fn step(&mut self, direction: isize) -> Step {
        let next = self.at as isize + direction;
        if next < 0 {
            return Step::Narrowest;
        }
        let Some(&(start, end)) = self.spans.get(next as usize) else { return Step::Widest };
        self.at = next as usize;
        self.selected = (start, end);
        Step::Moved(start, end)
    }
}

/// The box that asks what to call it instead.
pub struct SymbolRename {
    /// The identifier under the cursor when the box opened. Shown in the prompt and compared
    /// against what was typed, so a name left untouched closes the box without asking anything.
    pub old_name: String,
    pub from: (PathBuf, usize, usize),
    /// What has been typed so far, prefilled with `old_name`.
    pub typed: String,
}

/// One replacement, converted against the buffer it belongs to.
///
/// Absolute char indices, half-open, which is what [`Editor::replace_char_range`] takes — and
/// they are only true while the buffer has not moved, which is what [`RenameFile::revision`]
/// exists to check. `line` is the line the replacement *starts* on, and it rides along because
/// two things in the rename want it: the preview groups its rows by line, and the cursor is put
/// back by counting the edits before it on its own line.
///
/// Shared with the formatter, which produces the same thing out of a different question — one
/// span, replaced by one string — and reads none of it back: a formatter's spans routinely run
/// over several lines, so `line` names only where each begins and the format path neither draws
/// rows nor counts edits along one. What the two share is the rebuild below, which is the part
/// that must not exist twice.
pub struct BufferEdit {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub new_text: String,
}

/// The edits one open buffer would receive, in ascending order and not overlapping.
pub struct RenameFile {
    pub path: PathBuf,
    /// The buffer's revision when the preview was built.
    ///
    /// The whole guard on the offsets above. A clean buffer is reloaded from disk by the sweep in
    /// the frame loop without anybody pressing anything, and a preview built before that reload
    /// describes text that is no longer in the rope. Checked again at the moment Enter is
    /// pressed, because that is the only moment at which it matters.
    pub revision: u64,
    pub edits: Vec<BufferEdit>,
}

/// What a rename would change, ready to be read and then applied.
///
/// Built once, from the server's answer and the buffers as they are at that moment, and never
/// recomputed: the rows on screen and the offsets that get written are two views of the same
/// list, so what is applied is what was shown or nothing is.
pub struct RenamePreview {
    /// What is being renamed, and — when it is empty — the mark that this preview is not a rename
    /// at all.
    ///
    /// A code action reaching more than one buffer comes up in this same box, with these same
    /// refusals and this same Enter, and it has no old name: what it has is the server's own title
    /// for what it would do, which goes in `new_name` and is the whole caption. One field carrying
    /// that distinction rather than a second box carrying a second copy of the machinery — see
    /// [`i18n::msg_rename_preview_title`], which is where it is read.
    pub old_name: String,
    pub new_name: String,
    /// Where the key was pressed — the buffer whose cursor is put back afterwards, and its
    /// position as an absolute char index in that buffer before any of this is applied.
    pub from: (PathBuf, usize, usize),
    pub from_char: usize,
    pub files: Vec<RenameFile>,
    /// The lines to draw, diff-shaped: a header per file, then a `-`/`+` pair per changed line.
    pub rows: Vec<String>,
    /// How many changes in all, so the title does not have to add them up while it draws.
    pub edits: usize,
    pub scroll: usize,
    /// How many rows the body has room for, written by the renderer for the same reason the git
    /// panel's is: keeping a reading position sane needs the height, and only the renderer knows.
    pub body_rows: usize,
}

impl RenamePreview {
    /// Moves the reading position, stopping at the top and at the last screenful.
    pub fn scroll_by(&mut self, by: isize) {
        self.scroll = scrolled(self.scroll, self.rows.len(), self.body_rows, by);
    }
}

/// A reading position moved by `by`, stopping at the top and at the last screenful.
///
/// A free function because there are two previews now and they scroll identically — the rename's
/// and the project sweep's. Nothing about clamping a scroll knows which of them it is looking at,
/// and two copies of it would be two chances for one of them to stop scrolling one row short.
fn scrolled(scroll: usize, rows: usize, body_rows: usize, by: isize) -> usize {
    let page = body_rows.max(1);
    let last = rows.saturating_sub(page) as isize;
    (scroll as isize + by).clamp(0, last.max(0)) as usize
}

/// Where a file a sweep would change actually lives, and the guard that says it has not moved
/// since the preview was built.
///
/// The discriminator is the whole safety of replacing across a project, because the two roads
/// are not interchangeable and the choice is not an optimisation. A file with a tab open is
/// edited *through the rope*, always: a disk write under an open tab is picked up by the 700 ms
/// sweep in the frame loop, which reloads a clean buffer and clears its undo stack outright — so
/// the replacement would land and the one keystroke that could take it back would be gone — and
/// under a *dirty* tab it is worse still, since the buffer keeps its text, the disk keeps the
/// replacement, and the two diverge in silence. A file with no tab has no rope to edit, so it is
/// rewritten on disk, and everything it needs to be written back the way it was found travels
/// here with it.
pub enum SweepTarget {
    /// A file a tab holds. Edited through [`Editor::replace_char_range`], one call and therefore
    /// one step of undo, whether the buffer is clean or dirty.
    OpenBuffer {
        /// The buffer's revision when the preview was built — the guard on the char offsets, and
        /// the same one the rename uses. See [`RenameFile::revision`].
        revision: u64,
    },
    /// A file nothing has open. Rewritten whole, through `settings::write_atomic`.
    Disk {
        /// The file's timestamp when it was read. The disk's answer to a buffer's revision: if it
        /// has moved, something else has written the file and every offset below is measured
        /// against text that is no longer there.
        ///
        /// `None` from a filesystem that will not say, which then compares equal to the `None`
        /// read again at apply time — a guard that cannot be checked is not one to refuse on, and
        /// the preview is seconds old.
        mtime: Option<std::time::SystemTime>,
        /// How the file ends its lines, and whether it ends with one at all. Both recorded on the
        /// way in and re-applied on the way out, because the text below is normalized to `\n` the
        /// way a buffer's is: a sweep that turned a CRLF file into an LF one, or quietly grew a
        /// final newline, would show up as a diff of every line in the file.
        line_ending: crate::editor::LineEnding,
        final_newline: bool,
    },
}

/// One file a sweep would change: where it is, which road it takes, and the replacements.
pub struct SweepFile {
    pub path: PathBuf,
    pub target: SweepTarget,
    /// Absolute char indices into the file's text as it was read, ascending and not overlapping —
    /// the same shape [`edits_as_one_span`] takes, and produced by the same walk for both roads
    /// so a file cannot be replaced one way in a tab and another way on disk.
    pub edits: Vec<BufferEdit>,
}

/// What a replace across the project would change, ready to be read and then applied.
///
/// The rename's preview with the project's problem added: the rename refuses a file no tab holds,
/// and this one is *for* the files no tab holds. Everything else is deliberately the same — built
/// once and never recomputed, diff-shaped rows, one step of undo per buffer, all-or-nothing —
/// because it is the same problem and a reader who has agreed to one has agreed to the other.
pub struct ReplaceSweep {
    /// The query as it was typed, and what it becomes. Shown in the title; a pattern's groups are
    /// already resolved in the rows, which is where they can actually be read.
    pub query: String,
    pub replacement: String,
    /// Where the keyboard was when the sweep was asked for, so the cursor can be put back after
    /// text moves under it. `None` when no tab was open, which is a perfectly ordinary way to
    /// search a project.
    pub from: Option<(PathBuf, usize, usize)>,
    pub from_char: usize,
    pub files: Vec<SweepFile>,
    /// The lines to draw, diff-shaped: a header per file, then a `-`/`+` pair per changed line.
    pub rows: Vec<String>,
    pub edits: usize,
    pub scroll: usize,
    /// How many rows the body has room for, written by the renderer — the same arrangement as
    /// the rename preview's and the git panel's, for the same reason.
    pub body_rows: usize,
}

impl ReplaceSweep {
    pub fn scroll_by(&mut self, by: isize) {
        self.scroll = scrolled(self.scroll, self.rows.len(), self.body_rows, by);
    }

    /// How many of the files take each road. Two numbers rather than one because they are two
    /// different promises to the reader: the buffers can be undone and the disk cannot.
    fn split(&self) -> (usize, usize) {
        let buffers =
            self.files.iter().filter(|f| matches!(f.target, SweepTarget::OpenBuffer { .. })).count();
        (buffers, self.files.len() - buffers)
    }
}

/// A project search that was asked with a replacement, and so answers with a preview instead of
/// a list.
///
/// The flags ride along rather than being read off the app when the answer comes back. The walk
/// happens on a thread and the box is closed the moment it starts, so by the time this is used
/// the switches could have been moved by the *next* search being typed — and re-scanning with
/// flags the hits were not found under is how a preview comes to describe a query nobody asked.
struct PendingReplace {
    replacement: String,
    regex: bool,
    case_sensitive: bool,
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

/// Where the keyboard goes when the last tab closes and the editor can no longer hold it.
///
/// A free function because the rule is one an `App` cannot be built to test: whatever it lands
/// on has to be *on screen*. A hidden terminal is still a running shell, so focusing one that is
/// not drawn sends every keystroke to a command line nobody can read — the empty editor, which
/// ignores keys, is the safer place for them to go nowhere.
fn empty_state_focus(show_sidebar: bool, show_terminal: bool) -> Focus {
    if show_sidebar {
        Focus::FileTree
    } else if show_terminal {
        Focus::Terminal
    } else {
        Focus::Editor
    }
}

/// How a row of a list of places reads: where it is, then what is there.
///
/// The same shape as a search result and for the same two reasons — the path is relative to the
/// project because the part that repeats on every row is the part worth dropping, and the text
/// goes through [`crate::search::shorten`] because a generated line is thousands of characters
/// long and a picker row is one line. `line` is counted the way a person counts them.
///
/// A file that could not be read still gets a row, with the place and nothing after it: where
/// it is is most of what was asked for, and a row missing from the list is a use gone missing.
/// A whole set of edits to one buffer, as the single replacement that carries them out: where it
/// starts, where it ends, and what goes there.
///
/// The [`App::replace_all`] pattern, and for the same reason. A rename is one action and so is a
/// format, so each has to be one step to undo — replacing every site on its own would put a whole
/// copy of the file on the undo stack per site, and taking back a forty-site rename would mean
/// forty Ctrl+Z, a reformatted file rather more than that. So the run from the first edit to the
/// last is rebuilt here, replacements where the edits were and the text between them carried over
/// verbatim, and written back in a single edit.
///
/// Written once and used by both, which is why it says nothing about *why* the edits exist. The
/// arithmetic is the part that must never be written twice: a second copy of it would be a second
/// place for an off-by-one to eat a character, and only on files with edits of unequal length.
///
/// Every slice is taken before anything is written, which is not a nicety: replacing the first
/// character would move every offset measured after it.
///
/// `edits` must be ascending and non-overlapping. `None` for an empty list, which is a file the
/// server named and then asked nothing of.
fn edits_as_one_span(editor: &Editor, edits: &[BufferEdit]) -> Option<(usize, usize, String)> {
    rebuild_edits(edits, |from, to| editor.rope.slice(from..to).to_string())
}

/// The same rebuild, told where to get the untouched text from rather than being handed a buffer.
///
/// Split out because a replace across the project writes files nobody has open, and those have no
/// rope to slice: they are a `String` read a moment ago. The arithmetic is the part that must not
/// exist twice — it is where an off-by-one eats a character — so it exists here, and the two
/// roads differ only in how they say "the characters from here to there".
///
/// `slice` is asked for half-open char ranges, which is what [`BufferEdit`] counts in.
fn rebuild_edits(
    edits: &[BufferEdit],
    slice: impl Fn(usize, usize) -> String,
) -> Option<(usize, usize, String)> {
    let span_start = edits.first()?.start;
    let span_end = edits.last()?.end;
    let mut rebuilt = String::new();
    let mut carried = span_start;
    for edit in edits {
        if edit.start > carried {
            rebuilt.push_str(&slice(carried, edit.start));
        }
        rebuilt.push_str(&edit.new_text);
        carried = edit.end;
    }
    Some((span_start, span_end, rebuilt))
}

/// Why a whole format is being refused. Two reasons, and the caller turns each into a sentence —
/// the arithmetic below is a free function so it can be tested, and a free function has no
/// business knowing which language the status bar is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FormatRefusal {
    /// A line the buffer does not have, or a file that is no longer open under that name.
    Moved,
    /// Two edits over the same characters.
    Overlap,
}

/// A formatter's answer, converted from the server's line-and-column spans into the absolute,
/// ascending, non-overlapping character ranges [`edits_as_one_span`] takes.
///
/// The general converter the rename does not have, and deliberately not a loosening of the one it
/// does. A rename refuses a span that crosses a line — see [`crate::lsp::SpanEdit::spans_lines`],
/// which stays exactly as strict as it was — because a rename that spans lines is a server
/// describing something other than a name. For a formatter it is the ordinary case: replacing a
/// file with a laid-out copy is one edit from the top to past the bottom. So *both* ends are
/// converted against the line each actually sits on, which is the whole of what makes this
/// general: measuring the end column on the start line is right for a name and silently wrong
/// for everything else.
///
/// `chars_for` turns a column the server sent into the characters the editor counts, measured
/// against the line it belongs to. Passed in rather than chosen here because the units are the
/// file's server's, and this function has never heard of a server.
///
/// `lines` is the buffer's lines with their newlines stripped, which is what makes a column past
/// the end of a line clamp to the end of that line rather than reaching into the next one.
///
/// Every `Err` refuses the whole format. Half a laid-out file is worse than an unlaid one: it is
/// a file nobody wrote, in a buffer whose next Ctrl+Z takes back something the reader never saw
/// arrive.
fn format_spans(
    rope: &ropey::Rope,
    lines: &[String],
    chars_for: &dyn Fn(&str, usize) -> usize,
    edits: &[crate::lsp::SpanEdit],
) -> Result<Vec<BufferEdit>, FormatRefusal> {
    let last = rope.len_chars();
    let mut converted = Vec::with_capacity(edits.len());
    for edit in edits {
        // A start line the buffer does not have is the server describing text that is no longer
        // here. Refused rather than clamped, exactly as the rename refuses it and for the same
        // reason: a replacement in the wrong place is damage, not noise.
        let start_text = lines.get(edit.start_line).ok_or(FormatRefusal::Moved)?;
        let start = rope.line_to_char(edit.start_line) + chars_for(start_text, edit.start_col);
        let end = match lines.get(edit.end_line) {
            Some(end_text) => rope.line_to_char(edit.end_line) + chars_for(end_text, edit.end_col),
            // The one end clamped instead of refused, because it is not a server being wrong.
            // "To the end of the document" is spelled as a line one past the last — a count of
            // lines is one more than the last index — and some servers spell it `u32::MAX`
            // instead. Both mean the end, and the end is a place this file certainly has.
            None => last,
        };
        let start = start.min(last);
        converted.push(BufferEdit {
            start,
            // Clamped forward rather than refused, as the rename clamps a backwards span: read
            // as zero-width it becomes an insertion, which is a thing a formatter really does
            // ask for — a blank line between two functions is an edit that deletes nothing.
            end: end.min(last).max(start),
            line: edit.start_line,
            new_text: edit.new_text.clone(),
        });
    }
    converted.sort_by_key(|e| (e.start, e.end));
    // Two edits over the same characters would make the result depend on which was applied
    // first, and the rebuild has no order that is more right than the other. Adjacent is fine —
    // one ending exactly where the next begins is two runs of text, not one.
    if converted.windows(2).any(|pair| pair[1].start < pair[0].end) {
        return Err(FormatRefusal::Overlap);
    }
    Ok(converted)
}

/// The rows one file contributes to a rename preview: for every line an edit touches, the line as
/// it is and the line as it would be.
///
/// Diff-shaped on purpose, and not only for the look of it: the git panel already colours a line
/// starting with `-` or `+` and a header starting with `---`, so the preview is read by the one
/// pair of eyes that has read every other diff, and drawn by the code that already draws them.
/// The marker carries a space after it so a line of code that itself begins with `--` cannot be
/// mistaken for a file header.
///
/// Several edits on one line collapse into a single pair. Two occurrences of a name on one line
/// are one line changing, and showing it twice — once per edit, each time with the *other*
/// occurrence still in its old spelling — would be showing two intermediate states that never
/// exist.
///
/// `edits` must be ascending and non-overlapping, which is what the caller has just checked.
/// The rows one code action answer becomes.
///
/// A free function so the list can be built from a plan and read back without an application
/// around it, which is the only part of this feature that has a shape worth checking on its own:
/// what a row says, and that the action it carries is the action that was on it.
///
/// The kind goes in the right-hand column the palette uses for chords, exactly as the outline puts
/// the kind of a symbol there — it is the same job, the part of the row you read second, and it is
/// what tells a quick fix for the error under the caret from a refactoring that would apply
/// anywhere. Left off entirely where the server did not say, rather than filled with a word of
/// ours: an empty column is honest and `action` would be a label we invented.
/// What is selected, as absolute char offsets, or `None` for nothing.
///
/// A free function because the expansion walk asks it of the buffer twice for different reasons —
/// once to know what to grow out of, once to know whether the selection on screen is still its own
/// — and the two must be the same measurement or the walk would decide it had been overtaken by
/// its own last move. A column selection answers `None`: a rectangle is not a run of text, and the
/// thing that encloses it is not a question a language server has been asked.
fn selected_char_range(editor: &Editor) -> Option<(usize, usize)> {
    if editor.selection_block {
        return None;
    }
    let ((start_line, start_col), (end_line, end_col)) = editor.selection_range()?;
    let at = |line: usize, col: usize| {
        editor.rope.line_to_char(line) + col.min(editor.line_char_len(line))
    };
    Some((at(start_line, start_col), at(end_line, end_col)))
}

fn code_action_items(actions: Vec<crate::lsp::CodeAction>) -> Vec<crate::picker::PickItem> {
    actions
        .into_iter()
        .map(|action| crate::picker::PickItem {
            // A title is one line as far as a server is concerned and several as far as a list is:
            // the breaks become spaces, as a diagnostic's do in the list beside this one.
            label: action.title.replace('\n', " "),
            shortcut: (!action.kind.is_empty()).then(|| action.kind.clone()),
            action: crate::picker::PickAction::CodeAction(Box::new(action)),
        })
        .collect()
}

fn preview_rows(editor: &Editor, lines: &[String], edits: &[BufferEdit]) -> Vec<String> {
    diff_rows(lines, edits, |line| editor.rope.line_to_char(line))
}

/// The same rows, told where each line begins rather than being handed a buffer.
///
/// Split from [`preview_rows`] for the reason [`rebuild_edits`] is split from
/// [`edits_as_one_span`]: the sweep across the project draws these rows for files that have no
/// rope, only a `String` and the line offsets counted while walking it. One diff, drawn once.
fn diff_rows(
    lines: &[String],
    edits: &[BufferEdit],
    line_start_of: impl Fn(usize) -> usize,
) -> Vec<String> {
    let mut rows = Vec::new();
    let mut at = 0usize;
    while at < edits.len() {
        let line = edits[at].line;
        let end = at + edits[at..].iter().take_while(|e| e.line == line).count();
        let old: Vec<char> = lines.get(line).map(|l| l.chars().collect()).unwrap_or_default();
        let line_start = line_start_of(line);
        let mut new = String::new();
        let mut carried = 0usize;
        for edit in &edits[at..end] {
            let from = edit.start.saturating_sub(line_start).min(old.len()).max(carried);
            let to = edit.end.saturating_sub(line_start).min(old.len()).max(from);
            new.extend(&old[carried..from]);
            new.push_str(&edit.new_text);
            carried = to;
        }
        new.extend(&old[carried..]);
        rows.push(format!("- {}", old.iter().collect::<String>()));
        rows.push(format!("+ {new}"));
        at = end;
    }
    rows
}

/// One file walked for a sweep: its lines, where each of them begins, and every replacement.
///
/// The line starts are kept because the diff rows need them and counting them twice is counting
/// them differently once. They are char offsets, like everything a [`BufferEdit`] carries.
struct FileScan {
    lines: Vec<String>,
    line_starts: Vec<usize>,
    edits: Vec<BufferEdit>,
}

/// Every match in `text`, line by line, as the replacements one file would receive.
///
/// A re-scan rather than a reading of the search's own hits, and that is not waste: a
/// [`crate::search::Hit`] is *a line*, with the match's start and nothing about its end, and one
/// hit per line however many times the line matches. That shape is exactly right for a list of
/// places to go and useless for a list of characters to replace, so the query is asked again —
/// through the same [`crate::find::compile`], with the same flags — of the text as it is now.
///
/// Line by line, because that is how the search matched: `^` holds at the start of every line in
/// a project search, and a sweep whose anchors meant something else would replace text the list
/// never offered. Groups are resolved against the line for the same reason, by
/// [`crate::find::expand_at`].
///
/// `regex` says whether the query was a pattern. A literal query has no groups, so its
/// replacement is written out verbatim and a `$1` in it stays three characters — the rule the
/// Find box already follows.
///
/// A line the pattern gave up on — the backtrack limit — keeps whatever was found before it gave
/// up and moves to the next line, which is how the search treats the same line.
fn scan_for_replacements(
    text: &str,
    re: &fancy_regex::Regex,
    template: &str,
    regex: bool,
) -> FileScan {
    let mut scan = FileScan { lines: Vec::new(), line_starts: Vec::new(), edits: Vec::new() };
    // Char offset of the current line's first character. `lines()` drops the terminator, so it
    // is added back by hand — and the text reaching here is always normalized to `\n`, which is
    // what makes that one character rather than a guess.
    let mut line_start = 0usize;
    for (number, line) in text.lines().enumerate() {
        scan.lines.push(line.to_string());
        scan.line_starts.push(line_start);
        // Walked by byte offset, recorded by char index, exactly as the Find box walks a buffer:
        // the engine counts in bytes and everything downstream of here counts in characters.
        let mut byte = 0usize;
        let mut chars_before = 0usize;
        while let Ok(Some(m)) = re.find_from_pos(line, byte) {
            let start_char = chars_before + line[byte..m.start()].chars().count();
            let end_char = start_char + line[m.start()..m.end()].chars().count();
            scan.edits.push(BufferEdit {
                start: line_start + start_char,
                end: line_start + end_char,
                line: number,
                new_text: match regex {
                    true => crate::find::expand_at(re, line, m.start(), template),
                    false => template.to_string(),
                },
            });
            // An empty match — `a*`, or `^` — matches without consuming anything, so the walk is
            // moved on by hand or it never ends.
            let next = if m.end() > m.start() {
                m.end()
            } else {
                match line[m.start()..].chars().next() {
                    Some(c) => m.start() + c.len_utf8(),
                    None => break,
                }
            };
            chars_before = start_char + line[m.start()..next].chars().count();
            byte = next;
        }
        line_start += line.chars().count() + 1;
    }
    scan
}

/// Which road a file takes through a sweep, and the text the sweep is going to scan.
///
/// The single decision the whole feature turns on, in one place so it cannot be made twice and
/// differently. `held` is the tab that has this file open, if any, and its presence settles it:
/// a file with a tab is scanned and edited *through the rope*, always, and the text scanned is
/// the text on screen — a dirty buffer included, since what somebody has typed is what they mean
/// to replace in. See [`SweepTarget`] for what a disk write under an open tab actually costs.
///
/// `None` for a file with no tab that is not readable text, which is the same test the search
/// made on the way past.
fn sweep_text_and_target(held: Option<&Editor>, path: &Path) -> Option<(String, SweepTarget)> {
    match held {
        Some(editor) => {
            Some((editor.rope.to_string(), SweepTarget::OpenBuffer { revision: editor.revision() }))
        }
        None => read_for_sweep(path),
    }
}

/// A file with no tab, read as text to scan plus everything needed to write it back the way it
/// was found.
///
/// `None` for anything that is not text, which is the same test the search made on the way past:
/// reading as UTF-8 and failing is the cheapest way to skip a picture without opening it to ask.
fn read_for_sweep(path: &Path) -> Option<(String, SweepTarget)> {
    let mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
    let raw = std::fs::read_to_string(path).ok()?;
    let target = SweepTarget::Disk {
        mtime,
        line_ending: match raw.contains("\r\n") {
            true => crate::editor::LineEnding::Crlf,
            false => crate::editor::LineEnding::Lf,
        },
        final_newline: raw.ends_with('\n'),
    };
    // Normalized to `\n` for the scan, exactly as `Editor::open` normalizes a buffer. Without it
    // a `$` would match before a `\r` nobody typed, and every char offset past the first line
    // would be one out.
    Some((raw.replace("\r\n", "\n"), target))
}

/// A swept file's text put back the way the file was found.
///
/// The two steps at the end of [`Editor::save`], in the same order and for the same reason: the
/// final newline the file did or did not have, then the line ending it did or did not use. A
/// sweep that grew a trailing newline, or turned a CRLF file into an LF one, would show up in
/// review as every line in the file having changed — which is the one thing a replacement of
/// three words must not look like.
///
/// Mirrored rather than shared with `save`, deliberately. `save` writes a *buffer* — it also
/// clears the dirty flag and re-reads the mtime, neither of which means anything here — and
/// prying those two lines out of it would make the most safety-critical function in the editor
/// depend on this one.
fn text_for_disk(
    mut text: String,
    line_ending: crate::editor::LineEnding,
    final_newline: bool,
) -> String {
    if !final_newline && text.ends_with('\n') {
        text.pop();
    }
    if line_ending == crate::editor::LineEnding::Crlf {
        text = text.replace('\n', "\r\n");
    }
    text
}

fn located_label(root: &Path, path: &Path, line: usize, text: Option<&str>) -> String {
    let shown = path.strip_prefix(root).unwrap_or(path);
    match text {
        Some(text) => format!("{}:{}  {}", shown.display(), line, crate::search::shorten(text)),
        None => format!("{}:{}", shown.display(), line),
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

/// How many files the quick-open picker and the project search will look at. The list is built
/// on the frame thread, so it has to end somewhere; past this a name is found by typing it into
/// the search rather than by scrolling a list the length of a build tree.
const PROJECT_FILE_LIMIT: usize = 8000;

/// How deep the fallback walk goes before it stops descending.
///
/// Nothing anybody edits is thirty directories deep, so the cap only ever fires on a tree that
/// contains a way back to itself. That case has to be caught rather than survived: a recursion
/// deep enough to exhaust the stack does not panic, it aborts — the shield that keeps a bug
/// from closing the window never sees it, and every shell in the session goes with it.
const PROJECT_WALK_DEPTH: usize = 32;

/// Names the project list never offers: the VCS's own store, and the two build outputs that are
/// bigger than the source tree they sit in. Dotfiles join them unless they were asked for.
fn skipped_component(name: &str, show_hidden: bool) -> bool {
    name == ".git"
        || name == "target"
        || name == "node_modules"
        || (!show_hidden && name.starts_with('.'))
}

/// Collects files under `root` for the quick-open picker and the project search, capped at
/// `PROJECT_FILE_LIMIT`. Returns whether it stopped early — a list that quietly ends is one you
/// will read "nothing here" off once too often, when the truth was "nothing in the part I got to".
///
/// In a git repository the list comes from git, which already knows what is generated and what
/// is checked in: a project whose `.gitignore` mentions `build/` or `.venv` gets those left out
/// here too, instead of drowning the picker in them and reaching the cap on artefacts. Shelling
/// out is this project's idiom for asking git anything, and it is the reason no dependency here
/// has to grow its own ignore-file dialect. Anything else — not a repo, no git installed, a git
/// that failed — falls back to the walk.
pub fn collect_project_files(root: &std::path::Path, out: &mut Vec<PathBuf>, show_hidden: bool) -> bool {
    if let Some(truncated) = git_project_files(root, out, show_hidden) {
        return truncated;
    }
    walk_project_files(root, out, show_hidden, 0)
}

/// What the quick-open box calls itself. It says so when the list is only part of the project:
/// otherwise a name that is simply past the cap looks exactly like a name that is not there.
fn file_picker_title(lang: Lang, truncated: bool) -> &'static str {
    match truncated {
        true => i18n::t(lang, Key::PickerOpenFileCapped),
        false => i18n::t(lang, Key::PickerOpenFile),
    }
}

/// The file list as git sees it: everything tracked plus everything untracked that is not
/// ignored. `None` when git could not answer, which is the caller's cue to walk the tree.
fn git_project_files(root: &std::path::Path, out: &mut Vec<PathBuf>, show_hidden: bool) -> Option<bool> {
    let listed = std::process::Command::new("git")
        .current_dir(root)
        .args(["ls-files", "--cached", "--others", "--exclude-standard", "-z"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let room = PROJECT_FILE_LIMIT.saturating_sub(out.len());
    let (names, truncated) = git_listed_names(&listed.stdout, show_hidden, room);
    for name in names {
        let path = root.join(name);
        // The index still names a file deleted since the last commit, and names a submodule as
        // if it were a file. Both would open as an empty buffer, so both are dropped here.
        if path.is_file() {
            out.push(path);
        }
    }
    Some(truncated)
}

/// Splits what `git ls-files -z` printed into the paths worth offering.
///
/// NUL-separated on purpose: a file name may contain anything but a NUL — a space, a newline, a
/// quote — and git's default output escapes non-ASCII names into C string literals, which is a
/// spelling no path ever comes back from. Split on NUL there is nothing to unescape.
fn git_listed_names(stdout: &[u8], show_hidden: bool, room: usize) -> (Vec<PathBuf>, bool) {
    let mut names = Vec::new();
    for raw in stdout.split(|&b| b == 0).filter(|s| !s.is_empty()) {
        // git prints `/` on every platform, so this is the separator to split on whatever the
        // host calls one. A name that is not UTF-8 is skipped rather than guessed at.
        let Ok(name) = std::str::from_utf8(raw) else { continue };
        if name.split('/').any(|part| skipped_component(part, show_hidden)) {
            continue;
        }
        if names.len() >= room {
            return (names, true);
        }
        names.push(PathBuf::from(name));
    }
    (names, false)
}

/// The walk used where git cannot answer. Returns whether it stopped early.
fn walk_project_files(
    root: &std::path::Path,
    out: &mut Vec<PathBuf>,
    show_hidden: bool,
    depth: usize,
) -> bool {
    if out.len() >= PROJECT_FILE_LIMIT || depth >= PROJECT_WALK_DEPTH {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(root) else { return false };
    let mut truncated = false;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if skipped_component(&name, show_hidden) {
            continue;
        }
        // The kind comes off the directory entry itself, so this is not a second look at the
        // disk, and it describes the link rather than what the link points at — which is the
        // whole point: a link is never descended into. `ln -s .. loop` inside a project makes a
        // tree with no bottom, and the same file reached under two names is one file listed
        // twice. A link *to* a file is still offered, since opening it opens what it names.
        let Ok(kind) = entry.file_type() else { continue };
        let path = entry.path();
        if kind.is_symlink() {
            if path.is_file() {
                out.push(path);
            }
        } else if kind.is_dir() {
            truncated |= walk_project_files(&path, out, show_hidden, depth + 1);
        } else if kind.is_file() {
            out.push(path);
        }
        if out.len() >= PROJECT_FILE_LIMIT {
            return true;
        }
    }
    truncated
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
    /// A button held down over a pane whose program asked for the mouse: the pane's index and the
    /// button number it was told about, so the drag and the release it eventually gets are
    /// reported as the same button in the same pane the press happened in.
    TerminalMouse(usize, u16),
    /// The seam between the editor side of the window and the agent drawer.
    DrawerWidth,
    /// A button held on the drawer's left edge that has not moved yet.
    ///
    /// That column is two controls at once: the width seam, and the handle that closes the
    /// drawer. Nothing about *where* the press landed can tell them apart — they are the same
    /// cells — so what tells them apart is what the hand does next. A press that moves is a
    /// resize; a press that comes back up without having moved is a click, and a click there
    /// closes. So this variant arms neither: the real [`DragTarget::DrawerWidth`] is entered on
    /// the first motion event, and a release that still finds this here is the click.
    ///
    /// Deferring the drag is only done for this edge. Everywhere else a press on a seam is a
    /// grab and nothing else, and making them all wait for a movement would be a lag on every
    /// resize in the window to pay for a control only this one has.
    ///
    /// `on_handle` remembers whether the press was on the border column itself, because only
    /// that column closes: a seam accepts the cell either side of its border as aiming
    /// tolerance, and here the cell to the right is the agent's own pane and the one to the left
    /// belongs to whatever frame the drawer is standing next to. Closing the drawer from either
    /// would be a click landing on something the user was not pointing at.
    DrawerEdgePress { on_handle: bool },
    /// A text selection being dragged inside the drawer's pane. No index: there is only one.
    DrawerSelection,
    /// A button the drawer's agent was told went down, and has to be told came back up.
    DrawerMouse(u16),
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
    /// The agent drawer's bar. Not `Terminal(n)` for any `n`: the drawer is not one of the
    /// panel's windows, and a bar that shared an id with one of them would be dragged by the
    /// pointer hovering over the other.
    Drawer,
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
    /// Delta in percent for `drawer_pct`.
    Drawer(i16),
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
    /// Whether the agent drawer has a column right now. It is the rightmost one when it does,
    /// which is the third arrangement the two resolvers below have to know about.
    pub drawer_open: bool,
    /// Whether the debug panel has one. It sits between the frames and the drawer, so it is the
    /// rightmost column whenever the drawer is away and the one before it when it is not.
    pub debug_open: bool,
}

/// Where a directional move lands. A "frame" for this purpose is finer-grained than `Focus`:
/// the two halves of a split editor and each tiled terminal window are places you can be, and
/// an arrow should reach them the same way it reaches the sidebar.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum FocusTarget {
    Tree,
    Editor(EditorPane),
    Terminal(usize),
    Debug,
    Drawer,
}

/// Where the agent `Ctrl+Shift+A` is talking to lives.
///
/// An enum rather than an index because the drawer has no index: it is not one of the terminal
/// panel's windows, and the number that used to be this answer would have had to mean "not a
/// number" for one of its values.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AgentPane {
    /// The drawer. Checked first, always.
    Drawer,
    /// Terminal window `i`, by its place on screen.
    Terminal(usize),
}

/// What applying a workspace does to the drawer.
///
/// Pure, and short, because the interesting thing about it is what it *cannot* say. There is no
/// variant that rebuilds a drawer that already exists: `rebuild_terminals` drains and replaces
/// every terminal window on a workspace switch, and the drawer's promise is the opposite one —
/// an agent you are mid-conversation with survives opening another project. A workspace governs
/// the column, and only the column.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DrawerFromWorkspace {
    /// Nothing to do. A file that says nothing about a drawer has no opinion about one — it may
    /// simply predate the field — and acting on an opinion nobody expressed would put away a
    /// panel somebody was using.
    LeaveAlone,
    /// A drawer already exists: open or close its column, at this width. Its pane is untouched.
    SetOpen { open: bool, width: u16 },
    /// No drawer yet and the workspace wants one open. Nothing is being replaced here, because
    /// there was nothing to replace. `agent` is `None` where the file named nobody, or named
    /// somebody we do not know — the launcher is the honest answer to `agent = "clod"`.
    Summon { agent: Option<crate::session::Agent>, width: u16 },
}

pub fn drawer_from_workspace(
    saved: Option<&crate::workspace::WorkspaceDrawer>,
    have_one: bool,
) -> DrawerFromWorkspace {
    let Some(saved) = saved else { return DrawerFromWorkspace::LeaveAlone };
    if have_one {
        return DrawerFromWorkspace::SetOpen { open: saved.open, width: saved.width };
    }
    if !saved.open {
        // Nothing to open and nothing to close: a closed drawer that does not exist is a drawer
        // that does not exist.
        return DrawerFromWorkspace::LeaveAlone;
    }
    DrawerFromWorkspace::Summon {
        agent: saved.agent.as_deref().and_then(crate::session::Agent::of_program),
        width: saved.width,
    }
}

/// Which pane the context goes to, out of everything that claims to hold an agent.
///
/// Four claims, in this order, and the order is the whole function — which is why it is pure and
/// pinned by a test rather than woven into the pty walking that produces the claims.
///
/// The drawer comes first because it is the panel that exists to hold an agent: with one there,
/// it is the one you meant, even with another agent at a prompt two panes away. Within each
/// place, a *running* process beats a *declared* startup command, because a pane whose agent has
/// since exited is a shell, and a shell is not an agent — that precedence predates the drawer and
/// is not changed by it.
pub fn agent_precedence(
    drawer_running: Option<crate::session::Agent>,
    drawer_declared: Option<crate::session::Agent>,
    terminal_running: Option<(usize, crate::session::Agent)>,
    terminal_declared: Option<(usize, crate::session::Agent)>,
) -> Option<(AgentPane, crate::session::Agent)> {
    drawer_running
        .or(drawer_declared)
        .map(|agent| (AgentPane::Drawer, agent))
        .or_else(|| {
            terminal_running
                .or(terminal_declared)
                .map(|(index, agent)| (AgentPane::Terminal(index), agent))
        })
}

/// The frame that lies in the given direction, or `None` at the edge of the window.
///
/// Navigation is spatial rather than by category: you press the direction the thing you want is
/// in, and it does not matter whether that thing is a file tree, an editor pane or a shell. The
/// layout has two arrangements, and the agent drawer adds a column to the right of either:
///
/// ```text
///   terminals below (classic)        terminals on the right        with the drawer open
///   ┌──────┬──────────────┐          ┌──────┬─────────┬──────┐     ┌──────┬───────┬──────┐
///   │ tree │ editor       │          │ tree │ editor  │ term │     │ tree │ editor│ dr   │
///   ├──────┴──────────────┤          │      │         ├──────┤     ├──────┴───────┤ aw   │
///   │ term │ term         │          │      │         │ term │     │ term │ term  │ er   │
///   └──────┴──────────────┘          └──────┴─────────┴──────┘     └──────┴───────┴──────┘
///   windows side by side             windows stacked               always the rightmost
/// ```
///
/// The terminal strip spans the full width in the classic layout, which is why its windows are
/// walked with left/right there and with up/down when the panel is a column instead.
///
/// The drawer is always the last column, so Right reaches it from whatever the rightmost frame
/// would otherwise have been and Left leaves it for that same frame. It has no up or down: a
/// column that fills the height has nothing above or below it to go to.
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
            // Only once nothing nearer has claimed Right: the debug panel and then the drawer are
            // beyond the terminal column, not instead of it, and they are in that order because
            // that is the order they are carved in.
            Right if l.debug_open => Some(FocusTarget::Debug),
            Right if l.drawer_open => Some(FocusTarget::Drawer),
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
                // Past the last window along the tiling axis there is the drawer, or the window
                // edge. In the right-docked layout the axis is vertical and Right is not it, so
                // Right falls through to the arm below and reaches the drawer directly.
                s if s == next => (l.terminal_index < last_window)
                    .then_some(FocusTarget::Terminal(l.terminal_index + 1))
                    .or(l.debug_open.then_some(FocusTarget::Debug))
                    .or(l.drawer_open.then_some(FocusTarget::Drawer)),
                s if s == leave => back_to_editor,
                Right if l.debug_open => Some(FocusTarget::Debug),
                Right if l.drawer_open => Some(FocusTarget::Drawer),
                _ => None,
            }
        }
        // The debug panel: Left is the way back into the frames it took its column from, and
        // Right reaches the drawer, which is the only thing that can be beyond it. Like the
        // drawer it spans the full height, so up and down have nowhere to go.
        Focus::Debug => match side {
            Left if l.show_terminal && l.terminal_on_right => {
                Some(FocusTarget::Terminal(l.terminal_index.min(last_window)))
            }
            Left => Some(FocusTarget::Editor(if l.split_view {
                EditorPane::Right
            } else {
                EditorPane::Left
            })),
            Right if l.drawer_open => Some(FocusTarget::Drawer),
            _ => None,
        },
        // Left is the way out, into whatever the drawer is sitting beside: the debug panel where
        // there is one, then the terminal panel where it is the right-hand column, the editor
        // otherwise. Nothing else moves — the drawer spans the full height, so up and down have
        // nowhere to go.
        Focus::Drawer => match side {
            Left if l.debug_open => Some(FocusTarget::Debug),
            Left if l.show_terminal && l.terminal_on_right => {
                Some(FocusTarget::Terminal(l.terminal_index.min(last_window)))
            }
            Left => Some(FocusTarget::Editor(if l.split_view {
                EditorPane::Right
            } else {
                EditorPane::Left
            })),
            _ => None,
        },
    }
}

const SIDEBAR_STEP: i16 = 2;
const TERMINAL_STEP: i16 = 5;
/// The drawer moves in the same percentage steps the terminal panel does, because it is the same
/// kind of scalar and a seam that moved at a different speed from the one beside it would feel
/// like two different controls.
const DRAWER_STEP: i16 = 5;
const SPLIT_STEP: i16 = 5;
/// A tenth of the default weight: ten nudges take a window from its share to a neighbour's.
const WEIGHT_STEP: i16 = 100;

/// Resolves an arrow nudge on the focused frame to the seam it moves. `None` when the named
/// border coincides with the window edge — there is nothing there to drag. `grow` pushes the
/// border outward (the frame gets bigger); `!grow` pulls it inward.
///
/// The layout has four movable seams — sidebar↔editor, editor↔terminal, (in split view)
/// editor-left↔editor-right, and (with the drawer open) everything↔drawer — so every frame has
/// at most two of them, always on sides that the arrow keys can tell apart. The drawer's is the
/// window's rightmost seam, reachable from the drawer itself and from whichever frame it took
/// its column from.
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
                    // Past the last window there is no neighbour to trade weight with — but in
                    // the classic layout the strip's right end is the drawer's seam, and that
                    // one does move.
                    return (l.drawer_open && side == Right)
                        .then_some(ResizeCmd::Drawer(-s * DRAWER_STEP));
                }
                let delta = if toward_next { s * WEIGHT_STEP } else { -s * WEIGHT_STEP };
                return Some(ResizeCmd::TerminalWeight { seam, delta });
            }
            // The terminal touches the editor on exactly one side, set by its orientation.
            match (l.terminal_on_right, side) {
                (true, Left) => Some(ResizeCmd::Terminal(s * TERMINAL_STEP)),
                (false, Up) => Some(ResizeCmd::Terminal(s * TERMINAL_STEP)),
                // Docked right, the panel's other side is the drawer's seam: growing the
                // terminal takes the columns from the drawer, which is the frame on the far
                // side of it.
                (true, Right) if l.drawer_open => Some(ResizeCmd::Drawer(-s * DRAWER_STEP)),
                _ => None,
            }
        }
        // One seam, on its left, wherever the drawer's column was carved from. Growing the
        // drawer widens it, which is the direction the border moves.
        Focus::Drawer => match side {
            Left => Some(ResizeCmd::Drawer(s * DRAWER_STEP)),
            _ => None,
        },
        // The debug panel has no seam of its own. Its width is a share of the window worked out
        // by the layout rather than a setting somebody nudges, because it is a panel you open at
        // a breakpoint and close again — a scalar to remember for it would be a preference nobody
        // would ever have a reason to set twice.
        Focus::Debug => None,
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
                // With no terminal column between them, the editor's right border *is* the
                // drawer's seam; growing the editor takes the columns from the drawer.
                Right if terminal_far && l.drawer_open => Some(ResizeCmd::Drawer(-s * DRAWER_STEP)),
                Down if terminal_far && l.show_terminal && !l.terminal_on_right => {
                    Some(ResizeCmd::Terminal(-s * TERMINAL_STEP))
                }
                _ => None,
            }
        }
    }
}

/// Extends a pane's selection by one cell, anchoring it at the terminal's own cursor the first
/// time there is nothing to extend.
///
/// A free function rather than a method because two frames now select this way — the terminal
/// panel, addressed by index, and the drawer, which has no index — and the only thing they did
/// not share was how to reach the pane.
fn extend_pane_selection(term: &mut TerminalPanel, d_row: i16, d_col: i16) {
    let from = match term.selection {
        Some(selection) => selection.cursor,
        None => {
            let cursor = term.cursor_cell();
            term.begin_selection(cursor);
            cursor
        }
    };
    let next = (from.0.saturating_add_signed(d_row), from.1.saturating_add_signed(d_col));
    term.extend_selection(next);
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

/// How many tabs follow mode may open in one session.
///
/// A ceiling rather than a rotation: none of them is ever closed again by the editor. An agent
/// working through a refactor touches thirty files, and a strip of thirty tabs — one of which
/// you were reading — is not a view of anything. Five is about what fits in a tab strip and
/// stays legible; past that the status line says so and the Git panel is the place to see the
/// rest, which is what it has always been for.
const FOLLOW_TAB_LIMIT: usize = 5;

/// How many files may be waiting their turn to be shown. Insurance rather than policy: the queue
/// drains every sweep and can only ever produce `FOLLOW_TAB_LIMIT` tabs, but a `git checkout` of
/// a branch that differs by ten thousand files should not be held in memory to be ignored.
const FOLLOW_QUEUE_LIMIT: usize = 64;

/// Whether two paths name the same file, allowing for the several spellings one file has here:
/// `git status` is keyed the way the file tree spells its rows (`./src/main.rs` when the project
/// was opened as `.`), and a tab may hold the absolute path the picker gave it.
fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || match (a.canonicalize(), b.canonicalize()) {
            (Ok(x), Ok(y)) => x == y,
            _ => false,
        }
}

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

/// How many lines of a debug session's output are kept.
///
/// A ring and not a transcript, and capped here rather than wherever it happens to be drawn: a
/// debuggee is free to print a megabyte a second, and a session left running overnight must not
/// grow until the editor is the thing that dies. Five hundred is about ten screenfuls — enough to
/// scroll back through what the program said before it stopped, which is what anybody looks for.
const DEBUG_OUTPUT_LINES: usize = 500;

/// One debug session: an adapter, what it was pointed at, and what it has said since.
///
/// At most one exists at a time. That is a decision and not a limitation of the wire: a second
/// session would need a second stopped line, and the editor has exactly one idea of where a
/// program is stopped — the same [`App::stopped_at`] the interpreter debugger has always used.
pub struct DebugSession {
    client: crate::dap::Client,
    /// What is being debugged. Kept because nothing else remembers it once the launch has gone
    /// out, and the sentence said when the session ends names it.
    program: PathBuf,
    /// The arguments it was launched with — none, until something can ask for them — and the
    /// directory it runs in, which is the project root.
    ///
    /// Both are held rather than dropped after the launch because they are what the session *is*:
    /// a restart is the same three answers sent again, and the design's prompt asks for all three
    /// rather than only the program. Only the program is asked for so far — see
    /// [`App::open_debug_start`] — so nothing reads these two yet, which is why the compiler is
    /// told so here rather than left to warn about a field somebody would then be tempted to
    /// delete and have to invent again.
    #[allow(dead_code, reason = "a restart re-sends them, and the prompt will grow to ask for them")]
    args: Vec<String>,
    #[allow(dead_code, reason = "a restart re-sends them, and the prompt will grow to ask for them")]
    cwd: PathBuf,
    /// The thread the debuggee is stopped on, which is the thread every step names. `None` while
    /// it runs — which is also what makes "not stopped" a question this can answer honestly
    /// rather than a step sent into a running program.
    thread: Option<i64>,
    /// Which files the adapter has been told about.
    ///
    /// Kept because `setBreakpoints` replaces one file's whole list: a file whose last breakpoint
    /// is taken off leaves [`App::breakpoints`] entirely, and an adapter never told about the
    /// empty list would go on stopping at a breakpoint the editor no longer draws.
    published: std::collections::BTreeSet<PathBuf>,
    /// The `stackTrace` this session is waiting on to learn where it stopped, when the stopped
    /// event did not say. Held by seq so that an answer to some older question is ignored rather
    /// than read as this one's.
    awaiting_place: Option<i64>,
    /// The `threads` asked for when a stop named no thread at all, which the protocol allows.
    awaiting_thread: Option<i64>,
    /// The `threads` asked for because somebody wants the program caught where it is: DAP's
    /// `pause` names a thread, and a *running* program is precisely the one that has not told us
    /// about one. Held by seq, like the two above, so that this question and the one above cannot
    /// be answered by each other's reply — they arrive as the same event.
    awaiting_pause: Option<i64>,
    /// What the debuggee and the adapter have printed, oldest first, capped at
    /// [`DEBUG_OUTPUT_LINES`]. The category — `stdout`, `stderr`, `console` — travels with each
    /// line rather than being reduced to a flag here: which of them is worth showing is a
    /// decision about the panel, and the panel is wave 3.
    output: std::collections::VecDeque<(String, String)>,
}

impl DebugSession {
    fn new(client: crate::dap::Client, program: PathBuf, args: Vec<String>, cwd: PathBuf) -> Self {
        DebugSession {
            client,
            program,
            args,
            cwd,
            thread: None,
            published: std::collections::BTreeSet::new(),
            awaiting_place: None,
            awaiting_thread: None,
            awaiting_pause: None,
            output: std::collections::VecDeque::new(),
        }
    }

    /// The last `rows` lines the session printed, oldest first.
    ///
    /// Copied out rather than borrowed because the panel draws it beside things it reads from the
    /// panel's own state, and one `&self` borrow of the session that lived across the whole of the
    /// drawing would make the rest of it unreachable. A handful of short strings a frame is not a
    /// cost worth a lifetime for.
    pub fn output_tail(&self, rows: usize) -> Vec<String> {
        self.output.iter().rev().take(rows).rev().map(|(_, line)| line.clone()).collect()
    }

    /// Remembers one line the session printed, dropping the oldest once the ring is full.
    fn remember_output(&mut self, category: String, text: String) {
        // Split, because an adapter is entitled to hand over a whole paragraph in one event and a
        // ring of "lines" that holds twelve of them at once is not a ring of lines.
        for line in text.lines() {
            if self.output.len() >= DEBUG_OUTPUT_LINES {
                self.output.pop_front();
            }
            self.output.push_back((category.clone(), line.to_string()));
        }
    }
}

/// Which of the five things a running session can be asked to do. Menu rows and palette entries
/// come in as [`MenuAction`]s; this is the same five with everything that is not about the
/// debugger taken off, so the one place that checks "is there a session, and is it stopped"
/// answers for all of them at once.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DebugVerb {
    Continue,
    StepOver,
    StepIn,
    StepOut,
}

/// One watch: an expression somebody typed, and whatever the adapter last made of it.
///
/// The answer is a `Result` and not a string because the two outcomes are drawn differently and
/// mean opposite things: a value is what the expression *is*, and a refusal is the adapter saying
/// it cannot read that here — which is the ordinary answer for a watch on a local that is not in
/// scope in the frame you happen to be looking at, and not an error anybody needs shouting about.
pub struct DebugWatch {
    pub expression: String,
    /// `None` until an answer has come back for it at all, which is what the panel says while a
    /// question is out rather than showing the previous frame's number under this frame's name.
    pub answer: Option<Result<String, String>>,
}

/// What the debug panel holds: where the arrows are, what the adapter has said about this stop,
/// and the watches — which are the one part of it that outlives the session.
///
/// Beside [`DebugSession`] on the `App` rather than inside it, and that placement is the design:
/// a session is dropped the moment the program exits, and a watch list that went with it would
/// mean retyping every expression after every run. The rest of what is here *is* about one stop
/// and is cleared whenever the program moves — see [`Self::forget_stop`].
#[derive(Default)]
pub struct DebugPanel {
    /// Whether the column is on screen. Opened by a session starting, closed by it ending, and
    /// turned on and off in between from the Debug menu.
    pub open: bool,
    /// Where the arrows are, counted over every row the panel draws — headings included, since
    /// they are what the rows are between. Rows are rebuilt each frame, so this is clamped where
    /// it is read rather than kept in range here.
    pub selected: usize,
    /// The stack of the thread the program is stopped on, innermost first.
    frames: Vec<crate::dap::Frame>,
    /// Which of them everything else on the panel is about. Not the same as [`Self::selected`]:
    /// moving the arrows over the list changes what is highlighted, and pressing Enter is what
    /// changes which frame the variables and the watches are read in.
    frame: usize,
    /// The scopes of that frame — "Locals", "Registers", whatever the adapter groups by.
    scopes: Vec<crate::dap::Scope>,
    /// What each reference turned out to hold, once it has been asked for. Keyed by the
    /// adapter's own `variablesReference`, which is the only handle there is for the question.
    children: std::collections::HashMap<i64, Vec<crate::dap::Variable>>,
    /// Which references are open. Cleared with everything else when the program moves, because
    /// the protocol invalidates every reference on a resume: keeping them would mean asking the
    /// adapter about handles it has already forgotten.
    expanded: std::collections::BTreeSet<i64>,
    /// The watch list, in the order it was typed.
    watches: Vec<DebugWatch>,
    /// The `scopes` question out for the selected frame, by seq, so a late answer about the frame
    /// somebody has already moved off is dropped rather than drawn under the new one's name.
    awaiting_scopes: Option<i64>,
    /// The `variables` questions out, each remembering which reference it was about.
    awaiting_children: std::collections::HashMap<i64, i64>,
    /// The `evaluate` questions out, each remembering which watch it was for.
    awaiting_watch: std::collections::HashMap<i64, usize>,
}

impl DebugPanel {
    /// Everything about one stop, forgotten. Called wherever the program starts moving again:
    /// the frames, the variables and the answers are all statements about a place it has left.
    ///
    /// The watch *expressions* survive — they are the question, not the answer — but their values
    /// do not, which is what stops the panel showing last stop's numbers as though they were now.
    fn forget_stop(&mut self) {
        self.frames.clear();
        self.frame = 0;
        self.scopes.clear();
        self.children.clear();
        self.expanded.clear();
        self.awaiting_scopes = None;
        self.awaiting_children.clear();
        self.awaiting_watch.clear();
        for watch in self.watches.iter_mut() {
            watch.answer = None;
        }
    }

    /// The frame every question about a value is asked in, or `None` before a stack has arrived.
    fn current_frame(&self) -> Option<&crate::dap::Frame> {
        self.frames.get(self.frame)
    }
}

/// Which kind of thing a panel row is, which is what decides both how it is drawn and what Enter
/// does to it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DebugRowKind {
    /// A section caption. Never selectable: there is nothing to do to a word.
    Heading,
    /// A dim sentence standing where rows would be — "running…", "w adds one". Also not
    /// selectable, for the same reason.
    Note,
    /// One frame of the stack, by its place in it.
    Frame { index: usize, current: bool },
    /// One variable or one scope. `reference` is non-zero when there is something inside it, and
    /// that is exactly the condition under which Enter opens it.
    Variable { reference: i64, expanded: bool },
    /// One watch, by its place in the list.
    Watch { index: usize },
}

/// One row of the debug panel, reduced to the three pieces a line is drawn from.
///
/// Built whole and pure, in the spirit of the workspace viewer's table: what the panel decides at
/// a given state is then a question that can be asked without a terminal to look at.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DebugRow {
    pub kind: DebugRowKind,
    /// How far in the row sits, one step per level of the variable tree.
    pub depth: usize,
    /// The left-hand text: a section's word, a frame's function, a variable's or a watch's name.
    pub label: String,
    /// The right-hand text: a frame's site, a value, a refusal. Empty where a row has none.
    pub value: String,
    /// The adapter's word for the type, drawn dim after the value. Variables only.
    pub type_name: Option<String>,
    /// Whether `value` is a refusal rather than an answer. Carried on the row rather than looked
    /// up again at drawing time, because it is the one thing about a row that its three strings
    /// cannot say — "no variable named x here" is a perfectly ordinary sentence, and a panel that
    /// painted it as a value would be reading the adapter's apology as data.
    pub failed: bool,
}

impl DebugRow {
    /// Whether the arrows may land here. Headings and notes are text, not rows.
    pub fn selectable(&self) -> bool {
        !matches!(self.kind, DebugRowKind::Heading | DebugRowKind::Note)
    }

    fn heading(label: &str) -> DebugRow {
        DebugRow {
            kind: DebugRowKind::Heading,
            depth: 0,
            label: label.to_string(),
            value: String::new(),
            type_name: None,
            failed: false,
        }
    }

    fn note(label: &str) -> DebugRow {
        DebugRow {
            kind: DebugRowKind::Note,
            depth: 0,
            label: label.to_string(),
            value: String::new(),
            type_name: None,
            failed: false,
        }
    }
}

/// Which of the debugger's two single-line questions is being asked.
///
/// One box for both, because they are the same box: a title, a line of prompt, and one line of
/// answer with the caret in it. Two flags and two input strings would have been two copies of the
/// same four call sites — the key chain, the paste chain, the drawing and the dismissal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DebugAsk {
    /// What to debug. Opened by *Debug ▸ Start debugging*, prefilled with the guess.
    Debuggee,
    /// One expression to watch. Opened by `w` in the panel, empty.
    Watch,
}

/// A single-line question from the debugger, and what has been typed at it so far.
pub struct DebugPrompt {
    pub ask: DebugAsk,
    pub typed: String,
}

/// Every row the panel draws, in the design's order: frames, then variables, then watches.
///
/// The output tail is not here. It is drawn in a strip of its own along the bottom, so that the
/// rows above it can scroll under the arrows without the program's last few printed lines
/// scrolling away with them — and keeping it out means every index in this list is stable
/// whatever the panel's height turns out to be.
pub fn debug_panel_rows(panel: &DebugPanel, stopped: bool, lang: i18n::Lang) -> Vec<DebugRow> {
    let mut rows = vec![DebugRow::heading(i18n::t(lang, i18n::Key::DebugFrames))];
    // One dim line instead of three sections of stale data. While the program is moving there is
    // no frame, no scope and no value: everything the panel could show is an answer about a place
    // it has left, and showing it would be the panel quietly lying.
    if !stopped {
        rows.push(DebugRow::note(i18n::t(lang, i18n::Key::DebugRunning)));
        return rows;
    }
    if panel.frames.is_empty() {
        rows.push(DebugRow::note(i18n::t(lang, i18n::Key::DebugAsking)));
    }
    for (index, frame) in panel.frames.iter().enumerate() {
        rows.push(DebugRow {
            kind: DebugRowKind::Frame { index, current: index == panel.frame },
            depth: 0,
            label: frame.name.clone(),
            value: frame_site(frame),
            type_name: None,
            failed: false,
        });
    }

    rows.push(DebugRow::heading(i18n::t(lang, i18n::Key::DebugVariables)));
    if panel.scopes.is_empty() {
        rows.push(DebugRow::note(i18n::t(lang, i18n::Key::DebugAsking)));
    }
    for scope in &panel.scopes {
        rows.push(DebugRow {
            kind: DebugRowKind::Variable {
                reference: scope.reference,
                expanded: panel.expanded.contains(&scope.reference),
            },
            depth: 0,
            label: scope.name.clone(),
            value: String::new(),
            type_name: None,
            failed: false,
        });
        push_children(&mut rows, panel, scope.reference, 1);
    }

    rows.push(DebugRow::heading(i18n::t(lang, i18n::Key::DebugWatches)));
    if panel.watches.is_empty() {
        rows.push(DebugRow::note(i18n::t(lang, i18n::Key::DebugNoWatches)));
    }
    for (index, watch) in panel.watches.iter().enumerate() {
        // The adapter's own sentence, unedited, where it refused. "There is no variable named x
        // here" is the useful thing to read, and an editor's paraphrase of it would be worse.
        let (value, failed) = match watch.answer.as_ref() {
            Some(Ok(value)) => (value.clone(), false),
            Some(Err(message)) => (message.clone(), true),
            None => (i18n::t(lang, i18n::Key::DebugAsking).to_string(), false),
        };
        rows.push(DebugRow {
            kind: DebugRowKind::Watch { index },
            depth: 0,
            label: watch.expression.clone(),
            value,
            type_name: None,
            failed,
        });
    }
    rows
}

/// The rows under one open reference, and under theirs, as far as they have been asked for.
///
/// Recursive because the tree is, and bounded by what is in `expanded`: nothing appears here that
/// somebody did not open, so the recursion is over what has already been fetched rather than over
/// what could be.
fn push_children(rows: &mut Vec<DebugRow>, panel: &DebugPanel, reference: i64, depth: usize) {
    if reference == 0 || !panel.expanded.contains(&reference) {
        return;
    }
    let Some(children) = panel.children.get(&reference) else { return };
    for child in children {
        rows.push(DebugRow {
            kind: DebugRowKind::Variable {
                reference: child.reference,
                expanded: panel.expanded.contains(&child.reference),
            },
            depth,
            label: child.name.clone(),
            value: child.value.clone(),
            type_name: child.type_name.clone(),
            failed: false,
        });
        push_children(rows, panel, child.reference, depth + 1);
    }
}

/// Where the cursor lands after one nudge: the next row it may stand on, in that direction.
///
/// Captions and notes are stepped over rather than landed on — Enter on the word "Variables"
/// would have nothing to do — and either end stops rather than wrapping, the way every other list
/// in this editor does: a cursor that jumped from the last watch back to the top of the stack
/// would read as the panel having scrolled rather than as the end of it.
pub fn debug_next_row(rows: &[DebugRow], here: usize, delta: i32) -> usize {
    let landable: Vec<usize> =
        rows.iter().enumerate().filter(|(_, r)| r.selectable()).map(|(i, _)| i).collect();
    let (Some(&first), Some(&last)) = (landable.first(), landable.last()) else { return here };
    if delta < 0 {
        landable.iter().rev().find(|&&i| i < here).copied().unwrap_or(first)
    } else {
        landable.iter().find(|&&i| i > here).copied().unwrap_or(last)
    }
}

/// Where a frame is, in the words a row has room for: the file's own name and the line.
///
/// The name and not the path, because the path is the project's and repeats on every row of the
/// stack — and because the panel is a narrow column, where the part that repeats is the part
/// worth dropping. A frame with no source says only its line, which is what the adapter knew.
fn frame_site(frame: &crate::dap::Frame) -> String {
    match frame.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned())
    {
        Some(name) => format!("{name}:{}", frame.line),
        None => String::new(),
    }
}

/// What one key means to a focused debug panel.
///
/// A pure resolver rather than a match inside the handler, for the reason every other key table
/// here is one: what these letters do is the design's own table, and a table is worth being able
/// to check without an editor to press them in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DebugPanelKey {
    /// One of the four the adapter is asked for.
    Verb(DebugVerb),
    Stop,
    AddWatch,
    DropWatch,
    /// The arrows: `-1` up, `1` down.
    Move(i32),
    /// Enter: jump to a frame, or open what is under the cursor.
    Act,
    /// Out of the panel, back to the text.
    Leave,
}

/// The design's table, and nothing global changes to make it work: these are plain letters, and
/// they are only ever asked about while this one frame holds the keyboard. Anything carrying
/// Ctrl, Alt or Super has already been claimed by the chord layer before the focus is consulted,
/// and is refused here as well so that a chord which grows a meaning later cannot be shadowed.
pub fn debug_panel_key(key: KeyEvent) -> Option<DebugPanelKey> {
    match key.code {
        KeyCode::Up => return Some(DebugPanelKey::Move(-1)),
        KeyCode::Down => return Some(DebugPanelKey::Move(1)),
        KeyCode::Enter => return Some(DebugPanelKey::Act),
        KeyCode::Esc => return Some(DebugPanelKey::Leave),
        _ => {}
    }
    let KeyCode::Char(c) = key.code else { return None };
    if !is_a_typed_character(key) {
        return None;
    }
    match c.to_ascii_lowercase() {
        'c' => Some(DebugPanelKey::Verb(DebugVerb::Continue)),
        'n' => Some(DebugPanelKey::Verb(DebugVerb::StepOver)),
        's' => Some(DebugPanelKey::Verb(DebugVerb::StepIn)),
        'o' => Some(DebugPanelKey::Verb(DebugVerb::StepOut)),
        'x' => Some(DebugPanelKey::Stop),
        'w' => Some(DebugPanelKey::AddWatch),
        'd' => Some(DebugPanelKey::DropWatch),
        _ => None,
    }
}

/// The `[package] name` out of a `Cargo.toml`, when there is one.
///
/// Parsed with the `toml` crate the settings already depend on rather than scanned for a line
/// starting with `name` — a workspace root has a `[workspace] members` list with names all over
/// it, and the hand-rolled version of this reads the wrong one on the first real project it meets.
fn cargo_package_name(text: &str) -> Option<String> {
    // A `Table` and not a `Value`: since toml 1.0 a bare `Value` parses one *value* and refuses a
    // whole document, so the obvious spelling of this line reads every manifest ever written as
    // broken — silently, since a manifest that will not parse and a manifest with no package in
    // it are the same "no guess" from out here.
    let parsed: toml::Table = text.parse().ok()?;
    parsed.get("package")?.get("name")?.as_str().map(str::to_string)
}

/// What *Debug ▸ Start* runs, in the order the design lays out.
///
/// The remembered answer first, because it is an answer somebody gave; then the Cargo guess,
/// which is right for the projects this editor is written in and written for; then the project
/// root, which is not an executable and is not meant to be — it is the honest "I do not know",
/// and the refusal that follows names it rather than starting something at random.
fn debuggee_for(root: &Path, remembered: Option<&Path>) -> PathBuf {
    if let Some(remembered) = remembered {
        return if remembered.is_absolute() { remembered.to_path_buf() } else { root.join(remembered) };
    }
    if let Some(name) = std::fs::read_to_string(root.join("Cargo.toml"))
        .ok()
        .as_deref()
        .and_then(cargo_package_name)
    {
        // The debug profile, because this is a debugger: a release binary is compiled without
        // the line tables every breakpoint here is expressed in.
        return root.join("target").join("debug").join(name);
    }
    root.to_path_buf()
}

/// What the *Debug ▸ Start debugging* box opens with: [`debuggee_for`]'s answer, as text.
///
/// A function of its own so that "the box is prefilled with the guess" is one line rather than a
/// property of a method on `App` — the whole design decision is that these two are the same
/// string, and a box that opened on anything else would be the editor guessing quietly again.
fn debuggee_prefill(root: &Path, remembered: Option<&Path>) -> String {
    debuggee_for(root, remembered).to_string_lossy().into_owned()
}

/// The adapter a settings line asks for, or `None` where the line is empty and discovery should
/// have its turn.
///
/// Split on spaces rather than parsed as a shell would, exactly as a language server's command
/// line is split in `lsp::server_for`: a path with a space in it wants the quoting dialect this
/// codebase has decided not to invent, and half a quoting dialect is worse than none.
fn configured_adapter(setting: &str) -> Option<crate::dap::AdapterCommand> {
    let argv: Vec<String> = setting.split_whitespace().map(str::to_string).collect();
    crate::dap::AdapterCommand::from_argv(&argv)
}

/// What to tell the adapter about, given what it has already been told and where the breakpoints
/// are now.
///
/// Every file with breakpoints in it, and — the half that is easy to leave out — every file it
/// was told about that has none any more, with an empty list. `setBreakpoints` replaces one
/// file's whole set, so a file that simply stops being mentioned keeps the breakpoints it had:
/// the user takes the last one off, the gutter clears, and the program still stops there.
fn breakpoints_to_publish(
    published: &std::collections::BTreeSet<PathBuf>,
    current: &std::collections::HashMap<PathBuf, std::collections::BTreeSet<usize>>,
) -> Vec<(PathBuf, Vec<usize>)> {
    // Ordered, because two runs of the same state have to send the same thing in the same order —
    // a HashMap's order is not a fact about the breakpoints.
    let mut out: std::collections::BTreeMap<PathBuf, Vec<usize>> = current
        .iter()
        .map(|(path, lines)| (path.clone(), lines.iter().copied().collect()))
        .collect();
    for path in published {
        out.entry(path.clone()).or_default();
    }
    out.into_iter().collect()
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
/// Whether `key` carries a character the user meant to *type*, as opposed to a chord.
///
/// crossterm reports Ctrl+A as `Char('a')` with CONTROL set, so a box that matches on the code
/// alone puts an `a` in the field every time a chord is pressed over it — Ctrl+V being the one
/// that hurts, because the letter lands where the paste was supposed to and nothing says so.
/// Alt and the Command key are refused on the same grounds: none of these boxes gives an Alt
/// chord a meaning, so any of them arriving here is a shortcut aimed at something else.
fn is_a_typed_character(key: KeyEvent) -> bool {
    !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
}

/// Removes the last user-perceived character from `s`.
///
/// `String::pop` removes one Unicode scalar, which is not what a person means by "the last
/// character": `é` written as `e` + U+0301 loses its accent and leaves the letter, and an emoji
/// built from several scalars comes apart into pieces that are themselves emoji. Backspace
/// pressed once has to undo one keystroke's worth of text.
///
/// Full grapheme segmentation needs the Unicode tables, which this program does not carry, so
/// this is the pragmatic reading of the rule: a scalar takes with it any combining marks and
/// variation selectors sitting on it, and a scalar joined to what precedes it by a zero-width
/// joiner takes the joiner and its left-hand side too. That covers accents, keycaps (`1` +
/// U+FE0F + U+20E3) and the ZWJ families and professions, which is what people actually type.
/// It does not cover regional-indicator pairs: a flag is two scalars with nothing marking them
/// as one, and deleting one of them leaves the other as a lone letter — the same as everywhere
/// else that lacks the tables, and better than guessing.
fn pop_grapheme(s: &mut String) {
    const ZWJ: char = '\u{200d}';
    loop {
        let Some(c) = s.pop() else { return };
        if is_attached_to_what_precedes_it(c) {
            continue;
        }
        // A base character was removed. If it was joined to the one before it, that one is part
        // of the same picture and goes as well.
        if s.ends_with(ZWJ) {
            s.pop();
            continue;
        }
        return;
    }
}

/// The scalars that are drawn on top of the character before them rather than beside it: the
/// four combining blocks, the combining half marks, and the variation selectors that choose how
/// the base is rendered. Deleting one on its own would change how a character looks without
/// removing anything the user can see.
fn is_attached_to_what_precedes_it(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F      // combining diacritical marks
        | 0x1AB0..=0x1AFF    // …extended
        | 0x1DC0..=0x1DFF    // …supplement
        | 0x20D0..=0x20FF    // …for symbols, including the enclosing keycap
        | 0xFE00..=0xFE0F    // variation selectors
        | 0xFE20..=0xFE2F)   // combining half marks
}

/// Which single-line box is taking typing right now.
///
/// The boxes that own the keyboard are listed in [`App::a_modal_owns_the_keyboard`] and given
/// their keys in [`App::dispatch_modal_key`]; this names the subset of them that holds text, so
/// a paste can be put where the typing goes. Most of the rest are lists or one-letter questions,
/// which have nothing to do with a sentence arriving all at once.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum ModalTextField {
    SaveAs,
    VenvRegister,
    RunCommand,
    PickerQuery,
    FindQuery,
    FindReplace,
    GotoLine,
    /// Both of the debugger's boxes: which one is up is on the prompt itself, and the text goes
    /// to the same place either way.
    DebugPrompt,
    ProjectSearch,
    NewEntry,
    Rename,
    SymbolRename,
    TerminalRename,
    WorkspaceSave,
    GitPrompt,
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
        // Resolved once, here: `auto` is answered by what the terminal said its background was
        // when it was asked, at startup, and that answer does not change for the life of the
        // session.
        let theme = settings.theme.resolve(crate::preview::background());
        // Read here rather than lazily: a chord the user moved has to be moved before the first
        // key press, and a `[keys]` entry that did not take is worth a sentence on the status
        // line at the moment they can still see it.
        let (keymap, key_warnings) = crate::keymap::Keymap::build(&settings.keys, settings.lang);
        crate::terminal_panel::set_scrollback_len(settings.terminal_scrollback);
        crate::wsnap::set_plots_in_tabs(settings.plots_in_tabs);
        // Before the first shell is spawned, because a shell inherits `CLEE_SESSION` and an agent
        // started in it would otherwise be pointed at a directory that does not exist yet.
        let mcp = crate::mcp::Session::start();
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
            follow_seen: None,
            follow_queue: std::collections::VecDeque::new(),
            follow_opened: 0,
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
            drawer: None,
            should_quit: false,
            redraw: true,
            terminal_generation: 0,
            status_message: if key_warnings.is_empty() {
                i18n::t(Lang::default(), Key::StatusHelp).to_string()
            } else {
                // A `[keys]` entry that did not take effect is the one thing about this file
                // worth interrupting the usual greeting for: the user pressed a key, nothing
                // happened, and without this they would have no idea their own setting was the
                // reason. The greeting is there every session; this is there once.
                key_warnings.join("  ")
            },
            editor_viewport: (0, 0),
            pointer: None,
            highlighter: Highlighter::for_theme(theme),
            keymap,
            settings,
            theme,
            show_settings: false,
            settings_selected: 0,
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
            symbol_rename: None,
            rename_preview: None,
            run_menu: None,
            theme_menu: None,
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
            pending_upload: None,
            agent_edit_ask: None,
            agent_edit_queue: std::collections::VecDeque::new(),
            agent_edits_this_session: false,
            find: None,
            find_text: None,
            picker: None,
            completion: None,
            completion_anchor: (0, 0),
            startup_cols: term_cols,
            lsp: std::collections::HashMap::new(),
            lsp_error: std::collections::HashMap::new(),
            diagnostics: std::collections::HashMap::new(),
            lsp_raw: std::collections::HashMap::new(),
            lsp_sent: std::collections::HashMap::new(),
            lsp_seen: std::collections::HashMap::new(),
            lsp_paths: std::collections::HashMap::new(),
            lsp_completion: None,
            lsp_asked: None,
            lsp_listing: None,
            lsp_editing: None,
            lsp_formatting: None,
            lsp_acting: None,
            lsp_action_edit: None,
            lsp_widening: None,
            selection_walk: None,
            lsp_folding: std::collections::HashMap::new(),
            lsp_folds_asked: std::collections::HashMap::new(),
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
            debug: None,
            debug_panel: DebugPanel::default(),
            debug_prompt: None,
            debuggee: None,
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
            search_replace: String::new(),
            search_field: SearchField::Query,
            replace_asked: None,
            replace_sweep: None,
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
            mcp,
            git_wanted: None,
            last_autosave: Instant::now(),
            autosave_complained: false,
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
        // The joke is an animation: while one is crossing the status line, every frame moves it.
        if self.turtle.is_some() {
            self.redraw = true;
        }
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
    /// The colours to draw this frame in. Built from the theme every time rather than cached:
    /// it is a couple of dozen `Color`s copied once per frame, and a cache would be one more
    /// thing to remember to invalidate when the theme changes.
    pub fn palette(&self) -> crate::theme::Palette {
        self.theme.palette()
    }

    pub fn menu_states(&self) -> crate::menu::MenuStates {
        crate::menu::MenuStates {
            plots_in_tabs: self.settings.plots_in_tabs || !crate::wsnap::can_open_a_window(),
            md_toolbar: self.settings.show_md_toolbar,
            follow_agent_edits: self.settings.follow_agent_edits,
            drawer_open: self.drawer_is_open(),
        }
    }

    pub fn poll_splash(&mut self) {
        // Kept redrawing for as long as it is up: the splash is the first thing on screen, and
        // the frame that takes it away is one nobody else asks for.
        if self.show_splash {
            self.redraw = true;
        }
        if self.show_splash && self.splash_started.elapsed() >= SPLASH_DURATION {
            self.show_splash = false;
        }
    }

    pub fn poll_background_messages(&mut self) {
        while let Ok(msg) = self.bg_rx.try_recv() {
            self.status_message = msg;
            self.redraw = true;
        }
    }

    /// Hands decoded pictures to the tabs that asked for them. Matched by path rather than by
    /// index: tabs can be closed or reordered while a decode is in flight, and a stale index
    /// would put a picture in somebody else's tab.
    pub fn poll_previews(&mut self) {
        while let Ok(done) = self.preview_rx.try_recv() {
            // A reply is news whatever it says: a page to put up, a picture that failed, or a
            // mark cleared on a tab that was waiting for it.
            self.redraw = true;
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
            // Said once, after the borrow on the tab is over: a picture is put up first and
            // talked about afterwards.
            let mut note = None;
            match done.result {
                Ok(image) => {
                    // What the file turned out to hold past its first frame. Set on every read
                    // and not only the first, because a file changes underneath a tab: a
                    // still `.gif` written over an animated one must stop the old frames
                    // cycling, and an animated one written over a still must start.
                    match done.motion {
                        crate::preview::Motion::Still => {
                            preview.animation = None;
                            preview.animation_refused = false;
                        }
                        crate::preview::Motion::Animated(mut animation) => {
                            // The clock starts here rather than where the frames were decoded,
                            // so a long decode does not spend the first frame's time before
                            // anybody has seen it.
                            animation.restart();
                            preview.animation = Some(animation);
                            preview.animation_refused = false;
                        }
                        crate::preview::Motion::TooBig { width, height, frames } => {
                            preview.animation = None;
                            preview.animation_refused = true;
                            note = Some(i18n::msg_animation_too_large(
                                self.settings.lang,
                                width,
                                height,
                                frames,
                            ));
                        }
                    }
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
                    // would otherwise shrink a larger page straight back to the pane. A picture
                    // nobody has zoomed has no window: it is the whole of itself. See
                    // `Preview::window_of`.
                    let window = preview.window_of(&image);
                    // Which pane this was made for, recorded beside it. A picture asked for
                    // before its tab was ever drawn is fitted to nothing here, and the first
                    // frame that measures the pane is what notices. See `Preview::needs_refit`.
                    preview.fitted_for = (cols, rows);
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
            // An animation whose frames were too many to hold. The picture is up — the first
            // frame of it — and this is why it stands there. It is said in the status line
            // once, and the bar keeps a short mark for as long as the tab is open: a status
            // message is taken by the next gesture, and a tab that then merely looks like a
            // still picture would be a question with no answer left on screen.
            if let Some(note) = note {
                self.status_message = note;
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

    /// Whether the agent drawer has a column on screen right now.
    ///
    /// Open is about the column, not about the agent: a drawer whose agent has exited is still
    /// open, showing the list of four. Every layout question asks this one.
    pub fn drawer_is_open(&self) -> bool {
        self.drawer.as_ref().is_some_and(|d| d.open)
    }

    /// The drawer's pane, when there is an agent in it rather than the launcher.
    fn drawer_panel(&self) -> Option<&TerminalPanel> {
        self.drawer.as_ref()?.window.as_ref().map(|w| w.active_tab())
    }

    fn drawer_panel_mut(&mut self) -> Option<&mut TerminalPanel> {
        self.drawer.as_mut()?.window.as_mut().map(|w| w.active_tab_mut())
    }

    pub fn poll_terminal_exits(&mut self) {
        let lang = self.settings.lang;
        // A workspace's startup commands are typed here rather than at spawn time, once each
        // shell is actually at a prompt.
        for window in &mut self.terminals {
            for tab in &mut window.tabs {
                tab.flush_pending();
            }
        }
        // And the drawer, which is not in `terminals`. This is the *only* call site of
        // `flush_pending` there is, so an agent left out of it would never be started at all: its
        // pane would sit at a shell prompt with the command still queued, which on screen is
        // indistinguishable from the agent failing to launch.
        if let Some(window) = self.drawer.as_mut().and_then(|d| d.window.as_mut()) {
            for tab in &mut window.tabs {
                tab.flush_pending();
            }
        }
        let before: usize = self.terminals.len();
        // Reap exited tabs within each window, then drop any window left with no tabs.
        // A shell that has ended leaves no output behind it, so this is the one thing about a
        // pane that the output counter cannot notice: the tab simply stops being there.
        let mut reaped = false;
        for window in &mut self.terminals {
            reaped |= window.reap_exited();
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
            reaped = true;
        }
        // The drawer is reaped on its own terms, and deliberately *not* under the invariant
        // above. Never leaving the workspace without a terminal is right for the terminal panel;
        // applied here it would put a shell where an agent had been, which on screen is
        // indistinguishable from the agent still being there — the worst possible lie for a pane
        // whose whole job is holding a conversation. An agent that has ended returns the drawer
        // to the launcher: the conversation is over, and the choice is on offer again.
        if let Some(drawer) = self.drawer.as_mut() {
            let ended = drawer.window.as_mut().is_some_and(|window| {
                window.reap_exited();
                window.tabs.is_empty()
            });
            if ended {
                let agent = drawer.agent;
                drawer.back_to_launcher();
                if let Some(agent) = agent {
                    self.status_message = i18n::msg_drawer_agent_ended(lang, agent.label());
                }
                reaped = true;
            }
        }
        if reaped {
            self.redraw = true;
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
        // A paste is keys, and while a box is up the keys are its. Into a box that takes text it
        // arrives as text; over any other it does nothing at all — which is the point, because
        // doing nothing is what a box that has no use for it should do, and falling through to
        // the editor underneath is what it used to do instead.
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
            // A paste over the debug panel is dropped. Nothing in it is a text field: it is a
            // list of what the program is, and the one place text goes in — the watch box — has
            // its own paste arm and is not this frame.
            Focus::Debug => {}
            // Straight into the agent's pane, by the same route a terminal takes it — brackets
            // where the program asked for them. Over the launcher there is nothing a paste
            // could mean, so it is dropped rather than typed at a list.
            Focus::Drawer => {
                let Some(window) = self.drawer.as_mut().and_then(|d| d.window.as_mut()) else {
                    return;
                };
                let panel = window.active_tab_mut();
                let bytes = panel.paste_bytes(&text);
                panel.write_input(&bytes);
            }
        }
    }

    /// A paste while a box is up.
    ///
    /// Every box that takes typing takes it. The rest ignore it rather than passing it on: a
    /// paste over a question that wants one letter is not an answer to it, and a paste over a
    /// list is not anything.
    ///
    /// Newlines become spaces because every one of these boxes is a single line. A pasted commit
    /// message with a blank line in it would otherwise arrive as a message with two invisible
    /// characters in the middle of it.
    ///
    /// Where the text lands is decided by [`Self::modal_text_field`] rather than here, so the
    /// paste and the keyboard cannot disagree about which box is in front. What each field does
    /// *after* the text arrives still has to match its `Char` arm: a query that filters a list
    /// as you type has to filter it once the paste is in, or the box says one thing and shows
    /// another.
    fn paste_into_a_modal(&mut self, text: &str) {
        let Some(field) = self.modal_text_field() else { return };
        let text = text.replace(['\n', '\r'], " ");
        match field {
            ModalTextField::SaveAs => self.save_as_input.push_str(&text),
            ModalTextField::VenvRegister => self.venv_register_input.push_str(&text),
            ModalTextField::RunCommand => self.run_command_input.push_str(&text),
            ModalTextField::PickerQuery => {
                if let Some(p) = self.picker.as_mut() {
                    p.query.push_str(&text);
                    p.refilter();
                }
                self.refresh_picker();
            }
            ModalTextField::FindQuery => {
                if let Some(f) = self.find.as_mut() {
                    f.query.push_str(&text);
                }
                self.recompute_find();
            }
            ModalTextField::FindReplace => {
                if let Some(f) = self.find.as_mut() {
                    f.replace.push_str(&text);
                }
            }
            // The only box that refuses most of what it is given: it holds a line number, its
            // keyboard accepts digits and nothing else, and a pasted path or word would sit
            // there unparseable with no hint of why.
            ModalTextField::GotoLine => {
                self.goto_input.extend(text.chars().filter(char::is_ascii_digit))
            }
            // A path pasted into the debuggee box is exactly the point of the box, and an
            // expression pasted into the watch box is how anybody would move one out of the
            // source they are reading.
            ModalTextField::DebugPrompt => {
                if let Some(prompt) = self.debug_prompt.as_mut() {
                    prompt.typed.push_str(&text);
                }
            }
            ModalTextField::ProjectSearch => self.search_field_mut().push_str(&text),
            ModalTextField::NewEntry => self.new_entry_input.push_str(&text),
            ModalTextField::Rename => self.rename_input.push_str(&text),
            ModalTextField::SymbolRename => {
                if let Some(box_) = self.symbol_rename.as_mut() {
                    box_.typed.push_str(&text);
                }
            }
            ModalTextField::TerminalRename => self.terminal_field_mut().push_str(&text),
            ModalTextField::WorkspaceSave => self.workspace_save_input.push_str(&text),
            ModalTextField::GitPrompt => {
                if let Some(GitPrompt::Text { typed, .. }) =
                    self.git_panel.as_mut().and_then(|p| p.prompt.as_mut())
                {
                    typed.push_str(&text);
                }
            }
        }
    }

    /// Which box is taking typing, if the one in front takes any.
    ///
    /// The chain is [`Self::dispatch_modal_key`]'s, in the same order and for the same reason:
    /// these boxes can appear over one another, and the one in front is the one holding the
    /// cursor. Kept as a separate list only because most of what that chain dispatches to is a
    /// menu or a yes/no, which has no field to put text in — the two must be changed together,
    /// and a box added to one and not the other takes keys but not pastes.
    fn modal_text_field(&self) -> Option<ModalTextField> {
        // These four are in front of everything and none of them is a text box: a menu, a question
        // about unsaved work, a question about sending files to another machine, and a question
        // about letting an agent change a buffer.
        if self.context_menu.is_some()
            || self.unsaved_prompt.is_some()
            || self.pending_upload.is_some()
            || self.agent_edit_ask.is_some()
        {
            return None;
        }
        if self.show_save_as {
            return Some(ModalTextField::SaveAs);
        }
        if self.run_menu.is_some() || self.theme_menu.is_some() {
            return None;
        }
        if self.venv_register.is_some() {
            return Some(ModalTextField::VenvRegister);
        }
        if self.run_command_edit.is_some() {
            return Some(ModalTextField::RunCommand);
        }
        if self.picker.is_some() {
            return Some(ModalTextField::PickerQuery);
        }
        if let Some(find) = self.find.as_ref() {
            return Some(match find.focus_replace {
                true => ModalTextField::FindReplace,
                false => ModalTextField::FindQuery,
            });
        }
        if self.show_goto {
            return Some(ModalTextField::GotoLine);
        }
        if self.debug_prompt.is_some() {
            return Some(ModalTextField::DebugPrompt);
        }
        if self.show_search {
            return Some(ModalTextField::ProjectSearch);
        }
        if self.show_new_entry {
            return Some(ModalTextField::NewEntry);
        }
        if self.show_delete_confirm {
            return None;
        }
        if self.show_rename {
            return Some(ModalTextField::Rename);
        }
        if self.symbol_rename.is_some() {
            return Some(ModalTextField::SymbolRename);
        }
        // The preview that follows it takes no text: it is a list and a yes/no, and a sentence
        // pasted into it would have nowhere to go.
        if self.rename_preview.is_some() || self.replace_sweep.is_some() {
            return None;
        }
        if self.show_terminal_rename {
            return Some(ModalTextField::TerminalRename);
        }
        if self.show_workspace_save {
            return Some(ModalTextField::WorkspaceSave);
        }
        // The git panel's own boxes: a message or a name takes the text, and the one-letter
        // questions and the lists behind them take nothing.
        if let Some(panel) = self.git_panel.as_ref() {
            return matches!(panel.prompt, Some(GitPrompt::Text { .. })).then_some(ModalTextField::GitPrompt);
        }
        // The inspector, the manual, resize mode, the settings page and the menu bar all read
        // keys as commands. There is nowhere for a sentence to go in any of them.
        None
    }

    /// A paste into a terminal, which is nearly always just typing — and once in a while is a
    /// file dragged onto a pane logged into somewhere else.
    ///
    /// Two things stand between the two readings, because the cost of confusing them is a file
    /// leaving the machine. The first is shape: a drag arrives as rooted paths and nothing else,
    /// which is what [`dnd::looks_like_dropped_paths`] recognises — a sentence with a path in the
    /// middle of it is prose, and prose is typed. Checking only that the tokens exist on disk was
    /// not enough; `~/.ssh/id_ed25519` exists, and pasting it into a command you were composing
    /// is not an instruction to upload it.
    ///
    /// The second is the question. Even a real drag is answered before anything moves: an upload
    /// cannot be taken back once the file is on the other machine, and there is nothing on screen
    /// afterwards to say it happened.
    fn handle_terminal_paste(&mut self, text: &str) {
        let lang = self.settings.lang;
        let paths = dnd::upload_candidates(text);
        let ssh_target = if paths.is_empty() {
            None
        } else {
            self.focused_panel()
                .and_then(|t| t.child_pid())
                .and_then(dnd::detect_ssh_target)
        };
        if let Some(target) = ssh_target {
            self.status_message = i18n::msg_scp_confirm(lang, paths.len(), &target);
            self.pending_upload = Some(PendingUpload { target, paths });
        } else if let Some(term) = self.focused_panel_mut() {
            // Wrapped as a paste when the program asked for that, and only here: the upload
            // question above is answered with a keystroke and never reaches the pty at all, so
            // the brackets belong to the text being written and to nothing else.
            let bytes = term.paste_bytes(text);
            term.write_input(&bytes);
        }
    }

    /// The one letter that sends the files, in the language the question was asked in. Every
    /// other key is no — including the ones that would do something in the pane underneath,
    /// which is the whole point of asking first.
    fn handle_upload_prompt_key(&mut self, key: KeyEvent) {
        let lang = self.settings.lang;
        let Some(upload) = self.pending_upload.take() else { return };
        let yes = key.code == KeyCode::Char(i18n::yes_key(lang))
            || key.code == KeyCode::Char(i18n::yes_key(lang).to_ascii_uppercase());
        if yes {
            self.scp_paths_background(upload.target, upload.paths);
        } else {
            self.status_message = i18n::msg_scp_cancelled(lang);
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
                empty_state_focus(self.settings.show_sidebar, self.settings.show_terminal);
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

    /// Says the screen no longer matches the state. Cheap enough to call on a suspicion.
    pub fn mark_dirty(&mut self) {
        self.redraw = true;
    }

    /// Whether a frame is owed, clearing the debt. Asked once per turn of the loop.
    pub fn take_redraw(&mut self) -> bool {
        std::mem::take(&mut self.redraw)
    }

    /// Notices output that has arrived in any pane since the last look.
    ///
    /// A sum rather than a per-pane comparison: the panes come and go, their order changes, and
    /// no individual number means anything here — only that the total moved. Wrapping, because a
    /// counter that has been running long enough to overflow is still only being compared with
    /// itself, and a pane closing lowers the total exactly as legitimately as output raises it.
    pub fn poll_terminal_output(&mut self) {
        // The drawer's pane is folded into the same sum. It is not in `terminals`, and leaving it
        // out is not a small bug: nothing else raises the redraw flag for output, so the agent
        // would paint its first frame and then appear to freeze — every reply arriving in a
        // buffer nobody was drawing until an unrelated keystroke happened to ask for a frame.
        let now = self
            .terminals
            .iter()
            .chain(self.drawer.iter().filter_map(|d| d.window.as_ref()))
            .flat_map(|w| w.tabs.iter())
            .fold(0u64, |total, tab| total.wrapping_add(tab.generation()));
        if now != self.terminal_generation {
            self.terminal_generation = now;
            self.redraw = true;
        }
        // A pane that has not been revealed yet is waiting on a quiet moment rather than on
        // anything that can raise a flag, so it is drawn towards.
        if self
            .terminals
            .iter()
            .chain(self.drawer.iter().filter_map(|d| d.window.as_ref()))
            .flat_map(|w| w.tabs.iter())
            .any(|tab| tab.awaiting_reveal())
        {
            self.redraw = true;
        }
    }

    pub fn poll_external_changes(&mut self) {
        // Files may have been reloaded, the tree re-read and the git dots refreshed underneath.
        self.redraw = true;
        let lang = self.settings.lang;
        // Every buffer, not just the one in front of you. A tab in the background is still a
        // file, and a branch switched or a formatter run under it used to leave it holding text
        // that no longer existed anywhere — noticed only on saving over the newer version.
        //
        // Rendered views are skipped: their file is read by `reload_changed_previews`, which
        // knows to leave the picture up while the new one is made. Reloading the same file here
        // as well would blank the pane on its own rhythm.
        //
        // The message is the last one that had something to say, so a sweep that reloads three
        // files in silence and finds a fourth with unsaved edits still reports the one that
        // needs a decision.
        let mut said = None;
        for editor in self.editors.iter_mut().filter(|e| e.preview.is_none()) {
            if let Some(msg) = editor.check_external_changes(lang) {
                said = Some(msg);
            }
        }
        if let Some(msg) = said {
            self.status_message = msg;
        }
        self.reload_changed_previews();
        self.file_tree.refresh();
        spawn_git_status_refresh(self.root.clone(), self.git_status_tx.clone(), self.git_status_pending.clone());
    }

    /// Copies every changed unsaved buffer into the recovery directory, a few seconds at a time.
    ///
    /// What this is for, stated exactly, because a safety net believed to be finer than it is is
    /// worse than none. The panic shield in `main.rs` already means a bug inside CleeCode costs a
    /// status line and not the session — the loop carries on, the buffers are untouched, and
    /// nothing here is what saves you from that. This covers the endings the shield cannot see:
    /// `SIGKILL`, a stack overflow (which aborts without unwinding, so `catch_unwind` never
    /// runs), a machine losing power, a terminal emulator taking its children down with it. And
    /// it does *not* cover the last few seconds of typing: the copy is taken on a tick, so an
    /// edit made after the last one and before the ending was never written down anywhere.
    ///
    /// Self-throttled rather than ticked from `run`, in the shape `poll_run_watch` uses, because
    /// the interval is this function's business and not the loop's — and because the setting that
    /// turns it off is read here, where it can be changed mid-session and take effect at once.
    ///
    /// A buffer is copied only when it is dirty *and* its text has moved since its own last copy.
    /// The pair is trustworthy: `dirty` is set in exactly one place (`Editor::mark_edited_from`)
    /// and cleared in exactly one other (`Editor::save`), and the revision moves with the first.
    /// Without the revision half, a file left open unsaved would have its whole text rewritten
    /// every five seconds for as long as the editor stayed open.
    pub fn poll_autosave(&mut self) {
        if !self.settings.autosave_recovery || self.last_autosave.elapsed() < AUTOSAVE_INTERVAL {
            return;
        }
        self.last_autosave = Instant::now();
        let mut wrote = false;
        let mut failure = None;
        for editor in &mut self.editors {
            // A picture has no text to copy and can never be dirty — but saying so here means a
            // preview tab cannot become somebody's recovery directory through some future path
            // that forgets. The rest of the rule is in `recovery::needs_copy`, stated where it
            // can be read and tested on its own.
            if editor.preview.is_some() {
                continue;
            }
            if !crate::recovery::needs_copy(
                editor.dirty,
                editor.is_read_only(),
                editor.revision(),
                editor.autosaved_revision,
            ) {
                continue;
            }
            match crate::recovery::write_entry(
                editor.path.as_deref(),
                editor.recovery_id,
                &editor.rope.to_string(),
            ) {
                Ok(_) => {
                    editor.autosaved_revision = Some(editor.revision());
                    wrote = true;
                }
                Err(e) => failure = Some(e.to_string()),
            }
        }
        match failure {
            // Said once. The status line is one line and the next action takes it back, so a
            // sentence repeated every five seconds is not emphasis — it is the status line
            // becoming unusable for everything else.
            Some(detail) if !self.autosave_complained => {
                self.autosave_complained = true;
                let where_ = crate::recovery::dir()
                    .map(|d| d.display().to_string())
                    .unwrap_or_else(|| "~".to_string());
                self.status_message =
                    i18n::msg_recovery_failed(self.settings.lang, &where_, &detail);
            }
            Some(_) => {}
            None if wrote => self.autosave_complained = false,
            None => {}
        }
    }

    /// Offers back what an earlier session was in the middle of, if anything.
    ///
    /// Called once at startup by `run`, after whichever route put the tabs on screen. Nothing
    /// happens in the ordinary case — the directory is empty, or holds only copies belonging to
    /// other projects and to CleeCodes that are still running.
    pub fn offer_recovery(&mut self) {
        if !self.settings.autosave_recovery {
            return;
        }
        let found = crate::recovery::scan(&self.root);
        self.open_recovery_picker(found);
    }

    /// Puts the offer on screen, one row per copy. Does nothing when there is nothing to offer,
    /// which is what lets the restore call it again with whatever is left.
    fn open_recovery_picker(&mut self, entries: Vec<crate::recovery::Entry>) {
        if entries.is_empty() {
            return;
        }
        let lang = self.settings.lang;
        let root = std::fs::canonicalize(&self.root).unwrap_or_else(|_| self.root.clone());
        let now = std::time::SystemTime::now();
        let items = entries
            .into_iter()
            .map(|entry| {
                let name = match &entry.original {
                    // Relative to the project, like every other list of files here: the absolute
                    // path is the same forty characters on every row and says nothing.
                    Some(path) => {
                        path.strip_prefix(&root).unwrap_or(path).display().to_string()
                    }
                    None => i18n::t(lang, Key::UntitledFile).to_string(),
                };
                let age = i18n::msg_recovery_age(
                    lang,
                    now.duration_since(entry.saved).map(|d| d.as_secs()).unwrap_or(0),
                );
                crate::picker::PickItem {
                    label: format!("{name}  ·  {age}"),
                    shortcut: None,
                    action: crate::picker::PickAction::Recover(Box::new(entry)),
                }
            })
            .collect();
        // The title card gives way to this. It is two seconds of decoration in front of a
        // question about work that was nearly lost, and it would also swallow the first key
        // pressed at it.
        self.show_splash = false;
        self.picker = Some(crate::picker::Picker::new(
            i18n::t(lang, Key::PickerRecovery),
            crate::picker::PickerKind::Recovery,
            items,
        ));
    }

    /// Puts one copy back into a buffer, and takes the copy off disk.
    ///
    /// The buffer is left **dirty**, and that is the point rather than an oversight. CleeCode
    /// never decided to stop; deciding on the user's behalf that this text is now the file would
    /// be making a second decision they did not make either. So the text is put back where they
    /// left it and the choice of what to do with it is theirs — including throwing it away, which
    /// is why the whole replacement goes in as a *single* undo step: one Ctrl+Z and the buffer is
    /// the file that is on disk, exactly.
    ///
    /// The copy is removed either way. Its content is in the buffer now, and if it is still
    /// wanted five seconds from now the autosave tick will have written it again.
    fn restore_recovery(&mut self, entry: crate::recovery::Entry) {
        let lang = self.settings.lang;
        let idx = match &entry.original {
            Some(path) => {
                if path.exists() {
                    self.open_file_in_tab(path.clone());
                }
                match self.editors.iter().position(|e| e.path.as_deref() == Some(path.as_path())) {
                    Some(idx) => idx,
                    // The file itself is gone — deleted, or on a branch that no longer has it.
                    // The copy is then the only version of it left anywhere, so it comes back as
                    // a buffer pointed at the name it had, ready to be saved back into place.
                    None => {
                        let mut editor = Editor::empty();
                        editor.path = Some(path.clone());
                        editor.syntax_dirty = true;
                        let idx = self.adopt_editor(editor);
                        self.place_in_pane(self.editor_pane_focus, idx);
                        idx
                    }
                }
            }
            None => {
                let mut editor = Editor::empty();
                editor.rope = ropey::Rope::from_str(&entry.text);
                // No undo step to offer here: there is no earlier version of a buffer that was
                // never on disk. Marked by hand for the same reason — nothing was *edited*, the
                // buffer arrived already changed, and it has to look that way to the tab strip,
                // to the quit prompt and to the next autosave tick.
                editor.dirty = true;
                let idx = self.adopt_editor(editor);
                self.place_in_pane(self.editor_pane_focus, idx);
                self.focus = Focus::Editor;
                self.status_message = i18n::msg_recovery_restored(
                    lang,
                    i18n::t(lang, Key::UntitledFile),
                );
                let _ = std::fs::remove_file(&entry.file);
                return;
            }
        };
        let Some(editor) = self.editors.get_mut(idx) else { return };
        if editor.is_read_only() {
            // A file that has become binary or unreadable since. The copy stays where it is
            // rather than being dropped on the floor: it is still the only version of that work.
            self.status_message = i18n::msg_recovery_refused(lang, &self.editors[idx].title(lang));
            return;
        }
        let whole = editor.rope.len_chars();
        editor.replace_char_range(0, whole, &entry.text);
        // The cursor lands after the inserted text, which for a whole-file replacement is the
        // bottom of the file — a view scrolled to the end of a document nobody asked to be at
        // the end of. The undo snapshot was taken before this, so moving it now costs no step.
        editor.cursor_line = 0;
        editor.cursor_col = 0;
        self.focus = Focus::Editor;
        self.status_message = i18n::msg_recovery_restored(lang, &self.editors[idx].title(lang));
        let _ = std::fs::remove_file(&entry.file);
    }

    pub fn poll_git_status(&mut self) {
        while let Ok(status) = self.git_status_rx.try_recv() {
            // The sweep the sidebar's dots come from, read a second time for what it also says:
            // which files have moved since the last one. No watcher, no second process, no
            // guess about which agent is running — just two consecutive answers to a question
            // that was already being asked.
            let touched = match &self.follow_seen {
                Some(before) => crate::git_status::touched_between(before, &status),
                None => Vec::new(),
            };
            self.follow_seen = Some(status.clone());
            self.git_status = status;
            self.redraw = true;
            if self.settings.follow_agent_edits {
                self.follow_touched_files(touched);
            }
        }
    }

    /// Opens one of the files something has just written, if any of them is worth opening.
    ///
    /// One per sweep, deliberately: an agent's first move is often to touch a dozen files, and a
    /// dozen tabs arriving in one frame is not "showing" anything. At a sweep every 700 ms the
    /// interesting ones are all there within a few seconds, in the order git reports them, and
    /// the window never jumps more than once at a time.
    fn follow_touched_files(&mut self, touched: Vec<PathBuf>) {
        for path in touched {
            if self.follow_queue.len() >= FOLLOW_QUEUE_LIMIT {
                break;
            }
            if !self.follow_queue.contains(&path) {
                self.follow_queue.push_back(path);
            }
        }
        // Everything unopenable is dropped on the way past — a directory, a file already in a
        // tab, something that has been deleted again since — and the first one that is left is
        // shown. Then the loop returns: whatever is still queued waits for the next sweep.
        while let Some(path) = self.follow_queue.pop_front() {
            if !self.worth_following(&path) {
                continue;
            }
            if self.follow_opened >= FOLLOW_TAB_LIMIT {
                // Said once, at the moment it starts mattering, and the queue is let go: what
                // was in it will not be opened, and holding paths that will never be used is
                // just a leak with a long fuse. Nothing already open is closed to make room —
                // a tab is something you might be reading, and follow mode's whole promise is
                // that it does not touch what you are working on.
                if self.follow_opened == FOLLOW_TAB_LIMIT {
                    self.follow_opened += 1;
                    self.status_message =
                        i18n::msg_follow_full(self.settings.lang, FOLLOW_TAB_LIMIT);
                }
                self.follow_queue.clear();
                return;
            }
            self.follow_opened += 1;
            self.open_beside_without_focus(path);
            return;
        }
    }

    /// Whether a path something touched is one follow mode should put on screen.
    fn worth_following(&self, path: &Path) -> bool {
        // A directory carries a status too — `git_status` marks every parent of a changed file
        // so the tree can draw a dot on a collapsed folder — and there is nothing to open in one.
        if !path.is_file() {
            return false;
        }
        if self.editors.iter().any(|e| e.path.as_deref().is_some_and(|open| same_file(open, path))) {
            return false;
        }
        // Something that would be *run* rather than opened: a PDF with a viewer configured goes
        // to a terminal, and a command starting itself because a file changed is a surprise of a
        // completely different order.
        !(Editor::looks_binary(path) && self.run_command_for(&file_ext(path)).is_some())
    }

    /// Puts a file on screen in the half you are not typing in, and leaves the keyboard exactly
    /// where it was — the rule the figures of 0.9 established, by the same road they take.
    fn open_beside_without_focus(&mut self, path: PathBuf) {
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
        self.open_file_in_tab(path);
        // Everything about where the keyboard was, put back — including which tab the pane you
        // were in is showing. That last part is where this parts company with a figure: a
        // picture that took the front of a narrow window still cannot receive a keystroke, and a
        // text file can. In a window too narrow to split, the new tab therefore joins the strip
        // and waits there rather than arriving in front of the line you were typing.
        self.focus = was.0;
        self.editor_pane_focus = was.1;
        match was.1 {
            EditorPane::Left => self.active_editor = was.2,
            EditorPane::Right => self.active_editor_right = was.3,
        }
    }

    /// Said when follow mode has just been switched, from wherever it was switched.
    ///
    /// Outside a repository it is switched all the same and does nothing at all, which is the
    /// one state that has to be spoken aloud: a setting that reads "on" while no file ever
    /// appears looks like a broken feature rather than a missing repository.
    fn follow_mode_switched(&mut self, was: bool) {
        let now = self.settings.follow_agent_edits;
        if now == was {
            return;
        }
        let lang = self.settings.lang;
        self.status_message = if now && crate::git::toplevel(&self.root).is_none() {
            i18n::msg_follow_needs_a_repo(lang).to_string()
        } else {
            i18n::msg_follow_mode(lang, now).to_string()
        };
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
        // Emptied every time, unlike the query, which is prefilled. The asymmetry is the safety:
        // an empty replacement is what makes Enter a search, so a replacement left over from ten
        // minutes ago would turn the next Ctrl+Shift+H into a sweep nobody asked for — and the
        // query field, being prefilled, is the one place the reader is already looking.
        self.search_replace.clear();
        self.search_field = SearchField::Query;
        self.show_search = true;
    }

    /// The same box, opened on the field that makes it a replace.
    fn begin_project_replace(&mut self) {
        self.begin_project_search();
        self.search_field = SearchField::Replace;
    }

    /// The field typing goes into, so the two arms below never have to name both.
    fn search_field_mut(&mut self) -> &mut String {
        match self.search_field {
            SearchField::Query => &mut self.search_input,
            SearchField::Replace => &mut self.search_replace,
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.close_search_box(),
            KeyCode::Enter => self.start_project_search(),
            // Both fields are one line, so the vertical keys move between them too — the same
            // arrangement as the terminal's name-and-command box, and for the same reason:
            // nobody should have to guess that only Tab works.
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                self.search_field = self.search_field.other();
            }
            // The same two switches as the Find box, by the same two keys: one idea, one pair
            // of keys, wherever a query is typed. They belong to the query whichever field is
            // being typed into — there is one search here, not one per field.
            KeyCode::Char('u') if ctrl => self.search_case_sensitive = !self.search_case_sensitive,
            KeyCode::Char('n') if ctrl => self.search_regex = !self.search_regex,
            KeyCode::Backspace => pop_grapheme(self.search_field_mut()),
            KeyCode::Char(c) if is_a_typed_character(key) => self.search_field_mut().push(c),
            _ => {}
        }
    }

    /// Closes the box and forgets what was in it. The replacement goes too: a field holding text
    /// nobody can see is a field that turns the next Ctrl+Shift+H into a sweep.
    fn close_search_box(&mut self) {
        self.show_search = false;
        self.search_input.clear();
        self.search_replace.clear();
        self.search_field = SearchField::Query;
    }

    /// Starts the walk. One walk, whichever of the two things the box was asked for: a replace is
    /// a search whose answer is read differently, and running a second kind of scan for it would
    /// be two dialects of "what matches" for one query.
    fn start_project_search(&mut self) {
        let query = self.search_input.trim().to_string();
        if query.is_empty() {
            return;
        }
        // Untrimmed, unlike the query: a replacement is text to write, and leading or trailing
        // spaces in it are as meant as any other character. Trimming it would make `" -> "`
        // impossible to type.
        let replacement = self.search_replace.clone();
        self.show_search = false;
        let lang = self.settings.lang;
        self.status_message = match replacement.is_empty() {
            true => i18n::msg_search_running(lang, &query),
            false => i18n::msg_replace_running(lang, &query, &replacement),
        };
        // Set before the walk starts and read when it answers. An empty field leaves it `None`,
        // which is what makes the search below byte-for-byte the search it has always been.
        self.replace_asked = match replacement.is_empty() {
            true => None,
            false => Some(PendingReplace {
                replacement,
                regex: self.search_regex,
                case_sensitive: self.search_case_sensitive,
            }),
        };
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

    // ---- Replacing across the project -------------------------------------------------------
    //
    // The same discipline as the rename, because it is the same problem: a preview grouped by
    // file, one place where a query becomes a pattern, all-or-nothing application, one step of
    // undo per open buffer. What it adds is the half the rename refuses outright — files nothing
    // has open — and everything below that is not shared with the rename exists to make that half
    // safe: a file with a tab NEVER takes the disk road (see [`SweepTarget`]), a file without one
    // is rewritten through the temp-and-rename that `settings::write_atomic` does, and what the
    // file said about its own line endings is said back to it.
    //
    // The disk half has no undo. That is not hidden and cannot be fixed by hiding it: the preview
    // is the consent, and the sentence afterwards counts the files it rewrote out loud.

    /// Turns a finished search into a preview of the sweep, or into the one sentence saying why
    /// there is not going to be one.
    ///
    /// Every `Err` refuses the *whole* replace, and they are asked in the order of what the
    /// reader can do about them: narrow the query, close a read-only tab, or search again.
    fn replace_sweep_from(
        &self,
        outcome: &crate::search::Outcome,
        asked: &PendingReplace,
    ) -> Result<ReplaceSweep, String> {
        let lang = self.settings.lang;
        // A search that stopped at its limit looked at part of the project, and a *list* that is
        // part of the project is still useful — you go to one of the rows. A sweep is not: it is
        // a claim about every occurrence, and half of every occurrence is the shape of bug that
        // is found weeks later in a file nobody opened.
        if outcome.truncated {
            return Err(i18n::msg_replace_refused_truncated(lang, crate::search::HIT_LIMIT));
        }
        let re = crate::find::compile(&outcome.query, asked.regex, asked.case_sensitive)
            .map_err(|detail| i18n::msg_find_pattern_error(lang, &detail))?;

        // One entry per file, in path order, so the preview reads the same way twice running —
        // the hits arrive in walk order, which is an order of the filesystem's choosing.
        let mut paths: Vec<PathBuf> = outcome.hits.iter().map(|h| h.path.clone()).collect();
        paths.sort();
        paths.dedup();

        let held = |path: &Path| self.editors.iter().find(|e| e.path.as_deref() == Some(path));
        // Asked before anything is built, like the rename asks it: a tab that cannot be typed in
        // is a refusal the reader fixes by closing it, not something to discover file by file.
        if paths.iter().any(|p| held(p).is_some_and(|e| e.read_only)) {
            return Err(i18n::msg_replace_refused_read_only(lang).to_string());
        }

        let mut files = Vec::new();
        let mut rows = Vec::new();
        let mut total = 0usize;
        for path in paths {
            // Buffer first, always — see [`sweep_text_and_target`]. Unreadable, or no longer
            // text, is dropped rather than refused, for the reason a file that has stopped
            // matching is dropped: the search is a moment old.
            let Some((text, target)) = sweep_text_and_target(held(&path), &path) else { continue };
            let scan = scan_for_replacements(&text, &re, &asked.replacement, asked.regex);
            // The file has stopped matching between the walk and now — something wrote it, or a
            // buffer was typed into. Silently left out: that is not an error, it is a search
            // being a moment old, and a refusal here would make every sweep hostage to whatever
            // an agent happens to be doing in another file.
            if scan.edits.is_empty() {
                continue;
            }
            total += scan.edits.len();
            rows.push(i18n::msg_preview_file_header(
                lang,
                &path.strip_prefix(&self.root).unwrap_or(&path).display().to_string(),
                scan.edits.len(),
            ));
            rows.extend(diff_rows(&scan.lines, &scan.edits, |line| scan.line_starts[line]));
            files.push(SweepFile { path, target, edits: scan.edits });
        }
        // Everything the search found has moved on. Said rather than shown, because the
        // alternative is an empty box asking to be agreed to.
        if files.is_empty() {
            return Err(i18n::msg_replace_nothing_left(lang).to_string());
        }

        // Where the keyboard is, as an offset, worked out here because here is where the text it
        // is an offset into still exists.
        let from = self.editor().path.clone().map(|path| {
            let editor = self.editor();
            (path, editor.cursor_line, editor.cursor_col)
        });
        let from_char = from
            .as_ref()
            .and_then(|(path, line, col)| {
                held(path).map(|e| {
                    e.rope.line_to_char((*line).min(e.rope.len_lines().saturating_sub(1))) + col
                })
            })
            .unwrap_or(0);
        Ok(ReplaceSweep {
            query: outcome.query.clone(),
            replacement: asked.replacement.clone(),
            from,
            from_char,
            files,
            rows,
            edits: total,
            scroll: 0,
            body_rows: 1,
        })
    }

    /// The preview's keys, which are the rename preview's keys down to the letter — one shape of
    /// question, one set of answers.
    fn handle_replace_sweep_key(&mut self, key: KeyEvent) {
        let lang = self.settings.lang;
        let page = self.replace_sweep.as_ref().map(|s| s.body_rows.max(1) as isize).unwrap_or(1);
        if let Some(sweep) = self.replace_sweep.as_mut() {
            match key.code {
                KeyCode::Up => return sweep.scroll_by(-1),
                KeyCode::Down => return sweep.scroll_by(1),
                KeyCode::PageUp => return sweep.scroll_by(-page),
                KeyCode::PageDown => return sweep.scroll_by(page),
                KeyCode::Home => {
                    sweep.scroll = 0;
                    return;
                }
                KeyCode::End => return sweep.scroll_by(sweep.rows.len() as isize),
                _ => {}
            }
        }
        match key.code {
            KeyCode::Enter => self.apply_replace_sweep(),
            KeyCode::Char(c) if c.eq_ignore_ascii_case(&i18n::yes_key(lang)) => {
                self.apply_replace_sweep()
            }
            _ => {
                self.replace_sweep = None;
                self.status_message = i18n::msg_replace_cancelled(lang).to_string();
            }
        }
    }

    /// Writes the preview into the buffers and the files it was built against.
    ///
    /// Two phases, and the split is the all-or-nothing promise. The first re-checks every guard
    /// and works out every file's new contents in memory, writing nothing: a buffer's revision
    /// must be the one the offsets were measured against, a file's timestamp must be the one it
    /// was read at, and a file that has since been opened in a tab is a refusal rather than a
    /// disk write under somebody's undo stack. Only when all of that holds does the second phase
    /// start putting bytes anywhere.
    fn apply_replace_sweep(&mut self) {
        let lang = self.settings.lang;
        let Some(sweep) = self.replace_sweep.take() else { return };
        let moved = i18n::msg_replace_refused_moved(lang).to_string();

        let mut writes: Vec<(PathBuf, String)> = Vec::new();
        for file in &sweep.files {
            match &file.target {
                SweepTarget::OpenBuffer { revision } => {
                    let held = self
                        .editors
                        .iter()
                        .find(|e| e.path.as_deref() == Some(file.path.as_path()));
                    // The tab closed, the buffer was reloaded by the sweep in the frame loop, or
                    // it turned read-only. Any of the three and the offsets below describe text
                    // that is no longer in that rope.
                    if !held.is_some_and(|e| e.revision() == *revision && !e.read_only) {
                        self.status_message = moved;
                        return;
                    }
                }
                SweepTarget::Disk { mtime, line_ending, final_newline } => {
                    // A tab opened over it since the preview. Refused rather than written: the
                    // whole point of the two roads is that a file with a tab never takes this
                    // one, and "since the preview" does not make it an exception.
                    if self.editors.iter().any(|e| e.path.as_deref() == Some(file.path.as_path())) {
                        self.status_message = moved;
                        return;
                    }
                    let now = std::fs::metadata(&file.path).ok().and_then(|m| m.modified().ok());
                    if now != *mtime {
                        self.status_message = moved;
                        return;
                    }
                    let Ok(raw) = std::fs::read_to_string(&file.path) else {
                        self.status_message = moved;
                        return;
                    };
                    let chars: Vec<char> = raw.replace("\r\n", "\n").chars().collect();
                    // Belt and braces over the timestamp: a filesystem whose mtime has a one-
                    // second granularity can hide a write that landed in the same second, and an
                    // offset past the end of the text would be a panic rather than a refusal.
                    if file.edits.last().is_some_and(|e| e.end > chars.len()) {
                        self.status_message = moved;
                        return;
                    }
                    let Some((start, end, rebuilt)) =
                        rebuild_edits(&file.edits, |from, to| chars[from..to].iter().collect())
                    else {
                        continue;
                    };
                    let mut whole: String = chars[..start].iter().collect();
                    whole.push_str(&rebuilt);
                    whole.extend(&chars[end..]);
                    writes.push((
                        file.path.clone(),
                        text_for_disk(whole, *line_ending, *final_newline),
                    ));
                }
            }
        }

        // The files first, because they are the half that can fail — a read-only file, a full
        // disk. Stopping here leaves what was written written and every open buffer untouched,
        // which is the state the sentence below can describe truthfully; carrying on into the
        // buffers would leave the reader with edits on screen and a file that never took them.
        for (path, text) in &writes {
            if let Err(e) = settings::write_atomic(path, text.as_bytes()) {
                let shown = path.strip_prefix(&self.root).unwrap_or(path).display().to_string();
                self.status_message = i18n::msg_replace_write_failed(lang, &shown, &e.to_string());
                return;
            }
        }
        // And the buffers, which cannot. One `replace_char_range` per file, which is one
        // checkpoint and therefore one step of undo — the same shape as Replace All and as the
        // rename, and for the same reason: a sweep is one action, so taking it back in a given
        // file is one Ctrl+Z and not one per occurrence. A dirty buffer is fine; the edit goes
        // through the rope like any other.
        for file in &sweep.files {
            if !matches!(file.target, SweepTarget::OpenBuffer { .. }) {
                continue;
            }
            let Some(index) =
                self.editors.iter().position(|e| e.path.as_deref() == Some(file.path.as_path()))
            else {
                continue;
            };
            let editor = &mut self.editors[index];
            let Some((start, end, rebuilt)) = edits_as_one_span(editor, &file.edits) else {
                continue;
            };
            editor.replace_char_range(start, end, &rebuilt);
        }
        // The one buffer somebody is looking at, if the sweep reached it at all.
        let landed = sweep.from.clone().and_then(|(path, line, col)| {
            sweep.files.iter().find(|f| f.path == path).map(|file| (path, line, col, file))
        });
        if let Some((path, line, col, file)) = landed {
            self.restore_cursor_after_edits(&path, line, col, sweep.from_char, &file.edits);
        }
        let (buffers, disk) = sweep.split();
        self.status_message = i18n::msg_replace_applied(lang, sweep.edits, buffers, disk);
    }

    // ---- Breakpoints, and being stopped -----------------------------------------------------

    /// Puts a breakpoint on the cursor's line, or takes it off.
    ///
    /// For the interpreter debuggers this is written to a file for the session's hook to apply,
    /// never typed at the prompt: `dbstop` works through `evalin` from inside the hook —
    /// measured — so setting a breakpoint leaves no line in the transcript that the user did not
    /// write. For a debug adapter it goes straight down the wire. Either way the map here is the
    /// one set of breakpoints the editor has, and both backends are told about all of it.
    ///
    /// It used to refuse every file that was not `.m` or `.py`, which was the truth while Octave
    /// and Python were the only debuggers there were. A breakpoint in a `.c` or a `.rs` is now
    /// exactly what *Debug ▸ Start* stops on, and a gate on the extension would have made the
    /// compiled debugger a debugger with nowhere to stop. What is still refused is a buffer with
    /// no file: a breakpoint is a place in a file, and neither backend can be told about one that
    /// does not exist yet.
    fn toggle_breakpoint(&mut self) {
        let lang = self.settings.lang;
        let editor = self.editor();
        let Some(path) = editor.path.clone() else {
            self.status_message = i18n::msg_break_unsaved(lang);
            return;
        };
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
    ///
    /// The debug adapter is told from here too, and from here on purpose: this is the moment the
    /// breakpoints change, and hanging the second backend off the same moment is what makes a
    /// breakpoint toggled while a program is stopped reach the adapter without anybody having to
    /// remember a second call.
    fn publish_breakpoints(&mut self) {
        self.publish_breakpoints_to_adapter();
        let Some(watch) = self.figures.as_ref() else { return };
        let path = break_path_beside(&watch.path);
        // By function name, which is what `dbstop` takes and what a `.m` file is known by, and
        // by path, which is what pdb takes. Each language reads the field it can use.
        //
        // Only the files an interpreter could stop in. The map is wider than it was — a
        // breakpoint in a `.c` is a real breakpoint now — and handing pdb a path it has never
        // heard of is an error raised inside somebody's hook, in a session that was doing fine.
        let wanted: Vec<serde_json::Value> = self
            .breakpoints
            .iter()
            .filter(|(file, _)| crate::session::Language::of_path(file).is_some())
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

    /// Tells the running adapter, if there is one, where the breakpoints are now.
    ///
    /// Nothing at all when no session is running, which is the common case and costs a branch.
    /// Everything when there is one — including the files whose last breakpoint has just been
    /// taken off; see [`breakpoints_to_publish`] for why those are the ones that bite.
    fn publish_breakpoints_to_adapter(&mut self) {
        let Some(session) = self.debug.as_mut() else { return };
        for (path, lines) in breakpoints_to_publish(&session.published, &self.breakpoints) {
            session.client.set_breakpoints(&path, &lines);
            if lines.is_empty() {
                session.published.remove(&path);
            } else {
                session.published.insert(path);
            }
        }
    }

    // ---- The debugger -------------------------------------------------------------------------

    /// Everything the adapter has said since the last frame.
    ///
    /// Beside `poll_mcp` and `poll_lsp` in the frame loop, and shaped like them: drained whole,
    /// acted on one event at a time, and never waited for. The events are taken out of the client
    /// before they are applied because applying one may end the session — an `exited` drops the
    /// whole [`DebugSession`], and a loop still holding a borrow of it could not.
    pub fn poll_debug(&mut self) {
        let Some(session) = self.debug.as_mut() else { return };
        let events = session.client.poll();
        for event in events {
            self.apply_debug_event(event);
        }
    }

    /// What one event from the adapter means to the editor.
    fn apply_debug_event(&mut self, event: crate::dap::Event) {
        let lang = self.settings.lang;
        // Nothing here came from a keypress, so nothing else has marked the screen out of date,
        // and nearly every event below changes something a person can see — a status line at the
        // very least. A line of output is the one that has to be asked about: with the panel open
        // it is drawn, in the strip along the bottom, and a tail that lagged behind the program
        // would be a strip that lies; with the panel away nothing reads it, and a program printing
        // in a loop must not cost a frame each time for something nobody is looking at.
        if !matches!(event, crate::dap::Event::Output { .. }) || self.debug_panel.open {
            self.mark_dirty();
        }
        match event {
            // The client answers this one itself: the breakpoints and `configurationDone` go out
            // from inside `dap::Client::poll` the moment it arrives. There is nothing for a person
            // to see, and a status line saying "initialized" would be the editor reading its own
            // protocol out loud.
            crate::dap::Event::Initialized => {}
            // What the adapter made of one file's breakpoints. Still nothing to do with it: the
            // gutter already shows where they were asked for, the panel is about where the
            // program *is* rather than about where it was told to stop, and a line saying so on
            // every start would talk over the reason the session was started. Drawing a
            // breakpoint the adapter moved or refused differently in the gutter is real work with
            // a real audience, and it belongs to whoever does that gutter next.
            crate::dap::Event::Breakpoints { .. } => {}
            crate::dap::Event::Stopped { thread, reason, description, path, line } => {
                // A stop that named no thread leaves whatever was there: the protocol allows the
                // omission and means "everything stopped", and forgetting the thread over it
                // would turn the next step into a refusal.
                if let (Some(session), Some(thread)) = (self.debug.as_mut(), thread) {
                    session.thread = Some(thread);
                }
                // Whatever was marked belongs to the stop before this one, and the adapter is
                // not obliged to say it has moved: several never send `continued` for a step at
                // all. Cleared here so that the `StackTrace` arm below can tell "the event told
                // us where we are" from "nobody has yet" by looking at one field.
                self.stopped_at = None;
                if let (Some(path), Some(line)) = (path, line) {
                    // The adapter volunteered the place, which several do, and then the editor
                    // can follow it without waiting for an answer.
                    self.jump_to_stopped_line(path, line);
                }
                // Asked whether or not the place was volunteered, because the panel needs the
                // whole stack and not only the innermost line of it. This is the head of the
                // choreography: `stackTrace` answers, which asks `scopes`, which asks
                // `variables` one level down, and the watches go out beside them.
                self.ask_where_it_stopped();
                // A breakpoint hit while the panel was put away brings it back, as the design
                // says it must: stopping is the moment the panel is the thing you want, and
                // having to go and find a menu row first would be the editor making you ask
                // twice. Without stealing the keyboard, for the same reason the jump does not.
                if !self.debug_panel.open {
                    self.show_debug_panel();
                }
                // The adapter's own sentence when it wrote one, and the protocol's word for the
                // reason when it did not: "breakpoint", "step", "exception". Both are more use
                // than a fixed "stopped", which is the one thing the reader can already see.
                let why = description.unwrap_or(reason);
                self.status_message = i18n::msg_debugger_stopped(lang, &why);
            }
            crate::dap::Event::Continued { .. } => {
                self.clear_stopped_line();
                self.status_message = i18n::msg_debug_running(lang);
            }
            crate::dap::Event::Exited { code } => {
                self.end_debug_session();
                self.status_message = i18n::msg_debugger_exited(lang, code);
            }
            crate::dap::Event::Terminated => {
                self.end_debug_session();
                self.status_message = i18n::msg_debugger_over(lang);
            }
            crate::dap::Event::Output { category, text } => {
                if let Some(session) = self.debug.as_mut() {
                    session.remember_output(category, text);
                }
            }
            crate::dap::Event::Threads { id, threads } => {
                let Some(session) = self.debug.as_mut() else { return };
                // Two questions come back as this one event, and which was asked is the difference
                // between reading the answer and acting on it: a stop that named no thread wants
                // to know where it is, and a pause wants a thread to catch. Told apart by the seq
                // each was asked under, which is why both are held rather than counted.
                if session.awaiting_pause == Some(id) {
                    session.awaiting_pause = None;
                    // The first thread, for the same reason the stop below takes it: choosing
                    // among them is the panel's own work and nothing offers that choice yet. A
                    // program with no threads left has already finished, whatever the adapter has
                    // got round to saying about it.
                    let Some(thread) = threads.first().map(|t| t.id) else {
                        self.status_message = i18n::msg_debugger_no_thread(lang);
                        return;
                    };
                    // The stop that follows is the news, and it arrives as an ordinary `stopped`
                    // event with `pause` for its reason — so nothing else here has to know that
                    // this particular stop was asked for.
                    let _ = session.client.pause(thread);
                    return;
                }
                if session.awaiting_thread != Some(id) {
                    return;
                }
                session.awaiting_thread = None;
                // The first thread, because a stop that named none is a stop where every thread
                // is stopped, and the panel that lets somebody choose another one is wave 3.
                session.thread = threads.first().map(|t| t.id);
                self.ask_where_it_stopped();
            }
            crate::dap::Event::StackTrace { id, frames } => {
                let Some(session) = self.debug.as_mut() else { return };
                if session.awaiting_place != Some(id) {
                    return;
                }
                session.awaiting_place = None;
                // The innermost frame that has a file. A stop inside a library with no symbols
                // has frames and nowhere to point at, and the first frame that does is where the
                // reader's own code is — which is what they wanted to look at.
                let place = frames.iter().find_map(|f| f.path.clone().map(|path| (path, f.line)));
                // The panel's list, and the frame everything else on it is read in: the innermost
                // one, which is where the program actually is.
                self.debug_panel.frames = frames;
                self.debug_panel.frame = 0;
                self.refresh_debug_frame();
                // Followed only when the stop itself did not already say where it was: an adapter
                // that volunteered the place has been followed a frame ago, and jumping twice
                // would move a pane the reader may have scrolled since.
                if self.stopped_at.is_none()
                    && let Some((path, line)) = place
                {
                    self.jump_to_stopped_line(path, line);
                }
            }
            crate::dap::Event::Scopes { id, scopes } => {
                if self.debug_panel.awaiting_scopes != Some(id) {
                    return;
                }
                self.debug_panel.awaiting_scopes = None;
                self.debug_panel.scopes = scopes;
                self.open_first_level_of_scopes();
            }
            crate::dap::Event::Variables { id, variables } => {
                let Some(reference) = self.debug_panel.awaiting_children.remove(&id) else {
                    return;
                };
                self.debug_panel.children.insert(reference, variables);
            }
            crate::dap::Event::Evaluated { id, value, .. } => {
                let Some(index) = self.debug_panel.awaiting_watch.remove(&id) else { return };
                if let Some(watch) = self.debug_panel.watches.get_mut(index) {
                    watch.answer = Some(Ok(value));
                }
            }
            crate::dap::Event::Failed { id, command, message } => {
                // A watch the adapter cannot read is not a failure of the editor and does not
                // belong on the status line: "there is no variable named x here" is the honest
                // answer for a local that is not in scope in the frame being looked at, and it
                // belongs on that watch's own row, where the question is.
                if let Some(index) = self.debug_panel.awaiting_watch.remove(&id) {
                    if let Some(watch) = self.debug_panel.watches.get_mut(index) {
                        watch.answer = Some(Err(message));
                    }
                    return;
                }
                // The other two the panel asks are dropped from their waiting lists as well, so
                // that a refused `scopes` does not leave the panel waiting for an answer that is
                // never coming.
                self.debug_panel.awaiting_children.remove(&id);
                if self.debug_panel.awaiting_scopes == Some(id) {
                    self.debug_panel.awaiting_scopes = None;
                }
                self.status_message = i18n::msg_debugger_refused(lang, &command, &message);
            }
            crate::dap::Event::Dead { reason } => {
                self.end_debug_session();
                self.status_message = i18n::msg_debugger_dead(lang, &reason);
            }
        }
    }

    /// Asks the adapter where the program is, because the stop did not say.
    fn ask_where_it_stopped(&mut self) {
        let Some(session) = self.debug.as_mut() else { return };
        match session.thread {
            Some(thread) => session.awaiting_place = session.client.stack_trace(thread),
            // A stop attributed to no thread is allowed by the protocol and means every thread
            // stopped. Asking which threads exist is the honest next question; inventing a thread
            // id to step with would be the editor telling the adapter something it never said.
            None => session.awaiting_thread = session.client.threads(),
        }
    }

    /// Follows the adapter into the file the program stopped in.
    ///
    /// The same two things the interpreter debugger does, in the same order and through the same
    /// machinery: the line is remembered in [`Self::stopped_at`], which is what the renderer marks
    /// in the gutter and highlights across the row, and the file is *shown* rather than opened —
    /// the keyboard stays exactly where it was, because a session that stole the cursor mid-word
    /// would put the next thing typed into the file being debugged.
    fn jump_to_stopped_line(&mut self, path: PathBuf, line: usize) {
        // Adapters are allowed to answer with a path relative to the cwd they were started in,
        // which is the project root — the same root a relative path in the file tree is read
        // against, so resolving it here is not a guess.
        let path = if path.is_absolute() { path } else { self.root.join(path) };
        let path = self.as_the_editor_spells_it(path);
        self.stopped_at = Some((path.clone(), line));
        // Following the program does not get to change what the status line says. Showing a file
        // announces itself — "Opened: twice.c" — and for every adapter that leaves the place out
        // of its `stopped` event, and so is followed a `stackTrace` later, that announcement
        // lands *after* the sentence saying why the program stopped and wipes it out. The reader
        // is then told the one thing they can already see, instead of the one thing they cannot.
        let said = std::mem::take(&mut self.status_message);
        self.show_beside_without_focus(path, Some(line), None);
        self.status_message = said;
    }

    /// The name this editor already has for a file the adapter has just named.
    ///
    /// One file, two spellings, and they have to be reconciled somewhere. The adapter is told a
    /// path with the symlinks followed — see [`crate::dap`]'s `as_the_adapter_reads_it` for why it
    /// has to be — and it answers with that one; the editor holds whatever the project was opened
    /// as, which on a Mac is a `/var/folders/…` where the adapter says `/private/var/folders/…`.
    /// Taken as it comes, that answer would open a *second* tab of a file that is already on
    /// screen, mark the stopped line in the copy without the breakpoints, and leave the gutter mark
    /// in the copy without the stop.
    ///
    /// So an open tab that is the same file wins, by identity rather than by spelling. Nothing
    /// open means nothing to reconcile, and the adapter's own answer is then the best name there
    /// is.
    fn as_the_editor_spells_it(&self, path: PathBuf) -> PathBuf {
        if self.editors.iter().any(|e| e.path.as_deref() == Some(path.as_path())) {
            return path;
        }
        self.editors
            .iter()
            .filter_map(|editor| editor.path.clone())
            .find(|held| same_file(held, &path))
            .unwrap_or(path)
    }

    /// The program is running again, so the mark on the line stops being true.
    fn clear_stopped_line(&mut self) {
        self.stopped_at = None;
        if let Some(session) = self.debug.as_mut() {
            session.thread = None;
        }
        // The one place the panel is told the program has moved, because this is the one place
        // that already knows: every frame, scope and value it holds is an answer about the place
        // that has just stopped being where the program is. See [`DebugPanel::forget_stop`].
        self.debug_panel.forget_stop();
        self.mark_dirty();
    }

    /// Drops the session and everything drawn on its behalf.
    ///
    /// Dropping the [`dap::Client`] is what ends the adapter: its `Drop` disconnects and then
    /// makes sure the process is gone, which is the same bargain the language server client
    /// makes — an orphaned `lldb-dap` still holding a stopped process would be a far worse
    /// outcome than an impolite exit.
    fn end_debug_session(&mut self) {
        self.clear_stopped_line();
        self.debug = None;
        // The column goes with the session it was about. The watches stay — they are the
        // questions somebody wrote, not the answers this run gave — which is the whole reason
        // they live on the panel rather than in the session.
        self.debug_panel.open = false;
        self.debug_panel.selected = 0;
        // The keyboard cannot stay in a frame that is no longer drawn.
        if self.focus == Focus::Debug {
            self.focus = Focus::Editor;
        }
    }

    /// What this project's *Debug ▸ Start* would run. See [`debuggee_for`].
    fn debuggee_to_run(&self) -> PathBuf {
        debuggee_for(&self.root, self.debuggee.as_deref())
    }

    /// What to call the debuggee in a sentence: its path inside the project, or the whole of it
    /// where that comes to nothing — which is what the project root itself does, and the root is
    /// exactly what the guess falls back to when it has nothing better.
    fn debuggee_name(&self, program: &Path) -> String {
        let short = self.mcp_short(program);
        if short.is_empty() { program.to_string_lossy().into_owned() } else { short }
    }

    /// Asks what to debug, with the guess filled in.
    ///
    /// The design's rule, made good: *the editor does not guess silently*. What [`debuggee_for`]
    /// worked out is put in the box as ordinary editable text, so accepting it is one keystroke
    /// and correcting it is typing — which is the whole difference between a guess offered and a
    /// guess acted on.
    ///
    /// The refusal for a session that is already running comes *before* the box rather than after
    /// it: being asked which program to debug and then told that one is already being debugged
    /// would be a question that never had an answer.
    fn open_debug_start(&mut self) {
        let lang = self.settings.lang;
        if self.debug.is_some() {
            self.status_message = i18n::msg_debugger_already_running(lang);
            return;
        }
        let typed = debuggee_prefill(&self.root, self.debuggee.as_deref());
        self.debug_prompt = Some(DebugPrompt { ask: DebugAsk::Debuggee, typed });
    }

    /// The answer, whatever was left in the box.
    ///
    /// Remembered before it is tried, and remembered even where the start then fails: a path to a
    /// binary that has not been built yet is a perfectly good answer to "what do you debug here",
    /// and making the user retype it after every failed start would punish them for the build.
    ///
    /// An emptied box means the guess again — the convention this editor already uses for the run
    /// command box, where clearing the field puts the default back rather than setting the value
    /// to nothing.
    fn debug_start_answered(&mut self, typed: &str) {
        let typed = typed.trim();
        self.debuggee = (!typed.is_empty()).then(|| PathBuf::from(typed));
        self.debug_start();
    }

    /// Starts a debug session on the answer given above.
    fn debug_start(&mut self) {
        let lang = self.settings.lang;
        if self.debug.is_some() {
            self.status_message = i18n::msg_debugger_already_running(lang);
            return;
        }
        // The setting first, then the search. A machine with `lldb-dap` on its `PATH` and a line
        // in settings.toml pointing at something else means the line: discovery is the
        // convenience, and a configured adapter that lost to a discovered one would be a setting
        // that does nothing on exactly the machines it was written for.
        let Some(adapter) =
            configured_adapter(&self.settings.debug_adapter).or_else(crate::dap::find_adapter)
        else {
            self.status_message = i18n::msg_debugger_no_adapter(lang, std::env::consts::OS);
            return;
        };
        let program = self.debuggee_to_run();
        if !program.is_file() {
            self.status_message = i18n::msg_debugger_no_debuggee(lang, &self.debuggee_name(&program));
            return;
        }
        let cwd = self.root.clone();
        let mut client = match crate::dap::Client::start(&adapter, &cwd) {
            Ok(client) => client,
            Err(e) => {
                self.status_message = i18n::msg_debugger_adapter_failed(lang, &e);
                return;
            }
        };
        // Arguments are none and the working directory is the project root. Both are what the
        // prompt above would have carried, and both are what the design says the prompt should
        // ask for — so they are written down here as the answer given on the user's behalf,
        // rather than as fields nobody ever filled in.
        client.launch(&program, &[], &cwd);
        self.debug = Some(DebugSession::new(client, program.clone(), Vec::new(), cwd));
        // Every breakpoint in the editor, at once. The client holds them until the adapter says
        // it is ready to be configured, so this is early rather than too early.
        self.publish_breakpoints_to_adapter();
        self.debuggee = Some(program.clone());
        // The column, from the moment there is a session for it to be about. Nothing is in it
        // until the program stops, and that is the point: it says "running…" while the program
        // runs, so the first breakpoint hit lands somewhere the reader is already looking.
        self.show_debug_panel();
        self.status_message = i18n::msg_debugger_started(lang, &adapter.name(), &self.debuggee_name(&program));
    }

    /// Ends the session, taking the debuggee with it.
    ///
    /// `terminateDebuggee: true`, because the program was started by this editor for this
    /// session: leaving a stopped process behind when the thing that could resume it has gone is
    /// how a machine collects debuggees nobody can see.
    fn debug_stop(&mut self) {
        let lang = self.settings.lang;
        let Some(session) = self.live_session() else {
            self.status_message = i18n::msg_debugger_no_session(lang);
            return;
        };
        session.client.stop();
        // Named while it is still here to name: `end_debug_session` drops it.
        let program = session.program.clone();
        let program = self.debuggee_name(&program);
        self.end_debug_session();
        self.status_message = i18n::msg_debugger_ended(lang, &program);
    }

    /// Continues, or takes one step of whichever size was asked for.
    ///
    /// One place for all four, because the two refusals in front of them are the same two
    /// whichever was asked for: there has to be a session, and it has to be stopped. A step sent
    /// into a running program is a step the adapter fails, which the user then hears about as a
    /// refusal in the adapter's words rather than in an answer they can act on.
    fn debug_step(&mut self, verb: DebugVerb) {
        let lang = self.settings.lang;
        let Some(session) = self.live_session() else {
            self.status_message = i18n::msg_debugger_no_session(lang);
            return;
        };
        let Some(thread) = session.thread else {
            self.status_message = i18n::msg_debugger_not_stopped(lang);
            return;
        };
        // The seq each of these returns is the handle for its answer, and none of them has an
        // answer worth showing: a step that worked is announced by the `stopped` event that
        // follows it, and one that did not arrives as `Failed`.
        let _ = match verb {
            DebugVerb::Continue => session.client.continue_(thread),
            DebugVerb::StepOver => session.client.next(thread),
            DebugVerb::StepIn => session.client.step_in(thread),
            DebugVerb::StepOut => session.client.step_out(thread),
        };
    }

    /// The session, when there is one and it is still answering.
    ///
    /// The three verbs around it ask through this rather than for `self.debug` directly,
    /// because a client whose adapter has died drops every request on the floor and returns
    /// `None`: without the second half of the question, a *Continue* pressed in the moment between
    /// an adapter dying and the next frame noticing would do nothing and say nothing. The session
    /// is left standing — the poll that surfaces the death is the thing that clears it, and the
    /// sentence it prints is the one that explains what happened.
    fn live_session(&mut self) -> Option<&mut DebugSession> {
        self.debug.as_mut().filter(|session| !session.client.is_dead())
    }

    /// Catches a running program where it happens to be.
    ///
    /// The odd one out of the verbs, and the whole design of it follows from that: every other row
    /// in the menu needs the program stopped, and this one needs it running. So the two refusals
    /// are the two the others give, turned around — no session at all, and a program that is
    /// already stopped, which is the state where *Continue* is what was meant.
    ///
    /// The thread is asked for rather than remembered. DAP's `pause` names one, a running program
    /// is exactly the one that has not stopped to tell us which it is on, and the thread it was
    /// last stopped on may since have ended — so the adapter is asked what threads there are now
    /// and the answer picks the one to catch. See the `Threads` arm of [`Self::apply_debug_event`],
    /// where the two questions that come back as that one event are told apart.
    fn debug_pause(&mut self) {
        let lang = self.settings.lang;
        let Some(session) = self.live_session() else {
            self.status_message = i18n::msg_debugger_no_session(lang);
            return;
        };
        if session.thread.is_some() {
            self.status_message = i18n::msg_debugger_already_stopped(lang);
            return;
        }
        // A session whose adapter has not yet said it is ready to be configured has not started
        // the program either: there is nothing running to be caught, and saying so is more use
        // than the adapter's own complaint about a request it was not ready for.
        if !session.client.announced() {
            self.status_message = i18n::msg_debugger_still_starting(lang);
            return;
        }
        session.awaiting_pause = session.client.threads();
        self.status_message = i18n::msg_debugger_pausing(lang);
    }

    // ---- The debug panel ----------------------------------------------------------------------

    /// Whether the panel has a column right now.
    ///
    /// The session is asked as well as the flag, and on purpose: the panel is about a session, so
    /// a column left standing after the program exited would be a frame with nothing in it that
    /// the focus ring still had to walk through.
    pub fn debug_panel_is_open(&self) -> bool {
        self.debug.is_some() && self.debug_panel.open
    }

    /// Whether the program is stopped, which is the question the whole panel hangs on: stopped,
    /// there are frames, scopes and values; running, there is one dim line saying so.
    pub fn debug_is_stopped(&self) -> bool {
        self.debug.as_ref().is_some_and(|s| s.thread.is_some())
    }

    /// Every row the panel draws, from the state it has.
    pub fn debug_rows(&self) -> Vec<DebugRow> {
        debug_panel_rows(&self.debug_panel, self.debug_is_stopped(), self.settings.lang)
    }

    /// The tail of the session's output, for the strip along the bottom of the panel.
    pub fn debug_output_tail(&self, rows: usize) -> Vec<String> {
        self.debug.as_ref().map(|s| s.output_tail(rows)).unwrap_or_default()
    }

    /// Shows the panel because a session has begun.
    ///
    /// Without taking the keyboard, which is the same rule the stopped-line jump follows: a
    /// session starting is something the editor does *beside* what you were typing, and a column
    /// that appeared and swallowed the next keystroke would be the opposite of that promise. The
    /// menu row below takes the focus, because asking for the panel is asking to use it.
    fn show_debug_panel(&mut self) {
        self.debug_panel.open = true;
        self.debug_panel.selected = 0;
    }

    /// The Debug menu's own row: the panel, on or off, for a session that is already running.
    fn toggle_debug_panel(&mut self) {
        let lang = self.settings.lang;
        if self.debug.is_none() {
            self.status_message = i18n::msg_debugger_no_session(lang);
            return;
        }
        self.debug_panel.open = !self.debug_panel.open;
        if self.debug_panel.open {
            self.focus = Focus::Debug;
        } else if self.focus == Focus::Debug {
            self.focus = Focus::Editor;
        }
        self.status_message = i18n::msg_debug_panel_toggled(lang, self.debug_panel.open);
    }

    /// Asks everything that is about the frame the panel is now reading in.
    ///
    /// Called on every stop and on every frame change, and it is the whole of "the panel follows
    /// the selected frame": the scopes are asked for again, whatever was expanded under the old
    /// frame is dropped — those references belong to a frame nobody is looking at — and every
    /// watch is put to the adapter again in the new frame's context, because a local means
    /// something different one frame up.
    fn refresh_debug_frame(&mut self) {
        self.debug_panel.scopes.clear();
        self.debug_panel.children.clear();
        self.debug_panel.expanded.clear();
        self.debug_panel.awaiting_scopes = None;
        self.debug_panel.awaiting_children.clear();
        let frame = self.debug_panel.current_frame().map(|f| f.id);
        if let (Some(session), Some(frame)) = (self.debug.as_mut(), frame) {
            self.debug_panel.awaiting_scopes = session.client.scopes(frame);
        }
        self.evaluate_watches();
    }

    /// Opens every scope of the stopped frame, one level and no further.
    ///
    /// One level is the design's own cap, and it is not a nicety: a `variables` for every
    /// reference that came back would walk a linked list to its end, and a structure with a cycle
    /// in it forever — on every step. Everything below the first level waits for somebody to
    /// press Enter on it.
    ///
    /// A scope that says it is expensive is left closed. That flag exists precisely for the
    /// panel that expands on every stop, and reading a whole register file on each step of a
    /// program is what ignoring it costs.
    ///
    /// The cap is the loop: it walks the scopes and stops. A pass that followed every reference
    /// it was handed back would walk a linked list to its end, and a structure with a cycle in it
    /// forever — on every single step.
    fn open_first_level_of_scopes(&mut self) {
        let wanted: Vec<i64> = self
            .debug_panel
            .scopes
            .iter()
            .filter(|scope| scope.reference != 0 && !scope.expensive)
            .map(|scope| scope.reference)
            .collect();
        for reference in wanted {
            self.debug_panel.expanded.insert(reference);
            self.ask_for_variables(reference);
        }
    }

    /// Asks what is inside one reference, unless the answer is already here.
    ///
    /// The cap is not in here and cannot be: this asks about exactly the one reference it is
    /// given, and it is the callers that decide how many of those there are. The automatic pass
    /// above asks once per scope and stops; a keypress asks once per Enter.
    fn ask_for_variables(&mut self, reference: i64) {
        if reference == 0 || self.debug_panel.children.contains_key(&reference) {
            return;
        }
        let Some(session) = self.debug.as_mut() else { return };
        if let Some(seq) = session.client.variables(reference) {
            self.debug_panel.awaiting_children.insert(seq, reference);
        }
    }

    /// Puts every watch to the adapter again, in the frame the panel is reading in.
    ///
    /// All of them, every time, rather than only the ones that changed: nothing here knows which
    /// expression a step made true, and an expression that used to fail and now reads is exactly
    /// what somebody added a watch to find out.
    fn evaluate_watches(&mut self) {
        self.debug_panel.awaiting_watch.clear();
        let frame = self.debug_panel.current_frame().map(|f| f.id);
        let expressions: Vec<(usize, String)> = self
            .debug_panel
            .watches
            .iter()
            .enumerate()
            .map(|(i, watch)| (i, watch.expression.clone()))
            .collect();
        for (index, expression) in expressions {
            if let Some(watch) = self.debug_panel.watches.get_mut(index) {
                watch.answer = None;
            }
            let Some(session) = self.debug.as_mut() else { return };
            if let Some(seq) = session.client.evaluate(&expression, frame) {
                self.debug_panel.awaiting_watch.insert(seq, index);
            }
        }
    }

    /// Keys while the panel has the keyboard. See [`debug_panel_key`] for the table.
    fn handle_debug_panel_key(&mut self, key: KeyEvent) {
        let Some(action) = debug_panel_key(key) else { return };
        match action {
            // Straight to wave 2's dispatch, refusals and all: a `c` pressed at a program that is
            // running gets the same sentence the menu row would have given it, because it is the
            // same question asked from a different place.
            DebugPanelKey::Verb(verb) => self.debug_step(verb),
            DebugPanelKey::Stop => self.debug_stop(),
            DebugPanelKey::AddWatch => {
                self.debug_prompt = Some(DebugPrompt { ask: DebugAsk::Watch, typed: String::new() })
            }
            DebugPanelKey::DropWatch => self.drop_selected_watch(),
            DebugPanelKey::Move(delta) => self.move_debug_selection(delta),
            DebugPanelKey::Act => self.act_on_debug_row(),
            DebugPanelKey::Leave => self.focus = Focus::Editor,
        }
    }

    /// Moves the cursor to the next row it may land on, skipping the captions and the notes.
    ///
    /// Stops at either end rather than wrapping, the way every other list in this editor does: a
    /// cursor that jumped from the last watch back to the top frame would read as the panel having
    /// scrolled rather than as the end of it.
    fn move_debug_selection(&mut self, delta: i32) {
        let rows = self.debug_rows();
        self.debug_panel.selected = debug_next_row(&rows, self.debug_panel.selected, delta);
    }

    /// Enter on whatever the cursor is on.
    ///
    /// Three rows and three meanings, which is what the design asks for: a frame is a place, so
    /// Enter goes there; a variable with something inside it is a box, so Enter opens or shuts it;
    /// a watch is already showing everything it has, so Enter does nothing to it.
    fn act_on_debug_row(&mut self) {
        let rows = self.debug_rows();
        let Some(row) = rows.get(self.debug_panel.selected) else { return };
        match row.kind.clone() {
            DebugRowKind::Frame { index, .. } => self.select_debug_frame(index),
            DebugRowKind::Variable { reference, expanded } => {
                if reference == 0 {
                    return;
                }
                if expanded {
                    self.debug_panel.expanded.remove(&reference);
                } else {
                    self.debug_panel.expanded.insert(reference);
                    // One level, because one Enter was pressed. Nothing under this is fetched
                    // until somebody presses Enter on that too, which is what keeps a structure
                    // pointing at itself from being walked to the end of memory.
                    self.ask_for_variables(reference);
                }
            }
            _ => {}
        }
    }

    /// Reads the panel in another frame of the stack, and shows where that frame is.
    ///
    /// The jump goes through the same show-beside-without-focus discipline every programmatic
    /// navigation here uses: looking one frame up is looking, and it must not take the keyboard
    /// out of the panel you are looking from.
    fn select_debug_frame(&mut self, index: usize) {
        if self.debug_panel.frames.get(index).is_none() {
            return;
        }
        self.debug_panel.frame = index;
        let place = self
            .debug_panel
            .frames
            .get(index)
            .and_then(|f| f.path.clone().map(|path| (path, f.line)));
        if let Some((path, line)) = place {
            let path = if path.is_absolute() { path } else { self.root.join(path) };
            // Under the name the editor already has for it, for the same reason the stopped line
            // is. See [`Self::as_the_editor_spells_it`].
            let path = self.as_the_editor_spells_it(path);
            self.show_beside_without_focus(path, Some(line), None);
        }
        self.refresh_debug_frame();
    }

    /// Adds one watch and asks for it straight away, so that a expression typed at a stopped
    /// program answers now rather than at the next step.
    fn add_watch(&mut self, expression: &str) {
        let expression = expression.trim();
        if expression.is_empty() {
            return;
        }
        let lang = self.settings.lang;
        self.debug_panel
            .watches
            .push(DebugWatch { expression: expression.to_string(), answer: None });
        let index = self.debug_panel.watches.len() - 1;
        let frame = self.debug_panel.current_frame().map(|f| f.id);
        if let Some(session) = self.debug.as_mut()
            && let Some(seq) = session.client.evaluate(expression, frame)
        {
            self.debug_panel.awaiting_watch.insert(seq, index);
        }
        self.status_message = i18n::msg_watch_added(lang, expression);
    }

    /// `d` on a watch row: takes that one off the list.
    ///
    /// Only on a watch row. `d` over a frame or a variable does nothing rather than dropping the
    /// last watch, because a key that acts on something other than what the cursor is on is a key
    /// nobody can aim.
    fn drop_selected_watch(&mut self) {
        let lang = self.settings.lang;
        let rows = self.debug_rows();
        let Some(DebugRowKind::Watch { index }) = rows.get(self.debug_panel.selected).map(|r| r.kind.clone())
        else {
            return;
        };
        if index >= self.debug_panel.watches.len() {
            return;
        }
        let gone = self.debug_panel.watches.remove(index);
        // The answers still out are addressed by position, and every position after this one has
        // just moved. Dropped rather than renumbered: the next stop asks for all of them again,
        // and an answer landing on the wrong row in between would be worse than a blank one.
        self.debug_panel.awaiting_watch.clear();
        self.status_message = i18n::msg_watch_removed(lang, &gone.expression);
    }

    /// A click in the panel: the keyboard comes here, and the row under the pointer is selected.
    ///
    /// The row is worked out from `ui::debug_panel_areas` and `ui::debug_scroll` — the same two
    /// functions the drawing used to put it there — so the click and the screen cannot disagree
    /// about which row is where. A click on a caption or on a note moves the focus and leaves the
    /// cursor alone, because there is nothing there to select.
    fn click_debug_panel(&mut self, rect: Rect, col: u16, row: u16) {
        self.focus = Focus::Debug;
        let body = ui::debug_panel_areas(ui::inner_rect(rect)).rows;
        if !within(body, col, row) {
            return;
        }
        let rows = self.debug_rows();
        let scroll = ui::debug_scroll(rows.len(), body.height as usize, self.debug_panel.selected);
        let index = scroll + (row - body.y) as usize;
        if rows.get(index).is_some_and(|r| r.selectable()) {
            self.debug_panel.selected = index;
        }
    }

    /// Keys while one of the debugger's two single-line boxes is up.
    ///
    /// Ordinary typing, exactly like the Go-to-line box beside it — the only difference being
    /// that this one takes any character rather than digits, because a path and an expression are
    /// both text.
    fn handle_debug_prompt_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.debug_prompt = None,
            KeyCode::Enter => {
                let Some(prompt) = self.debug_prompt.take() else { return };
                match prompt.ask {
                    DebugAsk::Debuggee => self.debug_start_answered(&prompt.typed),
                    DebugAsk::Watch => self.add_watch(&prompt.typed),
                }
            }
            KeyCode::Backspace => {
                if let Some(prompt) = self.debug_prompt.as_mut() {
                    pop_grapheme(&mut prompt.typed);
                }
            }
            KeyCode::Char(c) if is_a_typed_character(key) => {
                if let Some(prompt) = self.debug_prompt.as_mut() {
                    prompt.typed.push(c);
                }
            }
            _ => {}
        }
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
            i18n::t(lang, Key::PickerVariables),
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
            self.redraw = true;
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
            _ if self.keymap.matches(KeyAction::InspectVariable, key) => {
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
    /// grep — not only on the tracebacks it was written for. When the line is a URL instead,
    /// it opens that in the browser.
    fn open_location_at(&mut self, pane: usize, row: u16) -> bool {
        let Some(text) = self.window_tab_mut(pane).and_then(|t| t.row_text(row)) else {
            return false;
        };
        let lang = self.settings.lang;
        // A URL that parses as a `path:line` — `http://localhost:3000` reads as the file
        // "http://localhost" at line 3000 — is a URL all the same, and not a file worth going
        // to look for.
        let location = crate::locate::find(&text).filter(|at| !crate::locate::is_http_url(&at.path));
        if let Some(at) = &location
            && let Some(path) = crate::locate::resolve(at, &self.root)
        {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            self.open_file_at(path, at.line.saturating_sub(1), at.column.saturating_sub(1));
            self.status_message = i18n::msg_jumped_to(lang, &name, at.line);
            return true;
        }
        // Nothing here to open as a file. A URL on the same row is worth more than a complaint
        // about a file that is not here — `see foo.py:3 and https://docs…` names both, and only
        // one of them can be reached.
        if self.open_url_at(&text, lang) {
            return true;
        }
        // It named something, and the something is not here. Saying so beats a double-click
        // that silently does nothing, and beats opening a file of that name from elsewhere.
        match location {
            Some(at) => {
                self.status_message = i18n::msg_jump_not_found(lang, &at.path);
                true
            }
            None => false,
        }
    }

    /// Opens the URL in a line of terminal output, when there is one. `true` when it opened one.
    ///
    /// Where the URL ends and what of the punctuation around it belongs to the sentence is
    /// `locate::find_url`'s to decide. What the opener did with it — the URL may be dead, or the
    /// opener may have failed — is a message rather than silence, because a double-click that
    /// does nothing says nothing.
    fn open_url_at(&mut self, text: &str, lang: Lang) -> bool {
        let Some(url) = crate::locate::find_url(text) else {
            return false;
        };
        self.status_message = match crate::dnd::open_url(url) {
            Ok(()) => i18n::msg_opened_url(lang, url),
            Err(e) => i18n::msg_open_url_failed(lang, url, &e),
        };
        true
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
            // A new snapshot moves the workspace view, the variables pane and whatever the
            // debugger is pointing at.
            self.redraw = true;
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
                    self.redraw = true;
                }
                None => {
                    self.open_figure_tab(path);
                    self.redraw = true;
                }
            }
        }
    }

    /// The buffers actually on screen: one per editor pane, which with the split closed is one.
    ///
    /// Not the same question as "which buffers are open" and not the same as "which one has the
    /// keyboard": a split shows two files at once, and the one being *looked at* in the other
    /// half is as visible as the one being typed in.
    fn on_screen_editors(&self) -> [Option<usize>; 2] {
        let showing = |pane: EditorPane| {
            let idx = self.pane_editor_index(pane);
            (!self.pane_tabs(pane).is_empty()).then_some(idx)
        };
        [showing(EditorPane::Left), self.split_view.then(|| showing(EditorPane::Right)).flatten()]
    }

    /// Puts the next frame of every animated picture on screen up when its time has come.
    ///
    /// The clock is the file's own: each frame carries how long it is shown for, and this only
    /// acts on the ones whose time is up, so calling it every turn of the loop costs a
    /// comparison per visible tab and nothing else. There is no timer and no thread — the loop
    /// already wakes thirty times a second for the keyboard, and an animation is exactly the
    /// kind of thing that should ride on a wake-up somebody else is paying for.
    ///
    /// Only what is on screen moves. An animation in a background tab has no viewer, and
    /// decoding and transmitting frames for one would be work whose entire output is discarded
    /// — while the frames themselves are kept, so bringing the tab forward carries straight on
    /// rather than starting again.
    ///
    /// It touches the keyboard, the focus and the tab strip not at all. A picture that moves is
    /// something you look at, and the moment one could take the cursor away from what is being
    /// written it would stop being worth having — the same rule a figure from a live session
    /// follows, for the same reason.
    pub fn poll_animations(&mut self) {
        let now = std::time::Instant::now();
        for idx in self.on_screen_editors().into_iter().flatten() {
            let due = self.editors[idx]
                .preview
                .as_mut()
                .and_then(|p| p.animation.as_mut())
                .is_some_and(|animation| animation.due(now));
            if due {
                self.show_frame(idx);
            }
        }
    }

    /// Puts the frame an animation is on up, fitted to the pane as it is right now.
    ///
    /// The one place a frame becomes a picture, whether the clock moved the animation on or the
    /// zoom moved under it. The three steps are the ones a picture arriving from the decoder
    /// takes — fitted to the pane, cropped to the window being looked at, handed to the
    /// protocol already on screen — and the last of those is what makes this a repaint rather
    /// than a blink: `show` keeps the id the terminal knows the picture by.
    fn show_frame(&mut self, idx: usize) {
        let Some(preview) = self.editors.get_mut(idx).and_then(|e| e.preview.as_mut()) else {
            return;
        };
        // Never drawn, so there is no pane to fit a frame into yet. Whatever opened the tab is
        // on its way down the ordinary picture road, and that is what will fill it.
        let (cols, rows) = (preview.area_cols, preview.area_rows);
        if cols == 0 || rows == 0 {
            return;
        }
        let (box_px, fit) = (preview.picture_box(), preview.fit);
        let Some(frame) = preview.animation.as_ref().and_then(|a| a.current()) else { return };
        let image = crate::preview::scale_frame(frame.clone(), box_px, fit);
        let window = preview.window_of(&image);
        preview.fitted_for = (cols, rows);
        preview.full = Some(image);
        preview.show(window);
        // Nothing else knows a frame changed, and a frame put up in a buffer nobody draws is a
        // frame nobody sees.
        self.redraw = true;
    }

    /// Fits every untouched picture on screen to the pane it is actually in.
    ///
    /// A picture is sized for its pane exactly once, where the decoder's answer arrives, against
    /// whatever the pane measured at that instant. Everything that changes the pane afterwards —
    /// a window resized, a seam dragged, the split opened or closed — leaves the picture sized
    /// for a pane that is gone, and the one place that would have noticed is the renderer, which
    /// only writes the new size down. Worse, a figure from a running script arrives *before* its
    /// pane has ever been drawn: it is fitted to nothing, and what reaches the screen is a
    /// pane-sized cut of the top-left corner of a full-size figure. A reader cannot tell a
    /// picture opened cropped from a script that plotted rubbish, so an opened figure has to be
    /// the whole figure — which means the pane, not the moment of arrival, has to be what
    /// decides the size.
    ///
    /// Only what is on screen and only what nobody has aimed by hand: a zoom or a pan is a
    /// decision about one picture, and a resize is no reason to overrule it.
    pub fn refit_previews(&mut self) {
        for idx in self.on_screen_editors().into_iter().flatten() {
            let stale =
                self.editors.get(idx).and_then(|e| e.preview.as_ref()).is_some_and(|p| p.needs_refit());
            if !stale {
                continue;
            }
            // Written down before the work starts, so a seam being dragged asks for one re-fit
            // per size it passes through rather than one per frame — and so a read that comes
            // back a failure is not asked for again and again at the same size.
            if let Some(preview) = self.editors[idx].preview.as_mut() {
                preview.fitted_for = (preview.area_cols, preview.area_rows);
            }
            // A picture that moves has every frame already in hand at its own size, so it is
            // re-fitted here and now. Re-reading the file would work too, and would also put the
            // animation back to its first frame — a visible jump for a change that is not about
            // the file at all. `rerender_preview` refuses it for the same reason.
            if self.editors[idx].preview.as_ref().is_some_and(|p| p.animation.is_some()) {
                self.show_frame(idx);
                continue;
            }
            // A still is read again at the new size rather than resampled from the copy in hand:
            // the copy was shrunk to the old pane, and enlarging that is how a figure ends up
            // soft in a pane that just got bigger. This is the same road a zoom takes, and it
            // keeps the picture on screen while the new one is decoded.
            self.reread_preview(idx, None);
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
        let read = (settling || looked.elapsed() >= RUN_WATCH_INTERVAL).then(|| {
            let dir = crate::wsnap::snapshot_dir();
            (
                crate::wsnap::open_figures(&dir, language.snapshot_lang()),
                crate::wsnap::snapshot_generation(&dir, language.snapshot_lang()),
            )
        });
        // A pane that is gone takes its run with it: there is nothing left to be at a prompt,
        // and nothing left to close figures in either.
        let at_prompt =
            self.terminals.get(terminal).map(|w| w.active_tab().is_at_prompt()).unwrap_or(true);
        let Some(watch) = self.run_watch.as_mut() else { return };
        if let Some((open, _)) = read.as_ref() {
            watch.looked = std::time::Instant::now();
            for number in open {
                if !watch.before.contains(number) && !watch.opened.contains(number) {
                    watch.opened.push(*number);
                    // A figure arriving while the run is being closed means it is still
                    // publishing, so the wait starts again from here rather than expiring in the
                    // middle of the burst. Only then: before the prompt is back there is nothing
                    // being waited for, and a stamp left from mid-run would already be stale.
                    if watch.settled.is_some() {
                        watch.quiet = Some(std::time::Instant::now());
                    }
                }
            }
        }
        if !at_prompt {
            watch.busy_seen = true;
            watch.settled = None;
            watch.generation = None;
            watch.quiet = None;
            return;
        }
        // Still at the prompt because the command has not started yet, rather than because it
        // has finished. A script quick enough never to be caught running is over by the time
        // the wait runs out, and its figures are in the snapshot by then too.
        if !watch.busy_seen && watch.started.elapsed() < RUN_WATCH_MAX {
            return;
        }
        let back = *watch.settled.get_or_insert_with(std::time::Instant::now);
        // The reading is throttled and the prompt is not, so the first tick after the prompt
        // returns can have nothing to compare. Nothing is lost by waiting for the next one: the
        // line above has already put the watch into its settling phase, which reads every tick.
        let Some((_, generation)) = read else { return };
        // Figures already attributed to this run are a session that has written, whatever the
        // counter says — that is the ordinary fast case, and it must not wait for the timeout.
        if *watch.generation.get_or_insert(generation) != generation || !watch.opened.is_empty() {
            watch.quiet.get_or_insert_with(std::time::Instant::now);
        }
        let quiet = watch.quiet.is_some_and(|since| since.elapsed() >= RUN_SETTLE);
        if !quiet && back.elapsed() < RUN_SETTLE_MAX {
            return;
        }
        let Some(watch) = self.run_watch.take() else { return };
        if !watch.opened.is_empty() {
            self.run_figures.insert(watch.file, watch.opened);
            self.redraw = true;
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

    // ---- The MCP bridge ------------------------------------------------------------------

    /// One call per frame: publish what the editor knows, and act on what an agent has asked for.
    ///
    /// Both halves are throttled inside [`crate::mcp::Session`] rather than here, so the frame
    /// loop can call this unconditionally the way it calls every other poll. Assembling a state
    /// copies paths and the selected text, which is why the throttle is asked *before* the state
    /// is built and not after.
    pub fn poll_mcp(&mut self) {
        if self.mcp.as_ref().is_some_and(crate::mcp::Session::due_for_state) {
            let state = self.mcp_state();
            if let Some(session) = self.mcp.as_mut() {
                session.publish(state);
            }
        }
        let requests = match self.mcp.as_mut() {
            Some(session) if session.due_for_requests() => session.take_requests(),
            _ => Vec::new(),
        };
        for request in requests {
            self.apply_mcp_request(request);
        }
        // Asked every frame and not only after a request arrived: a question that could not be
        // put up because a box was open has to go up when the box closes, and nothing else in the
        // program knows to tell it so.
        self.offer_next_agent_edit();
    }

    /// Everything published about this editor, as of now.
    ///
    /// Every path goes out resolved. A project opened as `.` holds its buffers as `./src/main.rs`,
    /// and an agent handed that would resolve it against its own working directory — which is
    /// where it happens to have been started, not where the editor is. The language server client
    /// keeps its own translation table for exactly this reason.
    fn mcp_state(&self) -> crate::mcp::State {
        let open_files: Vec<String> =
            self.editors.iter().filter_map(|e| e.path.as_deref()).map(|p| self.mcp_path(p)).collect();
        // The same paths, formatted the same way, in the same order — a subset an agent can
        // compare against the list above rather than a second list it has to reconcile with it.
        let dirty_files = self
            .editors
            .iter()
            .filter(|e| e.dirty)
            .filter_map(|e| e.path.as_deref())
            .map(|p| self.mcp_path(p))
            .collect();
        let editor = self.editor();
        let active = editor.path.as_deref().map(|path| crate::mcp::Active {
            path: self.mcp_path(path),
            // 1-based on the wire, because the other end thinks in the `path:line` a compiler
            // prints and the editor counts from zero internally.
            line: editor.cursor_line + 1,
            column: editor.cursor_col + 1,
            selection: crate::mcp::selection_for(editor.selected_text()),
        });
        let diagnostics = self
            .diagnostics
            .iter()
            .flat_map(|(path, marks)| {
                // Resolved once per file rather than once per mark: a file mid-refactor can carry
                // hundreds, and they all live at the same path.
                let path = self.mcp_path(path);
                marks.iter().map(move |mark| crate::mcp::Diagnostic {
                    path: path.clone(),
                    line: mark.line + 1,
                    severity: mark.severity.word().to_string(),
                    message: mark.message.clone(),
                })
            })
            .collect();
        crate::mcp::State {
            root: self.mcp_path(&self.root),
            open_files,
            dirty_files,
            active,
            diagnostics: crate::mcp::tidy_diagnostics(diagnostics),
        }
    }

    /// A path as an agent should see it: absolute, and with the links resolved when the file is
    /// really there. A path that resolves to nothing — a buffer whose file has been deleted —
    /// still goes out absolute rather than being dropped.
    fn mcp_path(&self, path: &Path) -> String {
        let absolute =
            if path.is_absolute() { path.to_path_buf() } else { self.root.join(path) };
        std::fs::canonicalize(&absolute).unwrap_or(absolute).to_string_lossy().into_owned()
    }

    /// Carries out one request from the MCP server.
    ///
    /// Three of the four happen the moment they are read, because none of them can lose anybody
    /// any work: showing a file, rendering one, and putting a sentence on the status line. The
    /// fourth writes into a buffer with unsaved changes in it, so it goes to
    /// [`Self::ask_or_apply_agent_edit`] and may end up as a question rather than as an edit.
    ///
    /// An action a version does not recognise never arrives here at all: `take_requests` cannot
    /// parse it and deletes the file, which is the behaviour a new tool talking to an old editor
    /// needs — nothing, rather than a guess.
    fn apply_mcp_request(&mut self, request: crate::mcp::Request) {
        let lang = self.settings.lang;
        match request {
            crate::mcp::Request::Open { path, line, end_line } => {
                let path = self.mcp_resolve(&path);
                self.show_beside_without_focus(path.clone(), line, end_line);
                if path.is_file() {
                    self.status_message = i18n::msg_agent_opened(lang, &self.mcp_short(&path), line);
                }
            }
            crate::mcp::Request::Preview { path } => {
                let path = self.mcp_resolve(&path);
                if self.preview_beside_without_focus(path.clone()) {
                    self.status_message = i18n::msg_agent_previewed(lang, &self.mcp_short(&path));
                }
            }
            crate::mcp::Request::Say { text } => {
                // Sanitised again, having already been cut by the server: this is a file on disk
                // that anything could have written, and what it says goes straight into a real
                // terminal's status bar.
                let text = crate::mcp::say_line(&text);
                if !text.is_empty() {
                    self.status_message = i18n::msg_agent_says(lang, &text);
                    self.mark_dirty();
                }
            }
            crate::mcp::Request::Edit { id, path, old, new } => {
                let edit = PendingAgentEdit { id, path: self.mcp_resolve(&path), old, new };
                self.ask_or_apply_agent_edit(edit);
            }
        }
    }

    /// A path as an agent named it, as a path this editor can use.
    ///
    /// A relative one means "in this project", which is the only root the agent was told about —
    /// see the `root` field of the published state.
    fn mcp_resolve(&self, path: &str) -> PathBuf {
        let path = PathBuf::from(path);
        if path.is_absolute() { path } else { self.root.join(path) }
    }

    /// How a file an agent touched is named on the status line: relative to the project when it
    /// is inside it, and whole when it is not. A status line is one line, and forty characters of
    /// `/Users/…/target/debug` in front of the name is the part nobody is reading.
    fn mcp_short(&self, path: &Path) -> String {
        path.strip_prefix(&self.root).unwrap_or(path).to_string_lossy().into_owned()
    }

    /// Renders a file in the pane the user is *not* typing in, and gives the keyboard back.
    ///
    /// `false` when there was nothing to show, so the caller can stay quiet: as with
    /// [`Self::show_beside_without_focus`], a path an agent guessed wrong is not worth a line in
    /// the middle of somebody else's work.
    ///
    /// A file the preview pane has nothing to say about falls back to simply opening it. Showing
    /// somebody the source of the thing they were promised beats showing them nothing, and the
    /// agent asked for this file to be in front of the user either way.
    fn preview_beside_without_focus(&mut self, path: PathBuf) -> bool {
        if !path.is_file() {
            return false;
        }
        let ext = file_ext(&path);
        let picture = crate::preview::is_previewable(&ext) || crate::preview::is_document(&ext);
        if !picture && !crate::preview::is_renderable(&ext) {
            self.show_beside_without_focus(path, None, None);
            return true;
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
        let rendered = self
            .editors
            .iter()
            .position(|e| e.preview.is_some() && e.path.as_deref() == Some(path.as_path()));
        if picture {
            self.open_preview_tab(path, crate::preview::is_document(&ext));
        } else if let Some(idx) = rendered {
            // Asking twice for the same file is an agent pointing at it again, not a request for
            // a second copy of the tab.
            self.focus_existing_tab(idx);
        } else {
            // A rendered markdown tab is a *view of a buffer*, not a second copy of the file, so
            // the source has to be open for anything to appear in it — see
            // `refresh_rendered_previews`, which looks the source up among the open editors. Both
            // land in the pane beside the user's work, the preview in front of the source.
            //
            // The pane is remembered before the source is opened, because opening a file that
            // already has a tab goes to *that* tab wherever it is, and takes `editor_pane_focus`
            // with it.
            let beside = self.editor_pane_focus;
            self.open_file_in_tab(path.clone());
            self.place_rendered_preview(path, beside);
        }
        // Everything about where the keyboard was, put back.
        self.focus = was.0;
        self.editor_pane_focus = was.1;
        if self.split_view {
            match was.1 {
                EditorPane::Left => self.active_editor = was.2,
                EditorPane::Right => self.active_editor_right = was.3,
            }
        }
        self.mark_dirty();
        true
    }

    /// Opens a file in the pane that is *not* being typed in, and gives the keyboard straight
    /// back — the rule the figures established in 0.9: show without taking.
    ///
    /// A file that is not there is passed over in silence. The request came from a program, not
    /// from a key somebody pressed, and an error banner for a path an agent guessed wrong is
    /// noise in the middle of somebody else's work.
    ///
    /// With `end_line` the span is *selected* rather than merely scrolled to, which is what makes
    /// it visible in a pane nobody is focused on: the renderer paints a selection wherever it
    /// finds one, focused or not, so an agent saying "these twelve lines" lands as twelve
    /// highlighted lines the user did not have to press anything to see. Selection semantics also
    /// give the other half of the rule for nothing — the moment the user clicks or types in that
    /// pane, the mark is gone.
    fn show_beside_without_focus(
        &mut self,
        path: PathBuf,
        line: Option<usize>,
        end_line: Option<usize>,
    ) {
        if !path.is_file() {
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
        self.open_file_in_tab(path);
        if let Some(line) = line {
            let idx = self.pane_editor_index(self.editor_pane_focus);
            if let Some(editor) = self.editors.get_mut(idx) {
                editor.goto_line(line);
                if let Some(end) = end_line {
                    let (from, to) = agent_span(editor, line, end);
                    // Backwards on purpose. `select_char_range` leaves the cursor at its second
                    // argument, and the cursor is what the renderer scrolls the pane to: named
                    // the other way round, a range longer than the pane would arrive showing its
                    // last line, which is not where anybody starts reading.
                    editor.select_char_range(to, from);
                }
            }
        }
        // Everything about where the keyboard was, put back.
        self.focus = was.0;
        self.editor_pane_focus = was.1;
        if self.split_view {
            match was.1 {
                EditorPane::Left => self.active_editor = was.2,
                EditorPane::Right => self.active_editor_right = was.3,
            }
        }
        // Nothing here came from an event, so nothing has marked the screen out of date.
        self.mark_dirty();
    }

    /// What happens to an `edit_buffer` before anything is written: the setting decides, and where
    /// the setting says "ask", the user does.
    ///
    /// Whatever the outcome, the server is answered. A tool call is blocked on the other side of
    /// this, and the one thing worse than refusing an agent's edit is leaving it holding a call
    /// open for two minutes to find out that it was refused.
    fn ask_or_apply_agent_edit(&mut self, edit: PendingAgentEdit) {
        match crate::settings::AgentEdits::of(&self.settings.agent_edits) {
            crate::settings::AgentEdits::Deny => {
                self.refuse_agent_edit(&edit, crate::mcp::edit_refused_by_setting());
            }
            crate::settings::AgentEdits::Allow => self.carry_out_agent_edit(edit),
            // Answered once, for the rest of the session. See `agent_edits_this_session`.
            crate::settings::AgentEdits::Ask if self.agent_edits_this_session => {
                self.carry_out_agent_edit(edit)
            }
            crate::settings::AgentEdits::Ask => {
                if self.agent_edit_queue.len() >= AGENT_EDIT_QUEUE {
                    let waiting = self.agent_edit_queue.len();
                    self.refuse_agent_edit(&edit, crate::mcp::edit_too_many(waiting));
                    return;
                }
                // The question itself goes up in `offer_next_agent_edit`, once the frame loop has
                // established that nothing else is holding the keyboard.
                self.agent_edit_queue.push_back(edit);
            }
        }
    }

    /// Puts the next waiting consent question on the status line, when there is room for it.
    ///
    /// Called once a frame from [`Self::poll_mcp`] rather than at the moment the request arrives,
    /// because "nothing else owns the keyboard" is a fact about *now*: an agent that asks while
    /// the user is in the find box gets its question the moment they close it, rather than never
    /// or on top of it.
    fn offer_next_agent_edit(&mut self) {
        if self.agent_edit_queue.is_empty() || self.a_modal_owns_the_keyboard() {
            return;
        }
        let Some(edit) = self.agent_edit_queue.pop_front() else { return };
        let (added, removed) = agent_edit_size(&edit.old, &edit.new);
        self.status_message = i18n::msg_agent_edit_confirm(
            self.settings.lang,
            &self.mcp_short(&edit.path),
            added,
            removed,
        );
        self.agent_edit_ask = Some(edit);
        self.mark_dirty();
    }

    /// The three answers to that question, in the language it was asked in.
    ///
    /// `A` is the one that is not a yes or a no: it says yes to this edit and to every other one
    /// this session, which is the shape of the promise somebody makes when they decide to watch an
    /// agent work rather than to vet each keystroke of it. Everything else is no, including the
    /// keys that would do something in the pane underneath — which is the whole point of asking
    /// before anything is written.
    fn handle_agent_edit_prompt_key(&mut self, key: KeyEvent) {
        let lang = self.settings.lang;
        let Some(edit) = self.agent_edit_ask.take() else { return };
        let yes = key.code == KeyCode::Char(i18n::yes_key(lang))
            || key.code == KeyCode::Char(i18n::yes_key(lang).to_ascii_uppercase());
        let always = matches!(key.code, KeyCode::Char('a') | KeyCode::Char('A'));
        if always {
            self.agent_edits_this_session = true;
        }
        if yes || always {
            self.carry_out_agent_edit(edit);
            // "Stop asking me" has to mean the ones already queued too. Asking three more times
            // straight after being told not to would read as the key not having worked.
            if always {
                while let Some(waiting) = self.agent_edit_queue.pop_front() {
                    self.carry_out_agent_edit(waiting);
                }
            }
        } else {
            let said = i18n::msg_agent_edit_declined(lang, &self.mcp_short(&edit.path));
            self.refuse_agent_edit(&edit, crate::mcp::edit_declined());
            self.status_message = said;
        }
        self.mark_dirty();
    }

    /// Applies an edit and tells the server how it went.
    fn carry_out_agent_edit(&mut self, edit: PendingAgentEdit) {
        let reply = self.apply_agent_edit(&edit);
        self.answer_agent_edit(reply);
    }

    /// Says no to an edit, in a sentence the agent can relay to the person who asked for it.
    fn refuse_agent_edit(&mut self, edit: &PendingAgentEdit, message: String) {
        self.answer_agent_edit(crate::mcp::Reply { id: edit.id, ok: false, message });
    }

    /// Writes the answer into the session directory, where the tool call is waiting for it.
    ///
    /// Silent without a session, which is a state this cannot really be in — the request came
    /// through one — but is the honest thing to do with an `Option` rather than unwrapping it on a
    /// path that runs in the frame loop.
    fn answer_agent_edit(&mut self, reply: crate::mcp::Reply) {
        if let Some(session) = self.mcp.as_ref() {
            session.reply(&reply);
        }
    }

    /// Carries out one agreed edit, on the buffer and not on the file.
    ///
    /// Every refusal here is a sentence rather than a code, and each says what to do next: the
    /// four ways this can fail are four different mistakes, and an agent told only "no" would
    /// retry the one that will never work.
    ///
    /// One [`Editor::replace_char_range`], so it is one step of undo — the same discipline the
    /// rename and the project sweep follow, and the reason a change somebody regrets agreeing to
    /// costs them one Ctrl+Z rather than a hunt through the file. The buffer is left *modified*
    /// and unsaved on purpose: saving is the user's, and an agent that could write to disk through
    /// this door would have made the consent question meaningless.
    fn apply_agent_edit(&mut self, edit: &PendingAgentEdit) -> crate::mcp::Reply {
        let lang = self.settings.lang;
        let short = self.mcp_short(&edit.path);
        let refuse = |message: String| crate::mcp::Reply { id: edit.id, ok: false, message };
        // Compared as resolved paths, the way they were published: an agent that read
        // `open_files` is holding the canonical name, and a buffer opened as `./src/main.rs` is
        // the same file under a different spelling.
        let wanted = self.mcp_path(&edit.path);
        let found = self.editors.iter().position(|e| {
            e.preview.is_none() && e.path.as_deref().is_some_and(|p| self.mcp_path(p) == wanted)
        });
        let Some(idx) = found else {
            return refuse(crate::mcp::edit_not_open(&short));
        };
        if self.editors[idx].is_read_only() {
            return refuse(crate::mcp::edit_read_only(&short));
        }
        let text = self.editors[idx].rope.to_string();
        let (start, end) = match only_match(&text, &edit.old) {
            Ok(span) => span,
            Err(0) => return refuse(crate::mcp::edit_no_match(&short)),
            Err(n) => return refuse(crate::mcp::edit_many_matches(&short, n)),
        };
        let editor = &mut self.editors[idx];
        editor.replace_char_range(start, end, &edit.new);
        // Marked the way an `open_file` range is, and for the same reason: the pane holding this
        // buffer may not be the one the user is typing in, and a change they cannot see is a
        // change that happened behind their back even though they agreed to it. Backwards for the
        // same reason as there — the cursor is what the pane scrolls to, and the line worth
        // scrolling to is the one the change begins on.
        editor.select_char_range(start + edit.new.chars().count(), start);
        let line = editor.rope.char_to_line(start) + 1;
        self.status_message = i18n::msg_agent_edited(lang, &short, line);
        self.mark_dirty();
        crate::mcp::Reply { id: edit.id, ok: true, message: crate::mcp::edit_applied(&short, line) }
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
        let now = Instant::now();
        let wanted: Vec<Vec<String>> = self
            .editors
            .iter()
            .filter(|e| e.preview.is_none())
            .filter_map(|e| e.path.as_deref())
            .filter_map(|path| self.lsp_argv_for(path))
            .filter(|argv| {
                argv.first().is_some_and(|program| {
                    !self.lsp.contains_key(program) && self.lsp_free_to_start(program, now)
                })
            })
            .collect();
        for argv in wanted {
            let program = argv[0].clone();
            if self.lsp.contains_key(&program) || !self.lsp_free_to_start(&program, now) {
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
                    self.lsp_error.insert(
                        program,
                        LspTrouble { detail, deaths: 0, when: now, worth_retrying: false },
                    );
                }
            }
        }
    }

    /// Whether a program that is not running may be started now.
    ///
    /// The record of what went wrong is kept even while a replacement is running, which is what
    /// makes the count a count *per session* rather than per life: a server that dies, restarts
    /// and dies again has died twice, and forgetting the first death on the restart would let it
    /// flap for as long as the editor is open.
    fn lsp_free_to_start(&self, program: &str, now: Instant) -> bool {
        match self.lsp_error.get(program) {
            None => true,
            Some(trouble) => trouble.may_start_again(now),
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
        // Anything a server says lands on screen: a squiggle, a hover, a line in the status bar.
        if !arrived.is_empty() {
            self.redraw = true;
        }
        for (program, event) in arrived {
            match event {
                crate::lsp::Event::Ready { utf16, offered } => {
                    let Some(client) = self.lsp.get_mut(&program) else { continue };
                    client.confirm_ready(utf16, offered);
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
                        self.lsp_raw.remove(&path);
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
                    self.diagnostics.insert(path.clone(), marks);
                    // And the same list unconverted, for the one question that needs a diagnostic
                    // rather than a squiggle — see [`Self::lsp_raw`]. Written here, beside the
                    // marks, so neither can outlive the other.
                    self.lsp_raw.insert(path, raw);
                }
                crate::lsp::Event::Completion { id, words } => {
                    self.absorb_lsp_completion(id, words);
                }
                crate::lsp::Event::Definition { id, target } => self.lsp_go_there(id, target),
                crate::lsp::Event::References { id, targets } => {
                    self.lsp_list_references(id, targets)
                }
                crate::lsp::Event::Symbols { id, symbols } => self.lsp_list_symbols(id, symbols),
                crate::lsp::Event::Rename { id, plan } => self.lsp_rename_answer(id, plan),
                crate::lsp::Event::Formatting { id, edits } => self.lsp_format_answer(id, edits),
                crate::lsp::Event::CodeActions { id, actions } => {
                    self.lsp_offer_code_actions(id, actions)
                }
                crate::lsp::Event::CodeActionEdit { id, plan } => {
                    self.lsp_code_action_edit(id, plan)
                }
                crate::lsp::Event::SelectionRange { id, chain } => {
                    self.lsp_widen_selection(id, chain)
                }
                crate::lsp::Event::FoldingRanges { id, ranges } => {
                    self.lsp_remember_folds(id, ranges)
                }
                crate::lsp::Event::Hover { id, text } => self.lsp_show_what_it_is(id, text),
                crate::lsp::Event::Answer { message } => {
                    // Straight back to the server that asked, on the thread that owns the pipe.
                    if let Some(client) = self.lsp.get_mut(&program) {
                        client.answer(message);
                    }
                }
                crate::lsp::Event::Stopped { detail } => {
                    // Only this one. Another server that is still running goes on underlining
                    // its own files, and the marks that came from the one that died are the only
                    // ones that have to go.
                    self.lsp.remove(&program);
                    // Counted, and timed. A server that has run and died may have hit something
                    // once — rust-analyzer running out of memory on a big project is the usual
                    // one — and the session that loses diagnostics for good over it is the whole
                    // rest of the afternoon. So it is started again after a pause, a couple of
                    // times, and then left alone: a program that dies every time it runs is not
                    // having a bad moment, and starting it forever would be worse than nothing.
                    let record = self.lsp_error.entry(program.clone()).or_insert(LspTrouble {
                        detail: String::new(),
                        deaths: 0,
                        when: Instant::now(),
                        worth_retrying: true,
                    });
                    record.detail = detail;
                    record.deaths += 1;
                    record.when = Instant::now();
                    record.worth_retrying = true;
                    self.lsp_completion = None;
                    self.lsp_asked = None;
                    self.lsp_listing = None;
                    // The preview, if one is up, is not cleared with it: it holds offsets into
                    // buffers this editor owns and needs nothing from the server to apply them.
                    self.lsp_editing = None;
                    self.lsp_formatting = None;
                    self.lsp_acting = None;
                    self.lsp_action_edit = None;
                    self.lsp_widening = None;
                    self.lsp_folding.clear();
                    // The walk goes with them, and so do the fold boundaries: both are the dead
                    // server's reading of files it is no longer reading. The selection on screen
                    // stays exactly as it is — it is text, and nothing about it stopped being true.
                    self.selection_walk = None;
                    // The files it was serving are forgotten too, so the ones still open are
                    // announced again from scratch to whatever takes its place.
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
            self.lsp_raw.remove(&path);
            // Asked again from scratch by whatever takes its place, which is what forgetting the
            // revision means here. The boundaries already handed to a buffer are left where they
            // are: they describe text nobody has touched, and dropping them would collapse the
            // fold markers of every open file the moment a server hiccuped.
            self.lsp_folds_asked.remove(&path);
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
    ///
    /// `trigger` says a character asked rather than a popup, and it is passed on twice: to the
    /// server, which answers a dot differently from a bare position, and to the slot, so the
    /// reply knows it is allowed to open a list of its own. Every guard below is the same in both
    /// cases, and the quiet one matters most here: a file with no server configured returns
    /// without a word. A `.` is typed a hundred times an hour, and a message about it — even a
    /// helpful one — would be the editor talking over the typing.
    fn lsp_ask_completion(&mut self, editor_index: usize, start: usize, trigger: Option<char>) {
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
        // Not before the handshake is finished: the file and the question would both be dropped
        // at the far end, and the revision written down here as sent. The popup already has the
        // words from the buffer, so this costs nothing anyone can see.
        if !client.ready() {
            return;
        }
        client.did_change(&absolute, &text);
        let asked = client.completion(&absolute, line, &line_text, col, trigger);
        // Recorded as sent, so the debounce does not turn round and send the same revision again.
        self.lsp_sent.insert(path, revision);
        self.lsp_completion = asked.map(|id| PendingCompletion {
            id,
            editor: editor_index,
            start,
            triggered: trigger.is_some(),
        });
    }

    /// Folds a server's answer into the popup that asked for it, opens one on it, or drops it.
    ///
    /// Three ways it is dropped, and none of them is an error: it answers a question we are no
    /// longer waiting for, the popup has closed or moved on, or the server had nothing to say.
    /// The popup carries on with the words from the buffer in every one of those cases, which is
    /// the property worth protecting — the list was never waiting on this.
    ///
    /// A trigger character's answer is the one that has no popup to fold into: it *is* the popup,
    /// and see [`Self::open_triggered_completion`] for why it waits until here to become one.
    fn absorb_lsp_completion(&mut self, id: i64, words: Vec<String>) {
        let Some(pending) = self.lsp_completion.as_ref().filter(|p| p.id == id) else { return };
        let (editor, start, triggered) = (pending.editor, pending.start, pending.triggered);
        self.lsp_completion = None;
        if words.is_empty() {
            return;
        }
        if triggered {
            self.open_triggered_completion(editor, start, words);
            return;
        }
        if !self.completion_live() {
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

    /// Puts up the popup a trigger character asked for, now that there is something to put in it.
    ///
    /// Nothing opened when the `.` was typed, on purpose. The buffer's words are not a stand-in
    /// for a member list — they are the wrong answer to a different question — so a list that
    /// flashed up full of them and rearranged itself two frames later would be worse than the one
    /// list that appears once, already right. The cost of waiting is that this can arrive into an
    /// editor that has moved on, so the anchor is checked here exactly as [`Self::completion_live`]
    /// checks it afterwards: same buffer, and everything typed since the dot still a word.
    ///
    /// A popup that opened in the meantime is left alone. Between the dot and the answer the user
    /// can have typed two more letters and brought up the ordinary list; that one is on screen,
    /// has been filtered against what is under the cursor, and replacing it under a finger that
    /// may already be on the arrows is the one thing this whole file is arranged to avoid. Its
    /// own request went out when it opened, so the server's names are not lost either.
    fn open_triggered_completion(&mut self, editor: usize, start: usize, words: Vec<String>) {
        if self.completion.is_some() || self.focus != Focus::Editor {
            return;
        }
        let idx = self.active_editor_index();
        if idx != editor {
            return;
        }
        let Some(ed) = self.editors.get(idx) else { return };
        let Some(prefix) =
            crate::complete::prefix_from(&ed.rope, start, ed.cursor_line, ed.cursor_col)
        else {
            return;
        };
        self.completion = crate::complete::Popup::from_trigger(
            idx,
            start,
            prefix,
            crate::complete::lsp_candidates(&words),
        );
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
            let Some(client) = self.lsp_client_for(&held) else { continue };
            // Nothing goes out before the handshake is finished. A server is entitled to ignore
            // everything sent before `initialized` and the good ones do — silently — so a
            // `didOpen` that raced a slow `initialize` would be dropped at the far end while
            // this side wrote the revision down as sent, and the file would sit there without a
            // single diagnostic until somebody typed in it.
            //
            // Nothing is queued: the revision is simply not recorded, so the next frame finds
            // the same file still unsent and offers it again. Waiting costs a frame, and the
            // alternative — a queue of things to say once the server answers — is a second
            // record of what is open, kept in step with this one by hand.
            if !client.ready() {
                continue;
            }
            client.did_open(&absolute, &text);
            self.lsp_sent.insert(held, revision);
        }
        for path in gone {
            if let Some((absolute, _)) =
                self.lsp_paths.iter().find(|(_, held)| held.as_path() == path.as_path())
            {
                let absolute = absolute.clone();
                if let Some(client) = self.lsp_client_for(&path).filter(|c| c.ready()) {
                    client.did_close(&absolute);
                }
                self.lsp_paths.remove(&absolute);
            }
            self.lsp_sent.remove(&path);
            self.lsp_seen.remove(&path);
            self.diagnostics.remove(&path);
            self.lsp_folds_asked.remove(&path);
        }
        // Last, because it only asks about files that have just been announced above.
        self.lsp_refresh_folds();
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
        // A server whose handshake has not finished cannot answer this, and the same thing is
        // true of it as of a file no server serves: there is nothing here to ask. It is a window
        // of a second at most, and pressing the key again once the server has said hello works.
        let Some(client) = self.lsp_client_for(&path).filter(|c| c.ready()) else {
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
            // UTF-8 is not "the same number": it is a byte offset into the line, and a definition
            // on a line with an accent or an emoji before it lands to the right of the name it
            // was pointing at — further right the more of them there are.
            crate::lsp::utf8_to_chars(&line_text, target.column)
        };
        // The protocol counts lines from zero and `goto_line` counts them the way a person does.
        self.editor_mut().goto_line(target.line + 1);
        // The jump may have landed nowhere: opening the target file can fail and leave no tab
        // at all, and then there is no buffer under this index to place a cursor in.
        let index = self.active_editor_index();
        let Some(editor) = self.editors.get_mut(index) else { return };
        let len = editor.line_char_len(editor.cursor_line);
        editor.cursor_col = column.min(len);
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

    /// Asks the server everywhere the thing under the cursor is used.
    ///
    /// The same opening as [`Self::lsp_go_to_definition`], down to sending the file first: the
    /// question is about *this* text, and a list of places in the text of four hundred
    /// milliseconds ago is a list of lines that have since moved.
    pub fn lsp_find_references(&mut self) {
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
        let Some(client) = self.lsp_client_for(&path).filter(|c| c.ready()) else {
            self.status_message = i18n::msg_lsp_none_here(lang).to_string();
            return;
        };
        client.did_change(&absolute, &text);
        match client.references(&absolute, line, &line_text, col) {
            Some(id) => {
                self.lsp_listing = Some(PendingAsk { id, from });
                self.status_message = i18n::msg_lsp_looking_references(lang).to_string();
            }
            None => self.status_message = i18n::msg_lsp_none_here(lang).to_string(),
        }
    }

    /// Turns the uses the server named into a list to choose from.
    fn lsp_list_references(&mut self, id: i64, targets: Vec<crate::lsp::Jump>) {
        let lang = self.settings.lang;
        let Some(asked) = self.lsp_listing.take().filter(|a| a.id == id) else { return };
        if targets.is_empty() {
            self.status_message = i18n::msg_lsp_no_references(lang).to_string();
            return;
        }
        // Back to the spelling the tabs use, for the same reason the jump does it: the server
        // answers in resolved paths, and a row that opened a second tab on a file already open
        // under another name would leave one file with two buffers.
        let mut targets: Vec<crate::lsp::Jump> = targets
            .into_iter()
            .map(|mut target| {
                target.path = self.lsp_paths.get(&target.path).cloned().unwrap_or(target.path);
                target
            })
            .collect();
        // By file and then by line. Servers answer in whatever order they indexed in, which is
        // not an order anybody can read — and grouping the rows by file is also what lets each
        // file be read off disk once instead of once per row.
        targets.sort_by(|a, b| {
            a.path.cmp(&b.path).then(a.line.cmp(&b.line)).then(a.column.cmp(&b.column))
        });
        let root = self.root.clone();
        let mut items = Vec::with_capacity(targets.len());
        let mut held: Option<(PathBuf, Vec<String>)> = None;
        for target in &targets {
            if held.as_ref().is_none_or(|(path, _)| path != &target.path) {
                held = Some((target.path.clone(), self.file_lines(&target.path)));
            }
            let text = held.as_ref().and_then(|(_, lines)| lines.get(target.line));
            let column = self.lsp_chars_for(
                &target.path,
                text.map(String::as_str).unwrap_or_default(),
                target.column,
            );
            items.push(crate::picker::PickItem {
                label: located_label(&root, &target.path, target.line + 1, text.map(String::as_str)),
                shortcut: None,
                action: crate::picker::PickAction::FileLine(
                    target.path.clone(),
                    target.line + 1,
                    column,
                ),
            });
        }
        self.open_server_list(Key::PickerReferences, crate::picker::PickerKind::References, items, asked.from);
    }

    /// Asks the server what names the file holds.
    ///
    /// The file goes first here too. An outline of the text as it was before the last few
    /// keystrokes is an outline whose every line number is off by however many lines have been
    /// typed since — which is the one thing this list is for.
    pub fn lsp_document_symbols(&mut self) {
        let lang = self.settings.lang;
        let index = self.active_editor_index();
        let Some(editor) = self.editors.get(index) else { return };
        let Some(path) = editor.path.clone() else { return };
        let from = (path.clone(), editor.cursor_line, editor.cursor_col);
        let text = editor.rope.to_string();
        let Some(absolute) = Self::lsp_absolute_for(&self.lsp_paths, &path) else {
            self.status_message = i18n::msg_lsp_needs_saving(lang).to_string();
            return;
        };
        self.lsp_paths.insert(absolute.clone(), path.clone());
        let Some(client) = self.lsp_client_for(&path).filter(|c| c.ready()) else {
            self.status_message = i18n::msg_lsp_none_here(lang).to_string();
            return;
        };
        client.did_change(&absolute, &text);
        match client.document_symbols(&absolute) {
            Some(id) => {
                self.lsp_listing = Some(PendingAsk { id, from });
                self.status_message = i18n::msg_lsp_looking_symbols(lang).to_string();
            }
            None => self.status_message = i18n::msg_lsp_none_here(lang).to_string(),
        }
    }

    /// Turns the file's names into a list to choose from.
    ///
    /// In document order, untouched. An outline is a picture of the file: sorted by name it
    /// would be a picture of a different one, and the reason anybody opens this is to find the
    /// function they know is *below* the one they are looking at.
    fn lsp_list_symbols(&mut self, id: i64, symbols: Vec<crate::lsp::SymbolRow>) {
        let lang = self.settings.lang;
        let Some(asked) = self.lsp_listing.take().filter(|a| a.id == id) else { return };
        if symbols.is_empty() {
            self.status_message = i18n::msg_lsp_no_symbols(lang).to_string();
            return;
        }
        // One file, so one read: the rows are all places in the buffer the question was asked
        // about, and the columns are all measured against its lines.
        let path = asked.from.0.clone();
        let lines = self.file_lines(&path);
        let items = symbols
            .iter()
            .map(|row| crate::picker::PickItem {
                // Two spaces a level. Enough to see the shape of the file at a glance, and
                // little enough that a name four levels in still has room for itself.
                label: format!("{}{}", "  ".repeat(row.depth), row.name),
                // What kind of thing it is, in the right-hand column the palette uses for
                // chords: it is the same job — the part of the row you read second.
                shortcut: Some(row.kind.to_string()),
                action: crate::picker::PickAction::FileLine(
                    path.clone(),
                    row.line + 1,
                    self.lsp_chars_for(
                        &path,
                        lines.get(row.line).map(String::as_str).unwrap_or_default(),
                        row.column,
                    ),
                ),
            })
            .collect();
        self.open_server_list(Key::PickerSymbols, crate::picker::PickerKind::Symbols, items, asked.from);
    }

    // ---- Renaming a name ---------------------------------------------------------------------
    //
    // The first thing a language server is asked that *writes*, and everything below is the
    // discipline that buys: nothing happens without a preview of exactly what would change and
    // where, each buffer takes one step of undo, and an answer this cannot carry out honestly is
    // refused whole rather than applied in part.
    //
    // Open buffers only, edited through the rope. Not a scruple — a limit with a reason. Writing
    // a file that has a tab open would be undone by the frame loop within the second: the sweep
    // in `Editor::check_external_changes` reloads a clean buffer that changed on disk and clears
    // its undo stack outright, so the rename would land and the one keystroke that could take it
    // back would be gone. A *dirty* tab is worse still: it keeps its text, the disk keeps the
    // rename, and the two diverge in silence. Editing the rope has neither problem, and files
    // with no tab are the honest refusal below rather than a special case here.

    /// Opens the box that asks what to call the name under the cursor instead.
    pub fn lsp_rename_symbol(&mut self) {
        let lang = self.settings.lang;
        let index = self.active_editor_index();
        let Some(editor) = self.editors.get(index) else { return };
        let Some(path) = editor.path.clone() else { return };
        // Asked before the box rather than after it, so a file no server serves costs a keypress
        // and not a name typed for nothing. Whether the server is *ready* is asked later, at the
        // moment the question actually goes out.
        if self.lsp_argv_for(&path).is_none() {
            self.status_message = i18n::msg_lsp_none_here(lang).to_string();
            return;
        }
        // The whole identifier, not the part before the caret: what is being renamed is a name,
        // and a box prefilled with half of one invites a rename of the other half.
        let Some((start, end)) = editor.word_at_cursor() else {
            self.status_message = i18n::msg_rename_nothing_here(lang).to_string();
            return;
        };
        let old_name = editor.rope.slice(start..end).to_string();
        let from = (path, editor.cursor_line, editor.cursor_col);
        self.symbol_rename = Some(SymbolRename { typed: old_name.clone(), old_name, from });
        self.status_message = String::new();
    }

    fn handle_symbol_rename_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.confirm_symbol_rename(),
            KeyCode::Esc => self.symbol_rename = None,
            KeyCode::Backspace => {
                if let Some(box_) = self.symbol_rename.as_mut() {
                    pop_grapheme(&mut box_.typed);
                }
            }
            KeyCode::Char(c) if is_a_typed_character(key) => {
                if let Some(box_) = self.symbol_rename.as_mut() {
                    box_.typed.push(c);
                }
            }
            _ => {}
        }
    }

    /// Sends the question, with the same opening as [`Self::lsp_find_references`] down to sending
    /// the file first: the answer is a set of *positions*, and positions in the text of four
    /// hundred milliseconds ago are positions in a file that has since moved under them.
    fn confirm_symbol_rename(&mut self) {
        let lang = self.settings.lang;
        let Some(asked) = self.symbol_rename.take() else { return };
        let new_name = asked.typed.trim().to_string();
        // Nothing typed, or the name it already has. Neither is worth a sentence: the box was
        // opened and closed, and the best possible answer to "rename foo to foo" is a preview of
        // nothing at all.
        if new_name.is_empty() || new_name == asked.old_name {
            self.status_message = String::new();
            return;
        }
        let (path, line, col) = asked.from.clone();
        let Some(editor) = self.editors.iter().find(|e| e.path.as_deref() == Some(path.as_path()))
        else {
            return;
        };
        let line_text = editor.rope.get_line(line).map(|l| l.to_string()).unwrap_or_default();
        let text = editor.rope.to_string();
        let Some(absolute) = Self::lsp_absolute_for(&self.lsp_paths, &path) else {
            self.status_message = i18n::msg_lsp_needs_saving(lang).to_string();
            return;
        };
        self.lsp_paths.insert(absolute.clone(), path.clone());
        let Some(client) = self.lsp_client_for(&path).filter(|c| c.ready()) else {
            self.status_message = i18n::msg_lsp_none_here(lang).to_string();
            return;
        };
        client.did_change(&absolute, &text);
        match client.rename(&absolute, line, &line_text, col, &new_name) {
            Some(id) => {
                self.status_message = i18n::msg_rename_asking(lang, &asked.old_name, &new_name);
                self.lsp_editing = Some(PendingRename {
                    id,
                    from: asked.from,
                    old_name: asked.old_name,
                    new_name,
                });
            }
            None => self.status_message = i18n::msg_lsp_none_here(lang).to_string(),
        }
    }

    /// Reads the server's answer, and either puts a preview on screen or says why it will not.
    fn lsp_rename_answer(&mut self, id: i64, plan: Result<crate::lsp::RenamePlan, String>) {
        let Some(asked) = self.lsp_editing.take().filter(|a| a.id == id) else { return };
        let plan = match plan {
            Ok(plan) => plan,
            // The server's own sentence, unwrapped and unexplained. It is the only party here
            // that knows why — "cannot rename this element" is rust-analyzer's answer about a
            // keyword, and dressing it up in an editor's words would lose the one useful word.
            Err(complaint) => {
                self.status_message = complaint;
                return;
            }
        };
        match self.edit_preview_from(&asked.from, &asked.old_name, &asked.new_name, plan) {
            Ok(preview) => {
                self.rename_preview = Some(preview);
                // The preview is the answer, so the line that said the question had gone out has
                // done its job — as with the lists, an "asking…" under an answer reads as one
                // still to come.
                self.status_message = String::new();
            }
            Err(refusal) => self.status_message = refusal,
        }
    }

    /// Turns a plan into a preview, or into the one sentence saying why there is not going to be
    /// one.
    ///
    /// Every `Err` here is a refusal of the *whole* thing, and the order they are asked in is the
    /// order of what the reader can do about them: a file operation is not something any amount of
    /// opening tabs would fix, a file with no tab is fixed by opening it, and the last two are
    /// about the edits themselves.
    ///
    /// Takes the three pieces of the question rather than the rename that asked it, because a
    /// rename is no longer the only thing that arrives here: a code action reaching more than one
    /// file comes down this same road, with the same refusals and the same box, and it has no old
    /// name to put in a title. That is what an empty `old_name` means — see [`RenamePreview`].
    fn edit_preview_from(
        &self,
        from: &(PathBuf, usize, usize),
        old_name: &str,
        new_name: &str,
        plan: crate::lsp::RenamePlan,
    ) -> Result<RenamePreview, String> {
        let lang = self.settings.lang;
        if plan.file_ops {
            return Err(i18n::msg_rename_refused_file_ops(lang).to_string());
        }
        // Back to the spelling the tabs use, for the same reason every other answer is mapped
        // back: the server answers in resolved paths — symlinks followed, `.` gone — and a file
        // matched under the wrong name is a file this would report as not open.
        let mut files: Vec<crate::lsp::FileEdits> = plan
            .files
            .into_iter()
            .map(|mut file| {
                file.path = self.lsp_paths.get(&file.path).cloned().unwrap_or(file.path);
                file
            })
            .filter(|file| !file.edits.is_empty())
            .collect();
        if files.is_empty() {
            return Err(i18n::msg_rename_no_changes(lang).to_string());
        }
        // By path, so the preview reads the same way twice running. The server answers in
        // whatever order it indexed in, which is not an order anybody can read.
        files.sort_by(|a, b| a.path.cmp(&b.path));
        // And one entry per file, whatever the answer did. A server may name a file twice — both
        // wire shapes in one reply, or two `documentChanges` entries for the same document — and
        // two entries for one buffer would be two rebuilds of the same span, the second measured
        // against text the first had already moved. Merged rather than refused: the edits are the
        // same edits, and it is only the bookkeeping below that cares how they arrived.
        files.dedup_by(|later, first| {
            let same = later.path == first.path;
            if same {
                let taken = std::mem::take(&mut later.edits);
                first.edits.extend(taken);
            }
            same
        });

        if files.iter().any(|f| f.edits.iter().any(crate::lsp::SpanEdit::spans_lines)) {
            return Err(i18n::msg_rename_refused_multiline(lang).to_string());
        }
        let held = |path: &Path| self.editors.iter().find(|e| e.path.as_deref() == Some(path));
        let outside = files.iter().filter(|f| held(&f.path).is_none()).count();
        if outside > 0 {
            return Err(i18n::msg_rename_refused_outside(lang, outside));
        }
        if files.iter().any(|f| held(&f.path).is_some_and(|e| e.read_only)) {
            return Err(i18n::msg_rename_refused_read_only(lang).to_string());
        }

        let mut targets = Vec::with_capacity(files.len());
        let mut rows = Vec::new();
        let mut total = 0usize;
        for file in &files {
            let Some(editor) = held(&file.path) else {
                return Err(i18n::msg_rename_refused_moved(lang).to_string());
            };
            let lines = self.file_lines(&file.path);
            let mut edits = Vec::with_capacity(file.edits.len());
            for edit in &file.edits {
                // A line the buffer does not have is the server describing text that is no longer
                // here. Refused rather than clamped, which is the difference between this and a
                // diagnostic: an underline in the wrong place is noise, a replacement in the
                // wrong place is damage.
                let Some(text) = lines.get(edit.start_line) else {
                    return Err(i18n::msg_rename_refused_moved(lang).to_string());
                };
                let start_col = self.lsp_chars_for(&file.path, text, edit.start_col);
                // Clamped forward rather than refused: a backwards span is a server being wrong
                // about a range, and read as a zero-width one it becomes an insertion — which the
                // preview then shows for what it is, character for character.
                let end_col = self.lsp_chars_for(&file.path, text, edit.end_col).max(start_col);
                let line_start = editor.rope.line_to_char(edit.start_line);
                edits.push(BufferEdit {
                    start: line_start + start_col,
                    end: line_start + end_col,
                    line: edit.start_line,
                    new_text: edit.new_text.clone(),
                });
            }
            edits.sort_by_key(|e| (e.start, e.end));
            // Two edits over the same characters would make the result depend on which was
            // applied first, and the span rebuild below has no order that is more right than the
            // other. Adjacent is fine — one ending exactly where the next begins is two names.
            if edits.windows(2).any(|pair| pair[1].start < pair[0].end) {
                return Err(i18n::msg_rename_refused_overlap(lang).to_string());
            }
            total += edits.len();
            rows.push(i18n::msg_preview_file_header(
                lang,
                &file.path.strip_prefix(&self.root).unwrap_or(&file.path).display().to_string(),
                edits.len(),
            ));
            rows.extend(preview_rows(editor, &lines, &edits));
            targets.push(RenameFile {
                path: file.path.clone(),
                revision: editor.revision(),
                edits,
            });
        }

        // Where the cursor is, as an offset, worked out here because here is where the text it is
        // an offset into still exists.
        let (path, line, col) = from.clone();
        let from_char = held(&path)
            .map(|e| e.rope.line_to_char(line.min(e.rope.len_lines().saturating_sub(1))) + col)
            .unwrap_or(0);
        Ok(RenamePreview {
            old_name: old_name.to_string(),
            new_name: new_name.to_string(),
            from: (path, line, col),
            from_char,
            files: targets,
            rows,
            edits: total,
            scroll: 0,
            body_rows: 1,
        })
    }

    fn handle_rename_preview_key(&mut self, key: KeyEvent) {
        let lang = self.settings.lang;
        let page = self.rename_preview.as_ref().map(|p| p.body_rows.max(1) as isize).unwrap_or(1);
        if let Some(preview) = self.rename_preview.as_mut() {
            match key.code {
                KeyCode::Up => return preview.scroll_by(-1),
                KeyCode::Down => return preview.scroll_by(1),
                KeyCode::PageUp => return preview.scroll_by(-page),
                KeyCode::PageDown => return preview.scroll_by(page),
                KeyCode::Home => {
                    preview.scroll = 0;
                    return;
                }
                KeyCode::End => return preview.scroll_by(preview.rows.len() as isize),
                _ => {}
            }
        }
        // Enter counts as yes here, unlike the delete confirmation, and the difference is that
        // this one can be taken back: one Ctrl+Z per buffer puts every one of these edits away
        // again. The letter is the localized one, so the key that means yes is the key the footer
        // prints. Everything else is no — including Esc, which is how it is spelled.
        match key.code {
            KeyCode::Enter => self.apply_rename_preview(),
            KeyCode::Char(c) if c.eq_ignore_ascii_case(&i18n::yes_key(lang)) => {
                self.apply_rename_preview()
            }
            _ => {
                // Which of the two sentences depends on what the box was showing, and the empty
                // old name is what says so — see [`RenamePreview`].
                let renaming =
                    self.rename_preview.as_ref().is_some_and(|p| !p.old_name.is_empty());
                self.rename_preview = None;
                self.status_message = if renaming {
                    i18n::msg_rename_cancelled(lang).to_string()
                } else {
                    i18n::msg_edit_preview_cancelled(lang).to_string()
                };
            }
        }
    }

    /// Writes the preview into the buffers it was built against.
    ///
    /// One [`Editor::replace_char_range`] per file, which is one checkpoint and therefore one step
    /// of undo — the same shape as Replace All, and for the same reason: a rename is one action,
    /// so taking it back has to be one Ctrl+Z and not one per occurrence. The run from the first
    /// edit to the last is rebuilt in memory — replacements where the edits were, the text
    /// between them carried over verbatim — and written back in a single edit.
    fn apply_rename_preview(&mut self) {
        let lang = self.settings.lang;
        let Some(preview) = self.rename_preview.take() else { return };
        // Every offset below was measured against these buffers at the moment the preview was
        // built, and the sweep in the frame loop can reload a clean one out from under it without
        // anybody pressing anything. Checked for all of them before any of them is written, so a
        // rename that cannot be finished is not half done either.
        let moved = preview.files.iter().any(|target| {
            self.editors
                .iter()
                .find(|e| e.path.as_deref() == Some(target.path.as_path()))
                .is_none_or(|e| e.revision() != target.revision)
        });
        if moved {
            self.status_message = i18n::msg_rename_refused_moved(lang).to_string();
            return;
        }
        for target in &preview.files {
            let Some(index) =
                self.editors.iter().position(|e| e.path.as_deref() == Some(target.path.as_path()))
            else {
                continue;
            };
            let editor = &mut self.editors[index];
            let Some((span_start, span_end, rebuilt)) = edits_as_one_span(editor, &target.edits)
            else {
                continue;
            };
            editor.replace_char_range(span_start, span_end, &rebuilt);
        }
        self.restore_cursor_after_rename(&preview);
        // Nothing is told to the servers here. The revision each buffer just bumped is what the
        // debounced `didChange` in the frame loop watches, so they are resynchronized four
        // hundred milliseconds after the last of these lands — one message per file, not one per
        // edit, and by the same path as any other typing.
        self.status_message = i18n::msg_rename_applied(
            lang,
            &preview.old_name,
            &preview.new_name,
            preview.edits,
            preview.files.len(),
        );
    }

    /// Puts the cursor back where the key was pressed, allowing for the text that moved under it.
    ///
    /// Only the buffer the question was asked from: `replace_char_range` leaves the cursor at the
    /// end of what it wrote, which is fine everywhere else — nobody is looking at those tabs, and
    /// the first thing that happens when they are looked at is that the cursor is put somewhere
    /// by hand anyway.
    ///
    /// The adjustment counts only the edits that end at or before where the cursor was, on its
    /// own line. An edit *containing* the cursor is not one of them: renaming `value` to `count`
    /// with the caret three characters in leaves it three characters in, which is where the eye
    /// is and where the next keystroke belongs.
    fn restore_cursor_after_rename(&mut self, preview: &RenamePreview) {
        let (path, line, col) = &preview.from;
        let Some(target) = preview.files.iter().find(|t| &t.path == path) else { return };
        self.restore_cursor_after_edits(path, *line, *col, preview.from_char, &target.edits);
    }

    /// The arithmetic of the two paragraphs above, given the one buffer's edits.
    ///
    /// Shared with the sweep across the project, which has the same problem for the same reason:
    /// `replace_char_range` leaves the cursor at the end of what it wrote, and the one buffer
    /// somebody is actually looking at is the one where that reads as the screen jumping.
    fn restore_cursor_after_edits(
        &mut self,
        path: &Path,
        line: usize,
        col: usize,
        from_char: usize,
        edits: &[BufferEdit],
    ) {
        let delta: isize = edits
            .iter()
            .filter(|e| e.line == line && e.end <= from_char)
            .map(|e| e.new_text.chars().count() as isize - (e.end - e.start) as isize)
            .sum();
        let Some(index) = self.editors.iter().position(|e| e.path.as_deref() == Some(path)) else {
            return;
        };
        let editor = &mut self.editors[index];
        editor.cursor_line = line.min(editor.rope.len_lines().saturating_sub(1));
        let wanted = (col as isize + delta).max(0) as usize;
        editor.cursor_col = wanted.min(editor.line_char_len(editor.cursor_line));
    }

    // ---- Laying the file out ------------------------------------------------------------------
    //
    // The second thing here that writes, and the one place it differs from the rename is worth
    // saying once rather than in each of the three functions below: there is no preview.
    //
    // A rename reaches files nobody is looking at, changes a handful of characters in each, and
    // can only be judged by reading what it would do — so it is read first. A format rewrites the
    // one buffer already on screen, and the reader is looking straight at the answer the moment it
    // lands. A diff of a whole file would be longer than the file, shown in a box smaller than the
    // editor behind it, and answered yes every single time. What makes it safe is not a question
    // asked beforehand but the thing the roadmap asked for: *un edit unico*, one step of undo, so
    // a format nobody liked is one Ctrl+Z away from never having happened.

    /// Asks the server how the current file should be laid out.
    ///
    /// The same opening as [`Self::lsp_find_references`], down to sending the file first, and for
    /// a sharper version of the same reason: a formatter answers in spans measured against the
    /// text it was last told about, and applying those to text that has since been typed into
    /// would not move a mark, it would delete the wrong characters.
    ///
    /// No arguments and no box. Which file is the question — there is only ever one — and the
    /// tab size and whether it is spaces come from the settings rather than from a prompt: they
    /// are already answered, in the two settings the editor itself indents by, and asking again
    /// would be an invitation to give a formatter a different answer from the one Tab gives.
    pub fn lsp_format_document(&mut self) {
        let lang = self.settings.lang;
        let index = self.active_editor_index();
        let Some(editor) = self.editors.get(index) else { return };
        let Some(path) = editor.path.clone() else { return };
        // Asked before the question goes out, not when the answer lands. A buffer that cannot be
        // typed in cannot be laid out either, and the honest moment to say so is the one the key
        // was pressed in — a round trip first would make it look as though the server refused.
        if editor.read_only {
            self.status_message = i18n::msg_format_read_only(lang).to_string();
            return;
        }
        let from = (path.clone(), editor.cursor_line, editor.cursor_col);
        let text = editor.rope.to_string();
        let Some(absolute) = Self::lsp_absolute_for(&self.lsp_paths, &path) else {
            self.status_message = i18n::msg_lsp_needs_saving(lang).to_string();
            return;
        };
        self.lsp_paths.insert(absolute.clone(), path.clone());
        // Read before the client is borrowed, since both come out of `self`.
        let (tab_size, insert_spaces) = (self.settings.tab_size, self.settings.insert_spaces);
        let Some(client) = self.lsp_client_for(&path).filter(|c| c.ready()) else {
            self.status_message = i18n::msg_lsp_none_here(lang).to_string();
            return;
        };
        client.did_change(&absolute, &text);
        match client.formatting(&absolute, tab_size, insert_spaces) {
            Some(id) => {
                self.lsp_formatting = Some(PendingAsk { id, from });
                self.status_message = i18n::msg_format_asking(lang).to_string();
            }
            None => self.status_message = i18n::msg_lsp_none_here(lang).to_string(),
        }
    }

    /// Reads the server's answer and writes it into the buffer, or says why it will not.
    ///
    /// The id is the whole guard against a stale reply, as everywhere else here: by the time this
    /// arrives the tab may have been closed, the file reloaded, or a second format asked for. An
    /// answer to a question this is no longer waiting for is dropped rather than applied.
    fn lsp_format_answer(&mut self, id: i64, edits: Result<Vec<crate::lsp::SpanEdit>, String>) {
        let lang = self.settings.lang;
        let Some(asked) = self.lsp_formatting.take().filter(|a| a.id == id) else { return };
        let edits = match edits {
            Ok(edits) => edits,
            // The server's own sentence, unwrapped and unexplained, for the reason the rename
            // gives: it is the only party that knows why.
            Err(complaint) => {
                self.status_message = complaint;
                return;
            }
        };
        // Nothing to do, said out loud. This is the answer a well-laid-out file gets, and it is
        // the one place a silent key would be actively misleading: "the format did nothing"
        // looks exactly like "the format did not happen", and only one of them is worth acting
        // on. The empty list is why a server's *refusal* is carried separately.
        if edits.is_empty() {
            self.status_message = i18n::msg_format_already(lang).to_string();
            return;
        }
        let (path, line, col) = asked.from;
        let converted = match self.format_edits_for(&path, &edits) {
            Ok(converted) => converted,
            Err(refusal) => {
                self.status_message = refusal;
                return;
            }
        };
        let Some(index) =
            self.editors.iter().position(|e| e.path.as_deref() == Some(path.as_path()))
        else {
            self.status_message = i18n::msg_format_refused_moved(lang).to_string();
            return;
        };
        let count = converted.len();
        let editor = &mut self.editors[index];
        let Some((start, end, rebuilt)) = edits_as_one_span(editor, &converted) else { return };
        editor.replace_char_range(start, end, &rebuilt);
        // Where the cursor was, clamped to what the file now has — line and column, not the
        // offset the rename puts back. The rename can be exact because it knows how much text
        // each of its edits added or removed before the caret; a formatter moves whole lines
        // around, and an offset carried through that lands somewhere arithmetically defensible
        // and visibly wrong. The line you were reading is the honest thing to keep.
        editor.cursor_line = line.min(editor.rope.len_lines().saturating_sub(1));
        editor.cursor_col = col.min(editor.line_char_len(editor.cursor_line));
        // Nothing is told to the server here, as after a rename: the revision the buffer just
        // bumped is what the debounced `didChange` in the frame loop watches.
        self.status_message = i18n::msg_format_applied(lang, count);
    }

    /// Every span the server's answer asks to be replaced, in absolute character indices, or the
    /// one sentence saying why none of it is going to happen.
    ///
    /// The finding and the wording; the arithmetic is in [`format_spans`], which is a free
    /// function so it can be tested against a rope and a list of edits rather than against an
    /// application with two shells running in it.
    ///
    /// Shared with the code action that lands in a single buffer, which is the same question about
    /// the same kind of answer: a list of spans in the server's units, against one open file, that
    /// has to become one replacement or none. The two refusals below are the two that can happen
    /// to either of them, which is why the wording fits both.
    fn format_edits_for(
        &self,
        path: &Path,
        edits: &[crate::lsp::SpanEdit],
    ) -> Result<Vec<BufferEdit>, String> {
        let lang = self.settings.lang;
        let Some(editor) = self.editors.iter().find(|e| e.path.as_deref() == Some(path)) else {
            return Err(i18n::msg_format_refused_moved(lang).to_string());
        };
        // Asked again, and not out of doubt about the first one: between the key press and this
        // answer the file may have been reopened read-only, and `replace_char_range` would then
        // return in silence and let the status line report a format that never happened.
        if editor.read_only {
            return Err(i18n::msg_format_read_only(lang).to_string());
        }
        let lines = self.file_lines(path);
        format_spans(&editor.rope, &lines, &|text, col| self.lsp_chars_for(path, text, col), edits)
            .map_err(|refusal| match refusal {
                FormatRefusal::Moved => i18n::msg_format_refused_moved(lang).to_string(),
                FormatRefusal::Overlap => i18n::msg_format_refused_overlap(lang).to_string(),
            })
    }

    // ---- What the server offers to do about it -------------------------------------------------
    //
    // The third question that writes, and the one that reuses both of the roads the first two
    // built rather than laying a third. What comes back is a list of things the server could do
    // here; picking one produces a `WorkspaceEdit`, and a `WorkspaceEdit` is a thing this
    // application already knows two ways of carrying out:
    //
    // * everything inside one open buffer goes down the format's road — converted with
    //   `format_spans`, which is the general one, because an action's spans cross lines as a
    //   matter of course (a `use` line inserted at the top, a block replaced by a guarded return)
    //   and applied as a single edit that one Ctrl+Z takes back;
    // * anything wider goes down the rename's — the preview, the revision check, and every one of
    //   the rename's refusals, including the honest one for a file the server names that no tab
    //   holds. Not a second policy: the same policy, asked the same question.
    //
    // On demand, never on save. The roadmap's rule, and the one this whole file obeys: first the
    // mechanism, then the policy about when to run it.

    /// Asks the server what can be done about the code under the cursor.
    ///
    /// The same opening as [`Self::lsp_find_references`], down to sending the file first, and for
    /// the format's sharper version of the reason: the answer is a set of spans measured against
    /// the text the server was last told about, and applying those to text that has since been
    /// typed into would not move a mark, it would delete the wrong characters.
    ///
    /// The selection when there is one and the caret as an empty range when there is not. Both are
    /// what the protocol means by a range, and the difference is a real one: "what can be done
    /// here" and "what can be done to *this*" are different questions, and a server given a
    /// selection answers the second — an extraction, a block turned inside out — where a caret
    /// gets the fix for the error it is sitting in.
    pub fn lsp_code_actions(&mut self) {
        let lang = self.settings.lang;
        let index = self.active_editor_index();
        let Some(editor) = self.editors.get(index) else { return };
        let Some(path) = editor.path.clone() else { return };
        let ((start_line, start_col), (end_line, end_col)) = editor
            .selection_range()
            .unwrap_or(((editor.cursor_line, editor.cursor_col), (editor.cursor_line, editor.cursor_col)));
        // Each end's own line, because each end's column is measured against the line it is on —
        // the mistake `format_spans` exists to avoid, made here instead.
        let line_of = |line: usize| editor.rope.get_line(line).map(|l| l.to_string()).unwrap_or_default();
        let (start_text, end_text) = (line_of(start_line), line_of(end_line));
        let from = (path.clone(), editor.cursor_line, editor.cursor_col);
        let text = editor.rope.to_string();
        let Some(absolute) = Self::lsp_absolute_for(&self.lsp_paths, &path) else {
            self.status_message = i18n::msg_lsp_needs_saving(lang).to_string();
            return;
        };
        self.lsp_paths.insert(absolute.clone(), path.clone());
        // What this file's server has said about it, in its own units and whole — the `code` and
        // the `data` it hangs off its own diagnostics are what it matches its quick fixes against.
        // Read before the client is borrowed, since both come out of `self`.
        let diagnostics = self.lsp_raw.get(&path).cloned().unwrap_or_default();
        let Some(client) = self.lsp_client_for(&path).filter(|c| c.ready()) else {
            self.status_message = i18n::msg_lsp_none_here(lang).to_string();
            return;
        };
        // Asked before the question goes out rather than after a refusal comes back. A server that
        // never claimed this can still be *sent* it — and would answer with a method-not-found the
        // status line would then print as though it were news.
        if !client.actions().offered {
            self.status_message = i18n::msg_code_actions_unsupported(lang).to_string();
            return;
        }
        client.did_change(&absolute, &text);
        match client.code_actions(
            &absolute,
            (start_line, &start_text, start_col),
            (end_line, &end_text, end_col),
            &diagnostics,
        ) {
            Some(id) => {
                self.lsp_acting = Some(PendingAsk { id, from });
                self.status_message = i18n::msg_code_actions_asking(lang).to_string();
            }
            None => self.status_message = i18n::msg_lsp_none_here(lang).to_string(),
        }
    }

    /// Turns what the server offered into a list to choose from.
    ///
    /// In the server's own order, untouched, for the reason the outline keeps document order: the
    /// order is the server's judgement of what is most likely wanted here — the quick fix for the
    /// error under the caret first, the refactorings that apply anywhere after it — and sorting by
    /// title would replace that judgement with the alphabet.
    fn lsp_offer_code_actions(
        &mut self,
        id: i64,
        actions: Result<Vec<crate::lsp::CodeAction>, String>,
    ) {
        let lang = self.settings.lang;
        let Some(asked) = self.lsp_acting.take().filter(|a| a.id == id) else { return };
        let actions = match actions {
            Ok(actions) => actions,
            // The server's own sentence, as after a rename and a format, and for the same reason:
            // it is the only party here that knows why.
            Err(complaint) => {
                self.status_message = complaint;
                return;
            }
        };
        if actions.is_empty() {
            self.status_message = i18n::msg_code_actions_none(lang).to_string();
            return;
        }
        let items = code_action_items(actions);
        self.open_server_list(
            Key::PickerCodeActions,
            crate::picker::PickerKind::CodeActions,
            items,
            asked.from,
        );
    }

    /// Carries out the action that was picked, or asks what it would change first.
    ///
    /// The second question is the ordinary case rather than the odd one: rust-analyzer names its
    /// assists and computes none of their edits until one is chosen, which is why this is where
    /// the resolve request lives — one round trip for the row somebody wanted, not a dozen for the
    /// eleven they did not.
    fn apply_code_action(
        &mut self,
        action: crate::lsp::CodeAction,
        origin: Option<(PathBuf, usize, usize)>,
    ) {
        let lang = self.settings.lang;
        // The list is only ever opened through `open_server_list`, which always writes one down.
        // Without it there is no buffer to put a cursor back in and no file to resolve against.
        let Some(from) = origin else { return };
        if let Some(plan) = action.edit {
            self.carry_out_code_action(&action.title, plan, from);
            return;
        }
        // No file is announced here and none needs to be: the resolve request names an action and
        // not a document, and the file it came from was sent when the list was asked for.
        let path = from.0.clone();
        let Some(client) = self.lsp_client_for(&path).filter(|c| c.ready()) else {
            self.status_message = i18n::msg_lsp_none_here(lang).to_string();
            return;
        };
        match client.resolve_code_action(&action.raw) {
            Some(id) => {
                self.status_message = i18n::msg_code_action_asking(lang, &action.title);
                self.lsp_action_edit = Some(PendingAction { id, from, title: action.title });
            }
            None => self.status_message = i18n::msg_lsp_none_here(lang).to_string(),
        }
    }

    /// Reads what the server filled in for the action that was picked, and carries it out.
    fn lsp_code_action_edit(
        &mut self,
        id: i64,
        plan: Result<Option<crate::lsp::RenamePlan>, String>,
    ) {
        let lang = self.settings.lang;
        let Some(asked) = self.lsp_action_edit.take().filter(|a| a.id == id) else { return };
        match plan {
            Err(complaint) => self.status_message = complaint,
            // The server answered without saying what it would change. An answer, and said out
            // loud for the reason an empty format answer is: a row that did nothing in silence is
            // a row somebody presses again.
            Ok(None) => self.status_message = i18n::msg_code_action_no_changes(lang).to_string(),
            Ok(Some(plan)) => self.carry_out_code_action(&asked.title, plan, asked.from),
        }
    }

    /// Writes one action's `WorkspaceEdit` into the buffers, down whichever of the two roads it
    /// belongs on.
    ///
    /// The fork is the only decision this function makes, and it is made on what the answer
    /// *reaches*: one open buffer is the format's case exactly — the text you are looking at,
    /// changed while you watch, one step of undo — and anything else is the rename's, because the
    /// edits are then somewhere nobody can see them arrive. A file the server names that no tab
    /// holds is refused by that road, with its count and its instruction, which is the honest
    /// answer here for the reason it is honest there.
    fn carry_out_code_action(
        &mut self,
        title: &str,
        plan: crate::lsp::RenamePlan,
        from: (PathBuf, usize, usize),
    ) {
        let lang = self.settings.lang;
        // Asked here as well as inside the preview, because the single-buffer road does not go
        // through it — and a create, a move or a delete of a file is not something either road
        // does. Refused whole, as the rename refuses it, and for its reason.
        if plan.file_ops {
            self.status_message = i18n::msg_rename_refused_file_ops(lang).to_string();
            return;
        }
        // Back to the spelling the tabs use, as every other answer is mapped back: the server
        // answers in resolved paths, and a file matched under the wrong name is one this would
        // report as not open.
        let files: Vec<crate::lsp::FileEdits> = plan
            .files
            .iter()
            .map(|file| crate::lsp::FileEdits {
                path: self.lsp_paths.get(&file.path).cloned().unwrap_or_else(|| file.path.clone()),
                edits: file.edits.clone(),
            })
            .filter(|file| !file.edits.is_empty())
            .collect();
        if files.is_empty() {
            self.status_message = i18n::msg_code_action_no_changes(lang).to_string();
            return;
        }
        let one_open_buffer = files.len() == 1
            && self.editors.iter().any(|e| e.path.as_deref() == Some(files[0].path.as_path()));
        if !one_open_buffer {
            // The rename's road, whole: its preview, its refusals, its revision check at Enter.
            // The empty old name is what tells the box it is not a rename — see [`RenamePreview`].
            match self.edit_preview_from(&from, "", title, plan) {
                Ok(preview) => {
                    self.rename_preview = Some(preview);
                    self.status_message = String::new();
                }
                Err(refusal) => self.status_message = refusal,
            }
            return;
        }
        let path = files[0].path.clone();
        let converted = match self.format_edits_for(&path, &files[0].edits) {
            Ok(converted) => converted,
            Err(refusal) => {
                self.status_message = refusal;
                return;
            }
        };
        let Some(index) =
            self.editors.iter().position(|e| e.path.as_deref() == Some(path.as_path()))
        else {
            self.status_message = i18n::msg_format_refused_moved(lang).to_string();
            return;
        };
        let count = converted.len();
        let (line, col) = (from.1, from.2);
        let editor = &mut self.editors[index];
        let Some((start, end, rebuilt)) = edits_as_one_span(editor, &converted) else { return };
        editor.replace_char_range(start, end, &rebuilt);
        // Where the cursor was, clamped to what the file now has — the format's treatment and not
        // the rename's arithmetic, for the format's reason: an action moves whole lines around,
        // and an offset carried through that lands somewhere defensible and visibly wrong.
        editor.cursor_line = line.min(editor.rope.len_lines().saturating_sub(1));
        editor.cursor_col = col.min(editor.line_char_len(editor.cursor_line));
        // Nothing is told to the server here, as after a rename and a format: the revision the
        // buffer just bumped is what the debounced `didChange` in the frame loop watches.
        self.status_message = i18n::msg_code_action_applied(lang, title, count);
    }

    // ---- Widening and narrowing the selection ---------------------------------------------------
    //
    // The first thing here asked of a server that changes nothing but the *selection*, and the
    // only one whose answer is used more than once: `textDocument/selectionRange` comes back as a
    // ladder — the identifier, the expression around it, the statement around that, the item around
    // that — and one request buys every rung. So the shape of this is a walk kept on the app, not a
    // request per keypress: the first Expand asks, every Expand after it climbs, and Shrink goes
    // back down without a word to anybody.
    //
    // The walk lives exactly as long as it is telling the truth. See [`SelectionWalk`] for the
    // three things it checks and why the check is a check rather than a hook.

    /// Widens the selection to the next thing that encloses it.
    ///
    /// Asks the server only when there is no walk in hand — which is the first press, and every
    /// press after anything else has moved the cursor. A second press while the ladder is alive
    /// climbs a rung in this process and sends nothing: the whole point of an answer that arrives
    /// as a chain is that the round trips stop after the first one.
    ///
    /// The question is asked at the *start* of an active selection rather than at the cursor. A
    /// selection running from an identifier out to the end of an expression has its caret at the
    /// far end, and a server asked about that place answers about whatever begins there — so the
    /// second press would climb a different ladder from the first, which reads on screen as the
    /// selection jumping sideways instead of growing.
    pub fn lsp_expand_selection(&mut self) {
        let lang = self.settings.lang;
        if self.step_selection_walk(1) {
            return;
        }
        // No walk, or one that stopped being true: this is a fresh question about wherever the
        // cursor is now, so anything left of the old one goes before the new one is asked for.
        self.selection_walk = None;
        let index = self.active_editor_index();
        let Some(editor) = self.editors.get(index) else { return };
        let Some(path) = editor.path.clone() else { return };
        let (line, col) = match editor.selection_range() {
            Some(((start_line, start_col), _)) => (start_line, start_col),
            None => (editor.cursor_line, editor.cursor_col),
        };
        let line_text = editor.rope.line(line).to_string();
        let text = editor.rope.to_string();
        let from = (path.clone(), line, col);
        let Some(absolute) = Self::lsp_absolute_for(&self.lsp_paths, &path) else {
            self.status_message = i18n::msg_lsp_needs_saving(lang).to_string();
            return;
        };
        self.lsp_paths.insert(absolute.clone(), path.clone());
        let Some(client) = self.lsp_client_for(&path).filter(|c| c.ready()) else {
            self.status_message = i18n::msg_lsp_none_here(lang).to_string();
            return;
        };
        // Asked before the question goes out, as the code action row does it and for its reason: a
        // server that never claimed this would answer with a method-not-found, and the status line
        // would print the server's protocol error as though it were an answer about the code.
        if !client.offered().selection_ranges {
            self.status_message = i18n::msg_selection_unsupported(lang).to_string();
            return;
        }
        // The file goes first, as it does before every question whose answer is a set of positions:
        // a ladder measured against the text of four hundred milliseconds ago would select the
        // right number of characters in the wrong place.
        client.did_change(&absolute, &text);
        match client.selection_range(&absolute, line, &line_text, col) {
            Some(id) => {
                self.lsp_widening = Some(PendingAsk { id, from });
                self.status_message = i18n::msg_selection_asking(lang).to_string();
            }
            None => self.status_message = i18n::msg_lsp_none_here(lang).to_string(),
        }
    }

    /// Narrows the selection back to the last thing it grew out of.
    ///
    /// Never asks anybody anything. Shrinking is only meaningful as the undo of an expansion — there
    /// is no such thing as "the thing inside this selection" without knowing which way you came —
    /// so with no walk in hand there is nothing to do but say so.
    pub fn lsp_shrink_selection(&mut self) {
        if self.step_selection_walk(-1) {
            return;
        }
        self.selection_walk = None;
        self.status_message = i18n::msg_selection_nothing_to_shrink(self.settings.lang).to_string();
    }

    /// Climbs one rung of a live walk, or says the walk is not usable and answers `false`.
    ///
    /// The one place the three liveness questions are asked, so Expand and Shrink cannot come to
    /// different conclusions about the same walk: it is about this buffer, at this revision, and
    /// the selection on screen is still the one the walk put there.
    ///
    /// Running out of ladder is a `true` with a sentence, not a `false`: the walk is perfectly
    /// alive, there is simply nothing wider in it — and answering `false` would send the same
    /// question to the server again to be told the same thing a round trip later.
    fn step_selection_walk(&mut self, direction: isize) -> bool {
        let lang = self.settings.lang;
        let index = self.active_editor_index();
        let Some(editor) = self.editors.get(index) else { return false };
        let Some(walk) = self.selection_walk.as_mut() else { return false };
        if !walk.still_true(editor) {
            return false;
        }
        match walk.step(direction) {
            Step::Moved(start, end) => {
                if let Some(editor) = self.editors.get_mut(index) {
                    editor.select_char_range(start, end);
                }
                self.status_message = String::new();
            }
            // Both ends are said out loud rather than silently refused, because a key that does
            // nothing is a key somebody presses harder.
            Step::Widest => self.status_message = i18n::msg_selection_widest(lang).to_string(),
            Step::Narrowest => {
                self.status_message = i18n::msg_selection_narrowest(lang).to_string()
            }
        }
        true
    }

    /// Reads the ladder the server sent, stands on the right rung of it, and keeps it.
    ///
    /// Which rung that is — and why the ones below it are kept — is [`SelectionWalk::starting_at`].
    fn lsp_widen_selection(&mut self, id: i64, chain: Result<Vec<crate::lsp::Span>, String>) {
        let lang = self.settings.lang;
        let Some(asked) = self.lsp_widening.take().filter(|a| a.id == id) else { return };
        let chain = match chain {
            Ok(chain) => chain,
            // The server's own sentence, as after a rename, a format and a code action.
            Err(complaint) => {
                self.status_message = complaint;
                return;
            }
        };
        let path = asked.from.0;
        // The buffer may have been closed, switched or typed into while the answer was in flight.
        // A ladder of char offsets into text that has moved is not a late answer, it is an answer
        // about a different file, so it is dropped rather than applied to whatever is there now.
        let Some(index) = self.editors.iter().position(|e| e.path.as_deref() == Some(path.as_path()))
        else {
            return;
        };
        let spans = self.selection_spans_for(&path, index, &chain);
        let Some(editor) = self.editors.get(index) else { return };
        let here = selected_char_range(editor).unwrap_or_else(|| {
            let at = editor.rope.line_to_char(editor.cursor_line) + editor.cursor_col;
            (at, at)
        });
        let empty = spans.is_empty();
        let revision = editor.revision();
        let Some(walk) = SelectionWalk::starting_at(path, revision, spans, here) else {
            // Either the server named nothing at all, or everything it named is already inside what
            // is selected — which on a whole-file selection is the ordinary end of the walk.
            self.status_message = if empty {
                i18n::msg_selection_nothing_here(lang).to_string()
            } else {
                i18n::msg_selection_widest(lang).to_string()
            };
            return;
        };
        let (start, end) = walk.selected;
        if let Some(editor) = self.editors.get_mut(index) {
            editor.select_char_range(start, end);
        }
        self.selection_walk = Some(walk);
        self.status_message = String::new();
    }

    /// The server's ladder as absolute char ranges in the buffer, innermost first.
    ///
    /// Both ends of every rung are converted, and each against the line it actually sits on: a
    /// range that starts on one line and ends on another has two columns measured in two different
    /// pieces of text, and converting both against the first line is the mistake `format_spans`
    /// exists to avoid. Rungs that cannot be placed in this buffer — a line number past its end,
    /// which is what a server answering about older text sends — are dropped one by one rather than
    /// costing the whole ladder.
    fn selection_spans_for(
        &self,
        path: &Path,
        index: usize,
        chain: &[crate::lsp::Span],
    ) -> Vec<(usize, usize)> {
        let Some(editor) = self.editors.get(index) else { return Vec::new() };
        let lines = editor.rope.len_lines();
        let mut out = Vec::with_capacity(chain.len());
        for span in chain {
            if span.start_line >= lines || span.end_line >= lines {
                continue;
            }
            let text_of = |line: usize| editor.rope.line(line).to_string();
            let start_col = self.lsp_chars_for(path, &text_of(span.start_line), span.start_col);
            let end_col = self.lsp_chars_for(path, &text_of(span.end_line), span.end_col);
            let at = |line: usize, col: usize| {
                editor.rope.line_to_char(line) + col.min(editor.line_char_len(line))
            };
            let (start, end) = (at(span.start_line, start_col), at(span.end_line, end_col));
            // Empty rungs are dropped rather than kept. A rung is something to *select*, and a
            // selection of nothing is no selection at all — the walk would land on one and
            // immediately decide it had been overtaken by somebody else's cursor.
            if start < end {
                out.push((start, end));
            }
        }
        out
    }

    // ---- Where the server says the blocks are ----------------------------------------------------
    //
    // Folding was here before any of this and goes on working exactly as it did: `toggle_fold`,
    // `is_hidden` and every drawing path are untouched. What changed is where the *boundary* comes
    // from — a server's `foldingRange` answer where there is one for the line, and the braces-then-
    // indentation heuristic everywhere else. See `Editor::foldable_range_at`, which is the one
    // place that decides between them, and `Editor::server_folds` for why an edited buffer stops
    // believing the server until the next save.

    /// Asks each open file's server where its blocks are, at the two moments that is worth asking.
    ///
    /// Derived from the state rather than hooked into the saves: a buffer that is clean and whose
    /// revision is not the one already asked about is a file the server has not seen this version
    /// of. That is true when a file is first opened, and true again after each save — and untrue in
    /// between, which is exactly the rule the cache needs, since a dirty buffer's line numbers are
    /// moving under the answer as it travels.
    fn lsp_refresh_folds(&mut self) {
        if self.lsp.is_empty() {
            return;
        }
        let mut wanted: Vec<(PathBuf, PathBuf, u64)> = Vec::new();
        for editor in self.editors.iter().filter(|e| e.preview.is_none()) {
            let Some(path) = editor.path.as_deref() else { continue };
            let revision = editor.revision();
            if editor.dirty || self.lsp_folds_asked.get(path) == Some(&revision) {
                continue;
            }
            // Only once it has been announced. Asking about a file the server has not been told
            // about is asking about a file it has never read.
            if !self.lsp_sent.contains_key(path) {
                continue;
            }
            let Some(absolute) = Self::lsp_absolute_for(&self.lsp_paths, path) else { continue };
            wanted.push((absolute, path.to_path_buf(), revision));
        }
        for (absolute, held, revision) in wanted {
            let Some(client) = self.lsp_client_for(&held).filter(|c| c.ready()) else { continue };
            if !client.offered().folding_ranges {
                // Written down anyway, so a server that does not do this is asked once per
                // revision and not once per frame for the rest of the session.
                self.lsp_folds_asked.insert(held, revision);
                continue;
            }
            if let Some(id) = client.folding_ranges(&absolute) {
                self.lsp_folding.insert(id, held.clone());
            }
            self.lsp_folds_asked.insert(held, revision);
        }
    }

    /// Hands one file's fold boundaries to the tab that holds it.
    ///
    /// The same route a diagnostic takes, and for the same reason: the answer names a file, the
    /// boundaries mean nothing without the buffer they are measured against, and a file no tab
    /// holds has nothing for them to be true about. Nothing is redrawn differently on their
    /// account — a fold marker only appears where something is foldable, and this changes which
    /// lines those are, not what happens when one is pressed.
    fn lsp_remember_folds(&mut self, id: i64, ranges: Vec<(usize, usize)>) {
        let Some(path) = self.lsp_folding.remove(&id) else { return };
        let Some(editor) = self.editors.iter_mut().find(|e| e.path.as_deref() == Some(path.as_path()))
        else {
            return;
        };
        // Not guarded on the buffer having stayed clean while the answer travelled: it is
        // `foldable_range_at` that refuses to read these while a buffer is dirty, in one place,
        // and a second copy of that rule here would be a second chance to get it wrong.
        editor.server_folds = ranges;
    }

    /// Offers everything the servers have said is wrong, as a list to jump into.
    ///
    /// The honest scope is the files that are open, and the title says so. A diagnostic for a
    /// file with no tab is dropped the moment it arrives — turning one into a [`crate::lsp::Mark`]
    /// needs the text to measure its columns against, and there is none — so this cannot be a
    /// picture of the project however much a column of `path:line` looks like one.
    ///
    /// No chord, unlike the other two. This is read after a build rather than mid-keystroke, and
    /// the menu and the palette are where a thing like that is looked for.
    fn open_diagnostics_picker(&mut self) {
        let lang = self.settings.lang;
        let root = self.root.clone();
        let items: Vec<crate::picker::PickItem> = {
            let mut rows: Vec<(&PathBuf, &crate::lsp::Mark)> = self
                .diagnostics
                .iter()
                .flat_map(|(path, marks)| marks.iter().map(move |mark| (path, mark)))
                .collect();
            // Worst first, then by file and line. What anybody opens this for is the errors, and
            // a list that opened on a hint about a doc comment is a list nobody scrolls.
            rows.sort_by(|(left_path, left), (right_path, right)| {
                right
                    .severity
                    .cmp(&left.severity)
                    .then(left_path.cmp(right_path))
                    .then(left.line.cmp(&right.line))
                    .then(left.start.cmp(&right.start))
            });
            rows.iter()
                .map(|(path, mark)| crate::picker::PickItem {
                    // A compiler message is several lines when it wants to be, and a row of a
                    // picker is one: the breaks become spaces rather than breaking the list.
                    label: located_label(
                        &root,
                        path,
                        mark.line + 1,
                        Some(&mark.message.replace('\n', " ")),
                    ),
                    shortcut: Some(mark.severity.word().to_string()),
                    // The column the mark starts at, which is already in characters — a
                    // diagnostic was converted when it arrived, against the buffer it is about.
                    action: crate::picker::PickAction::FileLine(
                        (*path).clone(),
                        mark.line + 1,
                        mark.start,
                    ),
                })
                .collect()
        };
        if items.is_empty() {
            self.status_message = i18n::msg_lsp_no_diagnostics(lang).to_string();
            return;
        }
        self.picker = Some(crate::picker::Picker::new(
            i18n::t(lang, Key::PickerDiagnostics),
            crate::picker::PickerKind::Diagnostics,
            items,
        ));
    }

    /// Puts one of the server's lists on screen and remembers where it was asked from, so
    /// confirming a row is a jump that can be come back from.
    fn open_server_list(
        &mut self,
        title: Key,
        kind: crate::picker::PickerKind,
        items: Vec<crate::picker::PickItem>,
        origin: (PathBuf, usize, usize),
    ) {
        let lang = self.settings.lang;
        let mut picker = crate::picker::Picker::new(i18n::t(lang, title), kind, items);
        picker.origin = Some(origin);
        self.picker = Some(picker);
        // The list is the answer, so the line that said the question had gone out has done its
        // job — and leaving "Asking…" under a list of answers reads as one still to come.
        self.status_message = String::new();
    }

    /// Every line of a file, as it is now.
    ///
    /// Out of the buffer when a tab holds it, off the disk when none does. The buffer first is
    /// the important half: it is the text the server was told about, and the file on disk may be
    /// a save behind it — so a row built from disk would quote a line the server never saw.
    ///
    /// Empty when the file cannot be read at all. That is a list of rows that say where without
    /// saying what, which is honest; inventing the text would not be.
    fn file_lines(&self, path: &Path) -> Vec<String> {
        if let Some(editor) = self.editors.iter().find(|e| e.path.as_deref() == Some(path)) {
            return editor
                .rope
                .lines()
                .map(|line| line.to_string().trim_end_matches('\n').to_string())
                .collect();
        }
        std::fs::read_to_string(path)
            .map(|text| text.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// A column the server sent, in the characters the editor counts, measured against the line
    /// it belongs to.
    ///
    /// The path decides the units because the units are the *file's* server's: a project with
    /// Rust and Python open is running two of them, and they need not have negotiated the same
    /// counting. Defaults to UTF-16 for a file no server serves, which is what the protocol says
    /// when nobody has said otherwise.
    fn lsp_chars_for(&self, path: &Path, line_text: &str, column: usize) -> usize {
        let utf16 = self
            .lsp_argv_for(path)
            .and_then(|argv| argv.first().cloned())
            .and_then(|program| self.lsp.get(&program))
            .map(crate::lsp::Client::utf16)
            .unwrap_or(true);
        if utf16 {
            crate::lsp::utf16_to_chars(line_text, column)
        } else {
            // UTF-8 is not "the same number": it is a byte offset into the line, and a row on a
            // line with an accent before the name lands to the right of it.
            crate::lsp::utf8_to_chars(line_text, column)
        }
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
        // The check above already implies this — a file is only recorded as sent once the server
        // was ready to be told about it — but the rule that nothing goes out before the handshake
        // is worth being true at each of the places that send something, rather than by
        // inference from another line twenty lines up.
        let Some(client) = self.lsp_client_for(&path).filter(|c| c.ready()) else { return };
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
            self.redraw = true;
            // Taken whatever the answer turns out to be: it belongs to *this* walk, and a
            // replacement left lying about would be applied to whatever the next search found.
            let asked = self.replace_asked.take();
            if let Some(detail) = &outcome.error {
                self.status_message = i18n::msg_find_pattern_error(lang, detail);
                continue;
            }
            if outcome.hits.is_empty() {
                // "Nowhere in N files" is a claim about the project, and a search that stopped at
                // a limit has not earned it: it only looked at part of the tree. Said the other
                // way round — nothing found, and the search was cut short — it is the truth.
                self.status_message = match outcome.truncated {
                    true => i18n::msg_search_done(lang, 0, outcome.files_searched, true),
                    false => i18n::msg_search_none(lang, &outcome.query, outcome.files_searched),
                };
                continue;
            }
            // A replacement was typed, so the hits are not a list of places to go: they are the
            // input to a preview of what would change. Everything above this line is the search
            // exactly as it was, which is the point — the second field changes what the answer
            // *opens*, not how the question was asked.
            if let Some(asked) = asked {
                match self.replace_sweep_from(&outcome, &asked) {
                    Ok(sweep) => {
                        self.replace_sweep = Some(sweep);
                        // The preview is the answer, so the line that said the walk was running
                        // has done its job.
                        self.status_message = String::new();
                    }
                    Err(refusal) => self.status_message = refusal,
                }
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
                i18n::t(lang, Key::PickerSearchResults),
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
            self.redraw = true;
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
            // The end of a commit is the last *screenful* of it, not the last line sitting alone
            // at the top of an empty reader. Scrolling to the bottom of a document is how you
            // read the end of it, and there is nothing to read below the final line.
            KeyCode::End => {
                let screenful = panel.body_rows.max(1) as isize - 1;
                detail.scroll = (max - screenful).clamp(0, max.max(0)) as usize;
            }
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
                KeyCode::Backspace => pop_grapheme(typed),
                // The modifiers are checked, and that is not pedantry: without it `Ctrl+V` puts a
                // `v` in the commit message, which is what a person pressing it least wants and
                // has no way to tell has happened until the commit is made.
                KeyCode::Char(c) if is_a_typed_character(key) => typed.push(c),
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
            // Asked here rather than left to git, which refuses an empty message too but says so
            // in a paragraph about the commit template — a wall of text about a box the user has
            // just seen, in place of the one sentence that says what to do about it.
            GitText::Commit | GitText::Amend if typed.is_empty() => {
                self.git_say(i18n::msg_git_needs_a_message(lang).to_string(), true)
            }
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
        //
        // And the tab may not be there at all — an unreadable or vanished file leaves nothing
        // open, so there is no buffer to put a cursor in.
        let idx = self.pane_editor_index(self.editor_pane_focus);
        let Some(editor) = self.editors.get_mut(idx) else { return };
        let len = editor.line_char_len(editor.cursor_line);
        editor.cursor_col = col.min(len);
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
                // Said once, on the way in, in the same slot as the read-only notice and for
                // the same reason: a tab that quietly does less than the others has to say so
                // at the moment it opens, or the user meets the missing colours as a defect.
                // Read-only wins the slot where both apply — a file you cannot save is the
                // more surprising of the two — and the persistent chip beside `row:col` keeps
                // the large-file fact on screen after this sentence is gone.
                let said = {
                    let editor = self.editor();
                    if editor.is_read_only() {
                        i18n::msg_opened_read_only(lang, &editor.title(lang))
                    } else if editor.is_large() {
                        i18n::msg_opened_large(
                            lang,
                            &editor.title(lang),
                            editor.megabytes(),
                            editor.undo_depth(),
                        )
                    } else {
                        i18n::msg_opened(lang, &editor.title(lang))
                    }
                };
                self.status_message = said;
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
            // Both go through `set_fit`, which also decides whether the view stays the editor's
            // to fit as the pane changes: "fit" is the automatic state and hands it back, "wide"
            // is a choice and keeps it. See `Preview::adjusted`.
            ui::NavControl::FitPage => preview.set_fit(crate::preview::Fit::Page),
            ui::NavControl::FitWidth => preview.set_fit(crate::preview::Fit::Width),
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
        let pane = if pane_idx == 0 { EditorPane::Left } else { EditorPane::Right };
        // `get`, because the pointer can be over a pane with every tab closed: there is no
        // buffer to ask about, and so nothing to zoom.
        let idx = self.pane_editor_index(pane);
        let (_, _, content) = ui::pane_areas(self, idx, *rect);
        if !within(content, col, row) {
            return false;
        }
        if self.editors.get(idx).map(|e| e.preview.is_none()).unwrap_or(true) {
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
        // Nowhere to go on a picture shown whole, and no bar over one either — but a drag that
        // arrived anyway must not be the thing that cuts it. See `Preview::pan_room`.
        let (max_x, max_y) = preview.pan_room();
        if (max_x, max_y) == (0, 0) {
            return;
        }
        let Some(full) = preview.full.as_ref() else { return };
        match axis {
            ui::Axis::Vertical => preview.scroll_px = position.min(max_y),
            ui::Axis::Horizontal => preview.scroll_x = position.min(max_x),
        }
        // Dragged to a place on the page by hand: from here the pane stops re-fitting it, or the
        // next resize would take the reader back to a corner they had just left.
        preview.adjusted = true;
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
        // A picture nobody has zoomed is shown whole, so there is nothing past the edge of the
        // pane to travel to and the arrows are somebody else's. See `Preview::pan_room`.
        let (max_x, max_y) = preview.pan_room();
        if max_x == 0 && max_y == 0 {
            return false;
        }
        let Some(full) = preview.full.as_ref() else { return false };
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
        // Moved by hand, so the view is the reader's from here: see `set_preview_scroll`.
        preview.adjusted = true;
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
        // An animated picture has nothing to re-read: every frame is already in hand at its own
        // size, and putting the current one up again fits it to whatever the zoom and the pane
        // have just become. Decoding the file again would also put the animation back to its
        // first frame, which is a visible jump for a change that is not about the file at all.
        if preview.animation.is_some() {
            self.show_frame(idx);
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
        // Everything is a run of text, never a rectangle. A column selection left switched on
        // would otherwise make Select All a block as wide as the last line and as tall as the
        // file — and then a keystroke would write down the whole of it.
        ed.selection_block = false;
        ed.selection_anchor = Some((0, 0));
        let last_line = ed.rope.len_lines().saturating_sub(1);
        ed.cursor_line = last_line;
        ed.cursor_col = ed.line_char_len(last_line);
    }

    fn close_active_editor(&mut self) {
        // The focused pane's tab, not the left pane's: in a split, Ctrl+W on the right half used
        // to close a file on the left that the user was not even looking at.
        let idx = self.active_editor_index();
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
        let lang = self.settings.lang;
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
                // "Save, then go" only goes if the save happened. A read-only file, a full disk
                // or a directory that vanished all end here, and quitting or closing on top of
                // the failure would throw away the very work the prompt was protecting —
                // silently, since the status line explaining it goes with the window.
                let saved = match action {
                    UnsavedPrompt::Quit => self.save_all(),
                    UnsavedPrompt::CloseTab(idx) => match self.editors.get_mut(idx) {
                        Some(ed) => match ed.save() {
                            Ok(()) => true,
                            Err(e) => {
                                self.status_message = i18n::msg_save_error(lang, &e.to_string());
                                false
                            }
                        },
                        None => true,
                    },
                };
                if saved {
                    self.perform_unsaved_action(action);
                }
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

    /// The active buffer as one string, made only when the last one no longer describes it.
    ///
    /// Keyed on which buffer it came from and how many times that buffer has changed; the
    /// character count comes along because a buffer index is only a position in a list, and
    /// closing a tab can slide a different file underneath it at the same revision.
    fn find_text(&mut self) -> &str {
        let editor = self.active_editor_index();
        let revision = self.editor().revision();
        let chars = self.editor().rope.len_chars();
        let fresh = matches!(&self.find_text, Some(c) if (c.editor, c.revision, c.chars) == (editor, revision, chars));
        if !fresh {
            let text = self.editor().rope.to_string();
            self.find_text = Some(FindText { editor, revision, chars, text });
        }
        self.find_text.as_ref().map(|c| c.text.as_str()).unwrap_or_default()
    }

    fn open_find(&mut self, _replace: bool) {
        let mut fs = crate::find::FindState::new();
        // Seed the query from a single-line selection, the way most editors do.
        if let Some(sel) = self.editor().selected_text() {
            if !sel.is_empty() && !sel.contains('\n') {
                fs.query = sel;
            }
        }
        let from = self.editor_cursor_char_idx();
        fs.recompute(self.find_text(), from);
        self.find = Some(fs);
        self.apply_find_selection();
    }

    /// Recomputes matches after the query changed, biasing the current match to the cursor.
    ///
    /// The state is lifted out for the duration so the scan can borrow the buffer's text at the
    /// same time; it goes back untouched.
    fn recompute_find(&mut self) {
        let from = self.editor_cursor_char_idx();
        let Some(mut f) = self.find.take() else { return };
        f.recompute(self.find_text(), from);
        self.find = Some(f);
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
        // Replace All is one action, so it has to be one step to undo: replacing each match on
        // its own put a whole copy of the file on the undo stack per match, and taking back a
        // thousand-match run meant a thousand Ctrl+Z.
        //
        // So the run from the first match to the last is rebuilt in memory — replacements where
        // the matches were, the text between them carried over verbatim — and written back in a
        // single edit. Each replacement is still worked out from the text it covers, since a
        // pattern's capture groups differ from match to match.
        let matches: Vec<(usize, usize)> = f.matches.clone();
        let count = matches.len();
        let (span_start, span_end) = (matches[0].0, matches[count - 1].1);
        let mut rebuilt = String::new();
        let mut carried = span_start;
        for &(s, e) in &matches {
            if s > carried {
                rebuilt.push_str(&self.editor().rope.slice(carried..s).to_string());
            }
            let matched = self.matched_text((s, e));
            let Some(replace) = self.find.as_ref().map(|f| f.replacement_for(&matched)) else { return };
            rebuilt.push_str(&replace);
            carried = e;
        }
        self.editor_mut().replace_char_range(span_start, span_end, &rebuilt);
        let lang = self.settings.lang;
        self.status_message = i18n::msg_replaced_all(lang, count);
        self.recompute_find();
    }

    fn handle_find_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                self.find = None;
                // The copy of the file the box was scanning goes with it: nothing else reads it,
                // and a closed search box has no business holding a whole buffer twice over.
                self.find_text = None;
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
                        pop_grapheme(&mut f.replace);
                    } else {
                        pop_grapheme(&mut f.query);
                    }
                }
                self.recompute_find();
            }
            KeyCode::Char(c) if is_a_typed_character(key) => {
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
            KeyCode::Backspace => pop_grapheme(&mut self.goto_input),
            KeyCode::Char(c) if c.is_ascii_digit() && is_a_typed_character(key) => {
                self.goto_input.push(c)
            }
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
            KeyCode::Backspace => pop_grapheme(&mut self.new_entry_input),
            KeyCode::Char(c) if is_a_typed_character(key) => self.new_entry_input.push(c),
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
                shortcut: it.shortcut.map(|s| crate::keymap::shortcut_hint(lang, &self.keymap, s)),
                action: crate::picker::PickAction::Command(it.action),
            })
            .collect();
        let title = i18n::t(lang, Key::PickerCommands);
        self.picker = Some(crate::picker::Picker::new(title, crate::picker::PickerKind::Commands, items));
    }

    fn open_file_picker(&mut self) {
        let (items, truncated) = self.project_file_items();
        let title = file_picker_title(self.settings.lang, truncated);
        self.picker = Some(crate::picker::Picker::new(title, crate::picker::PickerKind::Files, items));
    }

    /// Every file under the project root, the quick-open default: type a few characters to jump
    /// to one without walking the tree. The flag says the list stopped at the cap, so a name
    /// missing from it may still be in the project.
    fn project_file_items(&self) -> (Vec<crate::picker::PickItem>, bool) {
        let mut files = Vec::new();
        let truncated = collect_project_files(&self.root, &mut files, self.settings.show_hidden_files);
        files.sort();
        let root = self.root.clone();
        let items = files
            .into_iter()
            .map(|p| {
                let label = p.strip_prefix(&root).unwrap_or(&p).to_string_lossy().to_string();
                crate::picker::PickItem { label, shortcut: None, action: crate::picker::PickAction::OpenFile(p) }
            })
            .collect();
        (items, truncated)
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
                    let (items, truncated) = self.project_file_items();
                    let title = file_picker_title(self.settings.lang, truncated);
                    if let Some(picker) = self.picker.as_mut() {
                        picker.path_mode = false;
                        picker.filter_override = None;
                        picker.title = title;
                        picker.set_items(items);
                    }
                }
            }
        }
    }

    fn handle_picker_key(&mut self, key: KeyEvent) {
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
                    pop_grapheme(&mut p.query);
                    p.refilter();
                }
                self.refresh_picker();
            }
            KeyCode::Char(c) if is_a_typed_character(key) => {
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
        let mut recover = None;
        let mut code_action = None;
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
                crate::picker::PickAction::Recover(entry) => recover = Some(entry.clone()),
                crate::picker::PickAction::CodeAction(action) => {
                    code_action = Some(action.clone())
                }
            }
        }
        if let Some(action) = code_action {
            // Where the list was asked from, taken before the picker goes: it is the buffer the
            // edits are about and the cursor to put back afterwards, and unlike a jump it is not
            // pushed onto the stack — nothing here goes anywhere, so there would be nothing to
            // come back from.
            let origin = self.picker.as_ref().and_then(|p| p.origin.clone());
            self.picker = None;
            self.apply_code_action(*action, origin);
            return;
        }
        if let Some(entry) = recover {
            // The rest of the list is taken out before the picker goes, and put back up
            // afterwards. Two sessions' worth of unsaved work is several files, and a chooser
            // that closed on the first Enter would mean restarting CleeCode once per file — for
            // a list it had already built and was about to throw away.
            let rest: Vec<crate::recovery::Entry> = self
                .picker
                .take()
                .map(|p| {
                    p.items
                        .into_iter()
                        .filter_map(|item| match item.action {
                            crate::picker::PickAction::Recover(other) if other.file != entry.file => {
                                Some(*other)
                            }
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            self.restore_recovery(*entry);
            self.open_recovery_picker(rest);
            return;
        }
        if let Some(name) = inspect {
            self.picker = None;
            self.inspect(name);
            return;
        }
        if let Some((path, line, col)) = file_line {
            // A list the server filled was asked for from somewhere, and choosing a row is a
            // jump like any other — so the place it was asked from goes on the stack, and the
            // key that comes back from a definition comes back from here too. Written down
            // before the jump, so what is remembered is where the key was pressed.
            let origin = self.picker.as_ref().and_then(|p| p.origin.clone());
            self.picker = None;
            if let Some(origin) = origin {
                self.jumps.push(origin);
            }
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
        let path = self.editor().path.clone();
        match self.editor_mut().save() {
            Ok(()) => self.status_message = i18n::msg_saved(lang, &self.editor().title(lang)),
            Err(e) => self.status_message = i18n::msg_save_error(lang, &e.to_string()),
        }
        if let Some(path) = path {
            self.reload_keymap_if_settings_were_saved(&path);
        }
    }

    /// Editing settings.toml in the editor and saving it puts the new chords in force straight
    /// away, warnings and all.
    ///
    /// Tied to the save rather than to a file watcher on purpose. A watcher is a thread, a
    /// debounce and a class of surprise — the file changing under a machine that is mid-edit —
    /// in exchange for reacting to a change nobody made from here. The save is the moment the
    /// user finished the sentence, and it is the moment they are looking at the status line.
    ///
    /// Only the chords are reloaded. See `settings::read_keys_from_disk` for why the rest of the
    /// file is left alone.
    fn reload_keymap_if_settings_were_saved(&mut self, saved: &Path) {
        let Some(config) = settings::config_path() else { return };
        // Compared through the filesystem where it can be: settings.toml is very often a symlink
        // into somebody's dotfiles, and the tab would then be holding the path it was opened by
        // rather than the one the config lives at.
        let same = saved == config
            || std::fs::canonicalize(saved)
                .ok()
                .zip(std::fs::canonicalize(&config).ok())
                .is_some_and(|(a, b)| a == b);
        if !same {
            return;
        }
        let lang = self.settings.lang;
        let Some(keys) = settings::read_keys_from_disk() else { return };
        let (keymap, warnings) = crate::keymap::Keymap::build(&keys, lang);
        self.keymap = keymap;
        self.settings.keys = keys;
        self.status_message =
            if warnings.is_empty() { i18n::msg_keys_reloaded(lang) } else { warnings.join("  ") };
    }

    /// Opens settings.toml on the `[keys]` table, writing that table out first when the file has
    /// none.
    ///
    /// The block it seeds is every action commented out on the key it is on today, generated
    /// from the table in `keymap.rs` — so the answer to "what can I remap, and what is it
    /// called" is in the file the user is about to edit, rather than in a manual they would have
    /// to have read. Uncomment one line, change its chord, save.
    ///
    /// It appends rather than rewrites: the file is documented as hand-editable, and a menu
    /// entry that reformatted somebody's own settings on the way past would be a poor trade for
    /// a list of names.
    fn open_keybindings_file(&mut self) {
        let lang = self.settings.lang;
        let Some(path) = settings::config_path() else {
            self.status_message = i18n::msg_keys_no_config_dir(lang);
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        // A `[keys]` header already there means the user has been here; adding a second one
        // would make the file stop parsing altogether, which is the one outcome this must not
        // have.
        let has_section = existing.lines().any(|line| line.trim_start().starts_with("[keys]"));
        if !has_section {
            let mut text = existing;
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&crate::keymap::commented_section(lang));
            if let Err(e) = settings::write_atomic(&path, text.as_bytes()) {
                self.status_message = i18n::msg_save_error(lang, &e.to_string());
                return;
            }
        }
        self.open_file_in_tab(path);
    }

    /// Writes every named dirty buffer. Answers whether all of them landed, so an action that
    /// was waiting on the save — a quit, a tab close — can hold back when one did not.
    fn save_all(&mut self) -> bool {
        let lang = self.settings.lang;
        let mut saved = 0usize;
        let mut unnamed = 0usize;
        let mut errors = Vec::new();
        let mut written: Vec<PathBuf> = Vec::new();
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
                Ok(()) => {
                    saved += 1;
                    written.extend(editor.path.clone());
                }
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
        // Save All reaches settings.toml the same as Save does, so the chords follow it here too
        // — otherwise which of two keys you pressed would decide whether your own setting took.
        // Last, because it has a status message of its own to leave behind.
        for path in written {
            self.reload_keymap_if_settings_were_saved(&path);
        }
        errors.is_empty()
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
            KeyCode::Backspace => pop_grapheme(&mut self.save_as_input),
            KeyCode::Char(c) if is_a_typed_character(key) => self.save_as_input.push(c),
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
        // Save As on a buffer that already had a name leaves the old name behind, and with it a
        // recovery copy keyed to a file this buffer is no longer about. `Editor::save` below
        // clears the copy under the *new* name; this is the only place that still knows the old
        // one. Cleared before the write, since after it the buffer no longer remembers.
        let was = editor.path.take();
        if let Some(was) = &was {
            crate::recovery::forget(Some(was), editor.recovery_id);
        }
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
            // A failed write leaves the quit undone and the reason on screen, for the same
            // reason the prompt exists at all.
            if !self.save_all() {
                return;
            }
        }
        self.perform_unsaved_action(action);
    }

    fn close_editor_at(&mut self, idx: usize) {
        if idx >= self.editors.len() {
            return;
        }
        // A buffer with nothing unsaved in it has nothing left to recover, so its copy goes with
        // it rather than waiting to be offered back at the next start as work that is already on
        // disk. A *dirty* one keeps its copy deliberately: the tab is being closed over unsaved
        // changes, and one Esc at the next start is a cheaper mistake than the other one.
        if !self.editors[idx].dirty {
            crate::recovery::forget(
                self.editors[idx].path.as_deref(),
                self.editors[idx].recovery_id,
            );
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
            // The drawer falls in with the file tree here rather than with the terminal panel:
            // it holds one agent and has no strip of its own, so the only tabs the key can mean
            // are the editor's.
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
        // The executable belonged to the project that was open, so it is left with it. A guess
        // carried into another folder would offer to debug a binary from somewhere else entirely,
        // which is the one wrong answer a filled-in guess can give.
        self.debuggee = None;
        // The sweep that lands next belongs to another repository, and comparing it with this
        // one's would read as "everything in the new project has just been written". Forgotten
        // rather than replaced: the first sweep of a folder has nothing to be a difference from.
        self.follow_seen = None;
        self.follow_queue.clear();
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

    /// Hands the background back to the terminal, or takes it again. Written out at once rather
    /// than at exit: this is reached for when the screen has become unreadable, and having to do
    /// it again after every session would be its own small misery.
    ///
    /// The only way back to a translucent editor, now that every theme arrives with its own
    /// surface — and the next theme chosen takes it back, which is what `set_theme` is for.
    fn toggle_transparent_background(&mut self) {
        // A theme that paints its own surface is painting it either way, so the switch has
        // nothing to turn. Refused out loud rather than silently ignored: a control that moves
        // and changes nothing is worse than one that explains itself.
        let Some(next) = self.settings.next_transparent_background(self.theme) else {
            self.status_message =
                i18n::msg_background_owned_by_theme(self.settings.lang, self.theme.name());
            return;
        };
        self.settings.transparent_background = next;
        self.settings.save();
        self.status_message = i18n::msg_transparent_background(self.settings.lang, next);
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
            KeyCode::Backspace => pop_grapheme(self.terminal_field_mut()),
            KeyCode::Char(c) if is_a_typed_character(key) => self.terminal_field_mut().push(c),
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
            KeyCode::Backspace => pop_grapheme(&mut self.workspace_save_input),
            KeyCode::Char(c) if is_a_typed_character(key) => self.workspace_save_input.push(c),
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
            debuggee: self.debuggee.clone(),
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
            // Only what the drawer *is*, never what is in it. A workspace can say "open, this
            // wide, on codex"; it cannot say "and here is the conversation", because the
            // conversation is a running process and a TOML file is not where one of those goes.
            drawer: self.drawer.as_ref().map(|drawer| crate::workspace::WorkspaceDrawer {
                open: drawer.open,
                width: self.settings.drawer_pct,
                agent: drawer.agent.map(|a| a.workspace_name().to_string()),
            }),
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
        // Before the files are opened and before anything is run: the workspace is where this
        // project's answer to "what do I debug" lives, and a session started right after opening
        // one should find it already filled in.
        self.debuggee = ws.debuggee.clone();

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

        self.apply_workspace_drawer(ws.drawer.as_ref());

        self.rebuild_terminals(&ws, same_root);
        // Which shell you were looking at is part of the layout too, and it was being written to
        // the file and then ignored on the way back in.
        self.active_terminal = ws.active_terminal.min(self.terminals.len().saturating_sub(1));
        self.active_workspace = Some(name.clone());
        self.settings.last_workspace = Some(name.clone());
        self.status_message = i18n::msg_workspace_loaded(lang, &name);
    }

    /// What a workspace is allowed to say about the drawer.
    ///
    /// **A live drawer's pane is never rebuilt.** That is the promise the whole design is
    /// arranged around: `rebuild_terminals` below drains and replaces every terminal window on
    /// every workspace switch, and the drawer sits outside that vector precisely so an agent you
    /// are mid-conversation with survives opening another project. So the workspace governs the
    /// column — open or closed, and how wide — and nothing else.
    ///
    /// It may still *summon* one: a workspace saved with an agent in the drawer, applied in a
    /// session that has never opened one, starts that agent. Nothing is being replaced there,
    /// because there was nothing to replace.
    fn apply_workspace_drawer(&mut self, saved: Option<&crate::workspace::WorkspaceDrawer>) {
        let have_one = self.drawer.is_some();
        match drawer_from_workspace(saved, have_one) {
            DrawerFromWorkspace::LeaveAlone => {}
            DrawerFromWorkspace::SetOpen { open, width } => {
                self.settings.drawer_pct = width;
                self.settings.clamp_layout();
                if let Some(drawer) = self.drawer.as_mut() {
                    drawer.open = open;
                }
                if !open && self.focus == Focus::Drawer {
                    self.focus = Focus::Editor;
                }
            }
            DrawerFromWorkspace::Summon { agent, width } => {
                self.settings.drawer_pct = width;
                self.settings.clamp_layout();
                self.drawer = Some(crate::drawer::Drawer::with_launcher(agent));
                if let Some(agent) = agent {
                    self.launch_drawer_agent(agent);
                }
            }
        }
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
        let lang = self.settings.lang;
        let (title, kind) = if delete {
            (i18n::t(lang, Key::PickerWorkspaceDelete), crate::picker::PickerKind::WorkspaceDelete)
        } else {
            (i18n::t(lang, Key::PickerWorkspaceOpen), crate::picker::PickerKind::Workspaces)
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
        self.place_rendered_preview(source, pane);
    }

    /// The rendered tab itself, in the pane it is named for.
    ///
    /// Split out from the decision above because the MCP `preview` request wants the same tab in a
    /// pane chosen by a different rule — the half the user is *not* working in, which is already
    /// the half that code is standing in when it asks. Two callers, one way of building the tab.
    fn place_rendered_preview(&mut self, source: PathBuf, pane: EditorPane) {
        let lang = self.settings.lang;
        // The tab is *for* the source file and is a view *of* it: same path both times.
        let mut preview = crate::preview::Preview::rendered(source.clone());
        preview.inverted = self.settings.preview_dark_markdown;
        preview.set_text_only(self.settings.preview_markdown_text);
        let idx = self.adopt_editor(Editor::preview(source, preview));
        self.place_in_pane(pane, idx);
        // Says which of the two renderings you got — the tab's own answer, not the machine's
        // ability: with `preview_markdown_text` on, the machine may well be able to typeset a
        // document while the tab was asked for styled text, and a sentence built from ability
        // alone claims a document over text (the demo recording caught it doing exactly that).
        let text_asked = self.settings.preview_markdown_text;
        self.status_message = i18n::msg_markdown_preview(
            lang,
            crate::preview::markdown_as_document() && !text_asked,
            text_asked,
        );
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
                let lines = crate::preview::render_markdown(&text, self.palette());
                if let Some(preview) = self.editors[i].preview.as_mut() {
                    preview.state = crate::preview::State::Rendered { lines, revision };
                    preview.shown_revision = revision;
                }
                self.redraw = true;
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
                let lines = crate::preview::render_markdown(&text, self.palette());
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
            // Either the interim text went up or the pane went to "rendering": both are a
            // different pane from the one drawn last frame.
            self.redraw = true;
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
    /// Whether the figure in tab `idx` still has an interpreter behind it.
    ///
    /// Its six buttons do not touch the picture: they send a command to the session that drew it,
    /// which redraws and writes a new PNG. With that session gone — a figure from `Run`, whose
    /// shell ends with the script, or a prompt closed since — the command has nowhere to go, and
    /// the bar used to go on offering the buttons as if it did. Reported as "clicking the
    /// controls does nothing": the refusal was only ever a line in the status bar, which is the
    /// easiest thing on screen to miss.
    pub fn figure_has_a_session(&self, idx: usize) -> bool {
        let path = self.editors.get(idx).and_then(|e| e.path.clone());
        let Some((_, language)) = self.figure_for(path.as_deref()) else { return false };
        let pids: Vec<Option<u32>> =
            self.terminals.iter().map(|w| w.active_tab().child_pid()).collect();
        dnd::shell_running(language, &pids).is_some()
    }

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

    // ---- The agent drawer -------------------------------------------------------------------

    /// Opens the drawer, summoning one into being if this session has not had one yet, and puts
    /// the keyboard in it.
    ///
    /// Summoned on the launcher, with the agent you used last already highlighted — which is the
    /// whole of "the last one is remembered". Reopening a drawer that was merely hidden shows
    /// whatever was in it, because hiding never touched it.
    fn open_drawer(&mut self) {
        let remembered = crate::session::Agent::of_program(&self.settings.drawer_agent);
        // The launcher is about to say which of the four are installed, and the last time it said
        // so may have been before the user went and installed one at its invitation. Opening the
        // panel is the moment to ask again — see `drawer::installed`.
        crate::drawer::forget_installed();
        let drawer = self
            .drawer
            .get_or_insert_with(|| crate::drawer::Drawer::with_launcher(remembered));
        drawer.open = true;
        self.focus = Focus::Drawer;
    }

    /// The ribbon's click: the mouse's half of `Ctrl+Shift+A`, and only the summoning half.
    ///
    /// Exactly [`Self::open_drawer`], which is what that chord does when it has nobody to hand
    /// anything to — the launcher when no agent has been started, the running agent's pane when
    /// one has. Deliberately *not* `send_context_to_agent`: a click on the edge of the window is
    /// not a sentence anybody has started, and putting a file reference at an agent's prompt
    /// because a hand brushed the ribbon would be the one thing that key is careful never to do.
    fn summon_drawer_from_ribbon(&mut self) {
        let lang = self.settings.lang;
        self.open_drawer();
        self.status_message = i18n::msg_drawer_toggled(lang, true);
    }

    /// Puts the drawer away when the focus has left it and the mode is autocollapse.
    ///
    /// Called once after every event rather than at each of the dozen places `self.focus` is
    /// assigned — an arrow out, `Ctrl+Tab`, `Esc`, a click that landed on the editor, a menu
    /// item that moved the keyboard somewhere. Those are not one code path and they never will
    /// be, so the rule is applied where they all end instead: whatever just happened, this is
    /// what the screen owes. The state is real and not derived, so `open` goes on meaning "on
    /// screen" for everything that reads it — the layout, the focus ring, the workspace file.
    ///
    /// The pty is not touched, here or anywhere else the drawer closes. See
    /// [`crate::drawer::stays_open`] for why the focus is the signal.
    pub fn settle_drawer(&mut self) {
        if crate::drawer::stays_open(self.settings.drawer_pinned, self.focus == Focus::Drawer) {
            return;
        }
        if let Some(drawer) = self.drawer.as_mut() {
            drawer.open = false;
        }
    }

    /// Hides the drawer's column.
    ///
    /// The `Drawer` itself stays, pty and all. This is the same bargain the terminal panel makes
    /// under `Ctrl+J`, and it is the reason putting the drawer away is a cheap thing to do: what
    /// you are dismissing is a column of the screen, not a conversation.
    fn close_drawer(&mut self) {
        if let Some(drawer) = self.drawer.as_mut() {
            drawer.open = false;
        }
        // The keyboard cannot stay in a frame that is no longer drawn.
        if self.focus == Focus::Drawer {
            self.focus = Focus::Editor;
        }
    }

    fn toggle_drawer(&mut self) {
        let lang = self.settings.lang;
        if self.drawer_is_open() {
            self.close_drawer();
        } else {
            self.open_drawer();
        }
        self.status_message = i18n::msg_drawer_toggled(lang, self.drawer_is_open());
    }

    /// Starts `agent` in the drawer, replacing the launcher with its pane.
    ///
    /// Three details carry the design. The pane is spawned exactly like every other one — a
    /// shell, with the command held until it is at a prompt — so it inherits `CLEE_SESSION` from
    /// `with_startup` and the MCP server an agent starts in here is joined to *this* CleeCode by
    /// descent, for free. And the command is typed with `exec` in front of it, so the shell
    /// *becomes* the agent rather than waiting behind it: when the agent ends, the pane ends, and
    /// the drawer goes back to the launcher. Without it the agent would exit onto a shell prompt
    /// sitting in an agent-shaped panel, which is the one thing this panel must never show.
    /// `exec` that fails leaves an interactive shell exactly where it was, so "command not found"
    /// is still said out loud by the shell rather than guessed at by us.
    ///
    /// The third is what makes the descent worth anything: the line and the pane's own
    /// environment come from [`crate::mcp::drawer_launch`], which registers `clee --mcp` with the
    /// agent it is about to start — a flag for two of them, a name in the environment for the
    /// other two — so an agent launched here can ask what is open and where the cursor is without
    /// anybody having configured it first. `exec` is also what makes that the careful part: a
    /// flag an installed version does not know would print a usage message into a pane that has
    /// already given up its shell, so anything unproven falls back to the bare line above.
    fn launch_drawer_agent(&mut self, agent: crate::session::Agent) {
        let lang = self.settings.lang;
        let command = agent.workspace_name();
        let root = self.root.clone();
        let launch = crate::mcp::drawer_launch(agent, self.settings.agent_mcp);
        match TerminalPanel::with_startup_env(24, 80, &root, Some(&launch.line), &launch.env) {
            Ok(mut panel) => {
                panel.name = Some(agent.label().to_string());
                // The command as the rest of the app has to read it: `Agent::of_command` is one
                // of the two ways a pane is recognised as an agent's, and it reads the first
                // word. `exec claude` would name a program called `exec`.
                panel.startup_command = Some(command.to_string());
                let window = TerminalWindow {
                    tabs: vec![panel],
                    active: 0,
                    weight: crate::terminal_panel::TERMINAL_WEIGHT_DEFAULT,
                };
                if let Some(drawer) = self.drawer.as_mut() {
                    drawer.window = Some(window);
                    drawer.agent = Some(agent);
                    drawer.selected = agent.index();
                }
                // Written now rather than at exit, for the same reason the workspace is: a crash
                // must not cost the one thing the launcher remembers.
                self.settings.drawer_agent = command.to_string();
                self.settings.save();
                self.status_message = i18n::msg_drawer_started(lang, agent.label());
            }
            Err(e) => {
                self.status_message =
                    i18n::msg_drawer_start_error(lang, agent.label(), &e.to_string())
            }
        }
    }

    /// The launcher's one gesture, whichever hand makes it: Enter on the highlighted row, or a
    /// click on it.
    ///
    /// A name that is here starts. A name that is *not* here used to do nothing at all — the row
    /// was drawn dim and said so, and then swallowed the press, which is the worst thing a control
    /// can do: it looks broken rather than honest. So it now does the one useful thing there is to
    /// do about a missing program, and types its install command at a shell prompt.
    ///
    /// **Typed, never submitted.** The same rule as `Ctrl+Shift+A`, and here it is not a nicety:
    /// two of the four install with a script downloaded and piped into a shell, and an editor that
    /// pressed Enter on that line on somebody's behalf would be running a remote script because a
    /// mouse landed on a row. The line goes to the prompt, the user reads it, and Enter is theirs.
    ///
    /// Into a *shell*, not into the drawer: the drawer's pane is the agent's home and the agent is
    /// the thing that is missing. `a_shell_to_type_into` picks a free one or opens one, which is
    /// the same machinery Run and the git remotes use, with the same reason — a command typed
    /// where it cannot run is worse than a pane nobody asked for.
    fn choose_drawer_agent(&mut self, agent: crate::session::Agent) {
        if crate::drawer::installed(agent) {
            self.launch_drawer_agent(agent);
            return;
        }
        let lang = self.settings.lang;
        let command = crate::drawer::install_command(agent);
        // Whatever happens next, the answer to "is it installed?" is about to be worth asking
        // again — including when no shell could be found, since the user may well go and install
        // it in a terminal of their own.
        crate::drawer::forget_installed();
        let Some(at) = self.a_shell_to_type_into() else {
            self.status_message = crate::drawer::msg_install_no_shell(lang, agent.label(), command);
            return;
        };
        if let Some(term) = self.window_tab_mut(at) {
            term.type_line(command);
        }
        self.active_terminal = at;
        self.settings.show_terminal = true;
        // The keyboard follows the line: the next keystroke that matters is the Enter this
        // deliberately did not press, and it has to land where the command is.
        self.focus = Focus::Terminal;
        self.status_message = crate::drawer::msg_install_typed(lang, agent.label(), command);
    }

    /// Keys while the drawer has the keyboard.
    ///
    /// One focus, two keyboards, and which one is in force is simply what is drawn: the launcher
    /// answers to up, down and Enter, while a running agent gets everything, byte for byte, the
    /// way a terminal pane does.
    fn handle_drawer_key(&mut self, key: KeyEvent) {
        if self.drawer.as_ref().is_none_or(|d| !d.showing_launcher()) {
            self.handle_drawer_agent_key(key);
            return;
        }
        match key.code {
            KeyCode::Up => {
                if let Some(drawer) = self.drawer.as_mut() {
                    drawer.move_selection(-1);
                }
            }
            KeyCode::Down => {
                if let Some(drawer) = self.drawer.as_mut() {
                    drawer.move_selection(1);
                }
            }
            KeyCode::Enter => {
                if let Some(agent) = self.drawer.as_ref().map(|d| d.highlighted()) {
                    self.choose_drawer_agent(agent);
                }
            }
            // The keyboard goes back to the editor, and what the column does about it is the
            // mode's business, not this key's: pinned, it stays exactly where it is; on
            // autocollapse, `settle_drawer` finds the focus gone and puts it away.
            KeyCode::Esc => self.focus = Focus::Editor,
            _ => {}
        }
    }

    /// Keys into the drawer's running agent — the terminal panel's own rules, on the one pane
    /// that is not in the terminal panel. Shift+arrows select inside the pane, Shift+PageUp and
    /// PageDown walk the history, Esc drops a selection, and everything else is the agent's.
    fn handle_drawer_agent_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::SHIFT) {
            let step = match key.code {
                KeyCode::Left => Some((0, -1)),
                KeyCode::Right => Some((0, 1)),
                KeyCode::Up => Some((-1, 0)),
                KeyCode::Down => Some((1, 0)),
                _ => None,
            };
            if let Some((d_row, d_col)) = step {
                let text = self.drawer_panel_mut().and_then(|term| {
                    extend_pane_selection(term, d_row, d_col);
                    term.selection_text()
                });
                self.copy_selection_text(text);
                return;
            }
            let page = self.drawer_panel().map(|t| (t.rows.saturating_sub(1)).max(1) as isize);
            let paged = match key.code {
                KeyCode::PageUp => page.map(|p| -p),
                KeyCode::PageDown => page,
                _ => None,
            };
            if let Some(delta) = paged {
                let bytes = key_to_bytes(key);
                if let Some(term) = self.drawer_panel_mut() {
                    if term.alternate_screen() {
                        // A full-screen program — which every one of the four is — has no
                        // history of ours to page through and its own to page through instead.
                        term.write_input(&bytes);
                    } else {
                        term.scroll_by(delta);
                    }
                }
                return;
            }
        }
        if key.code == KeyCode::Esc && self.drawer_panel().is_some_and(|t| t.selection.is_some()) {
            if let Some(term) = self.drawer_panel_mut() {
                term.clear_selection();
            }
            return;
        }
        let bytes = key_to_bytes(key);
        if !bytes.is_empty()
            && let Some(term) = self.drawer_panel_mut()
        {
            // Typing snaps back to the live output, for the same reason a terminal pane does it:
            // the agent is about to answer, and an answer that lands off-screen is worse than
            // losing your place in the history.
            term.scroll_to_bottom();
            term.write_input(&bytes);
        }
    }

    // ---- Handing the editor's context to an agent -------------------------------------------

    /// Which on-screen terminal holds a coding agent, and which agent it is.
    ///
    /// The same shape as the question `send_to_session` asks about an interpreter, and the same
    /// answer to "which tabs count": the on-screen one of each window. Sending text into a
    /// hidden tab would put it at a prompt nobody is looking at, which is the one outcome this
    /// feature must not have — the text is a question the user is about to press Enter on.
    ///
    /// Two ways of recognising one, in this order. The process table is the truthful answer: an
    /// agent that has since exited leaves a shell behind, and the shell is not an agent. It reads
    /// each process by name and by the arguments it was started with, so an npm-installed
    /// `claude` — which runs as `node` with the script as its argument — is found there too.
    ///
    /// The pane's own startup command is the fallback, for the pane that was opened by a preset
    /// to run an agent and says so even where the table has gone quiet.
    ///
    /// The drawer is asked first, and it is asked exactly the same two questions: it runs the
    /// real CLI in a real pty, so the process table is as honest about it as about any other
    /// pane. Precedence rather than a separate feature — the drawer is the panel that exists to
    /// hold an agent, so an agent in it is the one you meant, even with another agent sitting at
    /// a prompt in an ordinary terminal.
    fn agent_pane(&self) -> Option<(AgentPane, crate::session::Agent)> {
        // One process-table snapshot for every candidate at once — the drawer's pane in front,
        // then the panel's windows. Reading the table is the expensive half of this question and
        // it happens on a keystroke, so asking it twice to keep two lists apart would be paying
        // for tidiness.
        let mut pids: Vec<Option<u32>> = vec![self.drawer_panel().and_then(|t| t.child_pid())];
        pids.extend(self.terminals.iter().map(|w| w.active_tab().child_pid()));
        let running = dnd::agent_running(&pids);
        let drawer_running = running.filter(|(i, _)| *i == 0).map(|(_, agent)| agent);
        let terminal_running = running.filter(|(i, _)| *i > 0).map(|(i, agent)| (i - 1, agent));
        let drawer_declared = self
            .drawer_panel()
            .and_then(|t| t.startup_command.as_deref())
            .and_then(crate::session::Agent::of_command);
        let terminal_declared = self.terminals.iter().enumerate().find_map(|(index, window)| {
            let command = window.active_tab().startup_command.as_deref()?;
            crate::session::Agent::of_command(command).map(|agent| (index, agent))
        });
        agent_precedence(drawer_running, drawer_declared, terminal_running, terminal_declared)
    }

    /// The path as it should be written at an agent's prompt: relative to the project root where
    /// the file is inside it, absolute where it is not. An agent is running in that root, so the
    /// short form is the one it can act on — and the one it prints back.
    fn path_for_agent(&self, path: &Path) -> String {
        path.strip_prefix(&self.root).unwrap_or(path).to_string_lossy().to_string()
    }

    /// What the editor has to say about where you are, in order of precedence: the selection you
    /// made, then a diagnostic the language server put under the cursor, then the cursor itself.
    ///
    /// Line numbers come out one-based, which is what the gutter shows and what `file:12` means
    /// to everyone who reads it — the agent included.
    fn agent_context(&self) -> Option<(PathBuf, crate::session::Context)> {
        use crate::session::Context;
        let editor = self.editor();
        // No path, nothing to point at: a reference to a buffer that is not a file is a
        // reference to nothing, and the agent would go looking for it.
        let path = editor.path.clone()?;
        if let Some(((from, _), (to, _))) = editor.selection_range() {
            let text = editor.selected_text().unwrap_or_default();
            return Some((path, Context::Selection { from: from + 1, to: to + 1, text }));
        }
        let (line, col) = (editor.cursor_line, editor.cursor_col);
        // The diagnostics on this line: the one the cursor is actually inside if there is one,
        // otherwise the worst of them. Sitting on a line with an error is the usual way of
        // asking about the error, and severity is the tiebreak for the same reason the status
        // bar uses it — an error is news, a hint is not.
        let on_this_line: Vec<&crate::lsp::Mark> =
            self.marks_for(Some(&path)).iter().filter(|mark| mark.line == line).collect();
        let mark = on_this_line
            .iter()
            .find(|mark| mark.start <= col && col < mark.end)
            .or_else(|| on_this_line.iter().max_by_key(|mark| mark.severity))
            .copied();
        Some(match mark {
            Some(mark) => {
                (path, Context::Diagnostic { line: line + 1, message: mark.message.clone() })
            }
            None => (path, Context::Cursor { line: line + 1 }),
        })
    }

    /// Hands the agent in one of the terminals the piece of context you are looking at.
    ///
    /// **And stops there.** The text lands at the agent's prompt and nothing is submitted: no
    /// newline is sent, ever. That is the discipline the whole feature is built on — what goes
    /// to an agent is a question with a cost, and deciding to ask it is the user's, made by
    /// pressing Enter themselves while looking at what they are about to send. It is also why
    /// this is a paste and not a `type_line`: `type_line` ends with a carriage return, which is
    /// right for an interpreter running a file and wrong for every word of this.
    ///
    /// The focus follows the text, which is the one thing this does on the user's behalf. Enter
    /// is the next key in the sentence they just started, and it has to land in the pane where
    /// the text is rather than in the buffer they were reading.
    pub fn send_context_to_agent(&mut self) {
        let lang = self.settings.lang;
        let Some((pane, agent)) = self.agent_pane() else {
            // Nobody to talk to — so the key summons the panel whose job that is, on its
            // launcher. The one chord this feature has does the whole of the feature: with an
            // agent it hands over the context, without one it gets you an agent. There is no
            // spare `Ctrl+Shift` letter to spend on the second half (`Z` is redo, and binding it
            // would shadow redo silently), and there does not need to be.
            self.open_drawer();
            self.status_message = i18n::msg_drawer_summoned(lang);
            return;
        };
        let Some((path, what)) = self.agent_context() else {
            self.status_message = i18n::msg_agent_unsaved(lang);
            return;
        };
        let name = self.path_for_agent(&path);
        let Some(term) = (match pane {
            AgentPane::Drawer => self.drawer_panel(),
            AgentPane::Terminal(idx) => self.window_tab(idx),
        }) else {
            return;
        };
        // Composed against what the pane can hold, then handed to the same paste path a
        // clipboard goes through — brackets where the program asked for them, nothing where it
        // did not.
        let text = agent.context(&name, &what, term.holds_a_paste());
        let bytes = term.paste_bytes(&text);
        let reference = text.lines().next().unwrap_or_default().to_string();
        // The focus follows the text either way: Enter is the next key in the sentence the user
        // has started, and it has to land where the text is.
        match pane {
            AgentPane::Drawer => {
                if let Some(term) = self.drawer_panel_mut() {
                    term.write_input(&bytes);
                }
                // And the drawer is opened whether or not it was on screen a moment ago. A
                // collapsed one still holds the agent and still wins the precedence above — the
                // conversation never stopped — so without this the text would arrive at a prompt
                // nobody can see, which is the one outcome this feature must not have.
                self.open_drawer();
                self.status_message =
                    i18n::msg_agent_sent_to_drawer(lang, &reference, agent.label());
            }
            AgentPane::Terminal(idx) => {
                if let Some(term) = self.window_tab_mut(idx) {
                    term.write_input(&bytes);
                }
                self.settings.show_terminal = true;
                self.active_terminal = idx;
                self.focus = Focus::Terminal;
                self.status_message =
                    i18n::msg_agent_sent_to_terminal(lang, &reference, agent.label(), idx);
            }
        }
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
                    generation: None,
                    quiet: None,
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
    /// Opens the theme drop-down on the row of the theme in use, so the list opens showing where
    /// you are rather than at the top.
    pub fn open_theme_menu(&mut self) {
        let here = crate::theme::ThemeChoice::all().iter().position(|c| *c == self.settings.theme);
        self.theme_menu = Some(here.unwrap_or(0));
        self.redraw = true;
    }

    fn handle_theme_menu_key(&mut self, key: KeyEvent) {
        let Some(selected) = self.theme_menu else { return };
        let choices = crate::theme::ThemeChoice::all();
        let len = choices.len();
        match key.code {
            KeyCode::Esc => self.theme_menu = None,
            KeyCode::Up => self.theme_menu = Some((selected + len - 1) % len),
            KeyCode::Down => self.theme_menu = Some((selected + 1) % len),
            KeyCode::Enter => {
                self.theme_menu = None;
                if let Some(choice) = choices.get(selected) {
                    self.set_theme(*choice);
                }
            }
            _ => {}
        }
        self.redraw = true;
    }

    /// Changes the colours everything is drawn in.
    ///
    /// Written out at once, like the background it is a cousin of: a theme is chosen because the
    /// screen is unreadable as it stands, and having to choose it again next session would be a
    /// poor answer. The highlighter is rebuilt rather than re-tinted — its syntect theme is what
    /// every coloured line borrows from — and every open buffer is told its colours are stale, so
    /// the next frame recolours the lines it is about to draw and no others.
    ///
    /// `Auto` resolves against what the terminal answered at startup, because the question cannot
    /// be asked again from here: the mouse is captured and the event loop owns stdin, so a query
    /// written now would have its reply read as a keypress. If the theme was fixed at startup
    /// nothing was asked, and choosing `Auto` gives the dark theme now and the right one from the
    /// next launch on. Said out loud rather than silently, which is what the status line is for.
    pub fn set_theme(&mut self, choice: crate::theme::ThemeChoice) {
        // A chosen theme owns its background. Whatever transparency was in force is handed back
        // here, so the incoming theme arrives whole instead of as its colours over the terminal's
        // surface — which was the bug: the switch stayed where the last theme had left it, the
        // frame repainted nothing, and a theme picked to fix an unreadable screen fixed nothing.
        // Answered ahead of the guard below because choosing the theme already in use is still a
        // choice, and it is the natural way to ask for the surface back.
        let reclaimed = self.settings.reclaim_background_for_the_theme();
        if self.settings.theme == choice {
            // Nothing else to change, but a background just reclaimed still has to be written
            // out and drawn.
            if reclaimed {
                self.settings.save();
                self.redraw = true;
            }
            return;
        }
        self.settings.theme = choice;
        self.settings.save();
        let theme = choice.resolve(crate::preview::background());
        self.theme = theme;
        self.highlighter = Highlighter::for_theme(theme);
        for editor in &mut self.editors {
            editor.forget_highlight();
        }
        self.status_message = match choice {
            // Which theme "follow the terminal" turned out to mean is the half worth reading.
            crate::theme::ThemeChoice::Auto => format!("Auto \u{00b7} {}", theme.name()),
            crate::theme::ThemeChoice::Fixed(theme) => theme.name().to_string(),
        };
        self.redraw = true;
    }

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
            KeyCode::Backspace => pop_grapheme(&mut self.run_command_input),
            KeyCode::Char(c) if is_a_typed_character(key) => self.run_command_input.push(c),
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
            i18n::t(self.settings.lang, Key::PickerVenvBrowse),
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
            KeyCode::Backspace => pop_grapheme(&mut self.venv_register_input),
            KeyCode::Char(c) if is_a_typed_character(key) => self.venv_register_input.push(c),
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
        // Left to right across the window, which is the order the frames are in.
        let mut order =
            vec![Focus::FileTree, Focus::Editor, Focus::Terminal, Focus::Debug, Focus::Drawer];
        if !self.settings.show_sidebar {
            order.retain(|f| *f != Focus::FileTree);
        }
        if !self.settings.show_terminal {
            order.retain(|f| *f != Focus::Terminal);
        }
        if !self.debug_panel_is_open() {
            order.retain(|f| *f != Focus::Debug);
        }
        if !self.drawer_is_open() {
            order.retain(|f| *f != Focus::Drawer);
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
            || self.pending_upload.is_some()
            || self.agent_edit_ask.is_some()
            || self.show_save_as
            || self.run_menu.is_some()
            || self.theme_menu.is_some()
            || self.venv_register.is_some()
            || self.run_command_edit.is_some()
            || self.picker.is_some()
            || self.find.is_some()
            || self.show_goto
            || self.debug_prompt.is_some()
            || self.show_search
            || self.show_new_entry
            || self.show_delete_confirm
            || self.show_rename
            || self.symbol_rename.is_some()
            || self.rename_preview.is_some()
            || self.replace_sweep.is_some()
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
        // Before the pane it was pasted into gets the key back: the question is about that pane,
        // and the next thing typed there is the answer to it and not a command.
        if self.pending_upload.is_some() {
            self.handle_upload_prompt_key(key);
            return;
        }
        // Beside the upload question and for the same reason: it is a one-letter answer about
        // something that cannot be taken back, and the next key pressed is the answer to it
        // rather than a command or a character.
        if self.agent_edit_ask.is_some() {
            self.handle_agent_edit_prompt_key(key);
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
        if self.theme_menu.is_some() {
            self.handle_theme_menu_key(key);
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
        if self.debug_prompt.is_some() {
            self.handle_debug_prompt_key(key);
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
        if self.symbol_rename.is_some() {
            self.handle_symbol_rename_key(key);
            return;
        }
        if self.rename_preview.is_some() {
            self.handle_rename_preview_key(key);
            return;
        }
        if self.replace_sweep.is_some() {
            self.handle_replace_sweep_key(key);
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

    /// Runs one of the application layer's actions. Which key it arrived on is the keymap's
    /// business; what the key does is here.
    ///
    /// The comments are the ones that used to sit beside each arm of the dispatch — why a letter
    /// is that letter, and why the ones nobody would guess are the ones that were left over.
    /// They are still worth having now that the letters are only defaults, because a default is
    /// the key almost everybody presses.
    fn run_key_action(&mut self, action: crate::keymap::Action) {
        use crate::keymap::Action;
        match action {
            Action::Manual => self.manual = Some(crate::manual::ManualState::new()),
            Action::Settings => self.show_settings = true,
            Action::RunFile => self.run_active_file(),
            // eXecute this much of it. R runs the file, X runs the piece — next to each other in
            // meaning, and X was one of the letters still free. Not Shift+Enter, which is what
            // every notebook uses and what a terminal cannot deliver: the encoding has had no
            // room for the Shift since VT100, so it would work in two emulators and silently do
            // nothing in the rest. Ctrl+X is still cut; this is Ctrl+Shift+X.
            Action::RunSelection => self.run_selection(),
            // A for agent, and one of the few letters still free — which is luck, because it is
            // the one anybody would have guessed. Next to X in meaning as well as on the
            // keyboard: X sends a piece of the file to an interpreter, A sends where you are to
            // an agent, and neither presses Enter for you.
            Action::SendToAgent => self.send_context_to_agent(),
            // Put a breakpoint on this line, or take it off.
            Action::ToggleBreakpoint => self.toggle_breakpoint(),
            // Inspect: what a variable actually contains, a screenful at a time.
            Action::InspectVariable => self.open_inspector_picker(),
            Action::NewTerminalWindow => self.new_terminal(),
            Action::NewTerminalTab => self.new_terminal_tab(),
            // One key closes the shell you are looking at. It takes the window with it when that
            // was its last tab, so there is nothing to remember about which of the two you meant.
            Action::CloseTerminalTab => self.close_active_terminal_tab(),
            Action::ToggleFold => self.editor_mut().toggle_fold(),
            Action::ResizeMode => self.resize_mode = !self.resize_mode,
            Action::MenuBar => self.menu.open(),
            Action::ContextMenu => self.open_context_menu_for_focus(),
            // J and L, next to each other, for a pair that is used as one: go and come back.
            // Neither is a letter anyone would guess, and neither had a better claim — the
            // mnemonic keys are spent, and F12 is not available here for the reason no feature
            // in CleeCode uses a function key.
            Action::GoToDefinition => self.lsp_go_to_definition(),
            Action::JumpBack => self.lsp_jump_back(),
            // Y and V, the two letters this layer still had. Neither is a mnemonic — R for
            // references is the run key and S for symbols is save all — and neither had a
            // better claim: what makes them findable is that the menu row beside Go to
            // definition prints them, which is the same way J and L are found.
            Action::FindReferences => self.lsp_find_references(),
            Action::DocumentSymbols => self.lsp_document_symbols(),
            // C for change, and the first of the free three: Z is redo by every habit anybody
            // brings here, which leaves Q for the one below. Some Linux terminal emulators bind
            // Ctrl+Shift+C to copy — a trade this project already made with T, N and W, and the
            // one chord in the list a `[keys]` entry is most likely to be moved off.
            Action::RenameSymbol => self.lsp_rename_symbol(),
            // Q, which is what was left, and it is not a bad place: it sits beside Ctrl+Q rather
            // than on top of it, and of the two things a slipped Shift can do here — quit, or
            // lay the file out — one prompts about unsaved work and the other is one Ctrl+Z from
            // undone. The mnemonic keys for this were spent long ago; the menu row prints the
            // chord, which is how J, L, Y and V are found too.
            Action::FormatDocument => self.lsp_format_document(),
            // The Super layer, and the only two defaults on it. Neither the letters nor the
            // Ctrl+Shift arrows had room left — the arrows are tabs and terminal windows already —
            // and this is the one pair of actions in the application that is genuinely pressed
            // several times in a row, so a menu row would not have done. What it costs is written
            // down where readers meet it: the Command key reaches an application only under the
            // kitty keyboard protocol, so in Terminal.app or under a window manager that keeps
            // Super for itself these two arrive nowhere and the menu rows are the way to them.
            Action::ExpandSelection => self.lsp_expand_selection(),
            Action::ShrinkSelection => self.lsp_shrink_selection(),
            Action::GitPanel => self.toggle_git_panel(),
            // H rather than the F that VS Code uses for this: Ctrl+Shift+F already folds, and a
            // key that does two things is a key that does the wrong one.
            Action::FindInProject => self.begin_project_search(),
            // Navigation lives on the arrows: the same physical keys on every layout, and no Fn
            // needed. Ctrl+<direction> moves to the frame that lies that way — sidebar, either
            // half of a split editor, or a tiled terminal window, without caring which kind it
            // is. Ctrl+Shift+←/→ is the one exception, moving between the tabs *inside* the
            // frame you are already in.
            Action::NextTab => self.cycle_focused_tab(true),
            Action::PrevTab => self.cycle_focused_tab(false),
            // Walks the terminal windows without having to know how they happen to be tiled —
            // the spatial arrows do it too, but which one depends on the layout.
            Action::NextTerminal => self.cycle_terminal(true),
            Action::PrevTerminal => self.cycle_terminal(false),
            // Name the focused terminal and give it a startup command; save the workspace under
            // a name; save every dirty buffer. All three used to be Alt chords.
            Action::RenameTerminal => self.start_terminal_rename(),
            Action::SaveWorkspace => self.begin_save_workspace(),
            Action::SaveAll => {
                self.save_all();
            }
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
        // The drawer strikes the same bargain, and it has to: an agent is a full-screen program
        // that reads Ctrl+C to stop what it is doing, and one that could not be interrupted
        // would be an agent you cannot work with. It is only ever reached when an agent is
        // running — the launcher answers to arrows and Enter, which are reserved above.
        if self.focus == Focus::Drawer && ctrl && !reserved && self.drawer_panel().is_some() {
            self.handle_drawer_agent_key(key);
            return;
        }

        // No Alt+<letter> and no Alt+<digit> anywhere in CleeCode. macOS only sends Option as
        // Meta on US keyboard layouts, so on any other one — Italian, German, French — those
        // chords never arrived at all, and a shortcut that silently does nothing is worse than
        // no shortcut. Alt with an *arrow* is a different matter and is still used: an Option
        // arrow produces no printable character, so the terminal forwards it as Meta whatever
        // the layout, which is also why editors have settled on Alt+↑/↓ for moving a line.

        // ---- The application layer: Ctrl+Shift+<letter>, Ctrl+Shift+<arrow> ------------------
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
        //
        // Every word of which is about one keyboard on one operating system, and none of it is
        // a reason anybody else has to live with. So these are the *defaults* now and no longer
        // the law: the table is in `keymap.rs`, and a `[keys]` entry in settings.toml moves any
        // one of them without touching the rest. What used to be two dozen match arms — each
        // naming its letter twice, because a terminal sends `Ctrl+Shift+M` as `m` here and `M`
        // there — is this one question, asked of a table that answers it in one place.
        if let Some(action) = self.keymap.action_for(key) {
            self.run_key_action(action);
            return;
        }

        match key.code {
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
            Focus::Debug => self.handle_debug_panel_key(key),
            Focus::Drawer => self.handle_drawer_key(key),
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
            _ if self.keymap.matches(KeyAction::MenuBar, key) => self.menu.close(),
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
            MenuAction::ToggleDrawer => self.toggle_drawer(),
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
            MenuAction::ConvertLineEndings => {
                let lang = self.settings.lang;
                let ed = self.editor_mut();
                if ed.read_only {
                    self.status_message = i18n::msg_format_read_only(lang).to_string();
                } else {
                    let to_crlf = ed.line_ending == crate::editor::LineEnding::Lf;
                    ed.line_ending = if to_crlf {
                        crate::editor::LineEnding::Crlf
                    } else {
                        crate::editor::LineEnding::Lf
                    };
                    // Marked dirty, not checkpointed: `save` reads this field to decide what to
                    // write, so the flip has to survive to the next save — but a Snapshot holds
                    // text and cursor, not the ending, so a checkpoint here would give Ctrl+Z
                    // nothing of this to undo and make it look like it restored the old ending
                    // when it didn't.
                    ed.dirty = true;
                    self.status_message = i18n::msg_line_endings_converted(lang, to_crlf);
                }
            }
            MenuAction::ToggleMenuBar => self.settings.show_menubar = !self.settings.show_menubar,
            // Not saved here: the settings file is written on the way out, the same as every
            // other switch on this menu.
            MenuAction::ToggleMdToolbar => {
                self.settings.show_md_toolbar = !self.settings.show_md_toolbar
            }
            MenuAction::ToggleFollowAgentEdits => {
                let was = self.settings.follow_agent_edits;
                self.settings.follow_agent_edits = !was;
                self.follow_mode_switched(was);
            }
            MenuAction::MdBold => self.md_format(ui::MdTool::Bold),
            MenuAction::MdItalic => self.md_format(ui::MdTool::Italic),
            MenuAction::MdStrike => self.md_format(ui::MdTool::Strike),
            MenuAction::MdCode => self.md_format(ui::MdTool::Code),
            MenuAction::MdHeading => self.md_format(ui::MdTool::Heading),
            MenuAction::MdBullet => self.md_format(ui::MdTool::Bullet),
            MenuAction::MdNumbered => self.md_format(ui::MdTool::Numbered),
            MenuAction::MdTask => self.md_format(ui::MdTool::Task),
            MenuAction::MdLink => self.md_format(ui::MdTool::Link),
            MenuAction::MdQuote => self.md_format(ui::MdTool::Quote),
            MenuAction::MdFence => self.md_format(ui::MdTool::Fence),
            MenuAction::ToggleTransparentBackground => self.toggle_transparent_background(),
            MenuAction::ShowThemes => self.open_theme_menu(),
            MenuAction::TogglePlotsInTabs => self.toggle_plots_in_tabs(),
            MenuAction::OpenSettings => self.show_settings = true,
            MenuAction::EditKeybindings => self.open_keybindings_file(),
            MenuAction::NewTerminal => self.new_terminal(),
            MenuAction::NewTerminalTab => self.new_terminal_tab(),
            MenuAction::CloseTerminalTab => self.close_active_terminal_tab(),
            MenuAction::RenameTerminal => self.start_terminal_rename(),
            MenuAction::CloseTerminal => self.close_active_terminal(),
            MenuAction::Save => self.save_active_file(),
            // Deliberately available for a named buffer too, to save a copy under a new name.
            MenuAction::SaveAs => self.begin_save_as(self.active_editor, None),
            MenuAction::SaveAll => {
                self.save_all();
            }
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
                            let bytes = term.paste_bytes(&text);
                            term.write_input(&bytes);
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
            MenuAction::SendToAgent => self.send_context_to_agent(),
            MenuAction::ToggleBreakpoint => self.toggle_breakpoint(),
            MenuAction::DebugStart => self.open_debug_start(),
            MenuAction::DebugPanel => self.toggle_debug_panel(),
            MenuAction::DebugStop => self.debug_stop(),
            MenuAction::DebugPause => self.debug_pause(),
            MenuAction::DebugContinue => self.debug_step(DebugVerb::Continue),
            MenuAction::DebugStepOver => self.debug_step(DebugVerb::StepOver),
            MenuAction::DebugStepIn => self.debug_step(DebugVerb::StepIn),
            MenuAction::DebugStepOut => self.debug_step(DebugVerb::StepOut),
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
            MenuAction::ReplaceProject => self.begin_project_replace(),
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
            MenuAction::FindReferences => self.lsp_find_references(),
            MenuAction::DocumentSymbols => self.lsp_document_symbols(),
            MenuAction::RenameSymbol => self.lsp_rename_symbol(),
            MenuAction::FormatDocument => self.lsp_format_document(),
            MenuAction::CodeActions => self.lsp_code_actions(),
            MenuAction::ExpandSelection => self.lsp_expand_selection(),
            MenuAction::ShrinkSelection => self.lsp_shrink_selection(),
            MenuAction::ShowDiagnostics => self.open_diagnostics_picker(),
            MenuAction::NewFile => self.open_new_entry(false),
            MenuAction::NewFolder => self.open_new_entry(true),
            MenuAction::OpenOutside => self.open_outside(),
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
        if key.code == KeyCode::Esc || self.keymap.matches(KeyAction::Manual, key) {
            self.manual = None;
            return;
        }
        let sections = crate::manual::sections(self.settings.lang, &self.keymap);
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
        if self.keymap.matches(KeyAction::ResizeMode, key) {
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
            FocusTarget::Debug => self.focus = Focus::Debug,
            FocusTarget::Drawer => self.focus = Focus::Drawer,
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
            drawer_open: self.drawer_is_open(),
            debug_open: self.debug_panel_is_open(),
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
            drawer_open: self.drawer_is_open(),
            debug_open: self.debug_panel_is_open(),
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
            Some(ResizeCmd::Drawer(d)) => {
                self.settings.drawer_pct = nudge_u16(self.settings.drawer_pct, d);
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
            // The bars ride the *content* frame, below the tab strip and the formatting bar,
            // which is the box the renderer draws them on.
            let (_, _, content) = ui::pane_areas(self, self.pane_editor_index(pane), *pane_rect);
            for axis in [ui::Axis::Vertical, ui::Axis::Horizontal] {
                out.push((ScrollbarId::Editor(pane, axis), content, axis));
            }
        }
        if let Some(terminals) = &areas.terminals {
            for (i, rect) in terminals.iter().enumerate() {
                out.push((ScrollbarId::Terminal(i), *rect, ui::Axis::Vertical));
            }
        }
        // Only when there is an agent in it: the launcher is a list of four names, and a
        // scrollbar down the side of it would be a control for scrolling nothing.
        if let (Some(rect), true) = (ui::drawer_rect(areas), self.drawer_panel().is_some()) {
            // First when it is overlaying, and only then: its bar rides the window's right edge,
            // which on autocollapse is exactly where the editor's own vertical bar is drawn
            // underneath it. The list is walked in order, so the one on top has to come first or
            // the pointer grabs the bar it cannot see.
            let bar = (ScrollbarId::Drawer, rect, ui::Axis::Vertical);
            if areas.drawer_overlay.is_some() {
                out.insert(0, bar);
            } else {
                out.push(bar);
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
            ScrollbarId::Drawer => ui::terminal_scroll_metrics(self.drawer_panel()?),
        }
    }

    /// What a preview's scrollbar describes: the rendered page, the window on it, and how much
    /// of it fits. `None` when the tab is not a preview, or the page fits whole.
    pub fn preview_scroll_view(&self, idx: usize, axis: ui::Axis) -> Option<(usize, usize, usize)> {
        let preview = self.editors.get(idx)?.preview.as_ref()?;
        // A picture shown whole has no window on anything: nothing is off the pane, so a bar
        // would be a control for scrolling a picture that is entirely in front of you.
        if preview.shown_whole() {
            return None;
        }
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

    /// Whether the pointer is resting on the drawer's ribbon, which is when it brightens.
    ///
    /// The scrollbars' rule, one column over: `scrollbar_engaged` is the precedent and this is
    /// the simpler half of it — there is no drag to account for, because the ribbon is a button
    /// and nothing about it follows the pointer.
    pub fn drawer_ribbon_engaged(&self, rect: Rect) -> bool {
        self.pointer.is_some_and(|(col, row)| within(rect, col, row))
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
            ScrollbarId::Drawer => {
                if let Some(term) = self.drawer_panel_mut() {
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
            ScrollbarId::Drawer => {
                if let Some(term) = self.drawer_panel_mut() {
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
        // The drawer's left border, first, because it is the outermost seam: it is the right
        // edge of whatever frame it took its column from, and that frame's own seam check would
        // otherwise claim the same two columns. Overlaid, it took its column from nobody and the
        // border is simply painted across whatever is behind it — the seam still drags, because
        // the width is worth adjusting in both modes and the border you can see is the one the
        // pointer means.
        if let Some(drawer) = ui::drawer_rect(areas) {
            let border_x = drawer.x;
            if row >= drawer.y && row < drawer.y + drawer.height && col + 1 >= border_x && col <= border_x + 1 {
                // Armed, not started: the closing handle is painted on this same column, and
                // which of the two the press meant is decided by whether it moves. See
                // [`DragTarget::DrawerEdgePress`].
                self.dragging = Some(DragTarget::DrawerEdgePress { on_handle: col == border_x });
                return true;
            }
        }
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
                // Skip the tab-bar row (the panes' top row) and the formatting bar under it: a
                // click on either is a click on what is drawn there, not a seam grab, or the
                // split drag would swallow the controls sitting on those rows.
                let idx = self.pane_editor_index(EditorPane::Right);
                let (tab_bar, toolbar, _) = ui::pane_areas(self, idx, *right);
                let controls = tab_bar.height + toolbar.map_or(0, |t| t.height);
                if row >= areas.editor.y + controls
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
                    // The window's right edge, unless something has taken a column off it: the
                    // terminal's percentage is a percentage of what is left after the drawer,
                    // so its seam has to be measured against that same right-hand edge or every
                    // drag comes out scaled by the drawer's width. The ribbon answers the same
                    // question when the drawer is not a column of the layout — it is one cell
                    // rather than a third of the window, but it is a cell the frames do not have
                    // and the arithmetic is the arithmetic.
                    let areas = ui::compute_layout(full, &ui::LayoutParams::from_app(self));
                    let main_right =
                        areas.drawer.or(areas.drawer_ribbon).map_or(full.width, |r| r.x);
                    let main_left = if self.settings.show_sidebar { self.settings.sidebar_width } else { 0 };
                    let main_width = main_right.saturating_sub(main_left).max(1);
                    let term_cols_from_right = main_right.saturating_sub(col.min(main_right));
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
            Some(DragTarget::DrawerWidth) => {
                // Measured from the window's right edge against the main area, which is what the
                // drawer's percentage is a percentage of — the column comes off `main_area`
                // before anything else is placed. See `ui::compute_layout`.
                let main_width = full.width.max(1);
                let cols_from_right = full.width.saturating_sub(col);
                self.settings.drawer_pct =
                    ((cols_from_right as u32 * 100) / main_width as u32) as u16;
                self.settings.clamp_layout();
            }
            Some(DragTarget::TerminalSplit(i)) => self.drag_terminal_split(i, col, row, full),
            // Never seen here: a press on the drawer's edge becomes `DrawerWidth` on the first
            // movement, and movement is the only thing that reaches this function.
            Some(DragTarget::DrawerEdgePress { .. })
            // All handled where the drag happens, against the frame it started in.
            | Some(DragTarget::TextSelection)
            | Some(DragTarget::TerminalSelection(_))
            | Some(DragTarget::TerminalMouse(..))
            | Some(DragTarget::DrawerSelection)
            | Some(DragTarget::DrawerMouse(_))
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
    /// Hands the tree's selection to the desktop — Preview, a browser, whatever the system
    /// opens that kind of file with.
    ///
    /// On the tree's selection and not on the open tab, which is what Rename and Delete beside it
    /// in the same pop-up act on: one pop-up, one subject. The files this is for are the ones
    /// CleeCode can only *show* — a PDF, a picture, a markdown document — and for those the tree
    /// is where you are pointing anyway.
    fn open_outside(&mut self) {
        let lang = self.settings.lang;
        let Some(path) = self.file_tree.selected_path() else { return };
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.status_message = match crate::dnd::open_with_the_desktop(&path) {
            Ok(()) => i18n::msg_opened_outside(lang, &name),
            Err(e) => i18n::msg_open_outside_failed(lang, &name, &e),
        };
    }

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
            KeyCode::Backspace => pop_grapheme(&mut self.rename_input),
            KeyCode::Char(c) if is_a_typed_character(key) => self.rename_input.push(c),
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
        // `rename` clobbers the destination without a word, so a typed name that happens to be
        // another file's would destroy it — and the tree would look like the rename had simply
        // worked. Same refusal as Save As.
        if new_path.exists() {
            self.status_message = i18n::msg_save_as_exists(lang, &new_path.display().to_string());
            return;
        }
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
            _ if self.keymap.matches(KeyAction::Settings, key) => self.show_settings = false,
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
                let followed = self.settings.follow_agent_edits;
                self.settings.activate(self.settings_selected);
                self.settings_changed();
                self.follow_mode_switched(followed);
            }
            KeyCode::Left => {
                let followed = self.settings.follow_agent_edits;
                self.settings.adjust(self.settings_selected, -1);
                self.settings_changed();
                self.follow_mode_switched(followed);
            }
            KeyCode::Right => {
                let followed = self.settings.follow_agent_edits;
                self.settings.adjust(self.settings_selected, 1);
                self.settings_changed();
                self.follow_mode_switched(followed);
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
            // Puts out the lines the last reload lit in the gutter. Every *edit* does this on
            // its own — see `Editor::mark_edited_from` — which leaves the reader who only wants
            // to look, and Esc is what that reader already presses to dismiss things.
            // …and the selection with them, which is what Esc means everywhere else: never mind.
            // A column selection makes that the way out of a mode — while one is up every
            // printable key writes on all of its lines, and the user needs one key that stops it
            // without moving the cursor or reopening the menu that started it.
            KeyCode::Esc => {
                let ed = self.editor_mut();
                ed.forget_arrived_lines();
                ed.clear_selection();
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
        // A popup a trigger opened lives on its anchor rather than on the word under the cursor,
        // and the difference is not a nicety: right after a `.` there is no word to read
        // backwards, and `prefix_at` would find the one before the dot — the single word the list
        // is not about. Forwards from the anchor, an empty run included. Backspacing over the dot
        // puts the cursor behind it and the popup shuts, which is the same rule seen from behind.
        if popup.triggered {
            return crate::complete::prefix_from(&ed.rope, popup.start, ed.cursor_line, ed.cursor_col)
                .is_some_and(|prefix| prefix == popup.prefix);
        }
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
    ///
    /// Three things can happen here and they are tried in that order: an open popup narrows or
    /// closes, a trigger character asks a server what can go in this position, and a word being
    /// typed brings the ordinary list up. The first two are not exclusive, and that is the one
    /// subtle thing in this function. Typing `value.` usually has a popup open on `value` at the
    /// moment the dot lands; the dot closes it — correctly, since the list was about a word that
    /// is now finished — and then the dot is *still* a trigger. Returning early there because a
    /// popup happened to be up would mean the feature worked only when nothing was on screen,
    /// which is the half of the time nobody would notice was missing.
    fn follow_completion(&mut self, key: KeyEvent, ctrl: bool) {
        let idx = self.active_editor_index();
        let Some(ed) = self.editors.get(idx) else { return };
        let here = crate::complete::prefix_at(&ed.rope, ed.cursor_line, ed.cursor_col);
        // Everything read out of the buffer is read here, before the popup is borrowed: the line
        // to the left of the cursor for the trigger, and the anchor's run for a triggered popup.
        let before: String = ed.rope.line(ed.cursor_line).chars().take(ed.cursor_col).collect();
        let cursor = ed.rope.line_to_char(ed.cursor_line) + ed.cursor_col;
        let anchored = self
            .completion
            .as_ref()
            .filter(|popup| popup.triggered)
            .and_then(|popup| {
                crate::complete::prefix_from(&ed.rope, popup.start, ed.cursor_line, ed.cursor_col)
            });
        if let Some(popup) = self.completion.as_mut() {
            let alive = if popup.triggered {
                match &anchored {
                    Some(prefix) if idx == popup.editor => popup.refilter(prefix),
                    _ => false,
                }
            } else {
                match &here {
                    Some((start, prefix)) if *start == popup.start && idx == popup.editor => {
                        popup.refilter(prefix)
                    }
                    _ => false,
                }
            };
            if !alive {
                self.completion = None;
            }
            // Still up means still about this word, and there is nothing else to do. Closed means
            // the key that closed it goes on to be judged on its own below.
            if self.completion.is_some() {
                return;
            }
        }
        if !self.settings.completion {
            return;
        }
        // The question goes out and nothing opens yet: the popup is built out of the answer, in
        // `open_triggered_completion`. With no server for this file the ask does nothing at all,
        // and a `.` in a text file stays a `.`.
        if let Some(trigger) = crate::complete::trigger_at(key.code, ctrl, &before) {
            self.lsp_ask_completion(idx, cursor, Some(trigger));
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
        if offers_buffer_words(ed) {
            index.add_buffer(&ed.rope, Some(ed.cursor_line));
        }
        index.add_keywords(ed.path.as_deref());
        // What the interpreter is holding right now, for a file in a language it speaks. This is
        // the third source the seam was built for in 0.7, and it offers what no buffer can: a
        // name made at the prompt exists nowhere in the file.
        let speaks = ed.path.as_deref().and_then(crate::session::Language::of_path).is_some();
        if speaks {
            index.add_session(&self.session_names());
        }
        // The other tabs count too: a name you are about to write is more often in the file you
        // were just in than nowhere at all.
        //
        // This loop is where the large-file rule earns the most: it runs over *every* open tab
        // on every popup, so one 50 MB file left open in the background would tax completion in
        // every other file in the session — a cost paid where nobody would think to look.
        for (i, other) in self.editors.iter().enumerate() {
            if i != idx && offers_buffer_words(other) {
                index.add_buffer(&other.rope, None);
            }
        }
        self.completion = crate::complete::Popup::open(idx, start, prefix, index.into_candidates());
        // Asked only once the popup is actually up. A question whose answer has nowhere to land
        // is a question not worth putting to a server that has to think about it.
        if self.completion.is_some() {
            self.lsp_ask_completion(idx, start, None);
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

    /// Runs one of the eleven markdown formatting actions on the focused pane's buffer.
    ///
    /// The one door for the bar, the Format menu and the command palette alike, so all three
    /// refuse in the same words — and refuse out loud. A button that does nothing and says
    /// nothing is indistinguishable from a button that is broken, which is why the two cases
    /// are told apart: the wrong kind of file, and the right kind in a place the action has no
    /// meaning (a rectangular selection, or a run of text crossing lines).
    pub fn md_format(&mut self, tool: ui::MdTool) {
        let lang = self.settings.lang;
        let idx = self.pane_editor_index(self.editor_pane_focus);
        if !ui::md_formattable(self, idx) {
            self.status_message = i18n::t(lang, Key::MsgMdOnlyMarkdown).to_string();
            return;
        }
        let placeholder = i18n::t(lang, Key::MdLinkPlaceholder);
        let Some(ed) = self.editors.get_mut(idx) else { return };
        let done = match tool {
            ui::MdTool::Bold => ed.md_toggle_inline("**"),
            ui::MdTool::Italic => ed.md_toggle_inline("*"),
            ui::MdTool::Strike => ed.md_toggle_inline("~~"),
            ui::MdTool::Code => ed.md_toggle_inline("`"),
            ui::MdTool::Heading => ed.md_cycle_heading(),
            ui::MdTool::Bullet => ed.md_toggle_bullet(),
            ui::MdTool::Numbered => ed.md_toggle_numbered(),
            ui::MdTool::Task => ed.md_toggle_task(),
            ui::MdTool::Link => ed.md_insert_link(placeholder),
            ui::MdTool::Quote => ed.md_toggle_quote(),
            ui::MdTool::Fence => ed.md_toggle_fence(),
        };
        if !done {
            self.status_message = i18n::t(lang, Key::MsgMdCantHere).to_string();
        }
        self.redraw = true;
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
        extend_pane_selection(term, d_row, d_col);
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
        let text = self.window_tab(index).and_then(|t| t.selection_text());
        self.copy_selection_text(text);
    }

    /// Puts a pane's selected text on the clipboard and says how much was taken. Silent on a
    /// selection of blank cells, so dragging across empty space doesn't wipe the clipboard — and
    /// shared with the drawer, whose pane is reached by a different route but selects the same
    /// way.
    fn copy_selection_text(&mut self, text: Option<String>) {
        let Some(text) = text else { return };
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

        // The rename preview is a reader as well, drawn on the git panel's own frame. The wheel
        // moves through it and a click puts it away — and a click *inside* it puts it away too,
        // which is the difference from the panel above. A click is not an agreement to write into
        // half a dozen files, so there is nothing in here for one to land on.
        if self.rename_preview.is_some() {
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    if let Some(preview) = self.rename_preview.as_mut() {
                        preview.scroll_by(-3);
                    }
                }
                MouseEventKind::ScrollDown => {
                    if let Some(preview) = self.rename_preview.as_mut() {
                        preview.scroll_by(3);
                    }
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    self.rename_preview = None;
                    self.status_message = i18n::msg_rename_cancelled(self.settings.lang).to_string();
                }
                _ => {}
            }
            return;
        }

        // The sweep's preview, on the same frame and with the same rule: the wheel reads it and
        // any click puts it away. A click is not an agreement to rewrite files on disk.
        if self.replace_sweep.is_some() {
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    if let Some(sweep) = self.replace_sweep.as_mut() {
                        sweep.scroll_by(-3);
                    }
                }
                MouseEventKind::ScrollDown => {
                    if let Some(sweep) = self.replace_sweep.as_mut() {
                        sweep.scroll_by(3);
                    }
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    self.replace_sweep = None;
                    self.status_message =
                        i18n::msg_replace_cancelled(self.settings.lang).to_string();
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
                if self.symbol_rename.is_some() {
                    self.symbol_rename = None;
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
                    self.close_search_box();
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
                if self.theme_menu.is_some() {
                    // Inside the list picks a theme; anywhere else dismisses it, like the menus.
                    // The button itself is outside, so a second click on it closes the list
                    // rather than reopening it on top of itself.
                    match ui::theme_menu_rect(self, full)
                        .map(ui::inner_rect)
                        .filter(|inner| within(*inner, col, row))
                    {
                        Some(inner) => {
                            let picked = (row - inner.y) as usize;
                            self.theme_menu = None;
                            if let Some(choice) = crate::theme::ThemeChoice::all().get(picked) {
                                self.set_theme(*choice);
                            }
                        }
                        None => self.theme_menu = None,
                    }
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
                // An overlaid drawer is asked before anything it is painted over, because what is
                // on top of the screen is on top of the pointer. Everything inside those cells is
                // the drawer's, and its own seam and scrollbar are named rather than looked up in
                // the general lists: the frames underneath are still laid out full width, so
                // their seams and their bars run through exactly these columns and would claim a
                // click meant for a border the user can see.
                if let Some(rect) = areas
                    .drawer_overlay
                    .filter(|r| row >= r.y && row < r.y + r.height && col + 1 >= r.x)
                {
                    if col <= rect.x + 1 {
                        // Armed rather than started, exactly as on the pinned column: this edge
                        // is the seam and the closing handle at once, and the movement is what
                        // says which. See [`DragTarget::DrawerEdgePress`].
                        self.dragging =
                            Some(DragTarget::DrawerEdgePress { on_handle: col == rect.x });
                        return;
                    }
                    if let Some((ScrollbarId::Drawer, part)) = self.scrollbar_at(col, row, areas) {
                        self.apply_scrollbar(ScrollbarId::Drawer, part, areas);
                        if matches!(part, ScrollbarPart::Track { .. }) {
                            self.dragging = Some(DragTarget::Scrollbar(ScrollbarId::Drawer));
                        }
                        return;
                    }
                    if !self.drawer_takes_press(
                        terminal_panel::BUTTON_LEFT,
                        col,
                        row,
                        mouse.modifiers,
                        areas,
                    ) {
                        self.click_drawer(rect, col, row);
                    }
                    return;
                }
                // The ribbon, which is on screen exactly while the drawer is away. Named here
                // rather than left to the walk below, for the same reason the overlaid drawer's
                // own controls are named above: it is the rightmost column of the window, and
                // every test after this one reaches a column *outwards* from the frame it
                // belongs to — a seam grab takes the cell either side of a border, and the
                // border the last frame ends on is the one next door to this.
                //
                // The collision the edge invites is the editor's vertical scrollbar, which rides
                // the right of the frame it is in. It does not arise, and that is the whole
                // reason the ribbon is a carved column rather than a strip painted over the
                // frames: with a column taken out of the main area every frame beside it ends
                // one cell earlier and its bar rides the last column of what it was left, so no
                // cell is owned twice. Asking first only keeps the tolerances off it.
                if ui::drawer_ribbon_rect(areas).is_some_and(|r| within(r, col, row)) {
                    self.summon_drawer_from_ribbon();
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
                // The debug panel's own column, claimed before the frames it was carved out of.
                // Nothing else owns these cells — it is a column of the layout, not an overlay —
                // so this is only about order, not about a conflict.
                if let Some(rect) = areas.debug.filter(|r| within(*r, col, row)) {
                    self.click_debug_panel(rect, col, row);
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
                    let idx = self.pane_editor_index(self.editor_pane_focus);
                    let (tab_bar, toolbar, content) = ui::pane_areas(self, idx, pane_rect);
                    // A preview's controls sit inside its frame, so they are claimed before the
                    // click can reach the picture behind them. The zones, not the buttons: the
                    // gap between two of them belongs to one of them, so a click a column wide
                    // of the mark still lands. See `ui::nav_bar_hit_zones`.
                    if let Some((control, _)) = ui::nav_bar_hit_zones(self, idx, content)
                        .into_iter()
                        .find(|(_, r)| within(*r, col, row))
                    {
                        self.preview_control(control);
                        return;
                    }
                    // The formatting bar owns its whole row, so a click on it never falls
                    // through to placing the cursor in the text below.
                    if let Some(toolbar) = toolbar.filter(|t| within(*t, col, row)) {
                        if let Some((tool, _)) = ui::md_toolbar_hit_zones(toolbar)
                            .into_iter()
                            .find(|(_, r)| within(*r, col, row))
                        {
                            self.md_format(tool);
                        }
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
                // A program that asked for the mouse gets the click, which is the whole point of
                // it having asked. Shift keeps it for us — see `terminal_takes_press`.
                if self.terminal_takes_press(
                    terminal_panel::BUTTON_LEFT,
                    col,
                    row,
                    mouse.modifiers,
                    areas,
                ) || self.drawer_takes_press(
                    terminal_panel::BUTTON_LEFT,
                    col,
                    row,
                    mouse.modifiers,
                    areas,
                ) {
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
                if let Some(rect) = ui::drawer_rect(areas).filter(|r| within(*r, col, row)) {
                    self.click_drawer(rect, col, row);
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
                // Over a pane that asked for the mouse, the right button is the program's too —
                // lazygit and mc both use it — and Shift is still the way to our own menu.
                if self.terminal_takes_press(
                    terminal_panel::BUTTON_RIGHT,
                    col,
                    row,
                    mouse.modifiers,
                    areas,
                ) || self.drawer_takes_press(
                    terminal_panel::BUTTON_RIGHT,
                    col,
                    row,
                    mouse.modifiers,
                    areas,
                ) {
                    return;
                }
                self.open_context_menu_at(col, row, areas);
            }
            MouseEventKind::Down(MouseButton::Middle) => {
                self.terminal_takes_press(
                    terminal_panel::BUTTON_MIDDLE,
                    col,
                    row,
                    mouse.modifiers,
                    areas,
                );
            }
            MouseEventKind::Drag(MouseButton::Left) => match self.dragging {
                Some(DragTarget::Sidebar)
                | Some(DragTarget::TerminalHeight)
                | Some(DragTarget::EditorSplit)
                | Some(DragTarget::DrawerWidth)
                | Some(DragTarget::TerminalSplit(_)) => {
                    self.continue_drag(col, row, full);
                }
                Some(DragTarget::DrawerEdgePress { .. }) => {
                    // The movement is what makes it a drag. From here on it is an ordinary width
                    // drag and the release will find nothing to close.
                    self.dragging = Some(DragTarget::DrawerWidth);
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
                        let idx = self.pane_editor_index(self.editor_pane_focus);
                        let (_, _, content) = ui::pane_areas(self, idx, pane_rect);
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
                Some(DragTarget::TerminalMouse(index, button)) => {
                    self.terminal_mouse_drag(index, button, MouseAction::Drag, col, row, areas);
                }
                Some(DragTarget::DrawerSelection) => {
                    if let Some(rect) = ui::drawer_rect(areas)
                        && let Some(cell) = cell_at(ui::terminal_content_rect(rect), col, row)
                        && let Some(term) = self.drawer_panel_mut()
                    {
                        term.extend_selection(cell);
                    }
                }
                Some(DragTarget::DrawerMouse(button)) => {
                    self.drawer_mouse_drag(button, MouseAction::Drag, col, row, areas);
                }
                None => {}
            },
            MouseEventKind::Up(button) => {
                // Completing a selection puts it on the clipboard straight away: there is no
                // spare key combination in a terminal pane for an explicit copy (Ctrl+C has to
                // reach the shell as an interrupt).
                match self.dragging {
                    Some(DragTarget::TerminalSelection(index))
                        if button == MouseButton::Left =>
                    {
                        self.finish_terminal_selection(index);
                        self.dragging = None;
                    }
                    // The program was told the button went down and has to be told it came back
                    // up, or it stays convinced something is still being held. Which button is
                    // checked, because a right-click while the left one is down is not the left
                    // one being let go of.
                    Some(DragTarget::TerminalMouse(index, held))
                        if held == terminal_panel::mouse_button_code(button) =>
                    {
                        self.terminal_mouse_drag(index, held, MouseAction::Release, col, row, areas);
                        self.dragging = None;
                    }
                    Some(DragTarget::DrawerSelection) if button == MouseButton::Left => {
                        // A drag that never left its cell is a click that focused the pane, not a
                        // one-character selection — the same rule the terminal panel applies.
                        let single = self
                            .drawer_panel()
                            .and_then(|t| t.selection)
                            .is_some_and(|s| s.is_single_cell());
                        if single {
                            if let Some(term) = self.drawer_panel_mut() {
                                term.clear_selection();
                            }
                        } else {
                            let text = self.drawer_panel().and_then(|t| t.selection_text());
                            self.copy_selection_text(text);
                        }
                        self.dragging = None;
                    }
                    Some(DragTarget::DrawerMouse(held))
                        if held == terminal_panel::mouse_button_code(button) =>
                    {
                        self.drawer_mouse_drag(held, MouseAction::Release, col, row, areas);
                        self.dragging = None;
                    }
                    // A press on the drawer's edge that never moved: not a resize, then, but a
                    // click on the handle painted there — and a click on that handle closes the
                    // drawer, the ✕'s own path. The pty goes on running.
                    Some(DragTarget::DrawerEdgePress { on_handle })
                        if button == MouseButton::Left =>
                    {
                        self.dragging = None;
                        if on_handle {
                            let lang = self.settings.lang;
                            self.close_drawer();
                            self.status_message = i18n::msg_drawer_toggled(lang, false);
                        }
                    }
                    _ if button == MouseButton::Left => self.dragging = None,
                    _ => {}
                }
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
        // An overlaid drawer is asked first, because it is painted over frames that still think
        // they own these cells: on autocollapse the editor is laid out full width, so the pane
        // test below would claim every notch dropped on the agent's conversation.
        if let Some(rect) = areas.drawer_overlay.filter(|r| within(*r, col, row)) {
            self.wheel_over_drawer(rect, col, row, delta);
            return;
        }
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
            let pane = if pane_idx == 0 { EditorPane::Left } else { EditorPane::Right };
            let (tab_bar, _, content) = ui::pane_areas(self, self.pane_editor_index(pane), *pane_rect);
            if within(tab_bar, col, row) {
                // Over the tab strip the wheel scrolls tabs sideways, one per notch, rather
                // than scrolling the text underneath.
                let step = if delta < 0 { -1 } else { 1 };
                self.scroll_tabs(pane, step, tab_bar.width);
                return;
            }
            if within(content, col, row) {
                // Scroll whichever pane the pointer is over, independent of focus. Asked through
                // `pane_editor_index` rather than read raw, and taken with `get_mut`: with every
                // tab closed the pane's index still points at a buffer that is no longer there,
                // and a wheel notch over the empty frame must not be a panic.
                let idx = self.pane_editor_index(pane);
                // A rendered preview holds no rope, so its length comes from the lines drawn.
                let rendered = self.rendered_len(idx);
                let Some(editor) = self.editors.get_mut(idx) else { return };
                let max_top = match rendered {
                    Some(len) => len.saturating_sub(1),
                    None => editor.rope.len_lines().saturating_sub(1),
                };
                editor.top_line = if delta < 0 {
                    editor.top_line.saturating_sub((-delta) as usize)
                } else {
                    (editor.top_line + delta as usize).min(max_top)
                };
            }
            return;
        }
        if let Some(term_areas) = &areas.terminals {
            if let Some(i) = term_areas.iter().position(|r| within(*r, col, row)) {
                // Like the editor panes: the wheel acts on what it is over, whether or not that
                // shell has the focus.
                let cell = cell_at(ui::terminal_content_rect(term_areas[i]), col, row);
                self.wheel_over_terminal(i, delta, cell);
                return;
            }
        }
        // The drawer, by the same rule and for the same reason: an agent is a full-screen
        // program that scrolls a view of its own, and a notch dropped on the way would make its
        // conversation unreadable past the height of the pane.
        if let Some(rect) = areas.drawer.filter(|r| within(*r, col, row)) {
            self.wheel_over_drawer(rect, col, row, delta);
        }
    }

    /// One wheel notch inside the drawer, wherever the drawer is.
    fn wheel_over_drawer(&mut self, rect: Rect, col: u16, row: u16, delta: isize) {
        let cell = cell_at(ui::terminal_content_rect(rect), col, row);
        let up = delta < 0;
        let Some(term) = self.drawer_panel_mut() else { return };
        if let Some((row, col)) = cell
            && let Some(report) = term.wheel_report(up, row, col)
        {
            // One notch, one report: how far a notch goes through a scrollback is our idea,
            // and a program that handles the wheel has its own.
            term.write_input(&report);
            return;
        }
        if !term.alternate_screen() {
            term.scroll_by(delta);
        }
    }

    /// Offers a button press to the program running in the pane under the pointer.
    ///
    /// `true` means it took it, and the click is then none of CleeCode's business — no selection
    /// anchor, no context menu. Shift is the way out and is checked first: holding it keeps the
    /// button for our own selection, which is what every terminal emulator does and the only way
    /// to copy text off the screen of a program that has grabbed the mouse.
    ///
    /// The press is remembered as a drag so the movement and the release that follow reach the
    /// same program, as the same button, in the pane it started in.
    fn terminal_takes_press(
        &mut self,
        button: u16,
        col: u16,
        row: u16,
        modifiers: KeyModifiers,
        areas: &ui::Areas,
    ) -> bool {
        if modifiers.contains(KeyModifiers::SHIFT) {
            return false;
        }
        let Some(term_areas) = &areas.terminals else { return false };
        let Some(index) = term_areas.iter().position(|r| within(*r, col, row)) else { return false };
        let Some(cell) = cell_at(ui::terminal_content_rect(term_areas[index]), col, row) else {
            return false;
        };
        if !self.report_terminal_mouse(index, cell, button, MouseAction::Press) {
            return false;
        }
        self.focus = Focus::Terminal;
        self.active_terminal = index;
        self.dragging = Some(DragTarget::TerminalMouse(index, button));
        true
    }

    /// Reports a movement or a release to the pane a press was already handed to. The cell is
    /// taken from that pane whatever the pointer is over now — `cell_at` clamps — because a drag
    /// that wanders outside is still a drag the program is tracking.
    fn terminal_mouse_drag(
        &mut self,
        index: usize,
        button: u16,
        action: MouseAction,
        col: u16,
        row: u16,
        areas: &ui::Areas,
    ) {
        let Some(rect) = areas.terminals.as_ref().and_then(|t| t.get(index)).copied() else {
            return;
        };
        if let Some(cell) = cell_at(ui::terminal_content_rect(rect), col, row) {
            self.report_terminal_mouse(index, cell, button, action);
        }
    }

    /// The drawer's version of [`Self::terminal_takes_press`]: the agent gets the click when it
    /// asked for the mouse, and Shift is the way to keep it for our own selection.
    fn drawer_takes_press(
        &mut self,
        button: u16,
        col: u16,
        row: u16,
        modifiers: KeyModifiers,
        areas: &ui::Areas,
    ) -> bool {
        if modifiers.contains(KeyModifiers::SHIFT) {
            return false;
        }
        let Some(rect) = ui::drawer_rect(areas).filter(|r| within(*r, col, row)) else { return false };
        let Some(cell) = cell_at(ui::terminal_content_rect(rect), col, row) else { return false };
        let Some(term) = self.drawer_panel_mut() else { return false };
        let Some(report) = term.mouse_report(button, MouseAction::Press, cell.0, cell.1) else {
            return false;
        };
        term.write_input(&report);
        self.focus = Focus::Drawer;
        self.dragging = Some(DragTarget::DrawerMouse(button));
        true
    }

    /// Reports a movement or a release to the drawer's agent, at whatever cell the pointer is
    /// over now — `cell_at` clamps, because a drag that wanders outside is still a drag the
    /// program is tracking.
    fn drawer_mouse_drag(
        &mut self,
        button: u16,
        action: MouseAction,
        col: u16,
        row: u16,
        areas: &ui::Areas,
    ) {
        let Some(rect) = ui::drawer_rect(areas) else { return };
        let Some(cell) = cell_at(ui::terminal_content_rect(rect), col, row) else { return };
        let Some(term) = self.drawer_panel_mut() else { return };
        if let Some(report) = term.mouse_report(button, action, cell.0, cell.1) {
            term.write_input(&report);
        }
    }

    /// A left click inside the drawer that the agent did not take.
    ///
    /// On the launcher it is the mouse's Enter: the name under the pointer is chosen — started if
    /// it is installed, offered for installing if it is not, which is [`Self::choose_drawer_agent`]
    /// either way. A click on the gap between two names only takes the focus — the ROADMAP asks
    /// for the names to be clickable, not for the whitespace around them to start something. On a
    /// running agent it anchors a selection, exactly as a click in a terminal pane does.
    fn click_drawer(&mut self, rect: Rect, col: u16, row: u16) {
        // The ✕ on the title bar, before anything about what is inside the frame: it is the one
        // cell of the drawer that is not the drawer's contents. It goes to the View menu's own
        // close — the column is hidden and the pty goes on running — and never to
        // `close_terminal`, which the identical button on every other pane leads to. The
        // resemblance is the point of the control and the reason it has to be claimed here.
        if ui::terminal_close_cell(rect) == Some((col, row)) {
            let lang = self.settings.lang;
            self.close_drawer();
            self.status_message = i18n::msg_drawer_toggled(lang, false);
            return;
        }
        self.focus = Focus::Drawer;
        if self.drawer.as_ref().is_some_and(|d| d.showing_launcher()) {
            // Asked of the same function that drew the list, so a click can never start the
            // agent above the one under the pointer.
            let (_, rows) = ui::drawer_launcher_rows(ui::inner_rect(rect));
            let Some(index) = rows.iter().position(|r| within(*r, col, row)) else { return };
            if let Some(drawer) = self.drawer.as_mut() {
                drawer.selected = index;
            }
            let Some(agent) = self.drawer.as_ref().map(|d| d.highlighted()) else { return };
            self.choose_drawer_agent(agent);
            return;
        }
        if let Some(cell) = cell_at(ui::terminal_content_rect(rect), col, row) {
            if let Some(term) = self.drawer_panel_mut() {
                term.begin_selection(cell);
            }
            self.dragging = Some(DragTarget::DrawerSelection);
        }
    }

    /// Writes one mouse report into pane `index`, if the program there wants that kind of event.
    fn report_terminal_mouse(
        &mut self,
        index: usize,
        cell: (u16, u16),
        button: u16,
        action: MouseAction,
    ) -> bool {
        let Some(term) = self.window_tab_mut(index) else { return false };
        let Some(report) = term.mouse_report(button, action, cell.0, cell.1) else { return false };
        term.write_input(&report);
        true
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
                // The ✕ is the same request as Ctrl+W and takes the same route through the
                // prompt. Straight to `close_editor_at` it threw unsaved edits away on one
                // click — the one click most likely to be a mis-aim at the tab next to it.
                if self.editors.get(editor_idx).map(|e| e.dirty).unwrap_or(false) {
                    self.unsaved_prompt = Some(UnsavedPrompt::CloseTab(editor_idx));
                } else {
                    self.close_editor_at(editor_idx);
                }
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
            // The editor's menu, over the panel's column: copy and paste are what a right-click
            // is for anywhere, and the panel has no actions of its own that a menu could add
            // which its own single letters do not already carry.
            Focus::Debug => (ContextTarget::Editor, areas.debug.unwrap_or(self.last_full)),
            // The same menu a terminal pane gets: what is in the drawer is a terminal, and copy,
            // paste and the rest mean there exactly what they mean in one.
            Focus::Drawer => {
                (ContextTarget::Terminal, ui::drawer_rect(&areas).unwrap_or(self.last_full))
            }
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
        let rect = match self.context_menu.as_ref().map(|m| ui::context_menu_rect(m, lang, &self.keymap, self.last_full)) {
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
        let sections = crate::manual::sections(self.settings.lang, &self.keymap);
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
        let count = crate::manual::sections(self.settings.lang, &self.keymap).len();
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
            self.toggle_transparent_background();
            return;
        }
        let themes = ui::menu_bar_theme_range(self, width);
        if !themes.is_empty() && themes.contains(&col) {
            self.open_theme_menu();
        }
    }

    fn mouse_menu(&mut self, col: u16, row: u16, full: Rect) {
        let dropdown = ui::menu_dropdown_rect(&self.menu, self.settings.lang, &self.keymap, full);
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
            let followed = self.settings.follow_agent_edits;
            self.settings_selected = idx;
            self.settings.activate(idx);
            self.settings_changed();
            self.follow_mode_switched(followed);
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

/// Whether a buffer's own words belong in the completion index.
///
/// A preview holds no text, so it holds no words. A buffer in the declared large-file mode
/// holds far too much: `Index::add_buffer` bounds its scan to a window around the cursor, but
/// the window is four thousand lines *of a rope with millions in it*, walked again on every
/// keystroke past the second letter — over lines nobody has read and words nobody wrote. The
/// popup still opens on the other sources (the language's keywords, what the interpreter is
/// holding), so completion does not vanish in a large file: it stops offering the one source
/// that costs the file's size to produce.
///
/// Split out because it is asked of the active buffer and of every other open tab, and the two
/// answers must be the same rule — and because it is testable, which `open_completion` around
/// it is not without a running app.
fn offers_buffer_words(ed: &Editor) -> bool {
    ed.preview.is_none() && !ed.is_large()
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

    /// The completion index's rule about which open buffers contribute their words: an ordinary
    /// file does, a large one does not — neither as the buffer being typed in nor as one of the
    /// other tabs, which is the expensive half.
    #[test]
    fn a_large_buffer_offers_no_words_to_the_completion_index() {
        let dir = setup_dir("large_completion");
        let small = dir.join("small.txt");
        std::fs::write(&small, "alpha beta\n").unwrap();
        let ed = Editor::open(small).unwrap();
        assert!(offers_buffer_words(&ed));

        let big = dir.join("big.txt");
        std::fs::write(&big, "gamma delta\n".repeat(400)).unwrap();
        let ed = Editor::open_with_limit(big, 1024).unwrap();
        assert!(!offers_buffer_words(&ed), "a large buffer is skipped, active or not");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One buffer holding a line of Rust, and the ladder a server would send about the `x` in it:
    /// the name, the expression, the statement, the function.
    fn walk_fixture() -> (Editor, Vec<(usize, usize)>) {
        let text = "fn main() {\n    let y = x + 1;\n}\n";
        let mut ed = Editor::empty();
        ed.insert_str(text);
        ed.dirty = false;
        // `x` is at char 24, `x + 1` runs to 29, the statement to 30, the function to 33.
        (ed, vec![(24, 25), (24, 29), (16, 30), (0, 33)])
    }

    /// The point of an answer that arrives as a ladder: the server is asked once and every press
    /// after the first is a step taken here, with nothing on the wire.
    #[test]
    fn widening_climbs_one_rung_a_press_and_narrowing_walks_back_down() {
        let (mut ed, spans) = walk_fixture();
        let path = PathBuf::from("main.rs");
        ed.path = Some(path.clone());
        // The caret sits on the `x`, with nothing selected: the first rung wider than an empty
        // selection is the name itself.
        ed.select_char_range(24, 24);
        let here = (24, 24);
        let mut walk =
            SelectionWalk::starting_at(path, ed.revision(), spans.clone(), here).expect("a rung");
        assert_eq!(walk.selected, (24, 25), "the name under the caret");
        ed.select_char_range(24, 25);

        // Out through the expression, the statement and the function, one press each.
        for expected in [(24, 29), (16, 30), (0, 33)] {
            assert!(walk.still_true(&ed), "nothing else has moved the selection");
            assert_eq!(walk.step(1), Step::Moved(expected.0, expected.1));
            ed.select_char_range(expected.0, expected.1);
        }
        // And the top of the ladder is an answer, not another question for the server.
        assert!(walk.still_true(&ed));
        assert_eq!(walk.step(1), Step::Widest);

        // Back in the way it came, rung for rung, down to where the widening started.
        for expected in [(16, 30), (24, 29), (24, 25)] {
            assert_eq!(walk.step(-1), Step::Moved(expected.0, expected.1));
            ed.select_char_range(expected.0, expected.1);
            assert!(walk.still_true(&ed));
        }
        assert_eq!(walk.step(-1), Step::Narrowest);
    }

    /// A selection already several rungs up is grown from where it is, not from the caret: the
    /// rung to stand on is the innermost that *strictly* contains what is on screen.
    #[test]
    fn a_fresh_ladder_stands_on_the_first_rung_wider_than_what_is_selected() {
        let (_, spans) = walk_fixture();
        let path = PathBuf::from("main.rs");
        // With the expression selected, the next thing out is the statement — not the expression
        // again, which is what a `contains` without the "strictly" would have chosen.
        let walk = SelectionWalk::starting_at(path.clone(), 0, spans.clone(), (24, 29)).unwrap();
        assert_eq!(walk.selected, (16, 30));
        // The rungs below are kept, so narrowing can go further in than this expansion began.
        assert_eq!(walk.at, 2);
        // With the whole of what the server can see selected, there is no rung at all — which is
        // the sentence the reader gets rather than a selection that does not move.
        assert!(SelectionWalk::starting_at(path, 0, spans, (0, 33)).is_none());
    }

    /// The walk's whole liveness rule, and the reason it is a check rather than a hook: nothing
    /// that moves a cursor has to remember to clear anything.
    #[test]
    fn anything_else_that_moves_the_selection_ends_the_walk() {
        let (mut ed, spans) = walk_fixture();
        let path = PathBuf::from("main.rs");
        ed.path = Some(path.clone());
        ed.select_char_range(24, 29);
        let walk = SelectionWalk::starting_at(path.clone(), ed.revision(), spans.clone(), (24, 29))
            .unwrap();
        ed.select_char_range(walk.selected.0, walk.selected.1);
        assert!(walk.still_true(&ed));

        // An arrow key: the selection on screen is no longer the one the walk put there.
        let mut moved = Editor::empty();
        std::mem::swap(&mut moved, &mut ed);
        moved.move_right();
        assert!(!walk.still_true(&moved), "a keystroke ends it");

        // A typed character: the offsets still match nothing, and the revision has moved besides.
        let (mut edited, _) = walk_fixture();
        edited.path = Some(path.clone());
        edited.select_char_range(walk.selected.0, walk.selected.1);
        edited.insert_str("z");
        assert!(!walk.still_true(&edited), "an edit ends it");

        // A different file, even with the very same text selected in it.
        let (mut other, _) = walk_fixture();
        other.path = Some(PathBuf::from("other.rs"));
        other.select_char_range(walk.selected.0, walk.selected.1);
        assert!(!walk.still_true(&other), "a file switch ends it");
    }

    /// git is asked in the form that survives a real project: names with spaces, names in other
    /// alphabets, and a `.git` directory that is never a file to open.
    #[test]
    fn the_git_file_list_is_split_on_nul_and_drops_what_is_never_offered() {
        let stdout = b"src/main.rs\0my notes.txt\0citt\xc3\xa0/relazione.tex\0.git/config\0\
                       target/debug/clee\0node_modules/left-pad/index.js\0.env\0";
        let (names, truncated) = git_listed_names(stdout, false, 100);
        assert!(!truncated);
        let shown: Vec<String> = names.iter().map(|p| p.to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(shown, vec!["src/main.rs", "my notes.txt", "città/relazione.tex"]);

        // Asked for the hidden files, the dotfile comes back — but the VCS store and the build
        // outputs never do, whatever was asked.
        let (names, _) = git_listed_names(stdout, true, 100);
        let shown: Vec<String> = names.iter().map(|p| p.to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(shown, vec!["src/main.rs", "my notes.txt", "città/relazione.tex", ".env"]);
    }

    /// Stopping is fine; stopping quietly is not. The flag is what lets "nothing found" be told
    /// apart from "nothing found in the part I got to".
    #[test]
    fn the_git_file_list_says_when_it_stopped_at_the_limit() {
        let stdout = b"a.rs\0b.rs\0c.rs\0";
        let (names, truncated) = git_listed_names(stdout, false, 2);
        assert_eq!(names.len(), 2);
        assert!(truncated, "two of three is not the project");

        let (names, truncated) = git_listed_names(stdout, false, 3);
        assert_eq!(names.len(), 3);
        assert!(!truncated, "all of it is all of it");
    }

    /// A link pointing back up its own tree makes a directory of infinite depth. Following it
    /// exhausts the stack, and a stack overflow is not a panic to be caught: the process aborts,
    /// taking every shell in the window with it. Reaching the end of this test at all is the
    /// property being checked.
    #[test]
    #[cfg(unix)]
    fn the_walk_never_follows_a_link_that_points_back_up_the_tree() {
        let dir = setup_dir("symlink_loop");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {}").unwrap();
        std::os::unix::fs::symlink(&dir, dir.join("src/loop")).unwrap();
        std::os::unix::fs::symlink(dir.join("src/main.rs"), dir.join("alias.rs")).unwrap();

        let mut files = Vec::new();
        let truncated = walk_project_files(&dir, &mut files, false, 0);
        assert!(!truncated, "a small project is not a truncated one");

        let mut names: Vec<String> =
            files.iter().map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().to_string()).collect();
        names.sort();
        // The link to a *file* is still offered — opening it opens what it names — while the
        // link to a directory is not descended into and the file behind it appears once.
        assert_eq!(names, vec!["alias.rs", "src/main.rs"]);

        std::fs::remove_dir_all(&dir).unwrap();
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

    /// The arithmetic an agent's edit stands on. Getting it wrong means either refusing a change
    /// that was perfectly unique or applying one to the wrong half of the file, and the second is
    /// the kind of thing a user only finds out about later.
    #[test]
    fn an_agents_edit_lands_only_where_its_text_sits_exactly_once() {
        let text = "let x = 1;\nlet y = 2;\nlet x = 3;\n";
        assert_eq!(only_match(text, "let y = 2;"), Ok((11, 21)));
        assert_eq!(only_match(text, "nothing like it"), Err(0), "no match is a buffer that moved on");
        assert_eq!(only_match(text, "let x = "), Err(2), "and two is a request to be clearer");
        // Counted in characters, not bytes: the offsets go straight to `replace_char_range`, and
        // a file with an accent above the edit would otherwise be cut mid-letter.
        let accented = "città\nlet x = 1;\n";
        assert_eq!(only_match(accented, "let x = 1;"), Ok((6, 16)));
        // The empty needle matches everywhere, which is not an edit anybody asked for.
        assert_eq!(only_match(text, ""), Err(0));
    }

    /// A range an agent points at is clamped at both ends: the transposition it can make, and the
    /// span so long that highlighting it says nothing at all.
    #[test]
    fn a_highlighted_range_is_a_gesture_and_not_a_whole_file() {
        assert_eq!(agent_span_lines(10, 22), (10, 22));
        assert_eq!(agent_span_lines(10, 4), (10, 10), "an end before the start is one line");
        assert_eq!(agent_span_lines(0, 3), (1, 3), "lines are 1-based on the wire");
        let (from, to) = agent_span_lines(1, 100_000);
        assert_eq!(to - from + 1, AGENT_SPAN_LINES);
    }

    /// What the consent question counts. The two numbers are what tell "change one word" and
    /// "replace the whole function" apart, and an empty side has to be nothing rather than one.
    #[test]
    fn the_size_of_an_edit_is_the_two_numbers_a_diff_would_print() {
        assert_eq!(agent_edit_size("one line", "another line"), (1, 1));
        assert_eq!(agent_edit_size("a\nb\nc", "z"), (1, 3));
        assert_eq!(agent_edit_size("a\nb", ""), (0, 2), "a deletion adds nothing");
        assert_eq!(agent_edit_size("", "a\nb\nc"), (3, 0), "and an insertion removes nothing");
    }

    /// Closing the last tab used to hand the keyboard to the terminal whether or not one was
    /// drawn. With the terminal pane off, the shell is still alive: what you typed at an empty
    /// window went to an invisible command line.
    #[test]
    fn the_empty_window_only_focuses_something_you_can_see() {
        assert_eq!(empty_state_focus(true, true), Focus::FileTree, "the tree first when it is up");
        assert_eq!(empty_state_focus(true, false), Focus::FileTree);
        assert_eq!(empty_state_focus(false, true), Focus::Terminal, "then a terminal that is up");
        // Nothing else on screen: keys go to the empty frame, which does nothing with them.
        assert_eq!(empty_state_focus(false, false), Focus::Editor);
    }

    /// The guess *Debug ▸ Start* fills in, from the sources the design names and in its order.
    ///
    /// The Cargo half is read with the `toml` crate rather than scanned for a line beginning with
    /// `name`, and this fixture is why: a workspace root lists its members by name, and the first
    /// `name =` in the file belongs to one of them. The naive reader debugs the wrong crate on
    /// the first real project it meets.
    #[test]
    fn the_debuggee_guess_reads_cargo_before_it_gives_up() {
        let dir = setup_dir("debuggee_guess");
        // No Cargo.toml at all: the root itself, which is not an executable and is not pretending
        // to be one — it is the "I do not know" the refusal then names.
        assert_eq!(debuggee_for(&dir, None), dir);

        std::fs::write(
            dir.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/other\"]\n\n[package]\nname = \"clee\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        assert_eq!(
            debuggee_for(&dir, None),
            dir.join("target").join("debug").join("clee"),
            "the package's own name, not the first name in the file"
        );

        // An answer somebody gave wins over any guess, and a relative one is read against the
        // project root — which is where they were standing when they wrote it.
        let elsewhere = PathBuf::from("/opt/build/thing");
        assert_eq!(debuggee_for(&dir, Some(&elsewhere)), elsewhere);
        assert_eq!(
            debuggee_for(&dir, Some(Path::new("build/thing"))),
            dir.join("build").join("thing")
        );

        // A Cargo.toml with no package section is a workspace manifest and names nothing to run.
        std::fs::write(dir.join("Cargo.toml"), "[workspace]\nmembers = [\"a\", \"b\"]\n").unwrap();
        assert_eq!(debuggee_for(&dir, None), dir);
        // And a broken one is not a reason to guess at the text: it is a reason to have no guess.
        std::fs::write(dir.join("Cargo.toml"), "[package\nname = ").unwrap();
        assert_eq!(debuggee_for(&dir, None), dir);
    }

    /// What a running adapter is told when the breakpoints change, and the half that bites.
    ///
    /// `setBreakpoints` replaces one file's whole list, so a file that simply stops being
    /// mentioned keeps every breakpoint it had. Taking the last one off a file empties the entry
    /// out of `App::breakpoints` entirely — so without the published set, the gutter clears and
    /// the program still stops there, which is the worst kind of wrong: invisible.
    #[test]
    fn taking_the_last_breakpoint_off_a_file_is_still_news_for_the_adapter() {
        let one = PathBuf::from("/p/src/main.rs");
        let two = PathBuf::from("/p/src/lib.rs");
        let mut current: std::collections::HashMap<PathBuf, std::collections::BTreeSet<usize>> =
            std::collections::HashMap::new();
        current.insert(one.clone(), [12usize, 4].into_iter().collect());

        // Nothing published yet: one file, and its lines in order rather than in whatever order
        // they were typed.
        let published = std::collections::BTreeSet::new();
        assert_eq!(
            breakpoints_to_publish(&published, &current),
            vec![(one.clone(), vec![4, 12])]
        );

        // Both files known to the adapter, and one of them now empty: it is named with an empty
        // list, which is how the adapter is told to forget it.
        let published: std::collections::BTreeSet<PathBuf> =
            [one.clone(), two.clone()].into_iter().collect();
        assert_eq!(
            breakpoints_to_publish(&published, &current),
            vec![(two.clone(), Vec::new()), (one.clone(), vec![4, 12])],
            "sorted by path, so two runs of the same state send the same thing"
        );

        // And with nothing left anywhere, every file the adapter knows is cleared rather than
        // left behind.
        assert_eq!(
            breakpoints_to_publish(&published, &std::collections::HashMap::new()),
            vec![(two, Vec::new()), (one, Vec::new())]
        );
    }

    /// The settings override is a command line, and an empty one is not an adapter — it is the
    /// absence of an answer, which is what lets discovery have its turn.
    #[test]
    fn the_configured_adapter_is_a_command_line_or_it_is_nothing() {
        assert_eq!(configured_adapter(""), None);
        assert_eq!(configured_adapter("   "), None, "whitespace is not a program name");
        let gdb = configured_adapter("gdb -i=dap").expect("a program and its argument");
        assert_eq!(gdb.program, "gdb");
        assert_eq!(gdb.args, vec!["-i=dap".to_string()]);
        assert_eq!(gdb.name(), "gdb");
        // A full path is what somebody with an adapter outside PATH writes, and the name in a
        // sentence is the program rather than the path it was found at.
        let mine = configured_adapter("/opt/llvm/bin/lldb-dap").expect("just a program");
        assert!(mine.args.is_empty());
        assert_eq!(mine.name(), "lldb-dap");
    }

    // ---- The debug panel ------------------------------------------------------------------

    /// A stopped program's panel state, built by hand: two frames, one scope with two locals in
    /// it, and one of those locals with a field of its own that has been asked for.
    ///
    /// Built rather than driven, because driving it would mean an adapter, and no test here
    /// starts one. What is under test is what the panel *decides* from a given state, which is
    /// exactly the seam this state is on either side of.
    fn stopped_panel() -> DebugPanel {
        let frame = |id: i64, name: &str, line: usize| crate::dap::Frame {
            id,
            name: name.to_string(),
            path: Some(PathBuf::from("/p/src/main.rs")),
            line,
            column: 1,
        };
        let variable = |name: &str, value: &str, reference: i64| crate::dap::Variable {
            name: name.to_string(),
            value: value.to_string(),
            type_name: Some("i32".to_string()),
            reference,
        };
        let mut panel = DebugPanel {
            frames: vec![frame(1, "inner", 12), frame(2, "outer", 40)],
            scopes: vec![crate::dap::Scope {
                name: "Locals".to_string(),
                reference: 100,
                expensive: false,
            }],
            ..DebugPanel::default()
        };
        panel.children.insert(100, vec![variable("total", "7", 0), variable("point", "Point", 101)]);
        panel.children.insert(101, vec![variable("x", "3", 0)]);
        panel.expanded.insert(100);
        panel.watches.push(DebugWatch {
            expression: "total * 2".to_string(),
            answer: Some(Ok("14".to_string())),
        });
        panel
    }

    /// The panel's three sections, in the design's order, with the current frame marked.
    #[test]
    fn the_panel_reads_frames_then_variables_then_watches() {
        let rows = debug_panel_rows(&stopped_panel(), true, i18n::Lang::En);
        let words: Vec<&str> = rows
            .iter()
            .filter(|r| r.kind == DebugRowKind::Heading)
            .map(|r| r.label.as_str())
            .collect();
        assert_eq!(words, vec!["Frames", "Variables", "Watches"], "{rows:#?}");

        // The innermost frame is the one everything else is read in, and it is the one marked.
        let frames: Vec<(&str, bool)> = rows
            .iter()
            .filter_map(|r| match r.kind {
                DebugRowKind::Frame { current, .. } => Some((r.label.as_str(), current)),
                _ => None,
            })
            .collect();
        assert_eq!(frames, vec![("inner", true), ("outer", false)]);
        // And it says where it is in the words a narrow column has room for: the file's own name.
        assert!(rows.iter().any(|r| r.value == "main.rs:12"), "{rows:#?}");

        let watch = rows.iter().find(|r| matches!(r.kind, DebugRowKind::Watch { .. })).unwrap();
        assert_eq!((watch.label.as_str(), watch.value.as_str()), ("total * 2", "14"));
        assert!(!watch.failed);
    }

    /// The scope is open one level, and no further: the field inside `point` has been fetched but
    /// nobody opened it, so it is not on screen.
    ///
    /// This is the cap the design asks for, and it is not a nicety — a panel that expanded
    /// everything it was handed would walk a linked list to its end on every step.
    #[test]
    fn the_panel_opens_one_level_and_waits_to_be_asked_for_the_next() {
        let mut panel = stopped_panel();
        let names = |panel: &DebugPanel| -> Vec<(usize, String)> {
            debug_panel_rows(panel, true, i18n::Lang::En)
                .iter()
                .filter(|r| matches!(r.kind, DebugRowKind::Variable { .. }))
                .map(|r| (r.depth, r.label.clone()))
                .collect()
        };
        assert_eq!(
            names(&panel),
            vec![
                (0, "Locals".to_string()),
                (1, "total".to_string()),
                (1, "point".to_string()),
            ],
            "the second level is fetched but not shown until it is opened"
        );

        // Opened: one step further in, and only the one that was opened.
        panel.expanded.insert(101);
        assert_eq!(
            names(&panel),
            vec![
                (0, "Locals".to_string()),
                (1, "total".to_string()),
                (1, "point".to_string()),
                (2, "x".to_string()),
            ]
        );

        // Shut again, and the panel is exactly what it was.
        panel.expanded.remove(&101);
        assert_eq!(names(&panel).len(), 3);
        // And the scope itself closes too, taking everything under it.
        panel.expanded.remove(&100);
        assert_eq!(names(&panel), vec![(0, "Locals".to_string())]);
    }

    /// While the program is moving there is one dim line and nothing else. Every frame, scope and
    /// value the panel holds is an answer about a place the program has left, and leaving them on
    /// screen would be the panel quietly lying about where the program is.
    #[test]
    fn a_running_program_gets_one_line_rather_than_the_last_stops_numbers() {
        let panel = stopped_panel();
        let rows = debug_panel_rows(&panel, false, i18n::Lang::En);
        assert!(rows.iter().all(|r| !r.selectable()), "nothing to select while it runs: {rows:#?}");
        assert!(
            rows.iter().any(|r| r.kind == DebugRowKind::Note && r.label.contains("running")),
            "{rows:#?}"
        );
        assert!(!rows.iter().any(|r| r.label == "inner" || r.label == "total"), "{rows:#?}");
        // Stopped, the same panel is full again: the rows went away, the state did not.
        assert!(debug_panel_rows(&panel, true, i18n::Lang::En).iter().any(|r| r.label == "inner"));
    }

    /// A watch the adapter would not read shows the adapter's own sentence, marked as a refusal
    /// rather than as a value — "there is no variable named x here" is the ordinary answer for a
    /// local that is not in scope in this frame, and reading it as data would be worse than
    /// showing nothing.
    #[test]
    fn a_watch_the_adapter_refused_says_so_in_the_adapters_words() {
        let mut panel = stopped_panel();
        panel.watches.push(DebugWatch {
            expression: "nowhere".to_string(),
            answer: Some(Err("no variable named nowhere".to_string())),
        });
        panel.watches.push(DebugWatch { expression: "pending".to_string(), answer: None });
        let rows = debug_panel_rows(&panel, true, i18n::Lang::En);
        let watches: Vec<(&str, &str, bool)> = rows
            .iter()
            .filter(|r| matches!(r.kind, DebugRowKind::Watch { .. }))
            .map(|r| (r.label.as_str(), r.value.as_str(), r.failed))
            .collect();
        assert_eq!(watches[1], ("nowhere", "no variable named nowhere", true));
        // And one nobody has answered yet says it is waiting rather than showing a stale number.
        assert_eq!(watches[2].0, "pending");
        assert!(watches[2].1.contains("asking"), "{:?}", watches[2]);
        assert!(!watches[2].2, "no answer yet is not a refusal");

        // Which watch `d` takes off is whichever row the cursor is on, and the row carries its
        // own place in the list rather than a position on screen that shifts with the stack.
        let second = rows.iter().position(|r| r.label == "nowhere").unwrap();
        assert_eq!(rows[second].kind, DebugRowKind::Watch { index: 1 });
    }

    /// An empty watch list still says something, because a section with nothing in it and no
    /// hint under it is a section nobody ever finds out how to fill.
    #[test]
    fn an_empty_watch_list_says_which_key_fills_it() {
        let mut panel = stopped_panel();
        panel.watches.clear();
        let rows = debug_panel_rows(&panel, true, i18n::Lang::En);
        let after_heading = rows
            .iter()
            .skip_while(|r| r.label != "Watches")
            .nth(1)
            .expect("something under the heading");
        assert_eq!(after_heading.kind, DebugRowKind::Note);
        assert!(after_heading.label.contains('w'), "{:?}", after_heading.label);
    }

    /// The arrows walk the rows you can act on and step over the captions between them.
    #[test]
    fn the_arrows_step_over_the_captions_and_stop_at_the_ends() {
        let rows = debug_panel_rows(&stopped_panel(), true, i18n::Lang::En);
        let landable: Vec<usize> =
            rows.iter().enumerate().filter(|(_, r)| r.selectable()).map(|(i, _)| i).collect();
        assert!(landable.len() >= 5, "{rows:#?}");

        // From the caption at the very top, down lands on the first frame and not on the caption.
        let mut at = 0;
        at = debug_next_row(&rows, at, 1);
        assert_eq!(at, landable[0]);
        for expected in &landable[1..] {
            at = debug_next_row(&rows, at, 1);
            assert_eq!(at, *expected);
            assert!(rows[at].selectable());
        }
        // At the end it stays rather than wrapping round to the top.
        assert_eq!(debug_next_row(&rows, at, 1), at);
        // And back up the same way, stopping at the first row rather than at the caption over it.
        for expected in landable.iter().rev().skip(1) {
            at = debug_next_row(&rows, at, -1);
            assert_eq!(at, *expected);
        }
        assert_eq!(debug_next_row(&rows, at, -1), at);

        // A panel with nothing to stand on leaves the cursor where it was.
        let running = debug_panel_rows(&stopped_panel(), false, i18n::Lang::En);
        assert_eq!(debug_next_row(&running, 0, 1), 0);
    }

    /// The design's table, and the rule that makes single letters safe: they are only ever asked
    /// about while this one frame holds the keyboard, and a letter carrying a modifier is not one
    /// of them — the chord layer has already had its turn by then, and shadowing a chord that
    /// grows a meaning later is exactly the bug this refusal prevents.
    #[test]
    fn the_panels_letters_are_the_debuggers_own_and_only_bare_ones() {
        let bare = |c: char| debug_panel_key(KeyEvent::from(KeyCode::Char(c)));
        assert_eq!(bare('c'), Some(DebugPanelKey::Verb(DebugVerb::Continue)));
        assert_eq!(bare('n'), Some(DebugPanelKey::Verb(DebugVerb::StepOver)));
        assert_eq!(bare('s'), Some(DebugPanelKey::Verb(DebugVerb::StepIn)));
        assert_eq!(bare('o'), Some(DebugPanelKey::Verb(DebugVerb::StepOut)));
        assert_eq!(bare('x'), Some(DebugPanelKey::Stop));
        assert_eq!(bare('w'), Some(DebugPanelKey::AddWatch));
        assert_eq!(bare('d'), Some(DebugPanelKey::DropWatch));
        assert_eq!(bare('z'), None, "a letter with no verb does nothing rather than something");
        // Shift is still the same letter — a terminal sends it as the capital — so it works.
        assert_eq!(bare('C'), Some(DebugPanelKey::Verb(DebugVerb::Continue)));

        // Every chord goes back to the application layer, `Ctrl+C` above all: a panel that ate it
        // would be a panel you could not copy from.
        for modifier in [KeyModifiers::CONTROL, KeyModifiers::ALT, KeyModifiers::SUPER] {
            for c in ['c', 'n', 's', 'o', 'x', 'w', 'd'] {
                assert_eq!(
                    debug_panel_key(KeyEvent::new(KeyCode::Char(c), modifier)),
                    None,
                    "{modifier:?}+{c} was claimed by the panel"
                );
            }
        }

        // The rows' own keys, which are the same everywhere in this editor.
        assert_eq!(debug_panel_key(KeyEvent::from(KeyCode::Up)), Some(DebugPanelKey::Move(-1)));
        assert_eq!(debug_panel_key(KeyEvent::from(KeyCode::Down)), Some(DebugPanelKey::Move(1)));
        assert_eq!(debug_panel_key(KeyEvent::from(KeyCode::Enter)), Some(DebugPanelKey::Act));
        assert_eq!(debug_panel_key(KeyEvent::from(KeyCode::Esc)), Some(DebugPanelKey::Leave));
    }

    /// The box *Debug ▸ Start debugging* opens is prefilled with the guess and with nothing else.
    ///
    /// That equality is the design's rule made good — *the editor does not guess silently* — and
    /// it is the one thing about the box worth pinning: a prefill that came from somewhere other
    /// than [`debuggee_for`] would be the editor offering one answer and acting on another.
    #[test]
    fn the_start_box_opens_on_exactly_the_guess() {
        let dir = setup_dir("debuggee_prefill");
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"clee\"\n").unwrap();
        assert_eq!(
            debuggee_prefill(&dir, None),
            debuggee_for(&dir, None).to_string_lossy(),
            "the box and the guess have to be the same string"
        );
        assert!(debuggee_prefill(&dir, None).ends_with("target/debug/clee"));
        // An answer already given is what the box offers next time, which is what "remembered"
        // means from in here.
        let mine = PathBuf::from("build/thing");
        assert!(debuggee_prefill(&dir, Some(&mine)).ends_with("build/thing"));
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

    /// The drawer is the rightmost column in both arrangements: Right reaches it from whatever
    /// was rightmost before, Left leaves it for that same frame, and it is not in the way when
    /// it is closed.
    #[test]
    fn the_arrows_reach_the_drawer_and_come_back() {
        use ResizeSide::*;
        // Classic: the drawer is to the right of the editor, and of the terminal strip below it.
        let editor = ResizeLayout { drawer_open: true, ..classic_like() };
        assert_eq!(focus_neighbour(&editor, Right), Some(FocusTarget::Drawer));
        let closed = ResizeLayout { drawer_open: false, ..classic_like() };
        assert_eq!(focus_neighbour(&closed, Right), None, "closed, the window edge is there");

        let strip_end = ResizeLayout {
            focus: Focus::Terminal,
            terminal_count: 2,
            terminal_index: 1,
            drawer_open: true,
            ..classic_like()
        };
        assert_eq!(focus_neighbour(&strip_end, Right), Some(FocusTarget::Drawer));
        let strip_middle = ResizeLayout { terminal_index: 0, ..strip_end };
        assert_eq!(
            focus_neighbour(&strip_middle, Right),
            Some(FocusTarget::Terminal(1)),
            "the next window first: the drawer is past the end of the strip, not beside each pane"
        );

        // Docked right, the terminal column sits between the editor and the drawer, so the
        // editor's Right must still find the terminal and not jump the queue.
        let docked = ResizeLayout { terminal_on_right: true, drawer_open: true, ..classic_like() };
        assert_eq!(focus_neighbour(&docked, Right), Some(FocusTarget::Terminal(0)));
        let from_terminal = ResizeLayout { focus: Focus::Terminal, ..docked };
        assert_eq!(focus_neighbour(&from_terminal, Right), Some(FocusTarget::Drawer));

        // And out again, into whatever the drawer took its column from.
        let in_drawer = ResizeLayout { focus: Focus::Drawer, ..editor };
        assert_eq!(focus_neighbour(&in_drawer, Left), Some(FocusTarget::Editor(EditorPane::Left)));
        assert_eq!(focus_neighbour(&in_drawer, Up), None, "a full-height column has no up");
        assert_eq!(focus_neighbour(&in_drawer, Down), None);
        let in_drawer_docked = ResizeLayout { focus: Focus::Drawer, ..docked };
        assert_eq!(focus_neighbour(&in_drawer_docked, Left), Some(FocusTarget::Terminal(0)));
    }

    /// The fourth seam. It is the window's rightmost, so it is reachable from the drawer itself
    /// and from whichever frame gave up the column — and growing either side shrinks the other.
    #[test]
    fn the_drawer_seam_moves_from_both_sides_of_it() {
        use ResizeSide::*;
        let editor = ResizeLayout { drawer_open: true, ..classic_like() };
        assert_eq!(resize_command(&editor, Right, true), Some(ResizeCmd::Drawer(-DRAWER_STEP)));
        assert_eq!(resize_command(&editor, Right, false), Some(ResizeCmd::Drawer(DRAWER_STEP)));

        let in_drawer = ResizeLayout { focus: Focus::Drawer, ..editor };
        assert_eq!(resize_command(&in_drawer, Left, true), Some(ResizeCmd::Drawer(DRAWER_STEP)));
        assert_eq!(resize_command(&in_drawer, Right, true), None, "its other side is the window");

        // Docked right, the terminal is the frame beside the drawer and the editor's Right is
        // the terminal seam, exactly as it was before the drawer existed.
        let docked = ResizeLayout { terminal_on_right: true, drawer_open: true, ..classic_like() };
        assert_eq!(resize_command(&docked, Right, true), Some(ResizeCmd::Terminal(-TERMINAL_STEP)));
        let from_terminal = ResizeLayout { focus: Focus::Terminal, ..docked };
        assert_eq!(
            resize_command(&from_terminal, Right, true),
            Some(ResizeCmd::Drawer(-DRAWER_STEP))
        );

        // In the classic strip the last window's right edge is the seam; the ones before it are
        // still trading weight with their neighbour.
        let last = ResizeLayout {
            focus: Focus::Terminal,
            terminal_count: 2,
            terminal_index: 1,
            drawer_open: true,
            ..classic_like()
        };
        assert_eq!(resize_command(&last, Right, true), Some(ResizeCmd::Drawer(-DRAWER_STEP)));
        let first = ResizeLayout { terminal_index: 0, ..last };
        assert!(matches!(
            resize_command(&first, Right, true),
            Some(ResizeCmd::TerminalWeight { seam: 0, .. })
        ));

        // Closed, none of this is reachable: the seam is the window's edge again.
        let closed = ResizeLayout { drawer_open: false, ..classic_like() };
        assert_eq!(resize_command(&closed, Right, true), None);
    }

    /// The order `Ctrl+Shift+A` resolves its target in, pinned where it can be read: the drawer
    /// first, and within each place a running process before a declared startup command.
    #[test]
    fn the_drawer_is_asked_before_the_terminals() {
        use crate::session::Agent;
        // The case the drawer exists for: an agent in it, another one at a prompt in a terminal.
        // The drawer wins, and it wins whichever way each of them was recognised.
        assert_eq!(
            agent_precedence(Some(Agent::Claude), None, Some((2, Agent::Codex)), None),
            Some((AgentPane::Drawer, Agent::Claude))
        );
        assert_eq!(
            agent_precedence(None, Some(Agent::Claude), Some((2, Agent::Codex)), None),
            Some((AgentPane::Drawer, Agent::Claude)),
            "a drawer known only by its startup command still beats a terminal"
        );
        // With nothing in the drawer, the terminals answer exactly as they did before it existed.
        assert_eq!(
            agent_precedence(None, None, Some((1, Agent::Codex)), Some((0, Agent::Gemini))),
            Some((AgentPane::Terminal(1), Agent::Codex)),
            "a running process beats a pane that merely says it was opened for one"
        );
        assert_eq!(
            agent_precedence(None, None, None, Some((0, Agent::Gemini))),
            Some((AgentPane::Terminal(0), Agent::Gemini))
        );
        // Nobody anywhere is what summons the drawer, so it has to be tellable from the rest.
        assert_eq!(agent_precedence(None, None, None, None), None);
    }

    /// A workspace governs the drawer's column and never its contents. The variant that would
    /// rebuild a live drawer does not exist, and this is where that is said out loud.
    #[test]
    fn a_workspace_never_rebuilds_a_drawer_that_is_already_there() {
        use crate::workspace::WorkspaceDrawer;
        let saved = |open, agent: Option<&str>| WorkspaceDrawer {
            open,
            width: 45,
            agent: agent.map(str::to_string),
        };

        // A live drawer: the file may open or close its column, and that is the whole of what it
        // may do. Note the agent named in the file is ignored — the one in the pane stays.
        assert_eq!(
            drawer_from_workspace(Some(&saved(true, Some("codex"))), true),
            DrawerFromWorkspace::SetOpen { open: true, width: 45 }
        );
        assert_eq!(
            drawer_from_workspace(Some(&saved(false, Some("codex"))), true),
            DrawerFromWorkspace::SetOpen { open: false, width: 45 }
        );

        // No drawer yet: an open one in the file is summoned, agent and all.
        assert_eq!(
            drawer_from_workspace(Some(&saved(true, Some("codex"))), false),
            DrawerFromWorkspace::Summon { agent: Some(crate::session::Agent::Codex), width: 45 }
        );
        assert_eq!(
            drawer_from_workspace(Some(&saved(true, Some("clod"))), false),
            DrawerFromWorkspace::Summon { agent: None, width: 45 },
            "a name we do not know opens the launcher rather than running it"
        );
        assert_eq!(
            drawer_from_workspace(Some(&saved(false, Some("codex"))), false),
            DrawerFromWorkspace::LeaveAlone,
            "a closed drawer that does not exist is a drawer that does not exist"
        );

        // A file from before the field existed says nothing, which is not the same as saying no.
        assert_eq!(drawer_from_workspace(None, true), DrawerFromWorkspace::LeaveAlone);
        assert_eq!(drawer_from_workspace(None, false), DrawerFromWorkspace::LeaveAlone);
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
            drawer_open: false,
            debug_open: false,
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

    /// One press of backspace undoes one character as the person who typed it counts them, not
    /// one scalar as Rust counts them. The decomposed forms are the ones that come out of a
    /// macOS filename or a paste from a browser, so a rename box gets them without asking.
    #[test]
    fn backspace_takes_a_letter_and_the_accent_drawn_on_it_together() {
        let mut typed = String::from("caffe\u{301}");
        pop_grapheme(&mut typed);
        assert_eq!(typed, "caff", "the accent and the e it sits on are one keystroke");

        // Several marks on one base, which is how Vietnamese and transliterated Greek arrive.
        let mut stacked = String::from("a\u{0323}\u{0302}");
        pop_grapheme(&mut stacked);
        assert_eq!(stacked, "");

        // A precomposed accent is a single scalar and needs none of this.
        let mut precomposed = String::from("caffè");
        pop_grapheme(&mut precomposed);
        assert_eq!(precomposed, "caff");
    }

    /// An emoji built out of several scalars comes back off as one picture. Deleting a piece of
    /// it is worse than doing nothing: the leftovers are themselves emoji, so the box shows a
    /// different picture rather than a shorter word.
    #[test]
    fn backspace_takes_a_joined_emoji_sequence_in_one_go() {
        // Family: man ZWJ woman ZWJ girl. Three presses would leave two people standing there.
        let mut family = String::from("hi \u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}");
        pop_grapheme(&mut family);
        assert_eq!(family, "hi ");

        // A keycap is a digit, a variation selector and an enclosing mark.
        let mut keycap = String::from("press 1\u{fe0f}\u{20e3}");
        pop_grapheme(&mut keycap);
        assert_eq!(keycap, "press ");

        // A profession: a person, the joiner, and the thing they are holding.
        let mut chef = String::from("\u{1f9d1}\u{200d}\u{1f373}");
        pop_grapheme(&mut chef);
        assert_eq!(chef, "");
    }

    /// The plain cases, including the one that has to stay a no-op: backspace on an empty box is
    /// pressed all the time and must not be a panic.
    #[test]
    fn backspace_on_an_ordinary_string_takes_exactly_one_character() {
        let mut empty = String::new();
        pop_grapheme(&mut empty);
        assert_eq!(empty, "");

        let mut word = String::from("src/main.rs");
        pop_grapheme(&mut word);
        assert_eq!(word, "src/main.r");

        // A lone emoji with nothing attached to it is one scalar and goes on its own.
        let mut single = String::from("ok \u{1f600}");
        pop_grapheme(&mut single);
        assert_eq!(single, "ok ");
    }

    /// A chord is not typing. crossterm reports Ctrl+V as the letter `v` with a modifier set,
    /// and every box that reads the letter without the modifier fills up with the shortcuts
    /// pressed over it.
    #[test]
    fn a_character_carrying_a_command_modifier_is_not_typing() {
        let plain = KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE);
        assert!(is_a_typed_character(plain));
        // Shift is part of typing: it is how a capital letter is made.
        assert!(is_a_typed_character(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT)));

        for chord in [KeyModifiers::CONTROL, KeyModifiers::ALT, KeyModifiers::SUPER] {
            assert!(
                !is_a_typed_character(KeyEvent::new(KeyCode::Char('v'), chord)),
                "{chord:?}+V is a shortcut, not a `v`"
            );
        }
    }

    /// A buffer holding `text`, for the two functions a rename is made of.
    fn buffer(text: &str) -> Editor {
        let mut editor = Editor::empty();
        editor.rope = ropey::Rope::from_str(text);
        editor
    }

    /// Every occurrence of `word` in `text`, as the converted edits a preview would hold.
    fn edits_over(editor: &Editor, word: &str, new_text: &str) -> Vec<BufferEdit> {
        let text = editor.rope.to_string();
        let chars: Vec<char> = text.chars().collect();
        let target: Vec<char> = word.chars().collect();
        (0..chars.len().saturating_sub(target.len() - 1))
            .filter(|&at| chars[at..at + target.len()] == target[..])
            .map(|at| BufferEdit {
                start: at,
                end: at + target.len(),
                line: editor.rope.char_to_line(at),
                new_text: new_text.to_string(),
            })
            .collect()
    }

    // ---- Replacing across the project -----------------------------------------------------
    //
    // The three things a sweep gets wrong quietly, each on its own: a line that matches more than
    // once, a file whose line endings it has to hand back untouched, and the choice of which road
    // a file takes. All three fail invisibly in a driver — the screen looks right and the bytes
    // are wrong — which is exactly what a unit test is for.

    /// The reason the preview re-scans instead of reading the search's hits: a hit is a line and
    /// carries one match, and a replace-all that stopped at the first one per line would be a
    /// replace-some wearing the same name.
    #[test]
    fn every_match_on_a_line_is_replaced_not_just_the_first() {
        let re = crate::find::compile("ab", false, true).unwrap();
        let scan = scan_for_replacements("ab xx ab\nno\nab ab ab\n", &re, "Z", false);
        assert_eq!(scan.edits.len(), 5, "two on the first line, three on the third");
        assert_eq!(scan.edits.iter().map(|e| e.line).collect::<Vec<_>>(), vec![0, 0, 2, 2, 2]);
        // Char offsets into the whole text, which is what the rebuild and the rows both count in.
        assert_eq!(scan.edits[0].start, 0);
        assert_eq!(scan.edits[1].start, 6);
        assert_eq!(scan.line_starts, vec![0, 9, 12]);

        // And the rows collapse a line's matches into one pair, the way the rename's do: two
        // intermediate states that never exist are not a preview of anything.
        let rows = diff_rows(&scan.lines, &scan.edits, |line| scan.line_starts[line]);
        assert_eq!(rows, vec!["- ab xx ab", "+ Z xx Z", "- ab ab ab", "+ Z Z Z"]);
    }

    /// An accent is where the engine's bytes and the editor's characters come apart, and every
    /// offset here is a character.
    #[test]
    fn offsets_land_on_characters_in_an_accented_file() {
        let re = crate::find::compile("città", false, true).unwrap();
        let text = "una città e una città\nperò\n";
        let scan = scan_for_replacements(text, &re, "paese", false);
        assert_eq!(scan.edits.len(), 2);
        let chars: Vec<char> = text.chars().collect();
        for edit in &scan.edits {
            assert_eq!(chars[edit.start..edit.end].iter().collect::<String>(), "città");
        }
    }

    /// A pattern's groups are resolved against the line the match was found in, because that is
    /// the text the project search matched against — and a literal query has no groups, so its
    /// dollars stay dollars.
    #[test]
    fn a_group_reaches_the_replacement_and_a_literal_dollar_does_not() {
        let re = crate::find::compile(r"(\w+)@(\w+)", true, true).unwrap();
        let scan = scan_for_replacements("ada@lovelace\nalan@turing\n", &re, "$2.$1", true);
        let becomes: Vec<&str> = scan.edits.iter().map(|e| e.new_text.as_str()).collect();
        assert_eq!(becomes, vec!["lovelace.ada", "turing.alan"]);

        let plain = crate::find::compile("cost", false, true).unwrap();
        let scan = scan_for_replacements("cost\n", &plain, "$1", false);
        assert_eq!(scan.edits[0].new_text, "$1", "no groups to quote, so no quoting");
    }

    /// What the file said about itself is said back to it. A sweep that turned a CRLF file into
    /// an LF one, or grew a final newline the file never had, would read in review as every line
    /// having changed — which is the one thing replacing three words must not look like.
    #[test]
    fn a_rewritten_file_keeps_its_line_endings_and_its_last_newline() {
        let dir = setup_dir("sweep_endings");
        let crlf = dir.join("crlf.txt");
        std::fs::write(&crlf, b"alfa here\r\nalfa again\r\n").unwrap();
        let bare = dir.join("bare.txt");
        std::fs::write(&bare, b"alfa here\nalfa again").unwrap();

        let re = crate::find::compile("alfa", false, true).unwrap();
        for path in [&crlf, &bare] {
            let (text, target) = read_for_sweep(path).expect("both are text");
            assert!(!text.contains('\r'), "the scan sees normalized text, whatever is on disk");
            let SweepTarget::Disk { line_ending, final_newline, .. } = target else {
                panic!("a file with no tab takes the disk road");
            };
            let scan = scan_for_replacements(&text, &re, "beta", false);
            assert_eq!(scan.edits.len(), 2);
            let chars: Vec<char> = text.chars().collect();
            let (start, end, rebuilt) =
                rebuild_edits(&scan.edits, |from, to| chars[from..to].iter().collect()).unwrap();
            let mut whole: String = chars[..start].iter().collect();
            whole.push_str(&rebuilt);
            whole.extend(&chars[end..]);
            let written = text_for_disk(whole, line_ending, final_newline);
            std::fs::write(path, written.as_bytes()).unwrap();
        }
        assert_eq!(std::fs::read(&crlf).unwrap(), b"beta here\r\nbeta again\r\n");
        assert_eq!(
            std::fs::read(&bare).unwrap(),
            b"beta here\nbeta again",
            "a file that ended without a newline still does"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The choice the whole feature turns on. A file with a tab is scanned in the rope — dirty
    /// included, since the text on screen is the text somebody means — and a file without one is
    /// read from disk with everything needed to write it back.
    #[test]
    fn a_file_with_a_tab_never_takes_the_disk_road() {
        let dir = setup_dir("sweep_roads");
        let path = dir.join("open.txt");
        std::fs::write(&path, "what is on disk\n").unwrap();

        let mut editor = buffer("what is in the buffer\n");
        editor.path = Some(path.clone());
        let (text, target) = sweep_text_and_target(Some(&editor), &path).expect("a tab holds it");
        assert_eq!(text, "what is in the buffer\n", "the text the user can see");
        assert!(matches!(target, SweepTarget::OpenBuffer { revision } if revision == editor.revision()));

        let (text, target) = sweep_text_and_target(None, &path).expect("and without a tab");
        assert_eq!(text, "what is on disk\n");
        assert!(matches!(target, SweepTarget::Disk { .. }));

        // Not text at all: skipped rather than mangled, the same answer the search gave it.
        let blob = dir.join("blob.bin");
        std::fs::write(&blob, [0x61, 0x00, 0xff, 0xfe]).unwrap();
        assert!(sweep_text_and_target(None, &blob).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// One replacement for the whole file, whatever the edits are spread over — which is what
    /// makes a rename one step of undo instead of one per occurrence.
    #[test]
    fn a_files_share_of_a_rename_is_a_single_replacement() {
        let editor = buffer("let count = count + 1;\nprintln!(\"{count}\");\n");
        let edits = edits_over(&editor, "count", "total");
        assert_eq!(edits.len(), 3, "two on the first line and one on the second");
        let (start, end, rebuilt) = edits_as_one_span(&editor, &edits).unwrap();
        // From the first occurrence to the last, and everything in between carried over as it is.
        assert_eq!((start, end), (4, 39));
        assert_eq!(rebuilt, "total = total + 1;\nprintln!(\"{total");
        // Applied, the file reads as the rename asked — and the text outside the span is
        // untouched, which is the half a single replacement could get wrong.
        let mut applied = buffer(&editor.rope.to_string());
        applied.replace_char_range(start, end, &rebuilt);
        assert_eq!(applied.rope.to_string(), "let total = total + 1;\nprintln!(\"{total}\");\n");
    }

    /// A replacement of a different length moves everything after it, and the rebuild is where
    /// that either works or quietly eats a character. Both directions, because a shorter name is
    /// the case where an off-by-one deletes real text rather than duplicating it.
    #[test]
    fn the_rebuild_survives_a_name_of_a_different_length() {
        for new_name in ["n", "a_much_longer_name"] {
            let editor = buffer("xx\nvalue.value(value)\nyy\n");
            let edits = edits_over(&editor, "value", new_name);
            assert_eq!(edits.len(), 3);
            let (start, end, rebuilt) = edits_as_one_span(&editor, &edits).unwrap();
            let mut applied = buffer(&editor.rope.to_string());
            applied.replace_char_range(start, end, &rebuilt);
            assert_eq!(
                applied.rope.to_string(),
                format!("xx\n{new_name}.{new_name}({new_name})\nyy\n")
            );
        }
    }

    /// The lines a buffer would hand the converter: its own, with the newlines off, which is what
    /// makes a column past the end of a line stop at that line.
    fn lines_of(editor: &Editor) -> Vec<String> {
        editor.rope.lines().map(|l| l.to_string().trim_end_matches('\n').to_string()).collect()
    }

    /// The column conversion for a server counting in UTF-8, which is what the fixtures are.
    fn as_bytes(text: &str, col: usize) -> usize {
        crate::lsp::utf8_to_chars(text, col)
    }

    fn span(start: (usize, usize), end: (usize, usize), new_text: &str) -> crate::lsp::SpanEdit {
        crate::lsp::SpanEdit {
            start_line: start.0,
            start_col: start.1,
            end_line: end.0,
            end_col: end.1,
            new_text: new_text.to_string(),
        }
    }

    /// The whole file replaced by a laid-out copy: one edit, both ends on different lines, which
    /// is the shape a rename refuses and a format gets constantly. The end is converted against
    /// the line it is actually on — measuring it on the *start* line is the mistake this exists
    /// to catch, and on line 0 of a four-line file it would land three lines short.
    #[test]
    fn a_whole_file_format_converts_both_ends_on_their_own_lines() {
        let editor = buffer("fn a() {\n        one();\n  two();\n}\n");
        let lines = lines_of(&editor);
        let laid_out = "fn a() {\n    one();\n    two();\n}\n";
        // From the top to the empty line after the last newline, which is how a server names the
        // whole document when it can count the buffer's lines.
        let edits = vec![span((0, 0), (4, 0), laid_out)];
        let converted = format_spans(&editor.rope, &lines, &as_bytes, &edits).unwrap();
        assert_eq!(converted.len(), 1);
        assert_eq!((converted[0].start, converted[0].end), (0, editor.rope.len_chars()));
        let (start, end, rebuilt) = edits_as_one_span(&editor, &converted).unwrap();
        let mut applied = buffer(&editor.rope.to_string());
        applied.replace_char_range(start, end, &rebuilt);
        assert_eq!(applied.rope.to_string(), laid_out);
    }

    /// The other shape: two disjoint edits that reindent two lines and leave the rest alone. They
    /// come back as one replacement spanning both, with the untouched text between them carried
    /// over verbatim — which is what makes a format one step of undo rather than one per line.
    #[test]
    fn two_disjoint_format_edits_become_one_replacement() {
        let editor = buffer("fn a() {\n        one();\n  two();\n}\n");
        let lines = lines_of(&editor);
        let edits = vec![
            span((1, 0), (1, 8), "    "),
            span((2, 0), (2, 2), "    "),
        ];
        let converted = format_spans(&editor.rope, &lines, &as_bytes, &edits).unwrap();
        assert_eq!(converted.len(), 2);
        let (start, end, rebuilt) = edits_as_one_span(&editor, &converted).unwrap();
        let mut applied = buffer(&editor.rope.to_string());
        applied.replace_char_range(start, end, &rebuilt);
        assert_eq!(applied.rope.to_string(), "fn a() {\n    one();\n    two();\n}\n");
    }

    /// "To the end of the document", in both of the ways servers spell it: the line *after* the
    /// last, since a count is one more than the last index, and `u32::MAX`. Clamped to the end of
    /// the rope rather than refused — this is the shape of a whole-file format, not a server
    /// describing text that has moved.
    #[test]
    fn an_end_past_the_last_line_clamps_to_the_end_of_the_file() {
        let editor = buffer("one\ntwo\n");
        let lines = lines_of(&editor);
        for past in [lines.len(), 4_294_967_295] {
            let edits = vec![span((0, 0), (past, 0), "laid out\n")];
            let converted = format_spans(&editor.rope, &lines, &as_bytes, &edits).unwrap();
            assert_eq!((converted[0].start, converted[0].end), (0, editor.rope.len_chars()));
        }
    }

    /// A *start* line the buffer does not have is the other case entirely, and it is refused: the
    /// server is describing text that is no longer here, and there is no honest place to put the
    /// replacement. The line that separates this test from the one above is the whole rule.
    #[test]
    fn a_start_line_the_buffer_lacks_refuses_the_whole_format() {
        let editor = buffer("one\ntwo\n");
        let lines = lines_of(&editor);
        let edits = vec![span((0, 0), (0, 3), "x"), span((9, 0), (9, 1), "y")];
        assert_eq!(
            format_spans(&editor.rope, &lines, &as_bytes, &edits).err(),
            Some(FormatRefusal::Moved),
            "and the edit it could have applied is not applied either"
        );
    }

    /// Two edits over the same characters, which the rebuild has no right answer for. Refused
    /// whole, and refused whichever order the server sent them in — they are sorted first.
    #[test]
    fn overlapping_format_edits_are_refused_whole() {
        let editor = buffer("aaaa bbbb\n");
        let lines = lines_of(&editor);
        let overlapping = vec![span((0, 0), (0, 6), "x"), span((0, 4), (0, 9), "y")];
        assert_eq!(
            format_spans(&editor.rope, &lines, &as_bytes, &overlapping).err(),
            Some(FormatRefusal::Overlap)
        );
        let mut backwards = overlapping;
        backwards.reverse();
        assert_eq!(
            format_spans(&editor.rope, &lines, &as_bytes, &backwards).err(),
            Some(FormatRefusal::Overlap),
            "the order it arrived in is not the question"
        );
        // Meeting exactly is not overlapping: one run of text ends where the next begins.
        let touching = vec![span((0, 0), (0, 4), "x"), span((0, 4), (0, 9), "y")];
        assert!(format_spans(&editor.rope, &lines, &as_bytes, &touching).is_ok());
    }

    /// The server may answer in any order it likes; the rebuild needs them ascending. Sorted
    /// here rather than trusted, because an unsorted list would not fail loudly — it would
    /// rebuild a span with the pieces swapped and write it out looking plausible.
    #[test]
    fn format_edits_are_sorted_before_they_are_rebuilt() {
        let editor = buffer("one\ntwo\nsix\n");
        let lines = lines_of(&editor);
        let edits = vec![span((2, 0), (2, 3), "three"), span((0, 0), (0, 3), "ONE")];
        let converted = format_spans(&editor.rope, &lines, &as_bytes, &edits).unwrap();
        assert!(converted[0].start < converted[1].start);
        let (start, end, rebuilt) = edits_as_one_span(&editor, &converted).unwrap();
        let mut applied = buffer(&editor.rope.to_string());
        applied.replace_char_range(start, end, &rebuilt);
        assert_eq!(applied.rope.to_string(), "ONE\ntwo\nthree\n");
    }

    /// A zero-width span is an insertion, and a formatter really does send them — a blank line
    /// put between two functions deletes nothing. It survives the conversion as itself.
    #[test]
    fn a_zero_width_format_edit_is_an_insertion() {
        let editor = buffer("fn a() {}\nfn b() {}\n");
        let lines = lines_of(&editor);
        let edits = vec![span((1, 0), (1, 0), "\n")];
        let converted = format_spans(&editor.rope, &lines, &as_bytes, &edits).unwrap();
        assert_eq!((converted[0].start, converted[0].end), (10, 10));
        let (start, end, rebuilt) = edits_as_one_span(&editor, &converted).unwrap();
        let mut applied = buffer(&editor.rope.to_string());
        applied.replace_char_range(start, end, &rebuilt);
        assert_eq!(applied.rope.to_string(), "fn a() {}\n\nfn b() {}\n");
    }

    /// Two edits that meet exactly are two names, not an overlap, and the text between them is
    /// nothing rather than a character to carry over.
    #[test]
    fn adjacent_edits_leave_nothing_between_them() {
        let editor = buffer("abab\n");
        let edits = vec![
            BufferEdit { start: 0, end: 2, line: 0, new_text: "X".to_string() },
            BufferEdit { start: 2, end: 4, line: 0, new_text: "Y".to_string() },
        ];
        let (start, end, rebuilt) = edits_as_one_span(&editor, &edits).unwrap();
        assert_eq!((start, end, rebuilt.as_str()), (0, 4, "XY"));
    }

    /// The preview shows each changed line once, with every edit on it already applied. Two
    /// occurrences on one line drawn as two pairs would show an intermediate state that never
    /// exists — the line with one of them renamed and the other not.
    #[test]
    fn several_edits_on_one_line_are_one_pair_of_rows() {
        let editor = buffer("let count = count + 1;\nprintln!(\"{count}\");\n");
        let edits = edits_over(&editor, "count", "total");
        let lines: Vec<String> =
            editor.rope.lines().map(|l| l.to_string().trim_end_matches('\n').to_string()).collect();
        let rows = preview_rows(&editor, &lines, &edits);
        assert_eq!(
            rows,
            vec![
                "- let count = count + 1;",
                "+ let total = total + 1;",
                "- println!(\"{count}\");",
                "+ println!(\"{total}\");",
            ]
        );
        // The marker carries a space, so a line of code that starts with `--` cannot be read as
        // the file header the panel colours differently.
        assert!(rows.iter().all(|row| !row.starts_with("---")));
    }

    /// What a row of the code action list says, and that the thing behind it is the thing that
    /// was on it. The kind is the second half of the row — it is what tells a fix for the error
    /// under the caret from a refactoring that would apply anywhere — and an action the server did
    /// not name a kind for gets no word of ours in its place.
    #[test]
    fn a_code_action_row_carries_the_title_the_kind_and_the_action() {
        let answer = serde_json::json!([
            {"title": "Import `HashMap`", "kind": "quickfix",
             "edit": {"changes": {"file:///p/a.rs": [
                 {"range": {"start": {"line": 0, "character": 0},
                            "end": {"line": 0, "character": 0}},
                  "newText": "use std::collections::HashMap;\n"}]}}},
            {"title": "Wrap the\nwhole thing", "edit": {"changes": {}}},
        ]);
        let actions = crate::lsp::offered_actions(Some(&answer), false);
        let items = code_action_items(actions);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].label, "Import `HashMap`");
        assert_eq!(items[0].shortcut.as_deref(), Some("quickfix"));
        // A title the server wrote across two lines is one row here, as a diagnostic's message is
        // in the list beside this one.
        assert_eq!(items[1].label, "Wrap the whole thing");
        assert_eq!(items[1].shortcut, None, "no kind is no word, rather than one of ours");
        // And the row carries the action itself, so nothing has to be asked again at Enter.
        match &items[0].action {
            crate::picker::PickAction::CodeAction(action) => {
                assert_eq!(action.title, "Import `HashMap`");
                assert!(action.edit.is_some());
            }
            _ => panic!("the row has to carry the action it is a row for"),
        }
    }

    /// One action, all of it inside one open buffer: converted down the format's road — which is
    /// the general one, and has to be, because an action's spans cross lines as a matter of course
    /// — and applied as a single replacement, which is what makes it one step of undo.
    ///
    /// The fixture is the commonest quick fix there is: a `use` line inserted at the top and a name
    /// corrected further down, two disjoint edits that must arrive together or not at all.
    #[test]
    fn a_code_action_inside_one_buffer_lands_as_one_edit() {
        let editor = buffer("fn main() {\n    let m = HashMap::new();\n}\n");
        let answer = serde_json::json!([{
            "title": "Import `HashMap`",
            "kind": "quickfix",
            "edit": {"changes": {"file:///p/a.rs": [
                {"range": {"start": {"line": 0, "character": 0},
                           "end": {"line": 0, "character": 0}},
                 "newText": "use std::collections::HashMap;\n"},
                {"range": {"start": {"line": 1, "character": 12},
                           "end": {"line": 1, "character": 19}},
                 "newText": "HashMap"},
            ]}},
        }]);
        let actions = crate::lsp::offered_actions(Some(&answer), false);
        let plan = actions[0].edit.as_ref().expect("the edit came with it");
        assert_eq!(plan.files.len(), 1, "one file, which is what sends it down this road");
        let lines = lines_of(&editor);
        let converted =
            format_spans(&editor.rope, &lines, &as_bytes, &plan.files[0].edits).unwrap();
        assert_eq!(converted.len(), 2);
        // One span from the first edit to the last, with the text between them carried over — and
        // therefore one `replace_char_range`, which is one checkpoint and one Ctrl+Z.
        let (start, end, rebuilt) = edits_as_one_span(&editor, &converted).unwrap();
        let mut applied = buffer(&editor.rope.to_string());
        applied.replace_char_range(start, end, &rebuilt);
        assert_eq!(
            applied.rope.to_string(),
            "use std::collections::HashMap;\nfn main() {\n    let m = HashMap::new();\n}\n"
        );
    }

    /// The preview measures in characters, not bytes: a line with an accent in it before the name
    /// is the case where counting the wrong unit shows a `+` row with the name in the wrong place
    /// — and the offsets that draw it are the offsets that get written.
    #[test]
    fn the_preview_counts_characters_and_not_bytes() {
        let editor = buffer("// città: value\n");
        let edits = edits_over(&editor, "value", "conto");
        assert_eq!(edits[0].start, 10, "ten characters, thirteen bytes");
        let lines = vec!["// città: value".to_string()];
        assert_eq!(
            preview_rows(&editor, &lines, &edits),
            vec!["- // città: value", "+ // città: conto"]
        );
    }
}
