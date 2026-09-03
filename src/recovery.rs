//! Copies of unsaved buffers, kept where they survive the process, and the offer to put them
//! back at the next start.
//!
//! CleeCode's README says *it does not close on you*, and the panic shield in `main.rs` is most
//! of that promise: a bug anywhere in a frame costs a status line, not the session. What the
//! shield cannot catch is the process not being there any more — `kill -9`, a stack overflow (no
//! unwinding, no `catch_unwind`), a machine losing power, a terminal emulator taking its children
//! with it when it dies. Every one of those ends the editor between one keystroke and the next,
//! and until this existed everything unsaved went with it.
//!
//! So: a copy of each dirty buffer, written every few seconds, into a directory the next CleeCode
//! reads. This is the one piece of CleeCode's runtime state that belongs in the **config**
//! directory rather than in the temp dir. `mcp::sessions_root` chose the temp dir precisely
//! because a session directory means nothing once the process is gone; the reasoning inverts
//! here, because meaning something once the process is gone is the entire point. `panic.log`
//! (`main.rs`) and the saved workspaces (`workspace.rs`) live in the config dir for the same
//! reason — they are what is left to look at afterwards.
//!
//! What this does not cover, said plainly because a safety net that is believed to be finer than
//! it is is worse than none: the last few seconds of typing. The copy is taken on a tick, so an
//! edit made after the last tick and before the crash was never written down anywhere. This
//! narrows the loss from "the session" to "a few seconds"; it does not remove it, and nothing
//! short of writing on every keystroke would.
//!
//! ## What is on disk
//!
//! One file per buffer, named so that the same buffer overwrites its own copy rather than
//! growing a new one every tick, and readable with `cat`: a one-line header saying what the
//! rest of the file is, then the buffer's text exactly as the buffer held it — `\n`-normalised,
//! because a recovered buffer goes back into a rope and not onto disk. Line endings are the save
//! path's business (`Editor::save` reapplies CRLF from what the file was opened as), and a
//! recovery copy that had already made that decision would make it twice.
//!
//! Named buffers are keyed by their canonical path, so the copy follows the file rather than the
//! tab. Unnamed ones are keyed by the pid that wrote them and a per-buffer number, because there
//! is nothing else to key them by — and they are the reason this module has an unnamed case at
//! all. A never-saved buffer is the only work in CleeCode that dies *without a trace*: the resume
//! written on exit keeps `last_open_files`, and `main.rs` builds that list with
//! `filter_map(|e| e.path.clone())`, which drops every buffer that has no path. Nothing else
//! anywhere remembered they existed.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The suffix every recovery file carries, and the only thing in the directory that is read.
///
/// It also keeps `settings::write_atomic`'s scratch file out of the way: that lands as
/// `.<name>.clee<pid>.tmp` beside its target, so a half-written copy never matches.
pub const SUFFIX: &str = ".clee-recovery";

/// The first line of every copy. Versioned because the format is on somebody's disk and a later
/// CleeCode has to be able to tell an older file from one it cannot read.
const HEADER: &str = "clee-recovery 1 ";

/// Where the copies live. `None` on a machine with no config directory, which costs recovery and
/// nothing else — the same stance `Settings` and `workspace` take.
pub fn dir() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("recovery"))
}

/// One copy on disk, read back and ready to be offered.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The file this was read from, so restoring can remove it afterwards.
    pub file: PathBuf,
    /// The buffer's own file, canonical, or `None` for a buffer that never had a name.
    pub original: Option<PathBuf>,
    /// When the copy was last written, for the "how old is this" the offer shows.
    pub saved: SystemTime,
    pub text: String,
}

/// The canonical form of a path, or the path itself when the filesystem cannot say — a file
/// deleted while a buffer still held it, most often.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// FNV-1a over the canonical path, so a buffer's copy always lands on the same file name.
///
/// Hand-rolled rather than `DefaultHasher`, and that is the whole reason it is here: the standard
/// hasher is explicitly not stable across Rust releases, so a toolchain upgrade would rename
/// every entry — leaving yesterday's copies behind under names nothing would ever overwrite
/// again. This one is fixed forever.
fn fingerprint(path: &Path) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in path.to_string_lossy().as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The file's own name, reduced to characters every filesystem accepts, so that a human reading
/// the directory can tell the copies apart without opening them. The hash beside it is what
/// actually identifies the buffer; this is for the reader.
fn readable_tail(path: &Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut out: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '-' })
        .collect();
    // Every character above is ASCII, so this cannot split one in half.
    out.truncate(40);
    if out.is_empty() { "file".to_string() } else { out }
}

/// What a buffer's copy is called. `original` must already be canonical; `id` identifies an
/// unnamed buffer within the session that wrote it and is ignored for a named one.
pub fn entry_name(original: Option<&Path>, id: u64) -> String {
    match original {
        Some(path) => format!("f-{:016x}-{}{SUFFIX}", fingerprint(path), readable_tail(path)),
        None => format!("untitled-{}-{id}{SUFFIX}", std::process::id()),
    }
}

/// Escapes the one thing a single-line header cannot hold: a path with a newline in it. Legal on
/// Unix, vanishingly rare, and the difference between "this file is skipped" and "this file
/// silently truncates somebody's buffer at the first line".
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\r', "\\r").replace('\n', "\\n")
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// The header line for a copy: what the text below it belongs to.
fn header(original: Option<&Path>, id: u64) -> String {
    match original {
        Some(path) => format!("{HEADER}file {}", escape(&path.to_string_lossy())),
        None => format!("{HEADER}untitled {} {id}", std::process::id()),
    }
}

/// What a header line says: the buffer's file, or the pid of the session that owned the unnamed
/// buffer. `None` for anything this version cannot read, which is left on disk untouched rather
/// than deleted — a copy a newer CleeCode wrote is that CleeCode's to deal with.
fn parse_header(line: &str) -> Option<(Option<PathBuf>, Option<u32>)> {
    let rest = line.strip_prefix(HEADER)?;
    if let Some(path) = rest.strip_prefix("file ") {
        return Some((Some(PathBuf::from(unescape(path))), None));
    }
    let owner = rest.strip_prefix("untitled ")?;
    let pid = owner.split_whitespace().next()?.parse::<u32>().ok()?;
    Some((None, Some(pid)))
}

/// Writes one buffer's copy, replacing the one already there.
///
/// Atomic like every other file CleeCode writes, and for a sharper reason than usual: this file
/// exists to be read by a process that starts after this one died, and the moment it is most
/// likely to die is the moment it is doing work. A plain write truncates first, so a crash
/// mid-copy would leave the recovery file holding half a buffer — a safety net that hands back
/// the top of your work is worse than one that hands back nothing, because it looks like the
/// whole of it.
pub fn write_entry(original: Option<&Path>, id: u64, text: &str) -> std::io::Result<PathBuf> {
    let dir = dir().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "no config directory to recover into")
    })?;
    std::fs::create_dir_all(&dir)?;
    let canonical = original.map(canonical);
    let path = dir.join(entry_name(canonical.as_deref(), id));
    let mut body = header(canonical.as_deref(), id);
    body.push('\n');
    body.push_str(text);
    crate::settings::write_atomic(&path, body.as_bytes())?;
    Ok(path)
}

/// Whether a buffer is owed a fresh copy on this tick.
///
/// A free function rather than four conditions inline in the tick, so the rule can be stated once
/// and checked without an `App`, a terminal or a clock. `copied` is the revision the buffer's
/// last copy held, `None` when it has none.
///
/// The `dirty` and `revision` pair is what makes this cheap and trustworthy at the same time.
/// Both have exactly one author — `Editor::mark_edited_from` sets `dirty` and moves `revision`
/// together, `Editor::save` is the only thing that clears `dirty` — so "unsaved, and different
/// from what was last copied" is two integer comparisons rather than a diff of the text. Without
/// the revision half a file left open unsaved would have its whole text rewritten to disk every
/// few seconds for as long as the editor stayed open.
pub fn needs_copy(dirty: bool, read_only: bool, revision: u64, copied: Option<u64>) -> bool {
    dirty && !read_only && copied != Some(revision)
}

/// Drops the copies a buffer no longer needs: the one under its name, and the one it had while
/// it was still unnamed.
///
/// Both, because a Save As turns one into the other and leaving the unnamed half behind would
/// mean being offered, at the next start, work that is already on disk under a name.
pub fn forget(original: Option<&Path>, id: u64) {
    let Some(dir) = dir() else { return };
    let _ = std::fs::remove_file(dir.join(entry_name(None, id)));
    if let Some(path) = original {
        let _ = std::fs::remove_file(dir.join(entry_name(Some(&canonical(path)), 0)));
    }
}

/// Removes the copies this session made for buffers that never had a name.
///
/// Called on the way out of a clean exit. An unnamed copy is offered back when the pid that wrote
/// it is no longer alive, and after a normal quit this session's pid is exactly that — so without
/// this, every ordinary exit would leave an `[untitled]` row waiting at the next start.
pub fn sweep_own_unnamed() {
    let Some(dir) = dir() else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let mine = format!("untitled-{}-", std::process::id());
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with(&mine) && name.ends_with(SUFFIX) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Whether the copy is older than the file it is a copy of.
///
/// A file saved by something else since the copy was taken — another editor, a formatter, a
/// branch switched underneath, or an earlier CleeCode that did get to save — makes the copy
/// yesterday's text. Offering it would mean putting the older of two versions in front of
/// somebody as though it were the newer one, which is the one thing a recovery offer must never
/// do. A file that is *gone* is not stale: then the copy is all there is left of it.
fn is_stale(original: &Path, saved: SystemTime) -> bool {
    match std::fs::metadata(original).and_then(|m| m.modified()) {
        Ok(on_disk) => on_disk > saved,
        Err(_) => false,
    }
}

/// Everything worth offering when CleeCode starts in `root`, newest copy first.
///
/// Reading is also tidying, because there is no other moment that has the whole directory in
/// front of it: a stale copy is removed wherever its file lives, this project or another, rather
/// than being carried forward to be rejected again at every future start. Staleness is a fact
/// about two timestamps and does not become truer by waiting.
///
/// Three kinds of entry are deliberately left where they are. A live named copy for a file
/// outside this project belongs to a session that will open that project. An unnamed copy whose
/// pid is still alive belongs to a CleeCode that is running right now — offering it would hand
/// one window the buffer another window is still typing into. And a header this version cannot
/// parse belongs to a newer CleeCode.
///
/// The one exception is an unnamed copy carrying *our own* pid, which cannot be ours — we have
/// written nothing yet. Pids come round again, so it is a dead predecessor's, and the name it
/// sits under is one this session is about to want. It is read into memory and its file removed
/// straight away, so the copy is still offered and nothing this session writes can land on top
/// of it.
pub fn scan(root: &Path) -> Vec<Entry> {
    let Some(dir) = dir() else { return Vec::new() };
    let Ok(listing) = std::fs::read_dir(&dir) else { return Vec::new() };
    let root = canonical(root);
    let mine = std::process::id();
    let mut alive = LivePids::new();
    let mut found = Vec::new();
    for listed in listing.flatten() {
        let name = listed.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.ends_with(SUFFIX) || name.starts_with('.') {
            continue;
        }
        let file = listed.path();
        let Ok(body) = std::fs::read_to_string(&file) else { continue };
        // Split rather than `lines()`: the text below the header keeps every byte it had,
        // including a final newline that `lines()` would eat.
        let (line, text) = match body.split_once('\n') {
            Some((line, text)) => (line, text),
            None => (body.as_str(), ""),
        };
        let Some((original, pid)) = parse_header(line) else { continue };
        // A copy whose age cannot be read is treated as ancient rather than skipped: it is still
        // somebody's work, and "how old is it" only decides which row it sorts into.
        let saved =
            std::fs::metadata(&file).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
        match (&original, pid) {
            (Some(path), _) => {
                if is_stale(path, saved) {
                    let _ = std::fs::remove_file(&file);
                    continue;
                }
                if !path.starts_with(&root) {
                    continue;
                }
            }
            (None, Some(pid)) if pid == mine => {
                let _ = std::fs::remove_file(&file);
            }
            (None, Some(pid)) => {
                if alive.holds(pid) {
                    continue;
                }
            }
            (None, None) => continue,
        }
        found.push(Entry { file, original, saved, text: text.to_string() });
    }
    // Newest first: the copy taken last is the one most likely to be the work being looked for.
    found.sort_by(|a, b| b.saved.cmp(&a.saved).then(a.file.cmp(&b.file)));
    found
}

/// The process table, read once and only if something actually asks about a pid.
///
/// Same machine as `mcp::sweep_orphans`, and read through `sysinfo` for the same reason: it is
/// how the rest of the program asks this question, and it answers it identically on macOS, Linux
/// and Windows. Lazy because the common case is a recovery directory with no unnamed copies in
/// it, and a full process refresh for nothing is a visible fraction of a startup.
struct LivePids(Option<sysinfo::System>);

impl LivePids {
    fn new() -> Self {
        LivePids(None)
    }

    fn holds(&mut self, pid: u32) -> bool {
        let sys = self.0.get_or_insert_with(|| {
            let mut sys = sysinfo::System::new();
            sys.refresh_processes_specifics(
                sysinfo::ProcessesToUpdate::All,
                true,
                sysinfo::ProcessRefreshKind::nothing(),
            );
            sys
        });
        sys.process(sysinfo::Pid::from_u32(pid)).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The name and the header are the two halves of "which buffer is this", and they have to
    /// agree with each other across a write and a read. A named buffer is identified by its path
    /// alone — the same file always lands on the same copy, whatever tab it was in — and an
    /// unnamed one by the session that wrote it plus its own number.
    #[test]
    fn an_entry_names_itself_the_same_way_twice_and_reads_back_as_what_it_was() {
        let file = Path::new("/tmp/some project/src/main.rs");
        assert_eq!(entry_name(Some(file), 0), entry_name(Some(file), 7), "the id is not part of a named copy");
        assert_ne!(entry_name(Some(file), 0), entry_name(Some(Path::new("/tmp/other/src/main.rs")), 0));
        // Readable at a glance, and safe as a file name: no slashes, no spaces.
        let named = entry_name(Some(file), 0);
        assert!(named.ends_with(&format!("-main.rs{SUFFIX}")), "{named}");
        assert!(!named.contains('/') && !named.contains(' '), "{named}");

        let (original, pid) = parse_header(&header(Some(file), 0)).expect("a named header parses");
        assert_eq!(original.as_deref(), Some(file));
        assert_eq!(pid, None);

        // Unnamed: no path, and the pid that owned it, which is what decides whether it is ever
        // offered to anybody.
        let untitled = entry_name(None, 3);
        assert!(untitled.starts_with(&format!("untitled-{}-3", std::process::id())), "{untitled}");
        assert_ne!(untitled, entry_name(None, 4), "two unnamed buffers keep two copies");
        let (original, pid) = parse_header(&header(None, 3)).expect("an unnamed header parses");
        assert_eq!(original, None);
        assert_eq!(pid, Some(std::process::id()));

        // A path with a newline in it is legal on Unix and would otherwise turn one header line
        // into two, silently truncating the buffer below it at the first line break.
        let awkward = PathBuf::from("/tmp/one\ntwo\\three/x.rs");
        let (original, _) = parse_header(&header(Some(&awkward), 0)).expect("an awkward path parses");
        assert_eq!(original, Some(awkward));

        // Anything this version does not understand is not a copy it may delete.
        assert!(parse_header("clee-recovery 9 file /x").is_none());
        assert!(parse_header("hello").is_none());
    }

    /// The gate the writer runs on every buffer, five seconds apart, for the life of the session.
    /// Getting it wrong is not a crash: it is either a whole file written to disk repeatedly
    /// while nobody types, or — the one that matters — a copy that quietly stops following the
    /// buffer it is a copy of.
    #[test]
    fn a_buffer_is_copied_when_it_is_unsaved_and_has_moved_since_its_last_copy() {
        // Never copied, and there are changes: this is the first tick after the first keystroke.
        assert!(needs_copy(true, false, 1, None));
        // Copied at this exact revision: nothing has been typed since, so nothing is written.
        // Without this, a file left open unsaved is rewritten every five seconds forever.
        assert!(!needs_copy(true, false, 4, Some(4)));
        // Typed into again since the copy.
        assert!(needs_copy(true, false, 5, Some(4)));
        // Saved. `dirty` is false and the copy has been removed by `Editor::save`; a tick that
        // wrote here would put back the file it just deleted.
        assert!(!needs_copy(false, false, 5, Some(4)));
        assert!(!needs_copy(false, false, 5, None));
        // A binary or undecodable buffer refuses to save and must not be copied either — the
        // copy would be text the editor never managed to read.
        assert!(!needs_copy(true, true, 1, None));
    }

    /// The rule that keeps a recovery offer honest: a copy is only worth having while it is
    /// newer than the file it copies. A file saved by something else since — another editor, a
    /// formatter, the branch switched underneath — makes the copy the older of the two, and
    /// handing that back as "your unsaved work" would quietly undo whatever happened in between.
    #[test]
    fn a_copy_older_than_the_file_it_copies_is_not_worth_offering() {
        let dir = std::env::temp_dir().join(format!("clee_recovery_stale_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("source.txt");
        std::fs::write(&file, "on disk\n").unwrap();
        let on_disk = std::fs::metadata(&file).unwrap().modified().unwrap();

        let older = on_disk - std::time::Duration::from_secs(60);
        let newer = on_disk + std::time::Duration::from_secs(60);
        assert!(is_stale(&file, older), "a copy taken before the last save is stale");
        assert!(!is_stale(&file, newer), "a copy taken after it is the work to hand back");

        // A file that is gone is not a file that is newer: then the copy is all there is left,
        // and refusing to offer it would be losing the work twice.
        std::fs::remove_file(&file).unwrap();
        assert!(!is_stale(&file, older));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
