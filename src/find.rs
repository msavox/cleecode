/// State for the in-file find / find-and-replace overlay. Matches are stored as absolute
/// char ranges `[start, end)` into the active editor's rope, recomputed whenever the query
/// or the buffer changes. Plain case-sensitive substring search for now (regex is a later
/// batch); the current match is surfaced to the user via the editor's normal selection.
pub struct FindState {
    pub query: String,
    pub replace: String,
    /// Whether typing edits the replace field (vs the query field); toggled with Tab.
    pub focus_replace: bool,
    pub matches: Vec<(usize, usize)>,
    pub current: usize,
}

impl FindState {
    pub fn new() -> Self {
        FindState {
            query: String::new(),
            replace: String::new(),
            focus_replace: false,
            matches: Vec::new(),
            current: 0,
        }
    }

    /// Recomputes match ranges against `text`, keeping the current index in bounds. `from`
    /// biases which match becomes current: the first match at or after that char index.
    pub fn recompute(&mut self, text: &str, from: usize) {
        self.matches.clear();
        if self.query.is_empty() {
            self.current = 0;
            return;
        }
        let q_chars = self.query.chars().count();
        let mut search_start = 0usize; // byte offset
        let mut char_base = 0usize; // char count up to search_start
        while let Some(rel) = text[search_start..].find(&self.query) {
            let byte_idx = search_start + rel;
            let char_idx = char_base + text[search_start..byte_idx].chars().count();
            self.matches.push((char_idx, char_idx + q_chars));
            // Advance by one char past the match start to allow overlapping-free progress.
            let next_byte = byte_idx + self.query.len().max(1);
            char_base = char_idx + text[byte_idx..next_byte.min(text.len())].chars().count();
            search_start = next_byte.min(text.len());
            if search_start >= text.len() {
                break;
            }
        }
        self.current = self
            .matches
            .iter()
            .position(|&(s, _)| s >= from)
            .unwrap_or(0)
            .min(self.matches.len().saturating_sub(1));
    }

    pub fn next(&mut self) {
        if !self.matches.is_empty() {
            self.current = (self.current + 1) % self.matches.len();
        }
    }

    pub fn prev(&mut self) {
        if !self.matches.is_empty() {
            self.current = (self.current + self.matches.len() - 1) % self.matches.len();
        }
    }

    pub fn current_match(&self) -> Option<(usize, usize)> {
        self.matches.get(self.current).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_all_occurrences() {
        let mut f = FindState::new();
        f.query = "ab".to_string();
        f.recompute("ab xx ab yy ab", 0);
        assert_eq!(f.matches, vec![(0, 2), (6, 8), (12, 14)]);
    }

    #[test]
    fn current_biased_to_from() {
        let mut f = FindState::new();
        f.query = "x".to_string();
        f.recompute("x..x..x", 3);
        // first match at or after char 3 is the one at index 3.
        assert_eq!(f.current_match(), Some((3, 4)));
    }

    #[test]
    fn next_and_prev_wrap() {
        let mut f = FindState::new();
        f.query = "a".to_string();
        f.recompute("aaa", 0);
        assert_eq!(f.current, 0);
        f.prev();
        assert_eq!(f.current, 2);
        f.next();
        assert_eq!(f.current, 0);
    }

    #[test]
    fn multibyte_char_indices() {
        let mut f = FindState::new();
        f.query = "é".to_string();
        // "café ... é": matches by char index, not byte index.
        f.recompute("café é", 0);
        assert_eq!(f.matches, vec![(3, 4), (5, 6)]);
    }
}
