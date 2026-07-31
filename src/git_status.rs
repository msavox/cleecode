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

/// Best-effort `git status` snapshot for `root`, keyed by absolute path. Returns an
/// empty map (no indicators shown) if `root` isn't inside a git repo or `git` isn't on
/// PATH; never treated as an error by callers, same convention as the ssh/scp helpers
/// in dnd.rs.
pub fn compute(root: &Path) -> HashMap<PathBuf, FileStatus> {
    let mut result = HashMap::new();
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
        let abs_path = root.join(&rel_path);
        insert_with_priority(&mut result, abs_path.clone(), status);

        let mut dir = abs_path.parent();
        while let Some(d) = dir {
            if !d.starts_with(root) || d == root {
                break;
            }
            insert_with_priority(&mut result, d.to_path_buf(), status);
            dir = d.parent();
        }
    }
    result
}
