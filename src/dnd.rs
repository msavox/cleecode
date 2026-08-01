use std::path::PathBuf;

/// Best-effort parse of paths from a bracketed-paste payload. Terminal emulators turn a
/// Finder drag-and-drop into pasted text (there is no real "drop position" we can see),
/// space-separated or one per line, quoted/backslash-escaped like a shell would. Only
/// tokens that resolve to something that actually exists on disk are kept, so an
/// ordinary text paste never gets misinterpreted as a file drop.
pub fn parse_dropped_paths(text: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let tokens = shell_words::split(line).unwrap_or_else(|_| vec![line.to_string()]);
        for token in tokens {
            let path = PathBuf::from(&token);
            if path.exists() {
                paths.push(path);
            }
        }
    }
    paths
}

/// Snapshot of the process table with each process's command line and parent pid, taken
/// via `sysinfo` so it works identically on macOS, Linux and Windows (no reliance on a
/// `ps` binary or its output format).
fn process_snapshot() -> sysinfo::System {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing().with_cmd(sysinfo::UpdateKind::Always),
    );
    sys
}

/// Best-effort detection of an `ssh` session running as a direct child of the given shell
/// pid, returning the destination argument it was invoked with (works with ~/.ssh/config
/// host aliases too, since we hand that same token to `scp` verbatim). Enumerates the
/// process table with `sysinfo`; never guaranteed to succeed.
pub fn detect_ssh_target(shell_pid: u32) -> Option<String> {
    let sys = process_snapshot();
    let parent = sysinfo::Pid::from_u32(shell_pid);
    for process in sys.processes().values() {
        if process.parent() != Some(parent) {
            continue;
        }
        let cmd = process
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        if let Some(target) = parse_ssh_command(cmd.trim()) {
            return Some(target);
        }
    }
    None
}

/// Best-effort check for whether a shell has a direct child process running (i.e. is
/// busy at something other than its prompt). Used to pick an idle terminal for the
/// editor's Run button; enumerates the process table the same way `detect_ssh_target`
/// does, so it's never guaranteed to succeed and just falls back to "busy" on any failure.
pub fn shell_is_busy(shell_pid: u32) -> bool {
    let sys = process_snapshot();
    let parent = sysinfo::Pid::from_u32(shell_pid);
    sys.processes().values().any(|p| p.parent() == Some(parent))
}

fn parse_ssh_command(cmd: &str) -> Option<String> {
    let mut tokens = cmd.split_whitespace();
    let first = tokens.next()?;
    let prog = first.rsplit('/').next().unwrap_or(first);
    if prog != "ssh" {
        return None;
    }
    tokens.filter(|t| !t.starts_with('-')).last().map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_plain_text_paste() {
        let paths = parse_dropped_paths("hello world, this is not a path");
        assert!(paths.is_empty());
    }

    #[test]
    fn finds_existing_file() {
        let dir = std::env::temp_dir().join(format!("clicode_dnd_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("dropped.txt");
        std::fs::write(&file, "x").unwrap();
        let text = format!("{}\n", file.display());
        let paths = parse_dropped_paths(&text);
        assert_eq!(paths, vec![file]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parses_ssh_target() {
        assert_eq!(parse_ssh_command("ssh myserver"), Some("myserver".to_string()));
        assert_eq!(parse_ssh_command("ssh -p 2222 user@host.com"), Some("user@host.com".to_string()));
        assert_eq!(parse_ssh_command("/usr/bin/ssh host"), Some("host".to_string()));
        assert_eq!(parse_ssh_command("bash"), None);
    }
}
