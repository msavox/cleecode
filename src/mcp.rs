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
//! The session directory carries one more thing, and it is the half of this that means a user
//! never has to configure anything: the registration each agent reads to know that `clee --mcp`
//! exists. See the divider "Registering the server with the agent the drawer starts" below.
//!
//! Reading has no blast radius, so it is exposed generously and every read-only tool answers out
//! of `state.json` without the editor being involved at all.
//!
//! Writing does have one, and it needs a second direction across the same gap. An `edit_buffer`
//! request is the only thing here that is *answered*: the editor drops `replies/req-<n>.json` and
//! the server, holding a synchronous tool call open, waits for it — because what it is waiting for
//! is a person deciding whether an agent may touch a buffer they have not saved. Everything else
//! is filed and forgotten, which is why every other tool returns the instant the file is on disk.

use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The name the editor exports and the server reads. Fixed, because it is the whole handshake.
pub const SESSION_ENV: &str = "CLEE_SESSION";

const STATE_FILE: &str = "state.json";
const REQUESTS_DIR: &str = "requests";

/// Where the editor's answers go. Beside the requests rather than inside them, so that reading a
/// directory to find work to do never turns up something the editor itself wrote.
const REPLIES_DIR: &str = "replies";

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

/// How often the server looks for the answer to an edit it has asked for. Ten times a second is
/// imperceptible next to the second or two a person takes to read the question, and it is a
/// directory listing rather than anything the editor has to be interrupted for.
const REPLY_INTERVAL: Duration = Duration::from_millis(100);

/// How long the answer is waited for before the tool gives up.
///
/// Two minutes because the thing being waited for is a human being reading a question, and the
/// alternative to waiting is an agent that carries on as though the edit had failed while the
/// prompt is still on screen. Long enough for somebody to come back from the kettle; short enough
/// that an agent left running against an editor nobody is sitting at eventually says so.
const REPLY_TIMEOUT: Duration = Duration::from_secs(120);

/// How much of an agent's line the status bar will carry. The bar is one line, and a sentence
/// longer than this is one that would be cut by the renderer instead — better cut here, where it
/// can be said in the tool's description that it will be.
const MAX_SAY: usize = 120;

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

/// What `edit_buffer` says when the question is still on the user's screen after two minutes.
///
/// Careful about what it does *not* claim: the edit was not refused, it was not answered, and the
/// prompt is still up — so it may yet be applied a minute from now. An agent told "it failed"
/// would retry, and a retry that lands on top of an edit that has since gone through is the one
/// outcome nobody can undo in a single keystroke.
const NO_ANSWER: &str = "the user did not answer within two minutes, so nothing has been applied \
                         yet — the question is still on their screen and the edit may still land. \
                         Read open_files or selection again before trying it a second time";

// ---- What the editor says back about an edit ------------------------------------------------

// Everything an `edit_buffer` can come back as, written here rather than in the editor and in
// English rather than in the user's language, because the reader is not the user: it is the model
// that asked, which has to decide what to do next and then say it to a person in whatever language
// they were speaking. Each of the refusals names what to do instead, since the four ways this can
// fail are four different mistakes and an agent told only "no" retries the one that never works.

/// The user, at the prompt, said no to this change.
pub fn edit_declined() -> String {
    "the user declined this change — do not retry it; ask them what they would rather have"
        .to_string()
}

/// `agent_edits = "deny"`. Not a decision made about this edit, and worth saying so: nothing the
/// agent could rephrase would get a different answer.
pub fn edit_refused_by_setting() -> String {
    "the user has turned agent edits off in CleeCode (agent_edits = \"deny\") — edit the file on \
     disk instead, and if it is dirty, leave it alone and tell them what you would have changed"
        .to_string()
}

/// Too many questions waiting to be answered at once. See `AGENT_EDIT_QUEUE` in `app.rs`.
pub fn edit_too_many(waiting: usize) -> String {
    format!(
        "there are already {waiting} edits waiting for the user to answer — wait for those to be \
         answered before asking for another"
    )
}

pub fn edit_not_open(path: &str) -> String {
    format!(
        "{path} is not open in the editor, so there is no buffer to change — edit the file on \
         disk as you normally would"
    )
}

pub fn edit_read_only(path: &str) -> String {
    format!("{path} is open read-only in the editor and cannot be changed through it")
}

pub fn edit_no_match(path: &str) -> String {
    format!(
        "old_string was not found in {path} — the buffer has moved on since you read it, so read \
         it again before trying"
    )
}

pub fn edit_many_matches(path: &str, times: usize) -> String {
    format!(
        "old_string matches {times} times in {path} — include enough of the surrounding lines to \
         make it unique"
    )
}

/// What went right, said in a way that leaves no doubt about the file on disk.
pub fn edit_applied(path: &str, line: usize) -> String {
    format!(
        "applied to the {path} buffer at line {line}. It is not saved: the buffer now differs \
         from the file on disk, and saving is the user's to do"
    )
}

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
    /// The subset of those whose buffer and file no longer agree: unsaved work, visible on the
    /// user's screen and nowhere on disk. An agent that rewrites one of these files the ordinary
    /// way destroys it, so the list is published for the same reason the warning is written into
    /// `open_files`' description — a tool that can do harm has to say where the harm is.
    ///
    /// Additive, so `STATE_VERSION` stays where it is: a server reading a state file written by an
    /// older editor sees an empty list, which is exactly what that editor was telling it.
    #[serde(default)]
    pub dirty_files: Vec<String>,
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

/// One thing the server asks the editor to do.
///
/// Tagged on `action` and not a struct with an action field, so that each thing the editor can be
/// asked for carries exactly what it needs and nothing else — an `edit` with no `line` and a
/// `say` with no `path` were both, in the flat shape this replaces, a field somebody had to
/// remember to ignore. The wire shape of an `open` is unchanged by the move, which matters:
/// a request written by the previous version and still lying in a session directory has to be
/// readable, and an action a version does not know is dropped by `take_requests` for free, since
/// serde refuses it and the file is deleted either way.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Request {
    /// Show a file, and when `end_line` is there, mark that span so the user can see what the
    /// agent means without having to be told a number.
    Open {
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        end_line: Option<usize>,
    },
    /// Render a file in the preview pane: a document rather than its source.
    Preview { path: String },
    /// One line for the status bar, already cut to size by the server.
    Say { text: String },
    /// Change text in an open buffer, once the user has said so. The only request that is
    /// answered, and `id` is what the answer is filed under — see [`Reply`].
    Edit {
        #[serde(with = "id_as_text")]
        id: u128,
        path: String,
        old: String,
        new: String,
    },
}

/// The request number, carried across as a decimal string rather than as a JSON number.
///
/// Two reasons, and either would be enough. serde_json refuses a 128-bit integer outright on the
/// way back in — "u128 is not supported" — and JSON numbers are doubles to most of the world
/// anyway, so a millisecond timestamp with a counter on the end would come back rounded from any
/// reader that went through a float. The number is what names the reply file, so it has to survive
/// bit for bit; a string is the one JSON shape that carries an integer of any width untouched.
mod id_as_text {
    pub fn serialize<S: serde::Serializer>(id: &u128, out: S) -> Result<S::Ok, S::Error> {
        out.serialize_str(&id.to_string())
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(input: D) -> Result<u128, D::Error> {
        use serde::Deserialize;
        let text = String::deserialize(input)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// The editor's answer to a request that had to be answered.
///
/// `message` is a sentence and not a code, for the reason [`NO_SESSION`] is: it is read by a
/// language model that has to tell a person what happened, and "the user declined" is something it
/// can relay while `EPERM` is something it has to guess at.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct Reply {
    #[serde(with = "id_as_text")]
    pub id: u128,
    pub ok: bool,
    pub message: String,
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
        // Made here rather than at the first answer: a server that has filed an edit starts
        // looking for its reply straight away, and a directory that does not exist yet is a
        // `read_dir` failing on every one of those looks.
        std::fs::create_dir_all(dir.join(REPLIES_DIR)).ok()?;
        // The registrations the drawer's agents are pointed at. Written here, before the editor
        // has spawned a single shell, because the drawer may start an agent on the first
        // keystroke of the session and a file that is not there yet is a file an agent is told
        // to read and cannot — see [`write_registrations`].
        write_registrations(&dir);
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

    /// Answers a request that was waiting for one.
    ///
    /// Best-effort and silent, like everything else this module writes: the server on the other
    /// end has a timeout precisely because an answer can fail to arrive, and there is nobody here
    /// to tell about it — the editor's user is looking at the status line, which has already said
    /// what happened in their own language.
    pub fn reply(&self, reply: &Reply) {
        write_reply(&self.dir, reply);
    }
}

/// Puts one answer in a session directory. Best-effort and silent; see [`Session::reply`].
pub fn write_reply(dir: &Path, reply: &Reply) {
    let dir = dir.join(REPLIES_DIR);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(text) = serde_json::to_string(reply) else { return };
    // Atomic for the reason the state file is: the reader is polling, and a file caught halfway
    // through being written is a tool result the agent would read as an editor answering in
    // nonsense.
    let _ = crate::settings::write_atomic(&dir.join(reply_name(reply.id)), text.as_bytes());
}

/// What one request's answer is called, on both sides of the gap.
fn reply_name(id: u128) -> String {
    format!("req-{id}.json")
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

// ---- Registering the server with the agent the drawer starts --------------------------------

// Everything from here to the next divider exists so that an agent started from the drawer has
// `clee --mcp` already registered, with nothing for the user to configure first.
//
// The four CLIs disagree about almost everything here, so this is four mechanisms rather than
// one — each verified against the installed binary rather than taken from documentation:
//
// - claude takes a JSON file on the command line, `--mcp-config <file>`, and *merges* what it
//   finds there with the servers the user has registered themselves. `--strict-mcp-config` would
//   suppress theirs, which is why it is not passed;
// - codex takes the two values straight on the command line as `-c key=value` overrides and
//   reads no file of ours at all;
// - opencode reads `$OPENCODE_CONFIG`, and merges it with the user's own config — a session
//   started this way still has their model and their provider;
// - gemini reads `$GEMINI_CLI_SYSTEM_SETTINGS_PATH`, and likewise lists both its own
//   `mcpServers` and the ones from the user's `~/.gemini/settings.json`.
//
// **Nothing here ever writes into the user's own configuration.** `~/.claude.json`, `~/.codex/`,
// `~/.config/opencode/` and `~/.gemini/` belong to the user, and an editor that edited them would
// be leaving something behind that outlives it — and that a person who uninstalled CleeCode would
// have to find and undo. The files this writes live in the session directory, which is
// per-editor-instance, removed on the way out and swept when a process was killed before it could
// remove its own.
//
// And it is the drawer only. An agent typed by hand into an ordinary pane is somebody running a
// program, not CleeCode starting one, and it keeps working exactly as it did: the manual
// registration is still documented, and `CLEE_SESSION` still reaches it by descent.

/// What the server is called in every one of the four configurations. One name, because it is
/// the name a model says out loud when it uses a tool from it.
const SERVER_NAME: &str = "clee";

/// The argument that turns this binary into the server. The other half of `main`'s `--mcp`.
const SERVER_FLAG: &str = "--mcp";

/// Which binary the registrations name.
///
/// [`std::env::current_exe`] and not the word `clee`, because the server has to be *this*
/// CleeCode: a development build run from `target/release` is not on anybody's PATH, and a `clee`
/// that is on the PATH is a different program with a different session directory. The name is the
/// fallback for the platforms where the current executable cannot be asked for — it is what the
/// user would have typed by hand anyway.
pub fn server_command() -> String {
    std::env::current_exe()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| SERVER_NAME.to_string())
}

/// The file `agent` reads its registration from, named as it sits in the session directory.
/// `None` is codex, which takes the whole thing on its command line.
fn config_file(agent: crate::session::Agent) -> Option<&'static str> {
    use crate::session::Agent;
    match agent {
        Agent::Claude => Some("claude-mcp.json"),
        Agent::OpenCode => Some("opencode.json"),
        Agent::Gemini => Some("gemini-settings.json"),
        Agent::Codex => None,
    }
}

/// The name the pane exports to point `agent` at that file.
///
/// Two of these are the agent's own — `OPENCODE_CONFIG` and `GEMINI_CLI_SYSTEM_SETTINGS_PATH` are
/// how those two are told where to look, and they are the whole mechanism there. The third is
/// ours: Claude Code takes the path as a flag, and the flag says `"$CLEE_MCP_CLAUDE"` rather than
/// ninety characters of temp path because that line is typed where the user can watch it and
/// lands in their shell history afterwards. A name is readable; a path under `$TMPDIR` is noise.
fn config_env(agent: crate::session::Agent) -> Option<&'static str> {
    use crate::session::Agent;
    match agent {
        Agent::Claude => Some("CLEE_MCP_CLAUDE"),
        Agent::OpenCode => Some("OPENCODE_CONFIG"),
        Agent::Gemini => Some("GEMINI_CLI_SYSTEM_SETTINGS_PATH"),
        Agent::Codex => None,
    }
}

/// The registration itself, in the shape `agent` expects to read it in.
///
/// Claude Code and gemini both spell it the way MCP's own documentation does; opencode has a
/// shape of its own, and needs to be told the server is a local process rather than a URL.
pub fn registration(agent: crate::session::Agent, server: &str) -> Option<Value> {
    use crate::session::Agent;
    match agent {
        Agent::Claude | Agent::Gemini => Some(json!({
            "mcpServers": { SERVER_NAME: { "command": server, "args": [SERVER_FLAG] } }
        })),
        Agent::OpenCode => Some(json!({
            "mcp": {
                SERVER_NAME: { "type": "local", "command": [server, SERVER_FLAG], "enabled": true }
            }
        })),
        Agent::Codex => None,
    }
}

/// Writes each file-reading agent's registration into the session directory.
///
/// Best-effort throughout, and deliberately silent: a registration that could not be written is
/// an agent that starts without it, which is exactly the state every agent was in before this
/// existed. Nothing here is allowed to be the reason a session fails to open.
fn write_registrations(dir: &Path) {
    let server = server_command();
    for agent in crate::session::Agent::all() {
        let (Some(name), Some(config)) = (config_file(agent), registration(agent, &server)) else {
            continue;
        };
        let Ok(text) = serde_json::to_string(&config) else { continue };
        let _ = crate::settings::write_atomic(&dir.join(name), text.as_bytes());
    }
}

/// The flag `agent`'s own `--help` has to mention before the drawer will pass it.
///
/// Only the two that take something on the command line. The other two are registered by an
/// environment variable, and a name a program has never heard of is a name it ignores — there is
/// nothing there to probe for and nothing that can go wrong.
fn registration_flag(agent: crate::session::Agent) -> Option<&'static str> {
    use crate::session::Agent;
    match agent {
        Agent::Claude => Some("--mcp-config"),
        Agent::Codex => Some("--config"),
        Agent::OpenCode | Agent::Gemini => None,
    }
}

/// What the drawer types into its pane, and what that pane alone carries in its environment.
pub struct Launch {
    /// The line typed at the shell's prompt once it is reading.
    pub line: String,
    /// Names exported on this pane's shell and no other. The two agents that are registered
    /// through the environment are why this exists, and the reason it is not simply added to
    /// every shell in [`crate::terminal_panel::TerminalPanel::with_startup`]: a `gemini` somebody
    /// types in an ordinary pane is their own program run their own way, and an editor that
    /// rewrote its settings path from underneath it would be answering a question nobody asked.
    pub env: Vec<(&'static str, PathBuf)>,
}

/// The line the drawer types to start `agent`, with the registration in it when there is one.
///
/// `exec` in front for the reason `App::launch_drawer_agent` gives: the shell *becomes* the
/// agent, so the pane ends when the agent does. Which also sets the price of getting the rest of
/// this line wrong — an unknown flag means the agent prints its usage and exits, `exec` has
/// already thrown the shell away, and the pane vanishes as though CleeCode were broken. That is
/// what `registered` is for; see [`accepts_registration`] for who answers it.
///
/// A free function taking the answer rather than asking for it, so both halves of that decision
/// can be read in a test without a machine that happens to have all four agents on it.
///
/// One shape on every platform, and that is parity rather than an oversight: the drawer has typed
/// `exec` since it existed, and `exec` is a POSIX shell builtin that cmd.exe has never had — so
/// where `"$NAME"` would not be expanded, the line it appears in was already not a line cmd.exe
/// could run. Whoever gives the drawer a Windows shape gives this one at the same time, in one
/// place, rather than finding half of it done differently here.
pub fn drawer_line(agent: crate::session::Agent, server: &str, registered: bool) -> String {
    use crate::session::Agent;
    let command = agent.workspace_name();
    if !registered {
        return format!("exec {command}");
    }
    match agent {
        Agent::Claude => {
            let name = config_env(agent).unwrap_or_default();
            format!("exec {command} --mcp-config \"${name}\"")
        }
        // codex has no file mechanism, so the two values go on the line literally. The path is
        // quoted for the shell rather than pasted bare: a CleeCode installed under
        // `/Applications/Clee Code.app` would otherwise hand codex half a path and a stray word.
        Agent::Codex => format!(
            "exec {command} -c mcp_servers.{SERVER_NAME}.command={} \
             -c mcp_servers.{SERVER_NAME}.args='[\"{SERVER_FLAG}\"]'",
            shell_words::quote(server)
        ),
        // Registered entirely in the environment: the line is the one that was always typed.
        Agent::OpenCode | Agent::Gemini => format!("exec {command}"),
    }
}

/// How the drawer starts `agent`: the line and the environment that go with each other.
///
/// `allowed` is `settings.agent_mcp`, the way back to the behaviour that shipped before this.
/// Every other reason to fall back to a bare launch is a reason not to risk the pane — the file
/// is not there, the installed version does not know the flag — and each of them lands on exactly
/// the line the drawer used to type.
pub fn drawer_launch(agent: crate::session::Agent, allowed: bool) -> Launch {
    let bare = || Launch { line: drawer_line(agent, "", false), env: Vec::new() };
    if !allowed {
        return bare();
    }
    // Where this agent reads a file, the file has to already be there. `Session::start` wrote it
    // before the first shell existed, so its absence means either that there is no session at all
    // or that the write failed — and pointing an agent at a path with nothing at it is precisely
    // the way to make a pane die on startup.
    let file = match config_file(agent) {
        Some(name) => match session_dir().map(|dir| dir.join(name)).filter(|path| path.exists()) {
            Some(path) => Some(path),
            None => return bare(),
        },
        None => None,
    };
    if !accepts_registration(agent) {
        return bare();
    }
    let env = match (config_env(agent), file) {
        (Some(name), Some(path)) => vec![(name, path)],
        _ => Vec::new(),
    };
    Launch { line: drawer_line(agent, &server_command(), true), env }
}

/// Whether `agent` as installed on this machine understands what the drawer is about to pass it.
///
/// Remembered for the life of the process rather than for a couple of seconds like
/// [`crate::drawer::installed`], and the difference is what each answer depends on. That one goes
/// stale because the launcher *offers to install* the missing agent, so a `false` is routinely
/// followed by the user making it true; this one changes only when the agent's own binary is
/// replaced, which nothing in CleeCode does or offers to do. And it is not free to ask: every
/// probe is a process spawned and waited for on the UI thread, and a node-based CLI takes a
/// noticeable fraction of a second to print its usage. Asked once per agent, lazily, so starting
/// claude never pays for the other three.
fn accepts_registration(agent: crate::session::Agent) -> bool {
    let Some(flag) = registration_flag(agent) else { return true };
    let mut memo = ACCEPTS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    match memo[agent.index()] {
        Some(known) => known,
        None => {
            let found = help_mentions(agent, flag);
            memo[agent.index()] = Some(found);
            found
        }
    }
}

/// The remembered answer for each agent, in [`crate::session::Agent::all`] order.
static ACCEPTS: std::sync::Mutex<[Option<bool>; 4]> = std::sync::Mutex::new([None; 4]);

/// The probe: run the agent's own `--help` and read whether the flag is in it.
///
/// The binary is found through [`crate::tools::tool`] rather than left to the PATH, for the
/// reason that module gives: started from the Dock this process has launchd's environment, and
/// an agent installed by Homebrew or npm is not on it. Getting that wrong here would answer "no"
/// on exactly the machines where the drawer works.
///
/// `false` whenever the question cannot be answered — the program is not there, it will not run,
/// it says nothing about the flag. Every one of those is a reason to type the line that has
/// always worked instead of the one that might not.
fn help_mentions(agent: crate::session::Agent, flag: &str) -> bool {
    let Some(name) = agent.programs().first() else { return false };
    let program = crate::tools::tool(name).unwrap_or_else(|| PathBuf::from(*name));
    let Ok(help) = std::process::Command::new(program)
        .arg("--help")
        // A program that reads its stdin while printing usage would otherwise sit there holding
        // the editor's own, which is the terminal CleeCode is drawn in.
        .stdin(std::process::Stdio::null())
        .output()
    else {
        return false;
    };
    // Both streams, because where a CLI prints its usage is its own business and clap alone has
    // sent it to either depending on how it was asked.
    String::from_utf8_lossy(&help.stdout).contains(flag)
        || String::from_utf8_lossy(&help.stderr).contains(flag)
}

// ---- The server's side ---------------------------------------------------------------------

/// The next request file name's number.
///
/// Milliseconds since the epoch with a per-process counter in the low digits: increasing, so the
/// editor can execute requests in the order they were made, and distinct even when one agent
/// asks for several things inside the same millisecond.
pub fn next_request_number() -> u128 {
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
///
/// The number is passed in rather than taken here, because for an edit it is also the number the
/// answer will come back under and the number written inside the request itself. One value, named
/// once by the caller, so those three cannot drift apart.
pub fn write_request(dir: &Path, id: u128, request: &Request) -> std::io::Result<()> {
    let requests = dir.join(REQUESTS_DIR);
    std::fs::create_dir_all(&requests)?;
    let text = serde_json::to_string(request)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let path = requests.join(format!("req-{id}.json"));
    crate::settings::write_atomic(&path, text.as_bytes())
}

/// How long the server waits for the editor to answer, and how often it looks.
///
/// Carried rather than read from the constants at the point of use, so that a test can ask the
/// same question with a millisecond of patience where a person is given two minutes.
#[derive(Clone, Copy, Debug)]
pub struct ReplyWait {
    pub interval: Duration,
    pub timeout: Duration,
}

impl Default for ReplyWait {
    fn default() -> Self {
        ReplyWait { interval: REPLY_INTERVAL, timeout: REPLY_TIMEOUT }
    }
}

/// Blocks until the editor answers request `id`, or until the patience runs out.
///
/// Blocking the stdio thread is the right thing here and not a compromise: a `tools/call` *is* a
/// synchronous call, the agent is waiting on it by design, and there is nothing else this process
/// does meanwhile. The reply is removed as it is read, so a directory left behind by a session
/// that ended badly never answers a later question with an older answer.
fn await_reply(dir: &Path, id: u128, wait: ReplyWait) -> Option<Reply> {
    let path = dir.join(REPLIES_DIR).join(reply_name(id));
    let started = Instant::now();
    loop {
        // Read before the clock is checked, so a zero timeout still collects an answer that is
        // already lying there — which is the shape every test of this takes.
        if let Ok(text) = std::fs::read_to_string(&path) {
            let _ = std::fs::remove_file(&path);
            if let Ok(reply) = serde_json::from_str::<Reply>(&text) {
                return Some(reply);
            }
            // An answer that will not parse is an answer nobody can act on. Treated as no answer
            // at all rather than as a failure, because the edit itself may well have happened.
            return None;
        }
        if started.elapsed() >= wait.timeout {
            return None;
        }
        std::thread::sleep(wait.interval);
    }
}

/// An agent's line, cut down to something a one-line status bar can hold.
///
/// Done here as well as in the editor, and the duplication is deliberate: this end is where the
/// text can still be refused with an explanation the agent will read, and the other end is the one
/// that must never be handed a control character whatever wrote the request file.
///
/// The first line that has anything on it, rather than strictly the first: a model that opens with
/// a newline meant the sentence after it, and answering that with an empty status bar would be
/// pedantry with no reader.
pub fn say_line(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default();
    line.chars().filter(|c| !c.is_control()).take(MAX_SAY).collect()
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
    serve_waiting(input, output, session, ReplyWait::default())
}

/// The server with its patience named, which is the form a test can afford to run.
pub fn serve_waiting(
    input: &mut impl BufRead,
    output: &mut impl Write,
    session: Option<&Path>,
    wait: ReplyWait,
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
        if let Some(reply) = handle(text, session, wait) {
            writeln!(output, "{reply}")?;
            output.flush()?;
        }
    }
}

/// One message in, at most one message out. `None` means silence, which is the correct answer to
/// a notification.
fn handle(text: &str, session: Option<&Path>, wait: ReplyWait) -> Option<Value> {
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
        "tools/call" => call(&params, session, wait),
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
            "description": "The files open in the CleeCode editor, in tab order, which one is \
                            active, and which ones are dirty. Use this to see what the user is \
                            working on before guessing at paths. A file listed as dirty has \
                            unsaved edits in the editor: the buffer on the user's screen and the \
                            file on disk disagree, so writing to that file would destroy work \
                            they can see. Use edit_buffer for those.",
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
                            Returns as soon as the request is filed, not when the file appears. \
                            With both line and end_line, CleeCode highlights that range so the \
                            user can see what you are talking about; the highlight clears the \
                            next time they touch that pane.",
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
                    "end_line": {
                        "type": "integer",
                        "description": "1-based last line of the range to highlight. Must be at \
                                        or after line, and is only read when line is given.",
                        "minimum": 1,
                    },
                },
                "required": ["path"],
            },
        },
        {
            "name": "preview",
            "description": "Ask CleeCode to render a file in its preview pane beside the user's \
                            work: markdown as a formatted document, images and PDFs as pictures. \
                            Use it to show the user something you produced. Returns when the \
                            request is filed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The file to render. Absolute, or relative to the project root.",
                    },
                },
                "required": ["path"],
            },
        },
        {
            "name": "say",
            "description": "Put one line in CleeCode's status bar, marked as coming from the \
                            agent. Use it to tell the user what you are doing while they watch \
                            the editor. The status bar is one line: only the first line of the \
                            text is shown, and it is cut at 120 characters.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "What to show. One short line, written for a person.",
                    },
                },
                "required": ["text"],
            },
        },
        {
            "name": "edit_buffer",
            "description": "Change text in a buffer that has UNSAVED edits in CleeCode — the \
                            files open_files lists as dirty. The user is asked for their consent \
                            and this call waits for the answer, up to two minutes. old_string \
                            must occur exactly once in the buffer. For files that are not dirty, \
                            edit the file on disk as you normally would: CleeCode reloads clean \
                            buffers by itself. The edit is applied to the buffer and not saved — \
                            saving stays the user's.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The open buffer to change. Absolute, or relative to the \
                                        project root.",
                    },
                    "old_string": {
                        "type": "string",
                        "description": "The exact text to replace. Must appear exactly once in \
                                        the buffer; include enough context to make it unique.",
                    },
                    "new_string": {
                        "type": "string",
                        "description": "What to put there instead. Empty deletes the old text.",
                    },
                },
                "required": ["path", "old_string", "new_string"],
            },
        },
    ])
}

/// `tools/call`, in the shape MCP wants it: a content list, and a flag saying whether it went
/// wrong. A tool failing is a result, not a JSON-RPC error — the error channel is for the
/// protocol, and a model needs to *read* what went wrong to do something about it.
fn call(params: &Value, session: Option<&Path>, wait: ReplyWait) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
    let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
    let (text, failed) = match run(name, &arguments, session, wait) {
        Ok(text) => (text, false),
        Err(text) => (text, true),
    };
    json!({ "content": [{ "type": "text", "text": text }], "isError": failed })
}

fn run(
    name: &str,
    arguments: &Value,
    session: Option<&Path>,
    wait: ReplyWait,
) -> Result<String, String> {
    match name {
        "open_files" => {
            let state = read_state(session)?;
            let active = state.active.as_ref().map(|a| a.path.clone());
            render(&json!({
                "root": state.root,
                "files": state.open_files,
                "active": active,
                "dirty": state.dirty_files,
            }))
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
            let path = wanted_path(arguments, "open_file")?;
            let line = arguments.get("line").and_then(Value::as_u64).map(|n| n.max(1) as usize);
            let end_line =
                arguments.get("end_line").and_then(Value::as_u64).map(|n| n.max(1) as usize);
            let request = Request::Open { path: path.to_string(), line, end_line };
            file(&dir, next_request_number(), &request)?;
            render(&json!({
                "status": "requested",
                "path": path,
                "line": line,
                "end_line": end_line,
                "note": "CleeCode opens it beside the user's work without taking the keyboard.",
            }))
        }
        "preview" => {
            let dir = session_with_editor(session)?;
            let path = wanted_path(arguments, "preview")?;
            file(&dir, next_request_number(), &Request::Preview { path: path.to_string() })?;
            render(&json!({
                "status": "requested",
                "path": path,
                "note": "CleeCode renders it beside the user's work without taking the keyboard.",
            }))
        }
        "say" => {
            let dir = session_with_editor(session)?;
            let text = arguments
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| "say needs a text".to_string())?;
            // Cut here rather than on arrival, so what comes back is what the user is looking at
            // — an agent that said two paragraphs should be able to see that one line of it
            // landed, and correct itself without being told.
            let line = say_line(text);
            if line.is_empty() {
                return Err("say needs something to show: there was no printable text in it"
                    .to_string());
            }
            file(&dir, next_request_number(), &Request::Say { text: line.clone() })?;
            render(&json!({ "status": "said", "text": line }))
        }
        "edit_buffer" => {
            let dir = session_with_editor(session)?;
            let path = wanted_path(arguments, "edit_buffer")?;
            let old = arguments
                .get("old_string")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .ok_or_else(|| {
                    "edit_buffer needs an old_string: the exact text to replace".to_string()
                })?;
            // Allowed to be empty, and that is a deletion. Distinguished from a missing argument,
            // which is a call the model got wrong and should be told about.
            let new = arguments
                .get("new_string")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    "edit_buffer needs a new_string: what to put there instead, or an empty \
                     string to delete the old text"
                        .to_string()
                })?;
            let id = next_request_number();
            let request = Request::Edit {
                id,
                path: path.to_string(),
                old: old.to_string(),
                new: new.to_string(),
            };
            file(&dir, id, &request)?;
            match await_reply(&dir, id, wait) {
                Some(reply) if reply.ok => {
                    render(&json!({ "status": "applied", "path": path, "note": reply.message }))
                }
                Some(reply) => Err(reply.message),
                None => Err(NO_ANSWER.to_string()),
            }
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

/// The `path` argument every tool that names a file wants, or a sentence saying which tool wanted
/// it. Named after the tool because a model calling three of them in a row has to be told which
/// of the three it got wrong.
fn wanted_path<'a>(arguments: &'a Value, tool: &str) -> Result<&'a str, String> {
    arguments
        .get("path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| format!("{tool} needs a path"))
}

/// Leaves a request for the editor, with the one sentence to say when the disk will not take it.
fn file(dir: &Path, id: u128, request: &Request) -> Result<(), String> {
    write_request(dir, id, request)
        .map_err(|e| format!("the request could not be left for the editor: {e}"))
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
        // A patience of nothing, because no test may sit for two minutes: every tool but
        // `edit_buffer` ignores it, and the two that care name their own.
        talk_waiting(lines, session, ReplyWait { interval: NO_TIME, timeout: NO_TIME })
    }

    /// A millisecond, which is what a test can afford to wait for anything.
    const NO_TIME: Duration = Duration::from_millis(1);

    fn talk_waiting(lines: &[&str], session: Option<&Path>, wait: ReplyWait) -> Vec<Value> {
        let input = lines.join("\n") + "\n";
        let mut reader = Cursor::new(input.into_bytes());
        let mut written: Vec<u8> = Vec::new();
        serve_waiting(&mut reader, &mut written, session, wait)
            .expect("the server must not fail on a closed stream");
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
            dirty_files: vec!["/proj/README.md".to_string()],
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

    /// A `Session` reading a directory somebody else made, for the tests that want to see what
    /// the server filed. Its clocks start in the past so the first look is not throttled away.
    fn a_reading_session(dir: &Path) -> Session {
        let past = Instant::now() - STATE_INTERVAL;
        Session {
            dir: dir.to_path_buf(),
            generation: 0,
            last: None,
            wrote_at: past,
            polled_at: past,
        }
    }

    /// A thread playing the editor: it waits for the first request to appear, decides what to
    /// answer, writes the reply, and hands the request back to the test.
    ///
    /// The only way to exercise a tool whose result is somebody's decision. It does by hand what
    /// `App::poll_mcp` does in the frame loop, and deliberately not through `Session`, whose `Drop`
    /// would take the scratch directory away while the server was still reading it.
    fn an_editor_answering(
        dir: &Path,
        answer: impl Fn(&Request) -> Reply + Send + 'static,
    ) -> std::thread::JoinHandle<Request> {
        let dir = dir.to_path_buf();
        std::thread::spawn(move || {
            let requests = dir.join(REQUESTS_DIR);
            for _ in 0..5_000 {
                let found = std::fs::read_dir(&requests).ok().and_then(|entries| {
                    entries.flatten().find_map(|entry| {
                        let name = entry.file_name();
                        let name = name.to_str()?;
                        name.strip_prefix("req-")?.strip_suffix(".json")?;
                        let text = std::fs::read_to_string(entry.path()).ok()?;
                        let request: Request = serde_json::from_str(&text).ok()?;
                        let _ = std::fs::remove_file(entry.path());
                        Some(request)
                    })
                });
                if let Some(request) = found {
                    write_reply(&dir, &answer(&request));
                    return request;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            panic!("the editor was never asked anything");
        })
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
    fn the_tool_list_is_every_tool_with_its_schema() {
        let replies = talk(&[r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#], None);
        let listed = replies[0]["result"]["tools"].as_array().expect("tools/list returns an array");
        let names: Vec<&str> = listed.iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(
            names,
            ["open_files", "selection", "diagnostics", "open_file", "preview", "say", "edit_buffer"]
        );
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
        // The one with unsaved work in it, named where an agent about to write to disk will see
        // it. Both lists, because "dirty" is a subset and not a separate set of files.
        assert_eq!(files["dirty"], json!(["/proj/README.md"]));

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
            vec![Request::Open { path: "src/main.rs".into(), line: Some(40), end_line: None }]
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
                Request::Open { path: format!("f{n}.rs"), line: Some(n), end_line: None };
            write_request(&dir, next_request_number(), &request)
                .expect("a request must be writable");
        }
        let mut session = Session {
            dir: dir.clone(),
            generation: 0,
            last: None,
            wrote_at: Instant::now(),
            polled_at: Instant::now(),
        };
        let paths: Vec<String> = session
            .take_requests()
            .into_iter()
            .map(|request| match request {
                Request::Open { path, .. } => path,
                other => panic!("only opens were written: {other:?}"),
            })
            .collect();
        assert_eq!(paths, ["f1.rs", "f2.rs", "f3.rs"]);
        drop(session);
    }

    /// The shape the previous version wrote is the shape this one reads.
    ///
    /// Not a hypothetical: a request file left in a session directory outlives nothing, but the
    /// two ends of this bridge are separate processes and can be separate builds — an agent
    /// holding an older `clee --mcp` open while the editor is upgraded is one `cargo install`
    /// away. `end_line` is absent there, and absent has to mean "no range" rather than "will not
    /// parse", which is the difference between a file opening and a request being deleted unread.
    #[test]
    fn a_request_written_by_the_previous_version_still_parses() {
        let old = r#"{"action":"open","path":"src/main.rs","line":40}"#;
        assert_eq!(
            serde_json::from_str::<Request>(old).expect("yesterday's request still parses"),
            Request::Open { path: "src/main.rs".into(), line: Some(40), end_line: None }
        );
        // And the one written without a line at all, which is what `open_file` files when the
        // agent names only a file.
        let bare = r#"{"action":"open","path":"README.md"}"#;
        assert_eq!(
            serde_json::from_str::<Request>(bare).expect("a request with only a path parses"),
            Request::Open { path: "README.md".into(), line: None, end_line: None }
        );
        // An action from a version that knows more than this one is refused, which is what makes
        // `take_requests` drop it instead of guessing.
        assert!(serde_json::from_str::<Request>(r#"{"action":"dance","path":"x"}"#).is_err());
    }

    /// Every shape, out and back. The two ends serialize and deserialize this independently, so a
    /// variant that survives one direction and not the other is a request the editor deletes
    /// without acting on and nobody ever hears about.
    #[test]
    fn every_request_shape_survives_the_round_trip() {
        let shapes = [
            Request::Open { path: "a.rs".into(), line: Some(3), end_line: Some(9) },
            Request::Preview { path: "notes.md".into() },
            Request::Say { text: "reading the parser".into() },
            Request::Edit {
                id: 1_756_000_000_000_123,
                path: "a.rs".into(),
                old: "let x = 1;".into(),
                new: "let x = 2;".into(),
            },
        ];
        for shape in shapes {
            let text = serde_json::to_string(&shape).expect("a request serialises");
            let back: Request = serde_json::from_str(&text).expect("and comes back: {text}");
            assert_eq!(back, shape, "{text}");
        }

        let reply = Reply { id: 1_756_000_000_000_123, ok: false, message: "no".into() };
        let text = serde_json::to_string(&reply).expect("a reply serialises");
        assert_eq!(serde_json::from_str::<Reply>(&text).expect("and comes back"), reply);
    }

    /// The status bar is one line and it is somebody else's screen. Whatever an agent sends, what
    /// is filed is one printable line of it.
    #[test]
    fn what_an_agent_says_is_cut_to_one_printable_line() {
        assert_eq!(say_line("reading the parser"), "reading the parser");
        assert_eq!(say_line("first\nsecond\nthird"), "first", "only the first line is shown");
        // A model that opens with a blank line meant the sentence after it.
        assert_eq!(say_line("\n\n  looking at main.rs  \n"), "looking at main.rs");
        // Control characters would move the cursor, clear the screen, or worse: the status bar is
        // drawn into a real terminal.
        assert_eq!(say_line("done\u{1b}[2Jgone"), "done[2Jgone");
        assert_eq!(say_line("a\u{7}b"), "ab");
        assert_eq!(say_line("x".repeat(MAX_SAY + 40).as_str()).chars().count(), MAX_SAY);
        assert_eq!(say_line(""), "");
        assert_eq!(say_line("\n \n"), "");
    }

    /// `say` files the cut line and hands it back, so an agent that wrote three paragraphs can see
    /// that one line of them landed. Text with nothing printable in it is refused instead, since
    /// an empty status bar is a tool that appears to have done nothing.
    #[test]
    fn say_files_what_the_user_will_actually_see() {
        let dir = a_session(&a_state());
        let replies = talk(
            &[
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"say","arguments":{"text":"rewriting the parser\nand then the lexer"}}}"#,
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"say","arguments":{"text":""}}}"#,
            ],
            Some(&dir),
        );
        assert_eq!(replies.len(), 2, "{replies:#?}");
        assert_eq!(replies[0]["result"]["isError"], false);
        let out: Value = serde_json::from_str(&tool_text(&replies[0])).expect("JSON out");
        assert_eq!(out["text"], "rewriting the parser");
        assert_eq!(replies[1]["result"]["isError"], true);

        let mut session = a_reading_session(&dir);
        assert_eq!(
            session.take_requests(),
            vec![Request::Say { text: "rewriting the parser".into() }]
        );
        drop(session);
    }

    /// `preview` and a range on `open_file` reach the editor as the shapes it acts on.
    #[test]
    fn preview_and_a_highlighted_range_reach_the_editor() {
        let dir = a_session(&a_state());
        let replies = talk(
            &[
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"preview","arguments":{"path":"notes.md"}}}"#,
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"open_file","arguments":{"path":"src/main.rs","line":40,"end_line":52}}}"#,
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"preview","arguments":{}}}"#,
            ],
            Some(&dir),
        );
        assert_eq!(replies[0]["result"]["isError"], false);
        assert_eq!(replies[1]["result"]["isError"], false);
        assert_eq!(replies[2]["result"]["isError"], true);
        assert!(tool_text(&replies[2]).contains("preview needs a path"));

        let mut session = a_reading_session(&dir);
        assert_eq!(
            session.take_requests(),
            vec![
                Request::Preview { path: "notes.md".into() },
                Request::Open { path: "src/main.rs".into(), line: Some(40), end_line: Some(52) },
            ]
        );
        drop(session);
    }

    /// The whole of `edit_buffer`: the request is filed, the call blocks, an editor answers, and
    /// the answer is what the agent reads. The editor here is a thread doing what `App` does —
    /// which is the only way to test a tool whose result is a person's decision.
    #[test]
    fn edit_buffer_waits_for_the_editor_and_carries_back_its_answer() {
        let dir = a_session(&a_state());
        let editor = an_editor_answering(&dir, |request| match request {
            Request::Edit { id, .. } => {
                Reply { id: *id, ok: true, message: "applied at line 12".into() }
            }
            other => panic!("the editor was asked for something else: {other:?}"),
        });
        let replies = talk_waiting(
            &[r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"edit_buffer","arguments":{"path":"src/main.rs","old_string":"let x = 1;","new_string":"let x = 2;"}}}"#],
            Some(&dir),
            ReplyWait { interval: NO_TIME, timeout: Duration::from_secs(20) },
        );
        assert_eq!(replies[0]["result"]["isError"], false, "{}", tool_text(&replies[0]));
        let out: Value = serde_json::from_str(&tool_text(&replies[0])).expect("JSON out");
        assert_eq!(out["status"], "applied");
        assert_eq!(out["note"], "applied at line 12", "the editor's own words reach the agent");

        let asked = editor.join().expect("the editor thread must not panic");
        match asked {
            Request::Edit { path, old, new, .. } => {
                assert_eq!(path, "src/main.rs");
                assert_eq!((old.as_str(), new.as_str()), ("let x = 1;", "let x = 2;"));
            }
            other => panic!("{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A refusal is a tool error carrying the editor's sentence, because the model has to tell the
    /// person at the other end why nothing happened — and must not read it as "try again".
    #[test]
    fn an_edit_the_user_declines_comes_back_as_a_tool_error() {
        let dir = a_session(&a_state());
        let editor = an_editor_answering(&dir, |request| match request {
            Request::Edit { id, .. } => Reply {
                id: *id,
                ok: false,
                message: "the user declined the change".into(),
            },
            other => panic!("{other:?}"),
        });
        let replies = talk_waiting(
            &[r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"edit_buffer","arguments":{"path":"a.rs","old_string":"x","new_string":"y"}}}"#],
            Some(&dir),
            ReplyWait { interval: NO_TIME, timeout: Duration::from_secs(20) },
        );
        assert_eq!(replies[0]["result"]["isError"], true);
        assert_eq!(tool_text(&replies[0]), "the user declined the change");
        let _ = editor.join();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Nobody at the keyboard. The tool has to give up, and it has to give up *without* saying the
    /// edit failed: the question is still on screen and may yet be answered yes.
    #[test]
    fn an_unanswered_edit_says_so_without_claiming_it_failed() {
        let dir = a_session(&a_state());
        let replies = talk_waiting(
            &[r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"edit_buffer","arguments":{"path":"a.rs","old_string":"x","new_string":"y"}}}"#],
            Some(&dir),
            ReplyWait { interval: NO_TIME, timeout: Duration::from_millis(20) },
        );
        assert_eq!(replies[0]["result"]["isError"], true);
        let said = tool_text(&replies[0]);
        assert!(said.contains("did not answer"), "{said}");
        assert!(said.contains("may still land"), "the edit is not being called a failure: {said}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The two arguments that cannot be guessed at. An empty `new_string` is a deletion and stays
    /// allowed; an empty `old_string` would match everywhere and is not.
    #[test]
    fn edit_buffer_insists_on_the_text_it_is_replacing() {
        let dir = a_session(&a_state());
        let replies = talk(
            &[
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"edit_buffer","arguments":{"path":"a.rs","old_string":"","new_string":"y"}}}"#,
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"edit_buffer","arguments":{"path":"a.rs","old_string":"x"}}}"#,
            ],
            Some(&dir),
        );
        assert_eq!(replies[0]["result"]["isError"], true);
        assert!(tool_text(&replies[0]).contains("old_string"));
        assert_eq!(replies[1]["result"]["isError"], true);
        assert!(tool_text(&replies[1]).contains("new_string"));
        let _ = std::fs::remove_dir_all(&dir);
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

    // ---- what the drawer hands the agent it starts -------------------------------------------

    /// Each of the three file-reading agents gets the shape its own configuration is written in,
    /// naming this binary and the flag that turns it into a server. A shape that has drifted is
    /// an agent that starts with no `clee` tool and no complaint from anybody.
    #[test]
    fn the_registration_is_written_in_each_agents_own_shape() {
        use crate::session::Agent;
        let claude = registration(Agent::Claude, "/opt/clee").expect("claude reads a file");
        assert_eq!(claude["mcpServers"]["clee"]["command"], "/opt/clee");
        assert_eq!(claude["mcpServers"]["clee"]["args"], json!(["--mcp"]));
        // gemini's settings file says it the same way, which is MCP's own documented shape.
        assert_eq!(registration(Agent::Gemini, "/opt/clee"), Some(claude));

        let opencode = registration(Agent::OpenCode, "/opt/clee").expect("opencode reads a file");
        assert_eq!(opencode["mcp"]["clee"]["type"], "local");
        assert_eq!(opencode["mcp"]["clee"]["command"], json!(["/opt/clee", "--mcp"]));
        assert_eq!(
            opencode["mcp"]["clee"]["enabled"],
            true,
            "opencode reads a server it is not told to enable as one it should not start"
        );

        assert_eq!(
            registration(Agent::Codex, "/opt/clee"),
            None,
            "codex is registered on its command line and reads no file of ours"
        );
    }

    /// The server named is *this* executable and not the word `clee`: a development build is not
    /// on anybody's PATH, and a `clee` that is would be a different editor with a different
    /// session directory.
    #[test]
    fn the_registration_names_the_binary_that_is_running() {
        let server = server_command();
        assert!(!server.is_empty());
        if let Ok(exe) = std::env::current_exe() {
            assert_eq!(server, exe.to_string_lossy());
        }
    }

    /// A session directory carries the three files before any shell exists, each holding the
    /// registration that agent reads — and each parseable, since the agent reading it is the one
    /// thing this code never gets to see happen.
    #[test]
    fn a_session_carries_a_registration_for_every_agent_that_reads_one() {
        use crate::session::Agent;
        let dir = std::env::temp_dir().join(format!("clee-mcp-register-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory must be creatable");
        write_registrations(&dir);

        for agent in Agent::all() {
            let Some(name) = config_file(agent) else {
                continue;
            };
            let text = std::fs::read_to_string(dir.join(name))
                .unwrap_or_else(|e| panic!("{name} must be written: {e}"));
            let written: Value = serde_json::from_str(&text).expect("what is written is JSON");
            assert_eq!(written, registration(agent, &server_command()).expect("a shape"));
        }
        assert!(!dir.join("codex.json").exists(), "codex is given no file to read");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The four launch lines, both ways round. The unregistered half is not a fallback nobody
    /// sees: it is what an older claude, a codex that never learnt `-c`, or a session directory
    /// that could not be written all land on, and it has to be exactly the line the drawer typed
    /// before any of this existed — `exec` and the agent's own name, nothing else.
    #[test]
    fn the_launch_line_says_the_registration_only_when_it_is_known_to_be_understood() {
        use crate::session::Agent;
        let server = "/opt/homebrew/bin/clee";

        assert_eq!(
            drawer_line(Agent::Claude, server, true),
            "exec claude --mcp-config \"$CLEE_MCP_CLAUDE\"",
            "the path travels in a name, not in the line the user watches being typed"
        );
        assert_eq!(
            drawer_line(Agent::Codex, server, true),
            "exec codex -c mcp_servers.clee.command=/opt/homebrew/bin/clee \
             -c mcp_servers.clee.args='[\"--mcp\"]'"
        );
        // These two are registered entirely through the environment, so their line never changes
        // — which is also why neither of them can be broken by a version that has moved on.
        assert_eq!(drawer_line(Agent::OpenCode, server, true), "exec opencode");
        assert_eq!(drawer_line(Agent::Gemini, server, true), "exec gemini");

        for agent in Agent::all() {
            let bare = drawer_line(agent, server, false);
            assert_eq!(bare, format!("exec {}", agent.workspace_name()));
            // `Agent::of_command` reads the first word of a startup command, and `exec` is not a
            // program: whatever else changes here, the pane's own recorded command must not be
            // this line. See `App::launch_drawer_agent`.
            assert_eq!(Agent::of_command(&bare), None, "the line is not a startup command");
        }
    }

    /// A CleeCode installed somewhere with a space in it hands codex a quoted path rather than
    /// half of one followed by a word codex would read as another override.
    #[test]
    fn a_server_path_with_a_space_in_it_survives_the_codex_line() {
        let line = drawer_line(crate::session::Agent::Codex, "/Applications/Clee Code/clee", true);
        assert!(line.contains("command='/Applications/Clee Code/clee'"), "{line}");
    }

    /// Turning the setting off puts every agent back on the line and the environment it had
    /// before any of this: one escape hatch, and it is a whole one.
    #[test]
    fn the_setting_turns_the_whole_thing_off() {
        for agent in crate::session::Agent::all() {
            let launch = drawer_launch(agent, false);
            assert_eq!(launch.line, format!("exec {}", agent.workspace_name()));
            assert!(launch.env.is_empty(), "{:?} carries nothing of ours", agent);
        }
    }

    /// A whole launch, on both sides of the one thing it depends on: the session directory.
    ///
    /// Without one there is nothing to point an agent at, and every agent that reads a file falls
    /// back to the bare line rather than naming a path with nothing at it. With one — which is
    /// the state the drawer is always in, since `Session::start` runs before the first shell — the
    /// name is exported, the file is there, and it holds what that agent reads.
    ///
    /// One test rather than two because both halves speak about this process's own session
    /// directory, and two tests would be two threads taking it away from each other.
    #[test]
    fn a_launch_carries_the_file_and_the_name_its_line_depends_on() {
        use crate::session::Agent;
        let file_readers = || Agent::all().into_iter().filter(|agent| config_file(*agent).is_some());

        if let Some(dir) = session_dir() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        for agent in file_readers() {
            let launch = drawer_launch(agent, true);
            assert_eq!(
                launch.line,
                format!("exec {}", agent.workspace_name()),
                "with no registration written, {agent:?} starts the way it always did"
            );
            assert!(launch.env.is_empty(), "and carries no name pointing at nothing");
        }

        let session = Session::start().expect("a session directory must be creatable");
        for agent in file_readers() {
            let launch = drawer_launch(agent, true);
            let (name, path) = launch.env.first().unwrap_or_else(|| {
                panic!("{agent:?} is registered through a file and nothing named it")
            });
            assert_eq!(Some(*name), config_env(agent));
            let text = std::fs::read_to_string(path).expect("the file the name points at exists");
            let written: Value = serde_json::from_str(&text).expect("and is the registration");
            assert_eq!(written, registration(agent, &server_command()).expect("a shape"));
        }

        // Whichever way the probe answered on the machine running this, a line that reads
        // `$CLEE_MCP_CLAUDE` has to come with the name it reads: a shell expanding a name nobody
        // set would hand claude an empty `--mcp-config` and take the pane with it.
        for agent in Agent::all() {
            let launch = drawer_launch(agent, true);
            if let Some(name) = config_env(agent).filter(|name| launch.line.contains(*name)) {
                assert!(
                    launch.env.iter().any(|(exported, _)| *exported == name),
                    "{agent:?}: the line reads ${name} and nothing exports it"
                );
            }
        }
        drop(session);
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
