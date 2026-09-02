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

/// What one match becomes once its capture groups are resolved.
///
/// `at` is where the match starts *in `text`*, in bytes, and the pattern is run there again
/// rather than against the matched characters on their own. That distinction is the difference
/// between working and quietly doing nothing: `foo(?=bar)` matches `foo` in `foobar`, and the
/// same pattern asked about `foo` alone matches nothing — so a replacement worked out from the
/// excised match comes back as the template written out literally. Anchors are the same story:
/// `^` holds at the start of the text it was found in and nowhere in the three characters next
/// to it. Lookaround is the reason this project uses fancy-regex at all, so it has to survive
/// the replacement too.
///
/// The one place a `$1` becomes a group, shared by the Find box and by the sweep across the
/// project for the same reason [`compile`] is shared: what `$1` means cannot depend on which of
/// the two boxes it was typed into.
///
/// A literal search is *not* this function's business — it has no groups to resolve, so there a
/// `$` is just a dollar and the caller hands the template back untouched.
///
/// The template written out as it stands is the answer when the pattern gave up, or when the
/// text has moved on since it was scanned. It is a poor answer, and it is a better one than
/// nothing.
pub fn expand_at(re: &Regex, text: &str, at: usize, template: &str) -> String {
    match re.captures_from_pos(text, at) {
        Ok(Some(caps)) => {
            let mut out = String::new();
            caps.expand(template, &mut out);
            out
        }
        _ => template.to_string(),
    }
}

/// Shortens `text` to `budget` characters, marking that it was cut. Newlines and tabs become
/// spaces first: a match can span a line break, and a preview that broke its own row in two
/// would push the rest of the box down as you typed.
fn ellipsise(text: &str, budget: usize) -> String {
    let flat: String = text.chars().map(|c| if c.is_whitespace() { ' ' } else { c }).collect();
    if flat.chars().count() <= budget {
        return flat;
    }
    flat.chars().take(budget.saturating_sub(1)).collect::<String>() + "…"
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
    /// The text the matches were found in, and where each of them sits in it in bytes — the
    /// ranges above are in chars, because that is what the rope counts in, and the engine counts
    /// in bytes.
    ///
    /// Kept because a replacement's capture groups can only be resolved *where the match was*.
    /// `foo(?=bar)` matches `foo` in `foobar`, and asking the same pattern about `foo` on its own
    /// finds nothing at all — so a replacement worked out from the excised match comes back
    /// unchanged, and Replace visibly does nothing. Anchors are the same story: `(?m)^` holds at
    /// a line start in the buffer and nowhere in the three characters it matched next to.
    ///
    /// Only a pattern search keeps them: a literal query has no groups to resolve, so it has no
    /// reason to hold a second copy of the buffer.
    haystack: Option<String>,
    byte_matches: Vec<(usize, usize)>,
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
            haystack: None,
            byte_matches: Vec::new(),
        }
    }

    /// Recomputes match ranges against `text`, keeping the current index in bounds. `from`
    /// biases which match becomes current: the first match at or after that char index.
    pub fn recompute(&mut self, text: &str, from: usize) {
        self.matches.clear();
        self.byte_matches.clear();
        self.haystack = None;
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
            self.byte_matches.push((m.start(), m.end()));

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
        if self.regex && !self.matches.is_empty() {
            self.haystack = Some(text.to_string());
        }
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
    ///
    /// The groups are resolved by [`expand_at`], which runs the pattern again *where the match
    /// was found*, in the text it was found in, rather than against the matched text on its own —
    /// see there for why that is the difference between working and quietly doing nothing.
    ///
    /// What is left here is the part only this box knows: which match the caller means. Nothing
    /// above `haystack` is a pattern search, so a literal query never reaches the expansion at
    /// all and its dollars stay dollars.
    pub fn replacement_for(&self, matched: &str) -> String {
        let (Some(re), Some(text)) = (&self.compiled, &self.haystack) else {
            return self.replace.clone();
        };
        // Which match this is: the caller has the text a match covered, not where it was, and
        // where it was is the one thing the groups need. Two matches covering the same text
        // resolve the same way, so the first of them is the right answer for either.
        let Some(&(at, _)) =
            self.byte_matches.iter().find(|&&(s, e)| text.get(s..e) == Some(matched))
        else {
            return self.replace.clone();
        };
        expand_at(re, text, at, &self.replace)
    }

    /// One line showing what the current match turns into, so "replace all" can be read before
    /// it is run rather than judged after it.
    ///
    /// It shows the *current* match, not a made-up example, and it matters most with a pattern:
    /// `$1` and `${name}` mean nothing until they are resolved against a particular match, so
    /// until they are resolved there is no way to tell a replacement that works from one that
    /// quietly writes the dollars out literally.
    ///
    /// `None` when there is nothing to preview: no match, or no replacement typed. An empty
    /// replacement is a deletion and deserves saying so, but only once the user has shown they
    /// mean to replace at all.
    pub fn preview(&self, matched: &str, budget: usize) -> Option<String> {
        if self.replace.is_empty() || self.matches.is_empty() {
            return None;
        }
        let becomes = self.replacement_for(matched);
        // A match can be a whole line; a preview is one line for two of them.
        let each = (budget.saturating_sub(3) / 2).max(4);
        Some(format!("{} → {}", ellipsise(matched, each), ellipsise(&becomes, each)))
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

    /// The whole point of the preview: with a pattern, `$1` is indistinguishable from a literal
    /// dollar until it is resolved against a real match — and by then the file has been changed.
    #[test]
    fn the_preview_resolves_the_groups_before_anything_is_replaced() {
        let mut f = FindState::new();
        f.regex = true;
        f.case_sensitive = true;
        f.query = r"(\w+)@(\w+)".to_string();
        f.replace = "$2.$1".to_string();
        f.recompute("ada@lovelace and alan@turing", 0);
        assert_eq!(f.preview("ada@lovelace", 60).unwrap(), "ada@lovelace → lovelace.ada");

        // A literal search has no groups, so the preview shows the dollar staying a dollar —
        // which is exactly the mistake it is there to catch.
        let mut plain = literal("cost");
        plain.replace = "$1".to_string();
        plain.recompute("cost", 0);
        assert_eq!(plain.preview("cost", 60).unwrap(), "cost → $1");
    }

    #[test]
    fn there_is_nothing_to_preview_without_a_match_or_a_replacement() {
        let mut f = literal("ab");
        f.recompute("ab ab", 0);
        assert_eq!(f.preview("ab", 60), None, "no replacement typed yet");

        f.replace = "cd".to_string();
        assert_eq!(f.preview("ab", 60).as_deref(), Some("ab → cd"));

        // An empty replacement is a deletion, and once a replacement field is in play the
        // preview says so rather than going quiet.
        f.replace = " ".to_string();
        assert_eq!(f.preview("ab", 60).as_deref(), Some("ab →  "));

        let mut nothing = literal("zz");
        nothing.replace = "y".to_string();
        nothing.recompute("ab ab", 0);
        assert_eq!(nothing.preview("zz", 60), None, "nothing matched, nothing to change");
    }

    /// A match can be a whole line, or span a line break. The preview is one row of a box whose
    /// height is fixed on purpose, so it has to stay one row whatever it is given.
    #[test]
    fn a_long_or_multi_line_match_still_previews_on_one_row() {
        let mut f = literal("x");
        f.replace = "y".to_string();
        f.recompute("x", 0);
        let long = "a".repeat(200);
        let preview = f.preview(&long, 40).unwrap();
        assert!(preview.chars().count() <= 40, "{} chars is too wide", preview.chars().count());
        assert!(preview.contains('…'), "a cut has to look like one: {preview}");

        let across = f.preview("one\ntwo\tthree", 60).unwrap();
        assert!(!across.contains('\n') && !across.contains('\t'), "{across} would break the row");
        assert!(across.starts_with("one two three"));
    }

    /// Lookaround is the reason this project uses fancy-regex rather than `regex`, and a
    /// replacement worked out from the matched text on its own throws it away: `foo(?=bar)`
    /// matched `foo`, and the same pattern asked about `foo` alone matches nothing — so Replace
    /// used to hand back the text it was given and look like a key that does nothing.
    #[test]
    fn a_replacement_keeps_the_context_the_match_was_found_in() {
        let mut f = FindState::new();
        f.regex = true;
        f.case_sensitive = true;
        f.query = "foo(?=bar)".to_string();
        f.replace = "X".to_string();
        f.recompute("foobar", 0);
        assert_eq!(f.matches, vec![(0, 3)]);
        assert_eq!(f.replacement_for("foo"), "X");
        assert_eq!(f.preview("foo", 60).unwrap(), "foo → X");

        // An anchor holds in the buffer and not in the three characters next to it. Each line
        // start is a different match, and each one keeps its own group.
        let mut f = FindState::new();
        f.regex = true;
        f.case_sensitive = true;
        f.query = r"(?m)^(\w+)".to_string();
        f.replace = "[$1]".to_string();
        f.recompute("uno\ndue\ntre", 0);
        assert_eq!(f.matches, vec![(0, 3), (4, 7), (8, 11)]);
        assert_eq!(f.replacement_for("due"), "[due]", "the second line, matched mid-buffer");
        assert_eq!(f.replacement_for("tre"), "[tre]");
    }

    /// The engine counts in bytes and the rope counts in chars, and an accent is where the two
    /// come apart. Both the ranges handed back and the text the groups are cut out of have to
    /// land on the same characters.
    #[test]
    fn groups_land_on_the_right_characters_in_an_accented_buffer() {
        let mut f = FindState::new();
        f.regex = true;
        f.case_sensitive = true;
        f.query = r"(città) (\w+)".to_string();
        f.replace = "$2 $1".to_string();
        let text = "una città bellissima e una città grande";
        f.recompute(text, 0);
        assert_eq!(f.matches, vec![(4, 20), (27, 39)]);
        // The char ranges say what the replacement is worked out from, so they have to agree.
        let first: String = text.chars().take(20).skip(4).collect();
        assert_eq!(first, "città bellissima");
        assert_eq!(f.replacement_for(&first), "bellissima città");
        let second: String = text.chars().take(39).skip(27).collect();
        assert_eq!(f.replacement_for(&second), "grande città");
    }

    /// The expansion on its own, because it now has a second caller: the sweep across the
    /// project resolves its groups against one *line* rather than against a whole buffer, and
    /// the two have to agree about what `$1` means or the same query would replace two different
    /// things depending on which box it was typed into.
    #[test]
    fn a_group_expands_where_the_match_was_and_nowhere_else() {
        let re = compile(r"(\w+)@(\w+)", true, true).unwrap();
        let text = "ada@lovelace and alan@turing";
        assert_eq!(expand_at(&re, text, 0, "$2.$1"), "lovelace.ada");
        assert_eq!(expand_at(&re, text, 17, "$2.$1"), "turing.alan");

        // Lookaround only holds in the text the match was found in, which is the whole reason
        // the position is passed rather than the matched characters.
        let re = compile("foo(?=bar)", true, true).unwrap();
        assert_eq!(expand_at(&re, "foobar", 0, "X"), "X");
        assert_eq!(expand_at(&re, "foo", 0, "X"), "X", "no match: the template as it stands");

        // An anchor is asked about the text it is given, so a line is a line and a buffer is a
        // buffer. Both are legitimate questions, and each caller asks the one it means.
        let re = compile(r"^(\w+)", true, true).unwrap();
        assert_eq!(expand_at(&re, "due", 0, "[$1]"), "[due]");
        assert_eq!(expand_at(&re, "uno\ndue", 4, "[$1]"), "[$1]", "^ does not hold mid-buffer");
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
