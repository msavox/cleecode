//! JetBrainsMono Nerd Font Mono (SIL OFL 1.1, see assets/fonts/OFL.txt), the "Mono"
//! patched variant so its icon glyphs are forced to single-cell width — the file tree's
//! per-file-type icons (see ui.rs's file_icon) are drawn from this font's Private Use
//! Area codepoints and need it (or another Nerd Font) to render as icons rather than
//! tofu boxes.
//!
//! Two doors into the same work. `clee --install-font` narrates on stdout, for a person at a
//! shell. The in-app offer — first launch, file not on disk — goes through `install_quietly`,
//! because a TUI is drawing over stdout and a println from a background thread would land in
//! the middle of somebody's editor. Either way the install itself only ever runs because a
//! person said yes to something that named it: the CLI flag, the offer's Enter, or the
//! Extras panel's row. Touching the user's fonts as a side effect of a normal launch is the
//! line this module does not cross.

use std::path::PathBuf;
use std::sync::mpsc::Sender;

const FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/CleeCodeMonoNerdFont-Regular.ttf");
const FONT_FILENAME: &str = "CleeCodeMonoNerdFont-Regular.ttf";
const FONT_FAMILY: &str = "JetBrainsMono Nerd Font Mono";

/// What a successful install has to tell the person who asked for it: where the file went,
/// and whether Ghostty's config was pointed at it (only ever true on Unix, where that config
/// lives; the flag is what decides between "restart Ghostty" and "select it in your terminal"
/// in the status line).
pub struct Installed {
    pub dest: PathBuf,
    pub ghostty_updated: bool,
}

/// Where the bundled font lives once installed — the same answer the installer writes to, so
/// "is it installed" and "install it" can never disagree about the address. `dirs::font_dir()`
/// on Unix (`~/Library/Fonts`, `~/.local/share/fonts`); Windows has no font_dir in `dirs`, so
/// the per-user fonts directory is spelled out the way `install` always wrote it.
fn installed_font_path() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        dirs::font_dir().map(|d| d.join(FONT_FILENAME))
    }
    #[cfg(windows)]
    {
        dirs::data_local_dir()
            .map(|d| d.join("Microsoft").join("Windows").join("Fonts").join(FONT_FILENAME))
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

/// Whether the bundled font's file is where the installer puts it. This is all we can see:
/// a user on another Nerd Font has perfect icons and no such file, which is why the offer
/// built on this question is asked exactly once and a "no" is remembered forever.
pub fn installed() -> bool {
    installed_font_path().is_some_and(|p| p.is_file())
}

/// The whole first-launch decision, pure so the tests can hold it still. Over ssh the answer
/// is always no — the font would land on the server, and the rendering happens in the
/// terminal on the client's desk (the same discernment the Ko-fi button makes). Already
/// offered is no, whatever was answered then. Disabled is the drivers' and CI's door out.
pub fn should_offer(
    file_present: bool,
    over_ssh: bool,
    already_offered: bool,
    disabled: bool,
) -> bool {
    !file_present && !over_ssh && !already_offered && !disabled
}

/// The kill switch the drivers hold, a sibling of `CLEE_UPDATE_CHECK`: with
/// `CLEE_FONT_OFFER=0` no modal ever goes up, so a driven session's screen is deterministic.
pub fn offer_disabled_by_env() -> bool {
    std::env::var("CLEE_FONT_OFFER").is_ok_and(|v| v == "0")
}

/// Whether this process is at the far end of an ssh connection, in which case installing a
/// font *here* would put it on the wrong machine.
pub fn over_ssh() -> bool {
    std::env::var("SSH_CONNECTION").is_ok_and(|v| !v.is_empty())
}

/// What the in-app install reports back, drained once a frame like every other channel.
pub enum FontEvent {
    Done { ghostty_updated: bool },
    Failed(String),
}

/// Runs `install_quietly` on its own thread, under the same shield every background job in
/// this app wears: a font install that could take the editor down would have its priorities
/// exactly backwards.
///
/// Deliberately not behind `offer_disabled_by_env`: that switch silences the question nobody
/// asked for, never the answer somebody gave — by the time this runs, Enter was pressed on a
/// modal that named what it would do. An explicit act obeys the person, not the environment.
pub fn spawn_install(tx: Sender<FontEvent>) {
    std::thread::spawn(move || {
        let _ = std::panic::catch_unwind(move || {
            let event = match install_quietly() {
                Ok(done) => FontEvent::Done { ghostty_updated: done.ghostty_updated },
                Err(e) => FontEvent::Failed(e),
            };
            let _ = tx.send(event);
        });
    });
}

/// Installs the bundled font and answers instead of narrating — the shared core both doors
/// lead to. Errors come back as the sentence a person should read, because both callers can
/// only show a string: stdout for the CLI, the status line for the offer.
pub fn install_quietly() -> Result<Installed, String> {
    #[cfg(unix)]
    {
        install_unix()
    }
    #[cfg(windows)]
    {
        install_windows()
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err("Font installation is not supported on this platform.".to_string())
    }
}

/// Installs the bundled font into the user's font directory, narrating on stdout. Run via
/// `clee --install-font` — the talking wrapper around `install_quietly`, for a shell where
/// println is the medium rather than a bug.
pub fn install() {
    println!("Installing bundled Nerd Font ({FONT_FAMILY})...");
    match install_quietly() {
        Ok(done) => {
            println!("Font installed: {}", done.dest.display());
            #[cfg(unix)]
            match done.ghostty_updated {
                true => println!("Ghostty config updated to use it; restart Ghostty (or run `ghostty +reload-config` if supported) to pick it up."),
                false => println!("Select \"{FONT_FAMILY}\" in your terminal's settings to use it."),
            }
            #[cfg(windows)]
            println!(
                "Font registered for your user and loaded into the current session. Restart your \
                 terminal app if it doesn't show up immediately."
            );
        }
        Err(e) => eprintln!("{e}"),
    }
}

/// macOS/Linux install: drop the .ttf into the per-user font directory, refresh the font
/// cache on Linux, and point Ghostty's config at the family if a config is present.
#[cfg(unix)]
fn install_unix() -> Result<Installed, String> {
    let Some(dest) = installed_font_path() else {
        return Err("Could not determine the user font directory.".to_string());
    };
    let fonts_dir = dest.parent().expect("the font path always has a directory").to_path_buf();
    std::fs::create_dir_all(&fonts_dir)
        .map_err(|e| format!("Could not create {}: {e}", fonts_dir.display()))?;
    std::fs::write(&dest, FONT_BYTES)
        .map_err(|e| format!("Could not write {}: {e}", dest.display()))?;

    // On Linux, applications only see a newly dropped font after the fontconfig cache is
    // rebuilt; best-effort, ignored if fc-cache isn't installed.
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("fc-cache").arg("-f").arg(&fonts_dir).status();
    }

    // Ghostty is the one terminal whose config is a file with a stable key, which makes
    // pointing it at the font a favour rather than a guess. Failure here is not failure of
    // the install: the font is down, and the caller's message falls back to "select it".
    let ghostty_updated = dirs::home_dir().is_some_and(|home| {
        let config = home.join(".config").join("ghostty").join("config");
        update_ghostty_config(&config).unwrap_or(false)
    });
    Ok(Installed { dest, ghostty_updated })
}

/// Replaces any existing `font-family` line in Ghostty's config with one pointing at the
/// installed font, or appends one if none exists. Returns Ok(true) if the file was
/// changed, Ok(false) if it already matched.
#[cfg(unix)]
fn update_ghostty_config(path: &std::path::Path) -> std::io::Result<bool> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let wanted_line = format!("font-family = \"{FONT_FAMILY}\"");
    if existing.lines().any(|l| l.trim() == wanted_line) {
        return Ok(false);
    }

    let mut lines: Vec<&str> = existing.lines().filter(|l| !l.trim_start().starts_with("font-family")).collect();
    lines.push(&wanted_line);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, lines.join("\n") + "\n")?;
    Ok(true)
}

/// Windows install: copy the .ttf into the per-user font directory
/// (`%LOCALAPPDATA%\Microsoft\Windows\Fonts`), register it under HKCU so it survives a
/// reboot, and load it into the current GDI session so it's usable immediately without a
/// logout — no admin rights required.
#[cfg(windows)]
fn install_windows() -> Result<Installed, String> {
    let Some(dest) = installed_font_path() else {
        return Err("Could not determine %LOCALAPPDATA%.".to_string());
    };
    let fonts_dir = dest.parent().expect("the font path always has a directory").to_path_buf();
    std::fs::create_dir_all(&fonts_dir)
        .map_err(|e| format!("Could not create {}: {e}", fonts_dir.display()))?;
    std::fs::write(&dest, FONT_BYTES)
        .map_err(|e| format!("Could not write {}: {e}", dest.display()))?;

    register_font_windows(&dest).map_err(|e| {
        format!(
            "Font copied but registration failed ({e}). Right-click {} and choose \"Install\".",
            dest.display()
        )
    })?;
    Ok(Installed { dest, ghostty_updated: false })
}

/// Registers the font for the current user: persists the HKCU mapping so it's available on
/// every future login, and calls `AddFontResourceW` to load it into the running GDI session.
/// Both need only per-user rights — no admin/UAC prompt. (We deliberately avoid the
/// `WM_FONTCHANGE` broadcast, whose `SendMessageW` signature shifts between `windows-rs`
/// releases; already-running apps pick the font up on their next launch instead.)
#[cfg(windows)]
fn register_font_windows(dest: &std::path::Path) -> anyhow::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::AddFontResourceW;
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    // Persist the mapping so the font is registered on every future login. The full path is
    // required for per-user fonts (system fonts under the Fonts dir may use a bare filename).
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(r"Software\Microsoft\Windows NT\CurrentVersion\Fonts")?;
    key.set_value(&format!("{FONT_FAMILY} (TrueType)"), &dest.to_string_lossy().to_string())?;

    // Load it into the current GDI session so newly launched apps see it right away.
    let wide: Vec<u16> = dest.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let added = unsafe { AddFontResourceW(PCWSTR(wide.as_ptr())) };
    if added == 0 {
        anyhow::bail!("AddFontResourceW reported no fonts added");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every gate of the first-launch offer, held still: the file already there, an ssh
    /// session (wrong machine), a question already asked once (whatever it answered), and the
    /// drivers' kill switch each silence it on their own; only all four clear lets it speak.
    #[test]
    fn the_font_offer_speaks_once_and_only_where_it_makes_sense() {
        assert!(should_offer(false, false, false, false));
        assert!(!should_offer(true, false, false, false), "the file is already down");
        assert!(!should_offer(false, true, false, false), "ssh: the font would land on the server");
        assert!(!should_offer(false, false, true, false), "asked once is asked");
        assert!(!should_offer(false, false, false, true), "a driven session never sees it");
    }
}
