//! The interpreter-side code, carried inside the binary and written out when it is needed.
//!
//! `assets/octave/*.m` and `assets/python/*.py` are what an Octave or Python session runs to
//! publish its workspace. They have to be somewhere an interpreter can `addpath` or import, and
//! a binary installed from Homebrew has no repository beside it — so they travel *in* the
//! executable and are unpacked into the same temporary directory the snapshots go to.
//!
//! Written once per run rather than shipped as data files: one file to install, one file to
//! update, and no way for the code in the binary and the code on disk to be different versions
//! of each other. The same reason `--install-app` builds its launcher from a template held here
//! rather than carrying a script around.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const OCTAVE: [(&str, &str); 7] = [
    ("cleecode_dbg.m", include_str!("../assets/octave/cleecode_dbg.m")),
    ("cleecode_slice.m", include_str!("../assets/octave/cleecode_slice.m")),
    ("cleecode_boot.m", include_str!("../assets/octave/cleecode_boot.m")),
    ("cleecode_figs.m", include_str!("../assets/octave/cleecode_figs.m")),
    ("cleecode_ws.m", include_str!("../assets/octave/cleecode_ws.m")),
    ("cleecode_ws_tick.m", include_str!("../assets/octave/cleecode_ws_tick.m")),
    ("wsinfo.m", include_str!("../assets/octave/wsinfo.m")),
];

const PYTHON: [(&str, &str); 3] = [
    ("cleecode_pyws.py", include_str!("../assets/python/cleecode_pyws.py")),
    ("cleecode_mpl.py", include_str!("../assets/python/cleecode_mpl.py")),
    ("pythonstartup.py", include_str!("../assets/python/pythonstartup.py")),
];

/// Where the Octave code was unpacked to. Empty if it could not be written, which an interpreter
/// treats as "no hook": `addpath('')` is harmless and the block that would install it is gated
/// on the variable being non-empty.
pub fn octave_lib() -> PathBuf {
    unpacked().0.clone()
}

pub fn python_lib() -> PathBuf {
    unpacked().1.clone()
}

fn unpacked() -> &'static (PathBuf, PathBuf) {
    static ONCE: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();
    ONCE.get_or_init(|| {
        let root = crate::wsnap::snapshot_dir().join("lib");
        (write_all(&root.join("octave"), &OCTAVE), write_all(&root.join("python"), &PYTHON))
    })
}

/// Writes a set of files into `dir`, returning it — or an empty path if anything went wrong.
///
/// A failure here is not worth a message: the workspace view simply never fills in, and the
/// editor and the prompt work exactly as they did before. Nothing the user did caused it and
/// there is nothing they can do about it.
fn write_all(dir: &Path, files: &[(&str, &str)]) -> PathBuf {
    if std::fs::create_dir_all(dir).is_err() {
        return PathBuf::new();
    }
    for (name, body) in files {
        if std::fs::write(dir.join(name), body).is_err() {
            return PathBuf::new();
        }
    }
    dir.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The check that matters is that they are *in* the binary at all: a released CleeCode has
    /// no repository beside it, so a path that resolves during development and not afterwards
    /// would be a feature that works only for whoever built it.
    #[test]
    fn the_interpreter_code_travels_inside_the_binary() {
        assert!(OCTAVE.iter().any(|(name, _)| *name == "cleecode_ws.m"));
        assert!(PYTHON.iter().any(|(name, _)| *name == "pythonstartup.py"));
        // Not empty, and recognisably the thing it claims to be.
        let tick = OCTAVE.iter().find(|(n, _)| *n == "cleecode_ws_tick.m").unwrap().1;
        assert!(tick.contains("jsonencode"), "the Octave hook writes the snapshot");
        // Every function the hook calls has to travel with it: one missing name and the whole
        // tick lands in its own catch, silently, and the panel simply never fills in.
        for called in ["cleecode_figs", "cleecode_ws_tick", "cleecode_dbg", "cleecode_slice"] {
            assert!(
                OCTAVE.iter().any(|(name, _)| *name == format!("{called}.m")),
                "{called} is called by the hook but does not travel with it"
            );
        }
        let startup = PYTHON.iter().find(|(n, _)| *n == "pythonstartup.py").unwrap().1;
        assert!(startup.contains("cleecode_pyws"), "the startup file installs the hook");
    }

    #[test]
    fn unpacking_writes_every_file_where_an_interpreter_can_find_it() {
        let dir = std::env::temp_dir().join(format!("cleecode_assets_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let out = write_all(&dir.join("octave"), &OCTAVE);
        assert_eq!(out, dir.join("octave"));
        for (name, body) in OCTAVE {
            let written = std::fs::read_to_string(dir.join("octave").join(name)).unwrap();
            assert_eq!(written, body, "{name} was written as it is held");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Somewhere unwritable is not an error to report: the workspace view never fills in and
    /// everything else carries on, which is the same stance the rest of CleeCode takes towards
    /// a language server that is not installed.
    #[test]
    fn nowhere_to_write_is_an_empty_path_rather_than_a_failure() {
        assert_eq!(write_all(Path::new("/proc/nonexistent/cleecode"), &OCTAVE), PathBuf::new());
    }
}
