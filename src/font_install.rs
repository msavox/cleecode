/// JetBrainsMono Nerd Font Mono (SIL OFL 1.1, see assets/fonts/OFL.txt), the "Mono"
/// patched variant so its icon glyphs are forced to single-cell width — the file tree's
/// per-file-type icons (see ui.rs's file_icon) are drawn from this font's Private Use
/// Area codepoints and need it (or another Nerd Font) to render as icons rather than
/// tofu boxes.
const FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/CleeCodeMonoNerdFont-Regular.ttf");
const FONT_FILENAME: &str = "CleeCodeMonoNerdFont-Regular.ttf";
const FONT_FAMILY: &str = "JetBrainsMono Nerd Font Mono";

/// Installs the bundled font into the user's font directory. Run via
/// `cleecode --install-font`; never runs implicitly, since touching the user's fonts (and,
/// on Unix, their terminal config) is the kind of thing that should be an explicit,
/// visible action rather than a side effect of a normal launch.
pub fn install() {
    println!("Installing bundled Nerd Font ({FONT_FAMILY})...");
    #[cfg(unix)]
    install_unix();
    #[cfg(windows)]
    install_windows();
    #[cfg(not(any(unix, windows)))]
    eprintln!("Font installation is not supported on this platform.");
}

/// macOS/Linux install: drop the .ttf into the per-user font directory (`~/Library/Fonts`
/// on macOS, `~/.local/share/fonts` on Linux), refresh the font cache on Linux, and point
/// Ghostty's config at the family if a config is present.
#[cfg(unix)]
fn install_unix() {
    let Some(fonts_dir) = dirs::font_dir() else {
        eprintln!("Could not determine the user font directory.");
        return;
    };
    if let Err(e) = std::fs::create_dir_all(&fonts_dir) {
        eprintln!("Could not create {}: {e}", fonts_dir.display());
        return;
    }
    let dest = fonts_dir.join(FONT_FILENAME);
    if let Err(e) = std::fs::write(&dest, FONT_BYTES) {
        eprintln!("Could not write {}: {e}", dest.display());
        return;
    }
    println!("Font installed: {}", dest.display());

    // On Linux, applications only see a newly dropped font after the fontconfig cache is
    // rebuilt; best-effort, ignored if fc-cache isn't installed.
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("fc-cache").arg("-f").arg(&fonts_dir).status();
    }

    if let Some(home) = dirs::home_dir() {
        let ghostty_config = home.join(".config").join("ghostty").join("config");
        match update_ghostty_config(&ghostty_config) {
            Ok(true) => println!("Ghostty config updated to use it: {}", ghostty_config.display()),
            Ok(false) => println!("Ghostty config already points at this font."),
            Err(e) => eprintln!("Could not update Ghostty config ({}): {e}", ghostty_config.display()),
        }
        println!("Restart Ghostty (or run `ghostty +reload-config` if supported) to pick it up.");
    }
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
fn install_windows() {
    let Some(local) = dirs::data_local_dir() else {
        eprintln!("Could not determine %LOCALAPPDATA%.");
        return;
    };
    let fonts_dir = local.join("Microsoft").join("Windows").join("Fonts");
    if let Err(e) = std::fs::create_dir_all(&fonts_dir) {
        eprintln!("Could not create {}: {e}", fonts_dir.display());
        return;
    }
    let dest = fonts_dir.join(FONT_FILENAME);
    if let Err(e) = std::fs::write(&dest, FONT_BYTES) {
        eprintln!("Could not write {}: {e}", dest.display());
        return;
    }
    println!("Font installed: {}", dest.display());

    match register_font_windows(&dest) {
        Ok(()) => println!(
            "Font registered for your user and loaded into the current session. Restart your \
             terminal app if it doesn't show up immediately."
        ),
        Err(e) => eprintln!(
            "Font copied but registration failed ({e}). Right-click {} and choose \"Install\".",
            dest.display()
        ),
    }
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
