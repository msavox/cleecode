//! Search across the project: the same query as the Find box, asked of every file instead of
//! the one in front of you.
//!
//! Deliberately our own walk rather than a call out to `rg` or `grep`. Shelling out would be
//! faster on a very large tree, but it would also mean two dialects of "pattern" — one when you
//! search a file, another when you search the project — and the faster of the two is not
//! installed everywhere, which on Windows means not usually. One engine, one syntax, one set of
//! results, and it runs on a thread so the size of the tree is not the editor's problem.
//!
//! Runs on the same footing as every other slow thing here: a thread, an `mpsc` channel, and a
//! `poll_*` that picks the answer up when it arrives.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

/// One matching line. A hit is a line, not a match: two matches on one line are one place to
/// go, and a list that says the same line twice is a list you have to read twice.
pub struct Hit {
    pub path: PathBuf,
    /// 1-based, as the line numbers on screen and `goto_line` both count.
    pub line: usize,
    /// Char offset of the match within the line, so the cursor lands on the word rather than at
    /// the start of it.
    pub col: usize,
    /// The line itself, with its indentation dropped and its length capped: this is a label in
    /// a list, and a 400-column minified line would push every other result off the screen.
    pub text: String,
}

pub struct Outcome {
    pub query: String,
    pub hits: Vec<Hit>,
    pub files_searched: usize,
    /// Set when the search stopped at `HIT_LIMIT`. Reported rather than hidden: a list that
    /// quietly ends is a list you will trust once too often.
    pub truncated: bool,
    /// Why there are no results, when the reason is the pattern rather than the project.
    pub error: Option<String>,
}

/// Enough results to be worth reading, few enough to stay a list rather than a second copy of
/// the project. Past this the query is the thing to fix, not the limit.
pub const HIT_LIMIT: usize = 2000;

/// Files larger than this are data, a build artefact, or something generated. Reading them is
/// slow and matching them is rarely what was meant.
const FILE_LIMIT: u64 = 2 * 1024 * 1024;

/// The longest a result line is kept at. Beyond this it is cut, with an ellipsis.
const TEXT_LIMIT: usize = 200;

/// Starts a search and returns immediately. `pending` keeps a second one from starting while
/// the first is still going: the walk is the expensive part and two of them race to fill the
/// same list.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    root: PathBuf,
    query: String,
    regex: bool,
    case_sensitive: bool,
    show_hidden: bool,
    tx: Sender<Outcome>,
    pending: Arc<AtomicBool>,
) {
    if pending.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let outcome = run(&root, query, regex, case_sensitive, show_hidden);
        let _ = tx.send(outcome);
        pending.store(false, Ordering::SeqCst);
    });
}

fn run(root: &Path, query: String, regex: bool, case_sensitive: bool, show_hidden: bool) -> Outcome {
    let compiled = match crate::find::compile(&query, regex, case_sensitive) {
        Ok(re) => re,
        Err(error) => {
            return Outcome { query, hits: Vec::new(), files_searched: 0, truncated: false, error: Some(error) };
        }
    };

    let mut files = Vec::new();
    // A file list that stopped at its own cap makes the search partial before it starts, and
    // that is the same news as stopping at `HIT_LIMIT`: what came back is true, and what did not
    // come back was never looked at.
    let capped = crate::app::collect_project_files(root, &mut files, show_hidden);

    let mut hits = Vec::new();
    let mut files_searched = 0usize;
    let mut truncated = capped;
    for path in &files {
        if hits.len() >= HIT_LIMIT {
            truncated = true;
            break;
        }
        if std::fs::metadata(path).map(|m| m.len() > FILE_LIMIT).unwrap_or(true) {
            continue;
        }
        // Anything that is not text reads as an error here, which is the cheapest way to skip a
        // picture without opening it to ask.
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        files_searched += 1;
        for (i, line) in text.lines().enumerate() {
            if hits.len() >= HIT_LIMIT {
                truncated = true;
                break;
            }
            // One hit per line, so `find_from_pos` is never needed: the first match is the place
            // to go and the rest of the line is what gets shown anyway. A line the pattern gave
            // up on (`Err`) is skipped like one that did not match — it has nothing to say about
            // that line, and the rest of the project is still worth searching.
            if let Ok(Some(m)) = compiled.find(line) {
                hits.push(Hit {
                    path: path.clone(),
                    line: i + 1,
                    col: line[..m.start()].chars().count(),
                    text: shorten(line),
                });
            }
        }
    }

    Outcome { query, hits, files_searched, truncated, error: None }
}

/// A line as it appears in the results: no indentation, no more than `TEXT_LIMIT` characters.
///
/// Shared with the lists a language server fills, which have the same row to draw and the same
/// two ways of ruining it: a deeply indented line that pushes its own text off the right of the
/// picker, and a generated line thousands of characters long.
pub fn shorten(line: &str) -> String {
    let trimmed = line.trim_start();
    if trimmed.chars().count() <= TEXT_LIMIT {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(TEXT_LIMIT).collect();
    format!("{cut}…")
}

/// How a hit reads in the list: where it is, then what is there. The path is relative to the
/// project when it can be, since the part that repeats on every row is the part worth dropping.
pub fn label(hit: &Hit, root: &Path) -> String {
    let path = hit.path.strip_prefix(root).unwrap_or(&hit.path);
    format!("{}:{}  {}", path.display(), hit.line, hit.text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cleecode_search_test_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        dir
    }

    #[test]
    fn finds_lines_across_files_and_says_where() {
        let dir = temp_project("basic");
        std::fs::write(dir.join("src/main.rs"), "fn main() {\n    let needle = 1;\n}\n").unwrap();
        std::fs::write(dir.join("notes.txt"), "no match here\nNEEDLE in the second file\n").unwrap();

        let out = run(&dir, "needle".to_string(), false, false, false);
        assert!(out.error.is_none());
        assert_eq!(out.hits.len(), 2, "case-insensitive by default, like the Find box");

        // Rows carry where to go, not just what was found.
        let rs = out.hits.iter().find(|h| h.path.ends_with("main.rs")).expect("the .rs file");
        assert_eq!(rs.line, 2);
        assert_eq!(rs.col, 8, "past the indentation the label drops");
        assert_eq!(rs.text, "let needle = 1;", "shown without its indentation");

        // Strict case is the same search asked the other way.
        let out = run(&dir, "NEEDLE".to_string(), false, true, false);
        assert_eq!(out.hits.len(), 1);
        assert!(out.hits[0].path.ends_with("notes.txt"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The point of one engine: a pattern means here what it means in the Find box, and a
    /// literal query stays literal in both.
    #[test]
    fn patterns_and_literals_read_the_same_as_in_a_file() {
        let dir = temp_project("patterns");
        std::fs::write(dir.join("src/a.rs"), "let a.b = 1;\nlet axb = 2;\n").unwrap();

        let out = run(&dir, "a.b".to_string(), false, false, false);
        assert_eq!(out.hits.len(), 1, "the dot is a dot");
        assert_eq!(out.hits[0].line, 1);

        let out = run(&dir, "a.b".to_string(), true, false, false);
        assert_eq!(out.hits.len(), 2, "as a pattern it matches both");

        let out = run(&dir, "(unclosed".to_string(), true, false, false);
        assert!(out.hits.is_empty());
        assert!(out.error.is_some(), "a broken pattern says so rather than reporting nothing");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A line matching twice is one place to go, and a binary file is not a place at all.
    #[test]
    fn one_hit_per_line_and_nothing_from_a_binary() {
        let dir = temp_project("shape");
        std::fs::write(dir.join("src/twice.rs"), "x and x and x\n").unwrap();
        std::fs::write(dir.join("blob.bin"), [0x78, 0x00, 0xff, 0xfe, 0x78]).unwrap();

        let out = run(&dir, "x".to_string(), false, false, false);
        assert_eq!(out.hits.len(), 1);
        assert_eq!(out.files_searched, 1, "the binary was never read as text");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn long_lines_are_cut_and_labels_are_relative() {
        let dir = temp_project("labels");
        let long = format!("    {}", "ab".repeat(400));
        std::fs::write(dir.join("src/min.js"), format!("{long}\n")).unwrap();

        let out = run(&dir, "abab".to_string(), false, false, false);
        assert_eq!(out.hits.len(), 1);
        let hit = &out.hits[0];
        assert_eq!(hit.text.chars().count(), TEXT_LIMIT + 1, "cut, with the ellipsis");
        assert!(hit.text.ends_with('…'));

        // The project root repeats on every row, so it is not on any of them.
        // Built rather than spelled: a label carries the platform's own separator, and
        // "src/min.js" is not what Windows writes.
        let label = label(hit, &dir);
        let want = format!("{}:1  ", Path::new("src").join("min.js").display());
        assert!(label.starts_with(&want), "got {label}, wanted it to start {want}");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
