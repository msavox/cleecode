use crate::highlight::{Highlighter, LineCache};
use crate::i18n::{self, Key, Lang};
use anyhow::Result;
use ratatui::style::Style;
use ropey::Rope;
use std::borrow::Cow;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

/// How lines are terminated on disk, detected on open and reapplied on save so we never
/// silently rewrite a file's line endings (a spurious full-file diff on the first save).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineEnding {
    Lf,
    Crlf,
}

/// The size above which a file opens in the declared large-file mode: no highlighting, no
/// completion index, a shallow undo history, and a word on the status bar saying so.
///
/// A constant and not a setting, on purpose. A knob is a promise to behave sensibly at every
/// value somebody turns it to, and this is not a preference — it is the one line where the
/// editor stops offering what it cannot afford and says which things it dropped. Fifty
/// megabytes is where the costs stop being theoretical: the per-buffer bills the mode avoids
/// are each documented at the site that avoids them.
///
/// Nothing here is a promise that a 50 MB file *edits* well — a checkpoint is still a full copy
/// of the text (see `Snapshot`), so typing costs one of those. It is a promise that opening one
/// does not quietly turn into gigabytes of highlight spans and undo snapshots while the editor
/// looks like it has hung.
pub const LARGE_FILE: u64 = 50 * 1024 * 1024;

/// How many undo steps a normal buffer keeps, and how many a large one does.
///
/// A `Snapshot` is the whole text, so the history's cost is depth × file size and nothing else.
/// Five hundred steps of a normal source file is a few megabytes and worth every byte; five
/// hundred steps of a 50 MB file is twenty-five gigabytes, which is not a deep history but an
/// out-of-memory kill with the user's unsaved work inside it. Twenty steps of the same file is
/// a gigabyte at worst and, far more usually, the handful of steps anyone actually walks back.
///
/// Reducing the depth rather than switching undo off: a large file is exactly where an
/// accidental keystroke is hardest to find again by eye.
const MAX_UNDO: usize = 500;
const MAX_UNDO_LARGE: usize = 20;

/// A point-in-time snapshot of the buffer used for undo/redo. Stores the full text plus
/// cursor position; consecutive same-kind edits coalesce so one keystroke ≠ one undo step.
#[derive(Clone)]
struct Snapshot {
    text: String,
    cursor_line: usize,
    cursor_col: usize,
}

/// The kind of the last edit, used to coalesce runs of the same operation into a single
/// undo step (typing a word is one undo, not one-per-character).
#[derive(Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Insert,
    Delete,
    /// Any structural edit (newline, paste, line move, …): always its own undo step.
    Other,
    None,
}

pub struct Editor {
    pub rope: Rope,
    pub path: Option<PathBuf>,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub top_line: usize,
    pub left_col: usize,
    /// Where the view was as of the last frame, and when it last moved, so the scrollbars can
    /// show while it is moving and fade out once it settles.
    ///
    /// Compared once per frame rather than stamped at every call site that scrolls: the wheel,
    /// the arrows, Go to line, folding and a resize all move the view, and one comparison
    /// catches every one of them — including the ones added later.
    scroll_seen: (usize, usize),
    scroll_moved: Option<Instant>,
    /// Where the cursor was, and how big the viewport was, as of the last frame. The view is
    /// dragged back to the cursor only when one of the two has changed — see `follow_cursor`.
    cursor_seen: (usize, usize),
    viewport_seen: (usize, usize),
    /// Bumped by every change to the text. A live preview watches it to know whether what it
    /// drew is still what the buffer says — the cheapest honest answer to "has this moved since
    /// I last rendered it", and far cheaper than comparing the text itself every frame.
    revision: u64,
    pub dirty: bool,
    /// Which recovery copy this buffer's autosave writes to while it has no file name, and what
    /// identifies its copy after a Save As has given it one. Handed out once per buffer and never
    /// reused, so two untitled tabs can never write over each other's work. See `recovery.rs`.
    pub recovery_id: u64,
    /// The revision the last recovery copy held, or `None` when there is no copy.
    ///
    /// This and `dirty` together are what keep the autosave tick from rewriting the same bytes
    /// every few seconds for as long as a file is left open unsaved. The pair can be trusted
    /// because both halves have exactly one author: `dirty` is set in `mark_edited_from` and
    /// nowhere else, and cleared by `save` and nowhere else, and `revision` moves with it.
    pub autosaved_revision: Option<u64>,
    pub disk_mtime: Option<SystemTime>,
    /// Coloured spans for the first `syntax_cache.valid_lines()` lines of the buffer, and nothing
    /// for the ones below: they are not stale, they are not made yet. The renderer asks for the
    /// lines it is about to draw before it draws them.
    pub highlighted: Vec<Vec<(Style, String)>>,
    /// Where `highlighted` has got to, and what it needs to carry on from there.
    syntax_cache: LineCache,
    /// Set when the whole of `highlighted` has to go: a language change, a settings change —
    /// anything an edit's "from this line down" cannot describe.
    pub syntax_dirty: bool,
    /// The line an edit in flight began on, noted when its undo checkpoint is taken.
    ///
    /// The cursor before an edit and the cursor after it bracket almost every edit there is, and
    /// the earlier of the two is where re-highlighting must start. The few that reach further up
    /// than either — indenting a selection from below its first line, replacing a range found
    /// somewhere else — say so themselves.
    pending_edit_line: Option<usize>,
    pub selection_anchor: Option<(usize, usize)>,
    /// Whether the selection is a rectangle rather than a run of text. A column selection has
    /// the same two endpoints; what changes is which cells between them count as inside.
    pub selection_block: bool,
    /// Active (collapsed) fold regions as (start_line, end_line), inclusive, sorted by start.
    pub folds: Vec<(usize, usize)>,
    /// Where the language server says this file's blocks begin and end, when one has been asked
    /// and has answered. The same `(start_line, end_line)` pair as `folds` above, which is not a
    /// coincidence: the protocol's `foldingRange` hides `startLine + 1 ..= endLine`, and so does
    /// `is_hidden`, so the numbers cross over without arithmetic.
    ///
    /// Reaches a buffer the way diagnostics do — the app holds the answer against the file's path
    /// and hands it to the tab that has that file open — because it belongs to the file rather than
    /// to this struct, and a buffer that outlives its server has no business inventing boundaries.
    ///
    /// Only ever read while `dirty` is false, and that rule is the whole of the cache's honesty:
    /// these are line numbers, an edit moves lines, and a list of them taken before the edit
    /// describes a file that no longer exists. See `foldable_range_at`.
    pub server_folds: Vec<(usize, usize)>,
    /// The lines the last reload from disk brought in: what somebody else — an agent, a
    /// formatter, a branch switched underneath — wrote while the file was open. Ascending, so
    /// the gutter can ask about one line with a binary search rather than a scan.
    ///
    /// Emptied by the first edit of your own, wherever it comes from: see `mark_edited_from`.
    changed_lines: Vec<usize>,
    /// Line-ending style detected on open, reapplied on save.
    pub line_ending: LineEnding,
    /// Whether the file ended with a trailing newline when opened; preserved on save.
    pub final_newline: bool,
    /// Set when the file couldn't be loaded as text (binary/undecodable, or a read error).
    /// Such a buffer is display-only and refuses to save, so we never truncate the original.
    pub read_only: bool,
    /// How many bytes this buffer's file was when it was last read, and the size it is judged
    /// against. Together they answer `is_large` — the declared large-file mode.
    ///
    /// Two numbers rather than a `bool`, so the mode and the size quoted in the message that
    /// announces it cannot drift apart: there is one measurement and everything else asks it.
    /// The limit is carried per buffer rather than read from `LARGE_FILE` at each site so a
    /// reload re-decides by the same rule the open used — the tests open with a small limit,
    /// and a buffer must not change mode because of who opened it.
    ///
    /// Both are refreshed by an external reload, since a file can grow past the line, or be
    /// truncated back under it, while it sits in a tab. A buffer that grows past it by *typing*
    /// stays in whatever mode it opened in: this is about the file on disk, and re-deciding
    /// mid-edit would drop the colours out from under someone in the middle of a paste.
    disk_len: u64,
    large_limit: u64,
    /// Set instead of a text buffer when the file is a picture: the tab draws it rather than
    /// pretending to hold lines. Every text operation already refuses on `read_only`, which
    /// such a tab always is, so this only has to change what gets drawn.
    pub preview: Option<crate::preview::Preview>,
    /// Oldest snapshot first, so the eviction that keeps the history bounded drops one from the
    /// front instead of shuffling five hundred of them down by one.
    undo_stack: VecDeque<Snapshot>,
    redo_stack: Vec<Snapshot>,
    last_edit: EditKind,
    /// While set, nested mutations skip their own checkpoint so a compound edit (paste,
    /// line move, comment toggle) collapses into a single undo step.
    in_compound: bool,
}

/// The next unnamed buffer's recovery number, counted per process.
///
/// Never reset and never reused, because the number is half of a file name on disk: a counter
/// that started again would let a second untitled buffer overwrite the copy of the first, and
/// the work lost would be exactly the work this is for.
fn next_recovery_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Editor {
    pub fn empty() -> Self {
        Editor {
            rope: Rope::new(),
            path: None,
            cursor_line: 0,
            cursor_col: 0,
            top_line: 0,
            left_col: 0,
            scroll_seen: (0, 0),
            scroll_moved: None,
            cursor_seen: (0, 0),
            viewport_seen: (0, 0),
            revision: 0,
            dirty: false,
            recovery_id: next_recovery_id(),
            autosaved_revision: None,
            disk_mtime: None,
            highlighted: Vec::new(),
            syntax_cache: LineCache::default(),
            syntax_dirty: true,
            pending_edit_line: None,
            selection_anchor: None,
            selection_block: false,
            folds: Vec::new(),
            server_folds: Vec::new(),
            changed_lines: Vec::new(),
            line_ending: LineEnding::Lf,
            final_newline: false,
            read_only: false,
            disk_len: 0,
            large_limit: LARGE_FILE,
            preview: None,
            undo_stack: VecDeque::new(),
            redo_stack: Vec::new(),
            last_edit: EditKind::None,
            in_compound: false,
        }
    }

    /// Whether a file is binary, judged the way `file(1)` does: a NUL byte near the front.
    ///
    /// Only the head is read, on purpose. This is asked *before* deciding to open something as
    /// text, and the files it exists to catch — pictures, PDFs — are exactly the ones it would
    /// be wasteful to slurp whole only to throw away. A file too short to hold a NUL, or one
    /// that cannot be read at all, is left to `open` to deal with properly.
    pub fn looks_binary(path: &std::path::Path) -> bool {
        use std::io::Read;
        let Ok(mut file) = std::fs::File::open(path) else { return false };
        let mut head = [0u8; 8192];
        match file.read(&mut head) {
            Ok(n) => head[..n].contains(&0),
            Err(_) => false,
        }
    }

    /// A tab that shows a file instead of holding it: read-only, no buffer, and a picture on
    /// its way. There is deliberately no rope behind it — an empty one that pretended to be the
    /// file's contents is exactly what this replaces.
    /// Whether this tab is a rendered view of another buffer rather than a file of its own.
    pub fn is_rendered_view(&self) -> bool {
        self.preview.as_ref().is_some_and(|p| p.source.is_some())
    }

    pub fn preview(path: PathBuf, preview: crate::preview::Preview) -> Self {
        let mut editor = Editor::empty();
        // Recorded up front like any other opened file, so the watcher that re-renders a
        // changed preview has a baseline. Left unset, every tab would look stale the instant it
        // opened and render itself twice.
        editor.disk_mtime = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());
        editor.path = Some(path);
        editor.read_only = true;
        editor.preview = Some(preview);
        editor
    }

    pub fn open(path: PathBuf) -> Result<Self> {
        Editor::open_with_limit(path, LARGE_FILE)
    }

    /// `open` with the large-file line passed in, which is the only reason this seam exists:
    /// a test can reach the mode with a file it can afford to write, rather than fifty
    /// megabytes of fixture per assertion. Nothing in the app calls it with anything but
    /// `LARGE_FILE` — the threshold is a declaration, not a setting.
    pub fn open_with_limit(path: PathBuf, limit: u64) -> Result<Self> {
        // One `metadata` call answers both questions: when the file was last written, and how
        // big it is. The size decides the mode below; asking for it separately would be a
        // second stat of the same file for one `u64`.
        let stat = std::fs::metadata(&path).ok();
        let disk_mtime = stat.as_ref().and_then(|m| m.modified().ok());
        let mut editor = Editor::empty();
        editor.disk_mtime = disk_mtime;
        editor.large_limit = limit;
        editor.disk_len = stat.as_ref().map_or(0, |m| m.len());

        // Read as bytes so we can tell "empty file" apart from "unreadable/binary file":
        // the old read_to_string(...).unwrap_or_default() turned both a read error and a
        // binary file into an empty buffer, which a later save would write over the
        // original. A NUL byte or invalid UTF-8 marks the buffer read-only instead.
        match std::fs::read(&path) {
            Ok(bytes) => {
                if bytes.contains(&0) {
                    editor.read_only = true;
                } else {
                    match String::from_utf8(bytes) {
                        Ok(content) => {
                            editor.line_ending =
                                if content.contains("\r\n") { LineEnding::Crlf } else { LineEnding::Lf };
                            editor.final_newline = content.ends_with('\n');
                            // Store normalized to '\n' internally; the ending is reapplied on save.
                            editor.rope = Rope::from_str(&content.replace("\r\n", "\n"));
                        }
                        Err(_) => editor.read_only = true,
                    }
                }
            }
            Err(_) => editor.read_only = true,
        }

        editor.path = Some(path);
        Ok(editor)
    }

    pub fn save(&mut self) -> Result<()> {
        if self.read_only {
            anyhow::bail!("buffer is read-only (binary or undecodable file); not saving");
        }
        // A buffer with no file name used to fall through and report success without writing
        // anything, so "save" on the quit prompt silently discarded the work.
        let Some(path) = self.path.clone() else {
            anyhow::bail!("buffer has no file name yet; needs Save As");
        };
        let mut text = self.rope.to_string();
        // Drop the internal trailing newline if the file didn't originally have one, so
        // saving an edit doesn't spuriously append a final newline.
        if !self.final_newline {
            if let Some(stripped) = text.strip_suffix('\n') {
                text = stripped.to_string();
            }
        }
        if self.line_ending == LineEnding::Crlf {
            text = text.replace('\n', "\r\n");
        }
        // Written to a sibling temp file and renamed into place. A plain write truncates the
        // user's file before the first byte lands, so a crash or a full disk halfway through a
        // save destroyed the only copy of the work — the one thing a save must never do.
        crate::settings::write_atomic(&path, text.as_bytes())?;
        self.dirty = false;
        // The work is on disk under its own name now, so the copy kept against a crash has
        // nothing left to protect. Hooked here rather than at the three call sites — Save, Save
        // All, Save As — because this is the one place a save is known to have *succeeded*, and
        // a copy removed after a failed write would be the one thing this module exists to
        // prevent. Removing a file that was never written is a no-op, so a buffer saved before
        // the first autosave tick costs nothing here.
        crate::recovery::forget(Some(&path), self.recovery_id);
        self.autosaved_revision = None;
        self.disk_mtime = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());
        Ok(())
    }

    /// True if the buffer has content that can meaningfully be edited/saved. Used to gate
    /// the read-only guard's messaging.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// True in the declared large-file mode. Asked once per frame by the renderer and once per
    /// open tab by the completion popup, so it is a comparison of two numbers already in the
    /// struct and nothing else — a mode that cost anything to ask about would be a poor way to
    /// spend a large file's budget.
    pub fn is_large(&self) -> bool {
        self.disk_len > self.large_limit
    }

    /// The file's size in whole megabytes, for the sentence that announces the mode. Rounded
    /// down, because the number is there to be recognised — "50 MB" against a file the user
    /// knows is fifty-something — and a decimal place would only invite reading it as exact.
    pub fn megabytes(&self) -> u64 {
        self.disk_len / (1024 * 1024)
    }

    /// Called periodically from the main loop. If the file changed on disk and we have
    /// no unsaved local edits, reload it silently. Returns a status message if something happened.
    pub fn check_external_changes(&mut self, lang: Lang) -> Option<String> {
        let path = self.path.clone()?;
        let mtime = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok())?;
        if Some(mtime) == self.disk_mtime {
            return None;
        }
        if self.dirty {
            // Don't clobber unsaved local edits; just remember the new mtime so we
            // don't keep re-triggering this message every tick.
            self.disk_mtime = Some(mtime);
            return Some(i18n::msg_externally_modified_kept(lang, &self.title(lang)));
        }
        let Ok(bytes) = std::fs::read(&path) else { return None };
        if bytes.contains(&0) {
            self.disk_mtime = Some(mtime);
            return None;
        }
        let Ok(content) = String::from_utf8(bytes) else {
            self.disk_mtime = Some(mtime);
            return None;
        };
        // Re-measured on every reload, both ways. A log or a generated file can cross the
        // large-file line while it sits open in a tab, and a buffer that kept the mode it was
        // born with would either colour a file it can no longer afford or stay grey after the
        // file was truncated back under it. Taken from the text that actually arrived rather
        // than from a second `metadata` call: it is the same number, already in hand.
        self.disk_len = content.len() as u64;
        self.line_ending = if content.contains("\r\n") { LineEnding::Crlf } else { LineEnding::Lf };
        self.final_newline = content.ends_with('\n');
        let arriving = content.replace("\r\n", "\n");
        // Worked out here because here is the last moment both texts exist: one line further
        // down the old rope is gone. Both sides are the normalized text, so a file that only
        // changed its line endings does not light up as though every line had been rewritten.
        let leaving = self.rope.to_string();
        self.changed_lines = changed_lines(&leaving, &arriving);
        self.rope = Rope::from_str(&arriving);
        self.disk_mtime = Some(mtime);
        self.syntax_dirty = true;
        self.revision = self.revision.wrapping_add(1);
        self.folds.clear();
        // And the server's boundaries with them, for the same reason: somebody else wrote this
        // file — an agent, a formatter, a branch switched underneath — and the block that started
        // on line forty may now start somewhere else. The buffer is clean, so nothing else would
        // stop them being believed; the revision has moved, so they are asked for again at once.
        self.server_folds.clear();
        // A silent reload starts a fresh edit timeline; the old history refers to gone text.
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.last_edit = EditKind::None;
        let max_line = self.rope.len_lines().saturating_sub(1);
        self.cursor_line = self.cursor_line.min(max_line);
        self.cursor_col = self.cursor_col.min(self.line_char_len(self.cursor_line));
        Some(i18n::msg_externally_reloaded(lang, &self.title(lang)))
    }

    /// Whether line `line` arrived in the last reload and has not been typed over since.
    pub fn line_arrived(&self, line: usize) -> bool {
        self.changed_lines.binary_search(&line).is_ok()
    }

    /// The lines the last reload brought in, ascending. The gutter asks one line at a time
    /// through `line_arrived`; this is for the tests, which are about the whole set.
    #[cfg(test)]
    pub fn arrived_lines(&self) -> &[usize] {
        &self.changed_lines
    }

    /// Puts the lights out. Esc asks for this directly; every edit does it through
    /// `mark_edited_from` without having to know the feature exists.
    pub fn forget_arrived_lines(&mut self) {
        self.changed_lines.clear();
    }

    pub fn title(&self, lang: Lang) -> String {
        let name = match &self.path {
            Some(p) => p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            None => i18n::t(lang, Key::UntitledFile).to_string(),
        };
        // A rendered view sits in the strip beside the very file it renders, under the same
        // name. The glyph is what tells the two apart at a glance, without reading.
        if self.is_rendered_view() { format!("\u{25a4} {name}") } else { name }
    }

    pub fn line_char_len(&self, line: usize) -> usize {
        if line >= self.rope.len_lines() {
            return 0;
        }
        let line_slice = self.rope.line(line);
        let mut len = line_slice.len_chars();
        // exclude trailing newline from column bounds
        if len > 0 {
            let last = line_slice.char(len - 1);
            if last == '\n' {
                len -= 1;
            }
        }
        len
    }

    /// Absolute char index of `(line, col)`, with the column held inside the line it names.
    ///
    /// The clamp is not decoration. A column selection keeps one column for every line it covers,
    /// and a line shorter than that column leaves the cursor sitting past its end — see
    /// `block_write_span`. Unclamped, the next edit at that position would land somewhere in the
    /// line *below*, or past the end of the rope, which ropey answers with a panic rather than
    /// forgiveness.
    fn char_idx(&self, line: usize, col: usize) -> usize {
        self.rope.line_to_char(line) + col.min(self.line_char_len(line))
    }

    /// Normalized (start, end) selection endpoints in document order, or None if empty/absent.
    pub fn selection_range(&self) -> Option<((usize, usize), (usize, usize))> {
        let anchor = self.selection_anchor?;
        let cursor = (self.cursor_line, self.cursor_col);
        if anchor == cursor {
            return None;
        }
        Some(if anchor <= cursor { (anchor, cursor) } else { (cursor, anchor) })
    }

    /// The rectangle a column selection covers: `(first line, last line, first column, last
    /// column)`, the columns exclusive at the end. `None` unless a column selection with actual
    /// width is in force.
    pub fn block_range(&self) -> Option<(usize, usize, usize, usize)> {
        if !self.selection_block {
            return None;
        }
        let (anchor_line, anchor_col) = self.selection_anchor?;
        let (line, col) = (self.cursor_line, self.cursor_col);
        let (c0, c1) = (anchor_col.min(col), anchor_col.max(col));
        (c0 != c1).then_some((anchor_line.min(line), anchor_line.max(line), c0, c1))
    }

    /// Which columns of `line` are selected, or `None` for none of them.
    ///
    /// The one place that decides what "selected" means, so the renderer, the copier and the
    /// deleter cannot drift apart — which is exactly how a rectangular selection would otherwise
    /// end up looking right and copying wrong. Columns past the end of a short line are clipped
    /// rather than padded: a rectangle drawn over ragged text selects only the text that is
    /// there.
    ///
    /// In column mode only `block_range` gets a say, including when it has nothing to say: the
    /// two endpoints of a rectangle with no width are also the ends of a run of text, and read
    /// that way they would shade — and copy, and cut — everything lying between them.
    pub fn selected_columns(&self, line: usize) -> Option<(usize, usize)> {
        let len = self.line_char_len(line);
        if self.selection_block {
            let (first, last, c0, c1) = self.block_range()?;
            if line < first || line > last {
                return None;
            }
            let (from, to) = (c0.min(len), c1.min(len));
            return (from < to).then_some((from, to));
        }
        let ((sl, sc), (el, ec)) = self.selection_range()?;
        if line < sl || line > el {
            return None;
        }
        Some((if line == sl { sc } else { 0 }, if line == el { ec } else { len }))
    }

    /// Where a keystroke in column mode writes: `(first line, last line, column)`.
    ///
    /// The same answer for both shapes a column selection can have, because typing treats them
    /// the same way: a rectangle with width loses its cells and the character takes their place
    /// at the left edge, and a rectangle with none — the state typing itself leaves behind — just
    /// gets the character at the column it stands on. `None` unless column mode is on with an
    /// anchor down.
    ///
    /// The column can sit past the end of a short line, which is the whole reason `char_idx`
    /// clamps: one column belongs to the block, not to any one of the lines under it.
    fn block_write_span(&self) -> Option<(usize, usize, usize)> {
        if !self.selection_block {
            return None;
        }
        let (anchor_line, anchor_col) = self.selection_anchor?;
        let (line, col) = (self.cursor_line, self.cursor_col);
        Some((anchor_line.min(line), anchor_line.max(line), anchor_col.min(col)))
    }

    /// The column to draw a caret on for `line`, for a column selection with no width.
    ///
    /// A rectangle with width shades itself through `selected_columns` and answers `None` here.
    /// One with no width has nothing to shade and everything to say: it is what typing in a block
    /// leaves standing, and the next key writes on every line it covers. Without a caret on each
    /// of them the user is editing N lines while seeing one cursor.
    ///
    /// Lines too short to reach the column get no caret, by the same clip-not-pad rule
    /// `selected_columns` states: they will not receive the character either.
    pub fn block_caret(&self, line: usize) -> Option<usize> {
        if self.block_range().is_some() {
            return None;
        }
        let (first, last, col) = self.block_write_span()?;
        if line < first || line > last {
            return None;
        }
        (self.line_char_len(line) >= col).then_some(col)
    }

    pub fn selected_text(&self) -> Option<String> {
        if self.selection_block {
            // As in `selected_columns`: in column mode a rectangle with no width is nothing
            // selected, not the run of text its two corners would otherwise describe.
            let (first, last, _, _) = self.block_range()?;
            // Each row of the rectangle becomes a line, so pasting it elsewhere reproduces the
            // shape — a short line contributes an empty one rather than being skipped.
            let rows: Vec<String> = (first..=last)
                .map(|line| match self.selected_columns(line) {
                    Some((from, to)) => {
                        let start = self.rope.line_to_char(line);
                        self.rope.slice(start + from..start + to).to_string()
                    }
                    None => String::new(),
                })
                .collect();
            return Some(rows.join("\n"));
        }
        let ((sl, sc), (el, ec)) = self.selection_range()?;
        let start_idx = self.rope.line_to_char(sl) + sc;
        let end_idx = self.rope.line_to_char(el) + ec;
        Some(self.rope.slice(start_idx..end_idx).to_string())
    }

    pub fn clear_selection(&mut self) {
        self.selection_anchor = None;
        self.selection_block = false;
        // The column a block held could be past the end of the line the cursor is on — see
        // `settle_column`. It belonged to the block, and the block is gone.
        self.cursor_col = self.cursor_col.min(self.line_char_len(self.cursor_line));
    }

    pub fn start_or_extend_selection(&mut self) {
        if self.selection_anchor.is_none() {
            self.selection_anchor = Some((self.cursor_line, self.cursor_col));
        }
    }

    // ---- Undo / redo ----------------------------------------------------------------

    fn snapshot(&self) -> Snapshot {
        Snapshot { text: self.rope.to_string(), cursor_line: self.cursor_line, cursor_col: self.cursor_col }
    }

    /// How many undo steps this buffer keeps: shallower in the declared large-file mode, where
    /// each step is a copy of a very large file.
    pub fn undo_depth(&self) -> usize {
        if self.is_large() { MAX_UNDO_LARGE } else { MAX_UNDO }
    }

    /// Records the pre-edit state for undo. Must be called BEFORE mutating the rope.
    /// Consecutive same-kind character edits coalesce into a single undo step (typing a
    /// word undoes as one), while `Other` edits always start a new step. No-op inside a
    /// compound edit, which checkpoints once up front.
    fn checkpoint(&mut self, kind: EditKind) {
        // Noted before the compound guard: a compound edit takes one checkpoint but marks the
        // buffer at every step inside it, and the earliest line is what re-highlighting needs.
        self.note_edit_line(self.cursor_line);
        if self.in_compound {
            return;
        }
        self.redo_stack.clear();
        let coalesce = kind != EditKind::Other && kind == self.last_edit && !self.undo_stack.is_empty();
        if !coalesce {
            // The history's whole cost is depth × file size, because a snapshot is the text.
            // See `MAX_UNDO`/`MAX_UNDO_LARGE` for the arithmetic that picks the two numbers.
            self.undo_stack.push_back(self.snapshot());
            if self.undo_stack.len() > self.undo_depth() {
                self.undo_stack.pop_front();
            }
        }
        self.last_edit = kind;
    }

    fn restore(&mut self, snap: Snapshot) {
        self.rope = Rope::from_str(&snap.text);
        let max_line = self.rope.len_lines().saturating_sub(1);
        self.cursor_line = snap.cursor_line.min(max_line);
        self.cursor_col = snap.cursor_col.min(self.line_char_len(self.cursor_line));
        // Both halves of the selection, not just the anchor: a rectangle left switched on
        // outlives the selection it belonged to, and the next Shift+arrow draws one.
        self.clear_selection();
        // A snapshot replaces the whole text, so no line of it can be assumed to have survived.
        self.mark_edited_from(0);
        self.folds.clear();
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo_stack.pop_back() else { return false };
        let current = self.snapshot();
        self.redo_stack.push(current);
        self.restore(prev);
        self.last_edit = EditKind::None;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo_stack.pop() else { return false };
        let current = self.snapshot();
        self.undo_stack.push_back(current);
        self.restore(next);
        self.last_edit = EditKind::None;
        true
    }

    /// Breaks undo coalescing so the next character edit starts a fresh undo step. Called
    /// after cursor movement, so "type, move, type" undoes in the two runs the user made.
    pub fn break_undo_coalescing(&mut self) {
        self.last_edit = EditKind::None;
    }

    // ---- Editing --------------------------------------------------------------------

    /// Deletes the active selection, if any, moving the cursor to its start. Returns
    /// whether a selection was actually deleted (callers use this to short-circuit).
    pub fn delete_selection(&mut self) -> bool {
        // A read-only buffer (binary/undecodable file, or a picture preview) has no business
        // accepting a keystroke's worth of edit; every mutating entry point below checks this
        // itself, since none of them route through a single shared chokepoint.
        if self.read_only {
            return false;
        }
        // A column selection with no width is dropped rather than deleted, so what follows sees
        // the buffer the way the ordinary keys do: Delete over one takes a character, not the
        // stretch of text between the block's two corners.
        if self.selection_block && self.block_range().is_none() {
            self.clear_selection();
        }
        if self.selection_range().is_none() && self.block_range().is_none() {
            return false;
        }
        self.checkpoint(EditKind::Other);
        self.delete_selection_raw()
    }

    /// Selection delete without its own undo checkpoint, for callers that already
    /// checkpointed (insert-over-selection, backspace-with-selection, …).
    fn delete_selection_raw(&mut self) -> bool {
        // A rectangle is cut out line by line, from the bottom up so the earlier lines keep the
        // indices they were measured at.
        if let Some((first, last, c0, _)) = self.block_range() {
            for line in (first..=last).rev() {
                if let Some((from, to)) = self.selected_columns(line) {
                    let start = self.rope.line_to_char(line);
                    self.rope.remove(start + from..start + to);
                }
            }
            self.cursor_line = first;
            self.cursor_col = c0;
            // Through `clear_selection`, which also brings the column back inside the line it
            // landed on: the block's left edge can be past the end of a short first line.
            self.clear_selection();
            self.mark_edited();
            return true;
        }
        // A column selection with no width has no cells to cut — but its two endpoints are still
        // a pair of positions, and read as a run of text they span every character between them.
        // Letting that fall through would make Enter in a three-line block swallow the two lines
        // under it. So the mode goes here, which is also what the callers want: each of them is
        // about to write a run of text at one place (a newline, a tab, a paste), and a rectangle
        // left standing over an edit that is not rectangular belongs to nothing.
        if self.selection_block {
            self.clear_selection();
        }
        let Some(((sl, sc), (el, ec))) = self.selection_range() else { return false };
        let start_idx = self.rope.line_to_char(sl) + sc;
        let end_idx = self.rope.line_to_char(el) + ec;
        self.rope.remove(start_idx..end_idx);
        self.cursor_line = sl;
        self.cursor_col = sc;
        self.selection_anchor = None;
        self.mark_edited();
        true
    }

    /// Line range an indent/outdent command should act on: the selection's line span,
    /// or just the current line when nothing is selected.
    fn indent_range(&self) -> (usize, usize) {
        if let Some(((sl, _), (el, ec))) = self.selection_range() {
            let end_line = if ec == 0 && el > sl { el.saturating_sub(1) } else { el };
            (sl, end_line)
        } else {
            (self.cursor_line, self.cursor_line)
        }
    }

    pub fn indent_selection(&mut self, tab_size: usize) {
        if self.read_only {
            return;
        }
        self.checkpoint(EditKind::Other);
        let (sl, end_line) = self.indent_range();
        let pad = " ".repeat(tab_size);
        for line in sl..=end_line {
            let idx = self.rope.line_to_char(line);
            self.rope.insert(idx, &pad);
        }
        // `indent_range` deliberately excludes a selection's last line when it ends at column
        // 0 (nothing on that line was indented), so the cursor or anchor sitting on that line
        // must not slide right along with the lines that actually gained a `pad`. And even on
        // an indented line, clamping to its new length keeps a stale wide column from landing
        // past end-of-line, which is what let the next edit index past `len_chars()`.
        if (sl..=end_line).contains(&self.cursor_line) {
            self.cursor_col = (self.cursor_col + tab_size).min(self.line_char_len(self.cursor_line));
        }
        if let Some((al, ac)) = self.selection_anchor {
            if (sl..=end_line).contains(&al) {
                let new_ac = (ac + tab_size).min(self.line_char_len(al));
                self.selection_anchor = Some((al, new_ac));
            }
        }
        // A selection can be dragged upwards, leaving the cursor on the last line of a range
        // that starts well above it — so the range says where the change starts, not the cursor.
        self.mark_edited_from(sl);
    }

    pub fn outdent_selection(&mut self, tab_size: usize) {
        if self.read_only {
            return;
        }
        self.checkpoint(EditKind::Other);
        let (sl, end_line) = self.indent_range();
        for line in sl..=end_line {
            let start = self.rope.line_to_char(line);
            let line_slice = self.rope.line(line);
            let mut remove = 0usize;
            for ch in line_slice.chars().take(tab_size) {
                if ch == ' ' {
                    remove += 1;
                } else if ch == '\t' {
                    remove += 1;
                    break;
                } else {
                    break;
                }
            }
            if remove > 0 {
                self.rope.remove(start..start + remove);
            }
        }
        let cursor_len = self.line_char_len(self.cursor_line);
        self.cursor_col = self.cursor_col.min(cursor_len);
        let anchor_len = self.selection_anchor.map(|(al, _)| self.line_char_len(al));
        if let (Some((_, ac)), Some(len)) = (self.selection_anchor.as_mut(), anchor_len) {
            *ac = (*ac).min(len);
        }
        self.mark_edited_from(sl);
    }

    pub fn insert_char(&mut self, ch: char) {
        if self.read_only {
            return;
        }
        self.checkpoint(EditKind::Insert);
        self.delete_selection_raw();
        let idx = self.char_idx(self.cursor_line, self.cursor_col);
        self.rope.insert_char(idx, ch);
        self.cursor_col += 1;
        self.mark_edited();
    }

    /// Types `ch` into every line of the active column selection, and leaves the block standing
    /// one column to the right so the next key does it again.
    ///
    /// This is the multi-cursor the roadmap asked for in the shape the editor already half had:
    /// one cursor and one anchor describing a column, rather than a list of independent carets.
    /// A rectangle with width loses its cells first — the character replaces what was selected,
    /// exactly as typing over an ordinary selection does.
    ///
    /// Lines shorter than the column are skipped, not padded out with spaces to reach it. That is
    /// the rule `selected_columns` states for reading a rectangle, and it has to be the same rule
    /// for writing one: a block dragged across ragged text would otherwise fill the short lines
    /// with trailing whitespace the user never typed, on every keystroke.
    ///
    /// One checkpoint for the whole column, so a keystroke costs one snapshot and one Ctrl+Z
    /// takes back all of the lines it wrote — N nested inserts would push N copies of the file.
    fn insert_char_block(&mut self, ch: char) {
        let Some((first, last, col)) = self.block_write_span() else { return };
        // Nothing reaches the column on any line: no edit, and so no checkpoint either — one
        // would throw away a redo the user had not touched. Deleting the rectangle first cannot
        // change this answer, since a line long enough to lose cells at `col` is still long
        // enough to reach `col` afterwards.
        if !(first..=last).any(|line| self.line_char_len(line) >= col) {
            return;
        }
        self.checkpoint(EditKind::Other);
        self.in_compound = true;
        if self.block_range().is_some() {
            // Cuts the rectangle and clears the mode with it; the block is put back below, now
            // with no width, which is what the caret column left behind is.
            self.delete_selection_raw();
        }
        // Bottom-up, so the lines above keep the offsets they were measured at.
        for line in (first..=last).rev() {
            if self.line_char_len(line) >= col {
                let idx = self.rope.line_to_char(line) + col;
                self.rope.insert_char(idx, ch);
            }
        }
        self.in_compound = false;
        self.last_edit = EditKind::Other;
        self.selection_block = true;
        self.selection_anchor = Some((first, col + 1));
        self.cursor_line = last;
        self.cursor_col = col + 1;
        self.mark_edited_from(first);
    }

    fn char_before_cursor(&self) -> Option<char> {
        let idx = self.cursor_char_idx();
        if idx == 0 {
            None
        } else {
            self.rope.get_char(idx - 1)
        }
    }

    fn char_at_cursor(&self) -> Option<char> {
        self.rope.get_char(self.cursor_char_idx())
    }

    /// Inserts `ch` with bracket/quote auto-pairing when `auto_pairs` is on: an opening
    /// bracket inserts its partner and leaves the cursor between them; typing a closing
    /// bracket right before the matching one steps over it instead of inserting a duplicate.
    pub fn insert_char_pairs(&mut self, ch: char, auto_pairs: bool) {
        if self.read_only {
            return;
        }
        // A column selection writes on every line it covers, and it does it without pairing:
        // one `(` typed into a block of eight lines is eight brackets and eight closers to step
        // back over, which is not what anybody meant by typing one character.
        if self.selection_block && self.selection_anchor.is_some() {
            self.insert_char_block(ch);
            return;
        }
        if !auto_pairs || self.selection_range().is_some() {
            self.insert_char(ch);
            return;
        }
        // Step over an auto-inserted closer instead of typing a second one.
        if is_closer(ch) && self.char_at_cursor() == Some(ch) {
            self.move_right();
            return;
        }
        if let Some(close) = close_partner(ch) {
            // For quotes, skip pairing next to a word (apostrophes, string suffixes, …).
            let is_quote = matches!(ch, '"' | '\'' | '`');
            let touches_word = self.char_before_cursor().map(|c| c.is_alphanumeric()).unwrap_or(false)
                || self.char_at_cursor().map(|c| c.is_alphanumeric()).unwrap_or(false);
            if is_quote && touches_word {
                self.insert_char(ch);
                return;
            }
            self.checkpoint(EditKind::Other);
            let idx = self.cursor_char_idx();
            let pair: String = [ch, close].iter().collect();
            self.rope.insert(idx, &pair);
            self.cursor_col += 1; // land between the pair
            self.mark_edited();
            return;
        }
        self.insert_char(ch);
    }

    /// Inserts a newline, expanding an empty bracket pair the cursor sits inside into a
    /// three-line block with the middle line indented by `indent_unit` (like most editors
    /// do when you press Enter between `{` and `}`).
    pub fn newline_smart(&mut self, auto_indent: bool, auto_pairs: bool, indent_unit: &str) {
        if self.read_only {
            return;
        }
        if auto_pairs {
            if let (Some(open), Some(close)) = (self.char_before_cursor(), self.char_at_cursor()) {
                if close_partner(open) == Some(close) && open != '"' && open != '\'' && open != '`' {
                    let base: String = self
                        .rope
                        .line(self.cursor_line)
                        .chars()
                        .take_while(|c| *c == ' ' || *c == '\t')
                        .collect();
                    let mid = format!("{base}{indent_unit}");
                    let insertion = format!("\n{mid}\n{base}");
                    self.checkpoint(EditKind::Other);
                    let idx = self.cursor_char_idx();
                    self.rope.insert(idx, &insertion);
                    self.set_cursor_char_idx(idx + 1 + mid.chars().count());
                    self.mark_edited();
                    return;
                }
            }
        }
        self.insert_newline(auto_indent);
    }

    /// Inserts a run of text with no newlines (used for space-expanded tabs).
    pub fn insert_str(&mut self, s: &str) {
        if self.read_only {
            return;
        }
        self.checkpoint(EditKind::Insert);
        self.delete_selection_raw();
        let idx = self.char_idx(self.cursor_line, self.cursor_col);
        self.rope.insert(idx, s);
        self.cursor_col += s.chars().count();
        self.mark_edited();
    }

    /// Inserts possibly multi-line text (e.g. a clipboard paste), splitting on '\n'. The
    /// whole paste is one undo step (nested inserts skip their own checkpoint).
    pub fn insert_multiline(&mut self, text: &str) {
        if self.read_only {
            return;
        }
        // The one door every paste comes through, and the buffer behind it holds '\n' and
        // nothing else — the file's own ending is put back on the way out to disk. Text copied
        // from a Windows program still has its carriage returns, and splitting that on '\n'
        // alone leaves one at the end of every line: invisible on screen, and saved as `\r\r\n`.
        let text = normalize_newlines(text);
        self.checkpoint(EditKind::Other);
        self.in_compound = true;
        self.delete_selection_raw();
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                self.insert_newline(false);
            }
            if !line.is_empty() {
                self.insert_str(line);
            }
        }
        self.in_compound = false;
        self.last_edit = EditKind::Other;
    }

    pub fn insert_newline(&mut self, auto_indent: bool) {
        if self.read_only {
            return;
        }
        self.checkpoint(EditKind::Other);
        self.delete_selection_raw();
        let indent = if auto_indent {
            let line = self.rope.line(self.cursor_line);
            line.chars().take_while(|c| *c == ' ' || *c == '\t').collect::<String>()
        } else {
            String::new()
        };
        let idx = self.char_idx(self.cursor_line, self.cursor_col);
        self.rope.insert_char(idx, '\n');
        self.cursor_line += 1;
        self.cursor_col = 0;
        if !indent.is_empty() {
            let insert_idx = self.char_idx(self.cursor_line, 0);
            self.rope.insert(insert_idx, &indent);
            self.cursor_col = indent.chars().count();
        }
        self.mark_edited();
    }

    /// Backspace over an active column selection: the counterpart of `insert_char_block`.
    ///
    /// A rectangle with width loses its cells, exactly as it always has; what is new is what it
    /// leaves behind — the block, now with no width, standing on the same lines at the left edge,
    /// so a Backspace and a keystroke go on meaning the same column. A rectangle with no width
    /// takes the character in front of the column off every line that has one, and steps the
    /// whole block back with it.
    ///
    /// Answers whether it handled the key, so `backspace` can carry on with the ordinary one.
    fn backspace_block(&mut self) -> bool {
        let Some((first, last, col)) = self.block_write_span() else { return false };
        if self.block_range().is_some() {
            self.checkpoint(EditKind::Other);
            self.in_compound = true;
            self.delete_selection_raw();
            self.in_compound = false;
            self.last_edit = EditKind::Other;
            self.selection_block = true;
            self.selection_anchor = Some((first, col));
            self.cursor_line = last;
            self.cursor_col = col;
            self.mark_edited_from(first);
            return true;
        }
        // At column zero there is nothing in front to take. The key is still the block's — it
        // must not fall through and start joining lines together, which is what a Backspace at
        // the start of a line does.
        if col == 0 {
            return true;
        }
        // A line reaching the column has a character at `col - 1`; a shorter one has nothing
        // under the block at all, and is left alone. Same rule, same reason, as writing.
        let lines: Vec<usize> = (first..=last).filter(|&line| self.line_char_len(line) >= col).collect();
        if lines.is_empty() {
            return true;
        }
        self.checkpoint(EditKind::Other);
        self.in_compound = true;
        for &line in lines.iter().rev() {
            let idx = self.rope.line_to_char(line) + col;
            self.rope.remove(idx - 1..idx);
        }
        self.in_compound = false;
        self.last_edit = EditKind::Other;
        self.selection_anchor = Some((first, col - 1));
        self.cursor_line = last;
        self.cursor_col = col - 1;
        self.mark_edited_from(first);
        true
    }

    pub fn backspace(&mut self) {
        if self.read_only {
            return;
        }
        if self.selection_block && self.selection_anchor.is_some() && self.backspace_block() {
            return;
        }
        if self.delete_selection() {
            return;
        }
        // Delete an empty bracket/quote pair as a unit: backspacing between `(` and `)`
        // removes both, undoing the auto-pair in one press.
        if let (Some(open), Some(close)) = (self.char_before_cursor(), self.char_at_cursor()) {
            if close_partner(open) == Some(close) {
                self.checkpoint(EditKind::Delete);
                let idx = self.cursor_char_idx();
                self.rope.remove(idx - 1..idx + 1);
                self.cursor_col -= 1;
                self.mark_edited();
                return;
            }
        }
        // The checkpoint is taken inside the branches that actually delete something. Taken up
        // front it also fired at the very start of the buffer, where Backspace does nothing at
        // all — and a checkpoint discards the redo stack, so a stray press at (0, 0) threw away
        // a redo the user had not touched.
        if self.cursor_col > 0 {
            self.checkpoint(EditKind::Delete);
            let idx = self.char_idx(self.cursor_line, self.cursor_col);
            self.rope.remove(idx - 1..idx);
            self.cursor_col -= 1;
            self.mark_edited();
        } else if self.cursor_line > 0 {
            self.checkpoint(EditKind::Delete);
            let prev_len = self.line_char_len(self.cursor_line - 1);
            let idx = self.char_idx(self.cursor_line, 0);
            self.rope.remove(idx - 1..idx);
            self.cursor_line -= 1;
            self.cursor_col = prev_len;
            self.mark_edited();
        }
    }

    pub fn delete_forward(&mut self) {
        if self.read_only {
            return;
        }
        if self.delete_selection() {
            return;
        }
        let idx = self.char_idx(self.cursor_line, self.cursor_col);
        // As with Backspace: nothing to delete at the end of the buffer, so nothing to record.
        if idx < self.rope.len_chars() {
            self.checkpoint(EditKind::Delete);
            self.rope.remove(idx..idx + 1);
            self.mark_edited();
        }
    }

    /// Whether `line` is currently hidden inside a collapsed fold (i.e. after its start).
    fn is_hidden(&self, line: usize) -> bool {
        self.folds.iter().any(|&(s, e)| line > s && line <= e)
    }

    /// If the cursor ended up inside a collapsed region, snap it back to that fold's
    /// visible start line so it never becomes invisible.
    fn clamp_out_of_folds(&mut self) {
        if let Some(&(s, _)) = self.folds.iter().find(|&&(s, e)| self.cursor_line > s && self.cursor_line <= e) {
            self.cursor_line = s;
            self.cursor_col = self.cursor_col.min(self.line_char_len(s));
        }
    }

    /// Determines the foldable range starting at `line`, if any.
    ///
    /// The server is asked first, in the sense that its answer is already in hand: `server_folds`
    /// holds what `textDocument/foldingRange` said about this file, and a range that begins on this
    /// very line is a boundary drawn by something that has parsed the language — the `impl` block
    /// whose brace is three lines down, the Python `def` whose body ends before the decorator, the
    /// import group no counting of braces would ever find. Where several begin here, the widest
    /// wins: that is the block the reader means by "this one", the way a fold marker on a function
    /// signature means the function and not its first statement.
    ///
    /// Where the server says nothing about this line — because it never answered, because it does
    /// not offer the request, or simply because nothing starts here — the rule this editor has
    /// always used stands unchanged underneath: a brace block (`{` ... matching `}`), or failing
    /// that an indentation block (Python-style: the following more-indented lines).
    ///
    /// And a *dirty* buffer never consults the server at all. The cached ranges are line numbers
    /// taken when the buffer and the server last agreed, and one typed newline makes every number
    /// below it a lie — so an edited file folds by the heuristic until the next save refreshes the
    /// cache. Falling back is the honest failure here: the braces are computed from the text on
    /// screen and cannot be stale.
    pub fn foldable_range_at(&self, line: usize) -> Option<(usize, usize)> {
        let total = self.rope.len_lines();
        if line >= total {
            return None;
        }
        if !self.dirty {
            let widest = self
                .server_folds
                .iter()
                .filter(|(start, end)| *start == line && *end < total)
                .max_by_key(|(_, end)| *end);
            if let Some(&range) = widest {
                return Some(range);
            }
        }
        let text = self.rope.line(line).to_string();
        if text.trim().is_empty() {
            return None;
        }
        let trimmed_end = text.trim_end();

        if trimmed_end.ends_with('{') {
            let mut depth = 1i32;
            let mut l = line + 1;
            while l < total {
                let line_text = self.rope.line(l).to_string();
                for ch in line_text.chars() {
                    match ch {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                return if l > line { Some((line, l)) } else { None };
                            }
                        }
                        _ => {}
                    }
                }
                l += 1;
            }
            return None;
        }

        let indent = text.chars().take_while(|c| *c == ' ' || *c == '\t').count();
        let mut l = line + 1;
        while l < total {
            let next = self.rope.line(l).to_string();
            if next.trim().is_empty() {
                l += 1;
                continue;
            }
            let next_indent = next.chars().take_while(|c| *c == ' ' || *c == '\t').count();
            if next_indent <= indent {
                return None;
            }
            let mut end = l;
            let mut m = l + 1;
            while m < total {
                let t = self.rope.line(m).to_string();
                if t.trim().is_empty() {
                    m += 1;
                    continue;
                }
                let ind = t.chars().take_while(|c| *c == ' ' || *c == '\t').count();
                if ind > indent {
                    end = m;
                    m += 1;
                } else {
                    break;
                }
            }
            return Some((line, end));
        }
        None
    }

    /// Toggles the fold at the cursor's line: collapses it if foldable and not already
    /// folded, or expands it if the cursor sits on an active fold's start line.
    pub fn toggle_fold(&mut self) {
        let line = self.cursor_line;
        if let Some(pos) = self.folds.iter().position(|&(s, _)| s == line) {
            self.folds.remove(pos);
            return;
        }
        if self.is_hidden(line) {
            return;
        }
        if let Some(range) = self.foldable_range_at(line) {
            self.folds.push(range);
            self.folds.sort_by_key(|&(s, _)| s);
        }
    }

    /// Walks forward from `start_line`, skipping lines hidden inside collapsed folds,
    /// yielding up to `max_rows` buffer line indices in the order they'd be rendered.
    pub fn visible_rows_from(&self, start_line: usize, max_rows: usize) -> Vec<usize> {
        let mut rows = Vec::new();
        let mut line = start_line;
        let total = self.rope.len_lines();
        while line < total && rows.len() < max_rows {
            rows.push(line);
            if let Some(&(_, end)) = self.folds.iter().find(|&&(s, _)| s == line) {
                line = end + 1;
            } else {
                line += 1;
            }
        }
        rows
    }

    pub fn move_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.line_char_len(self.cursor_line);
        }
        self.clamp_out_of_folds();
    }

    pub fn move_right(&mut self) {
        let len = self.line_char_len(self.cursor_line);
        if self.cursor_col < len {
            self.cursor_col += 1;
        } else if self.cursor_line + 1 < self.rope.len_lines() {
            self.cursor_line += 1;
            self.cursor_col = 0;
        }
        self.clamp_out_of_folds();
    }

    /// Settles the column after a vertical move: back inside the line the cursor landed on —
    /// unless a column selection is up, which owns the column for as long as it lasts.
    ///
    /// Without the exception a rectangle dragged down through ragged text collapses onto the
    /// width of the shortest line it passes, and never widens again: there is no goal column to
    /// widen back to, only the one the clamp left. It would also make the clip-not-pad rule of
    /// `selected_columns` unreachable from the keyboard, since no block could ever hold a column
    /// past the end of one of its own lines. Everything that reads the cursor while a block is up
    /// clips for itself — `selected_columns`, `insert_char_block`, `char_idx` — and
    /// `clear_selection` brings the cursor back inside its line on the way out of the mode.
    fn settle_column(&mut self) {
        if !self.selection_block {
            self.cursor_col = self.cursor_col.min(self.line_char_len(self.cursor_line));
        }
    }

    pub fn move_up(&mut self) {
        if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.settle_column();
        }
        self.clamp_out_of_folds();
    }

    pub fn move_down(&mut self) {
        if self.cursor_line + 1 < self.rope.len_lines() {
            self.cursor_line += 1;
            self.settle_column();
        }
        self.clamp_out_of_folds();
    }

    pub fn move_home(&mut self) {
        self.cursor_col = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor_col = self.line_char_len(self.cursor_line);
    }

    pub fn page_up(&mut self, page: usize) {
        self.cursor_line = self.cursor_line.saturating_sub(page);
        self.settle_column();
        self.clamp_out_of_folds();
    }

    pub fn page_down(&mut self, page: usize) {
        let max_line = self.rope.len_lines().saturating_sub(1);
        self.cursor_line = (self.cursor_line + page).min(max_line);
        self.settle_column();
        self.clamp_out_of_folds();
    }

    // ---- Word-wise motion & deletion ------------------------------------------------

    fn cursor_char_idx(&self) -> usize {
        self.char_idx(self.cursor_line, self.cursor_col)
    }

    fn set_cursor_char_idx(&mut self, idx: usize) {
        let idx = idx.min(self.rope.len_chars());
        let line = self.rope.char_to_line(idx);
        self.cursor_line = line;
        self.cursor_col = (idx - self.rope.line_to_char(line)).min(self.line_char_len(line));
    }

    /// Absolute char index of the previous word boundary before `from`.
    fn word_left_idx(&self, from: usize) -> usize {
        let mut idx = from;
        while idx > 0 && self.rope.char(idx - 1).is_whitespace() {
            idx -= 1;
        }
        if idx > 0 {
            let class = word_class(self.rope.char(idx - 1));
            while idx > 0 {
                let c = self.rope.char(idx - 1);
                if c.is_whitespace() || word_class(c) != class {
                    break;
                }
                idx -= 1;
            }
        }
        idx
    }

    /// Absolute char index of the next word boundary at or after `from`.
    fn word_right_idx(&self, from: usize) -> usize {
        let total = self.rope.len_chars();
        let mut idx = from;
        while idx < total && self.rope.char(idx).is_whitespace() {
            idx += 1;
        }
        if idx < total {
            let class = word_class(self.rope.char(idx));
            while idx < total {
                let c = self.rope.char(idx);
                if c.is_whitespace() || word_class(c) != class {
                    break;
                }
                idx += 1;
            }
        }
        idx
    }

    pub fn move_word_left(&mut self) {
        let target = self.word_left_idx(self.cursor_char_idx());
        self.set_cursor_char_idx(target);
        self.clamp_out_of_folds();
    }

    pub fn move_word_right(&mut self) {
        let target = self.word_right_idx(self.cursor_char_idx());
        self.set_cursor_char_idx(target);
        self.clamp_out_of_folds();
    }

    pub fn delete_word_left(&mut self) {
        if self.read_only {
            return;
        }
        if self.delete_selection() {
            return;
        }
        let end = self.cursor_char_idx();
        let start = self.word_left_idx(end);
        if start < end {
            self.checkpoint(EditKind::Delete);
            self.rope.remove(start..end);
            self.set_cursor_char_idx(start);
            self.mark_edited();
        }
    }

    pub fn delete_word_right(&mut self) {
        if self.read_only {
            return;
        }
        if self.delete_selection() {
            return;
        }
        let start = self.cursor_char_idx();
        let end = self.word_right_idx(start);
        if start < end {
            self.checkpoint(EditKind::Delete);
            self.rope.remove(start..end);
            self.mark_edited();
        }
    }

    // ---- Line operations ------------------------------------------------------------

    /// Duplicates the current line onto the line below, keeping the cursor column.
    pub fn duplicate_line(&mut self) {
        if self.read_only {
            return;
        }
        self.checkpoint(EditKind::Other);
        let line = self.cursor_line;
        let line_start = self.rope.line_to_char(line);
        let line_end = if line + 1 < self.rope.len_lines() {
            self.rope.line_to_char(line + 1)
        } else {
            self.rope.len_chars()
        };
        let content = self.rope.slice(line_start..line_end).to_string();
        if content.ends_with('\n') {
            self.rope.insert(line_end, &content);
        } else {
            // Last line with no trailing newline: add the separator between the two copies.
            self.rope.insert(line_end, &format!("\n{content}"));
        }
        self.cursor_line += 1;
        self.mark_edited();
    }

    /// Swaps lines `a` and `a+1` (`b`), preserving whether the block ends with a newline.
    fn swap_adjacent_lines(&mut self, a: usize, b: usize) {
        let a_start = self.rope.line_to_char(a);
        let b_start = self.rope.line_to_char(b);
        let b_end = if b + 1 < self.rope.len_lines() {
            self.rope.line_to_char(b + 1)
        } else {
            self.rope.len_chars()
        };
        let a_text = self.rope.slice(a_start..b_start).to_string();
        let b_text = self.rope.slice(b_start..b_end).to_string();
        let a_line = a_text.strip_suffix('\n').unwrap_or(&a_text);
        let (b_line, b_had_nl) = match b_text.strip_suffix('\n') {
            Some(s) => (s, true),
            None => (b_text.as_str(), false),
        };
        let new_text = if b_had_nl {
            format!("{b_line}\n{a_line}\n")
        } else {
            format!("{b_line}\n{a_line}")
        };
        self.rope.remove(a_start..b_end);
        self.rope.insert(a_start, &new_text);
    }

    pub fn move_line_up(&mut self) {
        if self.read_only || self.cursor_line == 0 {
            return;
        }
        self.checkpoint(EditKind::Other);
        let line = self.cursor_line;
        self.swap_adjacent_lines(line - 1, line);
        self.cursor_line -= 1;
        self.mark_edited();
    }

    pub fn move_line_down(&mut self) {
        let line = self.cursor_line;
        if self.read_only || line + 1 >= self.rope.len_lines() {
            return;
        }
        self.checkpoint(EditKind::Other);
        self.swap_adjacent_lines(line, line + 1);
        self.cursor_line += 1;
        self.mark_edited();
    }

    /// Toggles a line comment (`token`) on the current line or every line in the selection:
    /// uncomments if all non-blank lines are already commented, otherwise comments them.
    pub fn toggle_comment(&mut self, token: &str) {
        if self.read_only {
            return;
        }
        let (sl, el) = self.indent_range();
        let mut any = false;
        let mut all_commented = true;
        for line in sl..=el {
            let text = self.rope.line(line).to_string();
            if text.trim().is_empty() {
                continue;
            }
            any = true;
            if !text.trim_start().starts_with(token) {
                all_commented = false;
            }
        }
        if !any {
            return;
        }
        self.checkpoint(EditKind::Other);
        let token_chars = token.chars().count();
        for line in sl..=el {
            let text = self.rope.line(line).to_string();
            if text.trim().is_empty() {
                continue;
            }
            let indent = text.chars().take_while(|c| *c == ' ' || *c == '\t').count();
            let at = self.rope.line_to_char(line) + indent;
            if all_commented {
                self.rope.remove(at..at + token_chars);
                if at < self.rope.len_chars() && self.rope.char(at) == ' ' {
                    self.rope.remove(at..at + 1);
                }
            } else {
                self.rope.insert(at, &format!("{token} "));
            }
        }
        self.cursor_col = self.cursor_col.min(self.line_char_len(self.cursor_line));
        self.mark_edited_from(sl);
    }

    // ---- Markdown formatting --------------------------------------------------------
    //
    // Semantic toggles rather than blind insertion of syntax: the button that made a word bold
    // makes it plain again, which is the only behaviour that survives being pressed twice. Each
    // one is a single undo step — a checkpoint up front and direct surgery on the rope after it,
    // the way `toggle_comment` works — and each answers whether it changed anything, so the
    // caller can say so rather than leaving a click that did nothing unexplained.

    /// The two states in which none of these can act: a buffer that refuses every edit, and a
    /// rectangular selection, which is neither a run of text to wrap nor a span of lines to
    /// prefix. Checked first everywhere, so nothing below it ever checkpoints for nothing.
    fn md_editable(&self) -> bool {
        !self.read_only && !self.selection_block
    }

    /// Keeps the cursor and the selection's other end inside the lines they sit on.
    ///
    /// Called after every one of these edits, for the same reason `indent_selection` clamps: a
    /// column left over from a longer line is an index past `len_chars()` waiting for the next
    /// keystroke, and ropey asserts rather than forgiving one.
    fn md_clamp_ends(&mut self) {
        self.cursor_line = self.cursor_line.min(self.rope.len_lines().saturating_sub(1));
        self.cursor_col = self.cursor_col.min(self.line_char_len(self.cursor_line));
        if let Some((al, ac)) = self.selection_anchor {
            let al = al.min(self.rope.len_lines().saturating_sub(1));
            self.selection_anchor = Some((al, ac.min(self.line_char_len(al))));
        }
    }

    /// How many `c` in a row end at `idx`, reading backwards.
    fn md_run_before(&self, idx: usize, c: char) -> usize {
        let mut n = 0;
        while idx > n && self.rope.get_char(idx - n - 1) == Some(c) {
            n += 1;
        }
        n
    }

    /// How many `c` in a row start at `idx`, reading forwards.
    fn md_run_after(&self, idx: usize, c: char) -> usize {
        let mut n = 0;
        while self.rope.get_char(idx + n) == Some(c) {
            n += 1;
        }
        n
    }

    /// Whether the document reads exactly `s` starting at `at`.
    fn md_reads(&self, at: usize, s: &str) -> bool {
        let n = s.chars().count();
        at + n <= self.rope.len_chars() && self.rope.slice(at..at + n) == s
    }

    /// The word the cursor is on or against, as absolute char indices.
    ///
    /// Adjacency counts on both sides: pressing bold with the caret just after `word` is asking
    /// about that word, not about the empty space at the caret. The same reading is what "the
    /// name under the cursor" means to a rename — a caret sitting at the end of an identifier is
    /// pointing at it — which is why there is one of these rather than one per caller.
    ///
    /// A "word" here is a run of alphanumerics and underscores, which is an identifier in every
    /// language CleeCode can start a server for. It is not the language's own idea of one — `-`
    /// in Lisp, `$` in shell, `!` after a Rust macro — and it does not need to be: the server is
    /// asked about a *position*, and this only decides what the box is prefilled with.
    pub fn word_at_cursor(&self) -> Option<(usize, usize)> {
        let line_start = self.rope.line_to_char(self.cursor_line);
        let len = self.line_char_len(self.cursor_line);
        let col = self.cursor_col.min(len);
        let is_word = |i: usize| {
            self.rope.get_char(line_start + i).is_some_and(|c| c.is_alphanumeric() || c == '_')
        };
        let mut start = col;
        while start > 0 && is_word(start - 1) {
            start -= 1;
        }
        let mut end = col;
        while end < len && is_word(end) {
            end += 1;
        }
        (start < end).then(|| (line_start + start, line_start + end))
    }

    /// Wraps or unwraps a run of text in an inline marker: `**` bold, `*` italic, `` ` `` code,
    /// `~~` strike.
    ///
    /// Presence is read off the markers already around the text — outside the selection's edges
    /// first, then as its own first and last characters, so selecting `x` inside `**x**` and
    /// selecting `**x**` whole both mean the same thing. For the `*` family it is the length of
    /// the run that decides, not a string match: `**x**` is bold and not italic, `***x***` is
    /// both, and asking for italic over bold has to add one star per side rather than find none
    /// and add two more.
    ///
    /// The selection is left on the text without its markers, so the same button pressed again
    /// undoes what it just did. A multi-line selection is refused: the markers would land inside
    /// paragraphs they do not belong to.
    pub fn md_toggle_inline(&mut self, marker: &str) -> bool {
        if !self.md_editable() || marker.is_empty() {
            return false;
        }
        let unit = marker.chars().count();
        let (start, end) = match self.selection_range() {
            Some(((sl, sc), (el, ec))) => {
                if sl != el {
                    return false;
                }
                (self.rope.line_to_char(sl) + sc, self.rope.line_to_char(el) + ec)
            }
            None => match self.word_at_cursor() {
                Some(range) => range,
                // Nothing to wrap: leave the pair behind with the caret between its halves, the
                // way typing an opening bracket does.
                None => {
                    self.checkpoint(EditKind::Other);
                    let idx = self.cursor_char_idx().min(self.rope.len_chars());
                    self.rope.insert(idx, &format!("{marker}{marker}"));
                    self.cursor_col += unit;
                    self.clear_selection();
                    self.md_clamp_ends();
                    self.mark_edited();
                    return true;
                }
            },
        };
        let inner_len = end.saturating_sub(start);
        // The `*` family counts stars; the other two match their marker literally.
        let star = marker.starts_with('*');
        let (outside, inside) = if star {
            let present = |run: usize| if unit == 1 { run % 2 == 1 } else { run >= 2 };
            let outer = self.md_run_before(start, '*').min(self.md_run_after(end, '*'));
            let inner = self
                .md_run_after(start, '*')
                .min(self.md_run_before(end, '*'))
                .min(inner_len / 2);
            (present(outer), present(inner))
        } else {
            let outer = start >= unit
                && self.md_reads(start - unit, marker)
                && self.md_reads(end, marker);
            let inner = inner_len >= 2 * unit
                && self.md_reads(start, marker)
                && self.md_reads(end - unit, marker);
            (outer, inner)
        };

        self.checkpoint(EditKind::Other);
        // Both edges are cut or grown, and the far one goes first so the near one's index is
        // still the index it was measured at.
        let (inner_start, inner_end) = if outside {
            self.rope.remove(end..end + unit);
            self.rope.remove(start - unit..start);
            (start - unit, end - unit)
        } else if inside {
            self.rope.remove(end - unit..end);
            self.rope.remove(start..start + unit);
            (start, end - 2 * unit)
        } else {
            self.rope.insert(end, marker);
            self.rope.insert(start, marker);
            (start + unit, end + unit)
        };
        let first_line = self.rope.char_to_line(inner_start.min(self.rope.len_chars()));
        self.select_char_range(inner_start, inner_end);
        self.md_clamp_ends();
        self.mark_edited_from(first_line);
        true
    }

    /// The shared shape of every line-prefix toggle.
    ///
    /// `carried` reads a line already stripped of its indentation and answers how many of its
    /// characters are this kind of prefix — `Some(0)` for a line that counts as having one and is
    /// never to be touched, which is how a checkbox stays a checkbox under the bullet button.
    /// `to_add` is asked, for a line that has none, where past the indentation to write and what;
    /// its first argument is the line's position among the non-blank lines of the span, which is
    /// what numbers a numbered list.
    ///
    /// All-or-none, exactly as `toggle_comment`: a span whose non-blank lines all carry the
    /// prefix loses it, and any other span gains it on the lines that lack it. Blank lines are
    /// skipped either way — except when the span is a single blank line, which is how a list is
    /// started on an empty one.
    fn md_line_prefix(
        &mut self,
        carried: &dyn Fn(&str) -> Option<usize>,
        to_add: &dyn Fn(usize, &str) -> (usize, String),
    ) -> bool {
        if !self.md_editable() {
            return false;
        }
        let (sl, el) = self.indent_range();
        let last = self.rope.len_lines().saturating_sub(1);
        if sl > last {
            return false;
        }
        let el = el.min(last);
        let mut any = false;
        let mut all = true;
        for line in sl..=el {
            let text = self.rope.line(line).to_string();
            if text.trim().is_empty() {
                continue;
            }
            any = true;
            if carried(text.trim_start()).is_none() {
                all = false;
            }
        }
        let blank_only = !any && sl == el;
        if !any && !blank_only {
            return false;
        }
        // A lone blank line has nothing that could already carry the prefix, so it can only be
        // the adding direction.
        let all = any && all;
        self.checkpoint(EditKind::Other);
        let mut nth = 0usize;
        for line in sl..=el {
            let raw = self.rope.line(line).to_string();
            let body = raw.trim_end_matches(['\n', '\r']);
            if body.trim().is_empty() && !blank_only {
                continue;
            }
            let indent = body.chars().take_while(|c| *c == ' ' || *c == '\t').count();
            let rest: String = body.chars().skip(indent).collect();
            let base = self.rope.line_to_char(line) + indent;
            match (all, carried(&rest)) {
                (true, Some(n)) if n > 0 => {
                    let end = (base + n).min(self.rope.len_chars());
                    if end > base {
                        self.rope.remove(base..end);
                    }
                }
                (false, None) => {
                    let (at, text) = to_add(nth, &rest);
                    let at = (base + at).min(self.rope.len_chars());
                    self.rope.insert(at, &text);
                }
                _ => {}
            }
            nth += 1;
        }
        self.md_clamp_ends();
        self.mark_edited_from(sl);
        true
    }

    /// `- ` on every line of the span, or off it.
    ///
    /// A task line is left exactly as it is: it is already a bullet, and stripping the `- ` off
    /// `- [ ] thing` would leave `[ ] thing`, which is a bullet list item that looks like a
    /// checkbox and is not one.
    pub fn md_toggle_bullet(&mut self) -> bool {
        self.md_line_prefix(
            &|rest| {
                if md_task_marker(rest).is_some() {
                    Some(0)
                } else if rest.starts_with("- ") || rest.starts_with("* ") {
                    Some(2)
                } else {
                    None
                }
            },
            &|_, _| (0, "- ".to_string()),
        )
    }

    /// `- [ ] ` on every line of the span, or off it. A ticked box counts as present, so the
    /// button clears a list somebody has been working through rather than refusing to.
    pub fn md_toggle_task(&mut self) -> bool {
        self.md_line_prefix(
            &|rest| md_task_marker(rest).map(|_| 6),
            // A line that is already a bullet is promoted in place rather than given a second
            // dash: `- thing` becomes `- [ ] thing`.
            &|_, rest| {
                if rest.starts_with("- ") {
                    (2, "[ ] ".to_string())
                } else {
                    (0, "- [ ] ".to_string())
                }
            },
        )
    }

    /// `> ` on every line of the span, or off it.
    pub fn md_toggle_quote(&mut self) -> bool {
        self.md_line_prefix(
            &|rest| rest.starts_with("> ").then_some(2),
            &|_, _| (0, "> ".to_string()),
        )
    }

    /// `1. `, `2. `, … down the span, or off it.
    ///
    /// The numbers are the line's place in the span, not a continuation of whatever came before
    /// it: a list renumbered from one is what markdown renders anyway, and guessing at a
    /// preceding list would guess wrong across a blank line.
    pub fn md_toggle_numbered(&mut self) -> bool {
        self.md_line_prefix(&|rest| md_number_marker(rest), &|nth, _| (0, format!("{}. ", nth + 1)))
    }

    /// Steps the span's heading level round 0 → 1 → 2 → 3 → 0.
    ///
    /// One button rather than three, because the level you want is almost always one more or one
    /// fewer than the one you have, and a bar with `#`, `##` and `###` on it spends three targets
    /// saying the same thing. The level of the span's first non-blank line decides for all of
    /// them, so a heading and the line under it do not end up a level apart.
    pub fn md_cycle_heading(&mut self) -> bool {
        if !self.md_editable() {
            return false;
        }
        let last = self.rope.len_lines().saturating_sub(1);
        let (sl, el) = self.indent_range();
        if sl > last {
            return false;
        }
        let el = el.min(last);
        let mut level = None;
        for line in sl..=el {
            let text = self.rope.line(line).to_string();
            if text.trim().is_empty() {
                continue;
            }
            level = Some(md_heading_level(text.trim_start()));
            break;
        }
        let blank_only = level.is_none() && sl == el;
        let Some(level) = level.or(blank_only.then_some(0)) else { return false };
        let next = (level + 1) % 4;
        self.checkpoint(EditKind::Other);
        for line in sl..=el {
            let raw = self.rope.line(line).to_string();
            let body = raw.trim_end_matches(['\n', '\r']);
            if body.trim().is_empty() && !blank_only {
                continue;
            }
            let indent = body.chars().take_while(|c| *c == ' ' || *c == '\t').count();
            let rest: String = body.chars().skip(indent).collect();
            let base = self.rope.line_to_char(line) + indent;
            let old = md_heading_prefix_len(&rest);
            if old > 0 {
                let end = (base + old).min(self.rope.len_chars());
                if end > base {
                    self.rope.remove(base..end);
                }
            }
            if next > 0 {
                self.rope.insert(base, &format!("{} ", "#".repeat(next)));
            }
        }
        self.md_clamp_ends();
        self.mark_edited_from(sl);
        true
    }

    /// Turns the selection into `[selection](url)`, or inserts `[placeholder](url)` where the
    /// caret is, and leaves `url` selected — which is the part nobody has typed yet.
    pub fn md_insert_link(&mut self, placeholder_text: &str) -> bool {
        if !self.md_editable() {
            return false;
        }
        const URL: &str = "url";
        let (start, label, had_text) = match self.selection_range() {
            Some(((sl, sc), (el, ec))) => {
                if sl != el {
                    return false;
                }
                let start = self.rope.line_to_char(sl) + sc;
                let end = self.rope.line_to_char(el) + ec;
                (start, self.rope.slice(start..end).to_string(), true)
            }
            None => {
                (self.cursor_char_idx().min(self.rope.len_chars()), placeholder_text.to_string(), false)
            }
        };
        let end = (start + label.chars().count()).min(self.rope.len_chars());
        self.checkpoint(EditKind::Other);
        if had_text && end > start {
            self.rope.remove(start..end);
        }
        self.rope.insert(start, &format!("[{label}]({URL})"));
        let first_line = self.rope.char_to_line(start.min(self.rope.len_chars()));
        // Whichever half is still a placeholder is what the next keystroke should replace: the
        // address when the words came from a selection, the words when they did not.
        let (from, len) = if had_text {
            // `[` + the label + `](` stands between the start and the address.
            (start + label.chars().count() + 3, URL.chars().count())
        } else {
            (start + 1, label.chars().count())
        };
        self.select_char_range(from, from + len);
        self.md_clamp_ends();
        self.mark_edited_from(first_line);
        true
    }

    /// Fences the span in ``` lines, or takes an existing pair away.
    ///
    /// The pair is recognised only immediately above and below the span, which is what "this
    /// block" means when the cursor is inside one. With nothing selected on an empty line the
    /// two fences land around the caret, so the next thing typed is already inside the block.
    pub fn md_toggle_fence(&mut self) -> bool {
        if !self.md_editable() {
            return false;
        }
        let last = self.rope.len_lines().saturating_sub(1);
        let (sl, el) = self.indent_range();
        if sl > last {
            return false;
        }
        let el = el.min(last);
        let fenced = |ed: &Editor, line: usize| {
            line <= last && ed.rope.line(line).to_string().trim_start().starts_with("```")
        };
        let wrapped = sl > 0 && fenced(self, sl - 1) && el + 1 <= last && fenced(self, el + 1);
        self.checkpoint(EditKind::Other);
        if wrapped {
            // Bottom first: cutting it out cannot move a line above it.
            self.md_remove_line(el + 1);
            self.md_remove_line(sl - 1);
            self.cursor_line = self.cursor_line.saturating_sub(1);
            if let Some((al, ac)) = self.selection_anchor {
                self.selection_anchor = Some((al.saturating_sub(1), ac));
            }
            self.md_clamp_ends();
            self.mark_edited_from(sl.saturating_sub(1));
            return true;
        }
        // The closing fence goes on before the opening one, for the same reason. It is written
        // at the end of the span's last line rather than at the start of the one after it: the
        // last line of a file need not end in a newline, and there may be no line after it.
        let close_at = self.rope.line_to_char(el) + self.line_char_len(el);
        self.rope.insert(close_at, "\n```");
        let open_at = self.rope.line_to_char(sl);
        self.rope.insert(open_at, "```\n");
        self.cursor_line += 1;
        if let Some((al, ac)) = self.selection_anchor {
            self.selection_anchor = Some((al + 1, ac));
        }
        self.md_clamp_ends();
        self.mark_edited_from(sl);
        true
    }

    /// Cuts `line` out whole, newline included — and when it is the last line, the newline in
    /// front of it instead, so removing a closing fence does not leave the block ending on a
    /// blank line that was never in the file.
    fn md_remove_line(&mut self, line: usize) {
        let lines = self.rope.len_lines();
        if line >= lines {
            return;
        }
        let total = self.rope.len_chars();
        let mut start = self.rope.line_to_char(line);
        let end = if line + 1 < lines { self.rope.line_to_char(line + 1) } else { total };
        if end == total && start > 0 && self.rope.get_char(start - 1) == Some('\n') {
            start -= 1;
        }
        if start < end {
            self.rope.remove(start..end);
        }
    }

    /// Moves the cursor to the start of `line_1based` (clamped to the document), clearing
    /// any selection. Used by Go-to-line.
    pub fn goto_line(&mut self, line_1based: usize) {
        let max_line = self.rope.len_lines().saturating_sub(1);
        self.cursor_line = line_1based.saturating_sub(1).min(max_line);
        self.cursor_col = 0;
        self.selection_anchor = None;
        self.folds.clear();
    }

    /// Selects the absolute char range `[start, end)`, placing the cursor at `end`. Used by
    /// find to highlight the current match via the normal selection rendering.
    pub fn select_char_range(&mut self, start: usize, end: usize) {
        let total = self.rope.len_chars();
        let start = start.min(total);
        let end = end.min(total);
        let sl = self.rope.char_to_line(start);
        let sc = start - self.rope.line_to_char(sl);
        let el = self.rope.char_to_line(end);
        let ec = end - self.rope.line_to_char(el);
        self.folds.clear();
        self.selection_anchor = Some((sl, sc));
        self.cursor_line = el;
        self.cursor_col = ec;
    }

    /// Replaces the absolute char range `[start, end)` with `text` as one undo step, leaving
    /// the cursor just after the inserted text. Used by find-and-replace.
    pub fn replace_char_range(&mut self, start: usize, end: usize, text: &str) {
        if self.read_only {
            return;
        }
        let total = self.rope.len_chars();
        let start = start.min(total);
        let end = end.min(total);
        if start > end {
            return;
        }
        self.checkpoint(EditKind::Other);
        self.rope.remove(start..end);
        self.rope.insert(start, text);
        self.selection_anchor = None;
        // Replace-all works its way through the file without the cursor having been anywhere
        // near the match, so the range's own first line is the only honest answer here.
        let first_line = self.rope.char_to_line(start);
        self.set_cursor_char_idx(start + text.chars().count());
        self.mark_edited_from(first_line);
    }

    /// Marks the buffer changed: unsaved, one revision newer, and coloured only down to where
    /// the edit began.
    ///
    /// One call rather than the assignments it replaces, so an edit added later cannot quietly
    /// forget one of them — which is how a live preview would end up showing text that is no
    /// longer there.
    fn mark_edited(&mut self) {
        self.mark_edited_from(self.cursor_line);
    }

    /// As `mark_edited`, for an edit whose first changed line is known and lies above both ends
    /// of the cursor's journey — indenting a selection from its last line, say.
    fn mark_edited_from(&mut self, line: usize) {
        self.note_edit_line(line);
        let from = self.pending_edit_line.take().unwrap_or(line).min(self.cursor_line);
        // The marks say "this arrived from outside since you last touched the file", and the
        // moment you touch it that sentence stops being true. Put out here rather than at each
        // key, because every edit there is — typing, pasting, undo, a line moved, a comment
        // toggled, a replacement from the Find box — ends up in this one function, and the one
        // added next month will too.
        self.changed_lines.clear();
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
        self.highlighted.truncate(self.syntax_cache.invalidate_from(from));
    }

    /// Remembers the earliest line any part of the edit in flight has touched.
    fn note_edit_line(&mut self, line: usize) {
        self.pending_edit_line = Some(self.pending_edit_line.map_or(line, |seen| seen.min(line)));
    }

    /// Colours the buffer down to line `through`, which is as far as the renderer is about to
    /// look. Anything below that is left for the frame that scrolls to it.
    pub fn refresh_highlight(&mut self, highlighter: &Highlighter, through: usize) {
        if self.syntax_dirty {
            self.syntax_dirty = false;
            self.forget_highlight();
        }
        highlighter.extend_to(
            self.path.as_deref(),
            &self.rope,
            through,
            &mut self.syntax_cache,
            &mut self.highlighted,
        );
    }

    /// Throws the colours away, for a buffer that is not being highlighted at all.
    pub fn forget_highlight(&mut self) {
        if !self.highlighted.is_empty() || self.syntax_cache.valid_lines() != 0 {
            self.highlighted.clear();
            self.syntax_cache.clear();
        }
    }

    /// How many times the text has changed. Compared, never interpreted.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Notes whether the view moved since the last frame. Called once per frame by the renderer,
    /// which is the only place that sees every kind of scroll, whatever caused it.
    pub fn observe_scroll(&mut self) {
        let now = (self.top_line, self.left_col);
        if now != self.scroll_seen {
            self.scroll_seen = now;
            self.scroll_moved = Some(Instant::now());
        }
    }

    /// Marks the view as having just moved, for a scroll this buffer's own `top_line` cannot
    /// record — a preview moves a window over a picture, not a cursor over lines.
    pub fn mark_scrolled(&mut self) {
        self.scroll_moved = Some(Instant::now());
    }

    /// Whether the view moved within `window`, for deciding if a scrollbar still has a reason
    /// to be on screen.
    pub fn scrolled_within(&self, window: Duration) -> bool {
        self.scroll_moved.is_some_and(|at| at.elapsed() < window)
    }

    /// Keeps the cursor on screen, but only when the cursor is what moved — or when the viewport
    /// changed size under it.
    ///
    /// The renderer is the only place that knows the viewport, so it used to call `adjust_scroll`
    /// on every frame unconditionally. That quietly made the cursor a wall: the wheel could move
    /// the view until the cursor line fell off the edge, and from there every further notch was
    /// undone before it was ever drawn. Scrolling a long file with a trackpad stopped dead after
    /// one screen, and the only way on was to click into the text — moving the cursor, so the
    /// wall moved with it.
    ///
    /// Scrolling is a look, not a move: the view goes where it is sent and stays there, and the
    /// next arrow key or edit brings it back to the cursor, which is what every editor does.
    pub fn follow_cursor(&mut self, viewport_height: usize, viewport_width: usize) {
        let cursor = (self.cursor_line, self.cursor_col);
        let viewport = (viewport_height, viewport_width);
        if cursor == self.cursor_seen && viewport == self.viewport_seen {
            return;
        }
        self.cursor_seen = cursor;
        self.viewport_seen = viewport;
        self.adjust_scroll(viewport_height, viewport_width);
    }

    pub fn adjust_scroll(&mut self, viewport_height: usize, viewport_width: usize) {
        if viewport_height > 0 {
            if self.cursor_line < self.top_line {
                self.top_line = self.cursor_line;
            } else if self.cursor_line >= self.top_line + viewport_height {
                self.top_line = self.cursor_line + 1 - viewport_height;
            }
        }
        if viewport_width > 0 {
            if self.cursor_col < self.left_col {
                self.left_col = self.cursor_col;
            } else if self.cursor_col >= self.left_col + viewport_width {
                self.left_col = self.cursor_col + 1 - viewport_width;
            }
        }
    }
}

/// Text with every line ending spelled as a bare '\n', the way the buffer holds them.
///
/// Both spellings a carriage return arrives in are line breaks: the Windows pair, and the lone
/// '\r' of classic Mac text and of some terminals' bracketed paste. Borrowed unchanged when
/// there is nothing to do, which is the usual case.
fn normalize_newlines(text: &str) -> Cow<'_, str> {
    if text.contains('\r') {
        Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(text)
    }
}

/// The closing partner for an opening bracket or quote, or None if `ch` doesn't open a pair.
fn close_partner(ch: char) -> Option<char> {
    match ch {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '"' => Some('"'),
        '\'' => Some('\''),
        '`' => Some('`'),
        _ => None,
    }
}

fn is_closer(ch: char) -> bool {
    matches!(ch, ')' | ']' | '}' | '"' | '\'' | '`')
}

/// Character class for word-wise motion: word chars (identifiers) vs punctuation. Runs of
/// one class are skipped as a unit; whitespace is handled separately by the callers.
fn word_class(c: char) -> u8 {
    if c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

/// The checkbox at the head of `rest` (already past its indentation), and whether it is ticked.
/// `None` for a line that is not a task item.
fn md_task_marker(rest: &str) -> Option<bool> {
    match rest.get(..6) {
        Some("- [ ] ") => Some(false),
        Some("- [x] ") | Some("- [X] ") => Some(true),
        _ => None,
    }
}

/// How many characters of `rest` are an ordered-list marker — digits, a `.` or `)`, a space —
/// or `None` when it does not open with one. Parsed by hand: the crate list has no regex engine
/// in it and one marker is not a reason to add one.
fn md_number_marker(rest: &str) -> Option<usize> {
    let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let mut after = rest.chars().skip(digits);
    match (after.next(), after.next()) {
        (Some('.') | Some(')'), Some(' ')) => Some(digits + 2),
        _ => None,
    }
}

/// The heading level of `rest` (already past its indentation): the run of `#` that opens it,
/// and 0 when there is none or no space follows it — `#tag` is a word, not a title.
fn md_heading_level(rest: &str) -> usize {
    let hashes = rest.chars().take_while(|c| *c == '#').count();
    if hashes > 0 && rest.chars().nth(hashes) == Some(' ') { hashes } else { 0 }
}

/// How many characters that heading prefix takes, space included.
fn md_heading_prefix_len(rest: &str) -> usize {
    match md_heading_level(rest) {
        0 => 0,
        n => n + 1,
    }
}

/// The line-comment token for a file, chosen by extension — or by file name, for the ones
/// that have no extension to go on (`Makefile`, `Dockerfile`, `Gemfile`). None means "no known
/// line comment syntax", in which case the comment-toggle command is a no-op. Languages whose
/// only comment is a block (HTML, CSS, plain JSON) belong in that None on purpose: half a
/// `<!--` on every line would be worse than nothing.
pub fn comment_token(path: Option<&std::path::Path>) -> Option<&'static str> {
    let path = path?;
    let token = match path.extension().or_else(|| path.file_name())?.to_str()?.to_lowercase().as_str() {
        "rs" | "c" | "h" | "cpp" | "hpp" | "cc" | "cxx" | "hh" | "cs" | "js" | "jsx" | "mjs"
        | "cjs" | "ts" | "tsx" | "mts" | "cts" | "jsonc" | "json5" | "go" | "java" | "kt"
        | "kts" | "swift" | "scala" | "sbt" | "groovy" | "gradle" | "php" | "dart" | "zig"
        | "mm" | "d" | "v" | "sv" | "proto" | "sol" | "glsl" | "hlsl" | "wgsl" | "scss"
        | "less" | "styl" | "rego" | "jsonnet" => "//",
        "py" | "pyi" | "rb" | "gemspec" | "rake" | "sh" | "bash" | "zsh" | "fish" | "ps1"
        | "psm1" | "toml" | "yaml" | "yml" | "pl" | "r" | "jl" | "ex" | "exs" | "cr" | "nim"
        | "nix" | "tf" | "tfvars" | "hcl" | "awk" | "tcl" | "gd" | "cmake" | "pp" | "just"
        | "star" | "bzl" | "bazel" | "mk" | "conf" | "cfg" | "ini" | "env" | "makefile"
        | "dockerfile" | "containerfile" | "justfile" | "gemfile" | "rakefile" | "vagrantfile"
        | "brewfile" | "podfile" | "cmakelists.txt" | "gitignore" | ".gitignore" => "#",
        "lua" | "sql" | "hs" | "elm" | "purs" | "adb" | "ads" | "vhd" | "vhdl" => "--",
        "vim" | "vimrc" => "\"",
        "lisp" | "clj" | "cljs" | "cljc" | "scm" | "rkt" | "el" | "asm" => ";",
        // `.m` is Octave/MATLAB here — that is the language the Run button knows it as — not
        // Objective-C, whose `.mm` sibling is in the `//` list above.
        "tex" | "sty" | "cls" | "bib" | "erl" | "hrl" | "m" | "mat" => "%",
        _ => return None,
    };
    Some(token)
}

/// How wide a run of differing lines this is still willing to align, per side.
///
/// The comparison below is a longest common subsequence, which costs one table entry per pair of
/// lines — quadratic in both time and memory, and a file against itself at twenty thousand lines
/// would ask for a gigabyte of it. The trimming that happens first means this ceiling applies to
/// the *differing* part rather than to the file: a hundred-thousand-line file with a paragraph
/// rewritten in the middle is a problem a few lines wide and is answered exactly.
const DIFFERING_LINES_CAP: usize = 2_000;

/// The lines of `after` that were not already in `before`, as indices into `after`.
///
/// This is what makes an agent's edit visible. An agent does not type: it writes the whole file
/// at every change, so what arrives is a new text, and the only interesting question about it is
/// which of its lines are new. Deletions produce nothing — there is no line left to light — and
/// a file that came back identical produces nothing either.
///
/// Common leading and trailing lines are peeled off first, which is both an optimisation and the
/// honest description of the usual case: a file rewritten with a few lines different somewhere
/// inside it. What is left is aligned with a longest common subsequence; past
/// [`DIFFERING_LINES_CAP`] lines on either side the answer is *nothing at all* rather than
/// everything. A gutter lit from top to bottom tells the reader as little as an empty one, and
/// spends a screenful of colour saying it.
pub fn changed_lines(before: &str, after: &str) -> Vec<usize> {
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();

    let shorter = old.len().min(new.len());
    let mut head = 0;
    while head < shorter && old[head] == new[head] {
        head += 1;
    }
    let mut tail = 0;
    while tail < shorter - head && old[old.len() - 1 - tail] == new[new.len() - 1 - tail] {
        tail += 1;
    }
    let old_mid = &old[head..old.len() - tail];
    let new_mid = &new[head..new.len() - tail];

    // Nothing left on the old side: everything in the middle is new, and no alignment is needed
    // to know it. This is a pure insertion — a block pasted in, or a file that grew.
    if old_mid.is_empty() {
        return (head..head + new_mid.len()).collect();
    }
    // Nothing left on the new side: lines went away and none arrived.
    if new_mid.is_empty() {
        return Vec::new();
    }
    if old_mid.len() > DIFFERING_LINES_CAP || new_mid.len() > DIFFERING_LINES_CAP {
        return Vec::new();
    }

    let (n, m) = (old_mid.len(), new_mid.len());
    // Lengths of the longest common subsequence of the two suffixes starting at (i, j). `u16`
    // because the cap keeps every value under it, and the table is the memory that matters.
    let mut lcs = vec![0u16; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[at(i, j)] = if old_mid[i] == new_mid[j] {
                lcs[at(i + 1, j + 1)] + 1
            } else {
                lcs[at(i + 1, j)].max(lcs[at(i, j + 1)])
            };
        }
    }

    // Walked forwards from the top, so the lines come out ascending and the gutter can binary
    // search them. A tie goes to the old side: given the choice, a line is called deleted rather
    // than inserted, which keeps a replaced line from lighting its neighbour as well.
    let mut arrived = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old_mid[i] == new_mid[j] {
            i += 1;
            j += 1;
        } else if lcs[at(i + 1, j)] >= lcs[at(i, j + 1)] {
            i += 1;
        } else {
            arrived.push(head + j);
            j += 1;
        }
    }
    while j < m {
        arrived.push(head + j);
        j += 1;
    }
    arrived
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_navigate() {
        let mut ed = Editor::empty();
        ed.insert_char('h');
        ed.insert_char('i');
        ed.insert_newline(false);
        ed.insert_char('!');
        assert_eq!(ed.rope.to_string(), "hi\n!");
        assert_eq!(ed.cursor_line, 1);
        assert_eq!(ed.cursor_col, 1);

        ed.move_left();
        ed.move_left();
        assert_eq!(ed.cursor_line, 0);
        assert_eq!(ed.cursor_col, 2);
    }

    #[test]
    fn backspace_merges_lines() {
        let mut ed = Editor::empty();
        ed.insert_char('a');
        ed.insert_newline(false);
        ed.insert_char('b');
        assert_eq!(ed.rope.to_string(), "a\nb");
        ed.move_home();
        ed.backspace();
        assert_eq!(ed.rope.to_string(), "ab");
        assert_eq!(ed.cursor_line, 0);
        assert_eq!(ed.cursor_col, 1);
    }

    #[test]
    fn delete_forward_removes_next_char() {
        let mut ed = Editor::empty();
        ed.insert_char('a');
        ed.insert_char('b');
        ed.move_home();
        ed.delete_forward();
        assert_eq!(ed.rope.to_string(), "b");
    }

    #[test]
    fn auto_indent_copies_leading_whitespace() {
        let mut ed = Editor::empty();
        ed.insert_str("  abc");
        ed.insert_newline(true);
        ed.insert_char('x');
        assert_eq!(ed.rope.to_string(), "  abc\n  x");
    }

    #[test]
    fn save_writes_file() {
        let dir = std::env::temp_dir().join(format!("clicode_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.txt");
        let mut ed = Editor::empty();
        ed.path = Some(path.clone());
        ed.insert_char('x');
        ed.save().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "x");
        assert!(!ed.dirty);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A buffer of `lines` lines of Rust, named so the highlighter reads it as Rust.
    fn sample_buffer(lines: usize) -> Editor {
        let text: Vec<String> = (0..lines).map(|i| format!("fn f{i}() {{ let v = {i}; }}")).collect();
        let mut ed = Editor::empty();
        ed.path = Some(PathBuf::from("sample.rs"));
        ed.rope = Rope::from_str(&text.join("\n"));
        ed
    }

    fn coloured_whole(rope: &Rope) -> Vec<Vec<(Style, String)>> {
        let mut fresh = Editor::empty();
        fresh.path = Some(PathBuf::from("sample.rs"));
        fresh.rope = rope.clone();
        fresh.refresh_highlight(&Highlighter::new(), usize::MAX);
        fresh.highlighted
    }

    /// The colours above the line being typed on cannot have changed, so they are kept — that is
    /// the whole reason for holding them per line. What is kept has to be indistinguishable from
    /// what a fresh pass would have produced, which is what the second half checks.
    #[test]
    fn typing_keeps_the_colours_above_the_line_it_happens_on() {
        let highlighter = Highlighter::new();
        let mut ed = sample_buffer(200);
        ed.refresh_highlight(&highlighter, 199);
        assert_eq!(ed.highlighted.len(), 200);

        ed.cursor_line = 150;
        ed.cursor_col = 3;
        ed.insert_char('x');
        let kept = ed.highlighted.len();
        assert!(kept > 100 && kept <= 150, "most of the file survives a keystroke in the middle of it");

        ed.refresh_highlight(&highlighter, 199);
        assert_eq!(ed.highlighted, coloured_whole(&ed.rope));
    }

    /// An edit can start above the cursor at either end of it: a selection dragged downwards and
    /// indented leaves the cursor on its last line while the first line is what changed.
    #[test]
    fn indenting_a_selection_re_colours_from_its_first_line() {
        let highlighter = Highlighter::new();
        let mut ed = sample_buffer(200);
        ed.refresh_highlight(&highlighter, 199);

        ed.selection_anchor = Some((100, 0));
        ed.cursor_line = 150;
        ed.cursor_col = 0;
        ed.indent_selection(4);
        assert!(ed.highlighted.len() <= 100, "the first indented line is where the colours stop");

        ed.refresh_highlight(&highlighter, 199);
        assert_eq!(ed.highlighted, coloured_whole(&ed.rope));
    }

    /// An undo replaces the whole text, so no line of the old colouring can be assumed to have
    /// survived it.
    #[test]
    fn an_undo_re_colours_the_whole_buffer() {
        let highlighter = Highlighter::new();
        let mut ed = sample_buffer(200);
        ed.cursor_line = 150;
        ed.insert_char('/');
        ed.refresh_highlight(&highlighter, 199);
        assert!(!ed.highlighted.is_empty());

        assert!(ed.undo());
        assert!(ed.highlighted.is_empty());
        ed.refresh_highlight(&highlighter, 199);
        assert_eq!(ed.highlighted, coloured_whole(&ed.rope));
    }

    /// A keypress that changes nothing is not an edit: Backspace at the very start of the buffer
    /// used to take a checkpoint anyway, and a checkpoint drops the redo stack — so a stray press
    /// threw away a redo the user had never asked to lose.
    #[test]
    fn a_backspace_that_deletes_nothing_leaves_the_history_alone() {
        let mut ed = Editor::empty();
        ed.insert_str("abc");
        assert!(ed.undo());
        assert_eq!(ed.rope.to_string(), "");
        assert_eq!(ed.redo_stack.len(), 1);

        ed.cursor_line = 0;
        ed.cursor_col = 0;
        ed.backspace();
        assert_eq!(ed.redo_stack.len(), 1, "the redo is still there to be redone");
        assert!(ed.undo_stack.is_empty());
        assert!(ed.redo());
        assert_eq!(ed.rope.to_string(), "abc");
    }

    /// The same at the other end of the buffer, where Delete has nothing in front of it.
    #[test]
    fn a_delete_at_the_end_of_the_buffer_leaves_the_history_alone() {
        let mut ed = Editor::empty();
        ed.insert_str("abc");
        assert!(ed.undo());
        assert_eq!(ed.redo_stack.len(), 1);

        ed.delete_forward();
        assert_eq!(ed.redo_stack.len(), 1);
        assert!(ed.undo_stack.is_empty());
    }

    /// Undo ends the selection it lands in, rectangle and all. A block flag left switched on
    /// outlives the selection it belonged to, and the next Shift+arrow draws a rectangle the
    /// user never asked for.
    #[test]
    fn an_undo_ends_a_column_selection_as_well_as_an_ordinary_one() {
        let mut ed = Editor::empty();
        ed.insert_str("abcdef");
        ed.insert_newline(false);
        ed.insert_str("ghijkl");
        ed.selection_block = true;
        ed.selection_anchor = Some((0, 1));
        ed.cursor_line = 1;
        ed.cursor_col = 4;

        assert!(ed.undo());
        assert_eq!(ed.selection_anchor, None);
        assert!(!ed.selection_block, "the rectangle goes with the selection");
    }

    /// Text copied out of a Windows program arrives with its carriage returns still on it. The
    /// buffer holds '\n' alone — the file's own ending is put back when it is saved — so a '\r'
    /// left in the text is an invisible character that turns into `\r\r\n` on the way to disk.
    #[test]
    fn a_pasted_line_ending_is_normalised_on_the_way_in() {
        assert_eq!(normalize_newlines("a\r\nb\rc\nd"), "a\nb\nc\nd");
        assert!(matches!(normalize_newlines("nothing to do"), Cow::Borrowed(_)));

        let mut ed = Editor::empty();
        ed.insert_multiline("one\r\ntwo\r\n");
        assert_eq!(ed.rope.to_string(), "one\ntwo\n");
        assert!(!ed.rope.to_string().contains('\r'));
    }

    /// A rectangle over ragged text: the short line has nothing under the columns, and must
    /// contribute an empty row rather than borrowing characters that are not there.
    #[test]
    fn a_column_selection_takes_a_rectangle_not_a_run() {
        let mut ed = Editor::empty();
        ed.insert_str("abcdef\nGH\nijklmn");
        ed.selection_block = true;
        ed.selection_anchor = Some((0, 2));
        ed.cursor_line = 2;
        ed.cursor_col = 4;

        assert_eq!(ed.block_range(), Some((0, 2, 2, 4)));
        assert_eq!(ed.selected_columns(0), Some((2, 4)), "cd");
        assert_eq!(ed.selected_columns(1), None, "GH is too short to reach column 2");
        assert_eq!(ed.selected_columns(2), Some((2, 4)), "kl");
        assert_eq!(ed.selected_text().as_deref(), Some("cd\n\nkl"));

        // Cutting it removes only those cells, leaving the rest of every line in place.
        assert!(ed.delete_selection());
        assert_eq!(ed.rope.to_string(), "abef\nGH\nijmn");
        assert_eq!((ed.cursor_line, ed.cursor_col), (0, 2));
        assert!(!ed.selection_block, "the mode ends with the selection it applied to");
    }

    /// Dragged the other way it is the same rectangle: the corners are normalised, not the
    /// order they were given in.
    #[test]
    fn a_column_selection_does_not_care_which_corner_you_started_from() {
        let mut ed = Editor::empty();
        ed.insert_str("abcdef\nghijkl");
        ed.selection_block = true;
        ed.selection_anchor = Some((1, 4));
        ed.cursor_line = 0;
        ed.cursor_col = 1;
        assert_eq!(ed.block_range(), Some((0, 1, 1, 4)));
        assert_eq!(ed.selected_text().as_deref(), Some("bcd\nhij"));

        // Zero width is not a selection, however tall it is.
        ed.cursor_col = 4;
        ed.selection_anchor = Some((0, 4));
        assert_eq!(ed.block_range(), None);
        assert_eq!(ed.selected_text(), None);
    }

    /// Puts a column selection on lines `first..=last` between columns `c0` and `c1`, the way a
    /// drag with Alt or the Edit menu and Shift+arrows would leave it. `c0 == c1` is the caret
    /// column typing leaves behind.
    fn block(ed: &mut Editor, first: usize, last: usize, c0: usize, c1: usize) {
        ed.selection_block = true;
        ed.selection_anchor = Some((first, c0));
        ed.cursor_line = last;
        ed.cursor_col = c1;
    }

    /// The whole point of the feature: one key, one character on every line of the block — and
    /// the block still there afterwards, one column along, so the next key does it again. A line
    /// too short to reach the column is skipped rather than padded out to it, which is the rule
    /// `selected_columns` already states for reading a rectangle.
    #[test]
    fn typing_in_a_column_selection_writes_on_every_line_it_covers() {
        let mut ed = Editor::empty();
        ed.insert_str("abcdef\nGH\nijklmn");
        block(&mut ed, 0, 2, 4, 4);

        ed.insert_char_pairs('X', true);
        assert_eq!(ed.rope.to_string(), "abcdXef\nGH\nijklXmn", "GH cannot reach column 4");
        assert!(ed.selection_block, "the column outlives the keystroke, or it is not a column");
        assert_eq!(ed.selection_anchor, Some((0, 5)));
        assert_eq!((ed.cursor_line, ed.cursor_col), (2, 5));

        // And again, without touching anything in between: the second character lands under the
        // first on every line, which is what makes it a column and not three separate edits.
        ed.insert_char_pairs('Y', true);
        assert_eq!(ed.rope.to_string(), "abcdXYef\nGH\nijklXYmn");

        // One Ctrl+Z per keystroke, not one per line.
        assert!(ed.undo());
        assert_eq!(ed.rope.to_string(), "abcdXef\nGH\nijklXmn");
        assert!(ed.undo());
        assert_eq!(ed.rope.to_string(), "abcdef\nGH\nijklmn");
        assert!(ed.redo());
        assert_eq!(ed.rope.to_string(), "abcdXef\nGH\nijklXmn");
    }

    /// A block dragged down through a short line keeps its column. Clamped to the shortest line
    /// it passed it would collapse — permanently, since nothing remembers where it was — and the
    /// rule that a short line is skipped would describe a state no keyboard could reach.
    #[test]
    fn a_column_selection_keeps_its_column_across_a_line_too_short_for_it() {
        let mut ed = Editor::empty();
        ed.insert_str("abcdefgh\nGH\nijklmnop");
        block(&mut ed, 0, 0, 4, 4);
        ed.move_down();
        assert_eq!((ed.cursor_line, ed.cursor_col), (1, 4), "the column belongs to the block");
        ed.move_down();
        assert_eq!((ed.cursor_line, ed.cursor_col), (2, 4));
        ed.insert_char_pairs('|', true);
        assert_eq!(ed.rope.to_string(), "abcd|efgh\nGH\nijkl|mnop");

        // Leaving the mode hands the column back to the line the cursor is on.
        ed.cursor_line = 1;
        ed.clear_selection();
        assert_eq!(ed.cursor_col, 2);

        // And without a block, a vertical move clamps exactly as it always has.
        ed.cursor_line = 0;
        ed.cursor_col = 6;
        ed.move_down();
        assert_eq!((ed.cursor_line, ed.cursor_col), (1, 2));
    }

    /// Typing over a rectangle that has width replaces it, the way typing over any selection
    /// does — and what is left standing is the caret column, ready for the next character.
    #[test]
    fn typing_over_a_rectangle_replaces_it_and_goes_on_as_a_column() {
        let mut ed = Editor::empty();
        ed.insert_str("abcdef\nghijkl");
        block(&mut ed, 0, 1, 2, 4);

        ed.insert_char_pairs('Z', true);
        assert_eq!(ed.rope.to_string(), "abZef\nghZkl");
        assert_eq!(ed.block_range(), None, "the rectangle spent itself; the column remains");
        assert_eq!(ed.block_caret(0), Some(3));
        assert_eq!(ed.block_caret(1), Some(3));

        ed.insert_char_pairs('W', true);
        assert_eq!(ed.rope.to_string(), "abZWef\nghZWkl");
        assert!(ed.undo());
        assert_eq!(ed.rope.to_string(), "abZef\nghZkl", "the replacement is one step of its own");
    }

    /// A bracket typed into a block is one bracket per line and nothing else. Pairing eight
    /// closers nobody asked for, on eight lines, is not what one keystroke should mean.
    #[test]
    fn a_column_selection_does_not_auto_pair() {
        let mut ed = Editor::empty();
        ed.insert_str("ab\ncd");
        block(&mut ed, 0, 1, 2, 2);
        ed.insert_char_pairs('(', true);
        assert_eq!(ed.rope.to_string(), "ab(\ncd(");
    }

    /// Backspace is the same key on a column as on a line: it takes the character in front of
    /// the caret — of every caret. A rectangle with width loses its cells instead, and leaves the
    /// column standing at its left edge so the next key still means all of these lines.
    #[test]
    fn backspace_in_a_column_selection_eats_one_column_from_every_line() {
        let mut ed = Editor::empty();
        ed.insert_str("abcdef\nGH\nijklmn");
        block(&mut ed, 0, 2, 4, 4);

        ed.backspace();
        assert_eq!(ed.rope.to_string(), "abcef\nGH\nijkmn", "GH has nothing under the column");
        assert_eq!(ed.selection_anchor, Some((0, 3)));
        assert_eq!((ed.cursor_line, ed.cursor_col), (2, 3));
        assert!(ed.undo());
        assert_eq!(ed.rope.to_string(), "abcdef\nGH\nijklmn", "one step for the whole column");

        // At column zero there is nothing in front. The key stays the block's rather than
        // falling through and joining the lines together.
        block(&mut ed, 0, 2, 0, 0);
        ed.backspace();
        assert_eq!(ed.rope.to_string(), "abcdef\nGH\nijklmn");
        assert!(ed.selection_block);

        block(&mut ed, 0, 1, 2, 4);
        ed.backspace();
        assert_eq!(ed.rope.to_string(), "abef\nGH\nijklmn");
        assert!(ed.selection_block, "the rectangle goes, the column stays");
        assert_eq!(ed.block_caret(0), Some(2));
        // A line whose end *is* the column is not too short for it: the character goes on the
        // end, which is where the column points.
        ed.insert_char_pairs('!', true);
        assert_eq!(ed.rope.to_string(), "ab!ef\nGH!\nijklmn");
    }

    /// The caret column is drawn only where the next keystroke will actually write, and never
    /// under a rectangle that has width — that one shades itself.
    #[test]
    fn the_caret_column_is_drawn_where_the_next_key_would_write() {
        let mut ed = Editor::empty();
        ed.insert_str("abcdef\nGH\nijklmn");
        block(&mut ed, 0, 2, 4, 4);
        assert_eq!(ed.block_caret(0), Some(4));
        assert_eq!(ed.block_caret(1), None, "GH does not reach column 4, so nothing will land");
        assert_eq!(ed.block_caret(2), Some(4));

        block(&mut ed, 0, 1, 2, 4);
        assert_eq!(ed.block_caret(0), None, "a rectangle with width is shown as a selection");
        ed.clear_selection();
        assert_eq!(ed.block_caret(0), None, "and nothing at all is shown without the mode");
    }

    /// A column selection with no width has two endpoints, and read as an ordinary selection they
    /// span every character between them. Enter, Tab and a paste all delete the selection before
    /// they write: read the wrong way, one of them would swallow the lines under the block.
    #[test]
    fn a_widthless_column_selection_is_never_read_as_a_run_of_text() {
        let mut ed = Editor::empty();
        ed.insert_str("abcdef\nghijkl\nmnopqr");
        block(&mut ed, 0, 2, 3, 3);
        assert_eq!(ed.selected_text(), None);
        assert_eq!(ed.selected_columns(1), None);

        ed.insert_newline(false);
        assert_eq!(ed.rope.to_string(), "abcdef\nghijkl\nmno\npqr", "only a newline, at the caret");
        assert!(!ed.selection_block, "and the mode is dropped, since a newline is not a column");
    }

    /// The ordinary selection has to keep behaving exactly as before, since both now go through
    /// `selected_columns`.
    #[test]
    fn an_ordinary_selection_still_flows_like_text() {
        let mut ed = Editor::empty();
        ed.insert_str("abcdef\nghijkl");
        ed.selection_anchor = Some((0, 4));
        ed.cursor_line = 1;
        ed.cursor_col = 2;
        assert_eq!(ed.selected_columns(0), Some((4, 6)), "to the end of the first line");
        assert_eq!(ed.selected_columns(1), Some((0, 2)), "from the start of the last");
        assert_eq!(ed.selected_text().as_deref(), Some("ef\ngh"));
    }

    #[test]
    fn selection_copy_and_delete() {
        let mut ed = Editor::empty();
        ed.insert_str("hello world");
        ed.cursor_col = 0;
        ed.selection_anchor = Some((0, 0));
        ed.cursor_col = 5;
        assert_eq!(ed.selected_text().as_deref(), Some("hello"));
        assert!(ed.delete_selection());
        assert_eq!(ed.rope.to_string(), " world");
        assert_eq!(ed.cursor_col, 0);
        assert!(ed.selection_anchor.is_none());
    }

    #[test]
    fn indent_and_outdent_current_line() {
        let mut ed = Editor::empty();
        ed.insert_str("abc");
        ed.indent_selection(4);
        assert_eq!(ed.rope.to_string(), "    abc");
        ed.outdent_selection(4);
        assert_eq!(ed.rope.to_string(), "abc");
    }

    #[test]
    fn indent_selection_spanning_two_lines() {
        let mut ed = Editor::empty();
        ed.insert_str("one");
        ed.insert_newline(false);
        ed.insert_str("two");
        ed.selection_anchor = Some((0, 0));
        ed.cursor_line = 1;
        ed.cursor_col = 3;
        ed.indent_selection(2);
        assert_eq!(ed.rope.to_string(), "  one\n  two");
    }

    /// A selection ending exactly at column 0 hands `indent_range` a last line that never gets
    /// a `pad` inserted into it (there is nothing on that line to indent). The cursor sitting on
    /// that untouched line must not slide right along with the lines that did get indented —
    /// it used to, landing past end-of-line on a short last line, and the next keystroke would
    /// index ropey past `len_chars()` and panic.
    #[test]
    fn indenting_a_selection_ending_at_column_zero_leaves_the_cursor_where_it_was() {
        let mut ed = Editor::empty();
        ed.insert_str("aa");
        ed.insert_newline(false);
        ed.insert_str("bb");
        ed.insert_newline(false);
        ed.insert_str("c");
        ed.selection_anchor = Some((0, 0));
        ed.cursor_line = 2;
        ed.cursor_col = 0;

        ed.indent_selection(4);
        assert_eq!(ed.rope.to_string(), "    aa\n    bb\nc");
        assert_eq!((ed.cursor_line, ed.cursor_col), (2, 0));

        // The selection itself is still live — its anchor moved with the indented line 0, its
        // end (the cursor) held still on the untouched line 2 — so typing replaces it, the same
        // as typing over any other selection. What used to panic here was `delete_selection_raw`
        // computing an end index from a cursor column (4) that no longer existed on a 1-char
        // line: past `len_chars()`, and ropey's `remove` asserts on an out-of-range end.
        ed.insert_char('x');
        assert_eq!(ed.rope.to_string(), "    xc");
    }

    #[test]
    fn brace_fold_collapses_and_expands() {
        let mut ed = Editor::empty();
        ed.insert_str("fn main() {");
        ed.insert_newline(false);
        ed.insert_str("    println!(\"hi\");");
        ed.insert_newline(false);
        ed.insert_str("}");
        ed.cursor_line = 0;
        assert_eq!(ed.foldable_range_at(0), Some((0, 2)));
        ed.toggle_fold();
        assert_eq!(ed.folds, vec![(0, 2)]);
        assert_eq!(ed.visible_rows_from(0, 10), vec![0]);
        ed.toggle_fold();
        assert!(ed.folds.is_empty());
        assert_eq!(ed.visible_rows_from(0, 10), vec![0, 1, 2]);
    }

    /// Where a server has said where a block is, its boundary wins — and where it has said nothing
    /// about the line, the braces the editor has always counted still decide.
    #[test]
    fn a_servers_boundary_beats_the_braces_and_the_braces_survive_where_it_says_nothing() {
        let mut ed = Editor::empty();
        for line in ["use std::io;", "use std::fmt;", "", "fn main() {", "    let x = 1;", "}"] {
            ed.insert_str(line);
            ed.insert_newline(false);
        }
        // Fresh from disk as far as the fold cache is concerned; typing it in is what made it
        // dirty, and the rule below is the subject of its own test.
        ed.dirty = false;
        // Nothing at line 0 counts braces: it is a `use` line, and the heuristic sees no block.
        assert_eq!(ed.foldable_range_at(0), None);
        // The server sees the import group, and one range that starts where the braces do.
        ed.server_folds = vec![(0, 1), (3, 5)];
        assert_eq!(ed.foldable_range_at(0), Some((0, 1)), "the import group is only the server's");
        ed.cursor_line = 0;
        ed.toggle_fold();
        assert_eq!(ed.folds, vec![(0, 1)]);
        assert_eq!(ed.visible_rows_from(0, 10), vec![0, 2, 3, 4, 5, 6]);
        // Where several begin on one line the widest wins: that is the block a reader means by
        // "this one", the way a marker on a signature means the function and not its first line.
        ed.server_folds = vec![(3, 4), (3, 5)];
        assert_eq!(ed.foldable_range_at(3), Some((3, 5)));
        // And with the server saying nothing about that line, the braces answer as they always did.
        ed.server_folds = vec![(0, 1)];
        assert_eq!(ed.foldable_range_at(3), Some((3, 5)));
    }

    /// The cache's moment of truth. These are line numbers taken when the buffer and the server
    /// last agreed, and one typed newline makes every number below it a lie — so an edited buffer
    /// folds by the braces until the next save puts the two back in step.
    #[test]
    fn an_edited_buffer_stops_believing_the_servers_fold_boundaries() {
        let mut ed = Editor::empty();
        for line in ["use std::io;", "use std::fmt;", "", "fn main() {", "    let x = 1;", "}"] {
            ed.insert_str(line);
            ed.insert_newline(false);
        }
        ed.dirty = false;
        ed.server_folds = vec![(0, 1), (3, 5)];
        assert_eq!(ed.foldable_range_at(0), Some((0, 1)));

        // One character typed anywhere, and the import group the server named is no longer a
        // thing this buffer will fold on: the braces are computed from the text on screen and
        // cannot be stale, so they are what is left.
        ed.cursor_line = 0;
        ed.cursor_col = 0;
        ed.insert_str("// ");
        assert!(ed.dirty);
        assert_eq!(ed.foldable_range_at(0), None, "a dirty buffer folds by the braces alone");
        // The brace block is still found, because that answer never came from the server.
        assert_eq!(ed.foldable_range_at(3), Some((3, 5)));
        // And a save puts the cache back in force without anything having to re-announce it.
        ed.dirty = false;
        assert_eq!(ed.foldable_range_at(0), Some((0, 1)));
    }

    #[test]
    fn indentation_fold_python_style() {
        let mut ed = Editor::empty();
        ed.insert_str("def f():");
        ed.insert_newline(false);
        ed.insert_str("    return 1");
        ed.insert_newline(false);
        ed.insert_str("x = 2");
        ed.cursor_line = 0;
        assert_eq!(ed.foldable_range_at(0), Some((0, 1)));
        ed.toggle_fold();
        assert_eq!(ed.visible_rows_from(0, 10), vec![0, 2]);
    }

    #[test]
    fn cursor_snaps_out_of_collapsed_fold() {
        let mut ed = Editor::empty();
        ed.insert_str("fn main() {");
        ed.insert_newline(false);
        ed.insert_str("    a();");
        ed.insert_newline(false);
        ed.insert_str("}");
        ed.cursor_line = 0;
        ed.toggle_fold();
        ed.cursor_line = 1;
        ed.cursor_col = 0;
        ed.move_right();
        assert_eq!(ed.cursor_line, 0);
    }

    #[test]
    fn undo_redo_roundtrip_and_coalescing() {
        let mut ed = Editor::empty();
        ed.insert_char('a');
        ed.insert_char('b');
        ed.insert_char('c');
        assert_eq!(ed.rope.to_string(), "abc");
        // Consecutive inserts coalesce into a single undo step.
        assert!(ed.undo());
        assert_eq!(ed.rope.to_string(), "");
        assert!(ed.redo());
        assert_eq!(ed.rope.to_string(), "abc");
        // A cursor move breaks the run, so the next insert is its own step.
        ed.break_undo_coalescing();
        ed.insert_char('d');
        assert_eq!(ed.rope.to_string(), "abcd");
        assert!(ed.undo()); // removes just 'd'
        assert_eq!(ed.rope.to_string(), "abc");
        assert!(ed.undo()); // removes the "abc" run
        assert_eq!(ed.rope.to_string(), "");
        assert!(!ed.undo()); // nothing left
    }

    #[test]
    fn word_motion_and_delete() {
        let mut ed = Editor::empty();
        ed.insert_str("hello world foo");
        ed.cursor_col = 0;
        ed.move_word_right();
        assert_eq!(ed.cursor_col, 5); // end of "hello"
        ed.move_word_right();
        assert_eq!(ed.cursor_col, 11); // end of "world"
        ed.move_word_left();
        assert_eq!(ed.cursor_col, 6); // start of "world"
        ed.cursor_col = 15;
        ed.delete_word_left();
        assert_eq!(ed.rope.to_string(), "hello world ");
    }

    #[test]
    fn duplicate_and_move_lines() {
        let mut ed = Editor::empty();
        ed.insert_str("one");
        ed.insert_newline(false);
        ed.insert_str("two");
        ed.cursor_line = 0;
        ed.cursor_col = 0;
        ed.duplicate_line();
        assert_eq!(ed.rope.to_string(), "one\none\ntwo");
        assert_eq!(ed.cursor_line, 1);
        ed.move_line_down();
        assert_eq!(ed.rope.to_string(), "one\ntwo\none");
        assert_eq!(ed.cursor_line, 2);
        ed.move_line_up();
        assert_eq!(ed.rope.to_string(), "one\none\ntwo");
    }

    #[test]
    fn toggle_comment_current_line() {
        let mut ed = Editor::empty();
        ed.insert_str("    let x = 1;");
        ed.toggle_comment("//");
        assert_eq!(ed.rope.to_string(), "    // let x = 1;");
        ed.toggle_comment("//");
        assert_eq!(ed.rope.to_string(), "    let x = 1;");
    }

    #[test]
    fn comment_token_by_extension() {
        assert_eq!(comment_token(Some(std::path::Path::new("a.rs"))), Some("//"));
        assert_eq!(comment_token(Some(std::path::Path::new("a.py"))), Some("#"));
        assert_eq!(comment_token(Some(std::path::Path::new("a.unknownext"))), None);
        // `.m` is Octave here, like everywhere else in the app.
        assert_eq!(comment_token(Some(std::path::Path::new("plot.m"))), Some("%"));
        assert_eq!(comment_token(Some(std::path::Path::new("api.ts"))), Some("//"));
        assert_eq!(comment_token(Some(std::path::Path::new("main.tf"))), Some("#"));
        // No extension to go on: the file name is the only thing that says what it is.
        assert_eq!(comment_token(Some(std::path::Path::new("src/Makefile"))), Some("#"));
        assert_eq!(comment_token(Some(std::path::Path::new("Dockerfile"))), Some("#"));
        // Block comments only: better left alone than half-applied per line.
        assert_eq!(comment_token(Some(std::path::Path::new("index.html"))), None);
        assert_eq!(comment_token(Some(std::path::Path::new("main.css"))), None);
    }

    #[test]
    fn crlf_line_endings_preserved_on_save() {
        let dir = std::env::temp_dir().join(format!("clicode_crlf_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("crlf.txt");
        std::fs::write(&path, "a\r\nb\r\n").unwrap();
        let mut ed = Editor::open(path.clone()).unwrap();
        assert_eq!(ed.line_ending, LineEnding::Crlf);
        assert_eq!(ed.rope.to_string(), "a\nb\n"); // normalized internally
        ed.dirty = true;
        ed.save().unwrap();
        let raw = std::fs::read(&path).unwrap();
        assert_eq!(raw, b"a\r\nb\r\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Mirrors the Edit menu's "Convert line endings": flip `line_ending` directly (that
    /// command does nothing more than this and mark the buffer dirty — see
    /// `App::run_menu_action`) and check `save` honours the new setting rather than the one
    /// the file was opened with. Both directions, and `final_newline` untouched by either.
    #[test]
    fn converted_line_ending_is_what_save_writes() {
        let dir = std::env::temp_dir().join(format!("clicode_convert_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let crlf_path = dir.join("was_crlf.txt");
        std::fs::write(&crlf_path, "a\r\nb\r\n").unwrap();
        let mut ed = Editor::open(crlf_path.clone()).unwrap();
        assert_eq!(ed.line_ending, LineEnding::Crlf);
        assert!(ed.final_newline);
        ed.line_ending = LineEnding::Lf;
        ed.dirty = true;
        ed.save().unwrap();
        let raw = std::fs::read(&crlf_path).unwrap();
        assert_eq!(raw, b"a\nb\n");
        assert!(ed.final_newline, "converting the ending must not touch the final newline");

        let lf_path = dir.join("was_lf.txt");
        std::fs::write(&lf_path, "c\nd").unwrap(); // no trailing newline
        let mut ed = Editor::open(lf_path.clone()).unwrap();
        assert_eq!(ed.line_ending, LineEnding::Lf);
        assert!(!ed.final_newline);
        ed.line_ending = LineEnding::Crlf;
        ed.dirty = true;
        ed.save().unwrap();
        let raw = std::fs::read(&lf_path).unwrap();
        assert_eq!(raw, b"c\r\nd");
        assert!(!ed.final_newline, "converting the ending must not invent a final newline");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn binary_file_is_read_only_and_refuses_save() {
        let dir = std::env::temp_dir().join(format!("clicode_bin_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("data.bin");
        std::fs::write(&path, [0u8, 1, 2, 3, 0]).unwrap();
        let mut ed = Editor::open(path.clone()).unwrap();
        assert!(ed.is_read_only());
        assert!(ed.save().is_err());
        // Original bytes are untouched.
        assert_eq!(std::fs::read(&path).unwrap(), vec![0u8, 1, 2, 3, 0]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `read_only` is set on binary buffers and picture previews, neither of which has a
    /// meaningful notion of "edit". Every mutating entry point is expected to check the flag
    /// itself (there is no single chokepoint they all funnel through), so this drives a
    /// representative sample of them and checks that none leaves a mark: not on the text, not
    /// on the dirty bit, and not on the undo stack (an edit that quietly checkpointed but did
    /// nothing would still be a bug — undo would then produce a no-op step).
    #[test]
    fn a_read_only_buffer_refuses_every_edit_entry_point() {
        let mut ed = Editor::empty();
        ed.insert_str("abc");
        ed.insert_newline(false);
        ed.insert_str("def");
        ed.selection_anchor = Some((0, 0));
        ed.cursor_line = 0;
        ed.cursor_col = 2;
        ed.read_only = true;

        let text_before = ed.rope.to_string();
        let dirty_before = ed.dirty;
        let undo_depth_before = ed.undo_stack.len();

        assert!(!ed.delete_selection());
        ed.insert_char('z');
        ed.insert_char_pairs('(', true);
        ed.newline_smart(true, true, "    ");
        ed.insert_str("more");
        ed.insert_multiline("x\ny");
        ed.insert_newline(true);
        ed.backspace();
        ed.delete_forward();
        ed.delete_word_left();
        ed.delete_word_right();
        ed.indent_selection(4);
        ed.outdent_selection(4);
        ed.duplicate_line();
        ed.move_line_up();
        ed.move_line_down();
        ed.toggle_comment("//");
        ed.replace_char_range(0, 1, "Z");
        // The formatting bar's eleven, which reach the rope by their own surgery rather than
        // through any of the entry points above.
        assert!(!ed.md_toggle_inline("**"));
        assert!(!ed.md_toggle_inline("*"));
        assert!(!ed.md_toggle_inline("`"));
        assert!(!ed.md_toggle_inline("~~"));
        assert!(!ed.md_cycle_heading());
        assert!(!ed.md_toggle_bullet());
        assert!(!ed.md_toggle_numbered());
        assert!(!ed.md_toggle_task());
        assert!(!ed.md_toggle_quote());
        assert!(!ed.md_insert_link("text"));
        assert!(!ed.md_toggle_fence());

        assert_eq!(ed.rope.to_string(), text_before, "a read-only buffer must not be mutated");
        assert_eq!(ed.dirty, dirty_before);
        assert_eq!(ed.undo_stack.len(), undo_depth_before, "no checkpoint should have been pushed either");
    }

    // ---- Markdown formatting --------------------------------------------------------

    /// A buffer holding exactly `text`, with an empty history: these tests care about how many
    /// undo steps an action leaves behind, so the setting-up must leave none.
    fn md_buffer(text: &str) -> Editor {
        let mut ed = Editor::empty();
        ed.rope = Rope::from_str(text);
        ed
    }

    /// Every action, on a buffer each can act on. Named, so a failure says which one.
    fn md_actions() -> Vec<(&'static str, fn(&mut Editor) -> bool)> {
        vec![
            ("bold", |ed| ed.md_toggle_inline("**")),
            ("italic", |ed| ed.md_toggle_inline("*")),
            ("code", |ed| ed.md_toggle_inline("`")),
            ("strike", |ed| ed.md_toggle_inline("~~")),
            ("heading", |ed| ed.md_cycle_heading()),
            ("bullet", |ed| ed.md_toggle_bullet()),
            ("numbered", |ed| ed.md_toggle_numbered()),
            ("task", |ed| ed.md_toggle_task()),
            ("quote", |ed| ed.md_toggle_quote()),
            ("link", |ed| ed.md_insert_link("text")),
            ("fence", |ed| ed.md_toggle_fence()),
        ]
    }

    /// A toggle that only ever adds is not a toggle: the same button pressed twice has to leave
    /// the text where it found it. Which is why the selection is put back on the *inner* text
    /// rather than on what was written — left over the markers, the second press would read the
    /// stars as part of the selection and wrap them again.
    #[test]
    fn bold_wraps_a_selection_and_the_next_press_unwraps_it() {
        let mut ed = md_buffer("hello world");
        ed.selection_anchor = Some((0, 0));
        ed.cursor_col = 5;
        assert!(ed.md_toggle_inline("**"));
        assert_eq!(ed.rope.to_string(), "**hello** world");
        assert_eq!(ed.selected_text().as_deref(), Some("hello"));
        assert!(ed.md_toggle_inline("**"));
        assert_eq!(ed.rope.to_string(), "hello world");
        assert_eq!(ed.selected_text().as_deref(), Some("hello"));
    }

    /// Pressing bold with the caret in a word means that word. Without this the button was only
    /// useful after a selection, which is two gestures for the thing every word processor does
    /// in one.
    #[test]
    fn bold_with_no_selection_takes_the_word_the_cursor_is_in() {
        for col in [0, 2, 5] {
            let mut ed = md_buffer("hello world");
            ed.cursor_col = col;
            assert!(ed.md_toggle_inline("**"), "column {col}");
            assert_eq!(ed.rope.to_string(), "**hello** world", "column {col}");
        }
    }

    /// And with no word to take, the pair is left behind with the caret between its halves —
    /// the way typing an opening bracket behaves. Landing after the pair would mean deleting
    /// four characters to get out of a mistake.
    #[test]
    fn bold_on_an_empty_line_leaves_the_cursor_between_the_markers() {
        let mut ed = md_buffer("");
        assert!(ed.md_toggle_inline("**"));
        assert_eq!(ed.rope.to_string(), "****");
        assert_eq!(ed.cursor_col, 2);
    }

    /// Presence in the `*` family is the length of the run, not a string match. Reading `**x**`
    /// as "italic is already there" is what a `starts_with("*")` test does, and it made the
    /// italic button strip a word's bold instead of adding to it.
    #[test]
    fn italic_over_bold_adds_a_star_rather_than_finding_one() {
        let mut ed = md_buffer("**x**");
        ed.selection_anchor = Some((0, 2));
        ed.cursor_col = 3;
        assert!(ed.md_toggle_inline("*"));
        assert_eq!(ed.rope.to_string(), "***x***");
        assert_eq!(ed.selected_text().as_deref(), Some("x"));
        assert!(ed.md_toggle_inline("*"));
        assert_eq!(ed.rope.to_string(), "**x**", "the bold has to survive the italic going away");
    }

    /// Four levels and back to none, so one button reaches every heading a document needs and
    /// the way out is pressing it again rather than deleting hashes by hand.
    #[test]
    fn the_heading_button_cycles_round_to_plain_text() {
        let mut ed = md_buffer("Title");
        for expected in ["# Title", "## Title", "### Title", "Title"] {
            assert!(ed.md_cycle_heading());
            assert_eq!(ed.rope.to_string(), expected);
        }
    }

    /// A blank line inside a selected span is not a list item, and prefixing it would put a
    /// dash on the empty line that separates two paragraphs. It must also not count towards
    /// "are they all bulleted already", or a span with one blank line in it could never be
    /// un-bulleted.
    #[test]
    fn a_blank_line_in_the_span_gets_no_bullet_and_does_not_decide_the_toggle() {
        let mut ed = md_buffer("one\n\nthree");
        ed.selection_anchor = Some((0, 0));
        ed.cursor_line = 2;
        ed.cursor_col = 5;
        assert!(ed.md_toggle_bullet());
        assert_eq!(ed.rope.to_string(), "- one\n\n- three");
        assert!(ed.md_toggle_bullet());
        assert_eq!(ed.rope.to_string(), "one\n\nthree");
    }

    /// Numbers count down the span rather than repeating `1.`, and a span already numbered
    /// loses the numbers instead of gaining a second set.
    #[test]
    fn a_numbered_list_numbers_its_lines_and_comes_off_again() {
        let mut ed = md_buffer("a\nb\nc");
        ed.selection_anchor = Some((0, 0));
        ed.cursor_line = 2;
        ed.cursor_col = 1;
        assert!(ed.md_toggle_numbered());
        assert_eq!(ed.rope.to_string(), "1. a\n2. b\n3. c");
        assert!(ed.md_toggle_numbered());
        assert_eq!(ed.rope.to_string(), "a\nb\nc");
        // `1)` is the other spelling markdown accepts, and a list written that way has to be
        // recognised as one or the button would number it a second time.
        let mut ed = md_buffer("1) a");
        assert!(ed.md_toggle_numbered());
        assert_eq!(ed.rope.to_string(), "a");
    }

    /// A ticked box is still a task line. Matching only `- [ ] ` left a list you had worked
    /// through as the one list the button could not clear.
    #[test]
    fn the_task_button_recognises_a_box_that_has_been_ticked() {
        let mut ed = md_buffer("buy milk");
        assert!(ed.md_toggle_task());
        assert_eq!(ed.rope.to_string(), "- [ ] buy milk");
        assert!(ed.md_toggle_task());
        assert_eq!(ed.rope.to_string(), "buy milk");

        let mut ed = md_buffer("- [x] done");
        assert!(ed.md_toggle_task());
        assert_eq!(ed.rope.to_string(), "done");

        // A plain bullet is promoted in place: `- - [ ] item` is not a checkbox.
        let mut ed = md_buffer("- item");
        assert!(ed.md_toggle_task());
        assert_eq!(ed.rope.to_string(), "- [ ] item");
        // And the bullet button leaves a checkbox alone rather than turning it into `[ ] item`,
        // which looks like a checkbox and is not one.
        let mut ed = md_buffer("- [ ] item");
        assert!(ed.md_toggle_bullet());
        assert_eq!(ed.rope.to_string(), "- [ ] item");
    }

    /// The fences come off as whole lines, the closing one taking the newline in front of it —
    /// removing it with the newline *after* it left a blank line at the end of the file that
    /// had never been there.
    #[test]
    fn a_code_fence_wraps_the_span_and_the_next_press_takes_both_lines_away() {
        let mut ed = md_buffer("x");
        assert!(ed.md_toggle_fence());
        assert_eq!(ed.rope.to_string(), "```\nx\n```");
        assert_eq!(ed.cursor_line, 1, "the caret stays inside the block");
        assert!(ed.md_toggle_fence());
        assert_eq!(ed.rope.to_string(), "x");
    }

    /// The address is what nobody has typed yet, so it is what is selected: the next keystroke
    /// replaces it. Leaving the caret at the end meant selecting `url` by hand every time.
    #[test]
    fn a_link_leaves_its_address_selected() {
        let mut ed = md_buffer("click here");
        ed.selection_anchor = Some((0, 0));
        ed.cursor_col = 5;
        assert!(ed.md_insert_link("text"));
        assert_eq!(ed.rope.to_string(), "[click](url) here");
        assert_eq!(ed.selected_text().as_deref(), Some("url"));

        // With nothing selected the label is the placeholder, and that is what is selected.
        let mut ed = md_buffer("");
        assert!(ed.md_insert_link("text"));
        assert_eq!(ed.rope.to_string(), "[text](url)");
        assert_eq!(ed.selected_text().as_deref(), Some("text"));
    }

    /// Each of these is one gesture, so each has to be one Ctrl+Z. Built out of several rope
    /// operations apiece, they would otherwise checkpoint once per operation and leave a user
    /// pressing undo three times to get back a word.
    #[test]
    fn every_markdown_action_undoes_in_a_single_step() {
        for (name, act) in md_actions() {
            let mut ed = md_buffer("one two\nthree four");
            ed.selection_anchor = Some((0, 0));
            ed.cursor_col = 3;
            let before = ed.rope.to_string();
            assert!(act(&mut ed), "{name} did nothing to act on");
            assert_ne!(ed.rope.to_string(), before, "{name} claimed to have changed something");
            assert!(ed.undo(), "{name} left no undo step at all");
            assert_eq!(ed.rope.to_string(), before, "{name} takes more than one undo");
        }
    }

    /// A rectangle is neither a run of text to wrap nor a span of lines to prefix: every one of
    /// these refuses it outright rather than guessing which of the two it meant. Checked on the
    /// history as well — an action that checkpointed and then did nothing would leave an undo
    /// step that undoes nothing.
    #[test]
    fn a_column_selection_stops_every_markdown_action() {
        for (name, act) in md_actions() {
            let mut ed = md_buffer("one two\nthree four");
            ed.selection_anchor = Some((0, 0));
            ed.cursor_line = 1;
            ed.cursor_col = 3;
            ed.selection_block = true;
            let before = ed.rope.to_string();
            assert!(!act(&mut ed), "{name} acted on a rectangle");
            assert_eq!(ed.rope.to_string(), before, "{name}");
            assert!(ed.undo_stack.is_empty(), "{name} checkpointed for nothing");
        }
    }

    /// A selection crossing lines would put the closing marker in a paragraph the opening one
    /// is not in, which markdown does not render as anything. Refused, and said so, rather than
    /// written and left looking like a bug in the renderer.
    #[test]
    fn an_inline_marker_refuses_a_selection_that_crosses_lines() {
        let mut ed = md_buffer("one\ntwo");
        ed.selection_anchor = Some((0, 0));
        ed.cursor_line = 1;
        ed.cursor_col = 3;
        assert!(!ed.md_toggle_inline("**"));
        assert!(!ed.md_insert_link("text"));
        assert_eq!(ed.rope.to_string(), "one\ntwo");
        assert!(ed.undo_stack.is_empty());
    }

    #[test]
    fn auto_close_inserts_and_steps_over() {
        let mut ed = Editor::empty();
        ed.insert_char_pairs('(', true);
        assert_eq!(ed.rope.to_string(), "()");
        assert_eq!(ed.cursor_col, 1); // between the pair
        // Typing the closing bracket steps over instead of duplicating.
        ed.insert_char_pairs(')', true);
        assert_eq!(ed.rope.to_string(), "()");
        assert_eq!(ed.cursor_col, 2);
    }

    #[test]
    fn auto_close_backspace_deletes_pair() {
        let mut ed = Editor::empty();
        ed.insert_char_pairs('[', true);
        assert_eq!(ed.rope.to_string(), "[]");
        ed.backspace();
        assert_eq!(ed.rope.to_string(), "");
        assert_eq!(ed.cursor_col, 0);
    }

    #[test]
    fn quote_not_paired_next_to_word() {
        let mut ed = Editor::empty();
        ed.insert_str("dont");
        ed.insert_char_pairs('\'', true); // apostrophe after a word: no pairing
        assert_eq!(ed.rope.to_string(), "dont'");
    }

    #[test]
    fn newline_smart_expands_brace_block() {
        let mut ed = Editor::empty();
        ed.insert_char_pairs('{', true);
        assert_eq!(ed.rope.to_string(), "{}");
        ed.newline_smart(true, true, "    ");
        assert_eq!(ed.rope.to_string(), "{\n    \n}");
        assert_eq!(ed.cursor_line, 1);
        assert_eq!(ed.cursor_col, 4);
    }

    #[test]
    fn external_change_reloads_when_not_dirty() {
        let dir = std::env::temp_dir().join(format!("clicode_test_ext_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.txt");
        std::fs::write(&path, "original").unwrap();
        let mut ed = Editor::open(path.clone()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&path, "changed on disk").unwrap();
        let msg = ed.check_external_changes(Lang::En);
        assert!(msg.is_some());
        assert_eq!(ed.rope.to_string(), "changed on disk");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The safety rule of 0.13, which the live-file work must not have loosened: a buffer with
    /// unsaved edits is never overwritten by what is on disk, however new that is. The user's
    /// work wins over the agent's, always — and nothing lights up in the gutter either, because
    /// the text on screen is still the text you typed.
    #[test]
    fn a_dirty_buffer_is_never_reloaded_underneath_you() {
        let dir = std::env::temp_dir().join(format!("clee_dirty_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mine.txt");
        std::fs::write(&path, "one\ntwo\n").unwrap();
        let mut ed = Editor::open(path.clone()).unwrap();
        ed.insert_str("hand-typed ");
        assert!(ed.dirty);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&path, "written by somebody else\n").unwrap();

        let said = ed.check_external_changes(Lang::En);
        assert!(said.is_some(), "the status line has to say the two versions have parted");
        assert_eq!(ed.rope.to_string(), "hand-typed one\ntwo\n", "unsaved work was thrown away");
        assert!(ed.arrived_lines().is_empty());
        // And it says it once: the new mtime is remembered even though nothing was loaded, so
        // the message does not come back every tick.
        assert!(ed.check_external_changes(Lang::En).is_none());
        assert_eq!(ed.rope.to_string(), "hand-typed one\ntwo\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The whole of 3a end to end: a file rewritten from outside lights the lines that arrived,
    /// and the first key of your own puts them out.
    #[test]
    fn a_reload_lights_the_lines_that_arrived_and_an_edit_puts_them_out() {
        let dir = std::env::temp_dir().join(format!("clee_arrived_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agent.txt");
        std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
        let mut ed = Editor::open(path.clone()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        // What an agent does: the whole file, written again, with one line different.
        std::fs::write(&path, "one\nTWO\nthree\n").unwrap();

        assert!(ed.check_external_changes(Lang::En).is_some());
        assert_eq!(ed.arrived_lines(), [1]);
        assert!(!ed.line_arrived(0) && !ed.line_arrived(2));

        ed.insert_char('x');
        assert!(ed.arrived_lines().is_empty(), "typing leaves the marks describing a file that is gone");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The declared large-file mode, reached through the limit seam rather than through fifty
    /// megabytes of fixture: the flag, the shallower undo history, and the size the status
    /// message quotes.
    ///
    /// The undo half is the one that matters most, and it is checked by counting: a snapshot is
    /// the whole text, so a history that kept its normal depth on a very large file is the bug
    /// this mode exists to prevent, and "the depth is smaller" is not the same claim as "the
    /// stack actually stops there".
    #[test]
    fn a_large_file_opens_with_no_colours_and_a_short_history() {
        let dir = std::env::temp_dir().join(format!("clee_large_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let big = dir.join("big.txt");
        std::fs::write(&big, "x".repeat(3 * 1024 * 1024)).unwrap();
        let mut ed = Editor::open_with_limit(big.clone(), 1024 * 1024).unwrap();
        assert!(ed.is_large());
        assert_eq!(ed.megabytes(), 3);
        assert_eq!(ed.undo_depth(), MAX_UNDO_LARGE);

        // Each step is its own, because `Other` never coalesces: MAX_UNDO_LARGE + 5 edits, and
        // the oldest five must have been dropped off the front rather than kept.
        for _ in 0..MAX_UNDO_LARGE + 5 {
            ed.checkpoint(EditKind::Other);
        }
        assert_eq!(ed.undo_stack.len(), MAX_UNDO_LARGE);

        // The same file under the real limit is an ordinary buffer: the mode is the size, not
        // the file, and nothing about it is sticky.
        let small = dir.join("small.txt");
        std::fs::write(&small, "one\ntwo\n").unwrap();
        let ed = Editor::open(small).unwrap();
        assert!(!ed.is_large());
        assert_eq!(ed.undo_depth(), MAX_UNDO);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A file can cross the line while it is open — a log being written to, a build artefact
    /// regenerated — so the reload re-measures instead of trusting what the open decided. Both
    /// directions, because a file truncated back under the line should get its colours back.
    #[test]
    fn a_reload_re_decides_the_large_file_mode() {
        let dir = std::env::temp_dir().join(format!("clee_large_reload_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("growing.log");
        std::fs::write(&path, "one\ntwo\n").unwrap();
        let mut ed = Editor::open_with_limit(path.clone(), 1024).unwrap();
        assert!(!ed.is_large());

        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&path, "line\n".repeat(1000)).unwrap();
        assert!(ed.check_external_changes(Lang::En).is_some());
        assert!(ed.is_large(), "a file that grew past the line is in the mode from now on");
        assert_eq!(ed.undo_depth(), MAX_UNDO_LARGE);

        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&path, "truncated\n").unwrap();
        assert!(ed.check_external_changes(Lang::En).is_some());
        assert!(!ed.is_large(), "and out of it again when the file is back under the line");
        assert_eq!(ed.undo_depth(), MAX_UNDO);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The diff itself, away from any file: insertions, deletions, replacements, a file that
    /// came back identical, and the ceiling.
    #[test]
    fn the_line_diff_names_the_lines_that_are_new() {
        // Identical: nothing arrived, however long the file.
        assert!(changed_lines("a\nb\nc\n", "a\nb\nc\n").is_empty());

        // An insertion in the middle: only the inserted line, and it is named by its place in
        // the *new* text.
        assert_eq!(changed_lines("a\nb\n", "a\nnew\nb\n"), vec![1]);

        // A deletion leaves nothing to light.
        assert!(changed_lines("a\nb\nc\n", "a\nc\n").is_empty());

        // A replacement is one line, not two: the lines around it are recognised.
        assert_eq!(changed_lines("a\nb\nc\n", "a\nB\nc\n"), vec![1]);

        // A block appended, and a block prepended.
        assert_eq!(changed_lines("a\n", "a\nb\nc\n"), vec![1, 2]);
        assert_eq!(changed_lines("a\n", "b\nc\na\n"), vec![0, 1]);

        // From nothing, and to nothing.
        assert_eq!(changed_lines("", "a\nb\n"), vec![0, 1]);
        assert!(changed_lines("a\nb\n", "").is_empty());

        // Lines that moved rather than changed: the longest common subsequence keeps the run it
        // can and calls the rest arrivals, which is what a reader wants to look at.
        assert_eq!(changed_lines("a\nb\nc\nd\n", "a\nc\nb\nd\n"), vec![2]);
    }

    /// Past the ceiling nothing is marked — the documented answer, and the one that keeps a
    /// quadratic table from being asked for on a file where it would not fit.
    #[test]
    fn a_wide_enough_difference_is_left_unmarked() {
        let old: String = (0..DIFFERING_LINES_CAP + 1).map(|i| format!("old {i}\n")).collect();
        let new: String = (0..DIFFERING_LINES_CAP + 1).map(|i| format!("new {i}\n")).collect();
        assert!(changed_lines(&old, &new).is_empty(), "the ceiling is meant to give up quietly");

        // The ceiling is on the differing part, not on the file: a long file with one line
        // rewritten in the middle is still answered exactly.
        let mut lines: Vec<String> = (0..20_000).map(|i| format!("line {i}\n")).collect();
        let long: String = lines.concat();
        lines[9_999] = "written by somebody else\n".to_string();
        assert_eq!(changed_lines(&long, &lines.concat()), vec![9_999]);
    }

    /// What decides whether opening a file makes a buffer or shows the file instead, so it has
    /// to be right about ordinary source files above all: calling one of those binary would
    /// send it to a terminal instead of opening it.
    #[test]
    fn binary_is_judged_from_the_head_of_the_file() {
        let dir = std::env::temp_dir().join(format!("clee_bin_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, bytes: &[u8]| {
            let p = dir.join(name);
            std::fs::write(&p, bytes).unwrap();
            p
        };

        assert!(!Editor::looks_binary(&write("a.rs", b"fn main() {}\n")));
        assert!(!Editor::looks_binary(&write("empty.txt", b"")));
        // Accented text is multi-byte UTF-8, never NUL — the case a naive byte check gets wrong.
        assert!(!Editor::looks_binary(&write("it.md", "però è così — ok\n".as_bytes())));

        // A PNG's signature carries NULs in its first bytes.
        assert!(Editor::looks_binary(&write("i.png", b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR")));
        assert!(Editor::looks_binary(&write("mid.bin", b"text then\x00a nul")));

        // Only the head is read, so a NUL past it is not found — deliberate, and the reason a
        // 20 MB picture is not slurped whole just to be classified.
        let mut late = vec![b'x'; 9000];
        late.push(0);
        assert!(!Editor::looks_binary(&write("late.bin", &late)));

        // A file that isn't there is not binary; opening it reports its own error.
        assert!(!Editor::looks_binary(&dir.join("nope.txt")));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The wheel used to be able to scroll exactly one screen: `adjust_scroll` ran every frame,
    /// so the moment the cursor line left the viewport the view was yanked back to it, undoing
    /// each further notch before it was drawn. The view has to be allowed to leave the cursor
    /// behind, and to come back only when the cursor itself moves.
    #[test]
    fn the_wheel_can_scroll_past_the_cursor_and_the_cursor_still_pulls_the_view_back() {
        let rows = 10;
        let mut ed = Editor::empty();
        ed.rope = Rope::from_str(&(0..200).map(|i| format!("line {i}\n")).collect::<String>());

        // A first frame with everything where it started leaves the view alone.
        ed.follow_cursor(rows, 80);
        assert_eq!(ed.top_line, 0);

        // Two screens' worth of notches, with the cursor left on line 0 — the case that used to
        // stop dead at the first screen.
        for _ in 0..7 {
            ed.top_line += 3;
            ed.follow_cursor(rows, 80);
        }
        assert_eq!(ed.top_line, 21, "the view goes where it is sent and stays there");

        // Moving the cursor is what asks the view to follow: it comes back, showing the cursor.
        ed.cursor_line = 4;
        ed.follow_cursor(rows, 80);
        assert_eq!(ed.top_line, 4);

        // A viewport that shrinks under a still cursor must also keep it on screen: the cursor is
        // on the last visible row and the frame loses half its height.
        ed.cursor_line = 13;
        ed.follow_cursor(rows, 80);
        assert_eq!(ed.top_line, 4, "still visible, so nothing moves");
        ed.follow_cursor(5, 80);
        assert_eq!(ed.top_line, 9);
    }

    /// The scrollbars appear off one comparison made once a frame, so what counts as "the view
    /// moved" has to be exactly that and nothing else — editing or moving the cursor within the
    /// visible text must not flash them up.
    #[test]
    fn only_a_moved_view_counts_as_scrolling() {
        let window = Duration::from_millis(500);
        let mut ed = Editor::empty();

        // A first look at a view that has never moved is not a scroll.
        ed.observe_scroll();
        assert!(!ed.scrolled_within(window));

        ed.top_line = 12;
        ed.observe_scroll();
        assert!(ed.scrolled_within(window));

        // Frames where nothing moves leave the timestamp where it was rather than refreshing
        // it, which is what lets the bars ever fade out.
        ed.observe_scroll();
        assert!(!ed.scrolled_within(Duration::ZERO));

        // Sideways counts too, and is what the horizontal bar rides on.
        ed.left_col = 40;
        ed.observe_scroll();
        assert!(ed.scrolled_within(window));

        // Typing moves the cursor, not the view: no bar.
        let mut quiet = Editor::empty();
        quiet.observe_scroll();
        quiet.cursor_col = 7;
        quiet.dirty = true;
        quiet.observe_scroll();
        assert!(!quiet.scrolled_within(window));
    }

    /// A save must never be able to leave less on disk than it started with, and must not
    /// quietly change what the file *is*: the new content arrives whole, and a mode the user set
    /// deliberately — an executable script, a file only they can read — survives it. Writing
    /// through a temp file gets the first for free and would lose the second, since a fresh file
    /// is born from the umask rather than from the one it replaces.
    #[test]
    fn saving_replaces_the_file_whole_and_keeps_the_mode_it_had() {
        let dir = std::env::temp_dir().join(format!("clee_save_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("script.sh");
        std::fs::write(&path, "#!/bin/sh\necho old\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut ed = Editor::open(path.clone()).unwrap();
        ed.rope = Rope::from_str("#!/bin/sh\necho new\n");
        ed.dirty = true;
        ed.save().expect("a writable file must save");

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "#!/bin/sh\necho new\n");
        assert!(!ed.dirty);
        // The scratch file is a means, not a leftover: nothing but the file itself remains.
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left.len(), 1, "{left:?}");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755, "an executable script is still executable after a save");
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
