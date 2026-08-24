//! A language server client, spoken by hand over stdio.
//!
//! This is a *client*: it starts a language server and talks to it. The crate that looks like it
//! belongs here, `tower-lsp`, is for writing the other end — building on it would mean assembling
//! the server side of the protocol to talk to a server.
//!
//! There is no async runtime. CleeCode already has one concurrency pattern and uses it three
//! times over — a thread that works, an `mpsc` channel that answers, a `poll_*` in the frame
//! loop — and a language server on stdio is line-oriented JSON-RPC, which that pattern fits. A
//! second model would mean every contributor having to know which one applies where.
//!
//! Diagnostics first, and completion once they had been running for a release. Both are
//! non-modal and neither can corrupt a buffer: if the server dies, some underlines go away and
//! the popup falls back to the words already in the file. Nothing here rewrites text on its own
//! account — a completion is accepted by the same keystroke and the same one-step edit as a word
//! scraped out of the buffer, and the server only ever adds names to a list that already exists.

use lsp_types::{
    ClientCapabilities, CompletionClientCapabilities, CompletionItem, CompletionItemCapability,
    Diagnostic, DiagnosticSeverity, GeneralClientCapabilities, InitializeParams, InsertTextFormat,
    PositionEncodingKind, PublishDiagnosticsParams, TextDocumentClientCapabilities, Uri,
    WindowClientCapabilities,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::str::FromStr;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// What the client hands to the application. Deliberately not the protocol's own types: the app
/// should not have to know what a `TextDocumentIdentifier` is to draw a squiggle.
pub enum Event {
    /// The server answered the handshake. `utf16` is how it wants positions counted: servers may
    /// offer to count in UTF-8 instead, which spares a conversion and the drift that comes with
    /// getting it wrong.
    Ready { utf16: bool },
    /// A file's diagnostics, replacing whatever was held for it. An empty list means "clean" and
    /// must be delivered, not skipped — it is how errors disappear once they are fixed.
    ///
    /// Still in the protocol's own form, because turning a diagnostic into a [`Mark`] needs the
    /// file's text and this arrives on a thread that does not have it. [`marks_from`] is the one
    /// place that conversion happens; a second one would be a second chance to read the columns
    /// in the wrong units.
    Diagnostics { path: PathBuf, raw: Vec<Diagnostic> },
    /// The answer to one `textDocument/completion`, reduced to the words that can be typed.
    ///
    /// `id` is the request it answers, and it is the whole guard against a stale reply: by the
    /// time this arrives the cursor may have moved, the file may have changed, or the popup may
    /// be gone. The application matches the id against the one request it is waiting for and
    /// drops everything else, rather than trying to work out whether the words still apply.
    Completion { id: i64, words: Vec<String> },
    /// Where a definition is, or `None` when the server knows of none. The `None` is delivered
    /// rather than dropped: "nothing is defined there" is an answer, and a key that silently
    /// does nothing is a key you press again.
    Definition { id: i64, target: Option<Jump> },
    /// What the thing under the cursor is, in one line. `None` when the server had nothing.
    Hover { id: i64, text: Option<String> },
    /// A reply the server is owed, already written and waiting to be put on the wire.
    ///
    /// It travels this way round because the reader thread has no writer: the pipe into the
    /// server belongs to the frame loop, and handing a second thread a handle on it would mean
    /// two of them interleaving frames. So the thread that knows what to answer says what it is,
    /// and the thread that owns the pipe sends it.
    Answer { message: Value },
    /// The server stopped, with whatever it said on the way out. Everything keeps working; the
    /// underlines simply stop arriving.
    Stopped { detail: String },
}

/// One diagnostic, already reduced to what the renderer needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    /// Zero-based line in the buffer.
    pub line: usize,
    /// Character columns, half-open, on that line. Both already converted out of the protocol's
    /// UTF-16 counting — see [`utf16_to_chars`].
    pub start: usize,
    pub end: usize,
    pub severity: Severity,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Hint,
    Info,
    Warning,
    Error,
}

impl Severity {
    fn from_lsp(value: Option<DiagnosticSeverity>) -> Severity {
        match value {
            Some(DiagnosticSeverity::ERROR) => Severity::Error,
            Some(DiagnosticSeverity::WARNING) => Severity::Warning,
            Some(DiagnosticSeverity::INFORMATION) => Severity::Info,
            Some(DiagnosticSeverity::HINT) => Severity::Hint,
            // The protocol says an absent severity is the client's to decide. An unmarked
            // diagnostic is still the server telling you something is wrong with the line.
            _ => Severity::Error,
        }
    }
}

// ---- The wire ------------------------------------------------------------------------------

/// Wraps a JSON payload in the header the protocol frames messages with.
///
/// `\r\n`, not `\n`: the header is HTTP-shaped, and a server reading it strictly will sit waiting
/// for the carriage return that never comes.
pub fn frame(payload: &str) -> Vec<u8> {
    let mut out = format!("Content-Length: {}\r\n\r\n", payload.len()).into_bytes();
    out.extend_from_slice(payload.as_bytes());
    out
}

/// Reads one framed message, or `None` at end of stream.
///
/// The length is in *bytes*, and the body is read by byte count rather than by lines, because a
/// diagnostic message can contain anything — newlines included, which rust-analyzer's do.
pub fn read_message(reader: &mut impl BufRead) -> std::io::Result<Option<String>> {
    let mut length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break; // the blank line that ends the header
        }
        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            length = value.trim().parse().ok();
        }
        // Content-Type and anything else in the header is read and dropped: the protocol allows
        // fields we do not know, and a client that choked on one would break on a server that
        // simply said more than it had to.
    }
    let Some(length) = length else {
        // A header block with no length is not something to guess at — the next bytes could be
        // anything, and reading them as a message would put the stream permanently out of step.
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "message header carried no Content-Length",
        ));
    };
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    String::from_utf8(body)
        .map(Some)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

// ---- Paths and positions -------------------------------------------------------------------

/// A `file:` URI for an absolute path.
///
/// Written out by hand because `lsp_types::Uri` is a parser, not a builder — it has `FromStr` and
/// nothing that takes a path. The percent-encoding is the whole job: a project under
/// `~/Documenti/prova nuova/` produces a URI the server cannot match back to a file unless the
/// space and the accents are escaped, and the failure is silent — diagnostics simply never
/// arrive for that file.
pub fn uri_for(path: &Path) -> Option<Uri> {
    let text = path.to_str()?;
    // A Windows path arrives as `C:\src\main.rs` and has to leave as `/C:/src/main.rs`: the
    // authority is empty, the leading slash is required, and the separators are forward.
    let text = text.replace('\\', "/");
    // A `file:` URI names a file from the root of the filesystem; there is no such thing as one
    // relative to wherever the editor happened to be started. Refused rather than patched up
    // with a slash, which is what produced `file:///./src/main.rs` — a URI that parses, travels,
    // comes back, and matches nothing at either end.
    //
    // Asked of the text rather than through `Path::is_absolute`, which answers for the host it
    // is compiled on: a Windows path is relative to a Unix build, and the Windows code here is
    // built by people who cannot run it.
    if !is_rooted(&text) {
        return None;
    }
    let mut out = String::from("file://");
    if !text.starts_with('/') {
        out.push('/');
    }
    for byte in text.bytes() {
        match byte {
            // Unreserved by RFC 3986, plus the separators and the colon a drive letter needs.
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    Uri::from_str(&out).ok()
}

/// Whether a path (already written with forward slashes) starts at a filesystem root: a leading
/// slash, a `C:` drive letter, or a `//server/share` UNC.
fn is_rooted(text: &str) -> bool {
    if text.starts_with('/') {
        return true;
    }
    let bytes = text.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// The path a `file:` URI names, undoing the percent-encoding.
pub fn path_for(uri: &Uri) -> Option<PathBuf> {
    let text = uri.as_str().strip_prefix("file://")?;
    let mut bytes = Vec::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            match u8::from_str_radix(&hex, 16) {
                Ok(byte) => bytes.push(byte),
                // A stray `%` that is not an escape. Keeping it is the lesser wrong: the path
                // will not resolve, and a diagnostic for a file we cannot name is dropped rather
                // than attached to the wrong one.
                Err(_) => return None,
            }
        } else {
            let mut buf = [0u8; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
    }
    let text = String::from_utf8(bytes).ok()?;
    // `/C:/src/main.rs` came from `C:\src\main.rs` and has to go back.
    let looks_like_a_drive = text.len() > 2
        && text.starts_with('/')
        && text.as_bytes()[1].is_ascii_alphabetic()
        && text.as_bytes()[2] == b':';
    Some(PathBuf::from(if looks_like_a_drive { &text[1..] } else { &text[..] }))
}

/// Converts a column counted in UTF-16 code units — which is what the protocol means by
/// `character` — into one counted in characters, which is what the editor means by a column.
///
/// They agree on ASCII and part company everywhere else, and the disagreement is invisible in
/// testing done in English: `è` is one character and one UTF-16 unit, but an emoji is one
/// character and *two*, so an underline after one drifts left by a column per emoji on the line.
/// Servers may agree to count in UTF-8 instead, and this is not called when they do.
pub fn utf16_to_chars(line: &str, utf16_col: usize) -> usize {
    let mut units = 0usize;
    for (chars, c) in line.chars().enumerate() {
        if units >= utf16_col {
            return chars;
        }
        units += c.len_utf16();
    }
    line.chars().count()
}

/// The other direction, for a column we are about to *send*: characters to UTF-16 code units.
///
/// Its own function rather than a flag on [`utf16_to_chars`], because the two are asked at
/// opposite ends of the conversation — one reads what arrived, one writes what leaves — and a
/// single function with a direction argument is a single place to pass the wrong one.
pub fn chars_to_utf16(line: &str, col: usize) -> usize {
    line.chars().take(col).map(char::len_utf16).sum()
}

/// The same pair again for the servers that count in UTF-8 bytes: characters to bytes, for a
/// column we are about to send.
///
/// A server that asked for UTF-8 is asking for byte offsets into the line, not for characters —
/// they agree on ASCII, which is why sending the character column to one of them looks correct
/// on every line until somebody writes `// città` and completion starts answering about the
/// wrong word.
pub fn chars_to_utf8(line: &str, col: usize) -> usize {
    line.chars().take(col).map(char::len_utf8).sum()
}

/// Bytes to characters, for a column that arrived.
///
/// Rounds *left*: a byte offset that lands inside a character — which is a server being wrong,
/// or a server describing a version of the line we no longer have — names the character it fell
/// into rather than the one after it. Left is the direction that keeps a mark on the thing it is
/// about; rounding right walks a squiggle off the end of a line of accented text.
pub fn utf8_to_chars(line: &str, byte_col: usize) -> usize {
    if byte_col >= line.len() {
        // Past the end clamps rather than panicking: the server is describing text that has
        // since been edited, and the end of the line is the nearest true thing.
        return line.chars().count();
    }
    // Every character starting at or before the offset, less the one that starts *on* it.
    line.char_indices().take_while(|(i, _)| *i <= byte_col).count().saturating_sub(1)
}

/// Turns the protocol's diagnostics for one file into marks against its text.
///
/// `lines` is the file as the editor holds it. A diagnostic pointing past the end of the buffer
/// is dropped rather than clamped: the server is describing a version of the file that no longer
/// exists, and an underline in the wrong place is worse than none.
pub fn marks_from(diagnostics: &[Diagnostic], lines: &[String], utf16: bool) -> Vec<Mark> {
    let mut out = Vec::new();
    for d in diagnostics {
        let line = d.range.start.line as usize;
        let Some(text) = lines.get(line) else { continue };
        let column = |col: u32| {
            if utf16 {
                utf16_to_chars(text, col as usize)
            } else {
                // Counted in UTF-8 bytes: walk to the character boundary at or before it.
                utf8_to_chars(text, col as usize)
            }
        };
        let start = column(d.range.start.character);
        // A range spanning lines is shown on its first line only, out to the end of it: the
        // squiggle marks where to look, and the message says the rest.
        let end = if d.range.end.line as usize == line {
            column(d.range.end.character)
        } else {
            text.chars().count()
        };
        // A zero-width range is a real thing in the protocol — "expected something here" — and
        // has to cover a cell or it cannot be seen.
        let end = end.max(start + 1);
        out.push(Mark {
            line,
            start,
            end,
            severity: Severity::from_lsp(d.severity),
            message: d.message.lines().next().unwrap_or_default().to_string(),
        });
    }
    out
}

/// The word a completion item would put in the buffer, or `None` for one that cannot be reduced
/// to a word.
///
/// The popup completes *a word*: it replaces the identifier under the cursor and nothing else.
/// So an item is taken down to the identifier it starts with — rust-analyzer labels a function
/// `push_str(…)` and a macro `println!(…)`, and inserting either literally would leave brackets
/// in the file that the user has to go back and clean up. Taking the leading run gives
/// `push_str` and `println`, which is what was being typed towards.
///
/// `insert_text` is preferred over the label because the label is written to be *read* — it can
/// carry a type, an arrow, an ellipsis — while `insert_text` is written to be typed. It is
/// skipped when the server marks it as a snippet: we advertise no snippet support, so a snippet
/// arriving anyway is a server ignoring the handshake, and `${1:self}` in a buffer is worse than
/// a label that is merely ugly.
pub fn word_of(item: &CompletionItem) -> Option<String> {
    let snippet = item.insert_text_format == Some(InsertTextFormat::SNIPPET);
    let raw = match item.insert_text.as_deref() {
        Some(text) if !snippet => text,
        _ => item.label.as_str(),
    };
    let word: String =
        raw.trim_start().chars().take_while(|&c| c.is_alphanumeric() || c == '_').collect();
    // An item that begins with punctuation — an operator, a lifetime, `&str` — has no word at
    // its head, and half of one would be worse than leaving it out of the list.
    if !word.starts_with(|c: char| c.is_alphabetic() || c == '_') {
        return None;
    }
    Some(word)
}

// ---- The server ----------------------------------------------------------------------------

/// How long the typing has to stop before the server is told about the edit.
///
/// A message per keystroke is the trap here, and it is the same lesson the markdown previews
/// taught: the work is not the sending, it is what the other end does with it. rust-analyzer
/// re-analyses on every change, so typing a word without this would queue eight analyses of a
/// file that was never finished being written.
pub const QUIET: std::time::Duration = std::time::Duration::from_millis(400);

/// Whether an edit is settled enough to send. Named, so the rule is one thing rather than a
/// comparison spelled out wherever it is needed.
pub fn should_send(last_sent: Option<u64>, current: u64, since_change: std::time::Duration) -> bool {
    last_sent != Some(current) && since_change >= QUIET
}

/// The servers CleeCode knows the name of, by file extension.
///
/// Names and arguments only — nothing here is installed, configured or bundled, and a machine
/// without the program is the ordinary case rather than a fault. This is a table of *what to try
/// running*, which is why it can afford to be long: an entry for a server nobody has costs one
/// failed `spawn`, said once, and then nothing.
///
/// The argument lists are each server's own required spelling. `--stdio` is not decoration:
/// typescript-language-server and pyright default to a socket and simply sit there without it,
/// which reads as a server that started and never answered.
const SERVERS: &[(&[&str], &[&str])] = &[
    (&["rs"], &["rust-analyzer"]),
    (&["py", "pyi"], &["pyright-langserver", "--stdio"]),
    (&["ts", "tsx", "js", "jsx", "mjs", "cjs"], &["typescript-language-server", "--stdio"]),
    (&["go"], &["gopls"]),
    (&["c", "h", "cpp", "cxx", "cc", "hpp", "hxx"], &["clangd"]),
    (&["lua"], &["lua-language-server"]),
    (&["zig"], &["zls"]),
    (&["rb"], &["solargraph", "stdio"]),
    (&["sh", "bash"], &["bash-language-server", "start"]),
    (&["json", "jsonc"], &["vscode-json-language-server", "--stdio"]),
    (&["toml"], &["taplo", "lsp", "stdio"]),
    (&["tex", "latex", "bib"], &["texlab"]),
];

/// Which server to run for a file, or `None` for a language nothing here can offer one for.
///
/// `configured` is the user's own table — extension to command line — and it wins over the built
/// in one. That is the important half: the list above is a convenience, and the setting is what
/// makes this work for a language nobody thought to put in it, or with the fork of a server
/// somebody keeps in `~/bin`. A release should not be the way to reach a new language server.
///
/// The command line is split on spaces rather than parsed as a shell would: a path with a space
/// in it needs the `interpreter_paths` treatment and not a quoting dialect of our own, and a
/// half-implemented quoting dialect is worse than none.
pub fn server_for(path: &Path, configured: &BTreeMap<String, String>) -> Option<Vec<String>> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    if let Some(command) = configured.get(&extension) {
        let argv: Vec<String> = command.split_whitespace().map(str::to_string).collect();
        // An entry set to nothing is how a built-in is turned off — the alternative would be a
        // second setting whose only job is to say "not that one".
        return (!argv.is_empty()).then_some(argv);
    }
    SERVERS
        .iter()
        .find(|(extensions, _)| extensions.contains(&extension.as_str()))
        .map(|(_, argv)| argv.iter().map(|a| a.to_string()).collect())
}

/// What a request was asking, so the answer can be read as that.
///
/// The reader thread has nothing but the id to tell one response from another, so what the id
/// meant is written down when it goes out. This used to be a set of completion ids, with a
/// comment saying that "the first response is the handshake and everything after it is a
/// completion" would be true today and silently wrong the day a third kind of request appeared.
/// This is that day, twice over.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ask {
    Completion,
    Definition,
    Hover,
}

/// Where a definition is, before the file it names has been opened.
///
/// The column is in the server's units and stays that way. Turning it into a character offset
/// needs the target file's text, and this arrives on a thread that has never read it — the same
/// reason a diagnostic arrives as a `Diagnostic` and becomes a [`Mark`] somewhere else.
#[derive(Clone, Debug, PartialEq)]
pub struct Jump {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
}

/// The one place a definition answer is read, in any of the three shapes a server may send it.
///
/// `Location`, an array of them, or `LocationLink` — which names the same two fields
/// `targetUri` and `targetSelectionRange`. Servers pick whichever they like and are all correct,
/// so this reads whichever arrived rather than declaring a preference and losing the others.
///
/// The first of an array, deliberately. More than one definition is real — a trait method with
/// several implementations — and a picker for them is a feature of its own; going to the first
/// is what every editor does before it grows one, and it is right far more often than not.
pub fn first_location(result: Option<&Value>) -> Option<Jump> {
    let value = result?;
    let one = if value.is_array() { value.get(0)? } else { value };
    let uri = one.get("uri").or_else(|| one.get("targetUri"))?.as_str()?;
    let range = one
        .get("range")
        .or_else(|| one.get("targetSelectionRange"))
        .or_else(|| one.get("targetRange"))?;
    let line = range.pointer("/start/line")?.as_u64()? as usize;
    let column = range.pointer("/start/character")?.as_u64()? as usize;
    let uri: Uri = uri.parse().ok()?;
    Some(Jump { path: path_for(&uri)?, line, column })
}

/// A hover answer, reduced to the one line that fits in a status bar.
///
/// A hover is documentation: rust-analyzer sends the signature, then a rule, then paragraphs of
/// prose. All of it belongs in a window this release does not have, and the first meaningful
/// line of it is the part people actually look at — the type, or the signature. So that is what
/// is taken, with the code fence around it removed: ```` ```rust ```` is markup for a renderer
/// and noise in a status bar.
pub fn hover_text(result: Option<&Value>) -> Option<String> {
    let contents = result?.get("contents")?;
    // Three shapes again, and all three are in the specification: a string, a
    // `{language, value}` pair, a `MarkupContent`, or an array of any of those.
    let text = match contents {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(text) => Some(text.clone()),
                other => other.get("value").and_then(Value::as_str).map(str::to_string),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        other => other.get("value").and_then(Value::as_str)?.to_string(),
    };
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("```") && *line != "---")
        .map(str::to_string)
}

pub struct Client {
    pub name: String,
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Event>,
    next_id: i64,
    /// Files the server has been told about, so a second open is a change rather than a
    /// duplicate — servers are entitled to complain about being told twice.
    open: Vec<PathBuf>,
    /// Whether positions are counted in UTF-16. Settled during the handshake and remembered,
    /// because it decides how every column that arrives afterwards is read.
    utf16: bool,
    /// Whether the handshake is finished. Until it is, the server is entitled to ignore every
    /// notification and request sent to it, and the ones worth using do.
    ready: bool,
    /// What each request still out was asking, shared with the reader thread. See [`Ask`].
    pending: Arc<Mutex<HashMap<i64, Ask>>>,
}

impl Client {
    pub fn start_with(argv: &[&str], root: &Path) -> Result<Client, String> {
        let (program, args) = argv.split_first().ok_or("no server named")?;
        let mut child = Command::new(program)
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Kept rather than inherited: a server that writes to stderr would otherwise print
            // over the editor's own screen.
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("{program}: {e}"))?;
        let stdin = child.stdin.take().ok_or("no stdin on the server")?;
        let stdout = child.stdout.take().ok_or("no stdout on the server")?;
        let (tx, rx) = mpsc::channel();
        let name = program.to_string();
        let pending: Arc<Mutex<HashMap<i64, Ask>>> = Arc::new(Mutex::new(HashMap::new()));
        let reader_pending = Arc::clone(&pending);
        std::thread::spawn(move || read_loop(BufReader::new(stdout), tx, name, reader_pending));

        let mut client = Client {
            name: program.to_string(),
            child,
            stdin,
            rx,
            next_id: 1,
            open: Vec::new(),
            utf16: true,
            ready: false,
            pending,
        };
        client.initialize(root)?;
        Ok(client)
    }

    fn initialize(&mut self, root: &Path) -> Result<(), String> {
        let params = InitializeParams {
            // Deprecated in the protocol and still what several servers actually read.
            #[allow(deprecated)]
            root_uri: uri_for(root),
            capabilities: ClientCapabilities {
                text_document: Some(TextDocumentClientCapabilities {
                    // Said out loud rather than left to the default, because the default is the
                    // one we depend on: a server told nothing about snippets may send them
                    // anyway, and `${1:self}` inserted into a buffer is a mess the user has to
                    // undo. This popup types a word, and this is where it says so.
                    completion: Some(CompletionClientCapabilities {
                        completion_item: Some(CompletionItemCapability {
                            snippet_support: Some(false),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                window: Some(WindowClientCapabilities::default()),
                // Which units this client can count columns in, said out loud.
                //
                // A server may only choose from what the client offers, and a client that
                // offers nothing has said "UTF-16 only" — so the UTF-8 arithmetic here could
                // never be reached by a server that follows the specification, and was only
                // ever exercised by the ones that do not. UTF-16 is written first because it
                // is the order of preference: it is the encoding every server must support,
                // and the one no server can decline.
                general: Some(GeneralClientCapabilities {
                    position_encodings: Some(vec![
                        PositionEncodingKind::UTF16,
                        PositionEncodingKind::UTF8,
                    ]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        let id = self.request("initialize", serde_json::to_value(params).unwrap_or(Value::Null))?;
        let _ = id;
        Ok(())
    }

    /// Sends a request and returns the id it went out with. The answer arrives on the channel
    /// like everything else — nothing here waits for it, because the frame loop must not stop.
    fn request(&mut self, method: &str, params: Value) -> Result<i64, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        Ok(id)
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    fn send(&mut self, message: &impl Serialize) -> Result<(), String> {
        let text = serde_json::to_string(message).map_err(|e| e.to_string())?;
        self.stdin.write_all(&frame(&text)).map_err(|e| e.to_string())?;
        self.stdin.flush().map_err(|e| e.to_string())
    }

    /// Answers the handshake reply. Sending `initialized` before it arrives is the one ordering
    /// rule servers actually enforce, so this waits for [`Event::Ready`] rather than firing it
    /// off after `initialize`.
    pub fn confirm_ready(&mut self, utf16: bool) {
        self.utf16 = utf16;
        let _ = self.notify("initialized", json!({}));
        self.ready = true;
    }

    /// Puts a reply to the server on the wire.
    ///
    /// The reader thread works out *what* to answer and this sends it, because the writer lives
    /// here: one thread reads, the frame loop writes, and a second handle on the pipe would be
    /// two threads interleaving half-written frames on the same stream.
    pub fn answer(&mut self, message: Value) {
        let _ = self.send(&message);
    }

    pub fn utf16(&self) -> bool {
        self.utf16
    }

    /// Whether the handshake is finished and the server will act on what it is told.
    ///
    /// Everything sent before `initialized` is the server's to ignore, and the ones that follow
    /// the specification do exactly that — silently, which is the trouble: a `didOpen` dropped
    /// this way costs the file its diagnostics for as long as it stays untouched.
    pub fn ready(&self) -> bool {
        self.ready
    }

    /// A column in the units this server settled on, from the character column the editor keeps.
    ///
    /// One function for every question that carries a position, because the two spellings of
    /// this arithmetic drifted apart once already: completion converted and the definition and
    /// hover requests sent the character column raw, which is right in English and wrong on
    /// every line with an accent in it.
    fn column_for(&self, line_text: &str, col: usize) -> usize {
        if self.utf16 {
            chars_to_utf16(line_text, col)
        } else {
            chars_to_utf8(line_text, col)
        }
    }

    pub fn did_open(&mut self, path: &Path, text: &str) {
        let Some(uri) = uri_for(path) else { return };
        if self.open.iter().any(|p| p == path) {
            self.did_change(path, text);
            return;
        }
        self.open.push(path.to_path_buf());
        let _ = self.notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": uri.as_str(), "languageId": language_id(path), "version": 1, "text": text
            }}),
        );
    }

    /// Sends the whole file rather than a diff.
    ///
    /// Incremental sync would send less, and would need the editor to describe every edit as a
    /// range — a second representation of what an edit is, kept in step with the rope by hand.
    /// The saving is not worth that: this is only sent once the typing pauses, and a source file
    /// is small next to the analysis the server is about to do to it anyway.
    pub fn did_change(&mut self, path: &Path, text: &str) {
        let Some(uri) = uri_for(path) else { return };
        if !self.open.iter().any(|p| p == path) {
            self.did_open(path, text);
            return;
        }
        let version = self.next_version();
        let _ = self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": {"uri": uri.as_str(), "version": version},
                "contentChanges": [{"text": text}]
            }),
        );
    }

    pub fn did_close(&mut self, path: &Path) {
        let Some(uri) = uri_for(path) else { return };
        self.open.retain(|p| p != path);
        let _ = self.notify(
            "textDocument/didClose",
            json!({"textDocument": {"uri": uri.as_str()}}),
        );
    }

    /// Asks what could be typed at a position, and returns the id the question went out with.
    ///
    /// `line_text` is the line the cursor is on and `col` its column in *characters*; the
    /// conversion to whatever the server counts in happens here, at the one place that knows
    /// which it negotiated. Nothing waits for the answer — it arrives as [`Event::Completion`]
    /// some frames later, and the popup is already on screen with the words from the buffer by
    /// then. That is the whole shape of this feature: the list is never empty while it waits.
    pub fn completion(&mut self, path: &Path, line: usize, line_text: &str, col: usize) -> Option<i64> {
        let uri = uri_for(path)?;
        let character = self.column_for(line_text, col);
        let id = self
            .request(
                "textDocument/completion",
                json!({
                    "textDocument": {"uri": uri.as_str()},
                    "position": {"line": line, "character": character}
                }),
            )
            .ok()?;
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(id, Ask::Completion);
        }
        Some(id)
    }

    fn next_version(&mut self) -> i64 {
        self.next_id += 1;
        self.next_id
    }

    /// Asks where the thing under the cursor is defined.
    ///
    /// Same position arithmetic as [`Self::completion`], and the same reason it goes out without
    /// waiting for the debounce: the question is about *this* text, and an answer about the text
    /// of four hundred milliseconds ago is not a slower right answer, it is a wrong one.
    pub fn definition(&mut self, path: &Path, line: usize, line_text: &str, col: usize) -> Option<i64> {
        self.position_request("textDocument/definition", Ask::Definition, path, line, line_text, col)
    }

    /// Asks what the thing under the cursor is.
    pub fn hover(&mut self, path: &Path, line: usize, line_text: &str, col: usize) -> Option<i64> {
        self.position_request("textDocument/hover", Ask::Hover, path, line, line_text, col)
    }

    /// The shape both of those share: a method name, a file and a place in it.
    fn position_request(
        &mut self,
        method: &str,
        ask: Ask,
        path: &Path,
        line: usize,
        line_text: &str,
        col: usize,
    ) -> Option<i64> {
        let uri = uri_for(path)?;
        let character = self.column_for(line_text, col);
        let params = json!({
            "textDocument": { "uri": uri.as_str() },
            "position": { "line": line, "character": character },
        });
        let id = self.request(method, params).ok()?;
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(id, ask);
        }
        Some(id)
    }

    pub fn try_recv(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }

    /// Asks the server to stop, then stops waiting for it.
    ///
    /// A polite shutdown is one round trip, and this runs while the user is closing the editor —
    /// so it is asked for and not waited on. `kill` follows regardless: an orphaned
    /// rust-analyzer holding a core is a far worse outcome than an impolite exit.
    pub fn stop(&mut self) {
        let _ = self.notify("shutdown", Value::Null);
        let _ = self.notify("exit", Value::Null);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.stop();
    }
}

/// What to call a file's language when announcing it.
///
/// Servers use this rather than the extension, and several of them use it to decide whether they
/// are interested at all: pyright told a file is `plaintext` accepts it and reports nothing about
/// it, which reads exactly like a server that is broken. Every extension the table above can
/// start a server for has a name here, in the spelling the protocol's own list uses — the same
/// one every other editor sends, which is what makes it the spelling servers match against.
fn language_id(path: &Path) -> &'static str {
    let extension =
        path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    match extension.as_str() {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "ts" => "typescript",
        // The React dialects are languages of their own to a server: `.tsx` told it is plain
        // TypeScript is a file whose every tag is a syntax error.
        "tsx" => "typescriptreact",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "go" => "go",
        // `.h` is C: it is a C++ header just as often, and clangd works that out from the
        // compilation database rather than from what we call it, so the safe half of an
        // ambiguity that only it can resolve.
        "c" | "h" => "c",
        "cpp" | "cxx" | "cc" | "hpp" | "hxx" | "hh" => "cpp",
        "m" => "objective-c",
        "mm" => "objective-cpp",
        "lua" => "lua",
        "zig" => "zig",
        "rb" => "ruby",
        "sh" | "bash" => "shellscript",
        "json" => "json",
        "jsonc" => "jsonc",
        "toml" => "toml",
        "tex" | "latex" => "latex",
        "bib" => "bibtex",
        // A language nobody named — reachable through the user's own server table, which can
        // point any extension at any program. Saying "plain text" is the honest answer: we do
        // not know, and inventing a name would be a guess the server would act on.
        _ => "plaintext",
    }
}

/// The words of one completion answer, in the order the protocol says to offer them.
///
/// Sorted by `sortText` — which is the server's own judgement of what is most likely wanted here,
/// and the only thing in the reply that carries it. The array order is not that judgement: the
/// protocol tells clients to sort, so servers do not bother. Ignoring it and then claiming the
/// position in the list means something would be inventing a ranking out of nothing.
fn completion_words(result: Option<&Value>) -> Vec<String> {
    let Some(result) = result else { return Vec::new() };
    // Either shape the protocol allows — a bare array, or a list with the array under `items` —
    // reached for by name rather than by deserialising the reply whole. `CompletionList` has a
    // required `isIncomplete`, so a server that leaves it out would cost the entire list: an
    // empty popup, indistinguishable from a server that had nothing to say. `null` is that
    // second thing, and it is an ordinary answer rather than an error.
    let raw = match result {
        Value::Array(items) => items,
        _ => match result.get("items").and_then(Value::as_array) {
            Some(items) => items,
            None => return Vec::new(),
        },
    };
    let mut items: Vec<CompletionItem> = raw
        .iter()
        // One item that will not read is one row lost, not the list. They are independent, and
        // an unknown field in the twentieth is no reason to drop the first nineteen.
        .filter_map(|v| serde_json::from_value::<CompletionItem>(v.clone()).ok())
        .collect();
    let key = |i: &CompletionItem| i.sort_text.clone().unwrap_or_else(|| i.label.clone());
    items.sort_by_key(key);
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(items.len());
    for item in &items {
        let Some(word) = word_of(item) else { continue };
        // Two items can reduce to the same word — an inherent method and a trait one, say. They
        // are one row in a list that types a word, and the first is the better ranked.
        if seen.insert(word.clone()) {
            out.push(word);
        }
    }
    out
}

/// How to count columns, from what the server named in its handshake reply.
///
/// Only two answers are possible here, because only two were offered. `utf-32` is a real value in
/// the specification and this client cannot count in it — nor did it ask to; a server naming it,
/// or naming something a later version of the protocol invents, has answered a question it was
/// not asked. UTF-16 is the reading for all of those: it is the mandatory encoding, the one every
/// server must be able to speak, and therefore the only assumption that cannot leave the two ends
/// counting in different units.
pub fn negotiated_utf16(encoding: Option<&str>) -> bool {
    encoding != Some("utf-8")
}

/// The answer owed to a message the server sent us, or `None` when none is.
///
/// A message carrying both an `id` and a `method` is a request, and JSON-RPC says a request is
/// answered — a server left waiting on one may hold back the work behind it, and rust-analyzer's
/// configuration request is the first thing it asks. Notifications carry no `id` and are the
/// server talking to itself; nothing is owed for those.
///
/// The id is passed back exactly as it arrived, number or string, because it is the server's own
/// bookkeeping and not ours to normalise.
pub fn reply_to(message: &Value) -> Option<Value> {
    let method = message.get("method")?.as_str()?;
    let id = message.get("id")?.clone();
    match method {
        // "What are your settings for these scopes?" — answered with a null per item, which the
        // protocol reads as "nothing configured, use your defaults". There is no user-facing
        // setting to hand over yet, and an empty object would be a claim rather than an absence.
        "workspace/configuration" => {
            let items = message.pointer("/params/items").and_then(Value::as_array);
            let result = match items {
                Some(items) => Value::Array(vec![Value::Null; items.len()]),
                None => Value::Null,
            };
            Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
        }
        // Everything else is work this client does not do — and says so, rather than leaving the
        // server to time out or to wait forever. Refusing is the honest answer: we advertised no
        // capability for any of it, so a request for one is a server asking anyway.
        _ => {
            let message = format!("{method} is not something this client does");
            Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": message}}))
        }
    }
}

/// The reader thread. Everything the server says arrives here and leaves as an [`Event`].
fn read_loop(
    mut reader: BufReader<impl Read>,
    tx: Sender<Event>,
    name: String,
    pending: Arc<Mutex<HashMap<i64, Ask>>>,
) {
    let mut handshook = false;
    loop {
        let message = match read_message(&mut reader) {
            Ok(Some(text)) => text,
            Ok(None) => {
                let _ = tx.send(Event::Stopped { detail: format!("{name} closed its output") });
                return;
            }
            Err(e) => {
                let _ = tx.send(Event::Stopped { detail: format!("{name}: {e}") });
                return;
            }
        };
        let Ok(value) = serde_json::from_str::<Value>(&message) else { continue };
        match value.get("method").and_then(Value::as_str) {
            Some("textDocument/publishDiagnostics") => {
                let Some(params) = value.get("params") else { continue };
                let Ok(params) = serde_json::from_value::<PublishDiagnosticsParams>(params.clone())
                else {
                    continue;
                };
                let Some(path) = path_for(&params.uri) else { continue };
                let _ = tx.send(Event::Diagnostics { path, raw: params.diagnostics });
            }
            // Anything else the server says on its own account. Progress notes and log messages
            // are dropped — this release draws squiggles — but a *request* is answered even when
            // the answer is a refusal: a server waiting on a reply that never comes is a server
            // that may never get as far as the diagnostics.
            Some(_) => {
                if let Some(reply) = reply_to(&value) {
                    let _ = tx.send(Event::Answer { message: reply });
                }
            }
            None => {
                // A response, and the id says which question it answers. Asked first, so a
                // completion reply is never mistaken for anything else — including on a server
                // that answers `initialize` late enough for a request to overtake it.
                let asked = value
                    .get("id")
                    .and_then(Value::as_i64)
                    .and_then(|id| Some((id, pending.lock().ok()?.remove(&id)?)));
                if let Some((id, ask)) = asked {
                    let result = value.get("result");
                    let _ = tx.send(match ask {
                        Ask::Completion => {
                            Event::Completion { id, words: completion_words(result) }
                        }
                        Ask::Definition => {
                            Event::Definition { id, target: first_location(result) }
                        }
                        Ask::Hover => Event::Hover { id, text: hover_text(result) },
                    });
                    continue;
                }
                // The first response is the answer to `initialize`, the only request sent before
                // this point. It carries how the server wants positions counted.
                if !handshook && value.get("id").is_some() && value.get("result").is_some() {
                    handshook = true;
                    let encoding = value
                        .pointer("/result/capabilities/positionEncoding")
                        .and_then(Value::as_str);
                    let _ = tx.send(Event::Ready { utf16: negotiated_utf16(encoding) });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{Position, Range};

    /// The requests still out, as the client would have left them for the reader thread. Every
    /// one a completion unless a test says otherwise, which is what all but one of them wants.
    fn pending(ids: &[i64]) -> Arc<Mutex<HashMap<i64, Ask>>> {
        Arc::new(Mutex::new(ids.iter().map(|id| (*id, Ask::Completion)).collect()))
    }

    fn pending_asks(asks: &[(i64, Ask)]) -> Arc<Mutex<HashMap<i64, Ask>>> {
        Arc::new(Mutex::new(asks.iter().copied().collect()))
    }

    fn item(label: &str) -> CompletionItem {
        CompletionItem { label: label.to_string(), ..Default::default() }
    }

    fn diag(line: u32, from: u32, to: u32, message: &str) -> Diagnostic {
        Diagnostic {
            range: Range {
                start: Position { line, character: from },
                end: Position { line, character: to },
            },
            severity: Some(DiagnosticSeverity::ERROR),
            message: message.to_string(),
            ..Default::default()
        }
    }

    /// The two column conversions are opposites, and the reason both exist is that they stop
    /// agreeing exactly where testing in English would never look.
    #[test]
    fn columns_convert_back_and_forth_and_part_company_on_an_emoji() {
        let plain = "let x = 1;";
        for col in 0..=plain.chars().count() {
            assert_eq!(chars_to_utf16(plain, col), col, "ASCII counts the same either way");
            assert_eq!(utf16_to_chars(plain, col), col);
        }
        // Three characters before the cursor, one of which the protocol counts as two.
        let line = "a🎈b";
        assert_eq!(chars_to_utf16(line, 3), 4);
        assert_eq!(utf16_to_chars(line, 4), 3);
        // And an accent, which is one of each — the case that makes the emoji look like an
        // exception rather than the rule it is.
        assert_eq!(chars_to_utf16("caffè", 5), 5);
    }

    /// The other pair, for the servers that count in bytes. The line is the one this whole
    /// family of bugs is invisible without: ASCII counts the same in all three units, so a
    /// conversion left out entirely passes every test written in English.
    #[test]
    fn columns_convert_back_and_forth_in_bytes_as_well() {
        let plain = "let x = 1;";
        for col in 0..=plain.chars().count() {
            assert_eq!(chars_to_utf8(plain, col), col, "ASCII counts the same either way");
            assert_eq!(utf8_to_chars(plain, col), col);
        }
        // `// città`: seven ASCII characters and an `à` that takes two bytes.
        let line = "// città";
        assert_eq!(chars_to_utf8(line, 7), 7, "up to the accent, bytes and characters agree");
        assert_eq!(chars_to_utf8(line, 8), 9, "and the accent is two of them");
        assert_eq!(utf8_to_chars(line, 7), 7);
        assert_eq!(utf8_to_chars(line, 9), 8, "the end of the line, in either counting");
        // An offset inside a character rounds left, onto the character it fell into — which is
        // the one the server was talking about.
        assert_eq!(utf8_to_chars(line, 8), 7);
        // An emoji is four bytes and one character, so the drift is three columns per emoji.
        let emoji = "a🦀bc";
        assert_eq!(chars_to_utf8(emoji, 2), 5);
        assert_eq!(utf8_to_chars(emoji, 5), 2);
        assert_eq!(utf8_to_chars(emoji, 3), 1, "inside the emoji is still the emoji");
        // Past the end clamps rather than panicking on a diagnostic about older text.
        assert_eq!(utf8_to_chars("ab", 99), 2);
        assert_eq!(utf8_to_chars("", 0), 0);
    }

    /// The same line, arriving as a diagnostic from a server counting in bytes. The columns are
    /// the server's own and mean nothing to the buffer until this conversion happens.
    #[test]
    fn a_byte_column_becomes_a_character_column_on_the_way_in() {
        let lines = vec!["let città = 1;".to_string()];
        // Bytes 4 to 10 are `città`; characters 4 to 9 are the same five letters.
        let marks = marks_from(&[diag(0, 4, 10, "unused variable")], &lines, false);
        assert_eq!((marks[0].start, marks[0].end), (4, 9));
        // And the same numbers read as UTF-16 would be five characters further along a line
        // that is only fourteen long — which is what the old arithmetic did.
        let utf16 = marks_from(&[diag(0, 4, 10, "unused variable")], &lines, true);
        assert_eq!((utf16[0].start, utf16[0].end), (4, 10));
    }

    /// Only two answers are possible, because only two were offered. Everything else is a server
    /// naming a unit this client never said it could count in.
    #[test]
    fn the_encoding_is_the_one_that_was_offered_or_the_mandatory_one() {
        assert!(!negotiated_utf16(Some("utf-8")));
        assert!(negotiated_utf16(Some("utf-16")));
        // Real in the specification, never advertised by us, and not something this client can
        // count in — read as the encoding every server must support.
        assert!(negotiated_utf16(Some("utf-32")));
        assert!(negotiated_utf16(Some("utf-7-and-a-half")));
        // Said nothing at all, which the protocol defines as UTF-16.
        assert!(negotiated_utf16(None));
    }

    /// A request is a message with both an id and a method, and it is owed an answer — a server
    /// waiting on one may never get as far as the diagnostics.
    #[test]
    fn a_question_from_the_server_is_answered_and_a_remark_is_not() {
        // Nothing is owed for a notification, however much it looks like a request otherwise.
        assert_eq!(reply_to(&json!({"jsonrpc": "2.0", "method": "window/logMessage"})), None);
        // Nor for a response, which has an id and no method.
        assert_eq!(reply_to(&json!({"jsonrpc": "2.0", "id": 4, "result": null})), None);

        // Configuration: a null per item, which reads as "nothing set, use your defaults".
        let asked = json!({
            "jsonrpc": "2.0", "id": 3, "method": "workspace/configuration",
            "params": {"items": [{"section": "rust-analyzer"}, {"section": "files"}]}
        });
        let reply = reply_to(&asked).unwrap();
        assert_eq!(reply["id"], json!(3));
        assert_eq!(reply["result"], json!([null, null]));

        // Anything else is refused rather than left unanswered, and the id comes back in the
        // shape it arrived in — a string id is the server's own bookkeeping.
        let other = json!({"jsonrpc": "2.0", "id": "abc", "method": "client/registerCapability"});
        let reply = reply_to(&other).unwrap();
        assert_eq!(reply["id"], json!("abc"));
        assert_eq!(reply["error"]["code"], json!(-32601));
        assert!(reply.get("result").is_none(), "an error and a result are not both sent");
    }

    /// The reader thread has no writer, so the reply comes back out as an event for the frame
    /// loop to send. This is the part that would otherwise be a second thread writing frames
    /// into the same pipe.
    #[test]
    fn the_reply_travels_back_through_the_channel_that_owns_the_writer() {
        let wire = frame(
            r#"{"jsonrpc":"2.0","id":2,"method":"workspace/configuration",
                "params":{"items":[{"section":"rust-analyzer"}]}}"#,
        );
        let (tx, rx) = mpsc::channel();
        read_loop(BufReader::new(&wire[..]), tx, "stub".to_string(), pending(&[]));
        let events: Vec<Event> = rx.into_iter().collect();
        match &events[0] {
            Event::Answer { message } => {
                assert_eq!(message["id"], json!(2));
                assert_eq!(message["result"], json!([null]));
            }
            _ => panic!("the question has to come back out as an answer to send"),
        }
    }

    /// A server told a `.py` is `plaintext` accepts it and then reports nothing about it, which
    /// on screen is indistinguishable from a server that is broken.
    #[test]
    fn every_file_a_server_is_started_for_has_a_language_to_call_it() {
        assert_eq!(language_id(Path::new("main.rs")), "rust");
        assert_eq!(language_id(Path::new("app.py")), "python");
        assert_eq!(language_id(Path::new("main.go")), "go");
        assert_eq!(language_id(Path::new("main.lua")), "lua");
        assert_eq!(language_id(Path::new("Cargo.toml")), "toml");
        // The React dialects are languages of their own: `.tsx` called plain TypeScript is a
        // file whose every tag is a syntax error.
        assert_eq!(language_id(Path::new("app.ts")), "typescript");
        assert_eq!(language_id(Path::new("app.tsx")), "typescriptreact");
        assert_eq!(language_id(Path::new("app.jsx")), "javascriptreact");
        assert_eq!(language_id(Path::new("app.mjs")), "javascript");
        assert_eq!(language_id(Path::new("go.sh")), "shellscript");
        assert_eq!(language_id(Path::new("paper.bib")), "bibtex");
        // Case does not decide it, for the same reason it does not decide which server runs.
        assert_eq!(language_id(Path::new("MAIN.RS")), "rust");
        // Every extension the built-in table can start a server for has a name here; a language
        // reached only through the user's own table may not, and says so honestly.
        for (extensions, argv) in SERVERS {
            for extension in *extensions {
                let named = language_id(Path::new(&format!("file.{extension}")));
                let program = argv[0];
                assert_ne!(named, "plaintext", "{program} would be told {extension} is plain text");
            }
        }
        assert_eq!(language_id(Path::new("notes.txt")), "plaintext");
        assert_eq!(language_id(Path::new("Makefile")), "plaintext");
    }

    /// The popup types a word, so an item has to come down to one — brackets and all the rest of
    /// what a label is written to *show* stay out of the buffer.
    #[test]
    fn an_item_is_reduced_to_the_word_it_would_type() {
        assert_eq!(word_of(&item("push_str(…)")).as_deref(), Some("push_str"));
        assert_eq!(word_of(&item("println!(…)")).as_deref(), Some("println"));
        assert_eq!(word_of(&item("HashMap")).as_deref(), Some("HashMap"));
        // No word at the head at all: an operator, a lifetime, a reference type.
        assert_eq!(word_of(&item("&str")), None);
        assert_eq!(word_of(&item("'static")), None);
    }

    #[test]
    fn what_a_server_would_type_beats_what_it_would_show() {
        let mut with_insert = item("push_str(…)");
        with_insert.insert_text = Some("push_str".to_string());
        assert_eq!(word_of(&with_insert).as_deref(), Some("push_str"));

        // A snippet, from a server that ignored the handshake. The label is the lesser wrong:
        // `${1:value}` in a buffer is something the user has to go back and undo.
        let mut snippet = item("push_str(…)");
        snippet.insert_text = Some("push_str(${1:value})".to_string());
        snippet.insert_text_format = Some(InsertTextFormat::SNIPPET);
        assert_eq!(word_of(&snippet).as_deref(), Some("push_str"));
    }

    /// `sortText` is the server's ranking and the array order is not, so the words come out in
    /// the order the protocol says to offer them — not the order they happened to be written in.
    /// An item without one falls back to its label, which is the protocol's own rule and puts it
    /// after every numbered suggestion: a server that ranked some of its answers and not others
    /// meant the unranked ones less.
    #[test]
    fn the_words_come_out_in_the_servers_own_order() {
        let result = serde_json::json!({"items": [
            {"label": "zebra", "sortText": "0001"},
            {"label": "alpha", "sortText": "0009"},
            {"label": "beta"}
        ]});
        assert_eq!(completion_words(Some(&result)), vec!["zebra", "alpha", "beta"]);
    }

    /// Two items can reduce to the same word — an inherent method and a trait one. One row.
    #[test]
    fn two_items_that_type_the_same_word_are_one_row() {
        let result = serde_json::json!([
            {"label": "len()", "sortText": "0001"},
            {"label": "len(…)", "sortText": "0002"},
            {"label": "&self"}
        ]);
        assert_eq!(completion_words(Some(&result)), vec!["len"]);
    }

    /// `null` is an ordinary answer — the server has nothing to say here — and so is a shape we
    /// do not know. Neither is worth a guess at what might have been meant.
    #[test]
    fn nothing_to_offer_is_an_answer_not_a_failure() {
        assert!(completion_words(Some(&serde_json::Value::Null)).is_empty());
        assert!(completion_words(Some(&serde_json::json!("surprise"))).is_empty());
        assert!(completion_words(None).is_empty());
    }

    /// The id is the whole guard. A response we never asked for is a response for somebody else.
    #[test]
    fn a_completion_answer_is_recognised_by_the_id_it_carries() {
        let mut wire = Vec::new();
        wire.extend(frame(
            r#"{"jsonrpc":"2.0","id":7,"result":{"items":[{"label":"config_path"}]}}"#,
        ));
        // Same shape, an id nobody is waiting for. It falls through to the handshake check, which
        // wants the first response — so this must not come out as a completion *or* as `Ready`
        // once the real handshake has been seen.
        wire.extend(frame(r#"{"jsonrpc":"2.0","id":99,"result":{"items":[{"label":"nope"}]}}"#));
        let (tx, rx) = mpsc::channel();
        read_loop(BufReader::new(&wire[..]), tx, "stub".to_string(), pending(&[7]));
        let events: Vec<Event> = rx.into_iter().collect();
        match &events[0] {
            Event::Completion { id, words } => {
                assert_eq!(*id, 7);
                assert_eq!(words, &["config_path".to_string()]);
            }
            _ => panic!("the answer to the question we asked has to arrive"),
        }
        assert!(
            !events[1..].iter().any(|e| matches!(e, Event::Completion { .. })),
            "an id nobody is waiting for is not an answer to anything"
        );
    }

    #[test]
    fn a_message_is_framed_with_carriage_returns() {
        let out = frame("{}");
        assert_eq!(out, b"Content-Length: 2\r\n\r\n{}");
    }

    #[test]
    fn framing_survives_a_round_trip() {
        let mut wire = Vec::new();
        wire.extend(frame(r#"{"one":1}"#));
        wire.extend(frame(r#"{"two":2}"#));
        let mut reader = std::io::BufReader::new(&wire[..]);
        assert_eq!(read_message(&mut reader).unwrap().as_deref(), Some(r#"{"one":1}"#));
        assert_eq!(read_message(&mut reader).unwrap().as_deref(), Some(r#"{"two":2}"#));
        assert_eq!(read_message(&mut reader).unwrap(), None, "end of stream, not an error");
    }

    #[test]
    fn a_body_is_read_by_length_not_by_lines() {
        // rust-analyzer's messages contain newlines, and a reader that stopped at one would put
        // the stream permanently out of step with the sender.
        let payload = "{\"m\":\"line one\nline two\"}";
        let wire = frame(payload);
        let mut reader = std::io::BufReader::new(&wire[..]);
        assert_eq!(read_message(&mut reader).unwrap().as_deref(), Some(payload));
    }

    #[test]
    fn header_fields_we_do_not_know_are_skipped() {
        let mut wire = b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\n".to_vec();
        wire.extend(b"Content-Length: 2\r\n\r\n{}");
        let mut reader = std::io::BufReader::new(&wire[..]);
        assert_eq!(read_message(&mut reader).unwrap().as_deref(), Some("{}"));
    }

    #[test]
    fn a_header_without_a_length_is_an_error_rather_than_a_guess() {
        let wire = b"Content-Type: text/plain\r\n\r\n{}".to_vec();
        let mut reader = std::io::BufReader::new(&wire[..]);
        assert!(read_message(&mut reader).is_err());
    }

    /// The failure this guards against is silent: the URI simply never matches a file the server
    /// knows, and diagnostics stop arriving for that project with nothing on screen to say why.
    #[test]
    fn a_path_with_spaces_and_accents_survives_the_round_trip() {
        let path = Path::new("/Users/matteo/Documenti/prova nuova/città.rs");
        let uri = uri_for(path).unwrap();
        assert!(uri.as_str().contains("prova%20nuova"), "{}", uri.as_str());
        assert!(!uri.as_str().contains('à'), "the accent must be escaped: {}", uri.as_str());
        assert_eq!(path_for(&uri).unwrap(), path);
    }

    #[test]
    fn a_windows_path_becomes_a_uri_and_comes_back() {
        let uri = uri_for(Path::new(r"C:\src\main.rs")).unwrap();
        assert_eq!(uri.as_str(), "file:///C:/src/main.rs");
        assert_eq!(path_for(&uri).unwrap(), PathBuf::from(r"C:\src\main.rs".replace('\\', "/")));
    }

    #[test]
    fn a_plain_path_is_not_mangled() {
        let uri = uri_for(Path::new("/src/main.rs")).unwrap();
        assert_eq!(uri.as_str(), "file:///src/main.rs");
    }

    /// This one was found by driving the editor, not by reading it. An editor opened with `.` as
    /// its project holds `./src/main.rs`, and the old code prefixed a slash and produced
    /// `file:///./src/main.rs` — which parses, is sent, is echoed back by the server, and names
    /// nothing at either end. Nothing was underlined and nothing said why.
    #[test]
    fn a_relative_path_has_no_uri_and_says_so() {
        assert!(uri_for(Path::new("./src/main.rs")).is_none());
        assert!(uri_for(Path::new("src/main.rs")).is_none());
        assert!(uri_for(Path::new("main.rs")).is_none());
        // And the rooted forms are still accepted, whichever platform wrote them — asked of the
        // text, because a Windows path looks relative to a Unix build.
        assert!(uri_for(Path::new("/src/main.rs")).is_some());
        assert!(uri_for(Path::new(r"C:\src\main.rs")).is_some());
    }

    /// The bug this prevents is a squiggle that drifts one column left per emoji on the line —
    /// visible only to whoever writes them, which is why it needs a test rather than a look.
    #[test]
    fn utf16_columns_become_character_columns() {
        // Plain text: the two countings agree, and nothing moves.
        assert_eq!(utf16_to_chars("let x = 1;", 4), 4);
        // `è` is one character and one UTF-16 unit.
        assert_eq!(utf16_to_chars("// città", 8), 8);
        // An emoji is one character but two UTF-16 units, so column 4 in the protocol's counting
        // is column 3 in the editor's.
        assert_eq!(utf16_to_chars("a🦀bc", 4), 3);
        // Past the end clamps to the end rather than panicking on a stale diagnostic.
        assert_eq!(utf16_to_chars("ab", 99), 2);
    }

    #[test]
    fn marks_land_on_the_text_they_describe() {
        let lines = vec!["let x = 1;".to_string(), "let y: u8 = 300;".to_string()];
        let marks = marks_from(&[diag(1, 12, 15, "literal out of range")], &lines, true);
        assert_eq!(marks.len(), 1);
        assert_eq!((marks[0].line, marks[0].start, marks[0].end), (1, 12, 15));
        assert_eq!(marks[0].severity, Severity::Error);
    }

    #[test]
    fn an_empty_range_still_covers_a_cell() {
        // "expected something here" points between two characters, and a squiggle of no width
        // cannot be seen.
        let lines = vec!["let x = ".to_string()];
        let marks = marks_from(&[diag(0, 8, 8, "expected expression")], &lines, true);
        assert_eq!((marks[0].start, marks[0].end), (8, 9));
    }

    #[test]
    fn a_range_across_lines_is_shown_on_the_first_one() {
        let lines = vec!["fn f() {".to_string(), "}".to_string()];
        let mut d = diag(0, 3, 1, "unclosed");
        d.range.end.line = 1;
        let marks = marks_from(&[d], &lines, true);
        assert_eq!((marks[0].line, marks[0].start, marks[0].end), (0, 3, 8));
    }

    /// A diagnostic for a line the buffer no longer has describes a file that has moved on. An
    /// underline in the wrong place reads as a fact about the code, so nothing is better.
    #[test]
    fn a_diagnostic_past_the_end_of_the_buffer_is_dropped() {
        let lines = vec!["one line".to_string()];
        assert!(marks_from(&[diag(40, 0, 3, "stale")], &lines, true).is_empty());
    }

    #[test]
    fn an_edit_is_sent_once_the_typing_stops() {
        use std::time::Duration;
        // Mid-word: the revision has moved but the pause has not happened.
        assert!(!should_send(Some(3), 7, Duration::from_millis(50)));
        // The pause happened.
        assert!(should_send(Some(3), 7, QUIET));
        // Nothing changed, so nothing is sent however long the wait.
        assert!(!should_send(Some(7), 7, Duration::from_secs(30)));
        // A file the server has never been told about is a change.
        assert!(should_send(None, 1, QUIET));
    }

    /// The reader thread, driven from a canned transcript instead of a process. This is the part
    /// that decides what the rest of the program ever hears, so it is worth exercising without
    /// needing a language server installed to do it.
    #[test]
    fn a_server_transcript_becomes_the_events_the_app_sees() {
        let mut wire = Vec::new();
        // The answer to `initialize`, saying it counts in UTF-8.
        wire.extend(frame(
            r#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{"positionEncoding":"utf-8"}}}"#,
        ));
        // Something the server says on its own account and we do not act on.
        wire.extend(frame(r#"{"jsonrpc":"2.0","method":"window/logMessage","params":{"type":3,"message":"hello"}}"#));
        wire.extend(frame(
            r#"{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{
                 "uri":"file:///tmp/prova%20nuova/main.rs",
                 "diagnostics":[{"range":{"start":{"line":2,"character":4},
                                          "end":{"line":2,"character":9}},
                                 "severity":2,"message":"unused variable: `x`"}]}}"#,
        ));

        let (tx, rx) = mpsc::channel();
        read_loop(BufReader::new(&wire[..]), tx, "stub".to_string(), pending(&[]));
        let events: Vec<Event> = rx.into_iter().collect();
        assert_eq!(events.len(), 3, "ready, diagnostics, and the end of the stream");

        match &events[0] {
            Event::Ready { utf16 } => assert!(!utf16, "the server asked for UTF-8"),
            _ => panic!("the first thing out has to be the handshake"),
        }
        match &events[1] {
            Event::Diagnostics { path, raw } => {
                assert_eq!(path, &PathBuf::from("/tmp/prova nuova/main.rs"), "the space is decoded");
                assert_eq!(raw.len(), 1);
                assert_eq!(raw[0].severity, Some(DiagnosticSeverity::WARNING));
                let lines = vec![String::new(), String::new(), "    let x = 1;".to_string()];
                let marks = marks_from(raw, &lines, false);
                assert_eq!((marks[0].line, marks[0].start, marks[0].end), (2, 4, 9));
                assert_eq!(marks[0].severity, Severity::Warning);
            }
            _ => panic!("the diagnostics did not come through"),
        }
        match &events[2] {
            Event::Stopped { detail } => assert!(detail.contains("stub"), "{detail}"),
            _ => panic!("the end of the stream has to be announced, or nothing clears the marks"),
        }
    }

    /// An empty list is a message, not a silence: it is the only thing that makes a fixed error
    /// stop being drawn.
    #[test]
    fn clearing_a_files_diagnostics_is_itself_an_event() {
        let wire = frame(
            r#"{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics",
                "params":{"uri":"file:///tmp/main.rs","diagnostics":[]}}"#,
        );
        let (tx, rx) = mpsc::channel();
        read_loop(BufReader::new(&wire[..]), tx, "stub".to_string(), pending(&[]));
        let events: Vec<Event> = rx.into_iter().collect();
        match &events[0] {
            Event::Diagnostics { path, raw } => {
                assert_eq!(path, &PathBuf::from("/tmp/main.rs"));
                assert!(raw.is_empty());
            }
            _ => panic!("an empty list still has to arrive"),
        }
    }

    /// A server that says something malformed must not take the client with it: the next message
    /// is still read, because the framing is what keeps the stream in step, not the JSON.
    #[test]
    fn a_message_that_is_not_json_is_stepped_over() {
        let mut wire = Vec::new();
        wire.extend(frame("this is not json at all"));
        wire.extend(frame(
            r#"{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics",
                "params":{"uri":"file:///tmp/main.rs","diagnostics":[]}}"#,
        ));
        let (tx, rx) = mpsc::channel();
        read_loop(BufReader::new(&wire[..]), tx, "stub".to_string(), pending(&[]));
        let events: Vec<Event> = rx.into_iter().collect();
        assert!(matches!(events[0], Event::Diagnostics { .. }), "the good message still arrives");
    }

    fn argv(path: &str, configured: &[(&str, &str)]) -> Option<Vec<String>> {
        let configured: BTreeMap<String, String> =
            configured.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        server_for(Path::new(path), &configured)
    }

    #[test]
    fn a_server_is_offered_only_for_a_language_we_have_one_for() {
        assert_eq!(argv("main.rs", &[]), Some(vec!["rust-analyzer".to_string()]));
        // The arguments are part of the entry, and not decoration: without `--stdio` this one
        // listens on a socket and looks like a server that started and never answered.
        assert_eq!(
            argv("app.ts", &[]),
            Some(vec!["typescript-language-server".to_string(), "--stdio".to_string()])
        );
        // Case does not decide it. `README.MD` off a Windows share is the same file as `.md`.
        assert_eq!(argv("MAIN.RS", &[]), Some(vec!["rust-analyzer".to_string()]));
        assert_eq!(argv("notes.md", &[]), None);
        assert_eq!(argv("Makefile", &[]), None);
    }

    /// The user's own table is what keeps the built-in list from being the limit: a language
    /// nobody put in it, or the fork of a server somebody keeps in `~/bin`, without waiting for
    /// a release.
    #[test]
    fn the_users_own_table_wins_over_the_built_in_one() {
        // A language the built-in table says nothing about.
        assert_eq!(
            argv("thing.ml", &[("ml", "ocamllsp")]),
            Some(vec!["ocamllsp".to_string()])
        );
        // And one it does — replaced, arguments and all.
        assert_eq!(
            argv("main.rs", &[("rs", "my-analyzer --stdio")]),
            Some(vec!["my-analyzer".to_string(), "--stdio".to_string()])
        );
        // Set to nothing is how a built-in is turned off. The alternative would be a second
        // setting whose whole job is to say "not that one".
        assert_eq!(argv("main.rs", &[("rs", "")]), None);
        assert_eq!(argv("main.rs", &[("rs", "   ")]), None);
    }

    /// A definition comes back in any of three shapes and they are all correct, so all three are
    /// read rather than one being preferred and the others quietly missed.
    #[test]
    fn a_definition_is_read_in_whichever_shape_it_arrives() {
        let location = json!({
            "uri": "file:///src/main.rs",
            "range": { "start": { "line": 41, "character": 8 }, "end": { "line": 41, "character": 12 } }
        });
        let want = Jump { path: PathBuf::from("/src/main.rs"), line: 41, column: 8 };
        assert_eq!(first_location(Some(&location)), Some(want.clone()));
        // An array of them: the first is taken. More than one definition is real — a trait
        // method with several implementations — and going to the first is what every editor does
        // before it grows a picker for them.
        assert_eq!(first_location(Some(&json!([location]))), Some(want.clone()));
        // A LocationLink, which names the same two things differently.
        let link = json!([{
            "targetUri": "file:///src/main.rs",
            "targetRange": { "start": { "line": 40, "character": 0 }, "end": { "line": 45, "character": 1 } },
            "targetSelectionRange": { "start": { "line": 41, "character": 8 }, "end": { "line": 41, "character": 12 } }
        }]);
        assert_eq!(first_location(Some(&link)), Some(want), "the name, not the whole body");

        // "I know of no definition" is an answer and arrives as null or as an empty array.
        assert_eq!(first_location(Some(&Value::Null)), None);
        assert_eq!(first_location(Some(&json!([]))), None);
        assert_eq!(first_location(None), None);
    }

    /// A hover is documentation — a signature, a rule, then paragraphs. One line of a status bar
    /// is what there is, and the first meaningful line is the part anyone looks at.
    #[test]
    fn a_hover_comes_down_to_the_line_worth_reading() {
        let markup = json!({ "contents": {
            "kind": "markdown",
            "value": "```rust\nfn push_str(&mut self, string: &str)\n```\n\n---\n\nAppends a string slice."
        }});
        assert_eq!(
            hover_text(Some(&markup)).as_deref(),
            Some("fn push_str(&mut self, string: &str)"),
            "the fence and the rule are markup for a renderer, not text"
        );
        // The two older shapes, both still sent by servers in use.
        assert_eq!(hover_text(Some(&json!({ "contents": "usize" }))).as_deref(), Some("usize"));
        assert_eq!(
            hover_text(Some(&json!({ "contents": [{ "language": "go", "value": "var x int" }] }))).as_deref(),
            Some("var x int")
        );
        // Nothing to say, said as nothing rather than as an empty line.
        assert_eq!(hover_text(Some(&json!({ "contents": "" }))), None);
        assert_eq!(hover_text(Some(&json!({ "contents": "```\n```" }))), None);
        assert_eq!(hover_text(None), None);
    }

    /// The reader tells the three kinds of answer apart by what the id was asking, and nothing
    /// else. The set it used to keep could only say "a completion or not one", which was true
    /// while there was one other kind of request and wrong the moment there were three.
    #[test]
    fn an_answer_is_read_as_the_question_it_answers() {
        let bodies = [
            (1, Ask::Completion, json!(["alpha"])),
            (2, Ask::Definition, json!({
                "uri": "file:///a.rs",
                "range": { "start": { "line": 3, "character": 1 }, "end": { "line": 3, "character": 4 } }
            })),
            (3, Ask::Hover, json!({ "contents": "usize" })),
        ];
        let mut wire = Vec::new();
        for (id, _, result) in &bodies {
            wire.extend(frame(&json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()));
        }
        let (tx, rx) = mpsc::channel();
        let asks: Vec<(i64, Ask)> = bodies.iter().map(|(id, ask, _)| (*id, *ask)).collect();
        read_loop(BufReader::new(&wire[..]), tx, "stub".to_string(), pending_asks(&asks));
        let events: Vec<Event> = rx.into_iter().collect();
        assert!(matches!(events[0], Event::Completion { id: 1, .. }));
        assert!(matches!(events[1], Event::Definition { id: 2, target: Some(_) }));
        assert!(
            matches!(&events[2], Event::Hover { id: 3, text } if text.as_deref() == Some("usize"))
        );
    }

    /// The whole client against a real process: spawn, write the handshake, read the answer,
    /// announce a file, get diagnostics back for the URI we sent — which is the part a canned
    /// transcript cannot check, because a stub that invented its own URI would pass even with
    /// the path encoding broken.
    #[test]
    #[cfg(unix)]
    fn the_client_talks_to_a_real_process() {
        let stub = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/lsp_stub.py");
        if Command::new("python3").arg("--version").stdout(Stdio::null()).status().is_err() {
            panic!("this test needs python3 to run the stub server at {stub}");
        }
        // Spawned through python3 rather than by shebang, so it works from a checkout where the
        // executable bit did not survive.
        let dir = std::env::temp_dir().join(format!("cleecode_lsp_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.rs");
        std::fs::write(&file, "fn main() {\n    let dummy = 1;\n    let y = nope;\n}\n").unwrap();

        let mut client = match Client::start_with(&["python3", stub], &dir) {
            Ok(c) => c,
            Err(e) => panic!("the stub server would not start: {e}"),
        };
        let ready = wait_for(&client, |e| matches!(e, Event::Ready { .. }));
        let Some(Event::Ready { utf16 }) = ready else { panic!("no handshake reply") };
        assert!(!utf16, "the stub asks to be counted in UTF-8");
        client.confirm_ready(utf16);

        client.did_open(&file, "fn main() {\n    let dummy = 1;\n    let y = nope;\n}\n");
        let got = wait_for(&client, |e| matches!(e, Event::Diagnostics { .. }));
        let Some(Event::Diagnostics { path, raw }) = got else { panic!("no diagnostics arrived") };
        assert_eq!(path, file, "the URI made the round trip and named the file back");
        assert_eq!(raw.len(), 2);

        let lines: Vec<String> = std::fs::read_to_string(&file)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        let marks = marks_from(&raw, &lines, utf16);
        assert_eq!(marks[0].severity, Severity::Warning);
        assert_eq!((marks[0].line, marks[0].start, marks[0].end), (1, 8, 13));
        assert_eq!(marks[1].severity, Severity::Error);

        client.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    fn wait_for(client: &Client, want: impl Fn(&Event) -> bool) -> Option<Event> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Some(event) = client.try_recv() {
                if want(&event) {
                    return Some(event);
                }
                if matches!(event, Event::Stopped { .. }) {
                    return None;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        None
    }

    #[test]
    fn missing_server_is_an_answer_not_a_crash() {
        let started = Client::start_with(&["cleecode-no-such-language-server"], Path::new("."));
        let Err(err) = started else { panic!("a server that does not exist must not start") };
        assert!(err.contains("cleecode-no-such-language-server"), "{err}");
    }
}
