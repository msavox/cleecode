//! What an interpreter says about its own workspace, read from the file it writes.
//!
//! The interpreter publishes a JSON snapshot whenever a command finishes — Octave from an idle
//! hook, Python from its prompt — and CleeCode reads it. Nothing is typed at the prompt to ask:
//! injecting `whos` there would pollute the user's transcript, fight whatever they are half-way
//! through typing, and do nothing at all while the interpreter is busy.
//!
//! One shape for both languages, with a `lang` field saying which wrote it, so the panel is
//! written once. The producers are the prototypes in `assets/octave/` and `assets/python/`; the
//! contract between them and this file is documented in `docs/ide-mode-octave.md` §4.

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// One variable, as the interpreter described it.
#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Var {
    pub name: String,
    pub class: String,
    /// Native shape: `[10, 10]`, or `[3]` for a 1-D Python array. Octave's is always 2-D and
    /// Python's is not, and they are not forced to match — they mean different things.
    #[serde(default)]
    pub size: Vec<i64>,
    #[serde(default)]
    pub bytes: Option<i64>,
    /// The letters `whos` prints: `c` complex, `s` sparse, `g` global, `p` persistent.
    #[serde(default)]
    pub attr: String,
    // Null wherever they make no sense — a char array, a struct, a function handle — and also
    // where the array is too large to be worth scanning ten times a second. NaN and Inf
    // serialise as null too, which is why these are Option rather than f64 with a sentinel.
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub mean: Option<f64>,
    #[serde(default)]
    pub nans: u64,
    #[serde(default)]
    pub preview: String,
}

impl Var {
    /// The shape as a person writes it: `10x10`, or `3` for a plain vector.
    pub fn shape(&self) -> String {
        if self.size.is_empty() {
            return String::new();
        }
        self.size.iter().map(|n| n.to_string()).collect::<Vec<_>>().join("x")
    }
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Snapshot {
    /// The contract's version. Read rather than ignored: a file written by a newer CleeCode is
    /// said to be one, instead of being half-read into a table that looks right and is not.
    #[serde(default)]
    pub v: u32,
    /// Rises by one per snapshot. Compared, never interpreted — it is how the panel can say
    /// "this is newer" without trusting either clock.
    #[serde(default)]
    pub seq: u64,
    /// The directory the session is working in, shown so two prompts in two projects are
    /// tellable apart.
    #[serde(default)]
    pub cwd: String,
    /// `"octave"` or `"python"`. Absent in the Octave prototype's older output, which is why
    /// this defaults rather than failing the parse.
    #[serde(default)]
    pub lang: String,
    #[serde(default)]
    pub vars: Vec<Var>,
}

impl Snapshot {
    pub fn parse(text: &str) -> Option<Snapshot> {
        serde_json::from_str(text).ok()
    }

    /// Total bytes across the variables that reported any, for the panel's summary line.
    pub fn bytes(&self) -> i64 {
        self.vars.iter().filter_map(|v| v.bytes).sum()
    }
}

/// The variables by name.
///
/// One order, because the viewer has no keys to ask for another — it is a window you glance at.
/// Sorting by size, to find what is eating the memory, is worth having and belongs with whatever
/// gives it a keyboard.
pub fn ordered(vars: &[Var]) -> Vec<&Var> {
    let mut out: Vec<&Var> = vars.iter().collect();
    out.sort_by_key(|v| v.name.to_lowercase());
    out
}

/// A snapshot file being watched, and what was last read from it.
pub struct Watch {
    pub path: PathBuf,
    seen: Option<std::time::SystemTime>,
    pub snapshot: Option<Snapshot>,
}

impl Watch {
    pub fn new(path: PathBuf) -> Watch {
        Watch { path, seen: None, snapshot: None }
    }

    /// Re-reads if the file has changed since last time. `true` when something new arrived.
    ///
    /// mtime rather than a filesystem watcher: the file is written by rename, so it appears
    /// whole and its timestamp moves exactly once per snapshot. A watcher would be a thread and
    /// a dependency to learn the same thing a `stat` in the frame loop already knows.
    pub fn poll(&mut self) -> bool {
        let Ok(meta) = std::fs::metadata(&self.path) else { return false };
        let Ok(modified) = meta.modified() else { return false };
        if self.seen == Some(modified) {
            return false;
        }
        self.seen = Some(modified);
        let Ok(text) = std::fs::read_to_string(&self.path) else { return false };
        match Snapshot::parse(&text) {
            Some(snapshot) => {
                self.snapshot = Some(snapshot);
                true
            }
            // A half-written file should be impossible — the producers write to a temp file and
            // rename — but a corrupt one must not throw away the last good reading.
            None => false,
        }
    }
}

/// Where this CleeCode's snapshots live. Under the process id, so the files of one that is no
/// longer running are recognisable as such and two open at once do not collide.
pub fn snapshot_dir() -> PathBuf {
    std::env::temp_dir().join(format!("cleecode-{}", std::process::id()))
}

/// Where one pane's snapshot lives. One file per pane, so two prompts describe two sessions
/// instead of overwriting each other's answer.
pub fn snapshot_path(dir: &Path, pane_id: u64) -> PathBuf {
    dir.join(format!("ws-{pane_id}.json"))
}

/// The most recently written snapshot in `dir`.
///
/// Which pane the viewer is showing is answered by "whichever one last ran something", rather
/// than by wiring it to a particular pane. With one interpreter open that is the only answer;
/// with two it follows the one being worked in, which is what a single window that stays on
/// screen should do — and it means the viewer needs to know nothing about panes at all.
pub fn newest_in(dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e != "json").unwrap_or(true) {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else { continue };
        if best.as_ref().map(|(seen, _)| modified > *seen).unwrap_or(true) {
            best = Some((modified, path));
        }
    }
    best.map(|(_, path)| path)
}

/// The environment an interpreter needs to publish its workspace, for a shell that may end up
/// running either language — or neither, in which case the hooks are inert and cost nothing.
///
/// Both are set on every shell CleeCode starts. Octave's block only fires when the variable is
/// there, Python's startup file does nothing without it, and a shell that never starts either
/// interpreter simply carries two unread variables.
pub fn shell_env(dir: &Path, pane_id: u64, lib_octave: &Path, lib_python: &Path) -> Vec<(String, String)> {
    let snapshot = snapshot_path(dir, pane_id);
    vec![
        ("CLEECODE_OCTAVE_WS".to_string(), snapshot.to_string_lossy().into_owned()),
        ("CLEECODE_OCTAVE_LIB".to_string(), lib_octave.to_string_lossy().into_owned()),
        ("CLEECODE_PY_WS".to_string(), snapshot.to_string_lossy().into_owned()),
        ("PYTHONSTARTUP".to_string(), lib_python.join("pythonstartup.py").to_string_lossy().into_owned()),
        ("PYTHONPATH".to_string(), lib_python.to_string_lossy().into_owned()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "v": 1, "seq": 12, "time": 1787214595.164, "pid": 65454, "cwd": "/proj",
      "lang": "octave",
      "vars": [
        {"name":"a","class":"double","size":[1,10],"bytes":80,"attr":"",
         "min":1,"max":1001,"mean":500.5,"nans":0,"preview":"[999;2;3]"},
        {"name":"s","class":"char","size":[1,4],"bytes":4,"attr":"",
         "min":null,"max":null,"mean":null,"nans":0,"preview":"ciao"}
      ]}"#;

    #[test]
    fn a_snapshot_parses_into_what_the_panel_needs() {
        let snap = Snapshot::parse(SAMPLE).unwrap();
        assert_eq!((snap.v, snap.seq, snap.lang.as_str()), (1, 12, "octave"));
        assert_eq!(snap.cwd, "/proj");
        assert_eq!(snap.vars.len(), 2);
        assert_eq!(snap.vars[0].shape(), "1x10");
        assert_eq!(snap.vars[0].max, Some(1001.0));
        assert_eq!(snap.bytes(), 84);
    }

    /// A char array has no minimum, and neither has an array too large to have been scanned.
    /// Both arrive as null, and so do NaN and Inf — which is why these are Option rather than
    /// a number with a sentinel value that would print as one.
    #[test]
    fn a_statistic_that_makes_no_sense_is_absent_rather_than_zero() {
        let snap = Snapshot::parse(SAMPLE).unwrap();
        assert_eq!(snap.vars[1].min, None);
        assert_eq!(snap.vars[1].mean, None);
    }

    /// The Octave prototype predates the `lang` field, so its output has none. Failing the whole
    /// parse over a missing field would mean the panel showing nothing rather than a workspace
    /// without a language label.
    #[test]
    fn an_older_snapshot_without_a_language_still_reads() {
        let text = r#"{"v":1,"seq":1,"pid":1,"cwd":"/","vars":[{"name":"x","class":"double"}]}"#;
        let snap = Snapshot::parse(text).unwrap();
        assert_eq!(snap.lang, "");
        assert_eq!(snap.vars[0].name, "x");
        assert_eq!(snap.vars[0].shape(), "", "no size reported is no size shown");
    }

    #[test]
    fn nonsense_is_not_a_snapshot() {
        assert!(Snapshot::parse("not json").is_none());
        assert!(Snapshot::parse("").is_none());
    }

    #[test]
    fn variables_are_listed_by_name_whatever_order_they_arrived_in() {
        let named = |n: &str| Var { name: n.into(), ..Var::default() };
        let vars = vec![named("Zeta"), named("beta"), named("Alpha")];
        let names: Vec<String> = ordered(&vars).iter().map(|v| v.name.clone()).collect();
        // Case-insensitively, or a capital would sort a variable away from its neighbours.
        assert_eq!(names, ["Alpha", "beta", "Zeta"]);
    }

    #[test]
    fn a_watch_reads_once_per_change() {
        let dir = std::env::temp_dir().join(format!("cleecode_wsnap_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ws.json");
        let mut watch = Watch::new(path.clone());
        assert!(!watch.poll(), "nothing there yet is not an update");

        std::fs::write(&path, SAMPLE).unwrap();
        assert!(watch.poll(), "the first snapshot arrives");
        assert_eq!(watch.snapshot.as_ref().unwrap().seq, 12);
        assert!(!watch.poll(), "and is not read again until it changes");

        // A file that is not a snapshot must not throw away the last good one.
        std::fs::write(&path, "half a fi").unwrap();
        filetime_bump(&path);
        assert!(!watch.poll());
        assert_eq!(watch.snapshot.as_ref().unwrap().seq, 12, "the last good reading survives");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Some filesystems have a coarse enough mtime that two writes in the same test land on the
    /// same timestamp, which would make the watch skip the second one.
    fn filetime_bump(path: &Path) {
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
        let _ = std::fs::File::open(path).and_then(|f| f.set_times(
            std::fs::FileTimes::new().set_modified(later),
        ));
    }

    #[test]
    fn every_pane_gets_its_own_file() {
        let dir = snapshot_dir();
        assert_ne!(snapshot_path(&dir, 0), snapshot_path(&dir, 1));
        assert!(dir.to_string_lossy().contains(&std::process::id().to_string()));
    }

    /// The viewer follows whichever session last ran something, so it needs to know nothing
    /// about panes — and with two interpreters open it shows the one being worked in.
    #[test]
    fn the_newest_snapshot_is_the_one_shown() {
        let dir = std::env::temp_dir().join(format!("cleecode_newest_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(newest_in(&dir), None, "nothing written yet");

        std::fs::write(dir.join("ws-0.json"), SAMPLE).unwrap();
        std::fs::write(dir.join("ws-1.json"), SAMPLE).unwrap();
        filetime_bump(&dir.join("ws-1.json"));
        assert_eq!(newest_in(&dir), Some(dir.join("ws-1.json")));

        // Anything that is not a snapshot is not a candidate.
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        filetime_bump(&dir.join("notes.txt"));
        assert_eq!(newest_in(&dir), Some(dir.join("ws-1.json")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Both languages' variables are set on every shell. A shell that starts neither interpreter
    /// carries two unread names, which costs nothing; the alternative is guessing at spawn time
    /// what the user is about to type.
    #[test]
    fn a_shell_is_given_what_either_interpreter_would_need() {
        let env = shell_env(Path::new("/snaps"), 2, Path::new("/lib/octave"), Path::new("/lib/python"));
        let names: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert!(names.contains(&"CLEECODE_OCTAVE_WS"));
        assert!(names.contains(&"CLEECODE_OCTAVE_LIB"));
        assert!(names.contains(&"PYTHONSTARTUP"));
        assert!(names.contains(&"PYTHONPATH"));
        let by = |key: &str| env.iter().find(|(k, _)| k == key).unwrap().1.clone();
        assert_eq!(by("CLEECODE_OCTAVE_WS"), by("CLEECODE_PY_WS"), "one file per pane, not per language");
        assert!(by("PYTHONSTARTUP").ends_with("pythonstartup.py"));
    }
}
