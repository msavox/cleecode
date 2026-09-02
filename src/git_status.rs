use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Coarse git status for a file tree row. Deliberately loses X/Y nuance (staged vs.
/// unstaged) in favor of a single color per row, matching the "traffic light" sidebar
/// indicator this drives.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
}

fn classify(x: char, y: char) -> FileStatus {
    if x == '?' && y == '?' {
        FileStatus::Untracked
    } else if x == 'D' || y == 'D' {
        FileStatus::Deleted
    } else if x == 'R' || x == 'C' {
        FileStatus::Renamed
    } else if x == 'A' {
        FileStatus::Added
    } else {
        FileStatus::Modified
    }
}

/// When a file and one of its ancestor directories both need a dot, the directory
/// shows whichever status is "worst" by this ranking.
fn priority(status: FileStatus) -> u8 {
    match status {
        FileStatus::Modified => 5,
        FileStatus::Deleted => 4,
        FileStatus::Renamed => 3,
        FileStatus::Added => 2,
        FileStatus::Untracked => 1,
    }
}

fn insert_with_priority(map: &mut HashMap<PathBuf, FileStatus>, path: PathBuf, status: FileStatus) {
    match map.get(&path) {
        Some(existing) if priority(*existing) >= priority(status) => {}
        _ => {
            map.insert(path, status);
        }
    }
}

/// The path a status line is filed under: the one the file tree spells it with.
///
/// This is the whole of a bug that made the sidebar's git dots invisible for anyone who opened a
/// project the ordinary way. `git status` reports paths relative to the top of the repository, so
/// they used to be filed as `toplevel/rel` — absolute. The tree builds its paths from the root it
/// was given, and running `clee` in a directory gives it `.`, so its rows are `./main.rs`. Every
/// lookup missed, every row drew no dot, and nothing anywhere said why: an editor whose git marks
/// are simply absent looks exactly like a repository with nothing changed in it.
///
/// It is the same shape as the `file:///./src/main.rs` bug the language server had in 0.8 — a
/// path that is correct, and correct in a spelling the other end does not use.
///
/// `None` for a file inside the repository but above the folder that was opened: the tree cannot
/// show it, so there is nothing for a dot to sit on.
fn key_for(root: &Path, base: &Path, absolute: &Path) -> Option<PathBuf> {
    let inside = absolute.strip_prefix(base).ok()?;
    Some(root.join(inside))
}

/// Best-effort `git status` snapshot, keyed the way the file tree spells its paths — see
/// [`key_for`]. Returns an empty map (no indicators shown) if `root` isn't inside a git repo or
/// `git` isn't on PATH; never treated as an error by callers, same convention as the ssh/scp
/// helpers in dnd.rs.
///
/// `root` only has to be *somewhere inside* the repo (e.g. after navigating the sidebar
/// into a subfolder) — `git status` always reports paths relative to the repo's actual
/// top-level, never relative to the cwd it was invoked from, so that top-level is looked
/// up separately and used for joining instead of `root`.
pub fn compute(root: &Path) -> HashMap<PathBuf, FileStatus> {
    let mut result = HashMap::new();
    let Ok(toplevel_output) =
        std::process::Command::new("git").args(["rev-parse", "--show-toplevel"]).current_dir(root).output()
    else {
        return result;
    };
    if !toplevel_output.status.success() {
        return result;
    }
    let toplevel = PathBuf::from(String::from_utf8_lossy(&toplevel_output.stdout).trim().to_string());

    let Ok(output) = std::process::Command::new("git")
        .args(["status", "--porcelain=v1", "-z"])
        .current_dir(root)
        .output()
    else {
        return result;
    };
    if !output.status.success() {
        return result;
    }

    // The tree's root, resolved, so a status line's absolute path can be turned back into the
    // spelling the tree uses. Asked of the disk once per sweep rather than per file.
    let base = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

    let mut fields = output.stdout.split(|&b| b == 0).filter(|f| !f.is_empty());
    while let Some(entry) = fields.next() {
        if entry.len() < 4 {
            continue;
        }
        let x = entry[0] as char;
        let y = entry[1] as char;
        let rel_path = String::from_utf8_lossy(&entry[3..]).trim_end_matches('/').to_string();
        if x == 'R' || x == 'C' {
            // Rename/copy entries carry a second NUL-separated field with the old path.
            fields.next();
        }
        let status = classify(x, y);
        let Some(path) = key_for(root, &base, &toplevel.join(&rel_path)) else { continue };
        insert_with_priority(&mut result, path.clone(), status);

        // A folder takes the mark of the worst thing under it, so a change is visible without
        // opening every level down to it. Stops at the root the tree is showing rather than at
        // the top of the repository: above that there is no row to draw on.
        let mut dir = path.parent();
        while let Some(d) = dir {
            if d == root || d.as_os_str().is_empty() {
                break;
            }
            insert_with_priority(&mut result, d.to_path_buf(), status);
            dir = d.parent();
        }
    }
    result
}

/// The paths whose status is new, or different, between two consecutive sweeps.
///
/// This is the whole of the detection behind follow mode, and it costs nothing: `compute` above
/// already runs every few hundred milliseconds to keep the sidebar's dots honest, so the
/// difference between what it said last time and what it says now *is* the list of files
/// something has just written. Nothing here knows or cares what did the writing — an agent in a
/// terminal pane, a formatter, a `sed`, a branch switched in another window all look the same
/// from here, which is the point.
///
/// What it cannot see: a file written twice in a row. Its status is `Modified` both times, so
/// the second write shows up as no change at all. That is not worth fixing here — by then the
/// file is open, and an open file is reloaded by the mtime check instead.
///
/// Paths only ever *appearing* is not the same as files: `compute` also files a status against
/// every parent directory, and a caller that means files has to say so. Sorted, so a caller
/// taking one per sweep takes them in a stable order rather than in the map's.
pub fn touched_between(
    before: &HashMap<PathBuf, FileStatus>,
    after: &HashMap<PathBuf, FileStatus>,
) -> Vec<PathBuf> {
    let mut touched: Vec<PathBuf> = after
        .iter()
        .filter(|(path, status)| before.get(*path) != Some(*status))
        .map(|(path, _)| path.clone())
        .collect();
    touched.sort();
    touched
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keys come out in the spelling the tree uses, which is the spelling of the root it was
    /// given — the whole point, since the tree's rows are what looks them up.
    #[test]
    fn a_status_line_is_filed_the_way_the_tree_spells_it() {
        let base = Path::new("/home/ada/project");

        // Opened as ".", which is what running `clee` in a directory does. This is the case that
        // was broken: the dot was filed under the absolute path and looked up under `./main.rs`.
        let key = key_for(Path::new("."), base, Path::new("/home/ada/project/src/main.rs"));
        assert_eq!(key, Some(PathBuf::from("./src/main.rs")));

        // Opened by name, which is what the tests and a `clee ~/project` do.
        let key = key_for(base, base, Path::new("/home/ada/project/src/main.rs"));
        assert_eq!(key, Some(PathBuf::from("/home/ada/project/src/main.rs")));

        // Changed somewhere in the repository but above the folder that was opened. There is no
        // row for it, so there is nothing to file.
        assert_eq!(key_for(Path::new("."), base, Path::new("/home/ada/other.rs")), None);
    }

    /// The delta follow mode is built on: what is new, what changed status, and — just as
    /// important — what did not move, since a file reported every sweep would reopen itself
    /// every 700 milliseconds.
    #[test]
    fn two_sweeps_name_the_files_that_moved_between_them() {
        let snapshot = |entries: &[(&str, FileStatus)]| -> HashMap<PathBuf, FileStatus> {
            entries.iter().map(|(p, s)| (PathBuf::from(*p), *s)).collect()
        };

        let before = snapshot(&[("./src/main.rs", FileStatus::Modified)]);
        let after = snapshot(&[
            ("./src/main.rs", FileStatus::Modified),
            ("./src/new.rs", FileStatus::Untracked),
            ("./README.md", FileStatus::Modified),
        ]);
        assert_eq!(
            touched_between(&before, &after),
            vec![PathBuf::from("./README.md"), PathBuf::from("./src/new.rs")],
            "a file whose status is the same as last time has not been touched again"
        );

        // A status that changed under the same path counts: staged after being modified, or a
        // new file that has since been added.
        let staged = snapshot(&[("./src/main.rs", FileStatus::Added)]);
        assert_eq!(touched_between(&before, &staged), vec![PathBuf::from("./src/main.rs")]);

        // Nothing moved, and nothing is reported — the answer on almost every sweep.
        assert!(touched_between(&before, &before).is_empty());

        // A file put back the way it was leaves the map; a disappearance is not something to
        // open, so it is not reported.
        assert!(touched_between(&before, &HashMap::new()).is_empty());
    }
}
