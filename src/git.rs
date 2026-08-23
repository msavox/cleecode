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
    pub branches: Vec<Branch>,
    /// Every branch at once, with the parent links the lane layout draws from.
    pub graph: Vec<GraphCommit>,
    pub stashes: Vec<Stash>,
    /// The remotes by name, kept because it is what tells `origin/main` from a local branch
    /// called `feature/login` — both have a slash and only one of them is somewhere else.
    pub remotes: Vec<String>,
    /// A merge, pick, revert or rebase left half-done. The panel offers the way out of it, and
    /// only while there is one to offer.
    pub unfinished: Option<Unfinished>,
    /// Set when `root` is not in a repository, or `git` is not on PATH. The panel says so
    /// instead of showing three empty lists, which would look like a clean tree.
    pub error: Option<String>,
}

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
        branches: Vec::new(),
        graph: Vec::new(),
        stashes: Vec::new(),
        remotes: Vec::new(),
        unfinished: None,
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

    let format = format!("--format=%(HEAD){SEP}%(refname:short){SEP}%(upstream:short){SEP}%(upstream:track)");
    if let Ok(text) = git(root, &["for-each-ref", "--sort=-committerdate", &format, "refs/heads/"]) {
        snap.branches = text.lines().filter_map(parse_branch).collect();
    }

    // Asked before the graph, because the graph cannot label a ref without it.
    snap.remotes = remotes(root);
    snap.graph = graph(root, &snap.remotes);
    snap.stashes = stashes(root);
    snap.unfinished = unfinished(root);
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

// ---- The history as a shape ------------------------------------------------------------------

/// What points at a commit, kept apart by kind because they are read differently: a branch is
/// where work continues, a tag is a place someone marked, and the remote's copy of a branch is
/// the answer to "have I pushed this yet".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RefKind {
    /// The branch that is checked out. Drawn first and apart: it is the answer to "where am I".
    Head,
    Local,
    Remote,
    Tag,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RefName {
    pub kind: RefKind,
    pub text: String,
}

/// A commit as the graph needs it. `parents` is what makes it a graph rather than a list, and it
/// is why this is a second query rather than a field on [`Commit`]: the log tab wants the newest
/// fifty on this branch, the graph wants every branch at once and the shape between them.
#[derive(Clone, Debug)]
pub struct GraphCommit {
    pub hash: String,
    pub parents: Vec<String>,
    pub refs: Vec<RefName>,
    pub author: String,
    pub when: String,
    pub subject: String,
}

/// One entry of `git stash list`.
#[derive(Clone, Debug, PartialEq)]
pub struct Stash {
    /// `stash@{0}` — git's own name for it, which is also what every command here takes.
    pub name: String,
    pub subject: String,
}

/// A command git left half-finished, found by the file it leaves behind while it waits.
///
/// Asked of the filesystem rather than of `git status`, whose porcelain says the same thing in
/// prose that changes between versions. These four paths have been where they are since long
/// before any git a person is likely to be running.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unfinished {
    Merge,
    CherryPick,
    Revert,
    Rebase,
}

impl Unfinished {
    /// The command that puts the repository back the way it was before it started.
    fn abort(self) -> &'static str {
        match self {
            Unfinished::Merge => "merge",
            Unfinished::CherryPick => "cherry-pick",
            Unfinished::Revert => "revert",
            Unfinished::Rebase => "rebase",
        }
    }
}

/// Whether a merge, a cherry-pick, a revert or a rebase is sitting half-done in `top`.
///
/// `git_dir` rather than `top/.git`, because a worktree's `.git` is a file naming somewhere else
/// and a submodule's is a directory under the parent — asking git where it keeps its own state is
/// the only version of this that is right everywhere.
pub fn unfinished(root: &Path) -> Option<Unfinished> {
    let dir = PathBuf::from(git(root, &["rev-parse", "--git-dir"]).ok()?.trim());
    let dir = if dir.is_absolute() { dir } else { root.join(dir) };
    // Ordered so the most specific wins: a rebase that stops on a conflict also leaves a
    // CHERRY_PICK_HEAD behind when it is replaying a picked commit, and calling that a
    // cherry-pick would offer an abort that undoes one commit out of a whole rebase.
    if dir.join("rebase-merge").exists() || dir.join("rebase-apply").exists() {
        return Some(Unfinished::Rebase);
    }
    if dir.join("MERGE_HEAD").exists() {
        return Some(Unfinished::Merge);
    }
    if dir.join("REVERT_HEAD").exists() {
        return Some(Unfinished::Revert);
    }
    if dir.join("CHERRY_PICK_HEAD").exists() {
        return Some(Unfinished::CherryPick);
    }
    None
}

/// How much of the graph is drawn. Larger than the log's fifty because a graph is read for its
/// shape and a shape needs room, and still bounded: `--all` on a long-lived repository is every
/// commit anyone ever made.
const GRAPH_LIMIT: usize = 400;

/// Every branch at once, newest first, with the parent links that make it a graph.
///
/// `--date-order` and not `--topo-order`: both promise that no commit is drawn above its own
/// children, which is the one thing the lane layout needs to be correct, and date order keeps
/// work done on the same afternoon on the same screen. `--topo-order` would gather each branch
/// into an unbroken run, which reads well on paper and puts this morning's commit forty rows
/// below yesterday's.
pub fn graph(root: &Path, remotes: &[String]) -> Vec<GraphCommit> {
    let format = format!("--pretty=format:%h{SEP}%p{SEP}%D{SEP}%an{SEP}%ar{SEP}%s");
    let limit = format!("--max-count={GRAPH_LIMIT}");
    let Ok(text) = git(root, &["log", "--all", "--date-order", &limit, &format]) else {
        return Vec::new();
    };
    text.lines().filter_map(|line| parse_graph_commit(line, remotes)).collect()
}

fn parse_graph_commit(line: &str, remotes: &[String]) -> Option<GraphCommit> {
    let mut parts = line.split(SEP);
    let hash = parts.next()?.to_string();
    if hash.is_empty() {
        return None;
    }
    let parents = parts.next()?.split_whitespace().map(str::to_string).collect();
    let refs = parse_refs(parts.next().unwrap_or(""), remotes);
    let author = parts.next().unwrap_or("").to_string();
    let when = parts.next().unwrap_or("").to_string();
    let subject = parts.collect::<Vec<_>>().join(&SEP.to_string());
    Some(GraphCommit { hash, parents, refs, author, when, subject })
}

/// What `%D` writes: `HEAD -> main, origin/main, tag: v0.1`.
///
/// The remotes are passed in rather than guessed at from the slash. A branch called
/// `feature/login` has a slash in it and is not a remote branch, and a remote can be called
/// anything at all — `git remote` is the only thing that knows which is which.
pub fn parse_refs(decoration: &str, remotes: &[String]) -> Vec<RefName> {
    decoration
        .split(", ")
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            if let Some(tag) = entry.strip_prefix("tag: ") {
                return Some(RefName { kind: RefKind::Tag, text: tag.to_string() });
            }
            if let Some(branch) = entry.strip_prefix("HEAD -> ") {
                return Some(RefName { kind: RefKind::Head, text: branch.to_string() });
            }
            if entry == "HEAD" {
                // A detached HEAD. Worth naming rather than dropping: it is exactly the state
                // where knowing where you are is hardest and matters most.
                return Some(RefName { kind: RefKind::Head, text: "HEAD".to_string() });
            }
            // `origin/HEAD` is the remote's idea of its own default branch. It sits on the same
            // commit as `origin/main` for the whole life of most repositories, so drawing it
            // doubles the label and says nothing.
            if entry.ends_with("/HEAD") {
                return None;
            }
            let remote = remotes.iter().any(|r| entry.starts_with(&format!("{r}/")));
            Some(RefName {
                kind: if remote { RefKind::Remote } else { RefKind::Local },
                text: entry.to_string(),
            })
        })
        .collect()
}

/// The branch that is checked out and whether it has an upstream, asked of git rather than read
/// off the panel's snapshot.
///
/// The snapshot would be free and is not always true: the shell next door can change branch, and
/// a push built on a stale answer pushes the wrong one. Two `rev-parse` calls cost a couple of
/// milliseconds on any repository, which is what the frame loop can afford and what being right
/// costs here.
///
/// A detached HEAD gives `None`: `rev-parse --abbrev-ref` answers "HEAD", which is the one
/// branch name that names nothing.
pub fn head_branch(root: &Path) -> (Option<String>, bool) {
    let branch = git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "HEAD");
    let upstream = git(root, &["rev-parse", "--abbrev-ref", "@{u}"]).is_ok();
    (branch, upstream)
}

/// The remotes by name, which is what tells a remote-tracking branch from a local one with a
/// slash in its name.
pub fn remotes(root: &Path) -> Vec<String> {
    git(root, &["remote"])
        .map(|text| text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
        .unwrap_or_default()
}

pub fn stashes(root: &Path) -> Vec<Stash> {
    let format = format!("--pretty=format:%gd{SEP}%s");
    let Ok(text) = git(root, &["stash", "list", &format]) else { return Vec::new() };
    text.lines().filter_map(parse_stash).collect()
}

fn parse_stash(line: &str) -> Option<Stash> {
    let (name, subject) = line.split_once(SEP)?;
    (!name.is_empty()).then(|| Stash { name: name.to_string(), subject: subject.to_string() })
}

/// One commit in full: the message, what it touched, and the patch.
///
/// `--cc` rather than plain `-p` so a merge commit is not a blank page. `git show` on a merge
/// shows the message and nothing else by default — correct, since a merge's diff depends on
/// which parent you ask about, and unhelpful in a window that was opened to see what happened.
pub fn show(root: &Path, hash: &str) -> Result<Vec<String>, String> {
    // `--patch` is spelled out, and it is not redundant. `git show` shows a patch by default —
    // but asking for `--stat` *replaces* it with the summary rather than adding one, so the
    // obvious pair of flags gives a reader with the file names in it and no diff under them.
    // Nothing says so: the box opens, it has content, and the patch is simply not there.
    git(root, &["show", "--no-color", "--stat", "--patch", "--cc", hash])
        .map(|text| text.lines().map(str::to_string).collect())
}

// ---- Writing, continued ----------------------------------------------------------------------

/// Replaces the last commit with one that also carries whatever is staged.
///
/// The message is given rather than opened in an editor, for the same reason [`commit`] gives
/// one: there is no terminal here to hand git for one.
pub fn amend(root: &Path, message: &str) -> Result<String, String> {
    write(root, &["commit", "--amend", "-m", message])
}

/// The message the last commit already has, so amending starts from it instead of from an empty
/// box. Retyping a message to add one staged file is how a commit loses the sentence that
/// explained it.
pub fn head_message(root: &Path) -> Option<String> {
    git(root, &["log", "-1", "--pretty=format:%s"]).ok().map(|s| s.trim().to_string())
}

/// Makes a branch and moves onto it. `at` is where it starts — a commit picked out of the graph,
/// or wherever HEAD is when it is `None`.
pub fn create_branch(root: &Path, name: &str, at: Option<&str>) -> Result<String, String> {
    let mut args = vec!["checkout", "-b", name];
    if let Some(at) = at {
        args.push(at);
    }
    write(root, &args)
}

/// Deletes a branch, including one whose work is on no other branch.
///
/// `-D` and not `-d`, deliberately: `-d` refuses a branch that is not merged, and a panel that
/// asked "delete this branch?" and then refused would be asking a question it had no intention
/// of honouring. The question is asked before this is called, and it says what is being lost.
pub fn delete_branch(root: &Path, name: &str) -> Result<String, String> {
    write(root, &["branch", "-D", name])
}

/// Merges another branch into the one that is checked out.
///
/// `--no-edit` because a merge that stopped to open an editor would hang with nothing to type
/// in. A merge that conflicts is not an error here: it comes back as git's own message, and the
/// Status tab then shows the conflicting files with `UU` in front of them.
pub fn merge(root: &Path, branch: &str) -> Result<String, String> {
    write(root, &["merge", "--no-edit", branch])
}

/// Copies one commit onto the current branch.
pub fn cherry_pick(root: &Path, hash: &str) -> Result<String, String> {
    write(root, &["cherry-pick", hash])
}

/// Makes a new commit that undoes an old one. Nothing is lost: the commit being reverted stays
/// exactly where it is, which is why this needs no question in front of it.
pub fn revert(root: &Path, hash: &str) -> Result<String, String> {
    write(root, &["revert", "--no-edit", hash])
}

/// Moves the current branch to a commit and makes the working tree match it.
///
/// The second destructive thing in this module, and unlike [`discard`] only half of it can be
/// undone: the commits left behind are in the reflog for a while, the uncommitted changes this
/// throws away are nowhere at all.
pub fn reset_hard(root: &Path, hash: &str) -> Result<String, String> {
    write(root, &["reset", "--hard", hash])
}

pub fn tag(root: &Path, name: &str, at: &str) -> Result<String, String> {
    write(root, &["tag", name, at])
}

/// Puts the working tree away and gives it a name.
///
/// `stash save` and not `stash push`, which is the same call this module makes everywhere else:
/// `push` arrived in git 2.13 (2017) and `save` works in every git before and since. This is the
/// command most likely to be wanted on the old git of a server reached over ssh, so it is the
/// one place a deprecated spelling is worth more than a current one.
pub fn stash_push(root: &Path, message: &str) -> Result<String, String> {
    if message.is_empty() {
        write(root, &["stash", "save"])
    } else {
        write(root, &["stash", "save", message])
    }
}

/// Puts a stash back and forgets it.
pub fn stash_pop(root: &Path, name: &str) -> Result<String, String> {
    write(root, &["stash", "pop", name])
}

/// Puts a stash back and keeps it, which is the one to reach for when it might not apply cleanly.
pub fn stash_apply(root: &Path, name: &str) -> Result<String, String> {
    write(root, &["stash", "apply", name])
}

pub fn stash_drop(root: &Path, name: &str) -> Result<String, String> {
    write(root, &["stash", "drop", name])
}

/// Puts back whatever a half-finished merge, pick, revert or rebase was in the middle of.
pub fn abort(root: &Path, what: Unfinished) -> Result<String, String> {
    write(root, &[what.abort(), "--abort"])
}

// ---- The three that talk to another machine ---------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Remote {
    Fetch,
    Pull,
    Push,
}

/// The command line for a remote operation, as a person would type it — because that is what
/// happens to it: it is typed into one of the shells rather than run behind the panel.
///
/// This is the whole answer to the thing that kept push and pull out of the panel for three
/// releases. They can stop and ask for a passphrase, a two-factor code, or a host key, and a
/// modal panel has nowhere to put that question — so a panel that ran them itself would hang
/// with the question on a pipe nobody can see. A terminal is exactly the thing that can ask it,
/// and there is one on the screen already.
///
/// `-u` on the first push of a branch with no upstream, because the alternative is git printing
/// the same suggestion and doing nothing.
pub fn remote_command(op: Remote, branch: Option<&str>, has_upstream: bool) -> String {
    match op {
        // `--prune`, so a branch deleted on the other side stops being drawn here. Without it a
        // graph accumulates remote branches that no longer exist, and there is no hint which.
        Remote::Fetch => "git fetch --all --prune".to_string(),
        Remote::Pull => "git pull".to_string(),
        Remote::Push => match (has_upstream, branch) {
            (false, Some(branch)) => format!("git push -u origin {branch}"),
            _ => "git push".to_string(),
        },
    }
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
    fn a_commit_line_splits_into_its_six_parts() {
        let line = format!(
            "a1b2c3d{SEP}9f8e7d6 1122334{SEP}HEAD -> main{SEP}Ada Lovelace{SEP}3 days ago{SEP}Teach the engine to count"
        );
        let c = parse_graph_commit(&line, &[]).expect("a full line parses");
        assert_eq!(c.hash, "a1b2c3d");
        assert_eq!(c.parents, vec!["9f8e7d6", "1122334"]);
        assert_eq!(c.author, "Ada Lovelace");
        assert_eq!(c.when, "3 days ago");
        assert_eq!(c.subject, "Teach the engine to count");

        // An empty subject is a real commit, not a parse failure. So is a root, which has no
        // parents at all — read as a failure it would take the bottom off every graph.
        let line = format!("a1b2c3d{SEP}{SEP}{SEP}A{SEP}now{SEP}");
        let c = parse_graph_commit(&line, &[]).expect("still a commit");
        assert_eq!(c.subject, "");
        assert!(c.parents.is_empty());
        // A truncated line is not a commit, and must not become one with empty fields.
        assert!(parse_graph_commit("a1b2c3d", &[]).is_none());
        assert!(parse_graph_commit("", &[]).is_none());
    }

    /// What `%D` writes, taken apart. The remote is told from a local branch by asking `git
    /// remote`, not by looking for a slash: `feature/login` has one and is not a remote branch.
    #[test]
    fn a_decoration_is_read_with_the_remotes_in_hand() {
        let remotes = vec!["origin".to_string(), "upstream".to_string()];
        let refs = parse_refs("HEAD -> main, origin/main, tag: v0.1, feature/login, upstream/main", &remotes);
        let kinds: Vec<_> = refs.iter().map(|r| (r.kind, r.text.as_str())).collect();
        assert_eq!(
            kinds,
            vec![
                (RefKind::Head, "main"),
                (RefKind::Remote, "origin/main"),
                (RefKind::Tag, "v0.1"),
                (RefKind::Local, "feature/login"),
                (RefKind::Remote, "upstream/main"),
            ]
        );

        // With no remotes known, a slashed name is a local branch — which is what it is.
        let refs = parse_refs("origin/main", &[]);
        assert_eq!(refs[0].kind, RefKind::Local);

        // A detached HEAD is named rather than dropped: it is the state where knowing where you
        // are is hardest.
        assert_eq!(parse_refs("HEAD", &remotes)[0].kind, RefKind::Head);
        // `origin/HEAD` sits on the same commit as the remote's default branch for the life of
        // most repositories, and drawing it doubles the label to no purpose.
        assert!(parse_refs("origin/HEAD", &remotes).is_empty());
        assert!(parse_refs("", &remotes).is_empty());
    }

    #[test]
    fn a_stash_keeps_gits_own_name_for_it() {
        let line = format!("stash@{{0}}{SEP}WIP on main: a1b2c3d Teach the engine");
        let stash = parse_stash(&line).expect("parses");
        assert_eq!(stash.name, "stash@{0}");
        assert_eq!(stash.subject, "WIP on main: a1b2c3d Teach the engine");
        assert!(parse_stash("no separator here").is_none());
    }

    /// The first push of a branch nobody has pushed carries `-u`, because the alternative is git
    /// printing that exact suggestion and doing nothing.
    #[test]
    fn a_first_push_sets_the_upstream_rather_than_advising_it() {
        assert_eq!(remote_command(Remote::Push, Some("spike"), false), "git push -u origin spike");
        assert_eq!(remote_command(Remote::Push, Some("main"), true), "git push");
        // With no branch — a detached HEAD — there is no name to set an upstream to, and git's
        // own refusal is a better message than a guess.
        assert_eq!(remote_command(Remote::Push, None, false), "git push");
        assert_eq!(remote_command(Remote::Fetch, Some("main"), true), "git fetch --all --prune");
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
            assert!(snap.diff.is_empty() && snap.graph.is_empty() && snap.branches.is_empty());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
