//! Word completion: the popup that hangs under the cursor while you type.
//!
//! The candidates come from the words already in the open buffers, plus the keywords of the
//! language the file is written in. That is a deliberate floor rather than a stopgap: it needs
//! no server to install and no network, it works in a config file and in a language nobody
//! wrote a parser for, and it is what remains when anything smarter is unavailable.
//!
//! A language server is a *second source* into the same popup, not a second popup — which is why
//! [`Source`] exists before there is anything but the buffer to put in it.

use crate::picker::fuzzy_score;
use crossterm::event::{KeyCode, KeyModifiers};
use ropey::Rope;
use std::collections::HashMap;
use std::path::Path;

/// How many word characters must be typed before the popup appears on its own, and below which
/// it closes again. Two is enough to mean something and short enough not to arrive late.
pub const MIN_PREFIX: usize = 2;

/// Rows the popup shows at once; the list scrolls inside this if there are more.
pub const MAX_ROWS: usize = 8;

/// Shorter than this and a word is not worth offering: the popup takes more keystrokes to
/// navigate than the word takes to type.
const MIN_WORD: usize = 3;

/// Lines read either side of the cursor in the buffer being typed in, and from the top in the
/// others. This is not a corner cut for speed: a word four thousand lines away already ranks
/// below everything nearer it, so the window applies the same judgement the ranking makes —
/// only before the work rather than after it, which is what keeps a huge file from stuttering
/// every time the popup opens.
const SCAN_WINDOW: usize = 4000;

/// The distance recorded for a word found in a buffer other than the one being typed in. Larger
/// than any real line number we will meet, so those words sort after every local one without
/// needing a separate tier.
const OTHER_BUFFER: usize = usize::MAX / 2;

/// Where a candidate came from. The variants are ranked, not just labelled: see [`rank`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// A word that is already written somewhere in an open buffer.
    Buffer,
    /// A keyword of the file's language. Always offered last — they are short, and typing one
    /// is quicker than walking a list to it.
    Keyword,
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub text: String,
    pub source: Source,
    /// Lines between this word and the cursor, at the moment the index was built.
    pub distance: usize,
    /// How many times the word occurs across the buffers scanned.
    pub freq: u32,
}

/// The words known when the popup opened.
///
/// Built once and then only filtered. An index kept up to date incrementally would be worse than
/// a stale one: it would offer words that have since been deleted, and there is no keystroke at
/// which that is easier to explain.
#[derive(Default)]
pub struct Index {
    entries: HashMap<String, Candidate>,
}

impl Index {
    pub fn new() -> Self {
        Index::default()
    }

    /// Adds every word of a buffer. `near_line` is the cursor's line when this is the buffer
    /// being typed in, and `None` for the others.
    pub fn add_buffer(&mut self, rope: &Rope, near_line: Option<usize>) {
        let lines = rope.len_lines();
        let (from, to) = match near_line {
            Some(cursor) => (cursor.saturating_sub(SCAN_WINDOW), cursor.saturating_add(SCAN_WINDOW).min(lines)),
            None => (0, lines.min(SCAN_WINDOW)),
        };
        for line_idx in from..to {
            // A rope line is a slice, not a String, so this is the one allocation per line that
            // scanning cannot avoid — the word scanner needs `&str`.
            let text = rope.line(line_idx).to_string();
            let distance = match near_line {
                Some(cursor) => line_idx.abs_diff(cursor),
                None => OTHER_BUFFER,
            };
            for word in words(&text) {
                self.add(word, Source::Buffer, distance);
            }
        }
    }

    /// Adds the keywords of the language `path` is written in. Nothing at all for a file whose
    /// extension we do not know, which is correct: guessing keywords would put words in the list
    /// that cannot appear in the file.
    pub fn add_keywords(&mut self, path: Option<&Path>) {
        for kw in keywords(path) {
            self.add(kw, Source::Keyword, OTHER_BUFFER);
        }
    }

    fn add(&mut self, text: &str, source: Source, distance: usize) {
        match self.entries.get_mut(text) {
            Some(existing) => {
                existing.freq = existing.freq.saturating_add(1);
                existing.distance = existing.distance.min(distance);
                // A word that is also a keyword stays a keyword: the ranking rule about keywords
                // is about how quickly they are typed, and that does not change because the file
                // happens to contain one.
                if source == Source::Keyword {
                    existing.source = Source::Keyword;
                }
            }
            None => {
                self.entries.insert(
                    text.to_string(),
                    Candidate { text: text.to_string(), source, distance, freq: 1 },
                );
            }
        }
    }

    /// The candidates, in an order that does not depend on how a hash map happened to iterate —
    /// so two runs on the same buffers rank identically.
    pub fn into_candidates(self) -> Vec<Candidate> {
        let mut out: Vec<Candidate> = self.entries.into_values().collect();
        out.sort_by(|a, b| a.text.cmp(&b.text));
        out
    }
}

/// The identifier-shaped words of a line: letters, digits and underscores, starting with a
/// letter or an underscore so `2024` and `0x1f` are not offered as words.
pub fn words(line: &str) -> impl Iterator<Item = &str> {
    line.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| w.chars().count() >= MIN_WORD)
        .filter(|w| w.starts_with(|c: char| c.is_alphabetic() || c == '_'))
}

/// The word being typed immediately before `(line, col)`, and the absolute char index it starts
/// at. `None` when the cursor is not sitting at the end of a word.
pub fn prefix_at(rope: &Rope, line: usize, col: usize) -> Option<(usize, String)> {
    if line >= rope.len_lines() {
        return None;
    }
    let chars: Vec<char> = rope.line(line).chars().collect();
    let col = col.min(chars.len());
    // Typing in the middle of an existing word is editing it, not starting one; offering to
    // replace what is already to the right would be a surprise.
    if chars.get(col).is_some_and(|&c| is_word_char(c)) {
        return None;
    }
    let mut start = col;
    while start > 0 && is_word_char(chars[start - 1]) {
        start -= 1;
    }
    let prefix: String = chars[start..col].iter().collect();
    if prefix.is_empty() || !prefix.starts_with(|c: char| c.is_alphabetic() || c == '_') {
        return None;
    }
    Some((rope.line_to_char(line) + start, prefix))
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Indices into `candidates`, best first, for the word `prefix`.
///
/// The order is exact prefix, then prefix ignoring case, then fuzzy, then keywords — and the
/// keywords stay last even when one of them matches the prefix exactly. Inside a tier the word
/// nearest the cursor wins, and only then the commonest one.
///
/// `fuzzy_score` is deliberately confined to the third tier. It matches subsequences, which is
/// right for a command palette and wrong for code: typing `conf` must reach `config_path` before
/// `load_config`, and a subsequence matcher has no reason to prefer either.
pub fn rank(prefix: &str, candidates: &[Candidate]) -> Vec<usize> {
    let lower = prefix.to_lowercase();
    let mut scored: Vec<(u8, i32, usize, u32, usize)> = Vec::new();
    for (i, c) in candidates.iter().enumerate() {
        // Nothing to insert, so nothing to offer.
        if c.text == prefix {
            continue;
        }
        let Some((tier, score)) = tier_of(c, prefix, &lower) else { continue };
        scored.push((tier, -score, c.distance, c.freq, i));
    }
    // Tier, then fuzzy score (already negated so smaller is better), then nearness, then how
    // common the word is — reversed, because there more is better — and finally the index, which
    // is alphabetical and only there so the order never depends on chance.
    scored.sort_by(|a, b| {
        (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)).then(b.3.cmp(&a.3)).then(a.4.cmp(&b.4))
    });
    scored.into_iter().map(|(_, _, _, _, i)| i).collect()
}

fn tier_of(c: &Candidate, prefix: &str, lower: &str) -> Option<(u8, i32)> {
    let text_lower = c.text.to_lowercase();
    if c.source == Source::Keyword {
        // Keywords match on a prefix or not at all. A fuzzy hit on a five-letter keyword is
        // noise, and it is noise in the tier that is hardest to argue with.
        return text_lower.starts_with(lower).then_some((3, 0));
    }
    if c.text.starts_with(prefix) {
        return Some((0, 0));
    }
    if text_lower.starts_with(lower) {
        return Some((1, 0));
    }
    fuzzy_score(prefix, &c.text).map(|s| (2, s))
}

/// The popup itself: what was typed, what matches it, and which row is picked.
///
/// It is not a modal. Every overlay in `app.rs` swallows the keyboard until it is dismissed;
/// this one takes five keys and lets the rest through to the editor, then re-filters against
/// whatever the edit left behind.
pub struct Popup {
    /// The buffer this belongs to, so it closes rather than follows when tabs change.
    pub editor: usize,
    /// Absolute char index where the word being completed starts.
    pub start: usize,
    pub prefix: String,
    candidates: Vec<Candidate>,
    /// Indices into `candidates`, best first.
    pub matches: Vec<usize>,
    pub selected: usize,
    /// First visible row, so a long list scrolls under a fixed window.
    pub scroll: usize,
}

impl Popup {
    /// `None` when nothing matches, which is also the answer to "should this be open at all".
    pub fn open(editor: usize, start: usize, prefix: String, candidates: Vec<Candidate>) -> Option<Self> {
        let matches = rank(&prefix, &candidates);
        if matches.is_empty() {
            return None;
        }
        Some(Popup { editor, start, prefix, candidates, matches, selected: 0, scroll: 0 })
    }

    /// Re-filters against a new prefix. `false` means the popup should close: either the word
    /// shrank below the threshold, or nothing matches it any more.
    pub fn refilter(&mut self, prefix: &str) -> bool {
        if prefix.chars().count() < MIN_PREFIX {
            return false;
        }
        self.prefix = prefix.to_string();
        self.matches = rank(prefix, &self.candidates);
        self.selected = 0;
        self.scroll = 0;
        !self.matches.is_empty()
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.matches.is_empty() {
            return;
        }
        let len = self.matches.len() as isize;
        let idx = ((self.selected as isize + delta) % len + len) % len;
        self.selected = idx as usize;
        // Keep the picked row inside the window, wrapping along with the selection.
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + MAX_ROWS {
            self.scroll = self.selected + 1 - MAX_ROWS;
        }
    }

    pub fn selected(&self) -> Option<&Candidate> {
        self.matches.get(self.selected).and_then(|&i| self.candidates.get(i))
    }

    /// The rows to draw, and whether each is the picked one.
    pub fn visible(&self) -> impl Iterator<Item = (&Candidate, bool)> {
        self.matches
            .iter()
            .enumerate()
            .skip(self.scroll)
            .take(MAX_ROWS)
            .filter_map(move |(row, &i)| self.candidates.get(i).map(|c| (c, row == self.selected)))
    }

    pub fn len(&self) -> usize {
        self.matches.len()
    }
}

/// What the popup does with a key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyAction {
    /// The popup does not want it: the editor gets the key, and the popup re-filters against
    /// whatever the edit leaves behind. This is the answer for every key but five, and it is the
    /// whole difference between this overlay and the modal ones.
    Fall,
    Close,
    Up,
    Down,
    Accept,
}

/// The five keys the popup claims, and the one rule about modifiers: it claims none of them.
///
/// A bare ↑ walks the list, but Shift+↑ still extends the selection in the buffer, because a
/// modified arrow was never about the popup. Same for Ctrl+Enter and everything else with a
/// modifier held: those chords belong to the editor and go on belonging to it while a list of
/// words happens to be on screen.
pub fn key_action(code: KeyCode, modifiers: KeyModifiers) -> KeyAction {
    if !modifiers.is_empty() {
        return KeyAction::Fall;
    }
    match code {
        KeyCode::Esc => KeyAction::Close,
        KeyCode::Up => KeyAction::Up,
        KeyCode::Down => KeyAction::Down,
        // Tab accepts while the popup is up and indents everywhere else. That is one key doing
        // the only thing it could mean in each place, which is why it does not break the rule
        // against a key with two jobs.
        KeyCode::Tab | KeyCode::Enter => KeyAction::Accept,
        _ => KeyAction::Fall,
    }
}

/// Whether the key the editor has just handled should bring the popup up.
///
/// Only writing opens it. Backspacing back into a word, or arriving on one with the arrows,
/// leaves it shut: the popup should be something you summoned by typing, not something you can
/// find yourself standing in after moving the cursor.
pub fn opens_on(code: KeyCode, ctrl: bool, prefix: Option<&str>) -> bool {
    if ctrl {
        return false;
    }
    let KeyCode::Char(c) = code else { return false };
    if !(c.is_alphanumeric() || c == '_') {
        return false;
    }
    prefix.is_some_and(|p| p.chars().count() >= MIN_PREFIX)
}

/// The keywords of the language a file is written in, by extension.
///
/// Keyed the same way as `editor::comment_token`, and for the same reason: the extension is the
/// only thing about a file that is known before it is parsed.
pub fn keywords(path: Option<&Path>) -> &'static [&'static str] {
    let Some(path) = path else { return &[] };
    let Some(ext) = path.extension().or_else(|| path.file_name()) else { return &[] };
    let Some(ext) = ext.to_str() else { return &[] };
    match ext.to_lowercase().as_str() {
        "rs" => &[
            "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
            "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod",
            "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super",
            "trait", "true", "type", "unsafe", "use", "where", "while",
        ],
        "py" | "pyi" => &[
            "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
            "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in",
            "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True",
            "False", "try", "while", "with", "yield",
        ],
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" => &[
            "as", "async", "await", "break", "case", "catch", "class", "const", "continue",
            "debugger", "default", "delete", "do", "else", "enum", "export", "extends", "false",
            "finally", "for", "from", "function", "if", "implements", "import", "in", "instanceof",
            "interface", "let", "new", "null", "of", "return", "static", "super", "switch", "this",
            "throw", "true", "try", "typeof", "var", "void", "while", "with", "yield",
        ],
        "go" => &[
            "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough",
            "for", "func", "go", "goto", "if", "import", "interface", "map", "package", "range",
            "return", "select", "struct", "switch", "type", "var",
        ],
        "c" | "h" | "cpp" | "hpp" | "cc" | "cxx" | "hh" => &[
            "auto", "bool", "break", "case", "catch", "char", "class", "const", "constexpr",
            "continue", "default", "delete", "do", "double", "else", "enum", "extern", "false",
            "float", "for", "goto", "if", "inline", "int", "long", "namespace", "new", "nullptr",
            "private", "protected", "public", "return", "short", "signed", "sizeof", "static",
            "struct", "switch", "template", "this", "throw", "true", "try", "typedef", "typename",
            "union", "unsigned", "using", "virtual", "void", "volatile", "while",
        ],
        "java" | "kt" | "kts" => &[
            "abstract", "boolean", "break", "case", "catch", "class", "const", "continue",
            "default", "do", "double", "else", "enum", "extends", "final", "finally", "float",
            "for", "if", "implements", "import", "instanceof", "int", "interface", "long", "new",
            "null", "package", "private", "protected", "public", "return", "static", "super",
            "switch", "this", "throw", "throws", "try", "void", "while",
        ],
        "rb" | "gemspec" | "rake" => &[
            "alias", "begin", "break", "case", "class", "def", "defined", "do", "else", "elsif",
            "end", "ensure", "false", "for", "if", "in", "module", "next", "nil", "not", "or",
            "redo", "rescue", "retry", "return", "self", "super", "then", "true", "undef",
            "unless", "until", "when", "while", "yield",
        ],
        "sh" | "bash" | "zsh" => &[
            "case", "declare", "do", "done", "elif", "else", "esac", "export", "fi", "for",
            "function", "if", "in", "local", "readonly", "return", "select", "then", "until",
            "while",
        ],
        "lua" => &[
            "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto",
            "if", "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until",
            "while",
        ],
        "php" => &[
            "abstract", "array", "break", "case", "catch", "class", "clone", "const", "continue",
            "declare", "default", "echo", "else", "elseif", "empty", "endfor", "endforeach",
            "endif", "endwhile", "extends", "final", "finally", "for", "foreach", "function",
            "global", "if", "implements", "include", "instanceof", "interface", "isset", "list",
            "namespace", "new", "print", "private", "protected", "public", "require", "return",
            "static", "switch", "throw", "trait", "try", "unset", "use", "var", "while", "yield",
        ],
        "swift" => &[
            "any", "as", "associatedtype", "break", "case", "catch", "class", "continue",
            "default", "defer", "deinit", "do", "else", "enum", "extension", "fallthrough",
            "false", "for", "func", "guard", "if", "import", "in", "init", "inout", "internal",
            "let", "nil", "open", "operator", "private", "protocol", "public", "repeat", "return",
            "self", "static", "struct", "subscript", "super", "switch", "throw", "throws", "true",
            "try", "typealias", "var", "where", "while",
        ],
        "sql" => &[
            "ALTER", "AND", "AS", "ASC", "BETWEEN", "BY", "CASE", "CREATE", "DELETE", "DESC",
            "DISTINCT", "DROP", "ELSE", "END", "EXISTS", "FROM", "GROUP", "HAVING", "IN", "INDEX",
            "INNER", "INSERT", "INTO", "JOIN", "LEFT", "LIKE", "LIMIT", "NOT", "NULL", "ON", "OR",
            "ORDER", "OUTER", "SELECT", "SET", "TABLE", "THEN", "UNION", "UPDATE", "VALUES",
            "WHEN", "WHERE", "WITH",
        ],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(text: &str, source: Source, distance: usize, freq: u32) -> Candidate {
        Candidate { text: text.to_string(), source, distance, freq }
    }

    fn ranked<'a>(prefix: &str, cands: &'a [Candidate]) -> Vec<&'a str> {
        rank(prefix, cands).into_iter().map(|i| cands[i].text.as_str()).collect()
    }

    #[test]
    fn words_are_identifiers_only() {
        let found: Vec<&str> = words("let config_path = load(2024, x.field);").collect();
        // `x` is too short to offer, and `2024` does not start like a word.
        assert_eq!(found, vec!["let", "config_path", "load", "field"]);
    }

    #[test]
    fn an_exact_prefix_beats_a_subsequence() {
        // The reason fuzzy_score is not the primary criterion: `load_config` contains `conf` as
        // a subsequence, but `config_path` is the word being typed.
        let cands = vec![
            cand("load_config", Source::Buffer, 0, 9),
            cand("config_path", Source::Buffer, 40, 1),
        ];
        assert_eq!(ranked("conf", &cands), vec!["config_path", "load_config"]);
    }

    #[test]
    fn case_insensitive_prefix_sits_between_exact_and_fuzzy() {
        let cands = vec![
            cand("do_render", Source::Buffer, 0, 1),
            cand("Renderer", Source::Buffer, 0, 1),
            cand("render_line", Source::Buffer, 0, 1),
        ];
        assert_eq!(ranked("render", &cands), vec!["render_line", "Renderer", "do_render"]);
    }

    #[test]
    fn keywords_sort_last_even_on_an_exact_prefix() {
        let cands =
            vec![cand("impl", Source::Keyword, 0, 1), cand("implementation", Source::Buffer, 300, 1)];
        // The keyword matches the prefix exactly and the word does not, and the keyword still
        // goes second: it is four letters, typed faster than the list is walked.
        assert_eq!(ranked("imp", &cands), vec!["implementation", "impl"]);
    }

    #[test]
    fn nearness_outranks_frequency_inside_a_tier() {
        let cands = vec![
            cand("counter_total", Source::Buffer, 200, 12),
            cand("counter_local", Source::Buffer, 3, 1),
        ];
        assert_eq!(ranked("counter", &cands), vec!["counter_local", "counter_total"]);
    }

    #[test]
    fn frequency_breaks_a_tie_at_equal_distance() {
        let cands = vec![
            cand("value_one", Source::Buffer, 5, 1),
            cand("value_two", Source::Buffer, 5, 7),
        ];
        assert_eq!(ranked("value", &cands), vec!["value_two", "value_one"]);
    }

    #[test]
    fn the_word_already_typed_is_not_offered() {
        let cands = vec![cand("config", Source::Buffer, 0, 1)];
        assert!(ranked("config", &cands).is_empty());
    }

    #[test]
    fn keywords_do_not_match_fuzzily() {
        // `sruc` reaches `struct` as a subsequence. A keyword must not arrive that way.
        let cands = vec![cand("struct", Source::Keyword, 0, 1)];
        assert!(ranked("sruc", &cands).is_empty());
    }

    #[test]
    fn a_prefix_is_the_word_before_the_cursor() {
        let rope = Rope::from_str("let conf\nsecond line\n");
        assert_eq!(prefix_at(&rope, 0, 8), Some((4, "conf".to_string())));
        // At the end of the first word, that word is the prefix.
        assert_eq!(prefix_at(&rope, 0, 3), Some((0, "let".to_string())));
        // Inside a word, there is nothing to complete: this is editing it, not writing it.
        assert_eq!(prefix_at(&rope, 0, 6), None);
        // On the space, no word to complete.
        assert_eq!(prefix_at(&rope, 0, 4), None);
    }

    #[test]
    fn a_prefix_on_a_later_line_is_an_absolute_index() {
        let rope = Rope::from_str("abc\nxyz\n");
        let (start, prefix) = prefix_at(&rope, 1, 3).unwrap();
        assert_eq!(prefix, "xyz");
        assert_eq!(start, 4);
        assert_eq!(rope.slice(start..start + 3), "xyz");
    }

    #[test]
    fn an_index_counts_and_measures() {
        let rope = Rope::from_str("alpha beta\nalpha\ngamma\n");
        let mut index = Index::new();
        index.add_buffer(&rope, Some(2));
        let cands = index.into_candidates();
        let alpha = cands.iter().find(|c| c.text == "alpha").unwrap();
        assert_eq!(alpha.freq, 2);
        // Nearest of the two occurrences: line 1, one line from the cursor on line 2.
        assert_eq!(alpha.distance, 1);
        let gamma = cands.iter().find(|c| c.text == "gamma").unwrap();
        assert_eq!(gamma.distance, 0);
    }

    #[test]
    fn words_from_another_buffer_rank_behind_local_ones() {
        let here = Rope::from_str("render_here\n");
        let there = Rope::from_str("render_there\n");
        let mut index = Index::new();
        index.add_buffer(&here, Some(0));
        index.add_buffer(&there, None);
        let cands = index.into_candidates();
        assert_eq!(ranked("render", &cands), vec!["render_here", "render_there"]);
    }

    #[test]
    fn a_popup_closes_when_the_prefix_shrinks_or_stops_matching() {
        let cands = vec![cand("config_path", Source::Buffer, 0, 1)];
        let mut popup = Popup::open(0, 0, "conf".to_string(), cands).unwrap();
        assert!(popup.refilter("confi"));
        assert!(!popup.refilter("c"), "below the threshold, the popup closes");
        assert!(!popup.refilter("zzzz"), "nothing matches, the popup closes");
    }

    #[test]
    fn a_popup_with_no_match_never_opens() {
        let cands = vec![cand("alpha", Source::Buffer, 0, 1)];
        assert!(Popup::open(0, 0, "zz".to_string(), cands).is_none());
    }

    #[test]
    fn the_selection_wraps_and_drags_the_window_with_it() {
        let cands: Vec<Candidate> =
            (0..12).map(|i| cand(&format!("word_{i:02}"), Source::Buffer, i, 1)).collect();
        let mut popup = Popup::open(0, 0, "word".to_string(), cands).unwrap();
        assert_eq!(popup.len(), 12);
        popup.move_selection(-1);
        assert_eq!(popup.selected, 11, "up from the first row wraps to the last");
        assert_eq!(popup.scroll, 12 - MAX_ROWS, "and the window follows it down");
        popup.move_selection(1);
        assert_eq!(popup.selected, 0);
        assert_eq!(popup.scroll, 0);
    }

    #[test]
    fn scanning_stays_inside_a_window_around_the_cursor() {
        // One word per line, far more lines than the window.
        let text: String = (0..SCAN_WINDOW + 200).map(|i| format!("word_{i}\n")).collect();
        let rope = Rope::from_str(&text);
        let mut index = Index::new();
        index.add_buffer(&rope, Some(SCAN_WINDOW + 100));
        let cands = index.into_candidates();
        assert!(cands.iter().any(|c| c.text == format!("word_{}", SCAN_WINDOW + 100)));
        assert!(
            !cands.iter().any(|c| c.text == "word_0"),
            "the far end of a long file is out of the window"
        );
    }

    /// The claim the whole design rests on: this overlay is not modal. Five keys, and everything
    /// else reaches the editor — so typing never has to stop to dismiss a list of words.
    #[test]
    fn the_popup_claims_five_keys_and_lets_the_rest_through() {
        let none = KeyModifiers::NONE;
        assert_eq!(key_action(KeyCode::Up, none), KeyAction::Up);
        assert_eq!(key_action(KeyCode::Down, none), KeyAction::Down);
        assert_eq!(key_action(KeyCode::Tab, none), KeyAction::Accept);
        assert_eq!(key_action(KeyCode::Enter, none), KeyAction::Accept);
        assert_eq!(key_action(KeyCode::Esc, none), KeyAction::Close);
        for code in [
            KeyCode::Char('a'),
            KeyCode::Char('_'),
            KeyCode::Backspace,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Home,
            KeyCode::PageDown,
            KeyCode::Delete,
            KeyCode::BackTab,
        ] {
            assert_eq!(key_action(code, none), KeyAction::Fall, "{code:?} belongs to the editor");
        }
    }

    #[test]
    fn a_modifier_hands_even_the_claimed_keys_back() {
        // Shift+↑ extends the selection in the buffer; it does not walk the popup.
        assert_eq!(key_action(KeyCode::Up, KeyModifiers::SHIFT), KeyAction::Fall);
        assert_eq!(key_action(KeyCode::Enter, KeyModifiers::CONTROL), KeyAction::Fall);
        assert_eq!(key_action(KeyCode::Tab, KeyModifiers::ALT), KeyAction::Fall);
    }

    #[test]
    fn typing_opens_the_popup_and_nothing_else_does() {
        let word = Some("conf");
        assert!(opens_on(KeyCode::Char('f'), false, word));
        assert!(opens_on(KeyCode::Char('_'), false, word));
        // Deleting back into a word does not open it: the cursor is standing on a word, not
        // writing one.
        assert!(!opens_on(KeyCode::Backspace, false, word));
        assert!(!opens_on(KeyCode::Right, false, word));
        // A Ctrl chord that happens to leave a letter behind is still a chord.
        assert!(!opens_on(KeyCode::Char('v'), true, word));
        // Below the threshold, and on no word at all.
        assert!(!opens_on(KeyCode::Char('c'), false, Some("c")));
        assert!(!opens_on(KeyCode::Char(' '), false, None));
    }

    #[test]
    fn only_a_known_extension_brings_keywords() {
        assert!(keywords(Some(Path::new("main.rs"))).contains(&"impl"));
        assert!(keywords(Some(Path::new("notes.unknownext"))).is_empty());
        assert!(keywords(None).is_empty());
    }
}
