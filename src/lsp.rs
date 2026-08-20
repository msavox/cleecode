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
//! Only diagnostics. They are non-modal and cannot corrupt a buffer: if the server dies, some
//! underlines go away. Everything that touches the text while you type comes later, on a channel
//! that has been running for a release.

use lsp_types::{
    ClientCapabilities, Diagnostic, DiagnosticSeverity, InitializeParams, PublishDiagnosticsParams,
    TextDocumentClientCapabilities, Uri, WindowClientCapabilities,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::str::FromStr;
use std::sync::mpsc::{self, Receiver, Sender};

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
                text.char_indices().position(|(i, _)| i >= col as usize).unwrap_or(text.chars().count())
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

/// Which server to run for a file, or `None` for a language we have nothing to offer.
///
/// One entry, on purpose. A second server is a second set of startup quirks, and the first one
/// has to be proven before it is worth having two of them to debug at once.
pub fn server_for(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()? {
        "rs" => Some("rust-analyzer"),
        _ => None,
    }
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
}

impl Client {
    /// Starts a server for `root`, or explains why not.
    ///
    /// A missing server is an ordinary outcome, not an error to recover from: most machines do
    /// not have rust-analyzer, and CleeCode has to be exactly as useful there as it was before
    /// this file existed.
    pub fn start(program: &str, root: &Path) -> Result<Client, String> {
        Client::start_with(&[program], root)
    }

    /// The same, for a server that takes arguments. Separate rather than a `&[&str]` everywhere,
    /// because every caller in the program has exactly one word to give.
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
        std::thread::spawn(move || read_loop(BufReader::new(stdout), tx, name));

        let mut client = Client {
            name: program.to_string(),
            child,
            stdin,
            rx,
            next_id: 1,
            open: Vec::new(),
            utf16: true,
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
                text_document: Some(TextDocumentClientCapabilities::default()),
                window: Some(WindowClientCapabilities::default()),
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
    }

    pub fn utf16(&self) -> bool {
        self.utf16
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

    fn next_version(&mut self) -> i64 {
        self.next_id += 1;
        self.next_id
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

fn language_id(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        _ => "plaintext",
    }
}

/// The reader thread. Everything the server says arrives here and leaves as an [`Event`].
fn read_loop(mut reader: BufReader<impl Read>, tx: Sender<Event>, name: String) {
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
            // Anything else the server says on its own account — progress notes, log messages,
            // and requests we never advertised a capability for. Dropped: this release draws
            // squiggles, and a client that answered questions it had not been asked would be
            // agreeing to work it does not do.
            Some(_) => {}
            None => {
                // The first response is the answer to `initialize`, the only request sent before
                // this point. It carries how the server wants positions counted.
                if !handshook && value.get("id").is_some() && value.get("result").is_some() {
                    handshook = true;
                    let utf16 = value
                        .pointer("/result/capabilities/positionEncoding")
                        .and_then(Value::as_str)
                        .unwrap_or("utf-16")
                        != "utf-8";
                    let _ = tx.send(Event::Ready { utf16 });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{Position, Range};

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
        read_loop(BufReader::new(&wire[..]), tx, "stub".to_string());
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
        read_loop(BufReader::new(&wire[..]), tx, "stub".to_string());
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
        read_loop(BufReader::new(&wire[..]), tx, "stub".to_string());
        let events: Vec<Event> = rx.into_iter().collect();
        assert!(matches!(events[0], Event::Diagnostics { .. }), "the good message still arrives");
    }

    #[test]
    fn a_server_is_offered_only_for_a_language_we_have_one_for() {
        assert_eq!(server_for(Path::new("main.rs")), Some("rust-analyzer"));
        assert_eq!(server_for(Path::new("notes.md")), None);
        assert_eq!(server_for(Path::new("Makefile")), None);
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
        let started = Client::start("cleecode-no-such-language-server", Path::new("."));
        let Err(err) = started else { panic!("a server that does not exist must not start") };
        assert!(err.contains("cleecode-no-such-language-server"), "{err}");
    }
}
