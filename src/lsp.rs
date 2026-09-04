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
//!
//! Rename and formatting are the two requests whose answers are sets of edits, and neither is
//! something this module applies: they are parsed into a [`RenamePlan`] and a list of
//! [`SpanEdit`]s — neutral descriptions of what the server wants changed — and handed over. What
//! the application does with a rename is shown to the user before any of it reaches a buffer,
//! and refused whole where it cannot be shown honestly. A format is shown to nobody first, and
//! the difference is the scope: a rename reaches files nobody is looking at, a format rewrites
//! the one buffer on screen and lands as a single edit that one Ctrl+Z takes back.
//!
//! Code actions are the third, and they invent nothing: what a server offers to do about the
//! diagnostic under the cursor arrives as a [`CodeAction`] whose edit is a [`RenamePlan`], read by
//! the same [`rename_plan`] that reads a rename's, and the application routes it down the roads
//! those two already built. The only new thing on the wire is that a server is entitled to name an
//! action without saying yet what it would change — see [`Client::resolve_code_action`].
//!
//! `selectionRange` and `foldingRange` are the two that ask about neither facts nor edits but about
//! the *shape* of the file: what encloses the caret, and where each block begins and ends. They are
//! the answer to a question the roadmap asked for two releases — structural selection and semantic
//! folding without tree-sitter — and they are that answer because the server has already parsed the
//! file. Nothing here keeps a second model of the text: a chain of enclosing ranges is walked once
//! and thrown away, and a file's fold boundaries are a list of line numbers that stops being true
//! the moment somebody types.

use lsp_types::{
    ClientCapabilities, CodeActionCapabilityResolveSupport, CodeActionClientCapabilities,
    CompletionClientCapabilities, CompletionItem, CompletionItemCapability, Diagnostic,
    DiagnosticSeverity, GeneralClientCapabilities, InitializeParams, InsertTextFormat,
    PositionEncodingKind, PublishDiagnosticsParams, RenameClientCapabilities,
    TextDocumentClientCapabilities, Uri, WindowClientCapabilities,
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
    ///
    /// `offered` is the handful of capabilities read off that reply rather than assumed, and they
    /// are read because the features they gate have to be *offered* or not: everything else here
    /// degrades to an empty answer, while a menu row that asks a server for something it never
    /// claimed to do would spend a round trip to say so. See [`Offered`].
    Ready { utf16: bool, offered: Offered },
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
    /// Everywhere the thing under the cursor is used, in the order the server listed them.
    ///
    /// A list and not a first entry, which is the whole difference from [`Self::Definition`]:
    /// a definition is a place to go and the uses are a thing to read, so they arrive whole and
    /// are chosen from. An empty list is delivered rather than dropped, for the same reason the
    /// absent definition is: "nothing uses that" is an answer.
    References { id: i64, targets: Vec<Jump> },
    /// What the file contains, as the server sees it: names, what kind of thing each is, and
    /// how deeply nested. Flat or nested on the wire and always flat by the time it arrives —
    /// see [`symbol_rows`].
    Symbols { id: i64, symbols: Vec<SymbolRow> },
    /// What the thing under the cursor is, in one line. `None` when the server had nothing.
    Hover { id: i64, text: Option<String> },
    /// What the server wants changed to carry out one rename, or what it said instead.
    ///
    /// The only answer in this list that carries an error rather than dropping it. Everywhere
    /// else a refusal reads as an empty result and says so — "the server knows of no definition
    /// for that" is true either way. A rename is a question the user asked in words and waited
    /// for, and "you cannot rename that" is the answer: reporting it as "no changes" would be
    /// the editor putting its own words in the server's mouth.
    Rename { id: i64, plan: Result<RenamePlan, String> },
    /// How the server would lay one file out, or what it said instead.
    ///
    /// A bare list of spans rather than a [`RenamePlan`], because that is what the answer is:
    /// `textDocument/formatting` is asked about one document and answers about that document, so
    /// there is no second URI to read and no resource operation to refuse. The spans are in the
    /// server's units, like every other position that arrives here.
    ///
    /// Carries an error for the same reason [`Self::Rename`] does. A format is asked for on
    /// purpose and waited for, and an empty list already means something else here — "the file
    /// is already laid out the way I would lay it out" — so reporting a refusal as one would be
    /// the editor telling the user the opposite of what the server said.
    Formatting { id: i64, edits: Result<Vec<SpanEdit>, String> },
    /// What the server offers to do about the range the question named, or what it said instead.
    ///
    /// A list to choose from rather than something to carry out: an action is a title and a plan,
    /// and nothing here decides which of them anybody wants. An empty list is delivered rather
    /// than dropped, as everywhere else in this file — "there is nothing I can do here" is an
    /// answer, and the commonest one in the middle of a line that is not wrong.
    ///
    /// Carries an error for the reason [`Self::Rename`] and [`Self::Formatting`] do: it was asked
    /// for on purpose and waited for, and an empty list already means something else.
    CodeActions { id: i64, actions: Result<Vec<CodeAction>, String> },
    /// What one action the server had only named would actually change.
    ///
    /// `None` is a server that answered the resolve without filling the edit in — which is a thing
    /// that happens, and is not an error: it is an action that turns out to have nothing to say.
    CodeActionEdit { id: i64, plan: Result<Option<RenamePlan>, String> },
    /// The chain of ever-wider ranges around one position, innermost first.
    ///
    /// A chain rather than a range, which is the whole shape of this feature: the server is asked
    /// once and answers with the identifier, the expression it sits in, the statement that holds
    /// that, and so on outwards — so every later press of the key is walked in the editor without
    /// another round trip. In the server's own units, like every other position that arrives here.
    ///
    /// Carries an error for the reason [`Self::Rename`] does: it was asked for by a keypress and
    /// waited for, and an empty chain already means something else — "there is nothing around the
    /// caret I can name".
    SelectionRange { id: i64, chain: Result<Vec<Span>, String> },
    /// Where the server says this file's foldable blocks begin and end, as line numbers.
    ///
    /// The one answer in this list with no columns in it at all, and so the one that needs no unit
    /// conversion — see [`folding_ranges`]. Not carried as an error: nothing asked for this, it is
    /// asked on the editor's own account when a file is opened and when it is saved, and a server
    /// that refuses simply leaves the editor folding by braces as it did before any of this.
    FoldingRanges { id: i64, ranges: Vec<(usize, usize)> },
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
    /// The one word for it, in the spelling the protocol uses for these four.
    ///
    /// Not translated, and that is the point: it is the same word on the wire to an agent and in
    /// the column beside a diagnostic, and a reader who has seen one recognises the other.
    pub fn word(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
            Severity::Hint => "hint",
        }
    }

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
    References,
    Symbols,
    Hover,
    Rename,
    Formatting,
    /// `resolves` travels with the question because it decides how the *answer* is read: an action
    /// with no edit in it is one to ask again about when the server can be asked again, and one to
    /// drop when it cannot. The reader thread never sees a handshake reply, so what the server
    /// said it could do is written down here, at the moment the question goes out.
    CodeActions { resolves: bool },
    CodeActionResolve,
    SelectionRange,
    FoldingRanges,
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

/// One place, in any of the shapes a server may name one in.
///
/// `Location` or `LocationLink` — which names the same two fields `targetUri` and
/// `targetSelectionRange`. Servers pick whichever they like and are all correct, so this reads
/// whichever arrived rather than declaring a preference and losing the others.
fn one_location(value: &Value) -> Option<Jump> {
    let uri = value.get("uri").or_else(|| value.get("targetUri"))?.as_str()?;
    let range = value
        .get("range")
        .or_else(|| value.get("targetSelectionRange"))
        .or_else(|| value.get("targetRange"))?;
    let line = range.pointer("/start/line")?.as_u64()? as usize;
    let column = range.pointer("/start/character")?.as_u64()? as usize;
    let uri: Uri = uri.parse().ok()?;
    Some(Jump { path: path_for(&uri)?, line, column })
}

/// The one place a definition answer is read, in any of the three shapes a server may send it:
/// one location, an array of them, or a link. See [`one_location`] for the last two spellings.
///
/// The first of an array, deliberately. More than one definition is real — a trait method with
/// several implementations — and a picker for them is a feature of its own; going to the first
/// is what every editor does before it grows one, and it is right far more often than not.
pub fn first_location(result: Option<&Value>) -> Option<Jump> {
    let value = result?;
    let one = if value.is_array() { value.get(0)? } else { value };
    one_location(one)
}

/// Every place an answer names, for the questions whose answer is a list rather than a
/// destination.
///
/// The same three shapes as [`first_location`], because a server that sends a bare `Location`
/// for a definition sends one for a single reference too — and an answer read as nothing is
/// indistinguishable, on screen, from a name nothing uses. Entries that cannot be read are
/// dropped one by one rather than costing the whole list: a `references` answer with one
/// unparseable URI in it is still an answer about everywhere else.
///
/// Columns stay in the server's units here, as [`Jump`] says they do.
pub fn all_locations(result: Option<&Value>) -> Vec<Jump> {
    let Some(value) = result else { return Vec::new() };
    match value.as_array() {
        Some(items) => items.iter().filter_map(one_location).collect(),
        None => one_location(value).into_iter().collect(),
    }
}

/// One name a file contains, as a row of the outline.
///
/// `depth` is what makes this a tree without being one: the rows arrive in document order and
/// each carries how far in it sits, which is everything a list of lines needs to be indented
/// correctly and nothing more. A tree of children would have to be flattened to be drawn, and
/// the flattening is the only part anything here uses.
///
/// `column` is in the server's units, for the same reason [`Jump`]'s is.
#[derive(Clone, Debug, PartialEq)]
pub struct SymbolRow {
    pub name: String,
    /// What kind of thing it is, in one lowercase word — see [`symbol_kind`].
    pub kind: &'static str,
    pub depth: usize,
    /// Zero-based line, as the protocol counts them.
    pub line: usize,
    pub column: usize,
}

/// A `SymbolKind` number as a word short enough to sit in a column beside the name.
///
/// Total and deliberately dull: every number the protocol defines has a word, and anything else
/// — a number from a version of the specification this predates, or a server counting wrong —
/// is `sym` rather than a row that goes missing. The words are the ones the languages use for
/// themselves where they differ from the protocol's: an interface is a `trait` here because the
/// servers that send that number for Rust mean one.
fn symbol_kind(number: Option<u64>) -> &'static str {
    match number {
        Some(1) => "file",
        Some(2) => "mod",
        Some(3) => "ns",
        Some(4) => "pkg",
        Some(5) => "class",
        Some(6) => "method",
        Some(7) => "prop",
        Some(8) => "field",
        Some(9) => "ctor",
        Some(10) => "enum",
        Some(11) => "trait",
        Some(12) => "fn",
        Some(13) => "var",
        Some(14) => "const",
        Some(15) => "str",
        Some(16) => "num",
        Some(17) => "bool",
        Some(18) => "array",
        Some(19) => "object",
        Some(20) => "key",
        Some(21) => "null",
        Some(22) => "variant",
        Some(23) => "struct",
        Some(24) => "event",
        Some(25) => "operator",
        Some(26) => "type",
        _ => "sym",
    }
}

/// The one place a `documentSymbol` answer is read, in both of the shapes it comes in.
///
/// The protocol has two and a client may be sent either. `SymbolInformation[]` is flat and puts
/// the position under `location.range`; `DocumentSymbol[]` nests, and names the position twice —
/// `range` is the whole item, braces and body included, and `selectionRange` is just the name.
/// The name is what a jump should land on, so that is preferred where both are there.
///
/// CleeCode does not ask for the nested shape in the handshake, and reads it anyway. The three
/// spellings of a definition answer taught that lesson: what a server sends is decided by the
/// server, and a client that only reads what it asked for shows an empty list on the day one
/// of them sends the other thing.
pub fn symbol_rows(result: Option<&Value>) -> Vec<SymbolRow> {
    let mut rows = Vec::new();
    if let Some(items) = result.and_then(Value::as_array) {
        gather_symbols(items, 0, &mut rows);
    }
    rows
}

/// Walks one level of the answer, depth first, so the rows come out in document order.
fn gather_symbols(items: &[Value], depth: usize, rows: &mut Vec<SymbolRow>) {
    for item in items {
        let start = item
            .pointer("/selectionRange/start")
            .or_else(|| item.pointer("/location/range/start"))
            .or_else(|| item.pointer("/range/start"));
        // A row with no name or no position is not a row: there would be nothing to read and
        // nowhere to go. Its children are still walked, because one unreadable container should
        // not take everything inside it off the list with it.
        if let (Some(name), Some(start)) = (item.get("name").and_then(Value::as_str), start) {
            rows.push(SymbolRow {
                name: name.to_string(),
                kind: symbol_kind(item.get("kind").and_then(Value::as_u64)),
                depth,
                line: start.get("line").and_then(Value::as_u64).unwrap_or(0) as usize,
                column: start.get("character").and_then(Value::as_u64).unwrap_or(0) as usize,
            });
        }
        if let Some(children) = item.get("children").and_then(Value::as_array) {
            gather_symbols(children, depth + 1, rows);
        }
    }
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

// ---- What a rename would change --------------------------------------------------------------

/// Everything one `WorkspaceEdit` asks for, in a shape that owes nothing to the wire.
///
/// Deliberately not `lsp_types::WorkspaceEdit`. That type is an accurate model of the protocol,
/// which is the problem: it makes the caller walk two optional collections, three enum variants
/// of resource operation and an annotated spelling of a text edit before it can answer the only
/// two questions that matter here — *which files, which spans* and *is there anything in this we
/// cannot do*.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenamePlan {
    /// One entry per file the server named, in the order they were read off the answer.
    pub files: Vec<FileEdits>,
    /// Whether the answer also asked for a file to be created, renamed or deleted.
    ///
    /// A flag rather than a list of them, because the only thing the application does with it is
    /// refuse: creating and deleting files is not what "rename this name" means, and a client
    /// that quietly ignored those operations would apply half of what the server asked for and
    /// report it as the whole thing.
    pub file_ops: bool,
}

/// The edits one file is to receive.
#[derive(Clone, Debug, PartialEq)]
pub struct FileEdits {
    pub path: PathBuf,
    pub edits: Vec<SpanEdit>,
}

/// One replacement, in the server's own units — lines as it counts them, columns as it counts
/// those, for the same reason [`Jump`]'s column stays that way: turning a column into a character
/// offset needs the file's text, and this is parsed on a thread that has never read it.
#[derive(Clone, Debug, PartialEq)]
pub struct SpanEdit {
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub new_text: String,
}

impl SpanEdit {
    /// Whether the span runs off the end of the line it starts on.
    ///
    /// Asked because the answer is a refusal. The range machinery here clamps a multi-line span
    /// to its first line — see [`marks_from`], where that is exactly right for an underline — and
    /// a replacement clamped the same way would delete a different amount of text from the one
    /// the preview showed. A rename that spans lines is not something a rename should produce; if
    /// a server sends one it is describing something else, and refusing is the honest reading.
    pub fn spans_lines(&self) -> bool {
        self.start_line != self.end_line
    }
}

/// One `TextEdit` off the wire, in whichever of its two spellings arrived.
///
/// `AnnotatedTextEdit` is a `TextEdit` with an `annotationId` beside it, so reading the three
/// fields by name accepts both without having to know which is which.
fn one_edit(value: &Value) -> Option<SpanEdit> {
    let range = value.get("range")?;
    let at = |which: &str, field: &str| {
        range.pointer(&format!("/{which}/{field}")).and_then(Value::as_u64).map(|n| n as usize)
    };
    Some(SpanEdit {
        start_line: at("start", "line")?,
        start_col: at("start", "character")?,
        end_line: at("end", "line")?,
        end_col: at("end", "character")?,
        // An edit with no text is a deletion, which is a legal thing to be sent and reads as an
        // empty string everywhere below.
        new_text: value.get("newText").and_then(Value::as_str).unwrap_or_default().to_string(),
    })
}

/// The edits under one URI, or `None` when the URI is not one we can name a file with.
fn edits_for(uri: &str, edits: Option<&Value>) -> Option<FileEdits> {
    let uri: Uri = uri.parse().ok()?;
    let path = path_for(&uri)?;
    let edits: Vec<SpanEdit> =
        edits?.as_array()?.iter().filter_map(one_edit).collect();
    Some(FileEdits { path, edits })
}

/// The one place a `WorkspaceEdit` is read, in both of the shapes it comes in.
///
/// The protocol has two and a client may be sent either: `changes` is a flat map of URI to edits,
/// and `documentChanges` is an ordered list whose entries are either a `TextDocumentEdit` or a
/// resource operation. Both are read here whatever the handshake said — the same lesson the three
/// spellings of a definition answer taught: what a server sends is decided by the server, and a
/// client that only reads what it asked for shows nothing on the day one of them sends the other
/// thing. CleeCode advertises no `workspace.workspaceEdit` capability at all, which the
/// specification says means the flat `changes` shape; several servers send the other one anyway.
///
/// An entry that cannot be read is dropped rather than costing the whole answer, with one
/// exception: a resource operation sets [`RenamePlan::file_ops`], because *that* is a thing the
/// caller has to know was asked for in order to refuse it.
pub fn rename_plan(result: Option<&Value>) -> RenamePlan {
    let mut plan = RenamePlan::default();
    let Some(result) = result else { return plan };
    if let Some(changes) = result.get("changes").and_then(Value::as_object) {
        for (uri, edits) in changes {
            if let Some(file) = edits_for(uri, Some(edits)) {
                plan.files.push(file);
            }
        }
    }
    if let Some(items) = result.get("documentChanges").and_then(Value::as_array) {
        for item in items {
            // A resource operation is told apart by its `kind`, which a `TextDocumentEdit` does
            // not have. Read first, so a create/rename/delete cannot be mistaken for an edit with
            // an unreadable document.
            if item.get("kind").and_then(Value::as_str).is_some() {
                plan.file_ops = true;
                continue;
            }
            let Some(uri) = item.pointer("/textDocument/uri").and_then(Value::as_str) else {
                continue;
            };
            if let Some(file) = edits_for(uri, item.get("edits")) {
                plan.files.push(file);
            }
        }
    }
    plan
}

/// The edits one `textDocument/formatting` answer asks for, in the server's own units.
///
/// A bare array, and that is the whole difference from [`rename_plan`]: this answer is about the
/// document the question named and nothing else, so there is no URI to read, no second shape to
/// accept and no resource operation to guard against. `null` is a legal answer — it is how a
/// server says it would change nothing — and reads as the empty list, which is what it means.
///
/// The spans a formatter sends routinely cross lines: replacing the whole file with a laid-out
/// copy is one edit from `0:0` to past the last line, and that is the ordinary case rather than
/// the odd one. So nothing here refuses a multi-line span the way [`SpanEdit::spans_lines`] has
/// a rename refuse one — the two are asking different questions of the same struct, and the
/// application converts these against the buffer at both ends instead of clamping to a line.
///
/// An entry that cannot be read is dropped rather than costing the whole answer, as everywhere
/// else here: a stray member in one range should not be the reason a file cannot be laid out.
/// What it costs is a run of text carried over unformatted, which the reader can see.
pub fn format_edits(result: Option<&Value>) -> Vec<SpanEdit> {
    result
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(one_edit).collect())
        .unwrap_or_default()
}

// ---- What the server offers to do about it ---------------------------------------------------
//
// The third question whose answer is a set of edits, and the only one that is a *list* of them:
// a rename and a format each come back as the one thing the server would do, and a code action
// answer comes back as several things it could, of which somebody picks one. Everything below
// reads that list; what one of them would change is a `WorkspaceEdit` like any other, and is read
// by `rename_plan` rather than by a second parser that would have to be kept in step with it.

/// What the server said it can do about the code under a cursor, read off its handshake reply.
///
/// Two flags rather than one, because they are two different promises. `offered` is whether
/// `textDocument/codeAction` is answered at all — a server that never claimed it is not asked, so
/// the menu row says so instantly instead of spending a round trip on a refusal. `resolves` is
/// whether an action named without its edit can be filled in later, which is how rust-analyzer
/// sends most of its assists: the titles arrive at once and the edits are computed for the one
/// that gets picked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActionSupport {
    pub offered: bool,
    pub resolves: bool,
}

/// The two flags above, out of the `capabilities` object of a handshake reply.
///
/// `codeActionProvider` is a boolean or an options object in the specification, and both are read:
/// a server that answers `true` is offering the request and nothing more, and one that answers an
/// object may also be saying it can resolve. Anything else — an absent member, a shape from a
/// later version of the protocol — is read as "not offered", which is the reading that costs a
/// menu row rather than a request nobody can answer.
pub fn action_support(capabilities: Option<&Value>) -> ActionSupport {
    match capabilities.and_then(|c| c.get("codeActionProvider")) {
        Some(Value::Bool(offered)) => ActionSupport { offered: *offered, resolves: false },
        Some(Value::Object(options)) => ActionSupport {
            offered: true,
            resolves: options.get("resolveProvider").and_then(Value::as_bool).unwrap_or(false),
        },
        _ => ActionSupport::default(),
    }
}

/// One thing a server offers to do about a range of a file.
///
/// `edit` is the whole of what it would change, in the same [`RenamePlan`] a rename comes back as
/// — which is the point: a `WorkspaceEdit` is a `WorkspaceEdit` whichever question produced it,
/// and reading it a second way here would be a second chance to read it wrong.
#[derive(Clone, Debug, PartialEq)]
pub struct CodeAction {
    /// What the server calls it: "Import `HashMap`", "Convert to guarded return".
    pub title: String,
    /// `quickfix`, `refactor.extract`, `source.organizeImports` — or empty where the server did
    /// not say, which it is entitled not to.
    pub kind: String,
    /// What it would change, when the answer carried it. `None` is an action the server has so far
    /// only named; see [`Client::resolve_code_action`].
    pub edit: Option<RenamePlan>,
    /// The item exactly as it arrived, because that is what `codeAction/resolve` takes back: the
    /// action's `data` is the server's own bookkeeping and means nothing to anybody here, so it is
    /// carried whole rather than picked apart and reassembled.
    pub raw: Value,
}

/// Every action one `textDocument/codeAction` answer offers, in the order the server listed them.
///
/// `resolves` is what the server said about `codeAction/resolve` in its handshake — see
/// [`ActionSupport`] — and it decides the fate of an action that arrived without an edit.
///
/// Three things are dropped here, and each of them honestly:
///
/// * a bare `Command`, which the protocol allows in this list beside the actions proper. Carrying
///   one out means `workspace/executeCommand` and then obeying the workspace edits the server
///   pushes back on its own initiative — which is the one thing [`reply_to`] refuses, for reasons
///   written there. That is a different release, not a line of code;
/// * an action that carries a `command` *as well as* an edit, for the same reason: applying the
///   edit and dropping the command would do half of what the server asked and report it as all of
///   it, which is exactly what [`RenamePlan::file_ops`] exists to stop;
/// * an action with no edit at all on a server that cannot resolve one, because there would be
///   nothing to do if it were picked.
///
/// A `disabled` action is dropped too. The server has said in so many words that it cannot be
/// applied here, and a row that answers a keypress with the server's excuse is a row that should
/// not have been in the list.
pub fn offered_actions(result: Option<&Value>, resolves: bool) -> Vec<CodeAction> {
    let Some(items) = result.and_then(Value::as_array) else { return Vec::new() };
    let mut out = Vec::new();
    for item in items {
        // A `Command` and a `CodeAction` are told apart by this member: on the first it is the
        // command's name, on the second it is an object hanging off an action that also has a
        // title. Either way this client cannot run it, so either way the row goes.
        if item.get("command").is_some() || item.get("disabled").is_some() {
            continue;
        }
        let Some(title) = item.get("title").and_then(Value::as_str) else { continue };
        let edit = item.get("edit").map(|edit| rename_plan(Some(edit)));
        if edit.is_none() && !resolves {
            continue;
        }
        out.push(CodeAction {
            title: title.to_string(),
            kind: item.get("kind").and_then(Value::as_str).unwrap_or_default().to_string(),
            edit,
            raw: item.clone(),
        });
    }
    out
}

/// The edit a `codeAction/resolve` answer filled in, or `None` when it filled in nothing.
///
/// The answer is the same action back with more of it written out, so only the one member is read.
/// A server that returns the action unchanged has said it has nothing to change, which is an
/// answer and not a failure — and the caller says so rather than applying an empty plan in silence.
pub fn resolved_edit(result: Option<&Value>) -> Option<RenamePlan> {
    result?.get("edit").map(|edit| rename_plan(Some(edit)))
}

/// The diagnostics that touch a range, in the server's own units.
///
/// This is what makes a quick fix a quick fix: a server matches its fixes against the diagnostics
/// the client hands back in `context`, and one asked with an empty context answers with the
/// refactorings that apply anywhere and none of the fixes for the error you are sitting on.
///
/// Touching, not containing, and inclusive at both ends. A caret resting on the last character of
/// a squiggle — or on the empty range a "expected something here" points at — is somebody asking
/// about *that* error, and a half-open comparison would answer about the line instead.
pub fn diagnostics_in_range(
    diagnostics: &[Diagnostic],
    start: (usize, usize),
    end: (usize, usize),
) -> Vec<Diagnostic> {
    diagnostics
        .iter()
        .filter(|d| {
            let from = (d.range.start.line as usize, d.range.start.character as usize);
            let to = (d.range.end.line as usize, d.range.end.character as usize);
            from <= end && to >= start
        })
        .cloned()
        .collect()
}

// ---- The shape of the file, as the server sees it ---------------------------------------------
//
// The two questions of 0.21, and the only two here whose answers are neither facts to read nor
// edits to apply: they are the *structure* of the text. `selectionRange` says what encloses a
// position — the identifier, then the expression, then the statement — and `foldingRange` says
// where each block of the file begins and ends. Both are things a syntax tree would answer, and
// asking the server for them is how this editor has one without keeping a second model of the text
// in step with the rope. See the roadmap entry for why that trade decided against tree-sitter.

/// One range of a file, in the server's own units.
///
/// The columns stay that way for the reason [`Jump`]'s does: turning one into a character offset
/// needs the file's text, and this is parsed on a thread that has never read it. Deliberately not
/// a [`SpanEdit`] with an empty `new_text` — this describes a piece of the file, not a change to
/// it, and a struct with a field that is always empty is a struct that invites somebody to fill it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
}

/// One `Range` off the wire, by the four names the protocol gives its corners.
fn one_span(range: Option<&Value>) -> Option<Span> {
    let range = range?;
    let at = |which: &str, field: &str| {
        range.pointer(&format!("/{which}/{field}")).and_then(Value::as_u64).map(|n| n as usize)
    };
    Some(Span {
        start_line: at("start", "line")?,
        start_col: at("start", "character")?,
        end_line: at("end", "line")?,
        end_col: at("end", "character")?,
    })
}

/// The chain one `textDocument/selectionRange` answer describes, innermost first.
///
/// The answer is an array with one entry *per position asked about*, not per level — this client
/// asks about one — and each entry is a linked list: a range, and a `parent` that encloses it. So
/// the array is indexed once and the links are walked, which is what turns one request into every
/// press of the key: the whole ladder from the identifier to the item that holds it arrives at
/// once, and the editor climbs it on its own afterwards.
///
/// Two things are refused on the way out. A level identical to the one below it is dropped, because
/// on screen it is a keypress that appears to do nothing — servers do send them, an expression and
/// the statement that is only that expression being the ordinary case. And the walk is bounded:
/// `parent` is a link the server writes, nothing on this side can prove it is not a ring, and a
/// hundred levels is deeper than any real syntax tree and free to refuse.
pub fn selection_chain(result: Option<&Value>) -> Vec<Span> {
    let Some(value) = result else { return Vec::new() };
    // The bare object is read as well as the array. What a server sends is the server's decision,
    // which is the lesson the three spellings of a definition answer taught.
    let mut node = if value.is_array() { value.get(0) } else { Some(value) };
    let mut out: Vec<Span> = Vec::new();
    while let Some(current) = node {
        let Some(span) = one_span(current.get("range")) else { break };
        if out.last() != Some(&span) {
            out.push(span);
        }
        if out.len() >= 100 {
            break;
        }
        node = current.get("parent");
    }
    out
}

/// Where the server says each foldable block of a file begins and ends, as line numbers.
///
/// The one answer in this whole file with no column arithmetic anywhere near it, and that is worth
/// saying out loud rather than looking like an omission: `FoldingRange` carries `startCharacter`
/// and `endCharacter` and CleeCode reads neither, because a fold in this editor hides whole lines
/// — see `Editor::is_hidden`. Lines are counted from zero by the protocol and by the editor alike,
/// so the numbers cross over untouched. There is no UTF-16 to undo here because no character
/// column is read.
///
/// A range that ends where it starts is dropped: it would hide nothing, and a fold marker in the
/// gutter that collapses zero lines is a marker that looks broken.
pub fn folding_ranges(result: Option<&Value>) -> Vec<(usize, usize)> {
    let Some(items) = result.and_then(Value::as_array) else { return Vec::new() };
    items
        .iter()
        .filter_map(|item| {
            let start = item.get("startLine").and_then(Value::as_u64)? as usize;
            let end = item.get("endLine").and_then(Value::as_u64)? as usize;
            (end > start).then_some((start, end))
        })
        .collect()
}

/// What a server said it can do, read off its handshake reply rather than assumed.
///
/// All three gate a *surface*: a menu row, a chord, or a question the editor asks on its own
/// account when a file opens. Everything else in this client degrades to an empty answer and needs
/// no capability at all, and that is the rule for which of them are read here — a feature that has
/// to be offered or refused in words has to know, and a feature that can shrug does not ask.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Offered {
    pub actions: ActionSupport,
    /// Whether `textDocument/selectionRange` is answered. Its own sentence when it is not: the
    /// chord is on the keyboard whatever the file is, and "this server does not do that" is the
    /// only honest thing to say to somebody pressing it in a language whose server does not.
    pub selection_ranges: bool,
    /// Whether `textDocument/foldingRange` is answered. Nothing is said when it is not — folding
    /// goes on working off the braces, which is where it started — and the flag exists only so the
    /// question is not asked of every file a server that never answers it serves.
    pub folding_ranges: bool,
}

/// Everything read off the `capabilities` object of a handshake reply, in one place.
pub fn offered_by(capabilities: Option<&Value>) -> Offered {
    Offered {
        actions: action_support(capabilities),
        selection_ranges: provides(capabilities, "selectionRangeProvider"),
        folding_ranges: provides(capabilities, "foldingRangeProvider"),
    }
}

/// Whether one plain capability member says yes.
///
/// The protocol spells most of them three ways — a boolean, an options object, or a registration
/// object with a document selector in it — and all three mean the server answers the request. Only
/// an explicit `false`, an absent member, or a shape from a version of the specification this
/// predates read as no, which is the reading that costs a feature rather than a request nobody can
/// answer.
fn provides(capabilities: Option<&Value>, member: &str) -> bool {
    match capabilities.and_then(|c| c.get(member)) {
        Some(Value::Bool(yes)) => *yes,
        Some(Value::Object(_)) => true,
        _ => false,
    }
}

/// What a server said went wrong, as the one sentence a status line has room for.
///
/// Its own words, not ours. "The server refused" would be true of every possible failure here and
/// useful for none of them — what the reader needs is rust-analyzer's own "cannot rename this",
/// or pyright's own reason, said the way the server said it.
fn complaint(error: &Value) -> String {
    match error.get("message").and_then(Value::as_str) {
        Some(message) => message.to_string(),
        // A server that answered with an error object shaped like nothing in the specification.
        // Printed as it arrived rather than replaced with a sentence of ours, for the same reason.
        None => error.to_string(),
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
    /// Whether the handshake is finished. Until it is, the server is entitled to ignore every
    /// notification and request sent to it, and the ones worth using do.
    ready: bool,
    /// What it said it can do. Settled during the handshake and remembered for the reason
    /// [`Self::utf16`] is: it decides both which questions are worth asking and how the answers are
    /// read.
    offered: Offered,
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
            offered: Offered::default(),
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
                    //
                    // `context_support` is the other half of the same sentence: every request
                    // this client sends carries a `context`, and a server told nothing about it
                    // is entitled to ignore the member — which would cost exactly the feature it
                    // is there for, since a member list after a `.` is what a server answers
                    // differently when it knows the dot is why it was asked.
                    completion: Some(CompletionClientCapabilities {
                        completion_item: Some(CompletionItemCapability {
                            snippet_support: Some(false),
                            ..Default::default()
                        }),
                        context_support: Some(true),
                        ..Default::default()
                    }),
                    // Said out loud for the same reason as the snippet flag above: the default is
                    // the one this depends on. `prepare_support: false` tells the server not to
                    // expect a `textDocument/prepareRename` before the rename itself — this
                    // client asks the question once, and a server holding back its answer until a
                    // preparation request that never comes is a key that does nothing.
                    //
                    // `workspace.workspace_edit` is deliberately left unset. The specification
                    // reads an absent one as a client that can only be sent the flat `changes`
                    // shape, which is the smaller thing to be sent; [`rename_plan`] reads the
                    // other shape too, because what a server sends is the server's decision.
                    rename: Some(RenameClientCapabilities {
                        prepare_support: Some(false),
                        ..Default::default()
                    }),
                    // The two halves of "you may name an action now and tell me what it changes
                    // later". Without them a server is entitled to compute every edit up front —
                    // rust-analyzer's assists are expensive enough that it does not, and answers
                    // with titles and nothing else — so a client that reads unresolved actions
                    // and never said it could would be reading a list it asked to be sent full.
                    //
                    // `data_support` is the other half of the same sentence: the `data` member is
                    // the server's own bookkeeping, and it is what makes a resolve request name
                    // the same action back rather than a title we happened to keep.
                    //
                    // Nothing is said about `codeActionLiteralSupport`, and that is deliberate
                    // too: the specification reads its absence as a client that can only be sent
                    // bare `Command`s. [`offered_actions`] reads the action literals anyway, for
                    // the reason every other shape here is read — what a server sends is the
                    // server's decision — and drops the commands it cannot carry out.
                    code_action: Some(CodeActionClientCapabilities {
                        data_support: Some(true),
                        resolve_support: Some(CodeActionCapabilityResolveSupport {
                            properties: vec!["edit".to_string()],
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
    pub fn confirm_ready(&mut self, utf16: bool, offered: Offered) {
        self.utf16 = utf16;
        self.offered = offered;
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

    /// What this server said it can do about code actions.
    pub fn actions(&self) -> ActionSupport {
        self.offered.actions
    }

    /// What this server said it can do, whole.
    pub fn offered(&self) -> Offered {
        self.offered
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
    ///
    /// `trigger` is the character that asked the question, when a character did — a `.` or a `:`,
    /// chosen by [`crate::complete::trigger_at`]. It is the difference between "somebody pressed
    /// the completion key here" and "somebody typed a dot", and servers answer the two
    /// differently: the second is what makes rust-analyzer list the methods of a type rather than
    /// every name in scope. Both shapes are sent rather than one omitted, because a request that
    /// says `triggerKind: 1` has said something, and a request with no `context` at all has left
    /// the server to guess after this client promised it would not.
    pub fn completion(
        &mut self,
        path: &Path,
        line: usize,
        line_text: &str,
        col: usize,
        trigger: Option<char>,
    ) -> Option<i64> {
        let uri = uri_for(path)?;
        let character = self.column_for(line_text, col);
        // 1 is Invoked and 2 is TriggerCharacter, which are the only two this client can be in;
        // the third, "the list was incomplete and is being asked again", needs a list kept across
        // requests and there is none.
        let context = match trigger {
            Some(c) => json!({"triggerKind": 2, "triggerCharacter": c.to_string()}),
            None => json!({"triggerKind": 1}),
        };
        let id = self
            .request(
                "textDocument/completion",
                json!({
                    "textDocument": {"uri": uri.as_str()},
                    "position": {"line": line, "character": character},
                    "context": context
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
        self.position_request(
            "textDocument/definition",
            Ask::Definition,
            path,
            (line, line_text, col),
            Value::Null,
        )
    }

    /// Asks where the thing under the cursor is used.
    ///
    /// `includeDeclaration` is asked for, so the list holds the definition as well as the uses.
    /// It is the honest answer to "where is this name" — the place it comes from is one of the
    /// places it appears — and a list that silently left out the one row somebody was looking
    /// for would be read as the server not knowing about it.
    pub fn references(&mut self, path: &Path, line: usize, line_text: &str, col: usize) -> Option<i64> {
        self.position_request(
            "textDocument/references",
            Ask::References,
            path,
            (line, line_text, col),
            json!({ "context": { "includeDeclaration": true } }),
        )
    }

    /// Asks what the thing under the cursor is.
    pub fn hover(&mut self, path: &Path, line: usize, line_text: &str, col: usize) -> Option<i64> {
        self.position_request("textDocument/hover", Ask::Hover, path, (line, line_text, col), Value::Null)
    }

    /// Asks what would have to change for the thing under the cursor to be called `new_name`.
    ///
    /// The one request in this client whose answer is a set of edits rather than a fact. It is
    /// still only a question: the answer arrives as [`Event::Rename`] and nothing on this side of
    /// the wire touches a buffer — see [`rename_plan`] for what is made of it, and the
    /// application for what is done with that.
    pub fn rename(
        &mut self,
        path: &Path,
        line: usize,
        line_text: &str,
        col: usize,
        new_name: &str,
    ) -> Option<i64> {
        self.position_request(
            "textDocument/rename",
            Ask::Rename,
            path,
            (line, line_text, col),
            json!({ "newName": new_name }),
        )
    }

    /// The shape those three share: a method name, a file and a place in it.
    ///
    /// `at` is the place, as the editor holds it: the line, that line's text, and the column in
    /// *characters*. The text travels with the column because it is what the column is measured
    /// against — see [`Self::column_for`], which is the one place that arithmetic is written.
    ///
    /// `extra` is merged into the params for the methods that carry more than a position —
    /// `references` and its `context` — and is `Value::Null` for the ones that do not. Merged
    /// here rather than in a second copy of this function, because the conversion below is the
    /// part that must never be written twice: the two spellings of it drifted apart once already.
    fn position_request(
        &mut self,
        method: &str,
        ask: Ask,
        path: &Path,
        at: (usize, &str, usize),
        extra: Value,
    ) -> Option<i64> {
        let (line, line_text, col) = at;
        let uri = uri_for(path)?;
        let character = self.column_for(line_text, col);
        let mut params = json!({
            "textDocument": { "uri": uri.as_str() },
            "position": { "line": line, "character": character },
        });
        if let (Some(target), Some(more)) = (params.as_object_mut(), extra.as_object()) {
            for (key, value) in more {
                target.insert(key.clone(), value.clone());
            }
        }
        let id = self.request(method, params).ok()?;
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(id, ask);
        }
        Some(id)
    }

    /// Asks what names the file holds.
    pub fn document_symbols(&mut self, path: &Path) -> Option<i64> {
        self.document_request("textDocument/documentSymbol", Ask::Symbols, path, Value::Null)
    }

    /// Asks how the whole file should be laid out.
    ///
    /// The second request whose answer is a set of edits, and the first that is about a file
    /// rather than about a place in one — which is why it is here and not beside the rename.
    /// It is still only a question: the answer arrives as [`Event::Formatting`] and nothing on
    /// this side of the wire touches a buffer.
    ///
    /// `tab_size` and `insert_spaces` are the editor's own settings and travel with the request
    /// because `FormattingOptions` requires both. A server told nothing lays the file out to its
    /// own default, which is how a two-space project gets a file back indented with four — and
    /// the user would have no way of telling that from the formatter simply disagreeing.
    pub fn formatting(&mut self, path: &Path, tab_size: usize, insert_spaces: bool) -> Option<i64> {
        self.document_request(
            "textDocument/formatting",
            Ask::Formatting,
            path,
            json!({ "options": { "tabSize": tab_size, "insertSpaces": insert_spaces } }),
        )
    }

    /// Asks what could be done about a range of a file.
    ///
    /// Neither a position request nor a document one, which is why it is written out rather than
    /// pushed through either: the question carries a *range*, and both of its ends are columns
    /// that have to be converted against the line each actually sits on. `start` and `end` are
    /// each a line, that line's text, and the column in characters, exactly as
    /// [`Self::position_request`] takes one of them — and they are the same triple twice when
    /// there is no selection, which is how the protocol spells "here, at the caret".
    ///
    /// `diagnostics` is what CleeCode holds about this file, in the server's own units, and the
    /// ones that touch the range travel with the question. That is what turns this into "fix
    /// this error" rather than "what can be done in this file at all": see
    /// [`diagnostics_in_range`].
    ///
    /// `triggerKind: 1` is Invoked — a person pressed something. The other value is for the
    /// editor asking on its own account as the cursor moves, which nothing here does: this is on
    /// demand, and a request that claimed otherwise would be a claim about a feature that is not
    /// in this release.
    pub fn code_actions(
        &mut self,
        path: &Path,
        start: (usize, &str, usize),
        end: (usize, &str, usize),
        diagnostics: &[Diagnostic],
    ) -> Option<i64> {
        let uri = uri_for(path)?;
        let from = (start.0, self.column_for(start.1, start.2));
        let to = (end.0, self.column_for(end.1, end.2));
        let touching = diagnostics_in_range(diagnostics, from, to);
        let params = json!({
            "textDocument": { "uri": uri.as_str() },
            "range": {
                "start": { "line": from.0, "character": from.1 },
                "end": { "line": to.0, "character": to.1 },
            },
            "context": { "diagnostics": touching, "triggerKind": 1 },
        });
        let id = self.request("textDocument/codeAction", params).ok()?;
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(id, Ask::CodeActions { resolves: self.offered.actions.resolves });
        }
        Some(id)
    }

    /// Asks the server to fill in the edit of an action it has so far only named.
    ///
    /// The action goes back exactly as it arrived, which is the whole protocol here: the server
    /// put its own bookkeeping in `data` and reads it back out, and an action reassembled from
    /// the parts this client happens to care about would be an action it does not recognise.
    ///
    /// Sent when one is *picked*, not when the list is drawn. Resolving every row up front would
    /// be a dozen requests for the eleven nobody chose, and the specification puts this request
    /// exactly where the choice is.
    pub fn resolve_code_action(&mut self, action: &Value) -> Option<i64> {
        let id = self.request("codeAction/resolve", action.clone()).ok()?;
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(id, Ask::CodeActionResolve);
        }
        Some(id)
    }

    /// Asks what encloses one place in the file, and everything that encloses that.
    ///
    /// Written out rather than pushed through [`Self::position_request`] for one reason: the
    /// protocol takes `positions`, plural — a client may ask about several carets at once and get a
    /// chain for each. This one asks about one, because there is one cursor; the member is still an
    /// array of one, because that is what the request is.
    ///
    /// `line_text` and `col` are the editor's own, in characters, and the conversion happens here
    /// at the one place that knows what this server counts in — the same arithmetic every other
    /// question that carries a position does, and for the same reason.
    pub fn selection_range(
        &mut self,
        path: &Path,
        line: usize,
        line_text: &str,
        col: usize,
    ) -> Option<i64> {
        let uri = uri_for(path)?;
        let character = self.column_for(line_text, col);
        let params = json!({
            "textDocument": { "uri": uri.as_str() },
            "positions": [{ "line": line, "character": character }],
        });
        let id = self.request("textDocument/selectionRange", params).ok()?;
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(id, Ask::SelectionRange);
        }
        Some(id)
    }

    /// Asks where this file's blocks begin and end.
    ///
    /// A question about a whole file like the outline and the format, so it goes out the same way.
    /// The only one of the three nobody presses a key for: it is asked when a file is opened and
    /// again when it is saved — the two moments the buffer and the server are looking at the same
    /// text — and the answer is cached until an edit makes its line numbers lies.
    pub fn folding_ranges(&mut self, path: &Path) -> Option<i64> {
        self.document_request("textDocument/foldingRange", Ask::FoldingRanges, path, Value::Null)
    }

    /// The shape a question about a whole file takes: no position, so no column to convert.
    ///
    /// `extra` is merged into the params for the method that carries more than a document —
    /// `formatting` and its `options` — and is `Value::Null` for the one that does not, exactly
    /// as [`Self::position_request`] takes the members that ride along with a position. Merged
    /// here rather than in a second function for the same reason it is merged there.
    fn document_request(
        &mut self,
        method: &str,
        ask: Ask,
        path: &Path,
        extra: Value,
    ) -> Option<i64> {
        let uri = uri_for(path)?;
        let mut params = json!({ "textDocument": { "uri": uri.as_str() } });
        if let (Some(target), Some(more)) = (params.as_object_mut(), extra.as_object()) {
            for (key, value) in more {
                target.insert(key.clone(), value.clone());
            }
        }
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
///
/// `workspace/applyEdit` falls into the refusal below, and deliberately. It is the server asking
/// to write into the project on its own initiative, at a moment nobody chose, with no preview and
/// nothing to undo it as one step — the opposite of every rule the rename obeys. No capability
/// for it is advertised, so a server sending one is asking anyway, and -32601 is the honest
/// answer: this client does not do that.
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
                        Ask::References => {
                            Event::References { id, targets: all_locations(result) }
                        }
                        Ask::Symbols => Event::Symbols { id, symbols: symbol_rows(result) },
                        Ask::Hover => Event::Hover { id, text: hover_text(result) },
                        // The two places an `error` member is read rather than passed over. For
                        // every other question above, a response carrying an error instead of a
                        // result has no `result` to read and becomes the same empty answer as a
                        // server that simply knew nothing — which is the truth as far as the
                        // screen is concerned. These two were asked for by a keypress and waited
                        // for; see [`Event::Rename`] and [`Event::Formatting`].
                        Ask::Rename => Event::Rename {
                            id,
                            plan: match value.get("error") {
                                Some(error) => Err(complaint(error)),
                                None => Ok(rename_plan(result)),
                            },
                        },
                        Ask::Formatting => Event::Formatting {
                            id,
                            edits: match value.get("error") {
                                Some(error) => Err(complaint(error)),
                                None => Ok(format_edits(result)),
                            },
                        },
                        // And the third and fourth, for the same reason again: both were asked
                        // for by a keypress, and both have an empty answer that already means
                        // something — "there is nothing to do here", "this action changes
                        // nothing" — so a refusal read as one would say the wrong thing.
                        Ask::CodeActions { resolves } => Event::CodeActions {
                            id,
                            actions: match value.get("error") {
                                Some(error) => Err(complaint(error)),
                                None => Ok(offered_actions(result, resolves)),
                            },
                        },
                        Ask::CodeActionResolve => Event::CodeActionEdit {
                            id,
                            plan: match value.get("error") {
                                Some(error) => Err(complaint(error)),
                                None => Ok(resolved_edit(result)),
                            },
                        },
                        // The fifth, and the last of the ones a keypress waits for. An empty chain
                        // means "there is nothing around the caret I can name", which is a real
                        // answer on a blank line — so a refusal read as one would say the wrong
                        // thing about the file instead of what the server said about the request.
                        Ask::SelectionRange => Event::SelectionRange {
                            id,
                            chain: match value.get("error") {
                                Some(error) => Err(complaint(error)),
                                None => Ok(selection_chain(result)),
                            },
                        },
                        // And the one nobody waited for, which is why it is back to the plain
                        // reading: an error and an empty answer mean the same thing here — no
                        // boundaries from the server — and the editor folds by braces either way.
                        Ask::FoldingRanges => {
                            Event::FoldingRanges { id, ranges: folding_ranges(result) }
                        }
                    });
                    continue;
                }
                // The first response is the answer to `initialize`, the only request sent before
                // this point. It carries how the server wants positions counted.
                if !handshook && value.get("id").is_some() && value.get("result").is_some() {
                    handshook = true;
                    let capabilities = value.pointer("/result/capabilities");
                    let encoding = capabilities
                        .and_then(|c| c.get("positionEncoding"))
                        .and_then(Value::as_str);
                    let _ = tx.send(Event::Ready {
                        utf16: negotiated_utf16(encoding),
                        offered: offered_by(capabilities),
                    });
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

    /// The ladder is read whole, in the order it has to be walked, and the levels that would be a
    /// keypress doing nothing are not in it.
    #[test]
    fn a_selection_range_answer_is_read_as_a_ladder_from_the_inside_out() {
        // `foo` inside `foo.bar()` inside the statement, as a server sends it: an array of one
        // entry per position asked about, and each entry a chain of `parent` links outwards.
        let answer = json!([{
            "range": {"start": {"line": 3, "character": 8}, "end": {"line": 3, "character": 11}},
            "parent": {
                "range": {"start": {"line": 3, "character": 8}, "end": {"line": 3, "character": 17}},
                // The statement is exactly the expression, which servers do send — and which would
                // be a press of the key that appeared to do nothing, so it is dropped.
                "parent": {
                    "range": {"start": {"line": 3, "character": 8}, "end": {"line": 3, "character": 17}},
                    "parent": {
                        "range": {"start": {"line": 2, "character": 0}, "end": {"line": 5, "character": 1}}
                    }
                }
            }
        }]);
        let chain = selection_chain(Some(&answer));
        assert_eq!(
            chain,
            vec![
                Span { start_line: 3, start_col: 8, end_line: 3, end_col: 11 },
                Span { start_line: 3, start_col: 8, end_line: 3, end_col: 17 },
                Span { start_line: 2, start_col: 0, end_line: 5, end_col: 1 },
            ]
        );
        // The bare object is read too, for the reason every other shape here is.
        let bare = json!({
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 4}}
        });
        assert_eq!(selection_chain(Some(&bare)).len(), 1);
        // Nothing, in each of the ways a server says it.
        assert!(selection_chain(Some(&json!([]))).is_empty());
        assert!(selection_chain(Some(&Value::Null)).is_empty());
        assert!(selection_chain(None).is_empty());
        // A ring is refused rather than followed. `parent` is a link the server writes and this
        // side cannot prove it ends; a hundred levels is deeper than any real syntax tree.
        let mut ring = json!({
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}
        });
        for n in 1..300u64 {
            ring = json!({
                "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": n + 1}},
                "parent": ring
            });
        }
        assert_eq!(selection_chain(Some(&ring)).len(), 100);
    }

    /// The ladder arrives in the server's units and is the editor's business to convert — both
    /// ends, each against the line it is actually on. Checked here on the two conversions this
    /// file owns, because a chain whose ends were converted against the wrong line selects the
    /// right number of characters in the wrong place.
    #[test]
    fn both_ends_of_a_chain_convert_in_whichever_unit_the_server_counts_in() {
        let answer = json!([{
            "range": {"start": {"line": 0, "character": 4}, "end": {"line": 1, "character": 6}}
        }]);
        let span = selection_chain(Some(&answer))[0];
        // `let città = 1;` and `    x = "è";` — each end measured against its own line.
        let first = "let città = 1;";
        let second = "    x = \"è\";";
        assert_eq!(utf16_to_chars(first, span.start_col), 4);
        assert_eq!(utf16_to_chars(second, span.end_col), 6, "UTF-16 and characters agree up to the accent");
        // The same numbers from a server counting bytes are two different places.
        assert_eq!(utf8_to_chars(first, span.start_col), 4);
        assert_eq!(utf8_to_chars(second, span.end_col), 6);
        // And past the accent the two units part company, which is the whole reason both exist:
        // the `è` is one character and two bytes, so the closing quote is column 10 to the editor
        // and column 11 to a server counting bytes.
        assert_eq!(utf8_to_chars(second, 11), 10);
        assert_eq!(utf16_to_chars(second, 11), 11);
    }

    /// The one answer with no columns in it: line numbers straight across, in the same
    /// `(start, end)` pair the editor's own folds are held as.
    #[test]
    fn a_folding_answer_becomes_the_pairs_the_editor_already_folds_by() {
        let answer = json!([
            {"startLine": 0, "endLine": 4, "kind": "region"},
            {"startLine": 1, "endLine": 3, "startCharacter": 12, "endCharacter": 1},
            // Ends where it starts: it would hide nothing, and a marker that collapses no lines
            // looks broken.
            {"startLine": 7, "endLine": 7},
            // Missing half a range is one range lost, not the answer.
            {"startLine": 9}
        ]);
        assert_eq!(folding_ranges(Some(&answer)), vec![(0, 4), (1, 3)]);
        assert!(folding_ranges(Some(&Value::Null)).is_empty());
        assert!(folding_ranges(None).is_empty());
    }

    /// What a server said it can do, in each of the shapes it is entitled to say it in.
    #[test]
    fn the_two_new_capabilities_are_read_in_every_shape_a_server_sends_them() {
        let plain = json!({"selectionRangeProvider": true, "foldingRangeProvider": true});
        let offered = offered_by(Some(&plain));
        assert!(offered.selection_ranges && offered.folding_ranges);
        // An options object, or a registration object with a document selector in it: both mean
        // the request is answered.
        let objects = json!({
            "selectionRangeProvider": {"workDoneProgress": false},
            "foldingRangeProvider": {"documentSelector": [{"language": "rust"}], "id": "fold"}
        });
        let offered = offered_by(Some(&objects));
        assert!(offered.selection_ranges && offered.folding_ranges);
        // Said no, said nothing, and said something from a protocol this predates.
        let refused = json!({"selectionRangeProvider": false, "foldingRangeProvider": "someday"});
        let offered = offered_by(Some(&refused));
        assert!(!offered.selection_ranges && !offered.folding_ranges);
        assert_eq!(offered_by(None), Offered::default());
        // And the code action flags still come off the same reply, unchanged.
        let with_actions = json!({"codeActionProvider": {"resolveProvider": true}});
        assert_eq!(
            offered_by(Some(&with_actions)).actions,
            ActionSupport { offered: true, resolves: true }
        );
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
            Event::Ready { utf16, .. } => assert!(!utf16, "the server asked for UTF-8"),
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

    /// The uses of a name arrive in the same three shapes a definition does, and here the whole
    /// list is kept — which is the one difference, and the one thing worth checking.
    #[test]
    fn every_use_of_a_name_is_read_whichever_shape_it_arrives_in() {
        let here = json!({
            "uri": "file:///src/main.rs",
            "range": { "start": { "line": 41, "character": 8 }, "end": { "line": 41, "character": 12 } }
        });
        let there = json!({
            "uri": "file:///src/other.rs",
            "range": { "start": { "line": 3, "character": 0 }, "end": { "line": 3, "character": 4 } }
        });
        assert_eq!(
            all_locations(Some(&json!([here, there]))),
            vec![
                Jump { path: PathBuf::from("/src/main.rs"), line: 41, column: 8 },
                Jump { path: PathBuf::from("/src/other.rs"), line: 3, column: 0 },
            ],
            "both of them, in the order the server listed them"
        );
        // Links, which name the same two things differently — and whose selection range is the
        // name rather than the body it opens.
        let links = json!([{
            "targetUri": "file:///src/main.rs",
            "targetRange": { "start": { "line": 40, "character": 0 }, "end": { "line": 45, "character": 1 } },
            "targetSelectionRange": { "start": { "line": 41, "character": 8 }, "end": { "line": 41, "character": 12 } }
        }]);
        assert_eq!(
            all_locations(Some(&links)),
            vec![Jump { path: PathBuf::from("/src/main.rs"), line: 41, column: 8 }]
        );
        // A single use, sent bare rather than in an array of one. Reading this as nothing would
        // put "nothing uses that" on screen for a name that is used exactly once.
        assert_eq!(
            all_locations(Some(&here)),
            vec![Jump { path: PathBuf::from("/src/main.rs"), line: 41, column: 8 }]
        );
        // Nothing uses it, said as null or as an empty array.
        assert!(all_locations(Some(&Value::Null)).is_empty());
        assert!(all_locations(Some(&json!([]))).is_empty());
        assert!(all_locations(None).is_empty());
    }

    /// A file's names arrive flat or nested, both are in the specification, and CleeCode asks
    /// for neither — so both are read. The rows come out the same shape either way.
    #[test]
    fn a_files_names_are_read_flat_or_nested() {
        // The flat shape: `SymbolInformation`, whose position hides under `location`.
        let flat = json!([
            { "name": "main", "kind": 12,
              "location": { "uri": "file:///src/main.rs",
                            "range": { "start": { "line": 0, "character": 3 },
                                       "end": { "line": 4, "character": 1 } } } },
            { "name": "Config", "kind": 23, "containerName": "",
              "location": { "uri": "file:///src/main.rs",
                            "range": { "start": { "line": 6, "character": 7 },
                                       "end": { "line": 9, "character": 1 } } } },
        ]);
        assert_eq!(
            symbol_rows(Some(&flat)),
            vec![
                SymbolRow { name: "main".into(), kind: "fn", depth: 0, line: 0, column: 3 },
                SymbolRow { name: "Config".into(), kind: "struct", depth: 0, line: 6, column: 7 },
            ],
            "flat is flat: nothing is nested in anything"
        );

        // The nested shape: `DocumentSymbol`, which names the position twice and carries its
        // children with it. The selection range is the name; the range is the whole body, and
        // landing on its first brace is landing in the wrong place.
        let nested = json!([{
            "name": "Config", "kind": 23,
            "range": { "start": { "line": 6, "character": 0 }, "end": { "line": 12, "character": 1 } },
            "selectionRange": { "start": { "line": 6, "character": 7 }, "end": { "line": 6, "character": 13 } },
            "children": [
                { "name": "path", "kind": 8,
                  "range": { "start": { "line": 7, "character": 4 }, "end": { "line": 7, "character": 20 } },
                  "selectionRange": { "start": { "line": 7, "character": 4 }, "end": { "line": 7, "character": 8 } } },
                { "name": "load", "kind": 6,
                  "range": { "start": { "line": 9, "character": 4 }, "end": { "line": 11, "character": 5 } },
                  "selectionRange": { "start": { "line": 9, "character": 7 }, "end": { "line": 9, "character": 11 } },
                  "children": [
                      { "name": "attempt", "kind": 13,
                        "selectionRange": { "start": { "line": 10, "character": 12 },
                                            "end": { "line": 10, "character": 19 } } }
                  ] },
            ]
        }]);
        assert_eq!(
            symbol_rows(Some(&nested)),
            vec![
                SymbolRow { name: "Config".into(), kind: "struct", depth: 0, line: 6, column: 7 },
                SymbolRow { name: "path".into(), kind: "field", depth: 1, line: 7, column: 4 },
                SymbolRow { name: "load".into(), kind: "method", depth: 1, line: 9, column: 7 },
                SymbolRow { name: "attempt".into(), kind: "var", depth: 2, line: 10, column: 12 },
            ],
            "document order, each row carrying how far in it sits"
        );

        // A kind nothing here has a word for still gets a row: the name is the part being
        // looked for, and a number out of a newer specification is not a reason to lose it.
        let odd = json!([{ "name": "?", "kind": 99,
                           "selectionRange": { "start": { "line": 1, "character": 0 },
                                               "end": { "line": 1, "character": 1 } } }]);
        assert_eq!(symbol_rows(Some(&odd))[0].kind, "sym");

        // Nothing in the file, and nothing asked.
        assert!(symbol_rows(Some(&json!([]))).is_empty());
        assert!(symbol_rows(Some(&Value::Null)).is_empty());
        assert!(symbol_rows(None).is_empty());
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
        let Some(Event::Ready { utf16, offered }) = ready else { panic!("no handshake reply") };
        assert!(!utf16, "the stub asks to be counted in UTF-8");
        client.confirm_ready(utf16, offered);

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

    /// A `file:` URI for a path this test can write without caring what the host calls its root.
    fn uri(path: &str) -> String {
        format!("file://{path}")
    }

    /// The flat shape: a map of URI to edits, which is what a client advertising no
    /// `workspace.workspaceEdit` capability is supposed to be sent.
    #[test]
    fn the_flat_shape_of_a_workspace_edit_is_read() {
        let answer = json!({"changes": {
            uri("/p/src/main.rs"): [
                {"range": {"start": {"line": 3, "character": 8},
                           "end": {"line": 3, "character": 11}},
                 "newText": "renamed"},
                {"range": {"start": {"line": 9, "character": 4},
                           "end": {"line": 9, "character": 7}},
                 "newText": "renamed"},
            ],
        }});
        let plan = rename_plan(Some(&answer));
        assert!(!plan.file_ops);
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].path, PathBuf::from("/p/src/main.rs"));
        assert_eq!(
            plan.files[0].edits[0],
            SpanEdit {
                start_line: 3,
                start_col: 8,
                end_line: 3,
                end_col: 11,
                new_text: "renamed".to_string(),
            }
        );
        assert_eq!(plan.files[0].edits[1].start_line, 9);
        assert!(plan.files.iter().all(|f| f.edits.iter().all(|e| !e.spans_lines())));
    }

    /// The other shape, which is what several servers send whatever the handshake said. The
    /// annotated spelling of a text edit rides in it too, and is the same three fields plus one
    /// this does not read.
    #[test]
    fn the_document_changes_shape_is_read_as_well() {
        let answer = json!({"documentChanges": [
            {"textDocument": {"uri": uri("/p/a.rs"), "version": 4},
             "edits": [{"range": {"start": {"line": 0, "character": 0},
                                  "end": {"line": 0, "character": 3}},
                        "newText": "new"}]},
            {"textDocument": {"uri": uri("/p/b.rs"), "version": null},
             "edits": [{"range": {"start": {"line": 7, "character": 1},
                                  "end": {"line": 7, "character": 4}},
                        "newText": "new", "annotationId": "rename"}]},
        ]});
        let plan = rename_plan(Some(&answer));
        assert!(!plan.file_ops);
        let named: Vec<PathBuf> = plan.files.iter().map(|f| f.path.clone()).collect();
        assert_eq!(named, vec![PathBuf::from("/p/a.rs"), PathBuf::from("/p/b.rs")]);
        assert_eq!(plan.files[1].edits[0].new_text, "new");
        assert_eq!(plan.files[1].edits[0].start_col, 1);
    }

    /// A create, a rename or a delete of a *file* is not what "rename this name" means, and the
    /// flag is how the caller finds out it was asked for. The edits beside it are still read —
    /// what the caller does is refuse the whole answer, and it can only do that knowingly.
    #[test]
    fn a_file_operation_is_noticed_rather_than_quietly_skipped() {
        for kind in ["create", "rename", "delete"] {
            let answer = json!({"documentChanges": [
                {"textDocument": {"uri": uri("/p/a.rs")},
                 "edits": [{"range": {"start": {"line": 0, "character": 0},
                                      "end": {"line": 0, "character": 3}},
                            "newText": "new"}]},
                {"kind": kind, "uri": uri("/p/b.rs"), "oldUri": uri("/p/a.rs"),
                 "newUri": uri("/p/b.rs")},
            ]});
            let plan = rename_plan(Some(&answer));
            assert!(plan.file_ops, "a {kind} operation went unnoticed");
            assert_eq!(plan.files.len(), 1, "the edits beside it are still read");
        }
    }

    /// A span that runs off the end of its line is detected rather than clamped. Clamping is
    /// right for an underline and wrong for a replacement: it would delete a different amount of
    /// text from the one a preview showed.
    #[test]
    fn a_span_that_crosses_a_line_is_recognisable() {
        let answer = json!({"changes": {uri("/p/a.rs"): [
            {"range": {"start": {"line": 2, "character": 4},
                       "end": {"line": 5, "character": 1}},
             "newText": "new"},
        ]}});
        let plan = rename_plan(Some(&answer));
        assert!(plan.files[0].edits[0].spans_lines());
    }

    /// Nothing at all is an empty plan rather than a panic: a server may answer `null` to say it
    /// would change nothing, and an entry that cannot be read costs its own row and no more.
    #[test]
    fn an_unreadable_answer_costs_only_itself() {
        assert_eq!(rename_plan(None), RenamePlan::default());
        assert_eq!(rename_plan(Some(&Value::Null)), RenamePlan::default());
        let answer = json!({"changes": {
            "not-a-uri-at-all": [{"range": {"start": {"line": 0, "character": 0},
                                            "end": {"line": 0, "character": 1}},
                                  "newText": "x"}],
            uri("/p/a.rs"): [
                {"newText": "no range here"},
                {"range": {"start": {"line": 1, "character": 0},
                           "end": {"line": 1, "character": 2}}, "newText": "kept"},
            ],
        }});
        let plan = rename_plan(Some(&answer));
        assert_eq!(plan.files.len(), 1, "the unparseable URI took only its own file with it");
        assert_eq!(plan.files[0].edits.len(), 1, "the edit with no range took only itself");
        assert_eq!(plan.files[0].edits[0].new_text, "kept");
    }

    /// A rename is the one answer whose error is carried rather than read as "nothing to do".
    /// The server's own sentence, because "the server refused" would be true of every failure
    /// here and useful for none of them.
    #[test]
    fn a_refused_rename_arrives_with_what_the_server_said() {
        let wire = frame(
            r#"{"jsonrpc":"2.0","id":7,
                "error":{"code":-32602,"message":"cannot rename this element"}}"#,
        );
        let (tx, rx) = mpsc::channel();
        read_loop(BufReader::new(&wire[..]), tx, "stub".to_string(), pending_asks(&[(7, Ask::Rename)]));
        let events: Vec<Event> = rx.into_iter().collect();
        match &events[0] {
            Event::Rename { id, plan } => {
                assert_eq!(*id, 7);
                assert_eq!(plan.as_ref().err().map(String::as_str), Some("cannot rename this element"));
            }
            _ => panic!("the error did not come back as the answer to the rename"),
        }
    }

    /// The formatter's answer is a bare array, and the spans in it cross lines as a matter of
    /// course. Read as they were sent: a formatter replacing the whole file sends one edit from
    /// the top to past the bottom, and a parser that clamped it to its first line would delete
    /// the first line and call the file formatted.
    #[test]
    fn a_formatting_answer_is_a_bare_list_of_spans() {
        let answer = json!([
            {"range": {"start": {"line": 0, "character": 0},
                       "end": {"line": 3, "character": 0}},
             "newText": "fn main() {\n    ok();\n}\n"},
        ]);
        let edits = format_edits(Some(&answer));
        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0],
            SpanEdit {
                start_line: 0,
                start_col: 0,
                end_line: 3,
                end_col: 0,
                new_text: "fn main() {\n    ok();\n}\n".to_string(),
            }
        );
        assert!(edits[0].spans_lines(), "and it is the shape a rename refuses");
    }

    /// A whole-file format ends past the last line the buffer has, because a line *count* is one
    /// more than the last index — which is how a server spells "to the end of the document".
    ///
    /// Nothing is clamped here, and this test is the marker for that: the number arrives as it
    /// was sent, and the application clamps it against the rope, which is the only side that
    /// knows how many lines there are. A parser that clamped would have to guess.
    #[test]
    fn an_end_past_the_last_line_arrives_as_it_was_sent() {
        let answer = json!([
            {"range": {"start": {"line": 0, "character": 0},
                       "end": {"line": 4_294_967_295u32, "character": 0}},
             "newText": "laid out\n"},
        ]);
        let edits = format_edits(Some(&answer));
        assert_eq!(edits[0].end_line, 4_294_967_295);
    }

    /// Two smaller edits rather than one big one — reindenting two lines and leaving the rest —
    /// which is the other thing formatters do and the case the span rebuild is for.
    #[test]
    fn several_formatting_edits_all_arrive() {
        let answer = json!([
            {"range": {"start": {"line": 1, "character": 0},
                       "end": {"line": 1, "character": 8}}, "newText": "    "},
            {"range": {"start": {"line": 2, "character": 0},
                       "end": {"line": 2, "character": 6}}, "newText": "    "},
        ]);
        let edits = format_edits(Some(&answer));
        assert_eq!(edits.len(), 2);
        assert_eq!((edits[0].start_line, edits[1].start_line), (1, 2));
    }

    /// Nothing to do, said in each of the ways a server says it, is the empty list rather than a
    /// panic — and an entry with no range costs its own row and no more.
    #[test]
    fn an_unreadable_formatting_answer_costs_only_itself() {
        assert!(format_edits(None).is_empty());
        assert!(format_edits(Some(&Value::Null)).is_empty());
        assert!(format_edits(Some(&json!([]))).is_empty());
        assert!(format_edits(Some(&json!({"changes": {}}))).is_empty(), "not a WorkspaceEdit");
        let answer = json!([
            {"newText": "no range here"},
            {"range": {"start": {"line": 1, "character": 0},
                       "end": {"line": 1, "character": 2}}, "newText": "kept"},
        ]);
        let edits = format_edits(Some(&answer));
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "kept");
    }

    /// The capability that decides whether the question is asked at all, in both of the shapes a
    /// server may answer it in. A server that never named it is one this feature stays quiet
    /// about, which is the difference between a menu row that says so and a round trip that ends
    /// in a refusal.
    #[test]
    fn what_a_server_can_do_about_code_is_read_off_the_handshake() {
        // The bare form: it answers the request and cannot resolve.
        let plain = json!({ "codeActionProvider": true });
        assert_eq!(
            action_support(Some(&plain)),
            ActionSupport { offered: true, resolves: false }
        );
        // The options form, which is where the second flag lives. rust-analyzer answers this way.
        let full = json!({
            "codeActionProvider": { "codeActionKinds": ["quickfix", "refactor"], "resolveProvider": true }
        });
        assert_eq!(action_support(Some(&full)), ActionSupport { offered: true, resolves: true });
        // An options object that says nothing about resolving has said it cannot.
        let kinds = json!({ "codeActionProvider": { "codeActionKinds": ["quickfix"] } });
        assert_eq!(
            action_support(Some(&kinds)),
            ActionSupport { offered: true, resolves: false }
        );
        // Said no, said nothing, or said something out of a protocol this predates: all of them
        // are "do not ask", which is the reading that costs nobody a request.
        assert_eq!(action_support(Some(&json!({ "codeActionProvider": false }))), ActionSupport::default());
        assert_eq!(action_support(Some(&json!({ "hoverProvider": true }))), ActionSupport::default());
        assert_eq!(action_support(Some(&json!({ "codeActionProvider": "yes" }))), ActionSupport::default());
        assert_eq!(action_support(None), ActionSupport::default());
    }

    /// The ordinary quick fix: a title, a kind, and the edit that carries it out, in the same
    /// `WorkspaceEdit` a rename answers with — read by the same function, which is the whole
    /// point of it being that shape here.
    #[test]
    fn an_action_that_carries_its_edit_is_read_whole() {
        let answer = json!([{
            "title": "Import `HashMap`",
            "kind": "quickfix",
            "diagnostics": [{"range": {"start": {"line": 3, "character": 8},
                                       "end": {"line": 3, "character": 15}},
                             "message": "cannot find type `HashMap`"}],
            "edit": {"changes": {uri("/p/src/main.rs"): [
                {"range": {"start": {"line": 0, "character": 0},
                           "end": {"line": 0, "character": 0}},
                 "newText": "use std::collections::HashMap;\n"},
            ]}},
        }]);
        let actions = offered_actions(Some(&answer), false);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].title, "Import `HashMap`");
        assert_eq!(actions[0].kind, "quickfix");
        let plan = actions[0].edit.as_ref().expect("the edit arrived with it");
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].path, PathBuf::from("/p/src/main.rs"));
        assert_eq!(plan.files[0].edits[0].new_text, "use std::collections::HashMap;\n");
        // And the item is kept whole, because that is what a resolve request takes back.
        assert_eq!(actions[0].raw["title"], json!("Import `HashMap`"));
    }

    /// An action the server has only named. Kept where it can be asked about again and dropped
    /// where it cannot — a row that answered a keypress with nothing at all would be a row that
    /// should not have been offered.
    #[test]
    fn an_action_without_its_edit_waits_for_a_resolve_or_is_dropped() {
        let answer = json!([{
            "title": "Convert to guarded return",
            "kind": "refactor.rewrite",
            "data": {"id": "convert_to_guarded_return", "version": 7},
        }]);
        let resolvable = offered_actions(Some(&answer), true);
        assert_eq!(resolvable.len(), 1);
        assert!(resolvable[0].edit.is_none(), "the server has not said yet what it would change");
        assert_eq!(resolvable[0].raw["data"]["id"], json!("convert_to_guarded_return"));
        assert!(
            offered_actions(Some(&answer), false).is_empty(),
            "a server that cannot resolve has offered a row with nothing behind it"
        );

        // And the answer to the resolve, which is the same action back with the edit filled in.
        let resolved = json!({
            "title": "Convert to guarded return",
            "edit": {"documentChanges": [
                {"textDocument": {"uri": uri("/p/a.rs"), "version": 4},
                 "edits": [{"range": {"start": {"line": 2, "character": 4},
                                      "end": {"line": 6, "character": 5}},
                            "newText": "let Some(x) = x else { return };"}]},
            ]},
        });
        let plan = resolved_edit(Some(&resolved)).expect("the resolve filled it in");
        assert_eq!(plan.files[0].path, PathBuf::from("/p/a.rs"));
        // A server that answered without filling anything in has said it has nothing to change,
        // which the caller reports rather than applying an empty plan in silence.
        assert_eq!(resolved_edit(Some(&json!({"title": "Nothing to do"}))), None);
        assert_eq!(resolved_edit(None), None);
    }

    /// The three the list drops, and each of them for a reason worth writing down: this client
    /// has no `workspace/executeCommand` and refuses the workspace edits a server would push back
    /// through one, so an action that needs either is an action it cannot carry out.
    #[test]
    fn an_action_this_client_cannot_carry_out_is_dropped() {
        let edit = json!({"changes": {uri("/p/a.rs"): [
            {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
             "newText": "x"},
        ]}});
        let answer = json!([
            // A bare `Command`, which the protocol allows in this list.
            {"title": "Run the fixer", "command": "rust-analyzer.runFlycheck", "arguments": []},
            // An action whose work is a command, with no edit at all.
            {"title": "Regenerate", "kind": "source",
             "command": {"title": "Regenerate", "command": "gopls.generate"}},
            // And one with both: applying the edit alone would do half of what it asked.
            {"title": "Add import and reload", "kind": "quickfix", "edit": edit,
             "command": {"title": "Reload", "command": "ts.reload"}},
            // The server has said in so many words that this one cannot be applied here.
            {"title": "Extract into function", "kind": "refactor.extract",
             "disabled": {"reason": "Selection is not a valid expression"}, "edit": edit},
            // The one that survives.
            {"title": "Add import", "kind": "quickfix", "edit": edit},
        ]);
        let actions = offered_actions(Some(&answer), true);
        assert_eq!(actions.len(), 1, "only the one this client can actually do");
        assert_eq!(actions[0].title, "Add import");

        // Nothing offered, said in each of the ways a server says it.
        assert!(offered_actions(Some(&json!([])), true).is_empty());
        assert!(offered_actions(Some(&Value::Null), true).is_empty());
        assert!(offered_actions(None, true).is_empty());
        // A row with no title is a row with nothing to draw, and costs only itself.
        let untitled = json!([{"kind": "quickfix", "edit": edit}, {"title": "Kept", "edit": edit}]);
        let actions = offered_actions(Some(&untitled), true);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].title, "Kept");
    }

    /// The context is what makes a quick fix a quick fix: a server matches its fixes against the
    /// diagnostics it is handed back, and one asked with an empty context answers about the file
    /// in general and not about the error under the caret.
    #[test]
    fn the_context_carries_the_diagnostics_the_range_touches() {
        let held = [
            diag(3, 8, 15, "cannot find type `HashMap`"),
            diag(3, 20, 24, "unused variable"),
            diag(9, 0, 4, "somewhere else entirely"),
        ];
        // The caret inside the first one, as an empty range — which is how "here" is spelled.
        let touching = diagnostics_in_range(&held, (3, 10), (3, 10));
        assert_eq!(touching.len(), 1);
        assert_eq!(touching[0].message, "cannot find type `HashMap`");
        // Resting on the last character of it still counts: that is somebody asking about that
        // error, and a half-open comparison would answer about the line instead.
        assert_eq!(diagnostics_in_range(&held, (3, 15), (3, 15)).len(), 1);
        // A selection across both of the first line's, which is two questions at once.
        assert_eq!(diagnostics_in_range(&held, (3, 0), (3, 30)).len(), 2);
        // A line neither of them is on.
        assert!(diagnostics_in_range(&held, (5, 0), (5, 0)).is_empty());
        // And a selection that reaches the third one takes it and everything between.
        assert_eq!(diagnostics_in_range(&held, (3, 22), (9, 2)).len(), 2);
    }

    /// The third answer whose error is carried rather than read as "nothing to do". An empty list
    /// here already means "there is nothing I can do about that", which is the commonest answer
    /// of all — so a server's refusal read as one would be the editor inventing an answer.
    #[test]
    fn a_refused_code_action_question_arrives_with_what_the_server_said() {
        let wire = frame(
            r#"{"jsonrpc":"2.0","id":21,
                "error":{"code":-32603,"message":"content modified"}}"#,
        );
        let (tx, rx) = mpsc::channel();
        read_loop(
            BufReader::new(&wire[..]),
            tx,
            "stub".to_string(),
            pending_asks(&[(21, Ask::CodeActions { resolves: true })]),
        );
        let events: Vec<Event> = rx.into_iter().collect();
        match &events[0] {
            Event::CodeActions { id, actions } => {
                assert_eq!(*id, 21);
                assert_eq!(actions.as_ref().err().map(String::as_str), Some("content modified"));
            }
            _ => panic!("the error did not come back as the answer to the question"),
        }
    }

    /// The second answer whose error is carried rather than read as "nothing to do", and the one
    /// where the difference bites hardest: an empty formatting answer already means "already
    /// laid out", so a refusal read as one would tell the user the opposite of the truth.
    #[test]
    fn a_refused_format_arrives_with_what_the_server_said() {
        let wire = frame(
            r#"{"jsonrpc":"2.0","id":11,
                "error":{"code":-32601,"message":"formatting is not supported"}}"#,
        );
        let (tx, rx) = mpsc::channel();
        read_loop(
            BufReader::new(&wire[..]),
            tx,
            "stub".to_string(),
            pending_asks(&[(11, Ask::Formatting)]),
        );
        let events: Vec<Event> = rx.into_iter().collect();
        match &events[0] {
            Event::Formatting { id, edits } => {
                assert_eq!(*id, 11);
                assert_eq!(
                    edits.as_ref().err().map(String::as_str),
                    Some("formatting is not supported")
                );
            }
            _ => panic!("the error did not come back as the answer to the format"),
        }
    }
}
