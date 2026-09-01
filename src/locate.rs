//! Finding a file and a line inside a line of terminal output.
//!
//! A traceback names where the problem is, and then you retype it into the editor. Double-click
//! the line instead and CleeCode opens it there. Nothing about this is specific to the IDE
//! presets: `cargo`, `gcc`, `eslint`, `pytest` and `grep -n` all say `path:line:column`, and the
//! same double-click works on all of them.
//!
//! Every format here was taken from real output rather than from memory — see the tests, which
//! quote what the tool actually printed.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Location {
    /// As written in the output: absolute, relative, or a bare name that has to be looked for.
    pub path: String,
    /// One-based, as every compiler and interpreter counts them.
    pub line: usize,
    pub column: usize,
    /// True when `path` was a bare Octave function name rather than a file — `boom` for
    /// `boom.m`. It has to be looked for, and looking in the wrong place would open the wrong
    /// file, so the caller is told rather than left to guess.
    pub bare_name: bool,
}

/// The file and line a line of output is pointing at, if it is pointing at one.
///
/// The line and column that come back are one-based, whatever the output said. A tool that
/// prints `file.rs:0` — and some do, for a whole-file message — would otherwise hand a zero to
/// a field documented as one-based, and every caller would have to know not to believe it.
pub fn find(text: &str) -> Option<Location> {
    let mut at = python(text).or_else(|| octave(text)).or_else(|| generic(text))?;
    at.line = at.line.max(1);
    at.column = at.column.max(1);
    Some(at)
}

/// `  File "/abs/path/boom.py", line 2, in boom`
fn python(text: &str) -> Option<Location> {
    let rest = text.trim_start().strip_prefix("File \"")?;
    let (path, rest) = rest.split_once('"')?;
    let rest = rest.trim_start().strip_prefix(", line ")?;
    let line = take_number(rest)?;
    Some(Location { path: path.to_string(), line, column: 1, bare_name: false })
}

/// `    boom at line 3 column 3` or `    tb/run.m at line 1 column 16`
///
/// Octave names the *function* when it has one and the file when it does not, and a function
/// lives in a file of the same name — so a bare name is a lead rather than a location, and is
/// marked as one.
fn octave(text: &str) -> Option<Location> {
    let text = text.trim();
    let (name, rest) = text.split_once(" at line ")?;
    if name.is_empty() || name.contains(' ') {
        return None;
    }
    let line = take_number(rest)?;
    let column = rest
        .split_once(" column ")
        .and_then(|(_, c)| take_number(c))
        .unwrap_or(1);
    let bare_name = !name.ends_with(".m");
    Some(Location { path: name.to_string(), line, column, bare_name })
}

/// `path/to/file.rs:12:5`, `path/to/file.rs:12`, and `path/to/file.rs:12:some text` — which is
/// what `grep -n` prints, and what CleeCode's own project search prints.
///
/// The colon is split at from the left, taking the first place where what comes before it looks
/// like a path. That is what keeps `C:\src\main.rs:12:5` intact: splitting at the drive letter
/// leaves `C`, which looks like nothing, so the scan moves on to the real one.
fn generic(text: &str) -> Option<Location> {
    for token in text.split_whitespace().rev() {
        // `at /path/file.js:12:5)` and `src/app.rs:4045:9: warning` — a trailing bracket, comma
        // or colon is punctuation around the location rather than part of it.
        let token = token.trim_end_matches([')', ']', ',', ';', '.', ':', '\'', '"']);
        let token = token.trim_start_matches(['(', '[', '\'', '"']);
        for (at, _) in token.char_indices().filter(|(_, c)| *c == ':') {
            let (head, tail) = (&token[..at], &token[at + 1..]);
            if !looks_like_path(head) {
                continue;
            }
            // What follows is the line, and then either a column or the matched text.
            let (line_text, rest) = match tail.split_once(':') {
                Some((line, rest)) => (line, Some(rest)),
                None => (tail, None),
            };
            let Some(line) = number(line_text) else { continue };
            let column = rest.and_then(number).unwrap_or(1);
            return Some(Location { path: head.to_string(), line, column, bare_name: false });
        }
    }
    None
}

/// Enough of a path to be worth opening: it has an extension, or a separator, and is not a bare
/// number or a time of day. Without this, `12:30:45` in a log line reads as a file called `12`.
fn looks_like_path(text: &str) -> bool {
    if text.is_empty() || text.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let name = text.rsplit(['/', '\\']).next().unwrap_or(text);
    text.contains('/') || text.contains('\\') || name.contains('.')
}

fn number(text: &str) -> Option<usize> {
    if text.is_empty() || !text.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn take_number(text: &str) -> Option<usize> {
    let digits: String = text.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
    number(&digits)
}

/// Turns a location into a file that exists, or `None`.
///
/// A relative path is relative to the project, which is where the command that printed it was
/// run. A bare Octave function name becomes `<name>.m` and is looked for — shallowly, because a
/// name that matches twice in a big tree is a coin toss, and opening the wrong file is worse
/// than opening none.
pub fn resolve(location: &Location, root: &Path) -> Option<PathBuf> {
    let named = PathBuf::from(&location.path);
    if location.bare_name {
        let file = format!("{}.m", location.path);
        return shallow_find(root, &file, 3);
    }
    if named.is_absolute() {
        return named.exists().then_some(named);
    }
    let joined = root.join(&named);
    if joined.exists() {
        return Some(joined);
    }
    // Not where it was said to be. The name alone is still worth one look — a tool run from a
    // subdirectory prints paths relative to *its* directory, not to the project.
    let name = named.file_name()?.to_str()?.to_string();
    shallow_find(root, &name, 3)
}

/// The first file called `name` within `depth` directories of `root`. Deliberately not a full
/// walk: this runs on a click, and a search that takes a second would feel like a stall.
fn shallow_find(root: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    let direct = root.join(name);
    if direct.exists() {
        return Some(direct);
    }
    if depth == 0 {
        return None;
    }
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let hidden = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with('.') || n == "target" || n == "node_modules")
            .unwrap_or(false);
        if hidden || !path.is_dir() {
            continue;
        }
        if let Some(found) = shallow_find(&path, name, depth - 1) {
            return Some(found);
        }
    }
    None
}

/// The byte offset of the first `http://` or `https://` in `text`, or `None`.
///
/// Parsed by hand like the rest of this file — the crate list has no regex engine and a
/// double-click has no business paying for one. Only http(s) is a URL worth opening: anything
/// else a row prints — `www.`, `ftp://`, a bare hostname — is more likely a word being typed
/// than a link the user is pointing at.
pub fn find_url_start(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 7 <= bytes.len() {
        if bytes[i..].starts_with(b"http://") || bytes[i..].starts_with(b"https://") {
            // The match is ASCII, so `i` is always on a UTF-8 boundary, but say so rather
            // than let a future edit to the match slip a panic into the caller's slice.
            return Some(i).filter(|&i| text.is_char_boundary(i));
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever is on the line under the pointer, including things that are not a location.
    ///
    /// This reads terminal output, which is to say arbitrary text: a log line, a paste, half a
    /// binary file, somebody's stack trace in another alphabet. A double-click may not open the
    /// wrong file and may certainly not bring the editor down, and "no location here" is the
    /// answer for almost every line ever printed.
    #[test]
    fn a_line_that_is_not_a_location_is_not_read_as_one() {
        let very_long = "a".repeat(10_000);
        let odd = [
            "", " ", ":", "::", ":::::", "1:2:3", ":1:", "a:", ":a",
            "-9223372036854775808:1", "99999999999999999999:1:1",
            "file.rs:99999999999999999999", "file.rs:-3", "file.rs:0",
            "C:\\", "C:\\src", "/", "//", "\u{0}\u{1}", "日本語:12:3",
            "\u{1f422}.m:1:1", very_long.as_str(),
            "http://example.com:8080/x", "warning: unused variable at 12:5",
            "        ", "\t\t:\t", "--:--:--", "[2026-08-20 17:21:42] ok",
        ];
        for line in odd {
            if let Some(found) = find(line) {
                assert!(!found.path.is_empty(), "{line:?} gave an empty path");
                assert!(found.line >= 1, "{line:?} gave line {}", found.line);
            }
        }
    }

    /// A timestamp is the line every log prints, and it is three numbers separated by colons.
    #[test]
    fn a_clock_is_not_a_file() {
        for line in ["17:21:42", "[17:21:42] starting", "elapsed 00:03:19"] {
            let found = find(line);
            assert!(
                found.as_ref().is_none_or(|f| f.path.contains('.') || f.path.contains('/')),
                "{line:?} was read as {found:?}"
            );
        }
    }

    fn at(text: &str) -> Option<(String, usize, usize, bool)> {
        find(text).map(|l| (l.path, l.line, l.column, l.bare_name))
    }

    /// Quoted from what python3 actually printed, indentation included.
    #[test]
    fn a_python_traceback_names_its_file_and_line() {
        assert_eq!(
            at("  File \"/tmp/tb/boom.py\", line 2, in boom"),
            Some(("/tmp/tb/boom.py".to_string(), 2, 1, false))
        );
        // A path with a space in it survives, because the quotes are what delimit it.
        assert_eq!(
            at("  File \"/tmp/prova nuova/x.py\", line 40, in <module>"),
            Some(("/tmp/prova nuova/x.py".to_string(), 40, 1, false))
        );
    }

    /// Also quoted from real output. Octave names the function when it has one and the file when
    /// it does not, which is why one of these is a lead and the other is a location.
    #[test]
    fn an_octave_backtrace_names_a_function_or_a_file() {
        assert_eq!(at("    tb/run.m at line 1 column 16"), Some(("tb/run.m".to_string(), 1, 16, false)));
        assert_eq!(at("    boom at line 3 column 3"), Some(("boom".to_string(), 3, 3, true)));
        // The column is optional in some of Octave's messages.
        assert_eq!(at("    boom at line 7"), Some(("boom".to_string(), 7, 1, true)));
    }

    #[test]
    fn the_shape_every_other_tool_uses_works_too() {
        assert_eq!(at("src/app.rs:4045:9: warning: unused"), Some(("src/app.rs".to_string(), 4045, 9, false)));
        assert_eq!(at("  --> src/lsp.rs:120:5"), Some(("src/lsp.rs".to_string(), 120, 5, false)));
        assert_eq!(at("main.c:42: error: expected ';'"), Some(("main.c".to_string(), 42, 1, false)));
        assert_eq!(at("    at /app/server.js:19:11)"), Some(("/app/server.js".to_string(), 19, 11, false)));
    }

    /// A Windows path carries a colon of its own, which is why the numbers are read from the
    /// right rather than the left.
    /// What `grep -n` prints, and `ripgrep`, and CleeCode's own project search: the third field
    /// is the matched line, not a column. Found by double-clicking one and landing nowhere.
    #[test]
    fn a_grep_hit_is_a_location_even_though_the_third_field_is_text() {
        assert_eq!(
            at("dati.txt:5:CERCAMI qui"),
            Some(("dati.txt".to_string(), 5, 1, false))
        );
        assert_eq!(
            at("src/app.rs:120:    let x = 1;"),
            Some(("src/app.rs".to_string(), 120, 1, false))
        );
    }

    #[test]
    fn a_drive_letter_is_not_a_line_number() {
        assert_eq!(
            at(r"C:\src\main.rs:12:5: error"),
            Some((r"C:\src\main.rs".to_string(), 12, 5, false))
        );
    }

    /// The failure this prevents is a click that opens something absurd: a log timestamp reads
    /// exactly like `path:line:column` unless something insists the path looks like one.
    #[test]
    fn a_timestamp_is_not_a_file() {
        assert_eq!(at("12:30:45 INFO ready"), None);
        assert_eq!(at("nothing here at all"), None);
        assert_eq!(at(""), None);
        assert_eq!(at("$ cargo build"), None);
        // A bare word followed by a number is not a location either.
        assert_eq!(at("total 256"), None);
    }

    #[test]
    fn a_relative_path_is_resolved_against_the_project() {
        let root = std::env::temp_dir().join(format!("cleecode_locate_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/app.rs"), "").unwrap();
        std::fs::write(root.join("boom.m"), "").unwrap();

        let here = find("src/app.rs:10:2").unwrap();
        assert_eq!(resolve(&here, &root), Some(root.join("src/app.rs")));

        // A bare Octave name becomes a file and is looked for.
        let bare = find("    boom at line 3 column 3").unwrap();
        assert_eq!(resolve(&bare, &root), Some(root.join("boom.m")));

        // Named relative to somewhere else, but the name alone still finds it.
        let elsewhere = find("../../src/app.rs:1:1").unwrap();
        assert_eq!(resolve(&elsewhere, &root), Some(root.join("src/app.rs")));

        // And something that simply is not there stays not there.
        let missing = find("src/nothing.rs:1:1").unwrap();
        assert_eq!(resolve(&missing, &root), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A URL is handed to the browser, not read as a file location. Only http(s) counts — a
    /// bare `www.` or a typed `ftp://` is more likely a word being written than a link to open.
    #[test]
    fn a_url_is_found_and_a_word_is_not() {
        assert_eq!(find_url_start("see https://example.com/x now"), Some(4));
        assert_eq!(find_url_start("https://example.com"), Some(0));
        assert_eq!(find_url_start("http://example.com"), Some(0));
        assert_eq!(find_url_start("prefix https://a.io/path?q=1 tail"), Some(7));
        assert_eq!(find_url_start("visit www.example.com"), None);
        assert_eq!(find_url_start("ftp://example.com"), None);
        assert_eq!(find_url_start("no url here"), None);
        assert_eq!(find_url_start(""), None);
    }

    /// The offset a caller slices with has to be on a UTF-8 boundary, or the slice panics.
    /// The match is ASCII so this holds today; the test pins it so a future edit cannot
    /// slip a non-ASCII match in and crash every double-click that lands on a URL.
    #[test]
    fn a_found_url_start_is_always_on_a_utf8_boundary() {
        for line in [
            "日本語 https://example.com",
            "café https://example.com",
            "🚀 https://a.io/x",
            "https://example.com",
            "\u{1f422} https://x.io",
        ] {
            if let Some(start) = find_url_start(line) {
                assert!(line.is_char_boundary(start), "{line:?} gave {start}");
            }
        }
    }

    /// A URL with a port reads as a `path:line` — `http://localhost:3000` parses as the file
    /// "http://localhost" at line 3000 — because the generic parser does not know about URLs.
    /// The caller must notice the http(s) path and hand it to the browser instead of the tree.
    #[test]
    fn a_url_with_a_port_reads_as_a_path_the_caller_can_recognise() {
        let found = find("http://localhost:3000").unwrap();
        assert_eq!(found.path, "http://localhost");
        assert_eq!(found.line, 3000);
        assert!(found.path.starts_with("http://"));
    }
}
