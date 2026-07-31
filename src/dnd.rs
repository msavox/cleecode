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

/// Best-effort detection of an `ssh` session running as a direct child of the given shell
/// pid, returning the destination argument it was invoked with (works with ~/.ssh/config
/// host aliases too, since we hand that same token to `scp` verbatim). Relies on `ps`
/// being available and readable for our own user; never guaranteed to succeed.
pub fn detect_ssh_target(shell_pid: u32) -> Option<String> {
    let output = std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid=,command="])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(3, |c: char| c.is_whitespace());
        let pid = parts.next().and_then(|s| s.parse::<u32>().ok());
        let ppid = parts.next().and_then(|s| s.parse::<u32>().ok());
        let cmd = parts.next();
        let (Some(_pid), Some(ppid), Some(cmd)) = (pid, ppid, cmd) else {
            continue;
        };
        if ppid != shell_pid {
            continue;
        }
        if let Some(target) = parse_ssh_command(cmd.trim()) {
            return Some(target);
        }
    }
    None
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
