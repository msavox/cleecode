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
        let before = paths.len();
        for token in drop_tokens(line) {
            let path = PathBuf::from(&token);
            if path.exists() {
                paths.push(path);
            }
        }
        // Nothing in the line was a file, so try it whole. A path with spaces that arrived
        // unquoted — pasted from an address bar rather than dragged — is one file, not four
        // words, and only asking the disk can tell the two apart.
        if paths.len() == before {
            let whole = PathBuf::from(unquote(line));
            if whole.exists() {
                paths.push(whole);
            }
        }
    }
    paths
}

/// Splits a dropped line into candidate paths, the way the platform's own file manager writes
/// them.
///
/// Not `shell_words` on Windows: it is a POSIX splitter, where a backslash escapes the character
/// after it — so `C:\Users\me\notes.txt` came back as `C:Usersmenotes.txt`, which exists nowhere
/// and made every drop on Windows silently do nothing. There, quoting is double quotes and a
/// backslash is an ordinary character.
fn drop_tokens(line: &str) -> Vec<String> {
    if cfg!(windows) {
        quoted_tokens(line)
    } else {
        shell_words::split(line).unwrap_or_else(|_| vec![line.to_string()])
    }
}

/// Whitespace-separated, with double quotes holding a name with spaces together and nothing
/// else given any meaning.
fn quoted_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Strips one pair of matching quotes from around a whole line.
fn unquote(line: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = line.strip_prefix(quote).and_then(|l| l.strip_suffix(quote)) {
            return inner;
        }
    }
    line
}

/// The files a paste is offering to send somewhere, or nothing at all.
///
/// Both questions have to answer yes, and they are different questions. `parse_dropped_paths`
/// asks whether the tokens name files that are here; `looks_like_dropped_paths` asks whether the
/// paste is *made of* paths and nothing else. Only the first was ever asked, and a paste is not
/// only ever a drag: a line being composed at a shell with `~/.ssh/id_ed25519` pasted into the
/// middle of it names a file that exists, and sending it to the server was never what was meant.
/// A drag has no prose in it, so requiring the shape as well costs a real drop nothing.
pub fn upload_candidates(text: &str) -> Vec<PathBuf> {
    if !looks_like_dropped_paths(text) {
        return Vec::new();
    }
    parse_dropped_paths(text)
}

/// Whether a paste looks like a file drop whose files are somewhere else.
///
/// `parse_dropped_paths` keeps only what exists on *this* machine, so dragging a file onto a
/// CleeCode running over ssh yields nothing at all — the path is real, just not here. Silence is
/// the worst answer to that, so this recognises the shape of a dropped path without requiring it
/// to exist: absolute, one per line, and nothing that looks like prose.
pub fn looks_like_dropped_paths(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.is_empty() || lines.len() > 32 {
        return false;
    }
    // Read each line both ways rather than the platform's way. What is being described here is
    // the machine the files are on, and over ssh that is not this one: a line from Windows needs
    // the splitter that leaves backslashes alone, while a Mac path with an escaped space needs
    // the one that does not. Either reading being all paths is enough to call it a drop.
    lines.iter().all(|line| {
        let posix = shell_words::split(line).unwrap_or_else(|_| vec![line.to_string()]);
        all_rooted_paths(&posix) || all_rooted_paths(&quoted_tokens(line))
    })
}

/// Whether every token is a path rather than a word: rooted, and with a name at the end of it.
fn all_rooted_paths(tokens: &[String]) -> bool {
    !tokens.is_empty()
        && tokens.iter().all(|t| {
            (t.starts_with('/') || t.starts_with("~/") || t.starts_with("file://") || is_drive_path(t))
                && !t.ends_with('.')
                && t.len() > 2
        })
}

/// What a rooted path looks like when it comes from Windows: a drive letter, a colon, a
/// separator. Recognised whatever platform this is running on — the drop is being described by
/// the machine the files are on, which over ssh is not this one.
fn is_drive_path(text: &str) -> bool {
    let mut chars = text.chars();
    let drive = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    let colon = chars.next() == Some(':');
    let separator = matches!(chars.next(), Some('\\') | Some('/'));
    drive && colon && separator
}

/// Whether this process is itself running over ssh, which is what turns "those files are not
/// here" into the more useful "those files are on the machine you connected from".
pub fn running_over_ssh() -> bool {
    std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some()
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

/// Index of the first shell in `shell_pids` with an interpreter of `language` running inside it,
/// so a script can be handed to the session that is already open instead of starting a second
/// one. Takes a single process-table snapshot for all the shells, since this runs on every Run
/// and a refresh is not cheap. Returns `None` if the table can't be read.
pub fn shell_running(language: crate::session::Language, shell_pids: &[Option<u32>]) -> Option<usize> {
    let sys = process_snapshot();
    shell_pids
        .iter()
        .position(|pid| pid.is_some_and(|pid| has_interpreter_descendant(&sys, language, pid)))
}

/// Whether an interpreter of `language` is running under `shell_pid`. Searches descendants
/// rather than direct children only, because a launcher — `octave` is one — can sit between the
/// shell and the real interpreter.
fn has_interpreter_descendant(
    sys: &sysinfo::System,
    language: crate::session::Language,
    shell_pid: u32,
) -> bool {
    let mut frontier = vec![sysinfo::Pid::from_u32(shell_pid)];
    let mut visited = 0;
    // The tree under one shell is tiny; the bound only stops a pathological parent/child
    // cycle in the snapshot from spinning forever.
    while let Some(parent) = frontier.pop() {
        visited += 1;
        if visited > 64 {
            break;
        }
        for process in sys.processes().values() {
            if process.parent() != Some(parent) {
                continue;
            }
            if language.is_interpreter(&process.name().to_string_lossy()) {
                return true;
            }
            frontier.push(process.pid());
        }
    }
    false
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

    /// The opener is the platform's, and on Windows it is a shell builtin with an empty window
    /// title in front of the path — leave that out and the path becomes the title, so nothing
    /// opens and nothing says why.
    #[test]
    fn the_desktop_opener_is_the_one_this_platform_has() {
        let (program, before) = desktop_opener();
        assert!(!program.is_empty());
        if cfg!(target_os = "macos") {
            assert_eq!((program, before), ("open", &[] as &[&str]));
        } else if cfg!(windows) {
            assert_eq!(program, "cmd");
            assert_eq!(before, ["/C", "start", ""]);
        } else {
            assert_eq!(program, "xdg-open");
        }
    }

    /// Over ssh it refuses rather than running the opener on the far machine, where the desktop
    /// belongs to whoever is sitting at it — if there is one at all.
    #[test]
    fn over_ssh_there_is_nothing_to_hand_a_file_to() {
        if !running_over_ssh() {
            return;
        }
        assert_eq!(open_with_the_desktop(std::path::Path::new("/etc/hosts")), Err("over ssh".into()));
        assert_eq!(open_url("https://example.com"), Err("over ssh".into()));
    }

    /// Only http(s) may be handed to the opener, and the refusal happens before anything is
    /// spawned — so this is safe to run on a machine that is not ssh-ing anywhere.
    #[test]
    fn only_http_urls_are_handed_to_the_opener() {
        for url in ["file:///etc/passwd", "javascript:alert(1)", "ftp://example.com", "ssh://x", "mailto:a@b.c"] {
            assert_eq!(open_url(url), Err("not an http(s) URL".into()), "{url}");
        }
    }
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

    /// A Windows path is mostly backslashes, and a POSIX splitter eats every one of them as an
    /// escape: `C:\Users\me\notes.txt` came back as `C:Usersmenotes.txt`, so a drop on Windows
    /// found nothing and did nothing. Both splitters are tested from either platform, since the
    /// one that is wrong here is the one that runs there.
    #[test]
    fn a_windows_path_survives_being_split() {
        assert_eq!(quoted_tokens(r"C:\Users\me\notes.txt"), vec![r"C:\Users\me\notes.txt"]);
        // Double quotes hold a name with spaces together; nothing else means anything.
        assert_eq!(
            quoted_tokens(r#""C:\My Documents\a b.txt" C:\tmp\c.txt"#),
            vec![r"C:\My Documents\a b.txt", r"C:\tmp\c.txt"]
        );
        assert!(quoted_tokens("   ").is_empty());
        // What the POSIX splitter does to the same line, and why it is not used there.
        assert_eq!(shell_words::split(r"C:\Users\me\notes.txt").unwrap(), vec!["C:Usersmenotes.txt"]);
    }

    /// What may be offered for upload, and what may not. The paths exist in both halves of this
    /// — that is the point: existing on disk is what a path does, not what a drag does, and
    /// sending a private key to a server because its name was pasted into a half-typed command
    /// is the failure this rules out.
    #[test]
    fn only_a_paste_made_entirely_of_paths_is_a_drop() {
        let dir = std::env::temp_dir().join(format!("clee_dnd_upload_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = dir.join("id_ed25519");
        std::fs::write(&key, "x").unwrap();
        let shown = key.display().to_string();

        // Dragged: the paste is the path and nothing else, one file or several.
        assert_eq!(upload_candidates(&shown), vec![key.clone()]);
        assert_eq!(upload_candidates(&format!("{shown}\n{shown}")), vec![key.clone(), key.clone()]);

        // Typed: the same real file, named inside a sentence or a command being composed.
        assert!(upload_candidates(&format!("ssh-add {shown}")).is_empty());
        assert!(upload_candidates(&format!("the key is at {shown}, have a look")).is_empty());
        assert!(upload_candidates("git status").is_empty());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A path with spaces that arrived unquoted is one file, not several words. Only the disk
    /// can say, which is why this is a fallback rather than the rule.
    #[test]
    fn an_unquoted_path_with_spaces_is_still_one_file() {
        let dir = std::env::temp_dir().join(format!("clee_dnd_spaces_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("two words.txt");
        std::fs::write(&file, "x").unwrap();

        assert_eq!(parse_dropped_paths(&format!("{}\n", file.display())), vec![file.clone()]);
        // Quoted, it is found by the ordinary path through the splitter.
        assert_eq!(parse_dropped_paths(&format!("\"{}\"\n", file.display())), vec![file]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // Recognising an interpreter and quoting a path for it moved to `session.rs` when Python
    // arrived, and so did their tests: the question is the same for both languages, and one
    // answer that knows about both is what stops them drifting apart.

    #[test]
    fn parses_ssh_target() {
        assert_eq!(parse_ssh_command("ssh myserver"), Some("myserver".to_string()));
        assert_eq!(parse_ssh_command("ssh -p 2222 user@host.com"), Some("user@host.com".to_string()));
        assert_eq!(parse_ssh_command("/usr/bin/ssh host"), Some("host".to_string()));
        assert_eq!(parse_ssh_command("bash"), None);
    }


    /// A drop that cannot be honoured has to be recognised without the files being there, since
    /// not being there is the whole problem. It must not mistake ordinary pasted prose for one,
    /// or a stray paste in the file tree would start explaining ssh to somebody.
    #[test]
    fn a_drop_from_elsewhere_is_told_apart_from_prose() {
        assert!(looks_like_dropped_paths("/Users/someone/Desktop/photo.png"));
        assert!(looks_like_dropped_paths("'/Users/someone/my papers/report.pdf'"));
        assert!(looks_like_dropped_paths("/a/one.txt\n/a/two.txt"));
        assert!(looks_like_dropped_paths("~/Desktop/thing.md"));

        // A drop described by a Windows machine, which is what arrives over ssh from one.
        assert!(looks_like_dropped_paths(r"C:\Users\someone\Desktop\photo.png"));
        assert!(looks_like_dropped_paths(r#""C:\Users\someone\my papers\report.pdf""#));

        // Prose, a bare word, a relative path and an empty paste are all not drops.
        assert!(!looks_like_dropped_paths("the files are in /tmp, have a look"));
        assert!(!looks_like_dropped_paths("C: is the system drive"));
        assert!(!looks_like_dropped_paths("hello"));
        assert!(!looks_like_dropped_paths("src/main.rs"));
        assert!(!looks_like_dropped_paths(""));
        assert!(!looks_like_dropped_paths("   "));
    }
}


/// The command this desktop opens a file with, as a program and the arguments that go before the
/// path. Split out from the spawning so the choice can be tested on a machine that is not the
/// one it names.
///
/// Windows has no opener binary: `start` is a builtin of the shell, and its first quoted argument
/// is the *window title* — leave it out and a path in quotes becomes the title of a window that
/// never opens, which is the classic bug of this three-line function.
fn desktop_opener() -> (&'static str, &'static [&'static str]) {
    #[cfg(target_os = "macos")]
    {
        ("open", &[])
    }
    #[cfg(windows)]
    {
        ("cmd", &["/C", "start", ""])
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        ("xdg-open", &[])
    }
}

/// Hands a file to whatever the desktop opens that kind of file with, and returns once it has
/// been handed over — not once the other program is finished with it.
///
/// Refused over ssh, where there is a desktop at neither end that this could reach: the opener
/// would run on the far machine, against a display nobody is sitting at, and either fail slowly
/// or open a window for no one. Saying so is more useful than a viewer that never appears.
pub fn open_with_the_desktop(path: &std::path::Path) -> Result<(), String> {
    if running_over_ssh() {
        return Err("over ssh".to_string());
    }
    let (program, before) = desktop_opener();
    std::process::Command::new(program)
        .args(before)
        .arg(path)
        // Detached from CleeCode's own streams: an opener that printed to stdout would print
        // over the editor, and one that waited on stdin would wait for ever.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Hands a URL to the desktop's browser.
///
/// Only http(s) is a URL worth opening — anything else is a scheme the opener might hand to a
/// handler the user did not ask for, so it is refused here as well as at the call site. Same
/// guard as `open_with_the_desktop`: over ssh there is no desktop here to open it on.
pub fn open_url(url: &str) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("not an http(s) URL".to_string());
    }
    if running_over_ssh() {
        return Err("over ssh".to_string());
    }
    let (program, before) = desktop_opener();
    std::process::Command::new(program)
        .args(before)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}
