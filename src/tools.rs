//! Finding the programs CleeCode does not ship: rasterisers, converters, coding agents.
//!
//! A module of its own because the question is not one feature's. It began in `preview.rs`,
//! asked about pandoc and Ghostscript; the agent drawer asks exactly the same question about
//! `claude` and `codex`, and a second copy of the search would have been a second place for the
//! Dock's environment to be got wrong.

use std::path::{Path, PathBuf};

/// Where an external tool is looked for when the PATH does not lead to it.
///
/// CleeCode started from the Dock inherits launchd's environment, not a shell's: no
/// /opt/homebrew/bin, no /Library/TeX/texbin, nothing /etc/paths.d contributes — those reach the
/// PATH from a shell's startup files, and a launcher never reads them. Every tool this program
/// shells out to then looks uninstalled, so a PDF that previews perfectly from a terminal opens
/// from the Dock saying there is no rasteriser on the machine — and an agent that is installed
/// shows up in the drawer's launcher as one that is not.
///
/// The answer is to look where the things are actually installed rather than to trust the
/// environment we were handed. Asking the login shell for its PATH would be the other way, and
/// is what some editors do; it means running somebody's shell configuration at startup, in
/// whichever of five shells they use, to parse a list back out of it. A handful of known
/// prefixes is smaller, and does not depend on the shell being willing to answer.
const TOOL_DIRS: [&str; 5] = [
    "/opt/homebrew/bin",   // Homebrew on Apple silicon
    "/usr/local/bin",      // Homebrew on Intel, MacTeX's Ghostscript, and npm's global bin
    "/opt/local/bin",      // MacPorts
    "/Library/TeX/texbin", // MacTeX and TeX Live, the /etc/paths.d case above
    "/usr/local/texlive/bin/universal-darwin", // a TeX Live installed without the symlinks
];

/// Finds an external tool: the PATH first, since a user who put one somewhere deliberately means
/// that one, then the places above. `None` is the honest "it is not installed", which is what
/// lets a tab say so rather than half-working — and what lets the drawer's launcher draw an
/// agent dim rather than offering to start something that is not there.
pub fn tool(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs = std::env::split_paths(&path).chain(TOOL_DIRS.iter().map(PathBuf::from));
    lookup(dirs, name)
}

/// The search itself, over whichever directories it is given — which is what makes it testable
/// without taking the PATH away from a suite that runs its tests in parallel.
pub fn lookup(dirs: impl Iterator<Item = PathBuf>, name: &str) -> Option<PathBuf> {
    fn runnable(path: &Path) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        }
        #[cfg(not(unix))]
        path.is_file()
    }
    if name.is_empty() {
        return None;
    }
    dirs
        // Windows names its executables with the extension; on Unix the second candidate is
        // simply a file that never exists.
        .flat_map(|dir| [dir.join(name), dir.join(format!("{name}.exe"))])
        .find(|candidate| runnable(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of `lookup`: started from the Dock the process is handed an environment
    /// with none of the package managers' directories on the PATH, and the tools have to be
    /// found anyway. The empty list of directories below is that environment, exactly.
    #[test]
    fn a_tool_is_found_by_where_it_is_rather_than_by_the_path() {
        let none = || std::iter::empty::<PathBuf>();
        // Nowhere to look: nothing found, no panic. This is the launcher's PATH with the
        // fallback directories removed as well — the worst case the search has to survive.
        assert!(lookup(none(), "sh").is_none());

        // `sh` is on every Unix, in a directory nothing here reads from the environment, so it
        // stands in for a rasteriser installed somewhere the PATH does not mention.
        #[cfg(unix)]
        {
            let bin = || std::iter::once(PathBuf::from("/bin"));
            assert_eq!(lookup(bin(), "sh"), Some(PathBuf::from("/bin/sh")));
            // A name nothing provides stays absent however many directories are searched, which
            // is what lets "no rasteriser found" be the truth rather than a guess.
            assert!(lookup(bin(), "cleecode-no-such-tool").is_none());
            // Neither a directory nor a file without the execute bit is a tool, however
            // promising the name: a `gs` that cannot be run must not be reported as found.
            assert!(lookup(std::iter::once(PathBuf::from("/")), "bin").is_none());
            assert!(lookup(bin(), "").is_none());
        }
    }
}
