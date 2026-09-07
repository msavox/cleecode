//! The Extras panel: the README's "Optional extras", one Enter away instead of one README
//! away. Each row is a thing CleeCode reaches for and works without — the bundled font, the
//! PDF rasteriser, pandoc and its PDF engine, the picture-to-cells fallback — shown with
//! whether this machine has it. A missing one, chosen, gets its install command typed at a
//! shell prompt **unsent**, the drawer launcher's rule word for word: the line is read before
//! it is run, and the Enter is the user's. Nothing here installs anything by itself.

use crate::tools::tool;

/// The rows, in the order the panel shows them: the font first because it is ours, then the
/// preview tools in the README's own order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Extra {
    /// The bundled Nerd Font — the one row whose action is not a command but the same install
    /// the first-launch offer runs.
    Font,
    /// poppler's pdftoppm (or Ghostscript: either satisfies the row, exactly as either
    /// satisfies the preview — see preview.rs's rasteriser list).
    Pdf,
    /// pandoc, which turns markdown into a real document.
    Pandoc,
    /// typst, the lightest of the PDF engines pandoc can be handed.
    Typst,
    /// chafa, a picture inside a terminal pane that cannot draw real pixels.
    Chafa,
}

impl Extra {
    pub fn all() -> [Extra; 5] {
        [Extra::Font, Extra::Pdf, Extra::Pandoc, Extra::Typst, Extra::Chafa]
    }

    /// The name the row leads with — the program's own, since that is the word the install
    /// command will contain and the word an error message would use.
    pub fn name(self) -> &'static str {
        match self {
            Extra::Font => "Nerd Font",
            Extra::Pdf => "poppler",
            Extra::Pandoc => "pandoc",
            Extra::Typst => "typst",
            Extra::Chafa => "chafa",
        }
    }

    /// Whether this machine has it — asked of the same functions the features themselves ask,
    /// so the panel can never claim a state the preview would contradict.
    pub fn installed(self) -> bool {
        match self {
            Extra::Font => crate::font_install::installed(),
            Extra::Pdf => tool("pdftoppm").is_some() || tool("gs").is_some(),
            Extra::Pandoc => tool("pandoc").is_some(),
            Extra::Typst => tool("typst").is_some(),
            Extra::Chafa => tool("chafa").is_some(),
        }
    }
}

/// The package manager whose spelling the install commands borrow. One question, asked of the
/// PATH like everything else: brew first wherever it exists (macOS and linuxbrew alike, and
/// it is the spelling the README teaches), apt as the Linux fallback, scoop on Windows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PackageManager {
    Brew,
    Apt,
    Scoop,
    /// No manager found — the row can still say the program's name, and does.
    None,
}

pub fn package_manager() -> PackageManager {
    if tool("brew").is_some() {
        return PackageManager::Brew;
    }
    if cfg!(target_os = "linux") && tool("apt").is_some() {
        return PackageManager::Apt;
    }
    if cfg!(windows) && tool("scoop").is_some() {
        return PackageManager::Scoop;
    }
    PackageManager::None
}

/// The install command for one extra under one package manager, or `None` where honesty
/// beats a guess. The map is small and lives here whole: brew carries all four; apt names
/// poppler by its Debian split (`poppler-utils` is the half with pdftoppm) and has no typst
/// at all; scoop's main bucket has the three it has. A `None` row still helps — the panel
/// answers with the program's name and lets the user find their own road — and the Font is
/// always `None` because its action is our own installer, not a package manager's.
pub fn install_command(extra: Extra, pm: PackageManager) -> Option<&'static str> {
    use Extra::*;
    use PackageManager as Pm;
    match (pm, extra) {
        (_, Font) => None,
        (Pm::Brew, Pdf) => Some("brew install poppler"),
        (Pm::Brew, Pandoc) => Some("brew install pandoc"),
        (Pm::Brew, Typst) => Some("brew install typst"),
        (Pm::Brew, Chafa) => Some("brew install chafa"),
        (Pm::Apt, Pdf) => Some("sudo apt install poppler-utils"),
        (Pm::Apt, Pandoc) => Some("sudo apt install pandoc"),
        (Pm::Apt, Typst) => None,
        (Pm::Apt, Chafa) => Some("sudo apt install chafa"),
        (Pm::Scoop, Pdf) => Some("scoop install poppler"),
        (Pm::Scoop, Pandoc) => Some("scoop install pandoc"),
        (Pm::Scoop, Typst) => Some("scoop install typst"),
        (Pm::Scoop, Chafa) => None,
        (Pm::None, _) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The command map, held still: brew covers every tool row, the honest holes are where
    /// they were put on purpose (no typst in apt, no chafa in scoop, nothing at all without a
    /// manager), and the font never gets a command because its action is our own installer.
    #[test]
    fn the_install_commands_cover_what_they_claim_and_no_more() {
        for extra in Extra::all() {
            assert_eq!(install_command(extra, PackageManager::None), None);
            assert_eq!(install_command(Extra::Font, PackageManager::Brew), None);
            if extra != Extra::Font {
                let cmd = install_command(extra, PackageManager::Brew)
                    .expect("brew carries every tool row");
                assert!(cmd.starts_with("brew install "), "{cmd}");
            }
        }
        assert_eq!(install_command(Extra::Pdf, PackageManager::Apt), Some("sudo apt install poppler-utils"));
        assert_eq!(install_command(Extra::Typst, PackageManager::Apt), None, "apt has no typst; say the name");
        assert_eq!(install_command(Extra::Chafa, PackageManager::Scoop), None, "scoop's main bucket has no chafa");
    }
}
