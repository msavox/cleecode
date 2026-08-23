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

const OCTAVE: [(&str, &str); 11] = [
    ("cleecode_dbg.m", include_str!("../assets/octave/cleecode_dbg.m")),
    ("cleecode_slice.m", include_str!("../assets/octave/cleecode_slice.m")),
    ("cleecode_boot.m", include_str!("../assets/octave/cleecode_boot.m")),
    // Not a function: Octave runs a file with this name when the directory joins the load
    // path, which is how any Octave started in a CleeCode terminal gets the hook — not only
    // the preset's, which was the whole bug.
    ("PKG_ADD", include_str!("../assets/octave/PKG_ADD")),
    ("cleecode_figs.m", include_str!("../assets/octave/cleecode_figs.m")),
    ("cleecode_frame.m", include_str!("../assets/octave/cleecode_frame.m")),
    ("cleecode_grid.m", include_str!("../assets/octave/cleecode_grid.m")),
    ("cleecode_grid_undo.m", include_str!("../assets/octave/cleecode_grid_undo.m")),
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

    /// Every `cleecode_something (` in a body of Octave code: a call, as opposed to a mention
    /// in a comment, which is why the bracket is part of the pattern.
    fn calls_in(body: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (at, _) in body.match_indices("cleecode_") {
            let rest = &body[at..];
            let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(rest.len());
            let (name, after) = rest.split_at(end);
            // A call, not a comment: the next thing that is not a space has to be an opening
            // bracket. `cleecode_grid (f)` counts and "see cleecode_grid for why" does not.
            if after.trim_start().starts_with('(') {
                out.push(name.to_string());
            }
        }
        out
    }

    /// Every `cleecode_something` this code *defines*, whether as a file's own function or as
    /// a subfunction inside one. A call is satisfied by either.
    fn definitions_in(body: &str) -> Vec<String> {
        body.lines()
            .filter_map(|line| line.trim().strip_prefix("function "))
            // `function out = name (args)` and `function name (args)` both end up as the word
            // before the bracket.
            .filter_map(|rest| rest.rsplit('=').next()?.split('(').next())
            .map(|name| name.trim().to_string())
            .filter(|name| name.starts_with("cleecode_"))
            .collect()
    }

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
        //
        // Read out of the code rather than listed by hand, which the list above used to be.
        // A hand-written list only fails for a function somebody remembered to add to it, and
        // the failure this is guarding against is the one nobody remembered — which is exactly
        // what happened when the grid work moved into files of its own: they were called from
        // the first frame after the change and shipped in nothing.
        let called: std::collections::BTreeSet<String> =
            OCTAVE.iter().flat_map(|(_, body)| calls_in(body)).collect();
        let defined: std::collections::BTreeSet<String> =
            OCTAVE.iter().flat_map(|(_, body)| definitions_in(body)).collect();
        assert!(called.len() > 3, "the scan found almost nothing, so it is not scanning");
        for name in called {
            // Either it has a file of its own, or it is a subfunction of one that ships. Both
            // are fine; what is not fine is a name that reaches Octave and resolves to nothing.
            assert!(
                OCTAVE.iter().any(|(file, _)| *file == format!("{name}.m")) || defined.contains(&name),
                "{name} is called by the interpreter code but does not travel with it"
            );
        }
        // The one file that makes the hook apply to an Octave nobody told about it. Without
        // it only the preset's own session captured its figures, and every other Octave — one
        // typed at a shell tab, one started by the Run button — opened real plot windows
        // behind the terminal.
        let pkg_add = OCTAVE.iter().find(|(n, _)| *n == "PKG_ADD").unwrap().1;
        assert!(pkg_add.contains("cleecode_boot"), "PKG_ADD is what boots a session");
        let boot = OCTAVE.iter().find(|(n, _)| *n == "cleecode_boot.m").unwrap().1;
        assert!(
            boot.contains("CLEECODE_OCTAVE_WS"),
            "the boot must do nothing outside CleeCode, since PKG_ADD runs it unconditionally"
        );
        assert!(boot.contains("gnuplot"), "a headless session needs a toolkit that can print");

        let startup = PYTHON.iter().find(|(n, _)| *n == "pythonstartup.py").unwrap().1;
        assert!(startup.contains("cleecode_pyws"), "the startup file installs the hook");

        // The same check for Python, and for the same reason. Everything in that module runs
        // inside a `try` that must not break the user's REPL, so a name that failed to travel
        // has exactly one symptom: a panel that quietly stops changing.
        let hook = PYTHON.iter().find(|(n, _)| *n == "cleecode_pyws.py").unwrap().1;
        for called in ["_answer_slice", "_breakpoints", "_arm", "_history", "_debug_state",
                       "_frame_vars", "_figures", "_snapshot", "_log"] {
            assert!(
                hook.contains(&format!("def {called}(")),
                "{called} is called by the Python hook but is not defined in it"
            );
        }
        assert!(hook.contains("_pyrepl"), "history comes from PyREPL's reader, not readline");
        let backend = PYTHON.iter().find(|(n, _)| *n == "cleecode_mpl.py").unwrap().1;
        assert!(!backend.contains("print("), "the backend writes nothing to the transcript");
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
        // Under a regular *file*, which no platform will let a directory be created inside.
        // The old spelling was /proc/nonexistent — a path Windows is perfectly willing to
        // create, relative to whatever drive it is on, so the test failed there for years.
        let blocker = std::env::temp_dir().join(format!("cleecode-blocker-{}", std::process::id()));
        std::fs::write(&blocker, "not a directory").unwrap();
        assert_eq!(write_all(&blocker.join("octave"), &OCTAVE), PathBuf::new());
        let _ = std::fs::remove_file(&blocker);
    }
}
