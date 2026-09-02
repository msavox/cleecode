//! `clee --mcp`: CleeCode as a Model Context Protocol server, so one implementation serves
//! Claude Code, codex, opencode and gemini instead of four integrations.
//!
//! The awkward part is not the protocol, it is the topology. `clee --mcp` is a *child* of the
//! agent — the agent spawns it as a stdio server — while everything worth exposing (which files
//! are open, where the cursor is, what the language server said) lives in the editor process.
//! The two have to meet somewhere, and that somewhere is the filesystem:
//!
//! - the editor makes a session directory named after its pid and hands it to every shell it
//!   spawns as `CLEE_SESSION`. An agent started in a pane inherits it, so its `clee --mcp`
//!   inherits it too — nothing to discover, and no ambiguity when two CleeCodes are running;
//! - the editor publishes `state.json` into that directory, atomically and throttled;
//! - the server drops `requests/req-<n>.json` files there, which the editor picks up in its
//!   poll loop, acts on, and deletes.
//!
//! Files rather than a socket because that is this codebase's own pattern already — `wsnap`
//! hands interpreter snapshots across exactly this way — and because a Unix socket would leave
//! Windows out, which the rest of the program takes care not to do.
//!
//! One deliberate omission in this first version: no tool writes to a buffer. Reading has no
//! blast radius, so it is exposed generously; writing needs the user's consent, and the UI for
//! asking is not built yet.

use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The name the editor exports and the server reads. Fixed, because it is the whole handshake.
pub const SESSION_ENV: &str = "CLEE_SESSION";

const STATE_FILE: &str = "state.json";
const REQUESTS_DIR: &str = "requests";

/// The state file's shape, bumped if a field ever changes meaning rather than being added.
const STATE_VERSION: u32 = 1;

/// How much selected text travels. A selection is context for an agent, not a file transfer, and
/// a state file rewritten four times a second must not carry a megabyte of it.
const MAX_SELECTION: usize = 4 * 1024;

/// How many diagnostics are published. A project mid-refactor can have thousands, and past the
/// first few hundred they stop being an answer to "what is wrong here".
const MAX_DIAGNOSTICS: usize = 500;

/// The floor between two state writes: four a second is far faster than anyone can read and slow
/// enough that typing does not turn into disk traffic.
const STATE_INTERVAL: Duration = Duration::from_millis(250);

/// How often the request directory is looked at. In the same band as the editor's other polls,
/// which is fast enough that "open this file" feels immediate.
const REQUEST_INTERVAL: Duration = Duration::from_millis(200);

/// The protocol version answered when the client proposes one this server does not know.
const DEFAULT_PROTOCOL: &str = "2025-06-18";

/// Versions this server is happy to speak. MCP asks the server to echo the client's version when
/// it can, and to name its own when it cannot; all three of these differ only in ways that do not
/// touch a tools-only server.
const KNOWN_PROTOCOLS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18"];

/// What every tool says when it cannot find the editor. Written as a sentence rather than a code
/// because the reader is a language model relaying it to a person.
const NO_SESSION: &str =
    "not inside a CleeCode session — start this agent inside a CleeCode terminal";

// ---- What crosses the gap ------------------------------------------------------------------

/// Everything the editor publishes about itself.
///
/// Compared for equality on the editor side, so that a session where nothing has moved writes
/// nothing at all: the derive is load-bearing, and so is the ordering imposed on the diagnostics
/// before they land here — a `HashMap` iterated twice would otherwise look like a change.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default, PartialEq)]
pub struct State {
    /// The project root, so an agent can make sense of a relative path.
    pub root: String,
    /// Every open buffer that has a file behind it, in tab order.
    pub open_files: Vec<String>,
    /// The buffer being typed in, if any.
    pub active: Option<Active>,
    pub diagnostics: Vec<Diagnostic>,
}

/// The active buffer. Lines and columns are 1-based here and everywhere else this module
/// speaks, because the thing on the other end thinks in `path:line` the way a compiler prints it.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Active {
    pub path: String,
    pub line: usize,
    pub column: usize,
    /// The selected text, when there is a selection and it is small enough to be context.
    pub selection: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Diagnostic {
    pub path: String,
    pub line: usize,
    /// "error", "warning", "info" or "hint" — the language server's severity, spelled out.
    pub severity: String,
    pub message: String,
}

/// One thing the server asks the editor to do. Only `open` exists in this version.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct Request {
    pub action: String,
    pub path: String,
    pub line: Option<usize>,
}

/// The envelope actually written to disk: the state plus the two fields that describe the file
/// rather than the editor. Flattened, so the server can deserialize a plain [`State`] out of it
/// and ignore the rest.
#[derive(serde::Serialize)]
struct Envelope<'a> {
    version: u32,
    generation: u64,
    #[serde(flatten)]
    state: &'a State,
}

// ---- Where it lives ------------------------------------------------------------------------

/// The directory every session's directory sits in.
///
/// The temp dir and not the config dir, for the same reason `wsnap::snapshot_dir` chose it:
/// this is runtime state that means nothing after the process dies, and the config dir is for
/// what the user decided, not for what the editor is doing right now. It is also what keeps a
/// session out of the project's `git status` when a sandboxed config lives inside the project —
/// the pty drivers run exactly that way, and the Git panel driver caught the session dir being
/// staged along with the user's files.
pub fn sessions_root() -> Option<PathBuf> {
    Some(std::env::temp_dir().join("cleecode-sessions"))
}

/// This process's session directory.
///
/// Derived from the pid rather than stored, exactly as `wsnap::snapshot_dir` is, so that the
/// terminal panel can name it while spawning a shell without a handle on anything.
pub fn session_dir() -> Option<PathBuf> {
    sessions_root().map(|dir| dir.join(std::process::id().to_string()))
}

// ---- The editor's side ---------------------------------------------------------------------

/// The editor's end of the bridge: a directory, a throttle, and the memory of what was last said.
pub struct Session {
    dir: PathBuf,
    /// Counts the states actually written. Not used to decide anything here — it is for whoever
    /// reads the file and wants to know whether it moved since they last looked.
    generation: u64,
    /// The last state written, so an idle editor writes nothing rather than the same bytes over.
    last: Option<State>,
    wrote_at: Instant,
    polled_at: Instant,
}

impl Session {
    /// Opens a session directory for this process, sweeping away the ones left by CleeCodes that
    /// are no longer running.
    ///
    /// `None` when there is no config directory to put it in — a machine with no home. Nothing
    /// else in the editor depends on this succeeding.
    pub fn start() -> Option<Session> {
        let root = sessions_root()?;
        std::fs::create_dir_all(&root).ok()?;
        sweep_orphans(&root);
        let dir = root.join(std::process::id().to_string());
        // A pid comes round again eventually, and the leftovers of the last process to hold this
        // one would look like our own state to a server that read them.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(REQUESTS_DIR)).ok()?;
        // Both clocks start in the past, so the first frame publishes rather than waiting a
        // quarter of a second to say anything at all.
        let past = Instant::now() - STATE_INTERVAL;
        Some(Session { dir, generation: 0, last: None, wrote_at: past, polled_at: past })
    }

    /// Whether enough time has passed to be worth building a state to publish. Asked before the
    /// state is assembled, because assembling one copies every path and the selected text.
    pub fn due_for_state(&self) -> bool {
        self.wrote_at.elapsed() >= STATE_INTERVAL
    }

    /// Writes the state out, unless it is the one already on disk.
    pub fn publish(&mut self, state: State) {
        self.wrote_at = Instant::now();
        if self.last.as_ref() == Some(&state) {
            return;
        }
        self.generation += 1;
        let envelope = Envelope { version: STATE_VERSION, generation: self.generation, state: &state };
        let Ok(text) = serde_json::to_string(&envelope) else { return };
        // Atomic, like every other file this program writes: a server reading halfway through a
        // save would otherwise get a truncated document and call the editor broken.
        if crate::settings::write_atomic(&self.dir.join(STATE_FILE), text.as_bytes()).is_ok() {
            self.last = Some(state);
        }
    }

    pub fn due_for_requests(&self) -> bool {
        self.polled_at.elapsed() >= REQUEST_INTERVAL
    }

    /// Every request waiting, oldest first, removed from the directory as it is read.
    ///
    /// A file that will not parse is deleted along with the rest: leaving it would mean retrying
    /// the same failure five times a second for the life of the session.
    pub fn take_requests(&mut self) -> Vec<Request> {
        self.polled_at = Instant::now();
        let dir = self.dir.join(REQUESTS_DIR);
        let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
        let mut found: Vec<(u128, PathBuf)> = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            // `write_atomic` leaves its scratch file as a dotfile beside the target, so matching
            // the prefix is also what keeps a half-written request from being read.
            let Some(number) = name.strip_prefix("req-").and_then(|rest| rest.strip_suffix(".json"))
            else {
                continue;
            };
            let Ok(number) = number.parse::<u128>() else { continue };
            found.push((number, entry.path()));
        }
        found.sort_by_key(|(number, _)| *number);
        let mut requests = Vec::new();
        for (_, path) in found {
            let text = std::fs::read_to_string(&path);
            let _ = std::fs::remove_file(&path);
            if let Ok(text) = text
                && let Ok(request) = serde_json::from_str::<Request>(&text)
            {
                requests.push(request);
            }
        }
        requests
    }
}

impl Drop for Session {
    /// The directory belongs to this process and goes with it. Done in `Drop` rather than on the
    /// way out of `main` so that it also happens when the editor is closed by a panic the loop
    /// managed to contain.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Removes the session directories of processes that are no longer alive.
///
/// A CleeCode killed with `SIGKILL` never runs its `Drop`, and without this the directory would
/// sit there for good. The process table is read through `sysinfo`, the way the rest of the
/// program reads it, so this works the same on macOS, Linux and Windows.
fn sweep_orphans(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    let mine = std::process::id();
    let mut sys = sysinfo::System::new();
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing(),
    );
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|text| text.parse::<u32>().ok()) else { continue };
        if pid == mine || sys.process(sysinfo::Pid::from_u32(pid)).is_some() {
            continue;
        }
        let _ = std::fs::remove_dir_all(entry.path());
    }
}

// ---- The server's side ---------------------------------------------------------------------

/// The next request file name's number.
///
/// Milliseconds since the epoch with a per-process counter in the low digits: increasing, so the
/// editor can execute requests in the order they were made, and distinct even when one agent
/// asks for several things inside the same millisecond.
fn next_request_number() -> u128 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed) as u128;
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or(0);
    millis * 1000 + seq % 1000
}

/// Leaves a request in a session directory for the editor to find.
pub fn write_request(dir: &Path, request: &Request) -> std::io::Result<()> {
    let requests = dir.join(REQUESTS_DIR);
    std::fs::create_dir_all(&requests)?;
    let text = serde_json::to_string(request)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let path = requests.join(format!("req-{}.json", next_request_number()));
    crate::settings::write_atomic(&path, text.as_bytes())
}

/// Runs the server on stdin and stdout until the client goes away.
pub fn serve_stdio() {
    let session = std::env::var_os(SESSION_ENV).map(PathBuf::from).filter(|dir| !dir.as_os_str().is_empty());
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    // A client that closed the pipe is a client that has finished with us, not a failure to
    // report — and there is nowhere to report it to anyway.
    let _ = serve(&mut stdin.lock(), &mut stdout.lock(), session.as_deref());
}

/// The whole server, as a function over a reader and a writer, so the conversation can be tested
/// without a process.
///
/// MCP's stdio transport is newline-delimited JSON — one message per line, no `Content-Length` —
/// which is the one thing about it that differs from the LSP framing this codebase already has.
/// Reusing `lsp::frame` here would produce a server no client can talk to.
pub fn serve(
    input: &mut impl BufRead,
    output: &mut impl Write,
    session: Option<&Path>,
) -> std::io::Result<()> {
    let mut line = String::new();
    loop {
        line.clear();
        match input.read_line(&mut line) {
            // End of stream: the agent has finished. This is the normal way to stop.
            Ok(0) => return Ok(()),
            Ok(_) => {}
            // Bytes that are not UTF-8 cannot be a JSON-RPC message and cannot be answered
            // either, since the stream is no longer at a message boundary.
            Err(_) => return Ok(()),
        }
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        if let Some(reply) = handle(text, session) {
            writeln!(output, "{reply}")?;
            output.flush()?;
        }
    }
}

/// One message in, at most one message out. `None` means silence, which is the correct answer to
/// a notification.
fn handle(text: &str, session: Option<&Path>) -> Option<Value> {
    let message: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        // Malformed input is a bug in the client, not a reason to stop being a server.
        Err(e) => return Some(failure(Value::Null, -32700, &format!("parse error: {e}"))),
    };
    let Some(object) = message.as_object() else {
        return Some(failure(Value::Null, -32600, "invalid request: expected a JSON-RPC object"));
    };
    // No id, or a null one, means a notification: `notifications/initialized` and anything else
    // the client chooses to tell us are all answered the same way, by saying nothing.
    let id = object.get("id").filter(|id| !id.is_null())?.clone();
    let method = object.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => initialize(&params),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => call(&params, session),
        "ping" => json!({}),
        "" => return Some(failure(id, -32600, "invalid request: no method")),
        other => return Some(failure(id, -32601, &format!("unknown method: {other}"))),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn failure(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
    // Echo the client's version when it is one we know, name our own when it is not: that is
    // what the specification asks a server to do, and it is how a newer client is told to fall
    // back rather than being left to guess.
    let version = if KNOWN_PROTOCOLS.contains(&asked) { asked } else { DEFAULT_PROTOCOL };
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "clee", "version": env!("CARGO_PKG_VERSION") },
    })
}

/// The tools, with the descriptions the agent actually reads to decide whether to call them.
fn tools() -> Value {
    // Every schema is an object, because that is what MCP requires even when there is nothing to
    // pass; the three read-only tools simply have no properties.
    let nothing = json!({ "type": "object", "properties": {} });
    json!([
        {
            "name": "open_files",
            "description": "The files open in the CleeCode editor, in tab order, and which one \
                            is active. Use this to see what the user is working on before \
                            guessing at paths.",
            "inputSchema": nothing,
        },
        {
            "name": "selection",
            "description": "The active file in CleeCode, the cursor's line and column (1-based), \
                            and the selected text if there is a selection. This is what the user \
                            means by \"here\" or \"this\".",
            "inputSchema": nothing,
        },
        {
            "name": "diagnostics",
            "description": "The language server diagnostics CleeCode currently holds: path, \
                            line, severity and message. Optionally restricted to one file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Only diagnostics for this file. Absolute, or a suffix of \
                                        the path such as src/main.rs.",
                    },
                },
            },
        },
        {
            "name": "open_file",
            "description": "Ask CleeCode to show a file, optionally at a line. It opens beside \
                            what the user is doing and does not take the keyboard focus. \
                            Returns as soon as the request is filed, not when the file appears.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The file to show. Absolute, or relative to the project root.",
                    },
                    "line": {
                        "type": "integer",
                        "description": "1-based line to put the cursor on.",
                        "minimum": 1,
                    },
                },
                "required": ["path"],
            },
        },
    ])
}

/// `tools/call`, in the shape MCP wants it: a content list, and a flag saying whether it went
/// wrong. A tool failing is a result, not a JSON-RPC error — the error channel is for the
/// protocol, and a model needs to *read* what went wrong to do something about it.
fn call(params: &Value, session: Option<&Path>) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
    let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
    let (text, failed) = match run(name, &arguments, session) {
        Ok(text) => (text, false),
        Err(text) => (text, true),
    };
    json!({ "content": [{ "type": "text", "text": text }], "isError": failed })
}

fn run(name: &str, arguments: &Value, session: Option<&Path>) -> Result<String, String> {
    match name {
        "open_files" => {
            let state = read_state(session)?;
            let active = state.active.as_ref().map(|a| a.path.clone());
            render(&json!({ "root": state.root, "files": state.open_files, "active": active }))
        }
        "selection" => {
            let state = read_state(session)?;
            match state.active {
                Some(active) => render(&json!({
                    "path": active.path,
                    "line": active.line,
                    "column": active.column,
                    "selection": active.selection,
                })),
                None => render(&json!({ "active": Value::Null })),
            }
        }
        "diagnostics" => {
            let state = read_state(session)?;
            let wanted = arguments.get("path").and_then(Value::as_str).filter(|p| !p.is_empty());
            let list: Vec<&Diagnostic> = match wanted {
                // `Path::ends_with` compares whole components, so "main.rs" matches
                // "src/main.rs" and "ain.rs" matches nothing — which is what somebody typing a
                // short path means, and what a plain string suffix would get wrong.
                Some(wanted) => {
                    let wanted = Path::new(wanted);
                    state.diagnostics.iter().filter(|d| Path::new(&d.path).ends_with(wanted)).collect()
                }
                None => state.diagnostics.iter().collect(),
            };
            render(&json!({ "diagnostics": list }))
        }
        "open_file" => {
            let dir = session_with_editor(session)?;
            let path = arguments
                .get("path")
                .and_then(Value::as_str)
                .filter(|path| !path.is_empty())
                .ok_or_else(|| "open_file needs a path".to_string())?;
            let line = arguments.get("line").and_then(Value::as_u64).map(|n| n.max(1) as usize);
            let request =
                Request { action: "open".to_string(), path: path.to_string(), line };
            write_request(&dir, &request)
                .map_err(|e| format!("the request could not be left for the editor: {e}"))?;
            render(&json!({
                "status": "requested",
                "path": path,
                "line": line,
                "note": "CleeCode opens it beside the user's work without taking the keyboard.",
            }))
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

/// Pretty-printed, because what comes back is read by a model and read by a person debugging it.
fn render(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|e| format!("the answer could not be written: {e}"))
}

/// The session directory, once it is established that an editor is really there.
fn session_with_editor(session: Option<&Path>) -> Result<PathBuf, String> {
    let dir = session.ok_or_else(|| NO_SESSION.to_string())?;
    if !dir.join(STATE_FILE).exists() {
        return Err(NO_SESSION.to_string());
    }
    Ok(dir.to_path_buf())
}

fn read_state(session: Option<&Path>) -> Result<State, String> {
    let dir = session.ok_or_else(|| NO_SESSION.to_string())?;
    let text = std::fs::read_to_string(dir.join(STATE_FILE)).map_err(|_| NO_SESSION.to_string())?;
    serde_json::from_str(&text)
        .map_err(|e| format!("CleeCode's state file could not be read: {e}"))
}

// ---- Building a state --------------------------------------------------------------------

/// Trims a selection to what is worth carrying, and drops it entirely when there is none.
pub fn selection_for(text: Option<String>) -> Option<String> {
    let text = text.filter(|text| !text.is_empty())?;
    if text.len() > MAX_SELECTION { None } else { Some(text) }
}

/// Puts the diagnostics in a fixed order and caps them.
///
/// The order is what makes the "unchanged state writes nothing" rule work: they arrive out of a
/// `HashMap`, and two iterations of one are free to disagree.
pub fn tidy_diagnostics(mut list: Vec<Diagnostic>) -> Vec<Diagnostic> {
    list.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)).then(a.message.cmp(&b.message)));
    list.truncate(MAX_DIAGNOSTICS);
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Runs a conversation through the server and gives back the lines it answered with.
    fn talk(lines: &[&str], session: Option<&Path>) -> Vec<Value> {
        let input = lines.join("\n") + "\n";
        let mut reader = Cursor::new(input.into_bytes());
        let mut written: Vec<u8> = Vec::new();
        serve(&mut reader, &mut written, session).expect("the server must not fail on a closed stream");
        String::from_utf8(written)
            .expect("the server writes JSON, which is UTF-8")
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).expect("every line the server writes is JSON"))
            .collect()
    }

    fn a_state() -> State {
        State {
            root: "/proj".to_string(),
            open_files: vec!["/proj/src/main.rs".to_string(), "/proj/README.md".to_string()],
            active: Some(Active {
                path: "/proj/src/main.rs".to_string(),
                line: 12,
                column: 5,
                selection: Some("let x = 1;".to_string()),
            }),
            diagnostics: vec![Diagnostic {
                path: "/proj/src/main.rs".to_string(),
                line: 12,
                severity: "error".to_string(),
                message: "no such thing".to_string(),
            }],
        }
    }

    /// A session directory with a state file in it, in a scratch place of its own.
    fn a_session(state: &State) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "clee-mcp-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory must be creatable");
        let text = serde_json::to_string(&Envelope { version: STATE_VERSION, generation: 3, state })
            .expect("the state must serialise");
        std::fs::write(dir.join(STATE_FILE), text).expect("the state file must be writable");
        dir
    }

    /// The tool result's single text block, whatever it says.
    fn tool_text(reply: &Value) -> String {
        reply["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string()
    }

    #[test]
    fn the_handshake_answers_with_the_version_the_client_proposed() {
        let replies = talk(
            &[r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#],
            None,
        );
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0]["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(replies[0]["result"]["serverInfo"]["name"], "clee");
        assert!(replies[0]["result"]["capabilities"]["tools"].is_object());
    }

    /// A version this server has never heard of gets its own, rather than an echo that would
    /// promise a protocol it cannot speak.
    #[test]
    fn an_unknown_protocol_version_is_answered_with_our_own() {
        let replies = talk(
            &[r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2099-01-01"}}"#],
            None,
        );
        assert_eq!(replies[0]["result"]["protocolVersion"], DEFAULT_PROTOCOL);
    }

    /// A notification is answered with silence — including the one every client sends right
    /// after the handshake.
    #[test]
    fn a_notification_gets_no_reply() {
        let replies = talk(&[r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#], None);
        assert!(replies.is_empty(), "a notification must not be answered: {replies:?}");
    }

    #[test]
    fn the_tool_list_is_the_four_tools_with_schemas() {
        let replies = talk(&[r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#], None);
        let listed = replies[0]["result"]["tools"].as_array().expect("tools/list returns an array");
        let names: Vec<&str> = listed.iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(names, ["open_files", "selection", "diagnostics", "open_file"]);
        for tool in listed {
            assert_eq!(tool["inputSchema"]["type"], "object", "every schema is an object");
            assert!(tool["description"].as_str().is_some_and(|d| !d.is_empty()));
        }
    }

    #[test]
    fn ping_is_answered_with_an_empty_result() {
        let replies = talk(&[r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#], None);
        assert_eq!(replies[0]["id"], 7);
        assert!(replies[0]["result"].is_object());
    }

    #[test]
    fn the_read_only_tools_report_what_the_editor_published() {
        let state = a_state();
        let dir = a_session(&state);
        let replies = talk(
            &[
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"open_files"}}"#,
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"selection"}}"#,
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"diagnostics"}}"#,
            ],
            Some(&dir),
        );
        assert_eq!(replies.len(), 3);
        for reply in &replies {
            assert_eq!(reply["result"]["isError"], false, "{reply}");
        }

        let files: Value = serde_json::from_str(&tool_text(&replies[0])).expect("JSON out");
        assert_eq!(files["files"][1], "/proj/README.md");
        assert_eq!(files["active"], "/proj/src/main.rs");

        let selection: Value = serde_json::from_str(&tool_text(&replies[1])).expect("JSON out");
        assert_eq!(selection["line"], 12);
        assert_eq!(selection["column"], 5);
        assert_eq!(selection["selection"], "let x = 1;");

        let diagnostics: Value = serde_json::from_str(&tool_text(&replies[2])).expect("JSON out");
        assert_eq!(diagnostics["diagnostics"][0]["severity"], "error");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The filter takes a suffix of whole path components, so the short name an agent has to
    /// hand finds the file and a fragment of a name finds nothing.
    #[test]
    fn diagnostics_can_be_asked_about_one_file() {
        let dir = a_session(&a_state());
        let ask = |path: &str| {
            let line = format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"diagnostics","arguments":{{"path":"{path}"}}}}}}"#
            );
            let replies = talk(&[&line], Some(&dir));
            let out: Value = serde_json::from_str(&tool_text(&replies[0])).expect("JSON out");
            out["diagnostics"].as_array().map(Vec::len).unwrap_or(0)
        };
        assert_eq!(ask("src/main.rs"), 1);
        assert_eq!(ask("main.rs"), 1);
        assert_eq!(ask("ain.rs"), 0, "half a file name is not a file");
        assert_eq!(ask("/proj/README.md"), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `open_file` files a request and says so at once; the editor is what eventually acts on it.
    #[test]
    fn open_file_leaves_a_request_the_editor_can_read() {
        let dir = a_session(&a_state());
        let replies = talk(
            &[r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"open_file","arguments":{"path":"src/main.rs","line":40}}}"#],
            Some(&dir),
        );
        assert_eq!(replies[0]["result"]["isError"], false);
        let out: Value = serde_json::from_str(&tool_text(&replies[0])).expect("JSON out");
        assert_eq!(out["status"], "requested");

        let mut session = Session {
            dir: dir.clone(),
            generation: 0,
            last: None,
            wrote_at: Instant::now(),
            polled_at: Instant::now(),
        };
        let requests = session.take_requests();
        assert_eq!(
            requests,
            vec![Request { action: "open".into(), path: "src/main.rs".into(), line: Some(40) }]
        );
        assert!(session.take_requests().is_empty(), "a request is acted on once, not forever");
        // The struct owns the directory, and dropping it takes the scratch one with it.
        drop(session);
    }

    /// The requests come back in the order they were made, whatever order the filesystem hands
    /// the directory back in.
    #[test]
    fn requests_are_replayed_oldest_first() {
        let dir = a_session(&a_state());
        for n in 1..=3 {
            let request =
                Request { action: "open".into(), path: format!("f{n}.rs"), line: Some(n) };
            write_request(&dir, &request).expect("a request must be writable");
        }
        let mut session = Session {
            dir: dir.clone(),
            generation: 0,
            last: None,
            wrote_at: Instant::now(),
            polled_at: Instant::now(),
        };
        let paths: Vec<String> = session.take_requests().into_iter().map(|r| r.path).collect();
        assert_eq!(paths, ["f1.rs", "f2.rs", "f3.rs"]);
        drop(session);
    }

    /// Nothing on the other end. The tools have to say so in words a model can relay, and the
    /// server has to still be there for the next message.
    #[test]
    fn without_a_session_the_tools_explain_themselves_and_the_server_lives_on() {
        let replies = talk(
            &[
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"open_files"}}"#,
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"open_file","arguments":{"path":"a.rs"}}}"#,
                r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#,
            ],
            None,
        );
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0]["result"]["isError"], true);
        assert!(tool_text(&replies[0]).contains("not inside a CleeCode session"));
        assert_eq!(replies[1]["result"]["isError"], true);
        assert!(replies[2]["result"].is_object(), "the server keeps answering");
    }

    /// A session directory that exists but holds no state file is the same situation as no
    /// session at all: the editor is not there to be asked.
    #[test]
    fn a_session_directory_without_a_state_file_is_no_session() {
        let dir = std::env::temp_dir().join(format!("clee-mcp-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory must be creatable");
        let replies = talk(
            &[r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"selection"}}"#],
            Some(&dir),
        );
        assert_eq!(replies[0]["result"]["isError"], true);
        assert!(tool_text(&replies[0]).contains("not inside a CleeCode session"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_method_is_an_error_and_not_a_crash() {
        let replies = talk(
            &[
                r#"{"jsonrpc":"2.0","id":1,"method":"resources/list"}"#,
                r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#,
            ],
            None,
        );
        assert_eq!(replies[0]["error"]["code"], -32601);
        assert!(replies[1]["result"].is_object(), "the next message is still served");
    }

    #[test]
    fn an_unknown_tool_is_a_tool_error_rather_than_a_protocol_one() {
        let replies = talk(
            &[r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"rm_rf"}}"#],
            None,
        );
        assert_eq!(replies[0]["result"]["isError"], true);
        assert!(tool_text(&replies[0]).contains("unknown tool"));
    }

    /// Garbage on the wire: answered with a parse error and then forgotten. A server that died
    /// here would take the agent's whole connection with it.
    #[test]
    fn malformed_input_is_answered_and_survived() {
        let replies = talk(
            &[
                "{not json at all",
                "[1,2,3]",
                "",
                r#"{"jsonrpc":"2.0","id":5,"method":"ping"}"#,
            ],
            None,
        );
        assert_eq!(replies.len(), 3, "two complaints and one answer: {replies:?}");
        assert_eq!(replies[0]["error"]["code"], -32700);
        assert_eq!(replies[1]["error"]["code"], -32600);
        assert_eq!(replies[2]["id"], 5);
    }

    /// A message with a method nobody named is not a method to dispatch on.
    #[test]
    fn a_request_without_a_method_is_an_invalid_request() {
        let replies = talk(&[r#"{"jsonrpc":"2.0","id":1}"#], None);
        assert_eq!(replies[0]["error"]["code"], -32600);
    }

    #[test]
    fn open_file_without_a_path_says_what_is_missing() {
        let dir = a_session(&a_state());
        let replies = talk(
            &[r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"open_file","arguments":{}}}"#],
            Some(&dir),
        );
        assert_eq!(replies[0]["result"]["isError"], true);
        assert!(tool_text(&replies[0]).contains("needs a path"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Publishing is idempotent: the same state twice writes the file once, which is what keeps
    /// an idle editor from rewriting it four times a second for an afternoon.
    #[test]
    fn an_unchanged_state_is_not_written_again() {
        let dir = std::env::temp_dir().join(format!("clee-mcp-publish-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(REQUESTS_DIR)).expect("a scratch directory must exist");
        let past = Instant::now() - STATE_INTERVAL;
        let mut session =
            Session { dir: dir.clone(), generation: 0, last: None, wrote_at: past, polled_at: past };

        session.publish(a_state());
        assert_eq!(session.generation, 1);
        session.publish(a_state());
        assert_eq!(session.generation, 1, "the same state must not be written twice");

        let mut moved = a_state();
        if let Some(active) = moved.active.as_mut() {
            active.line = 13;
        }
        session.publish(moved);
        assert_eq!(session.generation, 2);

        // And what landed is readable by the server's own reader.
        let state = read_state(Some(&dir)).expect("the published state must parse");
        assert_eq!(state.active.map(|a| a.line), Some(13));
        drop(session);
        assert!(!dir.exists(), "a dropped session takes its directory with it");
    }

    /// The throttle is what the frame loop leans on: it calls every frame and expects to be told
    /// "not yet" most of the time.
    #[test]
    fn the_state_throttle_holds_between_writes() {
        let dir = std::env::temp_dir().join(format!("clee-mcp-throttle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory must exist");
        let past = Instant::now() - STATE_INTERVAL;
        let mut session =
            Session { dir: dir.clone(), generation: 0, last: None, wrote_at: past, polled_at: past };
        assert!(session.due_for_state(), "the first frame publishes");
        session.publish(a_state());
        assert!(!session.due_for_state(), "and the next one does not");
        drop(session);
    }

    /// A selection is context, not a transfer: past the cap it is left out rather than truncated
    /// into something an agent would read as the whole of it.
    #[test]
    fn an_enormous_selection_is_left_out_rather_than_cut_in_half() {
        assert_eq!(selection_for(None), None);
        assert_eq!(selection_for(Some(String::new())), None);
        assert_eq!(selection_for(Some("x".into())), Some("x".into()));
        assert_eq!(selection_for(Some("x".repeat(MAX_SELECTION + 1))), None);
    }

    #[test]
    fn diagnostics_are_ordered_so_that_an_unchanged_editor_looks_unchanged() {
        let diag = |path: &str, line: usize| Diagnostic {
            path: path.to_string(),
            line,
            severity: "error".to_string(),
            message: "boom".to_string(),
        };
        let tidied = tidy_diagnostics(vec![diag("b.rs", 1), diag("a.rs", 9), diag("a.rs", 2)]);
        let order: Vec<(String, usize)> =
            tidied.iter().map(|d| (d.path.clone(), d.line)).collect();
        assert_eq!(order, [("a.rs".to_string(), 2), ("a.rs".to_string(), 9), ("b.rs".to_string(), 1)]);

        let many: Vec<Diagnostic> = (0..MAX_DIAGNOSTICS + 50).map(|n| diag("a.rs", n)).collect();
        assert_eq!(tidy_diagnostics(many).len(), MAX_DIAGNOSTICS);
    }
}
