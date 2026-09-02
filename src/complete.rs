//! Word completion: the popup that hangs under the cursor while you type.
//!
//! The candidates come from the words already in the open buffers, plus the keywords of the
//! language the file is written in. That is a deliberate floor rather than a stopgap: it needs
//! no server to install and no network, it works in a config file and in a language nobody
//! wrote a parser for, and it is what remains when anything smarter is unavailable.
//!
//! A language server is a *second source* into the same popup, not a second popup — which is why
//! [`Source`] existed before there was anything but the buffer to put in it. It arrives late, by
//! its nature: the question goes out when the popup opens and the answer comes back frames later.
//! So the popup is never waiting on it — it opens on the words in the file, and the server's
//! names are folded into a list that is already on screen. See [`Popup::absorb`].
//!
//! There is one place where that order is deliberately reversed. After a trigger character — a
//! `.` or a `::`, see [`trigger_at`] — the words in the file are not merely incomplete, they are
//! *wrong*: what can follow a dot is decided by a type, and no amount of reading the buffer will
//! find the methods of one. So there the question goes out first and the popup opens on the
//! answer, or does not open at all. A list that flashes up full of the wrong words and corrects
//! itself two frames later is worse than one that appears once, already right. Such a popup is
//! marked [`Popup::triggered`], and it is the one that is allowed to stand with nothing typed
//! into it yet — see [`prefix_from`] for the anchor that keeps it honest.

use crate::picker::fuzzy_score;
use crossterm::event::{KeyCode, KeyModifiers};
use ropey::Rope;
use std::collections::HashMap;
use std::path::Path;

/// How many word characters must be typed before the popup appears on its own, and below which
/// it closes again. Two is enough to mean something and short enough not to arrive late.
///
/// It is a floor on *guessing*: below two letters the buffer's words are a list of everything,
/// which is no answer. A popup a trigger character opened is not guessing — the server was asked
/// about that exact position — so it is exempt, and stands with nothing typed into it at all.
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
    /// A name that exists in the interpreter *right now*: a variable the session is holding.
    ///
    /// The third source the seam was built for, and the one no amount of reading the file can
    /// supply — something defined at the prompt is in no buffer at all. It is offered as though
    /// it were on the cursor's own line, because that is how present it is.
    Session,
    /// A name a language server said could go here.
    ///
    /// The only source that knows what the *cursor* is looking at rather than what the file
    /// contains: after `self.` it offers the methods of that type, none of which need appear in
    /// the buffer at all. It arrives after the popup is up, so it is the one source that is
    /// folded in rather than scanned — see [`Popup::absorb`].
    Lsp,
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

    /// Adds the names an interpreter is holding right now.
    ///
    /// Distance zero: a variable that exists in the session is as near as a word can be, and
    /// nearer than one written forty lines up. A name that is both is one entry — the index is
    /// keyed by the text — and stays whichever the first one said, which is the buffer's, so it
    /// keeps the buffer's distance. That is the right answer either way: it is in both places.
    pub fn add_session(&mut self, names: &[String]) {
        for name in names {
            if name.chars().count() >= MIN_WORD {
                self.add(name, Source::Session, 0);
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
                // happens to contain one. A word that is also live in the session keeps whatever
                // it was — it is in both places, and where it is drawn from matters less than
                // that it is offered.
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

/// Turns a server's answer into candidates, in the order it gave them.
///
/// `distance` carries the server's ranking, and that is not an abuse of the field: distance means
/// "how near this is to the cursor", and a server's ordering is its own answer to exactly that
/// question — measured by what it knows about the position rather than by counting lines. So the
/// existing tie-break inside a tier goes on meaning one thing, and the server's first suggestion
/// competes with a word on the cursor's own line, which is about right.
///
/// The [`MIN_WORD`] floor is deliberately not applied. It exists because a word *scraped out of
/// text* is a guess, and a short guess is not worth a row in a list; a name a server offers is
/// not a guess. `fn`, `if` and `ok` belong in the list when the server says they belong there —
/// and [`rank`] already drops the one that is exactly what has been typed.
pub fn lsp_candidates(words: &[String]) -> Vec<Candidate> {
    words
        .iter()
        .enumerate()
        .map(|(i, text)| Candidate { text: text.clone(), source: Source::Lsp, distance: i, freq: 1 })
        .collect()
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

/// What has been typed since an anchor, when the cursor is still writing at it.
///
/// The question [`prefix_at`] cannot answer. That one reads backwards from the cursor and refuses
/// an empty word, which is load-bearing where it is used: a popup that opened on its own must be
/// about a word somebody is actually typing. A popup a trigger character opened is anchored
/// instead — it belongs to the position right after the `.`, and at the instant it opens there is
/// nothing typed into it yet. Reading backwards from the cursor there would find the word *before*
/// the dot, which is the one thing the list is not about.
///
/// So this reads forwards from the anchor, and `None` is the popup's death: the cursor has gone
/// to another line, or behind the anchor (backspaced past the `.`), or something that is not a
/// word character sits in between (a space, a bracket, another dot). Everything else is the
/// prefix, the empty string included.
pub fn prefix_from(rope: &Rope, start: usize, line: usize, col: usize) -> Option<String> {
    if line >= rope.len_lines() {
        return None;
    }
    let line_start = rope.line_to_char(line);
    // An anchor on another line is an anchor the cursor has left. Checked before the subtraction
    // below, which would underflow on exactly that case.
    if start < line_start {
        return None;
    }
    let chars: Vec<char> = rope.line(line).chars().collect();
    let col = col.min(chars.len());
    let from = start - line_start;
    // Behind the anchor, or off the end of a line that has since been cut short.
    if from > col || from > chars.len() {
        return None;
    }
    let run: String = chars[from..col].iter().collect();
    run.chars().all(is_word_char).then_some(run)
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
    /// Whether the arrows have been used since this list was built.
    ///
    /// The difference between a selection and a default. Row zero is where the popup opens, not
    /// somewhere the user chose to be — so a late answer from the server is free to take that
    /// row, and must, or its best suggestion arrives underneath the highlight and Enter types
    /// the wrong word. Once the arrows have been touched, the pick is the user's and is kept.
    touched: bool,
    /// Whether a trigger character opened this, rather than a word being typed.
    ///
    /// Two things follow from it, and they are the same thing said twice: this popup is about a
    /// *position* rather than about a word, so it may stand with nothing typed into it, and it is
    /// kept alive by its anchor ([`prefix_from`]) rather than by the word under the cursor. A
    /// popup that opened on its own is the other case in both, and neither rule crosses over.
    pub triggered: bool,
}

impl Popup {
    /// `None` when nothing matches, which is also the answer to "should this be open at all".
    pub fn open(editor: usize, start: usize, prefix: String, candidates: Vec<Candidate>) -> Option<Self> {
        let matches = rank(&prefix, &candidates);
        if matches.is_empty() {
            return None;
        }
        Some(Popup {
            editor,
            start,
            prefix,
            candidates,
            matches,
            selected: 0,
            scroll: 0,
            touched: false,
            triggered: false,
        })
    }

    /// The popup a trigger character opens, on the server's answer and on nothing else.
    ///
    /// No buffer index is folded in, and that is the whole point rather than an economy: after a
    /// dot the words in the file are the wrong list, and this feature exists to beat them. An
    /// answer with nothing in it opens nothing — `None` here is a server that had no members to
    /// offer, and a popup showing the file's words instead would be answering a question nobody
    /// asked.
    pub fn from_trigger(
        editor: usize,
        start: usize,
        prefix: String,
        candidates: Vec<Candidate>,
    ) -> Option<Self> {
        let mut popup = Popup::open(editor, start, prefix, candidates)?;
        popup.triggered = true;
        Some(popup)
    }

    /// Re-filters against a new prefix. `false` means the popup should close: either the word
    /// shrank below the threshold, or nothing matches it any more.
    ///
    /// The threshold is [`MIN_PREFIX`] for a popup that opened on a word and none at all for one
    /// a trigger opened: there, an empty prefix is where it started, and closing on it would shut
    /// the list in the same breath as opening it.
    pub fn refilter(&mut self, prefix: &str) -> bool {
        if !self.triggered && prefix.chars().count() < MIN_PREFIX {
            return false;
        }
        self.prefix = prefix.to_string();
        self.matches = rank(prefix, &self.candidates);
        self.selected = 0;
        self.scroll = 0;
        // A different word is a different list: whatever was picked was picked out of the old
        // one, and holding on to it would carry a choice across to somewhere it was never made.
        self.touched = false;
        !self.matches.is_empty()
    }

    /// Folds in candidates that arrived after the popup opened — the language server's answer.
    ///
    /// The hazard this exists to avoid is the list moving under a finger that is already on it.
    /// So a pick made with the arrows stays picked, found again by its text after the re-rank.
    /// A finger that has not moved is not on the list: there, the top row is where the popup
    /// opened rather than anywhere the user chose, and the server's best suggestion has to be
    /// allowed to take it — otherwise the good name arrives underneath the highlight and Enter
    /// still types the word that was there before. Re-ranking rather than appending is the same
    /// point from the other side: a suggestion that belongs at the top must be able to get there,
    /// or the second source is a footnote.
    ///
    /// Nothing happens at all when there is nothing new. A reply that only repeats words already
    /// scraped out of the buffer must not cost the user their selection.
    pub fn absorb(&mut self, extra: Vec<Candidate>) {
        let known: std::collections::HashSet<String> =
            self.candidates.iter().map(|c| c.text.clone()).collect();
        let before = self.candidates.len();
        // The buffer's word wins a tie on text, and it is not an arbitrary choice: it already
        // carries a real distance and a real frequency, which is more than a duplicate would.
        self.candidates.extend(extra.into_iter().filter(|c| !known.contains(&c.text)));
        if self.candidates.len() == before {
            return;
        }
        let held = self.touched.then(|| self.selected().map(|c| c.text.clone())).flatten();
        self.matches = rank(&self.prefix, &self.candidates);
        self.selected = 0;
        self.scroll = 0;
        if let Some(text) = held {
            self.select_text(&text);
        }
    }

    /// Puts the selection back on a word by name, and scrolls to it. Silent when it is gone,
    /// which leaves the selection where the caller put it.
    fn select_text(&mut self, text: &str) {
        let found = self
            .matches
            .iter()
            .position(|&i| self.candidates.get(i).is_some_and(|c| c.text == text));
        let Some(row) = found else { return };
        self.selected = row;
        self.scroll = if row < MAX_ROWS { 0 } else { row + 1 - MAX_ROWS };
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.matches.is_empty() {
            return;
        }
        let len = self.matches.len() as isize;
        let idx = ((self.selected as isize + delta) % len + len) % len;
        self.selected = idx as usize;
        self.touched = true;
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

/// Whether the character just typed is one a language server should be asked about, and which
/// one to tell it about.
///
/// `before` is the line to the left of the cursor, so it ends in the character that was just
/// written. Two triggers, hardcoded, and the deliberate v1: nothing in this program stores what a
/// server said it could do — the handshake reply is read for its position encoding and for
/// nothing else — so there is no table to look `completionProvider.triggerCharacters` up in.
/// Hardcoding two is the honest shape of that: `.` and `::` are the two every language this
/// editor knows about spells the same way, and the day one needs another — C++ and Rust both
/// offer members through `->`, which rust-analyzer and clangd both list — the fix is to keep the
/// server's own list at initialize and ask it here, not to lengthen this `match`.
///
/// A single `:` is not a trigger. It is a type annotation, a dictionary, a label and a ternary
/// far more often than it is half a path, and a popup on every one of those would be a popup on
/// nearly every line.
pub fn trigger_at(code: KeyCode, ctrl: bool, before: &str) -> Option<char> {
    if ctrl {
        return None;
    }
    let KeyCode::Char(c) = code else { return None };
    match c {
        '.' => Some('.'),
        ':' if before.ends_with("::") => Some(':'),
        _ => None,
    }
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

    /// The server's answer competes on the same terms as everything else — no tier of its own —
    /// and wins inside a tier because its ranking arrives as a distance of nearly nothing.
    #[test]
    fn a_servers_first_suggestion_outranks_a_word_from_across_the_file() {
        let cands = vec![
            cand("config_path", Source::Buffer, 300, 4),
            cand("config_reload", Source::Lsp, 0, 1),
            cand("config_write", Source::Lsp, 1, 1),
        ];
        assert_eq!(ranked("config", &cands), ["config_reload", "config_write", "config_path"]);

        // But a word on the cursor's own line still beats the server's fourth suggestion, which
        // is the point of letting them share a scale instead of stacking them.
        let near = vec![cand("config_path", Source::Buffer, 0, 1), cand("config_x", Source::Lsp, 4, 1)];
        assert_eq!(ranked("config", &near), ["config_path", "config_x"]);
    }

    /// The list may not move under a finger that is already on it. What was picked stays picked,
    /// found again by name after the re-rank.
    #[test]
    fn a_late_answer_keeps_whatever_was_already_picked() {
        let cands = vec![
            cand("render_frame", Source::Buffer, 2, 1),
            cand("render_line", Source::Buffer, 3, 1),
        ];
        let mut popup = Popup::open(0, 0, "render".to_string(), cands).unwrap();
        popup.move_selection(1);
        assert_eq!(popup.selected().unwrap().text, "render_line");

        // Two names the server offers, both of which would otherwise take the top rows.
        popup.absorb(lsp_candidates(&["render_all".to_string(), "render_cell".to_string()]));
        assert_eq!(popup.len(), 4, "the new names are in the list");
        assert_eq!(popup.selected().unwrap().text, "render_line", "and the picked row is still it");
        assert_eq!(
            popup.visible().next().unwrap().0.text,
            "render_all",
            "the server's first suggestion did reach the top — it was not merely appended"
        );
    }

    /// A reply that only repeats what was already there must cost nothing at all — not the
    /// selection, and not the scroll.
    #[test]
    fn an_answer_with_nothing_new_in_it_leaves_the_popup_alone() {
        let cands = vec![
            cand("render_frame", Source::Buffer, 2, 1),
            cand("render_line", Source::Buffer, 3, 1),
        ];
        let mut popup = Popup::open(0, 0, "render".to_string(), cands).unwrap();
        popup.move_selection(1);
        popup.absorb(lsp_candidates(&["render_frame".to_string(), "render_line".to_string()]));
        assert_eq!(popup.len(), 2);
        assert_eq!(popup.selected().unwrap().text, "render_line");
    }

    /// Row zero before the arrows are touched is a default, not a choice — so the server's best
    /// suggestion takes it. Getting this wrong is quiet and expensive: the good name is in the
    /// list, the highlight is on the old one, and Enter types the wrong word.
    #[test]
    fn a_late_answer_with_nothing_picked_yet_opens_on_the_best_row() {
        let cands = vec![cand("render_frame", Source::Buffer, 2, 1)];
        let mut popup = Popup::open(0, 0, "render".to_string(), cands).unwrap();
        popup.absorb(lsp_candidates(&["render_all".to_string()]));
        assert_eq!(popup.selected, 0);
        assert_eq!(popup.selected().unwrap().text, "render_all");
    }

    /// A picked row far down the list has to still be on screen after the re-rank, or keeping
    /// the selection would only mean losing sight of it.
    #[test]
    fn keeping_the_selection_scrolls_it_back_into_view() {
        let cands: Vec<Candidate> =
            (0..20).map(|i| cand(&format!("render_{i:02}"), Source::Buffer, i, 1)).collect();
        let mut popup = Popup::open(0, 0, "render".to_string(), cands).unwrap();
        for _ in 0..15 {
            popup.move_selection(1);
        }
        let held = popup.selected().unwrap().text.clone();
        popup.absorb(lsp_candidates(&["render_zz".to_string()]));
        assert_eq!(popup.selected().unwrap().text, held);
        let rows: Vec<&str> = popup.visible().map(|(c, _)| c.text.as_str()).collect();
        assert!(rows.contains(&held.as_str()), "{rows:?} does not show {held}");
    }

    /// The server's order is what `distance` carries, and the short names it offers are not
    /// held to the floor that keeps guessed words out of the list.
    #[test]
    fn the_servers_words_keep_its_order_and_are_not_measured_for_length() {
        let cands = lsp_candidates(&["ok".to_string(), "fn".to_string(), "config".to_string()]);
        assert_eq!(cands.len(), 3, "a two-letter name a server offered is not a guess");
        assert_eq!((cands[0].distance, cands[1].distance, cands[2].distance), (0, 1, 2));
        assert!(cands.iter().all(|c| c.source == Source::Lsp));
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

    /// The point of a third source: a variable made at the prompt is in no buffer, so nothing
    /// that reads the file can offer it, however long you have been using it.
    #[test]
    fn a_name_that_exists_only_in_the_session_is_offered() {
        let mut index = Index::new();
        index.add_buffer(&Rope::from_str("plot(measurements)\n"), Some(0));
        index.add_session(&["measurements_raw".to_string(), "calibration".to_string()]);
        let cands = index.into_candidates();
        assert_eq!(ranked("cal", &cands), vec!["calibration"]);
        // And it is as near as a word can be: nearer than one written far up the file.
        let live = cands.iter().find(|c| c.text == "calibration").unwrap();
        assert_eq!((live.source, live.distance), (Source::Session, 0));
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

    /// The anchor a triggered popup lives on: what has been typed since the `.`, and nothing read
    /// backwards from the cursor. The empty answer is the interesting one — it is the state the
    /// popup opens in.
    #[test]
    fn an_anchor_reads_forwards_and_an_empty_run_is_an_answer() {
        let rope = Rope::from_str("value.push\nsecond line\n");
        // Right after the dot, nothing typed yet.
        assert_eq!(prefix_from(&rope, 6, 0, 6), Some(String::new()));
        // And as the word grows under it.
        assert_eq!(prefix_from(&rope, 6, 0, 8), Some("pu".to_string()));
        assert_eq!(prefix_from(&rope, 6, 0, 10), Some("push".to_string()));
    }

    #[test]
    fn an_anchor_dies_when_the_cursor_leaves_it() {
        let rope = Rope::from_str("value.push here\nsecond line\n");
        // Backspaced past the `.`: the cursor is behind the anchor.
        assert_eq!(prefix_from(&rope, 6, 0, 5), None);
        // Something that is not a word character in between — the popup is about the run right
        // after the dot, and a space ended it.
        assert_eq!(prefix_from(&rope, 6, 0, 12), None);
        // Another line entirely, and a line that does not exist.
        assert_eq!(prefix_from(&rope, 6, 1, 3), None);
        assert_eq!(prefix_from(&rope, 6, 9, 0), None);
    }

    /// With nothing typed yet every name is in the exact-prefix tier, so the order that reaches
    /// the screen is the server's own — which is the only ranking there is after a dot.
    #[test]
    fn an_empty_prefix_leaves_the_servers_order_alone() {
        let cands = lsp_candidates(&[
            "push_back".to_string(),
            "pop_front".to_string(),
            "len_of".to_string(),
        ]);
        assert_eq!(ranked("", &cands), vec!["push_back", "pop_front", "len_of"]);
    }

    /// The exemption, and the anchor that bounds it: a triggered popup stands on an empty prefix,
    /// narrows as the word grows, and closes when nothing matches — never merely for being short.
    #[test]
    fn a_triggered_popup_stands_with_nothing_typed_into_it() {
        let cands = lsp_candidates(&["push_back".to_string(), "pop_front".to_string()]);
        let mut popup = Popup::from_trigger(0, 6, String::new(), cands).unwrap();
        assert!(popup.triggered);
        assert_eq!(popup.len(), 2);
        // One letter would have closed a popup that opened on a word.
        assert!(popup.refilter("p"));
        assert_eq!(popup.len(), 2);
        assert!(popup.refilter("pu"));
        assert_eq!(popup.len(), 1);
        // Back to nothing typed, which is where it started.
        assert!(popup.refilter(""));
        assert_eq!(popup.len(), 2);
        // And a word the server never offered closes it, as it closes any other list.
        assert!(!popup.refilter("zz"));
    }

    #[test]
    fn a_popup_that_opened_on_a_word_keeps_its_floor() {
        let cands = vec![cand("config_path", Source::Buffer, 0, 1)];
        let mut popup = Popup::open(0, 0, "conf".to_string(), cands).unwrap();
        assert!(!popup.triggered);
        assert!(!popup.refilter("c"), "the exemption belongs to the trigger, not to every popup");
    }

    /// An answer with nothing in it opens nothing: the buffer's words are not the fallback here,
    /// they are the thing being replaced.
    #[test]
    fn a_trigger_with_no_answer_opens_no_popup() {
        assert!(Popup::from_trigger(0, 6, String::new(), Vec::new()).is_none());
    }

    #[test]
    fn a_dot_triggers_and_a_lone_colon_does_not() {
        let dot = KeyCode::Char('.');
        let colon = KeyCode::Char(':');
        assert_eq!(trigger_at(dot, false, "value."), Some('.'));
        assert_eq!(trigger_at(colon, false, "std::"), Some(':'));
        // A type annotation, a dictionary, a label: one colon is none of the editor's business.
        assert_eq!(trigger_at(colon, false, "let x:"), None);
        // A chord that happens to leave a dot behind is still a chord, and no other key triggers.
        assert_eq!(trigger_at(dot, true, "value."), None);
        assert_eq!(trigger_at(KeyCode::Char('a'), false, "valuea"), None);
        assert_eq!(trigger_at(KeyCode::Enter, false, ""), None);
    }

    #[test]
    fn only_a_known_extension_brings_keywords() {
        assert!(keywords(Some(Path::new("main.rs"))).contains(&"impl"));
        assert!(keywords(Some(Path::new("notes.unknownext"))).is_empty());
        assert!(keywords(None).is_empty());
    }
}
