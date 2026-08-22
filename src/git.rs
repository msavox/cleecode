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
//! Reading came first and writing came after, in that order on purpose: reading has no way to
//! lose work. What writes is [`stage`], [`unstage`], [`stage_all`], [`commit`], [`discard`] and
//! [`switch`] — and every one of them is a command a person could have typed in the terminal
//! next door, which is the point of going through `git` rather than a library.
//!
//! The spellings are the old ones — `reset HEAD --`, `checkout HEAD --`, `checkout <branch>` —
//! rather than `restore` and `switch`, which arrived in git 2.23 in 2019. That is recent enough
//! to be missing on a long-lived server, and a long-lived server reached over ssh is exactly
//! where a terminal editor earns its keep. The newer commands are nicer to read and would fail
//! in the one place this has to work.

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

/// One file the working tree and the index do not agree about.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub path: PathBuf,
    /// The two letters `git status --porcelain` prints: what the index says, then what the
    /// working tree says. `??` for a file git has never been told about.
    ///
    /// Kept as git's own letters instead of an enum of our own. They are what every other git
    /// tool shows, what the manual pages explain, and there are more of them than a first pass
    /// would guess — `U` for a merge conflict on either side, `T` for a file that turned into a
    /// symlink. An enum would have to grow a fallback arm anyway, and a fallback arm is where
    /// the states nobody thought about go to be shown wrong.
    pub index: char,
    pub worktree: char,
}

impl Change {
    /// A file git has never been told about. It is worth its own question because two things
    /// treat it differently: staging it is `add` like anything else, but there is nothing to
    /// throw its changes back to.
    pub fn untracked(&self) -> bool {
        self.index == '?' && self.worktree == '?'
    }

    pub fn staged(&self) -> bool {
        !self.untracked() && self.index != ' '
    }

    pub fn unstaged(&self) -> bool {
        self.untracked() || self.worktree != ' '
    }
}

/// Everything the panel shows, fetched in one go so switching tabs never waits.
pub struct Snapshot {
    /// The top of the working tree.
    ///
    /// Kept because every path in `changes` is relative to it — `--porcelain` says so — while
    /// the panel is running in whatever directory CleeCode was opened on. Acting on
    /// `src/app.rs` from a root two levels down would name a file that is not there, and git
    /// would answer that the pathspec matched nothing, which is true and unhelpful.
    pub top: Option<PathBuf>,
    /// What is changed, staged and unstaged together — the list you act on.
    pub changes: Vec<Change>,
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
        top: None,
        changes: Vec::new(),
        diff_of: None,
        diff: Vec::new(),
        log: Vec::new(),
        branches: Vec::new(),
        error: None,
    };
    snap.top = toplevel(root);
    if snap.top.is_none() {
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

    if let Ok(text) = git(root, &["status", "--porcelain", "-z"]) {
        snap.changes = parse_changes(&text);
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

/// The files out of `git status --porcelain -z`.
///
/// `-z` rather than the line-based form, and that is not a detail: without it git *quotes* a path
/// with a space or an accent in it — `"src/prova nuova.rs"` — and every caller would have to know
/// how to take the quoting back off. With it the paths arrive exactly as they are on disk,
/// separated by a byte that cannot occur in one.
pub fn parse_changes(text: &str) -> Vec<Change> {
    let mut out = Vec::new();
    let mut fields = text.split('\0');
    while let Some(entry) = fields.next() {
        // "XY p" is the shortest thing that is an entry at all; the trailing empty field that
        // every NUL-terminated list ends with is not one.
        if entry.len() < 4 {
            continue;
        }
        let mut chars = entry.chars();
        let (Some(index), Some(worktree)) = (chars.next(), chars.next()) else { continue };
        // The two letters and the space after them are ASCII, so this is a character boundary.
        let path = PathBuf::from(&entry[3..]);
        // A rename or a copy is followed by the name the file had before, as a field of its own.
        // Skipping it is what keeps the walk in step: read as an entry, `src/old.rs` would
        // become a file whose status letters are `sr`.
        if matches!(index, 'R' | 'C') || matches!(worktree, 'R' | 'C') {
            let _ = fields.next();
        }
        out.push(Change { path, index, worktree });
    }
    out
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

// ---- Writing -------------------------------------------------------------------------------

/// Runs a command for its effect, giving back what git said so the panel can show it.
///
/// The output is kept rather than dropped: `git commit` says which branch, which short hash and
/// how many lines changed, and that sentence is the confirmation the action happened. An error
/// comes back as git wrote it, for the same reason — "pathspec did not match any files" is a
/// better message than anything this could invent from an exit code.
fn write(root: &Path, args: &[&str]) -> Result<String, String> {
    git(root, args).map(|out| out.trim().to_string())
}

/// Puts a file in the index. Works the same for one git has never seen: `add` is how a file
/// becomes tracked, so the panel needs no separate action for it.
pub fn stage(root: &Path, path: &Path) -> Result<String, String> {
    write(root, &["add", "--", &path.to_string_lossy()])
}

/// Takes a file back out of the index, leaving the file itself exactly as it is.
pub fn unstage(root: &Path, path: &Path) -> Result<String, String> {
    write(root, &["reset", "-q", "HEAD", "--", &path.to_string_lossy()])
}

/// Everything, deletions and new files included — which is what `-A` means and `.` does not.
pub fn stage_all(root: &Path) -> Result<String, String> {
    write(root, &["add", "-A"])
}

/// Commits what is staged.
///
/// The message goes as an argument rather than through an editor: there is no terminal to hand
/// git for one, and a commit that opened an editor CleeCode cannot show would hang with no way
/// out. Hooks and signing still run — that is the whole reason this shells out.
pub fn commit(root: &Path, message: &str) -> Result<String, String> {
    write(root, &["commit", "-m", message])
}

/// Throws away the changes to one file: index and working tree both back to `HEAD`.
///
/// The only thing in this module that destroys work — what is thrown away is in no reflog and no
/// stash, and nothing gets it back. `HEAD` and not the index on purpose: "discard my changes to
/// this file" means all of them, and leaving a staged copy behind would be a discard that
/// discarded half.
///
/// Refuses a file git has never been told about, which is not timidity: for those, `checkout`
/// has nothing to put back and the only way to honour the word would be to delete the file. That
/// is `rm` in the terminal, where it reads as what it is.
pub fn discard(root: &Path, change: &Change) -> Result<String, String> {
    if change.untracked() {
        return Err("git has never been told about this file — there is nothing to go back to"
            .to_string());
    }
    write(root, &["checkout", "-q", "HEAD", "--", &change.path.to_string_lossy()])
}

/// Moves to another branch. Refused by git itself when it would write over uncommitted work,
/// which is why it needs no guard here: the answer comes back as git's own refusal.
pub fn switch(root: &Path, branch: &str) -> Result<String, String> {
    write(root, &["checkout", "-q", branch])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two letters are git's own, and both of them mean something: one for the index, one
    /// for the working tree, and a file can be in two states at once.
    #[test]
    fn a_status_line_keeps_both_of_gits_letters() {
        let changes = parse_changes("M  src/app.rs\0 M src/ui.rs\0MM src/git.rs\0?? notes.txt\0");
        assert_eq!(changes.len(), 4);

        assert!(changes[0].staged() && !changes[0].unstaged(), "added and not touched since");
        assert!(!changes[1].staged() && changes[1].unstaged(), "changed and not added");
        assert!(changes[2].staged() && changes[2].unstaged(), "added, then changed again");
        assert!(changes[3].untracked() && changes[3].unstaged() && !changes[3].staged());
        assert_eq!(changes[3].path, PathBuf::from("notes.txt"));
    }

    /// A rename carries the old name as a field of its own. Read as an entry it would become a
    /// file called `rc/old.rs` whose status letters are `sr` — which is why the walk skips it.
    #[test]
    fn the_name_a_renamed_file_used_to_have_is_not_a_file() {
        let changes = parse_changes("R  src/new.rs\0src/old.rs\0 M src/ui.rs\0");
        assert_eq!(changes.len(), 2, "two entries, not three");
        assert_eq!(changes[0].path, PathBuf::from("src/new.rs"));
        assert_eq!(changes[1].path, PathBuf::from("src/ui.rs"));
        assert_eq!((changes[1].index, changes[1].worktree), (' ', 'M'), "still in step");
    }

    /// The whole reason for `-z`: without it git quotes this path and every caller would have to
    /// know how to take the quotes back off.
    #[test]
    fn a_path_with_a_space_in_it_arrives_as_it_is_on_disk() {
        let changes = parse_changes(" M src/prova nuova.rs\0");
        assert_eq!(changes[0].path, PathBuf::from("src/prova nuova.rs"));
    }

    /// A clean tree is an empty list, and the trailing separator every NUL-terminated list ends
    /// with is not a file called nothing.
    #[test]
    fn a_clean_tree_has_nothing_in_it() {
        assert!(parse_changes("").is_empty());
        assert!(parse_changes("\0").is_empty());
    }

    /// The one action that destroys work refuses the one case where it could not honour its own
    /// meaning: there is no `HEAD` version of a file git has never been told about.
    #[test]
    fn discarding_a_file_git_never_saw_is_refused() {
        let untracked = Change { path: PathBuf::from("notes.txt"), index: '?', worktree: '?' };
        assert!(discard(Path::new("."), &untracked).is_err());
    }

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
