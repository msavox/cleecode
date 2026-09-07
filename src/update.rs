//! The update check: "a new version is out", said once, quietly, and only ever acted on when
//! asked.
//!
//! CleeCode does not phone home. What runs here asks GitHub — which is already the channel
//! every release ships through — what the latest release is, and that is the whole
//! conversation: nothing of the user's is sent, nothing is downloaded, nothing is installed
//! *unless they say so*, every failure is silence, and one setting turns it all off. The
//! notice itself is a status line, not a dialog — except where the install method gives us a
//! command safe enough to offer running (brew, scoop: non-interactive, no sudo), in which case
//! it is the smallest question the app knows how to ask, declined by any key but the one that
//! accepts. Consent is per-occasion and never remembered as a default.
//!
//! The mechanics keep the same head-down posture. The ask happens on a background thread that
//! starts seconds after the shells do and cannot outrank them; it runs `curl` as a subprocess
//! rather than pulling an HTTP stack into the binary; it asks at most once a day, remembering
//! when in a small state file beside settings.toml; and it wraps itself in `catch_unwind`,
//! because a version check that could take the editor down would have its priorities exactly
//! backwards. A build running out of someone's `target/` directory is never notified at all:
//! whoever builds from source is ahead of the releases, not behind them.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::mpsc::Sender;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The one thing asked of the network, and the address it is asked at.
const LATEST_URL: &str = "https://api.github.com/repos/msavox/cleecode/releases/latest";

/// How long after startup the first (and only) ask of a session waits. The shells, the LSP and
/// the first frame all outrank news about a version that will still be new in ten seconds.
const STARTUP_DELAY: Duration = Duration::from_secs(6);

/// How long an answer is considered fresh: one day. The throttle is on *asking*, not on
/// succeeding — a failing network retried every launch would be the polling this module
/// promises not to do.
const CHECK_EVERY: u64 = 24 * 60 * 60;

/// How the running binary got onto this machine, read from its own path. It decides both the
/// wording of the notice and whether an upgrade can be offered at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InstallMethod {
    /// Under a `Cellar` directory: Homebrew's, on macOS or Linux.
    Brew,
    /// Under a `scoop` directory: the Windows bucket.
    Scoop,
    /// Under a `target` directory: `cargo build`, someone working on the source itself.
    Source,
    /// A tarball, a .deb, or anywhere else — real installs whose upgrade path we cannot name
    /// as one safe command, so they get the notice and the site.
    Other,
}

/// Reads the install method off the executable's path. A path test and nothing more, so the
/// tests need no filesystem: `Cellar` and `target` are matched as whole components (a project
/// named `mytarget` is not a cargo build), `scoop` case-insensitively anywhere (Scoop's own
/// layout puts it in `scoop\shims` or `scoop\apps`, but users relocate it).
pub fn install_method(exe: &Path) -> InstallMethod {
    let component_is = |name: &str| {
        exe.components().any(|c| c.as_os_str().to_string_lossy().eq_ignore_ascii_case(name))
    };
    if component_is("cellar") {
        return InstallMethod::Brew;
    }
    if exe.to_string_lossy().to_ascii_lowercase().contains("scoop") {
        return InstallMethod::Scoop;
    }
    if component_is("target") {
        return InstallMethod::Source;
    }
    InstallMethod::Other
}

/// The upgrade command an install method makes safe to run for the user: non-interactive, no
/// sudo, no questions the subprocess could sit waiting on. `None` is the decision *not* to
/// offer — tarball and .deb installs involve paths and privileges we will not guess at, and a
/// source build has nothing to upgrade to.
pub fn upgrade_command(method: InstallMethod) -> Option<&'static [&'static str]> {
    match method {
        InstallMethod::Brew => Some(&["brew", "upgrade", "clee"]),
        InstallMethod::Scoop => Some(&["scoop", "update", "clee"]),
        InstallMethod::Source | InstallMethod::Other => None,
    }
}

/// The same command as one line for a human to run, which is what the notice prints when the
/// offer is declined or when running it ourselves failed.
pub fn upgrade_command_line(method: InstallMethod) -> Option<String> {
    upgrade_command(method).map(|argv| argv.join(" "))
}

/// `v0.25.0` (or `0.25.0`) as three numbers, or `None` for anything else. Numeric fields and
/// exactly three of them: a tag this repo never made is not worth guessing about, and a parse
/// failure anywhere downstream means "say nothing", never "say something wrong".
pub fn parse_tag(tag: &str) -> Option<(u64, u64, u64)> {
    let bare = tag.trim().strip_prefix('v').unwrap_or(tag.trim());
    let mut parts = bare.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Whether `latest` is news to a binary that is `current` — a field-by-field comparison, since
/// the string form would call 0.9.2 newer than 0.24.1.
pub fn is_newer(latest: (u64, u64, u64), current: (u64, u64, u64)) -> bool {
    latest > current
}

/// The whole decision, pure so the tests can hold it still: does this tag, on this install, at
/// this moment in the notification history, earn a notice? Malformed tags and versions say no.
/// A source build says no whatever the numbers say. A version already notified says no — that
/// is the "once per version" half of silencing, the setting being the other half.
pub fn should_notify(
    latest_tag: &str,
    current_version: &str,
    method: InstallMethod,
    already_notified: &str,
) -> bool {
    if method == InstallMethod::Source {
        return false;
    }
    let (Some(latest), Some(current)) = (parse_tag(latest_tag), parse_tag(current_version)) else {
        return false;
    };
    if !is_newer(latest, current) {
        return false;
    }
    parse_tag(already_notified) != Some(latest)
}

/// What the check remembers between sessions, in `update.toml` beside settings.toml: when it
/// last asked, which version it has already mentioned, and the tail of the last upgrade's
/// output — kept for someone debugging a failed upgrade, never shown unasked.
#[derive(Default, Serialize, Deserialize)]
pub struct UpdateState {
    #[serde(default)]
    pub last_check: u64,
    #[serde(default)]
    pub notified: String,
    #[serde(default)]
    pub last_output: String,
}

fn state_path() -> Option<std::path::PathBuf> {
    crate::settings::config_dir().map(|d| d.join("update.toml"))
}

/// A missing or unreadable state file is a default one: the check has simply never run here.
pub fn load_state() -> UpdateState {
    state_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

/// Best-effort, like everything here: a state that cannot be written costs at worst one extra
/// ask or one repeated notice, neither of which is worth a visible error.
pub fn save_state(state: &UpdateState) {
    let Some(path) = state_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = toml::to_string(state) {
        let _ = std::fs::write(path, text);
    }
}

/// The kill switch the drivers and CI hold: with `CLEE_UPDATE_CHECK=0` in the environment,
/// nothing in this module touches the network, the disk, or the status line. Exported by
/// `pty_drive.py` for every driven session, so the tests stay offline and deterministic.
pub fn disabled_by_env() -> bool {
    std::env::var("CLEE_UPDATE_CHECK").is_ok_and(|v| v == "0")
}

/// What the background threads report back, drained once a frame like every other channel.
pub enum UpdateEvent {
    /// A newer release exists. The method rides along so the handler can decide between the
    /// plain notice and the offer without asking the filesystem again.
    Available { version: String, method: InstallMethod },
    /// The upgrade the user accepted has finished, well or badly.
    UpgradeFinished { ok: bool, version: String, method: InstallMethod },
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Pulls `"tag_name": "vX.Y.Z"` out of the release JSON. serde_json rather than a scrape, and
/// only this one field: everything else in that answer is nobody's business here.
fn tag_from_json(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    value.get("tag_name")?.as_str().map(str::to_string)
}

/// Starts the once-a-session check. Everything after the spawn happens on the thread — the
/// startup delay included — and the thread's whole body sits under `catch_unwind`: the panic
/// shield keeps a pane's crash from taking the app, and a *version check* has even less claim
/// to that power than a pane.
pub fn spawn_check(tx: Sender<UpdateEvent>, update_check_setting: bool) {
    if !update_check_setting || disabled_by_env() {
        return;
    }
    std::thread::spawn(move || {
        let _ = std::panic::catch_unwind(move || {
            std::thread::sleep(STARTUP_DELAY);
            let mut state = load_state();
            let now = now_secs();
            if now.saturating_sub(state.last_check) < CHECK_EVERY {
                return;
            }
            // Recorded before the network answers: the throttle is on asking, so a hanging or
            // failing GitHub is not retried at every launch until it behaves.
            state.last_check = now;
            save_state(&state);
            let output = std::process::Command::new("curl")
                .args(["-fsSL", "--max-time", "8", LATEST_URL])
                .stdin(std::process::Stdio::null())
                .output();
            let Ok(output) = output else { return };
            if !output.status.success() {
                return;
            }
            let Some(tag) = tag_from_json(&String::from_utf8_lossy(&output.stdout)) else {
                return;
            };
            let method = std::env::current_exe()
                .map(|exe| install_method(&exe))
                .unwrap_or(InstallMethod::Other);
            if !should_notify(&tag, env!("CARGO_PKG_VERSION"), method, &state.notified) {
                return;
            }
            let version = tag.trim().strip_prefix('v').unwrap_or(&tag).to_string();
            // Marked before the send, not after a keypress: "notified once per version" is a
            // promise about the notice appearing, and the notice is on its way.
            state.notified = tag.clone();
            save_state(&state);
            let _ = tx.send(UpdateEvent::Available { version, method });
        });
    });
}

/// Runs the upgrade the user just accepted, detached on its own thread. One attempt per
/// session, enforced by the caller; no retries, no progress bar, and the running process is
/// never touched — the package manager swaps the binary on disk while the live process keeps
/// its old inode, which is exactly why "starts next launch" is the honest success message.
/// The subprocess's output goes into the state file for whoever needs to debug a failure,
/// never onto the screen.
pub fn spawn_upgrade(method: InstallMethod, version: String, tx: Sender<UpdateEvent>) {
    if disabled_by_env() {
        return;
    }
    let Some(argv) = upgrade_command(method) else { return };
    std::thread::spawn(move || {
        let _ = std::panic::catch_unwind(move || {
            // Scoop is a .cmd, which CreateProcess will not run bare; cmd /C is the Windows
            // spelling of "run this the way a prompt would".
            let output = if cfg!(windows) {
                let mut cmd = std::process::Command::new("cmd");
                cmd.arg("/C").args(argv);
                cmd
            } else {
                let mut cmd = std::process::Command::new(argv[0]);
                cmd.args(&argv[1..]);
                cmd
            }
            .stdin(std::process::Stdio::null())
            .output();
            let ok = output.as_ref().is_ok_and(|o| o.status.success());
            if let Ok(o) = &output {
                let mut state = load_state();
                let text = format!(
                    "{}\n{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                );
                // The tail, not the whole transcript: this is a note for debugging one failed
                // run, and brew's full download log would bury the line that matters.
                state.last_output = text.chars().rev().take(2000).collect::<String>()
                    .chars().rev().collect();
                save_state(&state);
            }
            let _ = tx.send(UpdateEvent::UpgradeFinished { ok, version, method });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Field-by-field, not string order: 0.9.2 came before 0.24.1 and must say so.
    #[test]
    fn version_comparison_is_numeric_and_malformed_tags_never_notify() {
        assert!(should_notify("v0.25.0", "0.24.2", InstallMethod::Brew, ""));
        assert!(should_notify("0.25.0", "0.24.2", InstallMethod::Other, ""), "a bare tag counts");
        assert!(!should_notify("v0.24.2", "0.24.2", InstallMethod::Brew, ""), "equal is not news");
        assert!(!should_notify("v0.9.2", "0.24.1", InstallMethod::Brew, ""), "older is not news");
        assert!(should_notify("v1.0.0", "0.99.99", InstallMethod::Brew, ""));
        for bad in ["", "v", "latest", "v1.2", "v1.2.3.4", "va.b.c", "v1.2.x"] {
            assert!(!should_notify(bad, "0.24.2", InstallMethod::Brew, ""), "{bad:?} notified");
        }
        assert!(!should_notify("v0.25.0", "not-a-version", InstallMethod::Brew, ""));
    }

    /// The install method is read off the path alone, so these need no filesystem. `target`
    /// and `Cellar` are whole components; scoop is a substring because its own layout varies.
    #[test]
    fn the_install_method_is_read_off_the_executable_path() {
        let of = |p: &str| install_method(&PathBuf::from(p));
        assert_eq!(of("/opt/homebrew/Cellar/clee/0.24.2/bin/clee"), InstallMethod::Brew);
        assert_eq!(of("/home/linuxbrew/.linuxbrew/Cellar/clee/0.24.2/bin/clee"), InstallMethod::Brew);
        assert_eq!(of(r"C:\Users\x\scoop\shims\clee.exe"), InstallMethod::Scoop);
        assert_eq!(of(r"C:\Users\x\Scoop\apps\clee\current\clee.exe"), InstallMethod::Scoop);
        assert_eq!(of("/Users/x/GitHub/cleecode/target/release/clee"), InstallMethod::Source);
        assert_eq!(of("/Users/x/GitHub/cleecode/target/debug/clee"), InstallMethod::Source);
        assert_eq!(of("/usr/local/bin/clee"), InstallMethod::Other);
        assert_eq!(of("/home/x/mytarget/clee"), InstallMethod::Other, "component, not substring");
    }

    /// A source build is never notified, whatever the numbers say: whoever builds from master
    /// is ahead of the releases, not behind them.
    #[test]
    fn a_source_build_is_never_notified() {
        assert!(!should_notify("v9.9.9", "0.1.0", InstallMethod::Source, ""));
    }

    /// Once per version: the remembered tag silences its own version and nothing newer.
    #[test]
    fn a_version_is_notified_once_and_a_newer_one_still_gets_through() {
        assert!(!should_notify("v0.25.0", "0.24.2", InstallMethod::Brew, "v0.25.0"));
        assert!(!should_notify("0.25.0", "0.24.2", InstallMethod::Brew, "v0.25.0"), "same version, either spelling");
        assert!(should_notify("v0.26.0", "0.24.2", InstallMethod::Brew, "v0.25.0"));
    }

    /// Which installs get the offer, not just the notice: exactly the ones whose upgrade is one
    /// non-interactive command without sudo. The others get told, never acted for.
    #[test]
    fn only_brew_and_scoop_offer_to_run_the_upgrade() {
        assert_eq!(upgrade_command(InstallMethod::Brew), Some(&["brew", "upgrade", "clee"][..]));
        assert_eq!(upgrade_command(InstallMethod::Scoop), Some(&["scoop", "update", "clee"][..]));
        assert_eq!(upgrade_command(InstallMethod::Source), None);
        assert_eq!(upgrade_command(InstallMethod::Other), None);
    }

    /// The one field pulled out of GitHub's answer, and silence for anything malformed.
    #[test]
    fn the_tag_is_read_from_the_release_json_and_garbage_is_silence() {
        assert_eq!(tag_from_json(r#"{"tag_name":"v0.25.0","name":"v0.25.0"}"#).as_deref(), Some("v0.25.0"));
        assert_eq!(tag_from_json("not json"), None);
        assert_eq!(tag_from_json(r#"{"message":"Not Found"}"#), None);
    }
}
