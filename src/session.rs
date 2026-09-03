//! A live interpreter sitting at its own prompt in one of the terminals.
//!
//! Octave and Python are one feature here, not two. Everything the rest of the program wants —
//! which language a buffer is, whether an interpreter for it is already open, what to type at
//! that prompt to run a file, how to quote a path for it — differs by a line or two between them
//! and by nothing else. Built as two parallel implementations that might converge later, it
//! becomes two half-features that never do, so the seam goes in before the second language does.
//!
//! The Octave half of this was already in `dnd.rs`, hard-wired, for the Run button: handing a
//! `.m` file to an Octave that is already open rather than starting a second one. That is the
//! same question this asks, so it moved here and grew a second answer.

use std::path::Path;

/// The bare program name inside `program`: no directory in front of it, no `.exe` behind it.
///
/// The process table hands back a full path as often as a word, and a run command may hold
/// either. Both questions asked of a program name — is this an interpreter, is this an agent —
/// start here, so they start the same way.
fn program_stem(program: &str) -> &str {
    let base = program.rsplit(['/', '\\']).next().unwrap_or(program);
    base.strip_suffix(".exe").unwrap_or(base)
}

/// Whether a program is one that runs a script somebody else wrote, so that the name in the
/// process table is the interpreter's and the interesting name is in the arguments.
///
/// Only `node`, because only one of the agents is shipped that way: `claude` from npm is a
/// wrapper script, and running it puts `node` in the table with the script as its argument. The
/// list is a list so that a second agent packaged for a different runtime has somewhere to go.
fn is_script_interpreter(name: &str) -> bool {
    ["node"].contains(&program_stem(name).to_ascii_lowercase().as_str())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Language {
    Octave,
    Python,
}

impl Language {
    /// Which language a file is written in, or `None` for one we have no session for.
    pub fn of_path(path: &Path) -> Option<Language> {
        match path.extension()?.to_str()?.to_lowercase().as_str() {
            "m" => Some(Language::Octave),
            "py" | "pyw" => Some(Language::Python),
            _ => None,
        }
    }

    /// Program names that mean "an interpreter of this language is at its own prompt in here".
    ///
    /// `octave` is a launcher — on macOS it execs `octave-gui` even for a terminal session, and
    /// Windows ships `octave-cli.exe` — so every variant has to be recognised. Python's `python`
    /// and `python3` are both live, and `ipython` is here because it is a Python prompt even
    /// though what it wants said to it differs; that difference is [`Self::run_file`]'s problem.
    pub fn programs(self) -> &'static [&'static str] {
        match self {
            Language::Octave => &["octave", "octave-cli", "octave-gui", "octave-launch"],
            Language::Python => &["python", "python3", "ipython", "ipython3"],
        }
    }

    /// Whether a program name — a bare word from a run-command template, or a full path to the
    /// executable — is an interpreter for this language.
    pub fn is_interpreter(self, program: &str) -> bool {
        let stem = program_stem(program);
        // Compared without case, and that is not tidiness. Homebrew's Python on macOS is a
        // framework build, so the process the table shows is
        // `Python.framework/…/Python.app/Contents/MacOS/Python` — the name is `Python`, with a
        // capital P, for every `python3` installed with brew. Matched case-sensitively it was
        // not a Python at all, so sending a cell to a live session decided none was open and
        // typed the *shell* command `python3 /tmp/…/script.py` at the Python prompt, where it
        // is only `NameError: name 'python3' is not defined`. Windows has the same habit for
        // its own reasons. This is the Octave-on-headless-Linux bug below wearing a different
        // hat, and it was found the same way: by running the check that had never been run.
        let known = |name: &str| {
            !name.is_empty()
                && self.programs().iter().any(|p| p.eq_ignore_ascii_case(name))
        };
        if known(stem) {
            return true;
        }
        // `python3.13` is a python, and so is `python3.13t`. A version on the end is not part of
        // the name anywhere the name is written down, so it comes off before asking again.
        let bare = stem.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.' || c == 't');
        if known(bare) {
            return true;
        }
        // Octave writes its version with a dash in front: `octave-cli-11.3.0`. That is not a
        // spelling anyone types — it is what `/usr/bin/octave` execs on a build without Qt,
        // which is every headless Linux server — and it is the name the process table then
        // shows. Unrecognised, the Run button decided no Octave was open and typed the shell
        // command `octave --persist file.m` at the live Octave prompt, where it is only
        // `error: 'octave' undefined`.
        //
        // Truncation is handled by the same line: Linux caps a process name at 15 characters,
        // so the name actually read is `octave-cli-11.3`, and the version comes off either way.
        known(bare.strip_suffix('-').unwrap_or_default())
    }

    /// What to type at this language's prompt to run a file.
    ///
    /// A file, never the code itself. Pasting a multi-line block at a Python prompt is how the
    /// REPL ends up seeing an indented line with no header and answering `IndentationError`, and
    /// the same paste at an Octave prompt gets echoed line by line into the user's transcript.
    /// A temp file has neither problem, and it is one line at the prompt whatever it contains.
    pub fn run_file(self, path: &str) -> String {
        match self {
            Language::Octave => format!("run({}){}", self.quote(path), self.marker()),
            // `exec` rather than `runpy` or an import: it runs in the prompt's own namespace, so
            // what the snippet defines is there afterwards — which is the entire point of
            // sending it to a live session instead of starting a new one.
            Language::Python => format!("exec(open({}).read()){}", self.quote(path), self.marker()),
        }
    }

    /// A string literal holding `text`, for this language.
    pub fn quote(self, text: &str) -> String {
        match self {
            // Octave's single-quoted strings have exactly one special character, and it is
            // escaped by doubling.
            Language::Octave => format!("'{}'", text.replace('\'', "''")),
            // Python's are the usual thing, and a backslash in a Windows path makes this matter.
            Language::Python => {
                format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
            }
        }
    }

    /// The comment CleeCode leaves on the end of a command it typed itself.
    ///
    /// Two jobs, and both matter. In the transcript it says who did this — a line the user did
    /// not type appearing at their prompt should say so. And in the history it is what tells the
    /// panel to leave it out: a list of recent commands full of `figure(1); zoom(2);` is a list
    /// of what CleeCode did, which nobody asked to see.
    ///
    /// A comment rather than a convention about shape, so a user typing `figure(2)` themselves
    /// is never mistaken for us.
    pub fn marker(self) -> &'static str {
        match self {
            Language::Octave => "  %cleecode",
            Language::Python => "  # cleecode",
        }
    }

    /// The extension a scratch file for this language needs. Octave will not `run` a file that is
    /// not `.m`, and Python's tracebacks read better when the name looks like a module.
    pub fn scratch_extension(self) -> &'static str {
        match self {
            Language::Octave => "m",
            Language::Python => "py",
        }
    }

    /// What a snapshot calls this language, which is how a session on disk is recognised as
    /// one of ours rather than the other one's.
    pub fn snapshot_lang(self) -> &'static str {
        match self {
            Language::Octave => "octave",
            Language::Python => "python",
        }
    }

    /// What to say at the prompt to close these figures and leave every other one alone.
    ///
    /// Typed before a file is run again, with the numbers that file's previous run opened. The
    /// point is the numbering: both languages hand out the next free number, so a script that
    /// says `figure()` — or `plt.subplots()`, which is the same thing — makes new figures every
    /// time it runs, and three runs leave six tabs of what the person who wrote it thinks of as
    /// two plots. Closing the previous set frees the numbers, so the rerun draws into the tabs
    /// that are already open. A script that names its figures (`figure(1)`) was already fine and
    /// stays fine: it closes 1 and immediately creates 1 again.
    ///
    /// Neither form prints anything, and neither touches a figure it was not given — a plot made
    /// by hand at the prompt is not part of any run and must survive one.
    pub fn close_figures(self, numbers: &[i64]) -> String {
        let list =
            numbers.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ");
        match self {
            // `intersect` with what the session actually holds, because `close` on a number
            // that is not a figure is an error — and a figure the user closed by hand between
            // the two runs is exactly that.
            Language::Octave => {
                format!("close(intersect([{list}], get(0, 'children')'));{}", self.marker())
            }
            // Through the hook's own module rather than as an expression at the prompt: it
            // returns nothing, so nothing is echoed into the transcript, and it does the
            // "is matplotlib even imported" check where that check belongs.
            Language::Python => {
                format!("_cleecode_pyws.close_figures({list}){}", self.marker())
            }
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Language::Octave => "Octave",
            Language::Python => "Python",
        }
    }
}

/// A move around a figure.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Nav {
    In,
    Out,
    Left,
    Right,
    Up,
    Down,
    Reset,
}

impl Language {
    /// What to say at the prompt to move around figure `number`.
    ///
    /// The whole point is that this goes *back into the interpreter* rather than being done to
    /// the picture. Magnifying the pixels would leave the axis labels describing a range that is
    /// no longer on screen — the plot would say 0 to 100 while showing 25 to 75 — and no amount
    /// of sharpening fixes a number that is wrong. Re-drawing costs 37ms, measured, so the
    /// honest answer is also the affordable one.
    ///
    /// `view` is the current 3-D angle, which the figure's geometry sidecar already carries;
    /// rotating means naming the new angle, and there is no relative form to name it with.
    pub fn nav_command(self, nav: Nav, number: i64, is3d: bool, view: (f64, f64)) -> String {
        match self {
            Language::Octave => {
                // `set(0, "currentfigure", n)` and not `figure(n)`, and this is not a
                // stylistic preference: `figure(n)` on a figure that already exists *raises*
                // it, which in Octave means setting `visible` back to "on". CleeCode's
                // sessions run with `defaultfigurevisible` off precisely so no window ever
                // opens — but that default only applies when a figure is created, so one
                // `figure(n)` undoes it for good.
                //
                // The effect, measured on a Mac with the qt toolkit: pressing an arrow on a
                // figure tab popped a real Qt window and left it there, so the plot was on
                // screen twice — once as the tab and once as a window behind the terminal,
                // which is the exact thing the tab exists to avoid. Every nav key did it, and
                // so did the button beside it.
                //
                // `currentfigure` selects without raising. Checked in Octave 11.3.0: the
                // figure stays "off" and `xlim()` afterwards operates on the right one.
                let select = format!("set(0, 'currentfigure', {number}); ");
                let body = match (nav, is3d) {
                    // `zoom(factor)`, not `zoom on`: the mode wants a real window to click in,
                    // the factor form does not.
                    (Nav::In, _) => "zoom(2);".to_string(),
                    (Nav::Out, _) => "zoom(0.5);".to_string(),
                    (Nav::Reset, false) => "axis auto;".to_string(),
                    (Nav::Reset, true) => "view(-37.5, 30);".to_string(),
                    // A quarter of the span, which is far enough to be worth a keystroke and
                    // near enough that what you were looking at is still on screen.
                    (Nav::Left, false) => "xl = xlim(); xlim(xl - 0.25 * diff(xl));".to_string(),
                    (Nav::Right, false) => "xl = xlim(); xlim(xl + 0.25 * diff(xl));".to_string(),
                    (Nav::Up, false) => "yl = ylim(); ylim(yl + 0.25 * diff(yl));".to_string(),
                    (Nav::Down, false) => "yl = ylim(); ylim(yl - 0.25 * diff(yl));".to_string(),
                    (Nav::Left, true) => format!("view({}, {});", view.0 - 15.0, view.1),
                    (Nav::Right, true) => format!("view({}, {});", view.0 + 15.0, view.1),
                    (Nav::Up, true) => format!("view({}, {});", view.0, (view.1 + 15.0).min(90.0)),
                    (Nav::Down, true) => format!("view({}, {});", view.0, (view.1 - 15.0).max(-90.0)),
                };
                format!("{select}{body}{}", self.marker())
            }
            Language::Python => {
                let select = format!(
                    "import matplotlib.pyplot as _plt; _f = _plt.figure({number}); _a = _f.axes[0]; "
                );
                let body = match (nav, is3d) {
                    (Nav::In, _) => "_a.set_xlim(*[c + (l - c) / 2 for c in [sum(_a.get_xlim()) / 2] for l in _a.get_xlim()]); _a.set_ylim(*[c + (l - c) / 2 for c in [sum(_a.get_ylim()) / 2] for l in _a.get_ylim()])".to_string(),
                    (Nav::Out, _) => "_a.set_xlim(*[c + (l - c) * 2 for c in [sum(_a.get_xlim()) / 2] for l in _a.get_xlim()]); _a.set_ylim(*[c + (l - c) * 2 for c in [sum(_a.get_ylim()) / 2] for l in _a.get_ylim()])".to_string(),
                    (Nav::Reset, false) => "_a.autoscale()".to_string(),
                    (Nav::Reset, true) => "_a.view_init(30, -60)".to_string(),
                    (Nav::Left, false) => "_a.set_xlim(*[l - (_a.get_xlim()[1] - _a.get_xlim()[0]) / 4 for l in _a.get_xlim()])".to_string(),
                    (Nav::Right, false) => "_a.set_xlim(*[l + (_a.get_xlim()[1] - _a.get_xlim()[0]) / 4 for l in _a.get_xlim()])".to_string(),
                    (Nav::Up, false) => "_a.set_ylim(*[l + (_a.get_ylim()[1] - _a.get_ylim()[0]) / 4 for l in _a.get_ylim()])".to_string(),
                    (Nav::Down, false) => "_a.set_ylim(*[l - (_a.get_ylim()[1] - _a.get_ylim()[0]) / 4 for l in _a.get_ylim()])".to_string(),
                    (Nav::Left, true) => format!("_a.view_init({}, {})", view.1, view.0 - 15.0),
                    (Nav::Right, true) => format!("_a.view_init({}, {})", view.1, view.0 + 15.0),
                    (Nav::Up, true) => format!("_a.view_init({}, {})", (view.1 + 15.0).min(90.0), view.0),
                    (Nav::Down, true) => format!("_a.view_init({}, {})", (view.1 - 15.0).max(-90.0), view.0),
                };
                // The names are underscore-prefixed for the same reason the startup file's are:
                // they land in the user's own namespace and would otherwise show up as variables
                // in their workspace panel.
                format!("{select}{body}; _f.canvas.draw_idle(){}", self.marker())
            }
        }
    }
}

impl Language {
    /// What to say at the prompt to write figure `number` out as a file.
    ///
    /// PDF, because a plot leaves an editor to go into a document, and there it wants to be
    /// vector: a PNG of a plot is the right size exactly once. The PNG the tab is showing is
    /// already on disk for anyone who wants pixels.
    pub fn export_command(self, number: i64, path: &str) -> String {
        match self {
            Language::Octave => {
                // The handle straight to `print`, with nothing selected first. A numbered
                // figure's handle *is* its number, so there is nothing here that needs a
                // current figure — and `figure(n)` would raise a window, as it does in
                // `nav_command`, where the reasoning is written out.
                format!("print({number}, '-dpdf', {});{}", self.quote(path), self.marker())
            }
            Language::Python => format!(
                "import matplotlib.pyplot as _plt; _plt.figure({number}).savefig({}){}",
                self.quote(path),
                self.marker()
            ),
        }
    }
}

/// A coding agent sitting at its own prompt in one of the terminals.
///
/// The same shape as [`Language`], and for the same reason. Claude Code, opencode, codex and
/// gemini are terminal programs, so CleeCode does not have to embed one — it already hosts real
/// ptys. What the rest of the program wants to know about them differs by a line each: which
/// process names mean "that agent is running in here", how a file is named at its prompt, what
/// to call it on screen. Written as four integrations that might converge later, it becomes four
/// half-features that never do, so the seam goes in with the first one.
///
/// The seam is honest about being a seam. In v1 all four answer [`Agent::reference`] the same
/// way — `path:line`, plain text, which every one of them reads and which `locate.rs` already
/// turns back into a jump when the agent prints it. The point of the enum is that the day one of
/// them wants `@path` or a slash command, exactly one function changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Agent {
    Claude,
    OpenCode,
    Codex,
    Gemini,
}

/// A selection longer than this goes as a reference and nothing else.
///
/// Ten lines is about what fits at a prompt while still being readable as *the thing you
/// selected*; past that it is a wall of text in front of the question, and the agent can open
/// the file at the line — which is what the reference is for. It reads the file better than a
/// paste of it anyway.
pub const AGENT_INLINE_LINES: usize = 10;

/// What the editor has to say about where you are, in the order the keystroke prefers it: an
/// explicit selection, then a diagnostic the language server put under the cursor, then the
/// cursor itself. Line numbers are one-based — what the file's own gutter shows, and what an
/// agent means by `file:12`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Context {
    Selection { from: usize, to: usize, text: String },
    Diagnostic { line: usize, message: String },
    Cursor { line: usize },
}

impl Agent {
    /// Every agent, in the order the presets are declared.
    pub fn all() -> [Agent; 4] {
        [Agent::Claude, Agent::OpenCode, Agent::Codex, Agent::Gemini]
    }

    /// Program names that mean "this agent is at its own prompt in here".
    ///
    /// One name each, because each of the four ships one executable and that is what it is
    /// called. `claude` is the interesting case and it is interesting the other way: installed
    /// from npm it is a script run by `node`, so the process table often says `node` and no list
    /// of names can fix that. `gemini` is installed the same way and shares the same answer.
    /// Two answers where the name cannot answer — the arguments the process was started with
    /// ([`Agent::of_process`]) and the pane's own startup command ([`Agent::of_command`]).
    pub fn programs(self) -> &'static [&'static str] {
        match self {
            Agent::Claude => &["claude"],
            Agent::OpenCode => &["opencode"],
            Agent::Codex => &["codex"],
            Agent::Gemini => &["gemini"],
        }
    }

    /// Which agent a program name is, if any. A full path and a `.exe` are the same program as
    /// the bare word, and case is not compared: Windows writes names its own way.
    pub fn of_program(name: &str) -> Option<Agent> {
        let stem = program_stem(name);
        if stem.is_empty() {
            return None;
        }
        Agent::all()
            .into_iter()
            .find(|agent| agent.programs().iter().any(|p| p.eq_ignore_ascii_case(stem)))
    }

    /// Which agent a startup command starts, if any: the first word of it is the program, and
    /// everything after it is that program's business (`claude --resume`, `codex --model o3`).
    pub fn of_command(command: &str) -> Option<Agent> {
        Agent::of_program(command.split_whitespace().next()?)
    }

    /// Which agent a *running process* is, read from the name the process table shows together
    /// with the arguments it was started with.
    ///
    /// The name alone answers for a binary, and it is the whole of [`Agent::of_program`]. It does
    /// not answer for `claude` installed from npm: npm's wrapper is a script, and running a
    /// script means running the interpreter, so the process table says `node` and the agent is
    /// invisible to any search by name — which left Ctrl+Shift+A saying there was no agent while
    /// one sat at its prompt two panes away. A pane opened from the preset is covered by its
    /// startup command; a `claude` typed by hand into an ordinary shell was covered by nothing.
    ///
    /// What the wrapper cannot hide is the argument. `node` has to be told which script to run,
    /// and the script npm installed is called `claude` — so where the program is an interpreter,
    /// the file stem of the script it was handed is the agent's own name. Only the first argument
    /// that is not a flag is read: that is the script, and everything after it belongs to the
    /// script rather than to node.
    pub fn of_process(name: &str, argv: &[String]) -> Option<Agent> {
        if let Some(agent) = Agent::of_program(name) {
            return Some(agent);
        }
        if !is_script_interpreter(name) {
            return None;
        }
        // The interpreter's own name comes off the front — by what it is rather than by counting,
        // since whether a process table repeats argv[0] is the table's business and not ours.
        // A bare `node` with nothing left after it is a REPL and names no agent.
        argv.iter()
            .skip_while(|arg| is_script_interpreter(arg))
            .find(|arg| !arg.starts_with('-'))
            .and_then(|script| {
                // `server.js` keeps its extension through `program_stem`, so a plain Node program
                // is not mistaken for an agent even when it is called `claude.js`: what npm
                // installs, and what has to be recognised here, is the extensionless `claude`.
                Agent::of_program(script)
            })
    }

    /// The name of the built-in workspace that opens this agent, which is also what its terminal
    /// tab is called. Lower case, because it is the command you type.
    pub fn workspace_name(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::OpenCode => "opencode",
            Agent::Codex => "codex",
            Agent::Gemini => "gemini",
        }
    }

    /// The agent's place in [`Agent::all`], which is the order everything about the four is
    /// listed in: the presets, the drawer's launcher, and the memo below.
    pub fn index(self) -> usize {
        match self {
            Agent::Claude => 0,
            Agent::OpenCode => 1,
            Agent::Codex => 2,
            Agent::Gemini => 3,
        }
    }

    /// Whether this agent is installed on this machine.
    ///
    /// Asked once for all four and remembered, the same bargain [`crate::preview::has_pandoc`]
    /// makes: the answer is a walk of every directory on the PATH, the drawer's launcher asks it
    /// on every frame it is drawn, and a program does not get installed while a screen is up.
    ///
    /// It decides only how the name is *drawn* — an agent that is not here is shown dim and said
    /// to be missing, because the empty launcher is also where you find out what CleeCode knows
    /// how to run. It never removes a name from the list, and it is never the reason a start
    /// fails: the shell is the one that gets to say a command was not found.
    pub fn on_path(self) -> bool {
        static FOUND: std::sync::OnceLock<[bool; 4]> = std::sync::OnceLock::new();
        FOUND.get_or_init(|| {
            let mut found = [false; 4];
            for agent in Agent::all() {
                found[agent.index()] =
                    agent.programs().iter().any(|name| crate::tools::tool(name).is_some());
            }
            found
        })[self.index()]
    }

    /// What to call it on screen.
    pub fn label(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::OpenCode => "opencode",
            Agent::Codex => "codex",
            Agent::Gemini => "gemini",
        }
    }

    /// How a place in a file is named at this agent's prompt.
    ///
    /// `path:line`, in plain text, for all four. Not `@path`: that is Claude Code's file
    /// reference and it means *read this whole file*, which is a different request from *look
    /// here*. Every one of the four reads `path:line`, and it is the same spelling they print
    /// back — which `locate.rs` has turned into a double-click jump since long before this
    /// existed, so the round trip is already built.
    pub fn reference(&self, path: &str, line: usize) -> String {
        format!("{path}:{line}")
    }

    /// The same for a span of lines: `path:3-9`, and `path:3` where the span is one line.
    pub fn range_reference(&self, path: &str, from: usize, to: usize) -> String {
        if from == to { self.reference(path, from) } else { format!("{path}:{from}-{to}") }
    }

    /// The text one keystroke hands to this agent — the whole of what is sent, composed here so
    /// it can be read in a test rather than inferred from a pty.
    ///
    /// `holds_a_paste` is whether the program in the pane turned bracketed paste on. It decides
    /// whether anything multi-line may go at all: to a program that never asked, a newline is
    /// Enter, so pasting ten lines into it would send nine messages and the discipline this
    /// whole feature is built on — *CleeCode never presses Enter for you* — would be broken by
    /// the paste itself. Where it cannot be held, the reference goes alone; it says the same
    /// thing and the agent can read the file.
    pub fn context(&self, path: &str, what: &Context, holds_a_paste: bool) -> String {
        let fits = |text: &str| {
            let lines = text.lines().count();
            lines <= AGENT_INLINE_LINES && (holds_a_paste || lines <= 1)
        };
        match what {
            Context::Selection { from, to, text } => {
                let head = self.range_reference(path, *from, *to);
                let body = text.trim_end();
                if body.is_empty() || !fits(body) {
                    head
                } else {
                    format!("{head}\n{body}")
                }
            }
            // On one line with the reference: a diagnostic is a sentence, and a sentence about
            // a place reads as one thing.
            Context::Diagnostic { line, message } => {
                let head = self.reference(path, *line);
                let message = message.trim();
                if message.is_empty() || !fits(message) {
                    head
                } else {
                    format!("{head} {message}")
                }
            }
            Context::Cursor { line } => self.reference(path, *line),
        }
    }
}

/// Which piece of the file was sent, so the status line can say which without the caller
/// spelling out two nearly identical sentences.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Piece {
    Selection,
    Cell,
}

/// Whether a line begins a `%%` cell.
///
/// One rule for both languages, which is what lets the editor's side of this be written once.
/// The marker itself is shared — `%%` — and only the comment character in front of it differs,
/// so `%% section` in Octave and `# %%` in Python are the same thing said twice. Both are what
/// the surrounding worlds already write, so neither community has to learn a CleeCode
/// convention to use the feature.
pub fn is_cell_marker(line: &str) -> bool {
    let text = line.trim_start();
    // Strip one comment character and any space after it, so `# %%`, `#%%` and a bare `%%` all
    // arrive at the same place.
    let text = match text.strip_prefix('#') {
        Some(rest) => rest.trim_start(),
        None => text,
    };
    text.starts_with("%%")
}

/// The `%%` cell containing `line`, as a half-open range of line numbers.
///
/// A file with no markers is one cell, which is the honest reading: "run this cell" in a script
/// nobody has divided means run the script. The marker line is part of the cell it opens, so its
/// title travels with it and shows up in the transcript — which is how you tell, afterwards,
/// which piece was run.
pub fn cell_at(lines: &[&str], line: usize) -> (usize, usize) {
    if lines.is_empty() {
        return (0, 0);
    }
    let line = line.min(lines.len() - 1);
    let start = (0..=line).rev().find(|&i| is_cell_marker(lines[i])).unwrap_or(0);
    let end = ((line + 1)..lines.len()).find(|&i| is_cell_marker(lines[i])).unwrap_or(lines.len());
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_placed_by_its_extension() {
        assert_eq!(Language::of_path(Path::new("plot.m")), Some(Language::Octave));
        assert_eq!(Language::of_path(Path::new("train.py")), Some(Language::Python));
        assert_eq!(Language::of_path(Path::new("main.rs")), None);
        assert_eq!(Language::of_path(Path::new("Makefile")), None);
    }

    #[test]
    fn an_interpreter_is_recognised_by_any_of_its_names() {
        // macOS execs octave-gui even for a terminal session; Windows ships octave-cli.exe.
        assert!(Language::Octave.is_interpreter("octave"));
        assert!(Language::Octave.is_interpreter("octave-cli"));
        assert!(Language::Octave.is_interpreter("octave-gui"));
        assert!(Language::Octave.is_interpreter("octave-cli.exe"));
        assert!(Language::Octave.is_interpreter("/opt/homebrew/bin/octave"));
        assert!(Language::Octave.is_interpreter(
            r"C:\Program Files\GNU Octave\Octave-10.1.0\mingw64\bin\octave-cli.exe"
        ));
        // A name that merely starts the same way is not a match.
        assert!(!Language::Octave.is_interpreter("octaveish"));
        assert!(Language::Python.is_interpreter("python3"));
        assert!(Language::Python.is_interpreter("/usr/bin/python"));
        assert!(Language::Python.is_interpreter("ipython"));
        // A version on the end is still the same program.
        assert!(Language::Python.is_interpreter("python3.13"));
        assert!(Language::Python.is_interpreter("python3.14t"), "free-threaded builds too");
        // The name a headless Linux Octave actually runs under: `/usr/bin/octave` execs the
        // versioned cli binary when the build has no Qt, and the process table shows it cut to
        // fifteen characters. Both spellings are the same interpreter.
        assert!(Language::Octave.is_interpreter("octave-cli-11.3.0"));
        assert!(Language::Octave.is_interpreter("octave-cli-11.3"), "Linux cuts a name at 15");
        assert!(Language::Octave.is_interpreter("/usr/bin/octave-11.3.0"));
        // And they do not answer for each other.
        assert!(!Language::Octave.is_interpreter("python3"));
        // A dash and a word after it is a different program, not a version.
        assert!(!Language::Octave.is_interpreter("octave-cli-wrapper"));
        assert!(!Language::Python.is_interpreter("octave"));
        assert!(!Language::Python.is_interpreter("pythonista"));

        // Homebrew's Python on macOS is a framework build, so the process table shows
        // `Python.app/Contents/MacOS/Python` — a capital P, for every brew `python3` there is.
        // Matched with case it was not a Python, and a cell sent to a live session became the
        // shell command `python3 file.py` typed at the Python prompt.
        assert!(Language::Python.is_interpreter("Python"));
        assert!(Language::Python.is_interpreter(
            "/opt/homebrew/Cellar/python@3.14/3.14.7/Frameworks/Python.framework/Versions/3.14/Resources/Python.app/Contents/MacOS/Python"
        ));
        assert!(Language::Python.is_interpreter("Python3.14"));
        assert!(Language::Octave.is_interpreter("Octave-CLI"));
        // And nothing became a python that was not one.
        assert!(!Language::Python.is_interpreter("Pythonista"));
        assert!(!Language::Python.is_interpreter(""));
    }

    #[test]
    fn each_language_is_told_to_run_a_file_in_its_own_words() {
        assert!(Language::Octave.run_file("/tmp/cell.m").starts_with("run('/tmp/cell.m')"));
        assert!(Language::Python
            .run_file("/tmp/cell.py")
            .starts_with("exec(open(\"/tmp/cell.py\").read())"));
        // Everything CleeCode types carries its mark, so the transcript says who did it and the
        // history panel can leave it out.
        for language in [Language::Octave, Language::Python] {
            for command in [
                language.run_file("/tmp/x"),
                language.nav_command(Nav::In, 1, false, (0.0, 90.0)),
                language.export_command(1, "/tmp/x.pdf"),
            ] {
                assert!(command.ends_with(language.marker()), "unmarked: {command}");
            }
        }
    }

    /// A path with a quote in it is rare and a path with a backslash is not, and either one
    /// unescaped turns a command into a syntax error at somebody's prompt.
    #[test]
    fn a_path_is_quoted_for_the_prompt_it_is_going_to() {
        assert_eq!(Language::Octave.quote("/tmp/it's here/a.m"), "'/tmp/it''s here/a.m'");
        assert_eq!(Language::Python.quote(r"C:\tmp\a.py"), r#""C:\\tmp\\a.py""#);
        assert_eq!(Language::Python.quote(r#"say "hi""#), r#""say \"hi\"""#);
    }

    /// Verified against a live Octave before it was written down: zoom(2) takes [0 100] to
    /// [25 75], the pan forms move the window by a quarter of its span, and `axis auto` puts it
    /// back. What is checked here is that the right one is chosen, not that Octave works.
    #[test]
    fn moving_around_a_figure_is_said_in_the_language_of_the_session() {
        let octave = Language::Octave;
        assert!(octave.nav_command(Nav::In, 1, false, (0.0, 90.0)).starts_with("set(0, 'currentfigure', 1); zoom(2);"));
        assert!(octave.nav_command(Nav::Out, 3, false, (0.0, 90.0)).starts_with("set(0, 'currentfigure', 3); zoom(0.5);"));
        assert!(octave.nav_command(Nav::Right, 1, false, (0.0, 90.0)).contains("xlim(xl + 0.25"));
        assert!(octave.nav_command(Nav::Down, 1, false, (0.0, 90.0)).contains("ylim(yl - 0.25"));
        // Every figure is named before it is acted on, or the command lands on whichever one
        // the session last drew.
        for nav in [Nav::In, Nav::Out, Nav::Left, Nav::Right, Nav::Up, Nav::Down, Nav::Reset] {
            assert!(octave.nav_command(nav, 7, false, (0.0, 90.0)).starts_with("set(0, 'currentfigure', 7);"));
        }
    }

    /// On a surface the arrows turn it rather than sliding it, which is what they mean to
    /// anyone who has used the figure window: there is nothing off the edge to pan towards.
    #[test]
    fn arrows_rotate_a_surface_instead_of_panning_it() {
        let octave = Language::Octave;
        assert!(octave.nav_command(Nav::Right, 2, true, (45.0, 30.0)).starts_with("set(0, 'currentfigure', 2); view(60, 30);"));
        assert!(octave.nav_command(Nav::Left, 2, true, (45.0, 30.0)).starts_with("set(0, 'currentfigure', 2); view(30, 30);"));
        // Elevation stops at the poles rather than turning the surface inside out.
        assert!(octave.nav_command(Nav::Up, 2, true, (45.0, 85.0)).contains("view(45, 90)"));
        assert!(octave.nav_command(Nav::Down, 2, true, (45.0, -85.0)).contains("view(45, -90)"));
        assert!(octave.nav_command(Nav::Reset, 2, true, (45.0, 30.0)).starts_with("set(0, 'currentfigure', 2); view(-37.5, 30);"));
    }

    #[test]
    fn python_says_the_same_things_its_own_way() {
        let python = Language::Python;
        let zoom = python.nav_command(Nav::In, 1, false, (0.0, 0.0));
        assert!(zoom.contains("plt.figure(1)") && zoom.contains("set_xlim"));
        // Everything it leaves behind is underscore-prefixed, or it would appear in the user's
        // own workspace panel as a variable they never made.
        for name in ["_plt", "_f", "_a"] {
            assert!(zoom.contains(name), "{zoom}");
        }
        assert!(!zoom.contains(" plt") && !zoom.contains("= f "), "{zoom}");
        assert!(python.nav_command(Nav::Right, 1, true, (45.0, 30.0)).contains("view_init(30, 60)"));
    }

    #[test]
    fn a_figure_leaves_as_a_vector_file() {
        assert!(Language::Octave
            .export_command(1, "/proj/fig1.pdf")
            .starts_with("print(1, '-dpdf', '/proj/fig1.pdf');"));
        let python = Language::Python.export_command(2, "/proj/fig2.pdf");
        assert!(python.contains("figure(2).savefig(\"/proj/fig2.pdf\")"), "{python}");
        // A path with a quote in it still parses at the prompt it is going to.
        assert!(Language::Octave.export_command(1, "/it's/fig.pdf").contains("'/it''s/fig.pdf'"));
    }

    #[test]
    fn an_agent_is_recognised_by_its_program_name() {
        assert_eq!(Agent::of_program("claude"), Some(Agent::Claude));
        assert_eq!(Agent::of_program("opencode"), Some(Agent::OpenCode));
        assert_eq!(Agent::of_program("codex"), Some(Agent::Codex));
        assert_eq!(Agent::of_program("gemini"), Some(Agent::Gemini));
        // A full path and a Windows executable are the same program as the bare word.
        assert_eq!(Agent::of_program("/opt/homebrew/bin/claude"), Some(Agent::Claude));
        assert_eq!(Agent::of_program(r"C:\Users\x\AppData\npm\codex.exe"), Some(Agent::Codex));
        assert_eq!(Agent::of_program(r"C:\Users\x\AppData\npm\gemini.exe"), Some(Agent::Gemini));
        assert_eq!(Agent::of_program("Claude"), Some(Agent::Claude));
        // And nothing that merely starts the same way.
        assert_eq!(Agent::of_program("claudette"), None);
        assert_eq!(Agent::of_program("node"), None);
        assert_eq!(Agent::of_program(""), None);
        // Neither answers for the other, and an interpreter is not an agent.
        assert_eq!(Agent::of_program("octave"), None);

        // A startup command is a program and its arguments; the program is the first word.
        assert_eq!(Agent::of_command("claude --resume"), Some(Agent::Claude));
        assert_eq!(Agent::of_command("  opencode  "), Some(Agent::OpenCode));
        assert_eq!(Agent::of_command("npm run dev"), None);
        assert_eq!(Agent::of_command(""), None);
    }

    /// The npm wrapper, which is how most people have Claude Code: `claude` is a script, so the
    /// process table says `node` and the agent was invisible to Ctrl+Shift+A unless the pane had
    /// been opened by the preset. The script's own name is in the arguments, and that is what is
    /// read here.
    #[test]
    fn an_agent_run_by_node_is_still_that_agent() {
        let argv = |args: &[&str]| args.iter().map(|a| a.to_string()).collect::<Vec<_>>();

        // What `npm i -g @anthropic-ai/claude-code` leaves in the process table.
        assert_eq!(
            Agent::of_process("node", &argv(&["node", "/usr/local/bin/claude"])),
            Some(Agent::Claude)
        );
        // The same on Windows, where the interpreter has an extension and the path is written
        // the other way round.
        assert_eq!(
            Agent::of_process("node.exe", &argv(&["node.exe", r"C:\Users\x\AppData\npm\claude"])),
            Some(Agent::Claude)
        );
        // Flags to node itself sit before the script and are stepped over.
        assert_eq!(
            Agent::of_process("node", &argv(&["node", "--no-warnings", "/opt/npm/bin/claude", "--resume"])),
            Some(Agent::Claude)
        );
        // gemini-cli is installed from npm the same way, so it is found the same way.
        assert_eq!(
            Agent::of_process("node", &argv(&["node", "/usr/local/bin/gemini"])),
            Some(Agent::Gemini)
        );

        // An ordinary Node program is not an agent, and neither is a bare REPL.
        assert_eq!(Agent::of_process("node", &argv(&["node", "server.js"])), None);
        assert_eq!(Agent::of_process("node", &argv(&["node"])), None);
        assert_eq!(Agent::of_process("node", &[]), None);
        // A table that does not repeat argv[0] is read the same way, since the interpreter is
        // taken off the front by what it is rather than by counting.
        assert_eq!(Agent::of_process("node", &argv(&["/usr/local/bin/claude"])), Some(Agent::Claude));
        // Nor is a script that merely has an agent's name inside its own: the file npm installs
        // has no extension, and `claude.js` is somebody's own program.
        assert_eq!(Agent::of_process("node", &argv(&["node", "/srv/claude.js"])), None);

        // A real binary still answers by name, arguments or no arguments — the argument list is
        // read only where the name is an interpreter's.
        assert_eq!(Agent::of_process("codex", &argv(&["codex", "--model", "o3"])), Some(Agent::Codex));
        assert_eq!(Agent::of_process("opencode", &[]), Some(Agent::OpenCode));
        // And a shell that happens to have been handed a path is not an agent: only the runtime
        // the wrapper actually uses gets its arguments read.
        assert_eq!(Agent::of_process("bash", &argv(&["bash", "/usr/local/bin/claude"])), None);
    }

    /// The seam is real only if every agent is also a preset somebody can open by name.
    #[test]
    fn every_agent_has_a_preset_and_a_name_of_its_own() {
        let mut seen: Vec<&str> = Vec::new();
        for agent in Agent::all() {
            let name = agent.workspace_name();
            assert!(crate::workspace::is_built_in(name), "{name} is not a built-in workspace");
            assert_eq!(Agent::of_program(name), Some(agent), "{name} does not find itself");
            assert!(!seen.contains(&name));
            assert!(!agent.label().is_empty());
            seen.push(name);
        }
    }

    /// What one keystroke actually hands over, in the three cases and in order of precedence.
    #[test]
    fn the_context_sent_is_a_reference_and_at_most_what_fits_beside_it() {
        let agent = Agent::Claude;
        let path = "src/app.rs";

        // The cursor, and nothing else to say about it.
        assert_eq!(agent.context(path, &Context::Cursor { line: 12 }, true), "src/app.rs:12");

        // A short selection travels with its text, under a reference to where it came from.
        let short = Context::Selection { from: 3, to: 5, text: "a\nb\nc".to_string() };
        assert_eq!(agent.context(path, &short, true), "src/app.rs:3-5\na\nb\nc");
        // A one-line selection is not written as a range.
        let one = Context::Selection { from: 7, to: 7, text: "let x = 1;".to_string() };
        assert_eq!(agent.context(path, &one, true), "src/app.rs:7\nlet x = 1;");

        // A long one goes as the reference alone: the agent reads the file better than a wall
        // of it in front of the question.
        let text = (0..AGENT_INLINE_LINES + 1).map(|n| n.to_string()).collect::<Vec<_>>().join("\n");
        let long = Context::Selection { from: 1, to: AGENT_INLINE_LINES + 1, text };
        assert_eq!(agent.context(path, &long, true), format!("src/app.rs:1-{}", AGENT_INLINE_LINES + 1));

        // A diagnostic is a sentence about a place, so it goes on the line with it.
        let diag = Context::Diagnostic {
            line: 40,
            message: "cannot borrow `self` as mutable".to_string(),
        };
        assert_eq!(agent.context(path, &diag, true), "src/app.rs:40 cannot borrow `self` as mutable");
        // Nothing to say is not an empty line at somebody's prompt.
        let empty = Context::Diagnostic { line: 40, message: "   ".to_string() };
        assert_eq!(agent.context(path, &empty, true), "src/app.rs:40");
    }

    /// Where the pane never turned bracketed paste on, a newline in what we send *is* Enter —
    /// so nothing multi-line is sent at all. CleeCode does not press Enter for you, and a paste
    /// that presses it nine times would be the loudest possible way of breaking that.
    #[test]
    fn nothing_multi_line_goes_to_a_prompt_that_cannot_hold_a_paste() {
        let agent = Agent::Codex;
        let selection = Context::Selection { from: 3, to: 5, text: "a\nb\nc".to_string() };
        assert_eq!(agent.context("f.rs", &selection, false), "f.rs:3-5");
        // One line still travels: it is one line either way.
        let one = Context::Selection { from: 3, to: 3, text: "a".to_string() };
        assert_eq!(agent.context("f.rs", &one, false), "f.rs:3\na");
        let diag = Context::Diagnostic { line: 2, message: "no\nsuch\nfield".to_string() };
        assert_eq!(agent.context("f.rs", &diag, false), "f.rs:2");
    }

    /// v1 says the same thing to all four, on purpose, and that is worth pinning: the day one
    /// of them differs, this test is where the decision gets written down.
    #[test]
    fn all_four_agents_are_told_the_same_thing_in_v1() {
        let what = Context::Cursor { line: 9 };
        for agent in Agent::all() {
            assert_eq!(agent.reference("a/b.rs", 9), "a/b.rs:9");
            assert_eq!(agent.context("a/b.rs", &what, true), "a/b.rs:9");
        }
    }

    #[test]
    fn a_cell_marker_is_the_same_mark_behind_either_comment_character() {
        assert!(is_cell_marker("%% load the data"));
        assert!(is_cell_marker("  %% indented"));
        assert!(is_cell_marker("# %% load the data"));
        assert!(is_cell_marker("#%% no space"));
        assert!(!is_cell_marker("% an ordinary Octave comment"));
        assert!(!is_cell_marker("# an ordinary Python comment"));
        assert!(!is_cell_marker("x = 1  %% not at the start"));
        assert!(!is_cell_marker(""));
    }

    #[test]
    fn a_cell_runs_from_its_marker_to_the_next() {
        let lines = vec![
            "%% first",      // 0
            "a = 1;",        // 1
            "b = 2;",        // 2
            "%% second",     // 3
            "c = 3;",        // 4
        ];
        assert_eq!(cell_at(&lines, 0), (0, 3), "on the marker itself");
        assert_eq!(cell_at(&lines, 2), (0, 3), "inside the first cell");
        assert_eq!(cell_at(&lines, 3), (3, 5), "on the second marker");
        assert_eq!(cell_at(&lines, 4), (3, 5), "inside the second cell");
    }

    #[test]
    fn lines_before_the_first_marker_are_their_own_cell() {
        let lines = vec!["setup = 1;", "%% work", "x = 2;"];
        assert_eq!(cell_at(&lines, 0), (0, 1));
        assert_eq!(cell_at(&lines, 2), (1, 3));
    }

    /// A script nobody has divided is one cell, so "run this cell" means run the script rather
    /// than doing nothing and leaving you to work out why.
    #[test]
    fn a_file_with_no_markers_is_one_cell() {
        let lines = vec!["a = 1;", "b = 2;", "c = 3;"];
        assert_eq!(cell_at(&lines, 1), (0, 3));
        assert_eq!(cell_at(&[], 0), (0, 0));
        // A cursor past the end — a buffer that shrank under it — clamps rather than panicking.
        assert_eq!(cell_at(&lines, 99), (0, 3));
    }

    /// What a rerun types before it runs. The numbers are the previous run's, and everything
    /// about both forms is aimed at the same two rules: close only those, and print nothing.
    #[test]
    fn closing_a_runs_figures_names_them_and_says_nothing() {
        let octave = Language::Octave.close_figures(&[1, 2]);
        assert!(octave.starts_with("close(intersect([1, 2], get(0, 'children')'));"));
        // Guarded by what the session actually holds: `close` on a number that is not a figure
        // is an error, and the user may have closed one by hand between the two runs.
        assert!(octave.contains("get(0, 'children')"));
        assert!(octave.ends_with(Language::Octave.marker()));

        let python = Language::Python.close_figures(&[3]);
        assert_eq!(python, format!("_cleecode_pyws.close_figures(3){}", Language::Python.marker()));
        // Through the hook's module rather than as an expression, so the prompt echoes nothing.
        assert!(!python.contains("plt.close"));
    }
}
