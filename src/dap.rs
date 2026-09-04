//! A debug adapter client, spoken by hand over stdio.
//!
//! The Debug Adapter Protocol is the debugger's side of the same bargain `lsp.rs` struck for
//! language servers: one protocol, spoken to whichever program the machine happens to have —
//! `lldb-dap` here, `gdb -i=dap` there — so the editor learns a wire rather than a debugger.
//! It frames its messages exactly as LSP does, `Content-Length` and then JSON, which is why
//! the framing in this file is not written twice: [`crate::lsp::frame`] and
//! [`crate::lsp::read_message`] already do it, and a second copy of that arithmetic would be a
//! second chance to get it wrong.
//!
//! The concurrency is the one CleeCode uses everywhere else and nowhere else: a thread that
//! reads, an `mpsc` channel that carries, and a `poll` in the frame loop that drains. No async
//! runtime, no new crates. What is different from `lsp.rs` is where the work happens — there
//! the reader thread turns replies into events and hands answers back through the channel to be
//! written; here the reader thread does nothing but forward whole JSON messages, and [`Client::poll`]
//! does all the matching and all the answering. It can, because it runs in the frame loop and
//! therefore owns the pipe into the adapter: a debug session has to *write* in response to what
//! it reads — a `launch` when the handshake lands, breakpoints when the adapter says it is ready
//! to be configured, a refusal when the adapter asks for something — and the thread that owns the
//! writer is the only one that can do that without two threads interleaving half a frame.
//!
//! This module is protocol-pure, the way `lsp.rs` is: it knows requests, responses and events,
//! and it does not know what a pane or an editor is. Lines and columns on its public surface are
//! 1-based, which is not a conversion but a request — the handshake asks for `linesStartAt1` and
//! `columnsStartAt1`, so what arrives on the wire is already counted the way the rest of this
//! codebase counts.
//!
//! An adapter that dies is an event and not a panic. Everything else in a debug session is
//! optional; the session going away while a pane is open is the ordinary end of every debug
//! session there has ever been.

// Wave 1 is the protocol module alone: the application does not call any of this until wave 2
// wires the Debug menu, the panel and the breakpoint sync to it. Every method below is reached
// by the tests at the bottom of this file and by nothing else yet, which is exactly what
// `dead_code` is for and exactly why it is wrong here — the alternative would be to leave the
// warning standing for a release and teach everybody to ignore the one place the compiler is
// still allowed to speak.
#![allow(dead_code)]

use crate::lsp::{frame, read_message};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};

// ---- What the application is told ------------------------------------------------------------

/// What the client hands to the application. Deliberately not the protocol's own shapes: the app
/// should not have to know what a `variablesReference` is to draw a row — though it does have to
/// carry one back to ask for the children, which is why that one number survives the translation.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The adapter is ready to be configured. This is the `initialized` *event*, not the answer
    /// to the `initialize` request — the two are different messages arriving at different times,
    /// and confusing them is the classic way to build a client that hangs. See [`Client::poll`]
    /// for what this client does about it, which is most of the lifecycle.
    Initialized,
    /// What one file's breakpoints became after the adapter tried to place them.
    ///
    /// Keyed by path rather than by request id, because that is the question that was asked: a
    /// `setBreakpoints` replaces every breakpoint in one file, so its answer describes that file
    /// whole and nothing else. `verified` is the part worth carrying — a breakpoint on a line
    /// with no code is accepted, moved, or refused depending on the adapter, and the user has to
    /// be told which.
    Breakpoints { path: PathBuf, breakpoints: Vec<Breakpoint> },
    /// The debuggee stopped, and why.
    ///
    /// `thread` is optional because the protocol makes it optional: an adapter that stopped every
    /// thread at once is entitled to say so without naming one. A `None` is not a failure — it is
    /// an invitation to ask [`Client::threads`] — and inventing a thread id to avoid the `Option`
    /// would be this module telling the application something the adapter never said.
    ///
    /// `path` and `line` are filled in only when the adapter volunteered them in the event body,
    /// which several do and the specification does not require. Where they are absent the place
    /// is found by asking for a [`Client::stack_trace`], which is wave 2's job because it is the
    /// application that knows whether anybody is looking.
    Stopped {
        thread: Option<i64>,
        reason: String,
        description: Option<String>,
        path: Option<PathBuf>,
        line: Option<usize>,
    },
    /// The debuggee is running again, so whatever was drawn on the stopped line stops being true.
    Continued { thread: i64 },
    /// The debuggee exited on its own account, with its status.
    Exited { code: i64 },
    /// The session is over. Distinct from [`Self::Exited`]: a program can exit while the adapter
    /// stays up, and an adapter can end a session whose program never ran at all.
    Terminated,
    /// Something to print. For most adapters this is where the debuggee's own stdout and stderr
    /// come out, which is the whole reason it is surfaced rather than dropped — a debugger that
    /// swallows the program's output is a debugger nobody can use on the program they have.
    ///
    /// `category` is the protocol's own word: `stdout`, `stderr`, `console`, `telemetry`, or
    /// whatever else the adapter invented. Passed along rather than reduced to a flag, because
    /// deciding which of them is worth showing is a decision about the panel, not about the wire.
    Output { category: String, text: String },
    /// The threads the adapter knows about, answering one [`Client::threads`].
    Threads { id: i64, threads: Vec<ThreadInfo> },
    /// One thread's frames, innermost first, answering one [`Client::stack_trace`].
    StackTrace { id: i64, frames: Vec<Frame> },
    /// One frame's scopes, answering one [`Client::scopes`].
    Scopes { id: i64, scopes: Vec<Scope> },
    /// The children of one reference, answering one [`Client::variables`].
    Variables { id: i64, variables: Vec<Variable> },
    /// What an expression came to, answering one [`Client::evaluate`].
    Evaluated { id: i64, value: String, reference: i64 },
    /// A request the adapter refused, with what it said and which request it was.
    ///
    /// One event for every refusal, rather than a `Result` inside each answer the way `lsp.rs`
    /// carries them, because DAP marks failure uniformly: every response carries `success`, and a
    /// failed one carries a `message` and no body at all. There is nothing to put in the typed
    /// answer, so a typed answer would have to be an empty one — and "the adapter cannot evaluate
    /// that here" read as "that evaluates to nothing" is the editor putting words in its mouth.
    Failed { id: i64, command: String, message: String },
    /// The adapter stopped talking, with whatever it said on the way out. Nothing else breaks:
    /// the session is over, the panel says so, and the editor is an editor again.
    Dead { reason: String },
}

/// One breakpoint as the adapter placed it — or did not.
#[derive(Clone, Debug, PartialEq)]
pub struct Breakpoint {
    /// Whether the adapter will actually stop there. An unverified breakpoint is drawn
    /// differently and is the honest answer to a line with no code on it.
    pub verified: bool,
    /// Where it ended up, 1-based. Adapters are allowed to move a breakpoint to the next line
    /// that has code, and a gutter mark that stays where it was typed would be a lie.
    pub line: Option<usize>,
    /// Why it could not be placed, when the adapter bothered to say.
    pub message: Option<String>,
}

/// One frame of a stack, as the panel will list it.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    /// The adapter's own handle for this frame, and the thing every later question about it
    /// carries — scopes, evaluations. Not an index: adapters are free to number these as they
    /// like, and a position in a list stops being true the moment the program moves.
    pub id: i64,
    pub name: String,
    /// Where the frame is, when the adapter knows. Absent for frames with no source — a library
    /// without symbols, a signal trampoline — which are worth listing anyway, because a stack
    /// with holes in it is still the stack.
    pub path: Option<PathBuf>,
    /// 1-based, as asked for in the handshake.
    pub line: usize,
    pub column: usize,
}

/// One scope of one frame: "Locals", "Registers", and whatever else the adapter groups by.
#[derive(Clone, Debug, PartialEq)]
pub struct Scope {
    pub name: String,
    /// The handle to ask [`Client::variables`] for what is in it. Zero means "nothing here".
    pub reference: i64,
    /// Whether reading it costs something. The panel expands one level on every stop, and a
    /// scope that says it is expensive is the one not to expand without being asked.
    pub expensive: bool,
}

/// One variable, already reduced to the two strings a row is made of.
#[derive(Clone, Debug, PartialEq)]
pub struct Variable {
    pub name: String,
    pub value: String,
    /// The adapter's word for the type, when it offered one.
    pub type_name: Option<String>,
    /// Non-zero when this has children worth asking for. This is the one piece of the protocol's
    /// own bookkeeping that survives into the public surface, because there is no substitute: it
    /// is the only way to ask what is inside a struct.
    pub reference: i64,
}

/// One thread of the debuggee.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadInfo {
    pub id: i64,
    pub name: String,
}

/// The handful of things read off the adapter's answer to `initialize` rather than assumed.
///
/// Read rather than assumed because the protocol says to: `configurationDone` in particular is a
/// request a client must not send unless the adapter claimed it, and an adapter sent one it never
/// claimed is entitled to fail the session over it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// Whether `configurationDone` is expected at the end of the configuration sequence.
    pub configuration_done: bool,
    /// Whether `terminate` exists as a polite alternative to `disconnect`.
    pub terminate: bool,
    /// Whether `evaluate` is willing to be asked about the thing under the mouse.
    pub evaluate_for_hovers: bool,
}

/// What the capabilities object on an `initialize` reply says, read by name.
///
/// Every member is optional and every absence means "no", which is why this reads rather than
/// deserialises: an adapter that grows a capability we have never heard of must not cost us the
/// three we have.
fn capabilities_from(body: Option<&Value>) -> Capabilities {
    let flag = |name: &str| {
        body.and_then(|b| b.get(name)).and_then(Value::as_bool).unwrap_or(false)
    };
    Capabilities {
        configuration_done: flag("supportsConfigurationDoneRequest"),
        terminate: flag("supportsTerminateRequest"),
        evaluate_for_hovers: flag("supportsEvaluateForHovers"),
    }
}

// ---- Finding an adapter ----------------------------------------------------------------------

/// A program that speaks DAP, and the arguments that make it speak it on stdio.
///
/// A pair rather than a single string, and public rather than built only by [`find_adapter`],
/// because the settings file is going to hand one of these over: `gdb -i=dap` is a program and an
/// argument, `codelldb --port` is another shape again, and a discovered adapter and a configured
/// one have to be the same kind of thing or the configured one becomes a special case running
/// through the whole session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl AdapterCommand {
    /// One from a command line the user wrote, as a settings entry will supply it.
    pub fn from_argv(argv: &[String]) -> Option<AdapterCommand> {
        let (program, args) = argv.split_first()?;
        Some(AdapterCommand { program: program.clone(), args: args.to_vec() })
    }

    /// What to call it in a sentence: the program, without the path it was found at.
    pub fn name(&self) -> String {
        Path::new(&self.program)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&self.program)
            .to_string()
    }
}

/// The adapters this editor knows how to find, in the words the "install one of these" line will
/// use. Kept beside [`find_adapter`] so that the list somebody is told to install is the list
/// that was actually looked for — two lists would drift, and the one that drifted would be the
/// one in the message.
pub const ADAPTERS_WANTED: &[&str] = &["lldb-dap", "gdb 14 or newer"];

/// The debug adapter this machine has, if it has one.
///
/// Looked for in the order the design lays out: `lldb-dap` on `PATH` first, because it is the one
/// that needs no arguments and no version check; then, on macOS, whatever `xcrun` says — it ships
/// with the Xcode command-line tools and is not on anybody's `PATH`; then `gdb`, but only if it
/// is new enough to have DAP built in.
///
/// `None` is an ordinary answer and not a failure: see [`ADAPTERS_WANTED`] for what to say about
/// it. Wave 2 puts a settings override in front of all of this, which is why nothing here caches
/// — it is asked once when a session starts, and a debugger installed while the editor is open
/// should be found by the next start rather than the next restart.
pub fn find_adapter() -> Option<AdapterCommand> {
    if let Some(found) = on_path("lldb-dap") {
        return Some(AdapterCommand {
            program: found.to_string_lossy().into_owned(),
            args: Vec::new(),
        });
    }
    if let Some(found) = xcrun_adapter("lldb-dap") {
        return Some(AdapterCommand { program: found, args: Vec::new() });
    }
    if on_path("gdb").is_some() && gdb_speaks_dap(&ask_version("gdb")?) {
        return Some(AdapterCommand {
            program: "gdb".to_string(),
            // `-i=dap` and not `--interpreter=dap`: both work, and this is the spelling gdb's own
            // documentation uses for it.
            args: vec!["-i=dap".to_string()],
        });
    }
    None
}

/// Where a program is on `PATH`, if it is on it at all.
///
/// Written out rather than left to the spawn to discover, because discovery here is a *choice*:
/// three candidates are tried in order and the first that exists wins, so "does this exist" has
/// to be answerable without starting anything. Directories that do not read are skipped rather
/// than reported — an unreadable entry in somebody's `PATH` is their business, and it is not a
/// reason to refuse to debug.
fn on_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
        // Windows spells its executables with an extension, and `PATHEXT` is the list of them.
        // `.exe` is the only one an adapter would plausibly be, and guessing further would mean
        // finding `lldb-dap.txt` and trying to run it.
        #[cfg(windows)]
        {
            let candidate = directory.join(format!("{program}.exe"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// What `xcrun -f <tool>` says, when it says something that exists.
///
/// macOS only, and deliberately: `xcrun` is how the Xcode command-line tools are addressed, and
/// the tools it names live inside an `.app` bundle nobody would think to put on `PATH`. The
/// answer is checked against the filesystem before it is believed, because `xcrun` prints a path
/// for tools it merely *expects* to be there and a client that took its word would spawn a
/// program that does not exist and report the failure as a broken debugger.
#[cfg(target_os = "macos")]
fn xcrun_adapter(tool: &str) -> Option<String> {
    let output = Command::new("xcrun").arg("-f").arg(tool).stderr(Stdio::null()).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let found = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if found.is_empty() || !Path::new(&found).is_file() {
        return None;
    }
    Some(found)
}

/// The same question on a machine that has no `xcrun`, which is every machine that is not a Mac.
#[cfg(not(target_os = "macos"))]
fn xcrun_adapter(_tool: &str) -> Option<String> {
    None
}

/// What a program says when asked for its version, or nothing if asking did not work.
fn ask_version(program: &str) -> Option<String> {
    let output =
        Command::new(program).arg("--version").stderr(Stdio::null()).output().ok()?;
    String::from_utf8(output.stdout).ok()
}

/// Whether this gdb has DAP built into it, which is to say whether it is 14 or newer.
///
/// A gate rather than an attempt, because an older gdb given `-i=dap` does not fail: it starts,
/// complains about an unknown interpreter, and sits there — which from here is indistinguishable
/// from an adapter that is thinking. Refusing a gdb we cannot read the version of is the same
/// decision: it is better to say "install one of these" than to hang on a program that was never
/// going to answer.
pub fn gdb_speaks_dap(version_output: &str) -> bool {
    gdb_major(version_output).is_some_and(|major| major >= 14)
}

/// The major version out of a `gdb --version` banner.
///
/// Read from the end of the first line, which is where every gdb build puts it: plain builds say
/// `GNU gdb (GDB) 14.2`, and distributions push their own packaging in front of it —
/// `GNU gdb (Ubuntu 13.1-2ubuntu2) 13.1`. Scanning backwards for the last word that starts with a
/// digit finds the real version in both, where scanning forwards finds the packaging. A banner
/// with no number in it at all reads as `None`, and `None` is refused rather than guessed at.
fn gdb_major(version_output: &str) -> Option<u32> {
    let line = version_output.lines().next()?;
    for word in line.split_whitespace().rev() {
        let word = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        if !word.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let digits: String = word.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(major) = digits.parse::<u32>() {
            return Some(major);
        }
    }
    None
}

// ---- The wire --------------------------------------------------------------------------------

/// What the reader thread forwards. Whole JSON messages and nothing else: unlike `lsp.rs`, this
/// thread makes no decisions, because every decision a debug session makes needs the writer and
/// the writer belongs to the frame loop.
enum Incoming {
    Message(Value),
    /// The adapter's own stderr, a line at a time. Drained rather than discarded: an adapter that
    /// cannot find the executable it was pointed at says so here and nowhere else, and a session
    /// that silently fails to start is the worst failure this module has.
    Noise(String),
    /// The adapter went away, with the sentence to say about it.
    Gone(String),
}

/// What one request still out was asking. Kept on the client rather than shared with the reader
/// thread the way `lsp.rs` shares its map, because here it is only ever read in one place: the
/// thread forwards, and [`Client::poll`] matches.
enum Ask {
    Initialize,
    Launch,
    ConfigurationDone,
    Breakpoints { path: PathBuf },
    Continue { thread: i64 },
    Threads,
    StackTrace,
    Scopes,
    Variables,
    Evaluate,
    /// A request whose success says nothing worth reporting — a step, a pause, a disconnect. Its
    /// *failure* still is, which is why it is tracked at all: the alternative is a step key that
    /// does nothing and never says why.
    Acknowledged { command: String },
}

/// A debug session: one adapter process, one wire, one lifecycle.
pub struct Client {
    /// What to call it in a sentence shown to the user.
    pub name: String,
    /// The process, when there is one. `None` in the tests, where the other end of the pipes is a
    /// thread speaking a script — the wire is what this module is, and a process is only one way
    /// to get a pair of pipes onto it.
    child: Option<Child>,
    sink: Box<dyn Write + Send>,
    rx: Receiver<Incoming>,
    /// A second sender for that same channel, held for the stderr thread. Made at construction
    /// because an `mpsc::Receiver` cannot be asked for one later, and `None` afterwards — or from
    /// the start, for a client built over a plain reader, which has no process and no stderr.
    spare_sender: Option<Sender<Incoming>>,
    /// The client numbers its own messages, from one, and every message it sends takes the next
    /// number — requests and the answers to reverse requests alike. The adapter numbers its own
    /// separately; the two sequences never meet, which is why responses are matched on
    /// `request_seq` and not on `seq`.
    next_seq: i64,
    pending: HashMap<i64, Ask>,
    capabilities: Capabilities,
    /// Whether the answer to `initialize` has arrived. Until it has, nothing may be launched:
    /// this is the first half of the ordering this module exists to get right.
    handshook: bool,
    /// Whether the `initialized` event has arrived. Until it has, no breakpoint may be set and no
    /// configuration may be declared done: the second half.
    announced: bool,
    launched: bool,
    configured: bool,
    /// The launch arguments waiting for the handshake to land.
    waiting_launch: Option<Value>,
    /// Breakpoints waiting for the `initialized` event, one entry per file, last word winning.
    waiting_breakpoints: Vec<(PathBuf, Vec<usize>)>,
    /// Whether somebody asked for `configurationDone` before it could be sent.
    waiting_configuration_done: bool,
    /// A sentence about a write that did not work, waiting for the next [`Self::poll`] to report
    /// it. The request methods cannot report it themselves — they are called from a keypress and
    /// return an id, not an event — and a broken pipe noticed and dropped is a session that looks
    /// alive and answers nothing.
    trouble: Option<String>,
    dead: bool,
}

impl Client {
    /// Starts an adapter and opens the handshake.
    ///
    /// `cwd` is the project root, which is what an adapter resolves a relative source path
    /// against. The launch's own working directory is a separate thing and is asked for
    /// separately — see [`Self::launch`].
    pub fn start(adapter: &AdapterCommand, cwd: &Path) -> Result<Client, String> {
        let mut child = Command::new(&adapter.program)
            .args(&adapter.args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Piped and drained rather than inherited: an adapter writing to stderr would print
            // over the editor's own screen. Piped rather than nulled, unlike `lsp.rs`, because
            // what an adapter says here is often the only account of why a session never started.
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("{}: {e}", adapter.program))?;
        let stdin = child.stdin.take().ok_or("the adapter has no stdin")?;
        let stdout = child.stdout.take().ok_or("the adapter has no stdout")?;
        let stderr = child.stderr.take();
        let name = adapter.name();
        let mut client = Client::over(&name, Box::new(stdin), stdout)?;
        client.child = Some(child);
        if let Some(stderr) = stderr {
            client.drain_stderr(stderr);
        }
        Ok(client)
    }

    /// The same client over any pair of streams. This is where [`Self::start`] and the tests
    /// meet: a debug adapter is a thing that reads framed JSON and writes framed JSON, and
    /// nothing below this line cares whether the other end is a process or a script.
    fn over(
        name: &str,
        sink: Box<dyn Write + Send>,
        source: impl Read + Send + 'static,
    ) -> Result<Client, String> {
        let (tx, rx) = mpsc::channel();
        let spare_sender = Some(tx.clone());
        let reader_name = name.to_string();
        std::thread::spawn(move || read_loop(BufReader::new(source), tx, reader_name));
        let mut client = Client {
            name: name.to_string(),
            child: None,
            sink,
            rx,
            spare_sender,
            next_seq: 1,
            pending: HashMap::new(),
            capabilities: Capabilities::default(),
            handshook: false,
            announced: false,
            launched: false,
            configured: false,
            waiting_launch: None,
            waiting_breakpoints: Vec::new(),
            waiting_configuration_done: false,
            trouble: None,
            dead: false,
        };
        client.initialize()?;
        Ok(client)
    }

    /// Forwards the adapter's stderr onto the same channel, a line at a time.
    ///
    /// A thread of its own because a pipe nobody reads fills up, and an adapter blocked writing
    /// its complaint is an adapter that has stopped answering — the failure would look like a
    /// hang rather than like the error it is.
    ///
    /// It goes down the same channel as everything else so that the order is preserved: an
    /// adapter's complaint about a missing library and the `terminated` event that follows it
    /// belong in one sequence, and two channels would let the frame loop read them in either.
    fn drain_stderr(&mut self, stderr: std::process::ChildStderr) {
        // The spare sender kept aside at construction, because there is no way to ask an
        // `mpsc::Receiver` for a second one after the fact. Taken rather than cloned so that a
        // client can only ever grow one stderr thread.
        let Some(tx) = self.spare_sender.take() else { return };
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                let Ok(line) = line else { return };
                if tx.send(Incoming::Noise(line)).is_err() {
                    return;
                }
            }
        });
    }

    /// The handshake. Says what this client can count and what it cannot do.
    ///
    /// `linesStartAt1` and `columnsStartAt1` are the two that matter most here, and they are
    /// asked for rather than converted: everything else in this codebase counts lines from one
    /// when it talks to a person, so a client that asked for zero-based lines would be adding an
    /// arithmetic step to every path in and out for no gain at all.
    ///
    /// `supportsRunInTerminalRequest: false` is said out loud and is not decoration: it is the
    /// promise that makes refusing that request honest — see [`Self::refuse`].
    fn initialize(&mut self) -> Result<(), String> {
        let arguments = json!({
            "clientID": "cleecode",
            "clientName": "CleeCode",
            "adapterID": "cleecode",
            "locale": "en",
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "pathFormat": "path",
            "supportsVariableType": true,
            "supportsVariablePaging": false,
            "supportsRunInTerminalRequest": false,
            "supportsStartDebuggingRequest": false,
            "supportsProgressReporting": false,
            "supportsMemoryReferences": false,
        });
        match self.request("initialize", arguments, Ask::Initialize) {
            Some(_) => Ok(()),
            None => Err(self.trouble.take().unwrap_or_else(|| {
                format!("{} would not take the handshake", self.name)
            })),
        }
    }

    // ---- Sending -----------------------------------------------------------------------------

    /// Puts one message on the wire, numbering it as it goes.
    ///
    /// The `seq` is stamped here rather than by the callers so that there is exactly one counter
    /// and exactly one place it advances. A response to a reverse request is numbered from the
    /// same sequence as a request, which is what the protocol says: `seq` numbers *messages*, not
    /// requests.
    fn send(&mut self, mut message: Value) -> Result<(), String> {
        let seq = self.next_seq;
        self.next_seq += 1;
        if let Some(object) = message.as_object_mut() {
            object.insert("seq".to_string(), json!(seq));
        }
        let text = serde_json::to_string(&message).map_err(|e| e.to_string())?;
        self.sink.write_all(&frame(&text)).map_err(|e| e.to_string())?;
        self.sink.flush().map_err(|e| e.to_string())
    }

    /// Sends a request and returns the `seq` it went out with, which is what its answer will name
    /// in `request_seq`. Nothing here waits for that answer: the frame loop must not stop.
    fn request(&mut self, command: &str, arguments: Value, ask: Ask) -> Option<i64> {
        if self.dead {
            return None;
        }
        let seq = self.next_seq;
        let message = json!({"type": "request", "command": command, "arguments": arguments});
        match self.send(message) {
            Ok(()) => {
                self.pending.insert(seq, ask);
                Some(seq)
            }
            Err(e) => {
                self.trouble = Some(format!("{}: {e}", self.name));
                None
            }
        }
    }

    // ---- The lifecycle -----------------------------------------------------------------------

    /// Says what to debug. Sent as soon as the handshake lands, and held until then.
    ///
    /// The ordering is the one thing about DAP that clients get wrong, so it is worth writing
    /// down. There are two "ready" signals and they are different messages: the *response* to
    /// `initialize` says the adapter has read our capabilities, and the `initialized` *event*
    /// says it is ready to be told about breakpoints. The specification's blessed sequence is
    /// therefore: send `launch` once the initialize response arrives, and send `setBreakpoints`
    /// and then `configurationDone` once the initialized event arrives — which for most adapters
    /// means after the launch, because the launch is what makes them ready to be configured.
    ///
    /// Doing it the other way round — configuring first and launching last — also works on some
    /// adapters and deadlocks on the ones that only emit `initialized` in response to a launch,
    /// which is why this module picks the order that works everywhere rather than the order that
    /// reads more naturally.
    ///
    /// `stopOnEntry` is false: a debugger that stops before `main` on every start is a debugger
    /// whose first keystroke is always the same one. The breakpoints the user set are where they
    /// wanted to stop.
    pub fn launch(&mut self, program: &Path, args: &[String], cwd: &Path) {
        let arguments = json!({
            "program": program.to_string_lossy(),
            "args": args,
            "cwd": cwd.to_string_lossy(),
            "stopOnEntry": false,
        });
        if self.handshook {
            self.send_launch(arguments);
        } else {
            self.waiting_launch = Some(arguments);
        }
    }

    fn send_launch(&mut self, arguments: Value) {
        if self.launched {
            return;
        }
        self.launched = true;
        let _ = self.request("launch", arguments, Ask::Launch);
    }

    /// Replaces every breakpoint in one file. The lines are 1-based, as everything on this
    /// surface is.
    ///
    /// Whole-file rather than one at a time because that is what the request is: DAP has no "add
    /// a breakpoint", only "here is the complete list for this source", and an adapter told about
    /// one line forgets the others. The application's breakpoint map is already whole-file, so
    /// the two fit without either having to keep a second copy.
    ///
    /// Answered by [`Event::Breakpoints`] keyed on the same path. Before the `initialized` event
    /// arrives the request is held rather than sent, and held per file so that a user toggling
    /// the same line twice while the adapter starts sends one list and not two.
    pub fn set_breakpoints(&mut self, path: &Path, lines: &[usize]) {
        if !self.announced {
            self.waiting_breakpoints.retain(|(held, _)| held != path);
            self.waiting_breakpoints.push((path.to_path_buf(), lines.to_vec()));
            return;
        }
        self.send_breakpoints(path, lines);
    }

    fn send_breakpoints(&mut self, path: &Path, lines: &[usize]) {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        let arguments = json!({
            "source": {"path": path.to_string_lossy(), "name": name},
            "breakpoints": lines.iter().map(|line| json!({"line": line})).collect::<Vec<_>>(),
            // The protocol also has a bare `lines` array carrying the same numbers. It is
            // deprecated, and sending both would mean two statements of the same fact that can
            // disagree — so this sends the one that is current and lets an adapter old enough to
            // want the other be the adapter we do not support.
            "sourceModified": false,
        });
        let ask = Ask::Breakpoints { path: path.to_path_buf() };
        let _ = self.request("setBreakpoints", arguments, ask);
    }

    /// Declares the configuration finished, which is what lets the debuggee start running.
    ///
    /// Public because it is part of the protocol's vocabulary, and guarded because this module
    /// sends it itself as soon as the `initialized` event has been followed by the breakpoints:
    /// the guard is what makes an application that also calls it a no-op rather than a session
    /// with two `configurationDone`s in it, which some adapters treat as an error.
    ///
    /// Sent only when the adapter claimed `supportsConfigurationDoneRequest`. That is the
    /// protocol's rule and not caution: an adapter that did not claim it considers the
    /// configuration finished when the last configuration request is answered, and one more
    /// request afterwards is one it never agreed to receive.
    pub fn configuration_done(&mut self) {
        if !self.announced {
            self.waiting_configuration_done = true;
            return;
        }
        if self.configured || !self.capabilities.configuration_done {
            return;
        }
        self.configured = true;
        let _ = self.request("configurationDone", json!({}), Ask::ConfigurationDone);
    }

    /// Everything held back for the `initialized` event, in the order the protocol wants it:
    /// breakpoints first, and only then the word that the configuration is complete.
    fn flush_configuration(&mut self) {
        let held = std::mem::take(&mut self.waiting_breakpoints);
        for (path, lines) in held {
            self.send_breakpoints(&path, &lines);
        }
        self.waiting_configuration_done = false;
        self.configuration_done();
    }

    // ---- Running -----------------------------------------------------------------------------

    /// Runs on. Named with the underscore because `continue` is a keyword, and named `continue_`
    /// rather than `resume` because the word on the wire and the word in the menu are both
    /// "continue" and a third name for the same thing helps nobody.
    pub fn continue_(&mut self, thread: i64) -> Option<i64> {
        self.request("continue", json!({"threadId": thread}), Ask::Continue { thread })
    }

    /// Steps over one line.
    pub fn next(&mut self, thread: i64) -> Option<i64> {
        self.step("next", thread)
    }

    /// Steps into the call under the cursor.
    pub fn step_in(&mut self, thread: i64) -> Option<i64> {
        self.step("stepIn", thread)
    }

    /// Runs to the end of the current frame.
    pub fn step_out(&mut self, thread: i64) -> Option<i64> {
        self.step("stepOut", thread)
    }

    /// Stops a running debuggee where it happens to be.
    pub fn pause(&mut self, thread: i64) -> Option<i64> {
        self.step("pause", thread)
    }

    fn step(&mut self, command: &str, thread: i64) -> Option<i64> {
        let ask = Ask::Acknowledged { command: command.to_string() };
        self.request(command, json!({"threadId": thread}), ask)
    }

    // ---- Asking ------------------------------------------------------------------------------

    /// Asks for one thread's frames. Answered by [`Event::StackTrace`] under the id returned.
    pub fn stack_trace(&mut self, thread: i64) -> Option<i64> {
        // Bounded rather than asked for whole. `levels: 0` means "all of them", and a program
        // that recursed itself into a stack overflow — which is exactly the program somebody is
        // debugging — would answer with a hundred thousand frames for a panel that can show
        // thirty. The top of a stack is what anybody reads.
        let arguments = json!({"threadId": thread, "startFrame": 0, "levels": 64});
        self.request("stackTrace", arguments, Ask::StackTrace)
    }

    /// Asks what groups of variables one frame has. Answered by [`Event::Scopes`].
    pub fn scopes(&mut self, frame: i64) -> Option<i64> {
        self.request("scopes", json!({"frameId": frame}), Ask::Scopes)
    }

    /// Asks what is inside one reference — a scope's contents, or a struct's fields. Answered by
    /// [`Event::Variables`].
    pub fn variables(&mut self, reference: i64) -> Option<i64> {
        self.request("variables", json!({"variablesReference": reference}), Ask::Variables)
    }

    /// Asks what an expression comes to, in the context of one frame. Answered by
    /// [`Event::Evaluated`], or by [`Event::Failed`] when the adapter cannot read it — which is
    /// an ordinary outcome for a watch on a variable that is not in scope yet.
    ///
    /// `frame` is optional because the protocol allows the question without one, and a global
    /// expression asked before anything has stopped is a fair question.
    pub fn evaluate(&mut self, expression: &str, frame: Option<i64>) -> Option<i64> {
        let mut arguments = json!({"expression": expression, "context": "watch"});
        if let (Some(object), Some(frame)) = (arguments.as_object_mut(), frame) {
            object.insert("frameId".to_string(), json!(frame));
        }
        self.request("evaluate", arguments, Ask::Evaluate)
    }

    /// Asks what threads exist. Answered by [`Event::Threads`].
    pub fn threads(&mut self) -> Option<i64> {
        self.request("threads", json!({}), Ask::Threads)
    }

    // ---- Ending ------------------------------------------------------------------------------

    /// Ends the session. `terminate` decides what happens to the debuggee: killed with the
    /// session, or left running on its own.
    pub fn disconnect(&mut self, terminate: bool) -> Option<i64> {
        let ask = Ask::Acknowledged { command: "disconnect".to_string() };
        self.request("disconnect", json!({"terminateDebuggee": terminate}), ask)
    }

    /// Asks the adapter to go away, then stops waiting for it.
    ///
    /// The same bargain `lsp.rs` makes on shutdown, for the same reason: the polite ending is a
    /// round trip, this runs while the user is closing something, and an orphaned `lldb-dap`
    /// still holding a stopped process is a far worse outcome than an impolite exit.
    pub fn stop(&mut self) {
        let _ = self.disconnect(true);
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.dead = true;
    }

    // ---- Reading -----------------------------------------------------------------------------

    /// Drains everything the adapter has said since the last frame, answers what has to be
    /// answered, and returns what the application should hear about.
    ///
    /// This is where the session actually runs, and it takes `&mut self` for a reason: three of
    /// the things that arrive here have to be *replied to* — the handshake answer starts the
    /// launch, the `initialized` event sends the breakpoints, and a reverse request has to be
    /// refused out loud — and the writer lives on this thread. Doing it in the reader thread the
    /// way `lsp.rs` does would mean handing a second thread the pipe.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(incoming) = self.rx.try_recv() {
            match incoming {
                Incoming::Message(value) => self.digest(value, &mut out),
                Incoming::Noise(line) => {
                    out.push(Event::Output { category: "stderr".to_string(), text: line })
                }
                Incoming::Gone(reason) => {
                    self.dead = true;
                    out.push(Event::Dead { reason });
                }
            }
        }
        // A write that failed at any point since the last frame — from a keypress or from the
        // digesting just above — is reported here and only here, because this is the one place
        // that has somewhere to report it to.
        if let Some(reason) = self.trouble.take() {
            self.dead = true;
            out.push(Event::Dead { reason });
        }
        out
    }

    /// The three kinds of message that share this wire, told apart by their `type`.
    fn digest(&mut self, value: Value, out: &mut Vec<Event>) {
        match value.get("type").and_then(Value::as_str) {
            Some("event") => self.digest_event(&value, out),
            Some("response") => self.digest_response(&value, out),
            // A request travelling the wrong way. It is a real part of the protocol and not a
            // mistake — see [`Self::refuse`] for why it cannot simply be ignored.
            Some("request") => self.refuse(&value),
            // Anything else is a message shaped like nothing in the specification. Dropped: the
            // framing is what keeps the stream in step, not the contents.
            _ => {}
        }
    }

    fn digest_event(&mut self, value: &Value, out: &mut Vec<Event>) {
        let Some(name) = value.get("event").and_then(Value::as_str) else { return };
        let body = value.get("body");
        match name {
            "initialized" => {
                self.announced = true;
                out.push(Event::Initialized);
                self.flush_configuration();
            }
            "stopped" => out.push(Event::Stopped {
                thread: body.and_then(|b| b.get("threadId")).and_then(Value::as_i64),
                reason: text_at(body, "reason").unwrap_or_else(|| "stopped".to_string()),
                description: text_at(body, "description"),
                path: body
                    .and_then(|b| b.pointer("/source/path"))
                    .and_then(Value::as_str)
                    .map(PathBuf::from),
                line: body.and_then(|b| b.get("line")).and_then(Value::as_i64).map(as_line),
            }),
            "continued" => {
                let thread =
                    body.and_then(|b| b.get("threadId")).and_then(Value::as_i64).unwrap_or(0);
                out.push(Event::Continued { thread });
            }
            "exited" => {
                let code =
                    body.and_then(|b| b.get("exitCode")).and_then(Value::as_i64).unwrap_or(0);
                out.push(Event::Exited { code });
            }
            "terminated" => out.push(Event::Terminated),
            "output" => {
                let Some(text) = text_at(body, "output") else { return };
                // The protocol's own default when an adapter does not say which stream it is.
                let category = text_at(body, "category").unwrap_or_else(|| "console".to_string());
                out.push(Event::Output { category, text });
            }
            // `process`, `thread`, `module`, `breakpoint`, `capabilities` and the rest are real
            // and are not read here. Wave 1 runs a program and stops it; an event nobody acts on
            // would be an event somebody has to maintain.
            _ => {}
        }
    }

    fn digest_response(&mut self, value: &Value, out: &mut Vec<Event>) {
        // Matched on `request_seq` and not on `seq`, and matched here rather than by arrival
        // order: adapters answer out of order and interleave events between a question and its
        // answer, both of which are allowed and both of which happen.
        let Some(id) = value.get("request_seq").and_then(Value::as_i64) else { return };
        let Some(ask) = self.pending.remove(&id) else {
            // An answer to a question we are not holding: a duplicate, or a response to something
            // sent before a restart. Dropped rather than guessed at.
            return;
        };
        let command = value
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("that request")
            .to_string();
        if !value.get("success").and_then(Value::as_bool).unwrap_or(false) {
            let message = text_at(Some(value), "message")
                .unwrap_or_else(|| format!("{} refused {command}", self.name));
            out.push(Event::Failed { id, command, message });
            return;
        }
        let body = value.get("body");
        match ask {
            Ask::Initialize => {
                self.capabilities = capabilities_from(body);
                self.handshook = true;
                // The first half of the ordering, carried out: the launch was held for exactly
                // this moment. See [`Self::launch`] for why this is the moment.
                if let Some(arguments) = self.waiting_launch.take() {
                    self.send_launch(arguments);
                }
            }
            Ask::Launch | Ask::ConfigurationDone => {
                // Nothing to report. A launch that worked is announced by the program running —
                // output, then a stop — and an invented "it started" event would be one more
                // thing for the application to ignore.
            }
            Ask::Breakpoints { path } => {
                out.push(Event::Breakpoints { path, breakpoints: breakpoints_from(body) })
            }
            Ask::Continue { thread } => {
                // Synthesised, and deliberately. The specification says an adapter is *not*
                // expected to send a `continued` event for execution that a request obviously
                // resumed, so a client that waited for one would leave the stopped line
                // highlighted on a program that is long gone.
                out.push(Event::Continued { thread })
            }
            Ask::Threads => out.push(Event::Threads { id, threads: threads_from(body) }),
            Ask::StackTrace => out.push(Event::StackTrace { id, frames: frames_from(body) }),
            Ask::Scopes => out.push(Event::Scopes { id, scopes: scopes_from(body) }),
            Ask::Variables => out.push(Event::Variables { id, variables: variables_from(body) }),
            Ask::Evaluate => out.push(Event::Evaluated {
                id,
                value: text_at(body, "result").unwrap_or_default(),
                reference: body
                    .and_then(|b| b.get("variablesReference"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0),
            }),
            // A step or a pause that worked. The `stopped` event that follows is the news.
            Ask::Acknowledged { .. } => {}
        }
    }

    /// Answers a request the adapter sent to *us*.
    ///
    /// Reverse requests are the part of DAP that has no counterpart in this codebase's LSP
    /// client, and dropping one is not harmless: `runInTerminal` is an adapter asking the editor
    /// to start the debuggee in a terminal it controls, and several adapters wait for the answer
    /// before doing anything else. A dropped request is a session that never starts, with nothing
    /// on screen to say why.
    ///
    /// So it is refused, out loud, in a sentence. The handshake already said
    /// `supportsRunInTerminalRequest: false`, which makes this the second half of a promise
    /// rather than a surprise: an adapter asking anyway is asking for something we said we did
    /// not have, and "no" is the honest and immediate answer.
    fn refuse(&mut self, request: &Value) {
        let command =
            request.get("command").and_then(Value::as_str).unwrap_or("that").to_string();
        let request_seq = request.get("seq").and_then(Value::as_i64).unwrap_or(0);
        let message = format!("CleeCode does not offer {command}");
        let reply = json!({
            "type": "response",
            "request_seq": request_seq,
            "success": false,
            "command": command,
            "message": message,
        });
        if let Err(e) = self.send(reply) {
            self.trouble = Some(format!("{}: {e}", self.name));
        }
    }

    /// What the adapter said it can do. Settled during the handshake and remembered, because it
    /// decides which questions are worth asking.
    pub fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Whether the handshake has landed.
    pub fn handshook(&self) -> bool {
        self.handshook
    }

    /// Whether the adapter has said it is ready to be configured.
    pub fn announced(&self) -> bool {
        self.announced
    }

    /// Whether this session is over, one way or another.
    pub fn is_dead(&self) -> bool {
        self.dead
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The reader thread. Everything the adapter says arrives here and leaves whole.
///
/// It decides nothing, which is the difference from `lsp.rs`'s reader: a debug session's
/// decisions all need the writer, and the writer belongs to the frame loop.
fn read_loop(mut reader: BufReader<impl Read>, tx: Sender<Incoming>, name: String) {
    loop {
        match read_message(&mut reader) {
            Ok(Some(text)) => {
                // A frame that is not JSON costs itself and nothing else. The framing is what
                // keeps the stream in step; one unreadable message is not a reason to stop
                // reading the next.
                let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
                if tx.send(Incoming::Message(value)).is_err() {
                    return;
                }
            }
            Ok(None) => {
                let _ = tx.send(Incoming::Gone(format!("{name} closed its output")));
                return;
            }
            Err(e) => {
                let _ = tx.send(Incoming::Gone(format!("{name}: {e}")));
                return;
            }
        }
    }
}

// ---- Reading the bodies ------------------------------------------------------------------------

/// One string member of a body, when it is there and is a string.
fn text_at(body: Option<&Value>, member: &str) -> Option<String> {
    body?.get(member)?.as_str().map(str::to_string)
}

/// A line number off the wire. Already 1-based, because the handshake asked for that; clamped at
/// one because a zero would be an adapter ignoring what it agreed to and a zero in a gutter is a
/// line that does not exist.
fn as_line(raw: i64) -> usize {
    raw.max(1) as usize
}

fn breakpoints_from(body: Option<&Value>) -> Vec<Breakpoint> {
    let Some(list) = body.and_then(|b| b.get("breakpoints")).and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .map(|item| Breakpoint {
            // An absent `verified` reads as "no". The protocol makes it required, so an adapter
            // that left it out has told us nothing, and "we do not know that it will stop there"
            // is the truthful reading of nothing.
            verified: item.get("verified").and_then(Value::as_bool).unwrap_or(false),
            line: item.get("line").and_then(Value::as_i64).map(as_line),
            message: item.get("message").and_then(Value::as_str).map(str::to_string),
        })
        .collect()
}

fn threads_from(body: Option<&Value>) -> Vec<ThreadInfo> {
    let Some(list) = body.and_then(|b| b.get("threads")).and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|item| {
            Some(ThreadInfo {
                // A thread with no id is a thread nothing can be asked about, so it is dropped
                // rather than listed: a row in the panel that answers no question is worse than
                // no row.
                id: item.get("id").and_then(Value::as_i64)?,
                name: item
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("thread")
                    .to_string(),
            })
        })
        .collect()
}

fn frames_from(body: Option<&Value>) -> Vec<Frame> {
    let Some(list) = body.and_then(|b| b.get("stackFrames")).and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|item| {
            Some(Frame {
                id: item.get("id").and_then(Value::as_i64)?,
                name: item.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                // Absent for a frame with no source — a library without symbols, a signal
                // trampoline. Listed anyway: a stack with holes in it is still the stack, and
                // the frames above and below it are what somebody came to read.
                path: item.pointer("/source/path").and_then(Value::as_str).map(PathBuf::from),
                line: item.get("line").and_then(Value::as_i64).map(as_line).unwrap_or(1),
                column: item
                    .get("column")
                    .and_then(Value::as_i64)
                    .map(|c| c.max(1) as usize)
                    .unwrap_or(1),
            })
        })
        .collect()
}

fn scopes_from(body: Option<&Value>) -> Vec<Scope> {
    let Some(list) = body.and_then(|b| b.get("scopes")).and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .map(|item| Scope {
            name: item.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
            reference: item
                .get("variablesReference")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            expensive: item.get("expensive").and_then(Value::as_bool).unwrap_or(false),
        })
        .collect()
}

fn variables_from(body: Option<&Value>) -> Vec<Variable> {
    let Some(list) = body.and_then(|b| b.get("variables")).and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .map(|item| Variable {
            name: item.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
            value: item.get("value").and_then(Value::as_str).unwrap_or("").to_string(),
            type_name: item.get("type").and_then(Value::as_str).map(str::to_string),
            reference: item
                .get("variablesReference")
                .and_then(Value::as_i64)
                .unwrap_or(0),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// Everything the scripted adapter saw, in the order it saw it. Requests and the client's own
    /// replies alike, because the order between them is exactly what several of these tests are
    /// about.
    type Log = Arc<Mutex<Vec<Value>>>;

    /// Wires a client to a scripted adapter running on a thread.
    ///
    /// A script and not a debugger: these tests are about the wire, the lifecycle and the
    /// matching, and a real `lldb-dap` would make them a test of whether this machine has Xcode.
    /// The script is handed every message the client sends and a sink to answer on, and returns
    /// whether the adapter stays up — returning `false` is how a test kills it mid-request.
    fn scripted(
        mut script: impl FnMut(&Value, &mut dyn FnMut(Value)) -> bool + Send + 'static,
    ) -> (Client, Log) {
        let (to_adapter, from_client) = std::io::pipe().expect("a pipe towards the adapter");
        let (to_client, from_adapter) = std::io::pipe().expect("a pipe back to the client");
        let seen: Log = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(to_adapter);
            let mut writer = from_adapter;
            while let Ok(Some(text)) = read_message(&mut reader) {
                let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
                log.lock().unwrap().push(value.clone());
                let mut answer = |message: Value| {
                    let text = serde_json::to_string(&message).unwrap();
                    let _ = writer.write_all(&frame(&text));
                    let _ = writer.flush();
                };
                if !script(&value, &mut answer) {
                    return;
                }
            }
        });
        let client = Client::over("fake-dap", Box::new(from_client), to_client)
            .expect("the handshake goes out");
        (client, seen)
    }

    /// One successful response to the request numbered `request_seq`.
    fn response(request_seq: i64, command: &str, body: Value) -> Value {
        let mut message = json!({
            "type": "response",
            "request_seq": request_seq,
            "success": true,
            "command": command,
        });
        if !body.is_null() {
            message["body"] = body;
        }
        message
    }

    /// One event from the adapter.
    fn adapter_event(name: &str, body: Value) -> Value {
        let mut message = json!({"type": "event", "event": name});
        if !body.is_null() {
            message["body"] = body;
        }
        message
    }

    fn command_of(message: &Value) -> String {
        message.get("command").and_then(Value::as_str).unwrap_or("").to_string()
    }

    fn seq_of(message: &Value) -> i64 {
        message.get("seq").and_then(Value::as_i64).unwrap_or(0)
    }

    /// The commands the adapter was *asked* for, in order. The client's own replies to reverse
    /// requests are in the log too and are filtered out here, because they are not questions.
    fn commands(seen: &Log) -> Vec<String> {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|m| m.get("type").and_then(Value::as_str) == Some("request"))
            .map(command_of)
            .collect()
    }

    /// Runs the frame loop the way the application will, until `want` is satisfied or the clock
    /// runs out. Every wait in this file is bounded: a lifecycle that deadlocks has to fail a
    /// test, not hang the suite.
    fn pump_until(client: &mut Client, want: impl Fn(&Event) -> bool) -> Vec<Event> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut collected = Vec::new();
        while Instant::now() < deadline {
            let mut satisfied = false;
            for event in client.poll() {
                satisfied |= want(&event);
                collected.push(event);
            }
            if satisfied {
                return collected;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("nothing satisfied the wait in five seconds; saw {collected:?}");
    }

    /// Keeps pumping into a batch already collected, until something in it satisfies `want`.
    /// Needed because these events are drained once: a wait that started over would be waiting
    /// for a second copy of something that has already arrived.
    fn pump_onwards(
        client: &mut Client,
        collected: &mut Vec<Event>,
        want: impl Fn(&Event) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if collected.iter().any(&want) {
                return;
            }
            collected.extend(client.poll());
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("nothing satisfied the wait in five seconds; saw {collected:?}");
    }

    /// The same, waiting on the adapter's side of the conversation rather than on the client's.
    fn pump_until_seen(client: &mut Client, seen: &Log, wanted: usize) -> Vec<Event> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut collected = Vec::new();
        while Instant::now() < deadline {
            collected.extend(client.poll());
            if commands(seen).len() >= wanted {
                return collected;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("the adapter only ever saw {:?}", commands(seen));
    }

    /// The handshake is one framed message, and it says the things the rest of this module
    /// depends on having said — 1-based lines above all, since every position that arrives later
    /// is read as though this was asked for.
    #[test]
    fn the_handshake_goes_out_as_one_framed_request() {
        let (mut client, seen) = scripted(|_message, _answer| true);
        let _ = pump_until_seen(&mut client, &seen, 1);
        let log = seen.lock().unwrap();
        assert_eq!(log.len(), 1, "one request is one message");
        let handshake = &log[0];
        assert_eq!(handshake.get("type").and_then(Value::as_str), Some("request"));
        assert_eq!(command_of(handshake), "initialize");
        assert_eq!(seq_of(handshake), 1, "the client numbers itself from one");
        let arguments = handshake.get("arguments").expect("the handshake carries arguments");
        assert_eq!(arguments.get("adapterID").and_then(Value::as_str), Some("cleecode"));
        assert_eq!(arguments.get("linesStartAt1").and_then(Value::as_bool), Some(true));
        assert_eq!(arguments.get("columnsStartAt1").and_then(Value::as_bool), Some(true));
        assert_eq!(arguments.get("pathFormat").and_then(Value::as_str), Some("path"));
        assert_eq!(
            arguments.get("supportsRunInTerminalRequest").and_then(Value::as_bool),
            Some(false),
            "refusing that request later is only honest if it was declined here"
        );
    }

    /// Two messages that arrive in one read still come out as two. The framing is what separates
    /// them, and a reader that took a chunk of the pipe for a message would have been broken by
    /// every adapter that answers quickly.
    #[test]
    fn two_answers_in_one_chunk_both_arrive() {
        let mut wire = Vec::new();
        wire.extend(frame(
            r#"{"type":"response","request_seq":1,"success":true,"command":"initialize","body":{}}"#,
        ));
        wire.extend(frame(r#"{"type":"event","event":"initialized"}"#));
        let (tx, rx) = mpsc::channel();
        read_loop(BufReader::new(&wire[..]), tx, "stub".to_string());
        let arrived: Vec<Incoming> = rx.into_iter().collect();
        assert_eq!(arrived.len(), 3, "two messages and the end of the stream");
        match &arrived[0] {
            Incoming::Message(value) => {
                assert_eq!(value.get("command").and_then(Value::as_str), Some("initialize"))
            }
            _ => panic!("the response has to come first"),
        }
        match &arrived[1] {
            Incoming::Message(value) => {
                assert_eq!(value.get("event").and_then(Value::as_str), Some("initialized"))
            }
            _ => panic!("the event was swallowed by the response before it"),
        }
        match &arrived[2] {
            Incoming::Gone(reason) => assert!(reason.contains("stub"), "{reason}"),
            _ => panic!("the end of the stream has to be announced"),
        }
    }

    /// The ordering this module exists to get right, checked from the adapter's side: the launch
    /// waits for the answer to the handshake, and the breakpoints and the word "configured" wait
    /// for the `initialized` event — which this adapter, like `lldb-dap`, only sends once it has
    /// been told what to launch.
    #[test]
    fn the_launch_waits_for_the_handshake_and_the_breakpoints_wait_for_the_event() {
        let (mut client, seen) = scripted(|message, answer| {
            let seq = seq_of(message);
            match command_of(message).as_str() {
                "initialize" => answer(response(
                    seq,
                    "initialize",
                    json!({"supportsConfigurationDoneRequest": true}),
                )),
                "launch" => {
                    answer(response(seq, "launch", Value::Null));
                    answer(adapter_event("initialized", Value::Null));
                }
                "setBreakpoints" => answer(response(
                    seq,
                    "setBreakpoints",
                    json!({"breakpoints": [{"verified": true, "line": 12}]}),
                )),
                "configurationDone" => answer(response(seq, "configurationDone", Value::Null)),
                _ => {}
            }
            true
        });

        // Both asked for before the adapter has said anything at all, which is how the
        // application will do it: the user set a breakpoint and pressed Start.
        client.set_breakpoints(Path::new("/tmp/main.rs"), &[12]);
        client.launch(Path::new("/tmp/a.out"), &[], Path::new("/tmp"));

        let mut events = pump_until_seen(&mut client, &seen, 4);
        assert_eq!(
            commands(&seen),
            vec!["initialize", "launch", "setBreakpoints", "configurationDone"],
            "the order is the whole point of the lifecycle"
        );
        assert!(
            events.contains(&Event::Initialized),
            "the initialized event is surfaced as well as acted on: {events:?}"
        );
        pump_onwards(&mut client, &mut events, |e| matches!(e, Event::Breakpoints { .. }));
        let found = events
            .iter()
            .find_map(|e| match e {
                Event::Breakpoints { path, breakpoints } => Some((path.clone(), breakpoints.clone())),
                _ => None,
            })
            .expect("the breakpoints came back");
        assert_eq!(found.0, PathBuf::from("/tmp/main.rs"), "keyed by the file it was asked about");
        assert_eq!(found.1, vec![Breakpoint { verified: true, line: Some(12), message: None }]);
    }

    /// A breakpoint the adapter would not place says so, and says why. Drawing it the same as a
    /// verified one would be the editor promising a stop that will never happen.
    #[test]
    fn a_breakpoint_that_could_not_be_placed_says_so() {
        let (mut client, seen) = scripted(|message, answer| {
            let seq = seq_of(message);
            match command_of(message).as_str() {
                "initialize" => {
                    answer(response(seq, "initialize", json!({})));
                    answer(adapter_event("initialized", Value::Null));
                }
                "setBreakpoints" => answer(response(
                    seq,
                    "setBreakpoints",
                    json!({"breakpoints": [
                        {"verified": false, "message": "no code at that line"}
                    ]}),
                )),
                _ => {}
            }
            true
        });
        let _ = pump_until(&mut client, |e| matches!(e, Event::Initialized));
        client.set_breakpoints(Path::new("/tmp/main.rs"), &[999]);
        let events = pump_until(&mut client, |e| matches!(e, Event::Breakpoints { .. }));
        let Some(Event::Breakpoints { breakpoints, .. }) =
            events.iter().find(|e| matches!(e, Event::Breakpoints { .. }))
        else {
            panic!("no answer for the breakpoints: {events:?}")
        };
        assert_eq!(breakpoints.len(), 1);
        assert!(!breakpoints[0].verified);
        assert_eq!(breakpoints[0].message.as_deref(), Some("no code at that line"));
        // The adapter here never claimed `supportsConfigurationDoneRequest`, and so was never
        // sent one: the protocol says a client must not send what was not offered.
        assert!(
            !commands(&seen).iter().any(|c| c == "configurationDone"),
            "an unclaimed request must not be sent: {:?}",
            commands(&seen)
        );
    }

    /// Two questions, answered in the other order, with an event in the middle. All three are
    /// allowed and all three happen; matching on `request_seq` is what makes them harmless.
    #[test]
    fn an_answer_that_arrives_late_still_finds_its_question() {
        let mut held: Option<i64> = None;
        let (mut client, _seen) = scripted(move |message, answer| {
            let seq = seq_of(message);
            match command_of(message).as_str() {
                "initialize" => answer(response(seq, "initialize", json!({}))),
                // Held back on purpose, so the answer to the *later* question overtakes it.
                "threads" => held = Some(seq),
                "stackTrace" => {
                    answer(response(
                        seq,
                        "stackTrace",
                        json!({"stackFrames": [
                            {"id": 7, "name": "main", "line": 12, "column": 3,
                             "source": {"path": "/tmp/main.rs"}}
                        ]}),
                    ));
                    answer(adapter_event(
                        "output",
                        json!({"category": "stdout", "output": "hello\n"}),
                    ));
                    if let Some(threads) = held.take() {
                        answer(response(
                            threads,
                            "threads",
                            json!({"threads": [{"id": 1, "name": "main"}]}),
                        ));
                    }
                }
                _ => {}
            }
            true
        });

        let threads_id = client.threads().expect("the question goes out");
        let stack_id = client.stack_trace(1).expect("the question goes out");
        let events = pump_until(&mut client, |e| matches!(e, Event::Threads { .. }));

        let interesting: Vec<&Event> = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    Event::Threads { .. } | Event::StackTrace { .. } | Event::Output { .. }
                )
            })
            .collect();
        assert!(
            matches!(interesting[0], Event::StackTrace { id, .. } if *id == stack_id),
            "the later question was answered first: {interesting:?}"
        );
        assert!(
            matches!(interesting[1], Event::Output { .. }),
            "an event between a question and its answer is not a problem: {interesting:?}"
        );
        assert!(
            matches!(interesting[2], Event::Threads { id, .. } if *id == threads_id),
            "the held answer found its own question: {interesting:?}"
        );
        let Event::StackTrace { frames, .. } = interesting[0] else { unreachable!() };
        assert_eq!(
            frames[0],
            Frame {
                id: 7,
                name: "main".to_string(),
                path: Some(PathBuf::from("/tmp/main.rs")),
                line: 12,
                column: 3,
            }
        );
    }

    /// An adapter may ask the client for something. Silence is the one answer that is not allowed:
    /// several adapters wait on the reply and never start the program without it.
    #[test]
    fn a_request_from_the_adapter_is_refused_rather_than_dropped() {
        let (mut client, seen) = scripted(|message, answer| {
            if command_of(message) == "initialize"
                && message.get("type").and_then(Value::as_str) == Some("request")
            {
                answer(response(seq_of(message), "initialize", json!({})));
                answer(json!({
                    "seq": 41,
                    "type": "request",
                    "command": "runInTerminal",
                    "arguments": {"kind": "integrated", "args": ["/tmp/a.out"]},
                }));
            }
            true
        });

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut reply = None;
        while Instant::now() < deadline && reply.is_none() {
            let _ = client.poll();
            reply = seen
                .lock()
                .unwrap()
                .iter()
                .find(|m| {
                    m.get("type").and_then(Value::as_str) == Some("response")
                        && command_of(m) == "runInTerminal"
                })
                .cloned();
            std::thread::sleep(Duration::from_millis(2));
        }
        let reply = reply.expect("the adapter would have waited forever");
        assert_eq!(reply.get("success").and_then(Value::as_bool), Some(false));
        assert_eq!(reply.get("request_seq").and_then(Value::as_i64), Some(41));
        let message = reply.get("message").and_then(Value::as_str).unwrap_or_default();
        assert!(message.contains("runInTerminal"), "the refusal names what it refused: {message}");
    }

    /// A stop arrives typed, with the thread to ask about and the reason to show. The place is
    /// carried when the adapter volunteered it — this one does, some do not, and the ones that do
    /// not are wave 2's `stackTrace`.
    #[test]
    fn a_stop_arrives_with_its_thread_and_its_reason() {
        let (mut client, _seen) = scripted(|message, answer| {
            if command_of(message) == "initialize" {
                answer(response(seq_of(message), "initialize", json!({})));
                answer(adapter_event(
                    "stopped",
                    json!({
                        "threadId": 3,
                        "reason": "breakpoint",
                        "description": "stopped at breakpoint 1",
                        "line": 12,
                        "source": {"path": "/tmp/main.rs"},
                    }),
                ));
            }
            true
        });
        let events = pump_until(&mut client, |e| matches!(e, Event::Stopped { .. }));
        let stop = events.iter().find(|e| matches!(e, Event::Stopped { .. })).unwrap();
        assert_eq!(
            stop,
            &Event::Stopped {
                thread: Some(3),
                reason: "breakpoint".to_string(),
                description: Some("stopped at breakpoint 1".to_string()),
                path: Some(PathBuf::from("/tmp/main.rs")),
                line: Some(12),
            }
        );
    }

    /// A `continue` that worked is a program that is running, and the editor has to stop drawing
    /// the line it was stopped on. The protocol does not require a `continued` event after a
    /// request that obviously resumed, so the client says it itself.
    #[test]
    fn a_continue_that_worked_says_the_program_is_running_again() {
        let (mut client, _seen) = scripted(|message, answer| {
            let seq = seq_of(message);
            match command_of(message).as_str() {
                "initialize" => answer(response(seq, "initialize", json!({}))),
                // Answered, and with no `continued` event, exactly as the specification permits.
                "continue" => answer(response(seq, "continue", json!({"allThreadsContinued": true}))),
                _ => {}
            }
            true
        });
        let _ = client.continue_(3).expect("the request goes out");
        let events = pump_until(&mut client, |e| matches!(e, Event::Continued { .. }));
        assert!(events.contains(&Event::Continued { thread: 3 }), "{events:?}");
    }

    /// A request the adapter refuses is an answer, not a silence: a watch on a variable that is
    /// not in scope is the commonest one and is not a fault.
    #[test]
    fn a_refused_request_arrives_with_what_the_adapter_said() {
        let (mut client, _seen) = scripted(|message, answer| {
            let seq = seq_of(message);
            match command_of(message).as_str() {
                "initialize" => answer(response(seq, "initialize", json!({}))),
                "evaluate" => answer(json!({
                    "type": "response",
                    "request_seq": seq,
                    "success": false,
                    "command": "evaluate",
                    "message": "no variable named 'nope'",
                })),
                _ => {}
            }
            true
        });
        let id = client.evaluate("nope", Some(7)).expect("the question goes out");
        let events = pump_until(&mut client, |e| matches!(e, Event::Failed { .. }));
        assert!(
            events.contains(&Event::Failed {
                id,
                command: "evaluate".to_string(),
                message: "no variable named 'nope'".to_string(),
            }),
            "{events:?}"
        );
    }

    /// An adapter that dies with a question outstanding says so within a frame. The failure mode
    /// this rules out is the one that cannot be recovered from: an editor waiting forever on an
    /// answer that is never coming.
    #[test]
    fn an_adapter_that_dies_mid_request_says_so_rather_than_hanging() {
        let (mut client, _seen) = scripted(|message, answer| {
            match command_of(message).as_str() {
                "initialize" => {
                    answer(response(seq_of(message), "initialize", json!({})));
                    true
                }
                // The adapter goes away holding the question, which is what a crash looks like
                // from this side of the pipe.
                "stackTrace" => false,
                _ => true,
            }
        });
        let _ = client.stack_trace(1).expect("the question goes out");
        let events = pump_until(&mut client, |e| matches!(e, Event::Dead { .. }));
        let Some(Event::Dead { reason }) = events.iter().find(|e| matches!(e, Event::Dead { .. }))
        else {
            panic!("no death was announced: {events:?}")
        };
        assert!(reason.contains("fake-dap"), "the sentence names the adapter: {reason}");
        assert!(client.is_dead(), "a dead session does not go on answering");
    }

    /// The version gate, read the way gdb actually writes its banner. 13 is refused because
    /// `-i=dap` on it starts a program that never answers, which is worse than saying no.
    #[test]
    fn gdb_speaks_dap_only_from_fourteen_onwards() {
        assert!(!gdb_speaks_dap("GNU gdb (GDB) 13.2\nCopyright (C) 2023\n"));
        assert!(gdb_speaks_dap("GNU gdb (GDB) 14.0\n"));
        assert!(gdb_speaks_dap("GNU gdb (GDB) 15.1\n"));
        // The packaging a distribution puts in front of the version must not be read as the
        // version: this one is gdb 13, not gdb 2.
        assert!(!gdb_speaks_dap("GNU gdb (Ubuntu 13.1-2ubuntu2) 13.1\n"));
        assert!(gdb_speaks_dap("GNU gdb (Debian 14.2-1) 14.2\n"));
        assert_eq!(gdb_major("GNU gdb (GDB) 14.2\n"), Some(14));
        // Apple's ancient stub, and a banner with no number at all. Both refused rather than
        // guessed at.
        assert!(!gdb_speaks_dap("GNU gdb 6.3.50-20050815 (Apple version gdb-1824)\n"));
        assert_eq!(gdb_major("something that is not a version banner\n"), None);
        assert_eq!(gdb_major(""), None);
    }

    /// Discovery answers rather than failing, on a machine with an adapter and on one without.
    /// The list of what to install is checked here too, because it is the sentence somebody with
    /// neither will read.
    #[test]
    fn looking_for_an_adapter_is_an_answer_either_way() {
        assert_eq!(ADAPTERS_WANTED, &["lldb-dap", "gdb 14 or newer"]);
        if let Some(found) = find_adapter() {
            assert!(!found.program.is_empty(), "a found adapter has a program to run");
            assert!(!found.name().is_empty(), "and a name to put in a sentence");
        }
        // A settings entry is the same kind of thing as a discovered one, which is what lets
        // wave 2 put one in front of the other without reshaping anything.
        let configured = AdapterCommand::from_argv(&[
            "/opt/codelldb/adapter".to_string(),
            "--port".to_string(),
            "0".to_string(),
        ])
        .expect("a command line with a program in it");
        assert_eq!(configured.name(), "adapter");
        assert_eq!(configured.args, vec!["--port".to_string(), "0".to_string()]);
        assert_eq!(AdapterCommand::from_argv(&[]), None, "an empty line names no adapter");
    }

    /// Scopes and variables come back as the rows the panel draws, with the one number that has
    /// to survive the translation: the reference that asks what is inside.
    #[test]
    fn a_frames_contents_arrive_as_the_rows_that_will_be_drawn() {
        let (mut client, _seen) = scripted(|message, answer| {
            let seq = seq_of(message);
            match command_of(message).as_str() {
                "initialize" => answer(response(seq, "initialize", json!({}))),
                "scopes" => answer(response(
                    seq,
                    "scopes",
                    json!({"scopes": [
                        {"name": "Locals", "variablesReference": 100, "expensive": false},
                        {"name": "Registers", "variablesReference": 101, "expensive": true}
                    ]}),
                )),
                "variables" => answer(response(
                    seq,
                    "variables",
                    json!({"variables": [
                        {"name": "count", "value": "3", "type": "i32", "variablesReference": 0},
                        {"name": "row", "value": "Row { .. }", "variablesReference": 200}
                    ]}),
                )),
                "evaluate" => answer(response(
                    seq,
                    "evaluate",
                    json!({"result": "42", "type": "i32", "variablesReference": 0}),
                )),
                _ => {}
            }
            true
        });

        let scopes_id = client.scopes(7).expect("the question goes out");
        let events = pump_until(&mut client, |e| matches!(e, Event::Scopes { .. }));
        let Some(Event::Scopes { id, scopes }) =
            events.into_iter().find(|e| matches!(e, Event::Scopes { .. }))
        else {
            panic!("no scopes came back")
        };
        assert_eq!(id, scopes_id);
        assert_eq!(scopes[0], Scope { name: "Locals".into(), reference: 100, expensive: false });
        assert!(scopes[1].expensive, "an expensive scope says so, so nothing expands it uninvited");

        client.variables(100).expect("the question goes out");
        let events = pump_until(&mut client, |e| matches!(e, Event::Variables { .. }));
        let Some(Event::Variables { variables, .. }) =
            events.into_iter().find(|e| matches!(e, Event::Variables { .. }))
        else {
            panic!("no variables came back")
        };
        assert_eq!(variables[0].type_name.as_deref(), Some("i32"));
        assert_eq!(variables[0].reference, 0, "a leaf has nothing to ask about");
        assert_eq!(variables[1].reference, 200, "and a struct carries the handle to its fields");

        let evaluate_id = client.evaluate("2 * 21", Some(7)).expect("the question goes out");
        let events = pump_until(&mut client, |e| matches!(e, Event::Evaluated { .. }));
        assert!(
            events.contains(&Event::Evaluated {
                id: evaluate_id,
                value: "42".to_string(),
                reference: 0,
            }),
            "{events:?}"
        );
    }

    /// The end of a program, in the two messages that describe it. They are different facts —
    /// a process can exit while the adapter stays up — so both are surfaced.
    #[test]
    fn a_program_that_ends_says_so_twice_and_means_two_things() {
        let (mut client, _seen) = scripted(|message, answer| {
            if command_of(message) == "initialize" {
                answer(response(seq_of(message), "initialize", json!({})));
                answer(adapter_event("output", json!({"output": "done\n"})));
                answer(adapter_event("exited", json!({"exitCode": 3})));
                answer(adapter_event("terminated", Value::Null));
            }
            true
        });
        let events = pump_until(&mut client, |e| matches!(e, Event::Terminated));
        assert!(
            events.contains(&Event::Output {
                category: "console".to_string(),
                text: "done\n".to_string(),
            }),
            "an adapter that named no category means the console: {events:?}"
        );
        assert!(events.contains(&Event::Exited { code: 3 }), "{events:?}");
        assert!(events.contains(&Event::Terminated), "{events:?}");
    }

    /// A message shaped like nothing in the specification costs itself and nothing else — the
    /// framing is what keeps the stream in step, so the next message is still read.
    #[test]
    fn a_message_that_makes_no_sense_is_stepped_over() {
        let mut wire = Vec::new();
        wire.extend(frame("this is not json at all"));
        wire.extend(frame(r#"{"type":"nonsense"}"#));
        wire.extend(frame(r#"{"type":"event","event":"terminated"}"#));
        let (tx, rx) = mpsc::channel();
        read_loop(BufReader::new(&wire[..]), tx, "stub".to_string());
        let arrived: Vec<Incoming> = rx.into_iter().collect();
        assert_eq!(arrived.len(), 3, "the unreadable one never left the reader");
        match &arrived[1] {
            Incoming::Message(value) => {
                assert_eq!(value.get("event").and_then(Value::as_str), Some("terminated"))
            }
            _ => panic!("the good message still arrives"),
        }
    }

    /// A program that does not exist is an answer, not a crash — the same bargain the language
    /// server client makes when a server is not installed.
    #[test]
    fn an_adapter_that_is_not_installed_is_an_answer_not_a_crash() {
        let adapter = AdapterCommand {
            program: "cleecode-no-such-debug-adapter".to_string(),
            args: Vec::new(),
        };
        let started = Client::start(&adapter, Path::new("."));
        let Err(e) = started else { panic!("an adapter that does not exist must not start") };
        assert!(e.contains("cleecode-no-such-debug-adapter"), "{e}");
    }
}
