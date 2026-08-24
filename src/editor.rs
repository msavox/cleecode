use crate::i18n::{self, Key, Lang};
use anyhow::Result;
use ratatui::style::Style;
use ropey::Rope;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

/// How lines are terminated on disk, detected on open and reapplied on save so we never
/// silently rewrite a file's line endings (a spurious full-file diff on the first save).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineEnding {
    Lf,
    Crlf,
}

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
    pub disk_mtime: Option<SystemTime>,
    pub highlighted: Vec<Vec<(Style, String)>>,
    pub syntax_dirty: bool,
    pub selection_anchor: Option<(usize, usize)>,
    /// Whether the selection is a rectangle rather than a run of text. A column selection has
    /// the same two endpoints; what changes is which cells between them count as inside.
    pub selection_block: bool,
    /// Active (collapsed) fold regions as (start_line, end_line), inclusive, sorted by start.
    pub folds: Vec<(usize, usize)>,
    /// Line-ending style detected on open, reapplied on save.
    pub line_ending: LineEnding,
    /// Whether the file ended with a trailing newline when opened; preserved on save.
    pub final_newline: bool,
    /// Set when the file couldn't be loaded as text (binary/undecodable, or a read error).
    /// Such a buffer is display-only and refuses to save, so we never truncate the original.
    pub read_only: bool,
    /// Set instead of a text buffer when the file is a picture: the tab draws it rather than
    /// pretending to hold lines. Every text operation already refuses on `read_only`, which
    /// such a tab always is, so this only has to change what gets drawn.
    pub preview: Option<crate::preview::Preview>,
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
    last_edit: EditKind,
    /// While set, nested mutations skip their own checkpoint so a compound edit (paste,
    /// line move, comment toggle) collapses into a single undo step.
    in_compound: bool,
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
            disk_mtime: None,
            highlighted: Vec::new(),
            syntax_dirty: true,
            selection_anchor: None,
            selection_block: false,
            folds: Vec::new(),
            line_ending: LineEnding::Lf,
            final_newline: false,
            read_only: false,
            preview: None,
            undo_stack: Vec::new(),
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
        let disk_mtime = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());
        let mut editor = Editor::empty();
        editor.disk_mtime = disk_mtime;

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
        std::fs::write(&path, text)?;
        self.dirty = false;
        self.disk_mtime = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());
        Ok(())
    }

    /// True if the buffer has content that can meaningfully be edited/saved. Used to gate
    /// the read-only guard's messaging.
    pub fn is_read_only(&self) -> bool {
        self.read_only
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
        self.line_ending = if content.contains("\r\n") { LineEnding::Crlf } else { LineEnding::Lf };
        self.final_newline = content.ends_with('\n');
        self.rope = Rope::from_str(&content.replace("\r\n", "\n"));
        self.disk_mtime = Some(mtime);
        self.syntax_dirty = true;
        self.revision = self.revision.wrapping_add(1);
        self.folds.clear();
        // A silent reload starts a fresh edit timeline; the old history refers to gone text.
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.last_edit = EditKind::None;
        let max_line = self.rope.len_lines().saturating_sub(1);
        self.cursor_line = self.cursor_line.min(max_line);
        self.cursor_col = self.cursor_col.min(self.line_char_len(self.cursor_line));
        Some(i18n::msg_externally_reloaded(lang, &self.title(lang)))
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

    fn char_idx(&self, line: usize, col: usize) -> usize {
        self.rope.line_to_char(line) + col
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
    pub fn selected_columns(&self, line: usize) -> Option<(usize, usize)> {
        let len = self.line_char_len(line);
        if let Some((first, last, c0, c1)) = self.block_range() {
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

    pub fn selected_text(&self) -> Option<String> {
        if let Some((first, last, _, _)) = self.block_range() {
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

    /// Records the pre-edit state for undo. Must be called BEFORE mutating the rope.
    /// Consecutive same-kind character edits coalesce into a single undo step (typing a
    /// word undoes as one), while `Other` edits always start a new step. No-op inside a
    /// compound edit, which checkpoints once up front.
    fn checkpoint(&mut self, kind: EditKind) {
        if self.in_compound {
            return;
        }
        self.redo_stack.clear();
        let coalesce = kind != EditKind::Other && kind == self.last_edit && !self.undo_stack.is_empty();
        if !coalesce {
            const MAX_UNDO: usize = 500;
            self.undo_stack.push(self.snapshot());
            if self.undo_stack.len() > MAX_UNDO {
                self.undo_stack.remove(0);
            }
        }
        self.last_edit = kind;
    }

    fn restore(&mut self, snap: Snapshot) {
        self.rope = Rope::from_str(&snap.text);
        let max_line = self.rope.len_lines().saturating_sub(1);
        self.cursor_line = snap.cursor_line.min(max_line);
        self.cursor_col = snap.cursor_col.min(self.line_char_len(self.cursor_line));
        self.selection_anchor = None;
        self.mark_edited();
        self.folds.clear();
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo_stack.pop() else { return false };
        let current = self.snapshot();
        self.redo_stack.push(current);
        self.restore(prev);
        self.last_edit = EditKind::None;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo_stack.pop() else { return false };
        let current = self.snapshot();
        self.undo_stack.push(current);
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
            self.selection_anchor = None;
            self.selection_block = false;
            self.mark_edited();
            return true;
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
        self.mark_edited();
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
        self.mark_edited();
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

    pub fn backspace(&mut self) {
        if self.read_only {
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
        self.checkpoint(EditKind::Delete);
        if self.cursor_col > 0 {
            let idx = self.char_idx(self.cursor_line, self.cursor_col);
            self.rope.remove(idx - 1..idx);
            self.cursor_col -= 1;
            self.mark_edited();
        } else if self.cursor_line > 0 {
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
        self.checkpoint(EditKind::Delete);
        let idx = self.char_idx(self.cursor_line, self.cursor_col);
        if idx < self.rope.len_chars() {
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

    /// Determines the foldable range starting at `line`, if any: either a brace block
    /// (`{` ... matching `}`) or, failing that, an indentation-based block (Python-style:
    /// the following more-indented lines).
    pub fn foldable_range_at(&self, line: usize) -> Option<(usize, usize)> {
        let total = self.rope.len_lines();
        if line >= total {
            return None;
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

    pub fn move_up(&mut self) {
        if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.cursor_col.min(self.line_char_len(self.cursor_line));
        }
        self.clamp_out_of_folds();
    }

    pub fn move_down(&mut self) {
        if self.cursor_line + 1 < self.rope.len_lines() {
            self.cursor_line += 1;
            self.cursor_col = self.cursor_col.min(self.line_char_len(self.cursor_line));
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
        self.cursor_col = self.cursor_col.min(self.line_char_len(self.cursor_line));
        self.clamp_out_of_folds();
    }

    pub fn page_down(&mut self, page: usize) {
        let max_line = self.rope.len_lines().saturating_sub(1);
        self.cursor_line = (self.cursor_line + page).min(max_line);
        self.cursor_col = self.cursor_col.min(self.line_char_len(self.cursor_line));
        self.clamp_out_of_folds();
    }

    // ---- Word-wise motion & deletion ------------------------------------------------

    fn cursor_char_idx(&self) -> usize {
        self.rope.line_to_char(self.cursor_line) + self.cursor_col
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
        self.mark_edited();
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
        self.set_cursor_char_idx(start + text.chars().count());
        self.mark_edited();
    }

    /// Marks the buffer changed: unsaved, needing re-highlighting, and one revision newer.
    ///
    /// One call rather than the three assignments it replaces, so an edit added later cannot
    /// quietly forget one of them — which is how a live preview would end up showing text that
    /// is no longer there.
    fn mark_edited(&mut self) {
        self.dirty = true;
        self.syntax_dirty = true;
        self.revision = self.revision.wrapping_add(1);
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

        assert_eq!(ed.rope.to_string(), text_before, "a read-only buffer must not be mutated");
        assert_eq!(ed.dirty, dirty_before);
        assert_eq!(ed.undo_stack.len(), undo_depth_before, "no checkpoint should have been pushed either");
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
}
