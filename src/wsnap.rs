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
    /// The figures the session has open, each already printed to a PNG. Absent from a session
    /// that has never plotted, and from the Octave prototype before figures were wired up.
    #[serde(default)]
    pub figures: Vec<Figure>,
    /// The last few commands the user typed, newest last, with CleeCode's own injections
    /// already left out by the producer — it is the side that knows which were its.
    #[serde(default)]
    pub history: Vec<String>,
    /// Where the session is stopped, if it is. Absent from a session that has never been
    /// debugged, and from the prototype before the debugger was wired up.
    #[serde(default)]
    pub debug: Debug,
}

/// A session sitting at a breakpoint.
///
/// Reported by the same idle hook that reports everything else — it keeps firing at the `debug>`
/// prompt, which is what makes being stopped something CleeCode can see rather than something
/// the user has to say. While stopped, `vars` above is the *frame's* workspace and not the base
/// one, which is the difference between watching a program run and looking at what it left.
#[derive(Deserialize, Clone, Debug, Default)]
pub struct Debug {
    #[serde(default)]
    pub stopped: bool,
    /// The function stopped in, its file, and the line — all from `dbstack`, with CleeCode's own
    /// frames removed.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub line: usize,
    #[serde(default)]
    pub stack: Vec<Frame>,
}

/// One frame under the one we are stopped in. The producer sends its file too; nothing reads
/// it, because the file that matters is the one being stopped *in* and that is on `Debug`.
#[derive(Deserialize, Clone, Debug, Default)]
pub struct Frame {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub line: usize,
}

/// One figure, printed and described.
///
/// The geometry travels with the picture because a pane pixel has to become a data coordinate
/// without a round trip to the interpreter per mouse move. Nothing reads it yet — navigation is
/// the next step — but it is emitted now so the contract does not have to change to gain it.
#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Figure {
    pub fig: i64,
    pub path: String,
    /// The PNG's real size in pixels, forced rather than assumed: a figure asked for 800x600
    /// does not print to 800x600 unless the paper is set in inches to match.
    #[serde(default)]
    pub png: Vec<u32>,
    #[serde(default)]
    pub axes: Vec<Axes>,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Axes {
    /// The axes rectangle normalised to the figure, origin **bottom-left** — a terminal counts
    /// rows from the top, so this needs flipping before it means anything on screen.
    #[serde(default)]
    pub pos: Vec<f64>,
    #[serde(default)]
    pub xlim: Vec<f64>,
    #[serde(default)]
    pub ylim: Vec<f64>,
    /// `linear` or `log`. With a log axis the mapping goes through log10, and interpolating
    /// linearly would be quietly wrong rather than visibly broken.
    #[serde(default)]
    pub xscale: String,
    #[serde(default)]
    pub yscale: String,
    #[serde(default)]
    pub is3d: bool,
    #[serde(default)]
    pub view: Vec<f64>,
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

/// A rectangle of one variable's values, asked for and answered through a file.
///
/// The snapshot says what a variable *is*; this says what it *contains*. They are separate
/// because a 2000x2000 matrix is four million numbers and nobody wants those written to disk ten
/// times a second on the chance that somebody looks.
#[derive(Deserialize, Clone, Debug, Default)]
pub struct Slice {
    pub name: String,
    /// Why there is nothing to show: undefined now, or a kind of value with no grid in it.
    #[serde(default)]
    pub error: String,
    /// The whole variable's shape, not the rectangle's — it is what the paging is against.
    #[serde(default)]
    pub rows: usize,
    #[serde(default)]
    pub cols: usize,
    /// One-based corner of the rectangle that was actually sent, which may not be the one asked
    /// for: the interpreter clamps, because the variable can have been reassigned since the last
    /// snapshot said how big it was.
    #[serde(default)]
    pub r0: usize,
    #[serde(default)]
    pub c0: usize,
    /// Numbers, or lines of text. Which one is said by `text` — a char array read as a grid of
    /// character codes is what it looks like and not what it means.
    #[serde(default)]
    pub text: bool,
    #[serde(default)]
    data: serde_json::Value,
}

impl Slice {
    pub fn parse(text: &str) -> Option<Slice> {
        serde_json::from_str(text).ok()
    }

    /// The numbers, row by row. Empty for a text variable or an error.
    pub fn grid(&self) -> Vec<Vec<f64>> {
        let Some(rows) = self.data.as_array() else { return Vec::new() };
        rows.iter()
            .filter_map(|row| row.as_array())
            .map(|row| row.iter().map(|v| v.as_f64().unwrap_or(f64::NAN)).collect())
            .collect()
    }

    /// The lines, for a text variable.
    pub fn lines(&self) -> Vec<String> {
        let Some(rows) = self.data.as_array() else { return Vec::new() };
        rows.iter().filter_map(|v| v.as_str()).map(str::to_string).collect()
    }
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

/// A file watched for one variable's values, in the same way and for the same reasons.
pub struct SliceWatch {
    pub path: PathBuf,
    seen: Option<std::time::SystemTime>,
    pub slice: Option<Slice>,
}

impl SliceWatch {
    pub fn new(path: PathBuf) -> SliceWatch {
        SliceWatch { path, seen: None, slice: None }
    }

    pub fn poll(&mut self) -> bool {
        let Ok(modified) = std::fs::metadata(&self.path).and_then(|m| m.modified()) else {
            return false;
        };
        if self.seen == Some(modified) {
            return false;
        }
        self.seen = Some(modified);
        let Ok(text) = std::fs::read_to_string(&self.path) else { return false };
        match Slice::parse(&text) {
            Some(slice) => {
                self.slice = Some(slice);
                true
            }
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
    let figures = dir.join("figs");
    let _ = std::fs::create_dir_all(&figures);
    vec![
        ("CLEECODE_OCTAVE_WS".to_string(), snapshot.to_string_lossy().into_owned()),
        ("CLEECODE_OCTAVE_FIGS".to_string(), figures.to_string_lossy().into_owned()),
        (
            "CLEECODE_OCTAVE_SLICE".to_string(),
            dir.join(format!("slice-{pane_id}.json")).to_string_lossy().into_owned(),
        ),
        // Where CleeCode leaves the question. Asking through a file rather than by typing at
        // the prompt keeps the user's transcript theirs — the rule this whole design started
        // from — and means the answer does not depend on catching a line editor at the right
        // moment.
        // Where CleeCode leaves the breakpoints it wants. Applied by the hook through
        // `evalin`, which was measured to work — so a breakpoint set in the editor never
        // appears in the user's transcript.
        (
            "CLEECODE_OCTAVE_BREAK".to_string(),
            dir.join(format!("break-{pane_id}.json")).to_string_lossy().into_owned(),
        ),
        (
            "CLEECODE_OCTAVE_SLICE_REQ".to_string(),
            dir.join(format!("slicereq-{pane_id}.json")).to_string_lossy().into_owned(),
        ),
        ("CLEECODE_PY_FIGS".to_string(), figures.to_string_lossy().into_owned()),
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

    /// The geometry a figure carries is what a later step needs to turn a click into a data
    /// coordinate. Read now so the contract does not have to change to gain it.
    #[test]
    fn a_figure_arrives_with_the_geometry_of_its_axes() {
        let text = r#"{"v":1,"seq":2,"lang":"octave","vars":[],"figures":[
          {"fig":1,"path":"/tmp/fig1.png","png":[560,420],
           "axes":[{"pos":[0.13,0.11,0.775,0.815],"xlim":[20,60],"ylim":[-1,1],
                    "xscale":"linear","yscale":"linear","is3d":false,"view":[0,90]}]}]}"#;
        let snap = Snapshot::parse(text).unwrap();
        assert_eq!(snap.figures.len(), 1);
        let fig = &snap.figures[0];
        assert_eq!((fig.fig, fig.path.as_str()), (1, "/tmp/fig1.png"));
        assert_eq!(fig.png, vec![560, 420]);
        assert_eq!(fig.axes[0].xlim, vec![20.0, 60.0]);
        assert!(!fig.axes[0].is3d);
        assert_eq!(fig.axes[0].xscale, "linear");
    }

    #[test]
    fn a_session_that_never_plotted_has_no_figures_rather_than_failing_to_parse() {
        let snap = Snapshot::parse(SAMPLE).unwrap();
        assert!(snap.figures.is_empty());
    }

    /// Quoted from what the hook actually wrote while stopped at a breakpoint.
    #[test]
    fn a_stopped_session_says_where_it_is() {
        let text = r#"{"v":1,"seq":9,"lang":"octave","vars":[{"name":"a","class":"double"}],
          "debug":{"stopped":true,"name":"calcola","file":"/proj/calcola.m","line":3,
                   "stack":[{"name":"calcola","file":"/proj/calcola.m","line":3}]}}"#;
        let snap = Snapshot::parse(text).unwrap();
        assert!(snap.debug.stopped);
        assert_eq!((snap.debug.name.as_str(), snap.debug.line), ("calcola", 3));
        assert_eq!(snap.debug.stack.len(), 1);
        // While stopped the variables are the frame's own, which is what makes the panel worth
        // looking at during a debug session rather than after it.
        assert_eq!(snap.vars[0].name, "a");
    }

    #[test]
    fn a_session_that_is_not_stopped_says_so_by_default() {
        let snap = Snapshot::parse(SAMPLE).unwrap();
        assert!(!snap.debug.stopped);
        assert!(snap.debug.stack.is_empty());
    }

    /// Quoted from what the helper actually wrote, against a real Octave.
    #[test]
    fn a_slice_of_a_matrix_arrives_as_numbers() {
        let text = r#"{"name":"A","error":"","rows":6,"cols":6,"r0":1,"c0":1,
                       "data":[[35,1,6,26],[3,32,7,21],[31,9,2,22]],"text":false}"#;
        let slice = Slice::parse(text).unwrap();
        assert_eq!((slice.rows, slice.cols, slice.r0), (6, 6, 1));
        assert_eq!(slice.grid()[0], vec![35.0, 1.0, 6.0, 26.0]);
        assert!(slice.lines().is_empty());
    }

    #[test]
    fn a_char_array_arrives_as_text_rather_than_as_character_codes() {
        let text = r#"{"name":"s","rows":1,"cols":9,"data":["due righe"],"text":true}"#;
        let slice = Slice::parse(text).unwrap();
        assert!(slice.text);
        assert_eq!(slice.lines(), vec!["due righe"]);
        assert!(slice.grid().is_empty());
    }

    /// A variable that has been cleared, or one there is no grid for, comes back saying so
    /// rather than coming back empty and leaving the panel to invent a reason.
    #[test]
    fn something_with_no_grid_says_why() {
        let gone = Slice::parse(r#"{"name":"x","error":"'x' undefined","rows":0,"cols":0}"#).unwrap();
        assert!(gone.error.contains("undefined"));
        assert!(gone.grid().is_empty());
        let cell = Slice::parse(r#"{"name":"c","error":"c is a cell"}"#).unwrap();
        assert!(cell.error.contains("cell"));
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
        assert!(names.contains(&"CLEECODE_OCTAVE_FIGS"));
        assert!(names.contains(&"PYTHONPATH"));
        let by = |key: &str| env.iter().find(|(k, _)| k == key).unwrap().1.clone();
        assert_eq!(by("CLEECODE_OCTAVE_WS"), by("CLEECODE_PY_WS"), "one file per pane, not per language");
        assert!(by("PYTHONSTARTUP").ends_with("pythonstartup.py"));
    }
}
