use std::path::{Path, PathBuf};

/// JetBrainsMono Nerd Font Mono (SIL OFL 1.1, see assets/fonts/OFL.txt), the "Mono"
/// patched variant so its icon glyphs are forced to single-cell width — the file tree's
/// per-file-type icons (see ui.rs's file_icon) are drawn from this font's Private Use
/// Area codepoints and need it (or another Nerd Font) to render as icons rather than
/// tofu boxes.
const FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/CleeCodeMonoNerdFont-Regular.ttf");
const FONT_FILENAME: &str = "CleeCodeMonoNerdFont-Regular.ttf";
const FONT_FAMILY: &str = "JetBrainsMono Nerd Font Mono";

/// Installs the bundled font into the user's font directory and, best-effort, points
/// Ghostty's config at it. Run via `cleecode --install-font`; never runs implicitly, since
/// editing the user's terminal config is the kind of thing that should be an explicit,
/// visible action rather than a side effect of a normal launch.
pub fn install() {
    println!("Installing bundled Nerd Font ({FONT_FAMILY})...");
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        eprintln!("HOME is not set, cannot determine where to install the font.");
        return;
    };

    let fonts_dir = home.join("Library").join("Fonts");
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

    let ghostty_config = home.join(".config").join("ghostty").join("config");
    match update_ghostty_config(&ghostty_config) {
        Ok(true) => println!("Ghostty config updated to use it: {}", ghostty_config.display()),
        Ok(false) => println!("Ghostty config already points at this font."),
        Err(e) => eprintln!("Could not update Ghostty config ({}): {e}", ghostty_config.display()),
    }
    println!("Restart Ghostty (or run `ghostty +reload-config` if supported) to pick it up.");
}

/// Replaces any existing `font-family` line in Ghostty's config with one pointing at the
/// installed font, or appends one if none exists. Returns Ok(true) if the file was
/// changed, Ok(false) if it already matched.
fn update_ghostty_config(path: &Path) -> std::io::Result<bool> {
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
