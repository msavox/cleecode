//! State for the in-file find / find-and-replace overlay.
//!
//! Matches are stored as absolute char ranges `[start, end)` into the active editor's rope,
//! recomputed whenever the query, the flags or the buffer change; the current match is surfaced
//! to the user via the editor's normal selection.
//!
//! Both kinds of search go through the same regex engine. A literal query is escaped rather than
//! scanned by hand, which is what lets case-insensitivity be the engine's problem: lowercasing
//! the text ourselves would have moved the char indices under some scripts, and the whole point
//! of these ranges is that they line up with the rope.

use fancy_regex::{Regex, RegexBuilder};

/// Turns what was typed into a compiled query, or into one line saying why it isn't one.
///
/// The single place a query becomes a pattern, so the box in a file and the search across the
/// project cannot drift into two dialects: what `(\w+)` means has to be the same question
/// wherever it is asked.
///
/// A literal query is escaped rather than searched for by hand, and case-insensitivity is an
/// inline flag rather than a builder switch, so both kinds of search reach the engine by the
/// same route.
pub fn compile(query: &str, regex: bool, case_sensitive: bool) -> Result<Regex, String> {
    let pattern = if regex { query.to_string() } else { fancy_regex::escape(query).into_owned() };
    let pattern = if case_sensitive { pattern } else { format!("(?i){pattern}") };
    RegexBuilder::new(&pattern).build().map_err(|e| first_line(&e.to_string(), "invalid pattern"))
}

/// Engine errors run to several lines with a diagram under the offending character; an overlay
/// has one line. The first is the sentence that says what is wrong.
pub fn first_line(message: &str, fallback: &str) -> String {
    message.lines().next().unwrap_or(fallback).to_string()
}

pub struct FindState {
    pub query: String,
    pub replace: String,
    /// Whether typing edits the replace field (vs the query field); toggled with Tab.
    pub focus_replace: bool,
    pub matches: Vec<(usize, usize)>,
    pub current: usize,
    /// Whether the query is a pattern rather than the text to look for.
    pub regex: bool,
    /// Whether case matters. Off to begin with: a search typed in a hurry is nearly always meant
    /// loosely, and the strict reading is one key away and shown on screen.
    pub case_sensitive: bool,
    /// Why the query found nothing it could use — an unbalanced bracket, or a pattern that gave
    /// up. Shown in the overlay, because a pattern being wrong looks exactly like a pattern
    /// matching nothing, and the two want different fixes.
    pub error: Option<String>,
    /// The compiled query, kept so a replacement can refer to its capture groups.
    compiled: Option<Regex>,
}

impl FindState {
    pub fn new() -> Self {
        FindState {
            query: String::new(),
            replace: String::new(),
            focus_replace: false,
            matches: Vec::new(),
            current: 0,
            regex: false,
            case_sensitive: false,
            error: None,
            compiled: None,
        }
    }

    /// Recomputes match ranges against `text`, keeping the current index in bounds. `from`
    /// biases which match becomes current: the first match at or after that char index.
    pub fn recompute(&mut self, text: &str, from: usize) {
        self.matches.clear();
        self.current = 0;
        self.error = None;
        self.compiled = None;
        if self.query.is_empty() {
            return;
        }

        let compiled = match compile(&self.query, self.regex, self.case_sensitive) {
            Ok(re) => re,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };

        // Walked by byte offset, recorded by char index: the rope counts in chars, the engine
        // in bytes, and `chars_at` is the running bridge between the two so neither is
        // recounted from the start of the file for every match.
        let mut byte = 0usize;
        let mut chars_at = 0usize;
        loop {
            let found = match compiled.find_from_pos(text, byte) {
                Ok(found) => found,
                Err(e) => {
                    // A pattern that exceeded the backtrack limit. Whatever was found before it
                    // gave up is still true, so it is kept and the reason is shown alongside.
                    self.error = Some(first_line(&e.to_string(), "pattern gave up"));
                    break;
                }
            };
            let Some(m) = found else { break };

            let start_char = chars_at + text[byte..m.start()].chars().count();
            let end_char = start_char + text[m.start()..m.end()].chars().count();
            self.matches.push((start_char, end_char));

            // An empty match — `a*` against "bbb", or `^` — matches at every position without
            // consuming anything, so the walk has to be moved on by hand or it never ends.
            let next = if m.end() > m.start() {
                m.end()
            } else {
                match text[m.start()..].chars().next() {
                    Some(c) => m.start() + c.len_utf8(),
                    None => break,
                }
            };
            chars_at = start_char + text[m.start()..next].chars().count();
            byte = next;
        }

        self.compiled = Some(compiled);
        self.current = self
            .matches
            .iter()
            .position(|&(s, _)| s >= from)
            .unwrap_or(0)
            .min(self.matches.len().saturating_sub(1));
    }

    /// What `matched` becomes when replaced. A pattern's replacement may refer to its capture
    /// groups (`$1`, `${name}`); a literal search has no groups to refer to, so there a `$` is
    /// just a dollar and is left alone.
    pub fn replacement_for(&self, matched: &str) -> String {
        match (self.regex, &self.compiled) {
            (true, Some(re)) => re.replace(matched, self.replace.as_str()).into_owned(),
            _ => self.replace.clone(),
        }
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

    /// Literal search is the common case and must stay literal: a query full of punctuation is
    /// text to look for, not a pattern that happens to be full of syntax errors.
    fn literal(query: &str) -> FindState {
        let mut f = FindState::new();
        f.query = query.to_string();
        f.case_sensitive = true;
        f
    }

    #[test]
    fn finds_all_occurrences() {
        let mut f = literal("ab");
        f.recompute("ab xx ab yy ab", 0);
        assert_eq!(f.matches, vec![(0, 2), (6, 8), (12, 14)]);
    }

    #[test]
    fn current_biased_to_from() {
        let mut f = literal("x");
        f.recompute("x..x..x", 3);
        // first match at or after char 3 is the one at index 3.
        assert_eq!(f.current_match(), Some((3, 4)));
    }

    #[test]
    fn next_and_prev_wrap() {
        let mut f = literal("a");
        f.recompute("aaa", 0);
        assert_eq!(f.current, 0);
        f.prev();
        assert_eq!(f.current, 2);
        f.next();
        assert_eq!(f.current, 0);
    }

    #[test]
    fn multibyte_char_indices() {
        let mut f = literal("é");
        // "café ... é": matches by char index, not byte index.
        f.recompute("café é", 0);
        assert_eq!(f.matches, vec![(3, 4), (5, 6)]);
    }

    /// Everything goes through the regex engine now, so a literal query still has to be treated
    /// as text — otherwise searching for `a.b` or `(` would silently mean something else, or
    /// nothing at all.
    #[test]
    fn a_literal_query_is_not_a_pattern() {
        let mut f = literal("a.b");
        f.recompute("a.b axb", 0);
        assert_eq!(f.matches, vec![(0, 3)], "the dot is a dot");

        let mut f = literal("c++");
        f.recompute("c++ and c+++", 0);
        assert_eq!(f.matches, vec![(0, 3), (8, 11)]);
        assert!(f.error.is_none(), "punctuation is not a syntax error in a literal search");
    }

    #[test]
    fn case_insensitive_by_default_and_strict_on_request() {
        let mut f = FindState::new();
        f.query = "CAFÉ".to_string();
        f.recompute("café CAFÉ Café", 0);
        assert_eq!(f.matches.len(), 3);

        f.case_sensitive = true;
        f.recompute("café CAFÉ Café", 0);
        assert_eq!(f.matches, vec![(5, 9)], "only the one written that way");
    }

    #[test]
    fn a_pattern_matches_and_its_groups_reach_the_replacement() {
        let mut f = FindState::new();
        f.regex = true;
        f.case_sensitive = true;
        f.query = r"(\w+)@(\w+)".to_string();
        f.replace = "$2.$1".to_string();
        f.recompute("ada@lovelace and alan@turing", 0);
        assert_eq!(f.matches, vec![(0, 12), (17, 28)]);
        assert_eq!(f.replacement_for("ada@lovelace"), "lovelace.ada");

        // A literal search has no groups, so a dollar in the replacement stays a dollar.
        let mut plain = literal("cost");
        plain.replace = "$1".to_string();
        plain.recompute("cost", 0);
        assert_eq!(plain.replacement_for("cost"), "$1");
    }

    /// A pattern is typed one character at a time, so it spends most of its life invalid. That
    /// has to read as "not a pattern yet", not as "no matches here".
    #[test]
    fn a_broken_pattern_says_so_instead_of_finding_nothing() {
        let mut f = FindState::new();
        f.regex = true;
        f.query = "(unclosed".to_string();
        f.recompute("(unclosed", 0);
        assert!(f.matches.is_empty());
        assert!(f.error.is_some(), "the overlay has something to show");

        // And it recovers: the error does not outlive the query that caused it.
        f.query = "(closed)".to_string();
        f.recompute("(closed)", 0);
        assert_eq!(f.matches.len(), 1);
        assert!(f.error.is_none());
    }

    /// A pattern that matches nothing at all still advances, or the walk pins itself in place
    /// and the editor stops answering — the worst outcome available to a search box.
    #[test]
    fn empty_matches_do_not_trap_the_walk() {
        let mut f = FindState::new();
        f.regex = true;
        f.query = "x*".to_string();
        f.recompute("abc", 0);
        // One empty match at each position, the end included, and then it stops.
        assert_eq!(f.matches, vec![(0, 0), (1, 1), (2, 2), (3, 3)]);

        let mut f = FindState::new();
        f.regex = true;
        f.query = "^".to_string();
        f.recompute("uno\ndue\ntre", 0);
        assert_eq!(f.matches.len(), 1, "without (?m) there is one start of text");
    }
}
