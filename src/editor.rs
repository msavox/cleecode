use crate::i18n::{self, Key, Lang};
use anyhow::Result;
use ratatui::style::Style;
use ropey::Rope;
use std::path::PathBuf;
use std::time::SystemTime;

pub struct Editor {
    pub rope: Rope,
    pub path: Option<PathBuf>,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub top_line: usize,
    pub left_col: usize,
    pub dirty: bool,
    pub disk_mtime: Option<SystemTime>,
    pub highlighted: Vec<Vec<(Style, String)>>,
    pub syntax_dirty: bool,
    pub selection_anchor: Option<(usize, usize)>,
    /// Active (collapsed) fold regions as (start_line, end_line), inclusive, sorted by start.
    pub folds: Vec<(usize, usize)>,
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
            dirty: false,
            disk_mtime: None,
            highlighted: Vec::new(),
            syntax_dirty: true,
            selection_anchor: None,
            folds: Vec::new(),
        }
    }

    pub fn open(path: PathBuf) -> Result<Self> {
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let disk_mtime = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());
        Ok(Editor {
            rope: Rope::from_str(&content),
            path: Some(path),
            cursor_line: 0,
            cursor_col: 0,
            top_line: 0,
            left_col: 0,
            dirty: false,
            disk_mtime,
            highlighted: Vec::new(),
            syntax_dirty: true,
            selection_anchor: None,
            folds: Vec::new(),
        })
    }

    pub fn save(&mut self) -> Result<()> {
        if let Some(path) = &self.path {
            std::fs::write(path, self.rope.to_string())?;
            self.dirty = false;
            self.disk_mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
        }
        Ok(())
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
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        self.rope = Rope::from_str(&content);
        self.disk_mtime = Some(mtime);
        self.syntax_dirty = true;
        self.folds.clear();
        let max_line = self.rope.len_lines().saturating_sub(1);
        self.cursor_line = self.cursor_line.min(max_line);
        self.cursor_col = self.cursor_col.min(self.line_char_len(self.cursor_line));
        Some(i18n::msg_externally_reloaded(lang, &self.title(lang)))
    }

    pub fn title(&self, lang: Lang) -> String {
        match &self.path {
            Some(p) => p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            None => i18n::t(lang, Key::UntitledFile).to_string(),
        }
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

    pub fn selected_text(&self) -> Option<String> {
        let ((sl, sc), (el, ec)) = self.selection_range()?;
        let start_idx = self.rope.line_to_char(sl) + sc;
        let end_idx = self.rope.line_to_char(el) + ec;
        Some(self.rope.slice(start_idx..end_idx).to_string())
    }

    pub fn clear_selection(&mut self) {
        self.selection_anchor = None;
    }

    pub fn start_or_extend_selection(&mut self) {
        if self.selection_anchor.is_none() {
            self.selection_anchor = Some((self.cursor_line, self.cursor_col));
        }
    }

    /// Deletes the active selection, if any, moving the cursor to its start. Returns
    /// whether a selection was actually deleted (callers use this to short-circuit).
    pub fn delete_selection(&mut self) -> bool {
        let Some(((sl, sc), (el, ec))) = self.selection_range() else { return false };
        let start_idx = self.rope.line_to_char(sl) + sc;
        let end_idx = self.rope.line_to_char(el) + ec;
        self.rope.remove(start_idx..end_idx);
        self.cursor_line = sl;
        self.cursor_col = sc;
        self.selection_anchor = None;
        self.dirty = true;
        self.syntax_dirty = true;
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
        let (sl, end_line) = self.indent_range();
        let pad = " ".repeat(tab_size);
        for line in sl..=end_line {
            let idx = self.rope.line_to_char(line);
            self.rope.insert(idx, &pad);
        }
        self.cursor_col += tab_size;
        if let Some((al, ac)) = self.selection_anchor.as_mut() {
            if (sl..=end_line).contains(&*al) {
                *ac += tab_size;
            }
        }
        self.dirty = true;
        self.syntax_dirty = true;
    }

    pub fn outdent_selection(&mut self, tab_size: usize) {
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
        self.dirty = true;
        self.syntax_dirty = true;
    }

    pub fn insert_char(&mut self, ch: char) {
        self.delete_selection();
        let idx = self.char_idx(self.cursor_line, self.cursor_col);
        self.rope.insert_char(idx, ch);
        self.cursor_col += 1;
        self.dirty = true;
        self.syntax_dirty = true;
    }

    /// Inserts a run of text with no newlines (used for space-expanded tabs).
    pub fn insert_str(&mut self, s: &str) {
        self.delete_selection();
        let idx = self.char_idx(self.cursor_line, self.cursor_col);
        self.rope.insert(idx, s);
        self.cursor_col += s.chars().count();
        self.dirty = true;
        self.syntax_dirty = true;
    }

    /// Inserts possibly multi-line text (e.g. a clipboard paste), splitting on '\n'.
    pub fn insert_multiline(&mut self, text: &str) {
        self.delete_selection();
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                self.insert_newline(false);
            }
            if !line.is_empty() {
                self.insert_str(line);
            }
        }
    }

    pub fn insert_newline(&mut self, auto_indent: bool) {
        self.delete_selection();
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
        self.dirty = true;
        self.syntax_dirty = true;
    }

    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor_col > 0 {
            let idx = self.char_idx(self.cursor_line, self.cursor_col);
            self.rope.remove(idx - 1..idx);
            self.cursor_col -= 1;
            self.dirty = true;
            self.syntax_dirty = true;
        } else if self.cursor_line > 0 {
            let prev_len = self.line_char_len(self.cursor_line - 1);
            let idx = self.char_idx(self.cursor_line, 0);
            self.rope.remove(idx - 1..idx);
            self.cursor_line -= 1;
            self.cursor_col = prev_len;
            self.dirty = true;
            self.syntax_dirty = true;
        }
    }

    pub fn delete_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        let idx = self.char_idx(self.cursor_line, self.cursor_col);
        if idx < self.rope.len_chars() {
            self.rope.remove(idx..idx + 1);
            self.dirty = true;
            self.syntax_dirty = true;
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
}
