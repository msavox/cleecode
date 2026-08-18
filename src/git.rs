//! Reading a repository: what changed, what happened, and where you are.
//!
//! Everything goes through `git` on PATH rather than through libgit2. That is not the usual
//! choice and it is a deliberate one. libgit2 is a C dependency, and this ships to macOS, Linux
//! and Windows MSVC and builds from source under Homebrew — the kind of dependency that has
//! already cost this project a day. It also does not run hooks, does not sign, and does not know
//! about credential helpers, so what it reports and what the user's own `git` reports can differ.
//! Since the whole idea here is real terminals, the answer the panel gives should be the answer
//! the terminal next to it would give.
//!
//! Nothing in this module writes. Staging and committing can be added later; reading has no way
//! to lose work, which is why it comes first.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Fields are separated by U+001F (unit separator): it cannot appear in a branch name, a commit
/// subject or a path, so nothing has to be escaped or guessed at.
const SEP: char = '\u{1f}';

pub struct Commit {
    pub hash: String,
    pub author: String,
    /// Relative, as git itself puts it: "3 days ago" answers the question better than a date.
    pub when: String,
    pub subject: String,
}

pub struct Branch {
    pub name: String,
    pub current: bool,
    pub upstream: Option<String>,
    /// git's own summary of how far apart the two are — "[ahead 2]", "[ahead 1, behind 3]" —
    /// passed through rather than parsed. It is already the sentence anyone wants.
    pub track: Option<String>,
}

/// Everything the panel shows, fetched in one go so switching tabs never waits.
pub struct Snapshot {
    /// Which file the diff is of, when it is of one.
    pub diff_of: Option<PathBuf>,
    pub diff: Vec<String>,
    pub log: Vec<Commit>,
    pub branches: Vec<Branch>,
    /// Set when `root` is not in a repository, or `git` is not on PATH. The panel says so
    /// instead of showing three empty lists, which would look like a clean tree.
    pub error: Option<String>,
}

/// How much history the panel keeps. Enough to recognise where you are; the terminal beside it
/// is the place for archaeology.
const LOG_LIMIT: usize = 50;

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        // A pager would wait for a keypress that can never arrive from here.
        .env("GIT_PAGER", "cat")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { "git failed".to_string() } else { err });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Whether `root` is inside a working tree at all, and where its top is.
pub fn toplevel(root: &Path) -> Option<PathBuf> {
    git(root, &["rev-parse", "--show-toplevel"]).ok().map(|s| PathBuf::from(s.trim()))
}

/// The whole panel's worth of answers. `file` narrows the diff to one path when a file is open;
/// without it the diff is the whole working tree.
pub fn snapshot(root: &Path, file: Option<PathBuf>) -> Snapshot {
    let mut snap = Snapshot {
        diff_of: None,
        diff: Vec::new(),
        log: Vec::new(),
        branches: Vec::new(),
        error: None,
    };
    if toplevel(root).is_none() {
        snap.error = Some("not a git repository".to_string());
        return snap;
    }

    // Against HEAD rather than the index, so staged and unstaged changes are both in the
    // picture: the question the panel answers is "what have I changed", and half an answer
    // depending on whether something was added is worse than none.
    let mut diff_args = vec!["diff", "HEAD", "--"];
    let file_arg;
    if let Some(path) = &file {
        file_arg = path.to_string_lossy().into_owned();
        diff_args.push(&file_arg);
        snap.diff_of = file.clone();
    }
    match git(root, &diff_args) {
        Ok(text) => snap.diff = text.lines().map(str::to_string).collect(),
        Err(e) => snap.error = Some(e),
    }

    let format = format!("--pretty=format:%h{SEP}%an{SEP}%ar{SEP}%s");
    let limit = format!("-{LOG_LIMIT}");
    if let Ok(text) = git(root, &["log", &limit, &format]) {
        snap.log = text.lines().filter_map(parse_commit).collect();
    }

    let format = format!("--format=%(HEAD){SEP}%(refname:short){SEP}%(upstream:short){SEP}%(upstream:track)");
    if let Ok(text) = git(root, &["for-each-ref", "--sort=-committerdate", &format, "refs/heads/"]) {
        snap.branches = text.lines().filter_map(parse_branch).collect();
    }
    snap
}

fn parse_commit(line: &str) -> Option<Commit> {
    let mut parts = line.split(SEP);
    let hash = parts.next()?.to_string();
    let author = parts.next()?.to_string();
    let when = parts.next()?.to_string();
    // A subject can be empty, and the rest of the line is all of it — a subject cannot contain
    // the separator, but joining is still cheaper than trusting that.
    let subject = parts.collect::<Vec<_>>().join(&SEP.to_string());
    (!hash.is_empty()).then_some(Commit { hash, author, when, subject })
}

fn parse_branch(line: &str) -> Option<Branch> {
    let mut parts = line.split(SEP);
    let head = parts.next()?;
    let name = parts.next()?.to_string();
    let upstream = parts.next().unwrap_or("").trim().to_string();
    let track = parts.next().unwrap_or("").trim().to_string();
    (!name.is_empty()).then_some(Branch {
        name,
        current: head.trim() == "*",
        upstream: (!upstream.is_empty()).then_some(upstream),
        track: (!track.is_empty()).then_some(track),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_commit_line_splits_into_its_four_parts() {
        let line = format!("a1b2c3d{SEP}Ada Lovelace{SEP}3 days ago{SEP}Teach the engine to count");
        let c = parse_commit(&line).expect("a full line parses");
        assert_eq!(c.hash, "a1b2c3d");
        assert_eq!(c.author, "Ada Lovelace");
        assert_eq!(c.when, "3 days ago");
        assert_eq!(c.subject, "Teach the engine to count");

        // An empty subject is a real commit, not a parse failure.
        let line = format!("a1b2c3d{SEP}A{SEP}now{SEP}");
        assert_eq!(parse_commit(&line).expect("still a commit").subject, "");
        // A truncated line is not a commit, and must not become one with empty fields.
        assert!(parse_commit("a1b2c3d").is_none());
        assert!(parse_commit("").is_none());
    }

    #[test]
    fn a_branch_line_knows_which_one_you_are_on() {
        let line = format!("*{SEP}main{SEP}origin/main{SEP}[ahead 2]");
        let b = parse_branch(&line).expect("a full line parses");
        assert!(b.current);
        assert_eq!(b.name, "main");
        assert_eq!(b.upstream.as_deref(), Some("origin/main"));
        assert_eq!(b.track.as_deref(), Some("[ahead 2]"));

        // A branch with no upstream has none, rather than an empty one that would draw as a
        // stray separator.
        let line = format!("{SEP}spike{SEP}{SEP}");
        let b = parse_branch(&line).expect("parses");
        assert!(!b.current);
        assert_eq!(b.name, "spike");
        assert!(b.upstream.is_none() && b.track.is_none());
    }

    /// Somewhere that is not a repository has to say so. Three empty lists would read as a
    /// clean tree, which is the opposite of the truth.
    #[test]
    fn outside_a_repository_it_says_so_rather_than_looking_clean() {
        let dir = std::env::temp_dir().join(format!("cleecode_git_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // /tmp on macOS is not inside a repo; if the temp dir ever were, this would be a false
        // pass rather than a false failure, so it is checked rather than assumed.
        if toplevel(&dir).is_none() {
            let snap = snapshot(&dir, None);
            assert!(snap.error.is_some());
            assert!(snap.diff.is_empty() && snap.log.is_empty() && snap.branches.is_empty());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
