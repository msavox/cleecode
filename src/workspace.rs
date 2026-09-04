//! Named workspaces: a saved snapshot of a whole set-up — project root, open files, frame
//! sizes, and the terminal windows/tabs with their names and startup commands — so a layout
//! can be reopened exactly as it was left, shells included.
//!
//! One TOML file per workspace under `<config>/workspaces/`, named after a slug of the
//! workspace's name so the files stay hand-editable and copyable between machines.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One shell inside a terminal window: what it is called, and what to run when the workspace
/// is opened (`claude`, `octave`, `npm run dev`…). Both optional — a plain unnamed shell is
/// the common case.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct WorkspaceTab {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub startup_command: Option<String>,
}

/// One tiled terminal window: its share of the terminal region, which of its tabs was on
/// screen, and the tabs themselves.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WorkspaceTerminal {
    #[serde(default = "default_weight")]
    pub weight: u16,
    #[serde(default)]
    pub active: usize,
    #[serde(default)]
    pub tabs: Vec<WorkspaceTab>,
}

fn default_weight() -> u16 {
    crate::terminal_panel::TERMINAL_WEIGHT_DEFAULT
}

/// The frame geometry a workspace restores. Deliberately a copy of the layout half of
/// `Settings` rather than the whole thing: a workspace is about shape, not preferences —
/// nobody wants opening one to flip their tab size or language.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WorkspaceLayout {
    pub show_sidebar: bool,
    pub show_terminal: bool,
    pub show_menubar: bool,
    pub sidebar_width: u16,
    pub terminal_pct: u16,
    pub terminal_on_right: bool,
    pub split_view: bool,
    pub split_pct: u16,
}

/// What the agent drawer was doing when the workspace was saved.
///
/// Every field defaulted, unlike [`WorkspaceLayout`] beside it, and for a reason rather than by
/// oversight: the layout block predates the file format being hand-edited and its all-required
/// convention is now what the round-trip test guards. This block is new, so a `[drawer]` written
/// by hand with the one line somebody meant — `open = true` — has to load.
///
/// The agent is stored by its program name (`claude`, `codex`) rather than by a number, because
/// that is what the file is for: it is the word you would type, and a workspace file is meant to
/// be readable by whoever opens it in an editor.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct WorkspaceDrawer {
    #[serde(default)]
    pub open: bool,
    #[serde(default = "default_drawer_width")]
    pub width: u16,
    #[serde(default)]
    pub agent: Option<String>,
}

fn default_drawer_width() -> u16 {
    crate::settings::DRAWER_PCT_DEFAULT
}

/// Scalars first, then the nested tables: TOML rejects a bare value written after a table,
/// and `save` swallows serialization errors, so the field order here is load-bearing.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Workspace {
    pub name: String,
    pub root: PathBuf,
    #[serde(default)]
    pub open_files: Vec<PathBuf>,
    #[serde(default)]
    pub active_file: Option<PathBuf>,
    #[serde(default)]
    pub active_venv: Option<String>,
    /// The executable *Debug ▸ Start* runs, remembered so the question is asked once per project
    /// rather than once per session.
    ///
    /// Beside `active_venv` because it is the same kind of fact: a choice about how this project
    /// is run, made once and worth keeping, rather than anything about the shape of the window.
    /// `None` writes nothing at all, so a file saved before the debugger existed and a file saved
    /// by somebody who has never debugged are the same file.
    #[serde(default)]
    pub debuggee: Option<PathBuf>,
    #[serde(default)]
    pub active_terminal: usize,
    pub layout: WorkspaceLayout,
    /// The drawer, when there was one.
    ///
    /// It serializes as a table (`[drawer]`), so it belongs down here with `layout` and above
    /// `terminals` — every scalar is already written by the time TOML reaches it, and the array
    /// of tables that follows may come after another table. `None` is written as nothing at all,
    /// which is why a file saved before the drawer existed and a file saved without one are the
    /// same file.
    #[serde(default)]
    pub drawer: Option<WorkspaceDrawer>,
    #[serde(default)]
    pub terminals: Vec<WorkspaceTerminal>,
}

/// The layout workspace. It is not a file and never becomes one: picking it puts the frames back
/// the way CleeCode ships them, in whatever project you are already in. It exists because the
/// layout is easy to wander away from — a hidden sidebar here, a dragged seam there — and there
/// was no way back short of editing settings by hand.
pub const DEFAULT_NAME: &str = "Default layout";

/// The workspaces CleeCode ships rather than reads from a file.
///
/// Two of them set up a session for a language: a prompt for it already running, a shell beside
/// it, and the frames arranged the way that kind of work wants them. One puts the frames back to
/// CleeCode's own shape — the default layout. And one strips them away entirely: no sidebar, no
/// terminal, no menu bar, just the editor — the micro/nano feel, quoted rather than named, since
/// the preset is called `minimal` and not after either of them. Coding agents used to be four
/// more of these, a tab with the agent's name and a shell beside it; they are not any more, now
/// that the agent drawer is a keystroke away and a preset that only opened a terminal program had
/// nothing left to add. Being built in is why the four that remain cannot be deleted — there is
/// nothing on disk to delete — and why they travel between projects.
pub const BUILT_INS: [&str; 4] = [DEFAULT_NAME, "octave", "pylab", "minimal"];

/// Whether `name` is one of the built-ins. Compared by slug, so case and spacing do not matter.
pub fn is_built_in(name: &str) -> bool {
    BUILT_INS.iter().any(|b| slug(b) == slug(name))
}

/// The built-in of that name, if any, spelled the way CleeCode spells it — so a message about a
/// clash can name the built-in rather than echo back what the user typed.
pub fn built_in_named(name: &str) -> Option<&'static str> {
    BUILT_INS.iter().copied().find(|b| slug(b) == slug(name))
}

/// What a built-in needs to know about the machine it is opening on.
pub struct Shape {
    pub root: PathBuf,
    /// The window's width in columns. A layout that is right at 200 columns and unusable at 80
    /// is not the best layout, it is the best layout for whoever wrote it.
    pub cols: u16,
    /// The Python to start, already resolved against any active virtualenv — otherwise `pylab`
    /// would open a prompt without the packages the project was set up for, which is the one
    /// thing that preset exists to avoid.
    pub python: String,
    /// The command that opens the workspace view, or `None` where CleeCode cannot name its own
    /// executable — in which case the preset is the prompt and the shell, and nothing pretends
    /// otherwise.
    pub workspace_view: Option<String>,
}

/// Above this a window has room to spare, and the file tree can be a little narrower without
/// becoming a column of truncated names — the width goes to the editor, which is splitting in
/// two to put a figure beside the code that drew it.
///
/// It used to decide something larger: whether the prompt sat beside the editor or under it.
/// It does not any more — the presets stack, always — because a three-column window leaves the
/// editor's two halves about a third each once a figure opens, and a plot that size is a
/// thumbnail.
const SIDE_BY_SIDE_COLS: u16 = 150;

/// A built-in workspace by name, shaped for the window it is opening in.
pub fn built_in(name: &str, shape: &Shape) -> Option<Workspace> {
    let name = built_in_named(name)?;
    Some(match name {
        // `--persist` because `--eval` otherwise runs and exits; `--path` so the interpreter can
        // find the boot file, which installs the hook only if CleeCode asked for one. `octave`
        // and not `octave-cli`: plotting needs the qt toolkit, and the cli build here has only
        // fltk, which wants an X display that is not installed on a Mac.
        "octave" => session_workspace(
            name,
            shape,
            // No --path and no --eval: OCTAVE_PATH puts the library on the load path and its
            // PKG_ADD boots the hook, so this reads like the command a user would type.
            "octave --no-gui --persist",
            "octave",
        ),
        "pylab" => session_workspace(name, shape, &shape.python, "python"),
        // Bare: no sidebar, no terminal, no menu bar — the editor and nothing else, the
        // micro/nano feel quoted rather than named. Layout-only like the default arm below, and
        // for the same reason: the point is the shape of the window, not a set of shells, so
        // `apply_workspace` keeps whatever terminals are already running when the project has
        // not changed.
        "minimal" => Workspace {
            name: name.to_string(),
            root: shape.root.clone(),
            open_files: Vec::new(),
            active_file: None,
            active_venv: None,
            // A built-in has no opinion about what to debug, the same way it has none about the
            // drawer: it is a shape for the window, not a project's own choices.
            debuggee: None,
            active_terminal: 0,
            layout: WorkspaceLayout {
                show_sidebar: false,
                show_terminal: false,
                show_menubar: false,
                sidebar_width: 30,
                terminal_pct: 35,
                terminal_on_right: false,
                split_view: false,
                split_pct: 50,
            },
            drawer: None,
            terminals: Vec::new(),
        },
        // The layout one keeps no terminals of its own: the point is the shape of the window,
        // not a set of shells, and `apply_workspace` keeps the ones already running when the
        // project has not changed.
        _ => Workspace {
            name: name.to_string(),
            root: shape.root.clone(),
            open_files: Vec::new(),
            active_file: None,
            active_venv: None,
            // A built-in has no opinion about what to debug, the same way it has none about the
            // drawer: it is a shape for the window, not a project's own choices.
            debuggee: None,
            active_terminal: 0,
            layout: WorkspaceLayout {
                show_sidebar: true,
                show_terminal: true,
                show_menubar: true,
                sidebar_width: 30,
                terminal_pct: 35,
                terminal_on_right: false,
                split_view: false,
                split_pct: 50,
            },
            // The built-ins say nothing about the drawer: they are about the shape of the window
            // and the shells in it, and a preset that closed a drawer somebody already had open
            // would be answering a question it was not asked.
            drawer: None,
            terminals: Vec::new(),
        },
    })
}

/// The shape both language presets share, which is the whole argument for having a seam: they
/// differ by the command that starts the interpreter and by what the tab is called.
///
/// **One terminal window, two tabs.** A second *window* would take screen away from the editor
/// permanently to hold a shell that is used for a minute at a time; a second tab costs nothing
/// and is one keystroke away. Tab 1 is the interpreter, because that is where Ctrl+Shift+X
/// lands and it should be what you are looking at.
///
/// **The prompt goes underneath, across the full width** — the classic arrangement, and it wins
/// here for a reason that only shows up once plotting works. A figure arrives as a tab and is
/// read *beside* the code that drew it, which means the editor splits in two; with the prompt
/// also taking a column, those halves are a third of the window each, and a plot a third of a
/// window wide is a thumbnail. Underneath, the editor keeps the whole width to divide, and the
/// prompt keeps it too — which is what numeric output wanted in the first place, since a matrix
/// row wraps into nonsense in a narrow pane. The sidebar stays, narrower: this kind of work is
/// usually a handful of scripts, so the tree is for finding them rather than for living in.
///
/// **The workspace view is a second window, not a third tab**, and that is the whole difference
/// between watching your variables and asking about them. A tab is somewhere you go; a window is
/// something you glance at. You run a cell and look up — which is what the Octave and MATLAB
/// desktops dock it for.
fn session_workspace(name: &str, shape: &Shape, start: &str, tab: &str) -> Workspace {
    let wide = shape.cols >= SIDE_BY_SIDE_COLS;
    let mut terminals = vec![WorkspaceTerminal {
        // Two thirds to the prompt, one to the workspace: the prompt is where the work happens
        // and the view is read a line at a time.
        weight: default_weight() * 2,
        active: 0,
        tabs: vec![
            WorkspaceTab { name: Some(tab.to_string()), startup_command: Some(start.to_string()) },
            // A plain shell, for the things a prompt is bad at: git, ls, pip. Unnamed and
            // unstarted, so it costs a shell and no screen.
            WorkspaceTab { name: Some("shell".to_string()), startup_command: None },
        ],
    }];
    if let Some(view) = &shape.workspace_view {
        terminals.push(WorkspaceTerminal {
            weight: default_weight(),
            active: 0,
            tabs: vec![WorkspaceTab {
                name: Some("workspace".to_string()),
                startup_command: Some(view.clone()),
            }],
        });
    }
    Workspace {
        name: name.to_string(),
        root: shape.root.clone(),
        open_files: Vec::new(),
        active_file: None,
        active_venv: None,
        debuggee: None,
        active_terminal: 0,
        layout: WorkspaceLayout {
            show_sidebar: true,
            show_terminal: true,
            show_menubar: true,
            sidebar_width: if wide { 24 } else { 26 },
            // A share of the height, and a modest one: a prompt needs fewer rows to be useful
            // than an editor does, and on a wide window those rows are already long.
            terminal_pct: 38,
            terminal_on_right: false,
            split_view: false,
            split_pct: 50,
        },
        // See the layout preset above: a built-in has no opinion about the drawer.
        drawer: None,
        terminals,
    }
}

/// Filename stem for a workspace: lowercase, with every run of non-alphanumerics collapsed to
/// a single dash, so "My Project (2)" and "my-project-2" don't fight over the same file while
/// still producing something readable in the directory listing.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() { "workspace".to_string() } else { trimmed.to_string() }
}

/// Where workspaces live. `None` when there is no config directory to speak of, in which case
/// saving is quietly unavailable — the same stance `Settings` takes.
pub fn dir() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("workspaces"))
}

fn file_in(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{}.toml", slug(name)))
}

pub fn save_in(dir: &Path, ws: &Workspace) -> Result<PathBuf, String> {
    // Names the built-in that was actually clashed with. It used to name `DEFAULT_NAME`
    // whichever one you hit, which was true when there was only one and became a lie the moment
    // there were three — and a refusal that names the wrong thing reads as a bug in the save.
    if let Some(built_in) = built_in_named(&ws.name) {
        return Err(format!("\"{built_in}\" is a built-in workspace and cannot be overwritten"));
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let text = toml::to_string_pretty(ws).map_err(|e| e.to_string())?;
    let path = file_in(dir, &ws.name);
    // Saving over a workspace is the moment its previous version stops existing, so the swap has
    // to be one step: a truncating write interrupted halfway leaves a layout that neither opens
    // nor can be got back. See `settings::write_atomic`.
    crate::settings::write_atomic(&path, text.as_bytes()).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn load_in(dir: &Path, name: &str) -> Option<Workspace> {
    let text = std::fs::read_to_string(file_in(dir, name)).ok()?;
    toml::from_str(&text).ok()
}

/// What to open for `name`: a file of the user's own, or a built-in.
///
/// The file wins, and that is the point of this function existing.
///
/// The old code asked `is_default(name)` *before* it ever looked on disk, so a built-in shadowed
/// a user's own workspace of that name silently — theirs simply stopped opening, with nothing
/// said. Harmless while the only reserved name was "Default layout"; the moment `octave` became
/// reserved it could hide a file somebody had saved months earlier. Saving under a built-in name
/// is refused, so a clash can only be one of those older files: irreplaceable, where the built-in
/// is documented and reproducible. The caller is told, so the shadowing is a sentence on screen
/// rather than a workspace that quietly stopped existing.
pub fn resolve_in(dir: &Path, name: &str, shape: &Shape) -> (Option<Workspace>, Option<&'static str>) {
    match (load_in(dir, name), built_in_named(name)) {
        (Some(theirs), Some(built_in)) => (Some(theirs), Some(built_in)),
        (Some(theirs), None) => (Some(theirs), None),
        (None, Some(_)) => (built_in(name, shape), None),
        (None, None) => (None, None),
    }
}

/// The same, against the real workspace directory. `None` for the directory means there is
/// nowhere to have saved anything, so a built-in is all there can be.
pub fn resolve(name: &str, shape: &Shape) -> (Option<Workspace>, Option<&'static str>) {
    match dir() {
        Some(dir) => resolve_in(&dir, name, shape),
        None => (built_in(name, shape), None),
    }
}

/// Every readable workspace, by name. Unparsable files are skipped rather than reported: a
/// hand-edited file with a typo shouldn't make the whole list unavailable.
pub fn list_in(dir: &Path) -> Vec<Workspace> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Workspace> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "toml").unwrap_or(false))
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .filter_map(|text| toml::from_str::<Workspace>(&text).ok())
        .collect();
    out.sort_by_key(|w| w.name.to_lowercase());
    out
}

pub fn delete_in(dir: &Path, name: &str) -> bool {
    std::fs::remove_file(file_in(dir, name)).is_ok()
}

pub fn save(ws: &Workspace) -> Result<PathBuf, String> {
    let dir = dir().ok_or_else(|| "no config directory".to_string())?;
    save_in(&dir, ws)
}

pub fn load(name: &str) -> Option<Workspace> {
    load_in(&dir()?, name)
}

pub fn list() -> Vec<Workspace> {
    dir().map(|d| list_in(&d)).unwrap_or_default()
}

pub fn delete(name: &str) -> bool {
    dir().map(|d| delete_in(&d, name)).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cleecode_ws_test_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample(name: &str) -> Workspace {
        Workspace {
            name: name.to_string(),
            root: PathBuf::from("/work/project"),
            open_files: vec![PathBuf::from("/work/project/src/main.rs")],
            active_file: Some(PathBuf::from("/work/project/src/main.rs")),
            active_venv: Some(".venv".to_string()),
            debuggee: Some(PathBuf::from("/work/project/target/debug/project")),
            active_terminal: 1,
            layout: WorkspaceLayout {
                show_sidebar: true,
                show_terminal: true,
                show_menubar: false,
                sidebar_width: 26,
                terminal_pct: 40,
                terminal_on_right: true,
                split_view: true,
                split_pct: 60,
            },
            drawer: Some(WorkspaceDrawer {
                open: true,
                width: 45,
                agent: Some("codex".to_string()),
            }),
            terminals: vec![
                WorkspaceTerminal {
                    weight: 1200,
                    active: 0,
                    tabs: vec![WorkspaceTab {
                        name: Some("claude".to_string()),
                        startup_command: Some("claude".to_string()),
                    }],
                },
                WorkspaceTerminal {
                    weight: 800,
                    active: 1,
                    tabs: vec![
                        WorkspaceTab::default(),
                        WorkspaceTab { name: Some("octave".to_string()), startup_command: Some("octave".to_string()) },
                    ],
                },
            ],
        }
    }

    /// `save` swallows serialization errors, so a field ordering TOML rejects (a scalar after a
    /// table) would silently stop workspaces from being written at all.
    #[test]
    fn workspace_survives_a_round_trip_through_disk() {
        let dir = temp_dir("roundtrip");
        let ws = sample("My Project");
        let path = save_in(&dir, &ws).expect("must save");
        assert_eq!(path.file_name().unwrap(), "my-project.toml");

        let back = load_in(&dir, "My Project").expect("must load back");
        assert_eq!(back, ws);
        // Terminal names and startup commands are the whole point: pinned explicitly.
        assert_eq!(back.terminals[0].tabs[0].name.as_deref(), Some("claude"));
        assert_eq!(back.terminals[0].tabs[0].startup_command.as_deref(), Some("claude"));
        assert_eq!(back.terminals[1].tabs[1].startup_command.as_deref(), Some("octave"));
        assert_eq!(back.terminals[1].active, 1);
        // The drawer is a table written between `[layout]` and the `[[terminals]]` array, which
        // is the placement the ordering rule above is about — a field added anywhere else would
        // have stopped every workspace from being written, silently.
        let drawer = back.drawer.expect("the drawer survives the round trip");
        assert_eq!((drawer.open, drawer.width, drawer.agent.as_deref()), (true, 45, Some("codex")));
        // The debuggee is the answer to a question asked once per project, so the whole point of
        // it is surviving the trip to disk: pinned explicitly, next to the terminal commands that
        // are pinned for the same reason.
        assert_eq!(
            back.debuggee.as_deref(),
            Some(Path::new("/work/project/target/debug/project")),
            "what Debug ▸ Start runs has to come back"
        );
        // And it is written where a scalar can go — before the first table — or nothing at all
        // would have been written: `save` swallows serialization errors.
        let text = std::fs::read_to_string(&path).unwrap();
        let (before_tables, _) = text.split_once("[layout]").expect("the layout block is there");
        assert!(before_tables.contains("debuggee"), "written among the scalars:\n{text}");
    }

    /// A workspace saved without a drawer writes no `[drawer]` block at all, so files from
    /// before the drawer existed and files from a session that never opened one are the same
    /// file — and both load.
    #[test]
    fn a_workspace_without_a_drawer_writes_nothing_about_one() {
        let dir = temp_dir("no-drawer");
        let mut ws = sample("Plain");
        ws.drawer = None;
        let path = save_in(&dir, &ws).expect("must save");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("drawer"), "no empty block is left behind:\n{text}");
        assert_eq!(load_in(&dir, "Plain").expect("must load back"), ws);
    }

    /// The `[drawer]` block is hand-editable like the rest of the file: one line of it has to be
    /// enough. This is why its fields are defaulted where `[layout]`'s are required.
    #[test]
    fn a_hand_written_drawer_block_needs_only_the_line_you_meant() {
        let dir = temp_dir("hand-drawer");
        let text = "name = \"bare\"\nroot = \"/work/project\"\n\n[layout]\n\
                    show_sidebar = true\nshow_terminal = true\nshow_menubar = true\n\
                    sidebar_width = 30\nterminal_pct = 35\nterminal_on_right = false\n\
                    split_view = false\nsplit_pct = 50\n\n[drawer]\nopen = true\n";
        std::fs::write(dir.join("bare.toml"), text).unwrap();
        let ws = load_in(&dir, "bare").expect("a one-line drawer block must load");
        let drawer = ws.drawer.expect("the block is there");
        assert!(drawer.open);
        assert_eq!(drawer.width, crate::settings::DRAWER_PCT_DEFAULT, "the rest defaults");
        assert_eq!(drawer.agent, None, "and an unnamed drawer opens on the launcher");
    }

    #[test]
    fn listing_is_alphabetical_and_deleting_removes_one() {
        let dir = temp_dir("listing");
        for name in ["zeta", "Alpha", "middle"] {
            save_in(&dir, &sample(name)).unwrap();
        }
        // A stray file that isn't a workspace must not break the listing.
        std::fs::write(dir.join("notes.txt"), "hello").unwrap();
        std::fs::write(dir.join("broken.toml"), "name = ").unwrap();

        let names: Vec<String> = list_in(&dir).into_iter().map(|w| w.name).collect();
        assert_eq!(names, vec!["Alpha".to_string(), "middle".to_string(), "zeta".to_string()]);

        assert!(delete_in(&dir, "middle"));
        assert!(!delete_in(&dir, "middle"), "deleting twice reports failure rather than pretending");
        let names: Vec<String> = list_in(&dir).into_iter().map(|w| w.name).collect();
        assert_eq!(names, vec!["Alpha".to_string(), "zeta".to_string()]);
    }

    /// A hand-edited file with a typo is skipped — and, the part worth pinning, left exactly
    /// where it is. Someone's saved layout is theirs: the cost of a typo is that one workspace
    /// not appearing until it is fixed, never the file being quietly repaired away.
    #[test]
    fn a_broken_workspace_file_is_skipped_but_never_touched() {
        let dir = temp_dir("broken");
        save_in(&dir, &sample("good")).unwrap();
        let broken = dir.join("bad.toml");
        let text = "name = \nroot = oops\n";
        std::fs::write(&broken, text).unwrap();

        let names: Vec<String> = list_in(&dir).into_iter().map(|w| w.name).collect();
        assert_eq!(names, vec!["good".to_string()]);
        assert_eq!(std::fs::read_to_string(&broken).unwrap(), text);

        assert!(load_in(&dir, "bad").is_none());
        assert_eq!(std::fs::read_to_string(&broken).unwrap(), text);
    }

    #[test]
    fn saving_the_same_name_twice_overwrites_rather_than_piling_up() {
        let dir = temp_dir("overwrite");
        save_in(&dir, &sample("work")).unwrap();
        let mut updated = sample("work");
        updated.active_terminal = 0;
        updated.terminals.truncate(1);
        save_in(&dir, &updated).unwrap();

        let all = list_in(&dir);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].terminals.len(), 1);
    }

    fn shape(cols: u16) -> Shape {
        Shape {
            root: PathBuf::from("/somewhere"),
            cols,
            python: "python3".to_string(),
            workspace_view: Some("clee --watch-workspace /tmp/snaps".to_string()),
        }
    }

    /// A built-in workspace is always offered and can never be removed, which only holds if it
    /// never becomes a file: saving over one of their names is refused.
    #[test]
    fn the_built_in_workspaces_never_become_files() {
        let dir = temp_dir("builtin");
        assert!(is_built_in("Default layout") && is_built_in("default  LAYOUT"), "matched by slug");
        assert!(is_built_in("octave") && is_built_in("Octave") && is_built_in("pylab"));
        assert!(is_built_in("minimal") && is_built_in("Minimal"));
        // A workspace of your own called "default" is not one of them and must survive.
        assert!(!is_built_in("default") && !is_built_in("Defaults") && !is_built_in("octavelab"));

        let mut ws = sample(DEFAULT_NAME);
        assert!(save_in(&dir, &ws).is_err(), "saving over a built-in name is refused");
        // And the refusal names the one that was clashed with, not always the first.
        let clash = save_in(&dir, &sample("pylab")).unwrap_err();
        assert!(clash.contains("pylab"), "{clash}");
        assert!(list_in(&dir).is_empty());

        // Someone's own "default" is a different thing and stays listed.
        save_in(&dir, &sample("default")).expect("a workspace called default is ordinary");
        assert_eq!(list_in(&dir).len(), 1);

        // Any other name still saves normally.
        ws.name = "real".to_string();
        assert!(save_in(&dir, &ws).is_ok());
        assert_eq!(list_in(&dir).len(), 2);

        // The layout one carries no files or shells: picking it is about the shape of the
        // window, not about replacing what you have open.
        let built = built_in(DEFAULT_NAME, &shape(200)).unwrap();
        assert_eq!(built.root, PathBuf::from("/somewhere"));
        assert!(built.layout.show_sidebar && built.layout.show_terminal && !built.layout.split_view);
        assert!(built.terminals.is_empty() && built.open_files.is_empty());
        assert!(built_in("nothing of the sort", &shape(200)).is_none());
    }

    /// Each language preset is a prompt already running plus a shell beside it, in one window —
    /// a second *window* would take screen from the editor permanently to hold something used a
    /// minute at a time.
    #[test]
    fn a_language_preset_opens_a_prompt_and_a_shell_in_one_window() {
        for (name, expected) in [("octave", "octave --no-gui"), ("pylab", "python3")] {
            let ws = built_in(name, &shape(200)).unwrap();
            let tabs = &ws.terminals[0].tabs;
            assert_eq!(tabs.len(), 2, "{name}: the interpreter and a shell share a window");
            let start = tabs[0].startup_command.as_deref().unwrap();
            assert!(start.starts_with(expected), "{name}: starts {expected}, got {start}");
            assert_eq!(ws.terminals[0].active, 0, "{name}: the prompt is what you are looking at");
            assert_eq!(tabs[1].startup_command, None, "{name}: the shell starts nothing");
        }
    }

    /// Bare: no sidebar, no terminal, no menu bar, no terminals of its own — same shape as the
    /// default layout preset, just with every chrome flag off, since the point is the editor and
    /// nothing else around it.
    #[test]
    fn the_minimal_preset_is_layout_only_with_every_chrome_flag_off() {
        let ws = built_in("minimal", &shape(200)).unwrap();
        assert!(
            !ws.layout.show_sidebar && !ws.layout.show_terminal && !ws.layout.show_menubar,
            "every chrome flag is off: {:?}",
            ws.layout
        );
        assert!(ws.terminals.is_empty(), "no terminals of its own");
        assert!(ws.open_files.is_empty());
        assert_eq!(ws.root, PathBuf::from("/somewhere"));
        assert!(ws.drawer.is_none(), "a built-in has no opinion about the drawer");
    }

    /// A window, not a third tab, and that is the whole difference between watching your
    /// variables and asking about them: a tab is somewhere you go, a window is something you
    /// glance at.
    #[test]
    fn the_workspace_view_is_a_window_of_its_own() {
        let ws = built_in("octave", &shape(200)).unwrap();
        assert_eq!(ws.terminals.len(), 2, "the prompt's window and the workspace's");
        let view = &ws.terminals[1];
        assert_eq!(view.tabs.len(), 1, "nothing shares it");
        assert_eq!(view.tabs[0].name.as_deref(), Some("workspace"));
        assert!(view.tabs[0].startup_command.as_deref().unwrap().contains("--watch-workspace"));
        assert!(
            ws.terminals[0].weight > view.weight,
            "the prompt gets the larger share: it is where the work happens"
        );
    }

    /// Where CleeCode cannot name its own executable there is no viewer to start, and the preset
    /// is the prompt and the shell — rather than a window running a command that is not there.
    #[test]
    fn without_a_viewer_the_preset_is_still_a_preset() {
        let mut shape = shape(200);
        shape.workspace_view = None;
        let ws = built_in("pylab", &shape).unwrap();
        assert_eq!(ws.terminals.len(), 1);
        assert_eq!(ws.terminals[0].tabs.len(), 2);
    }

    /// A selected virtualenv has to reach the preset, or `pylab` opens a prompt without the
    /// packages the project was set up for — which is the one thing it exists to avoid.
    #[test]
    fn pylab_starts_the_python_it_was_given() {
        let mut shape = shape(200);
        shape.python = "/proj/.venv/bin/python3".to_string();
        let ws = built_in("pylab", &shape).unwrap();
        assert_eq!(
            ws.terminals[0].tabs[0].startup_command.as_deref(),
            Some("/proj/.venv/bin/python3")
        );
    }

    /// A layout that is right at 200 columns and unusable at 80 is not the best layout, it is
    /// the best layout for whoever wrote it — so the presets are checked at both ends.
    ///
    /// The prompt goes underneath at every width. It used to move beside the editor on a wide
    /// window, which reads well until a figure opens: the editor splits to put the plot next to
    /// the code, and three columns leave each half about a third of the window.
    #[test]
    fn a_preset_stacks_the_frames_at_every_width() {
        for cols in [200, 150, 90] {
            let layout = built_in("octave", &shape(cols)).unwrap().layout;
            assert!(!layout.terminal_on_right, "at {cols} columns the prompt goes underneath");
            assert!(layout.show_sidebar, "the tree survives either way");
            assert!(
                layout.terminal_pct < 50,
                "at {cols} columns the editor keeps the larger share of the height"
            );
        }
        // The tree gives its columns to the editor once there are enough to give.
        let wide = built_in("octave", &shape(200)).unwrap().layout;
        let narrow = built_in("octave", &shape(90)).unwrap().layout;
        assert!(wide.sidebar_width < narrow.sidebar_width);
    }

    /// The bug this closes was silent: a built-in answered before the disk was ever consulted,
    /// so a workspace somebody had saved under that name simply stopped opening.
    #[test]
    fn a_saved_workspace_wins_over_a_built_in_of_the_same_name() {
        let dir = temp_dir("shadow");
        // Written straight to the file, since saving under the name is refused — which is how
        // such a file can only be older than the built-in that now claims the name.
        let mut mine = sample("octave");
        mine.root = PathBuf::from("/mine");
        let text = toml::to_string_pretty(&mine).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("octave.toml"), text).unwrap();

        let (found, shadowed) = resolve_in(&dir, "octave", &shape(200));
        assert_eq!(found.unwrap().root, PathBuf::from("/mine"), "theirs opens, not the preset");
        assert_eq!(shadowed, Some("octave"), "and the caller is told, so it can say so");

        // With no file of that name, the built-in answers and nothing is shadowed.
        let (found, shadowed) = resolve_in(&dir, "pylab", &shape(200));
        assert_eq!(found.unwrap().terminals.len(), 2, "the preset, prompt and workspace both");
        assert_eq!(shadowed, None);

        // A name that is neither is neither.
        let (found, shadowed) = resolve_in(&dir, "nowhere", &shape(200));
        assert!(found.is_none() && shadowed.is_none());
    }

    #[test]
    fn slugs_are_stable_readable_filenames() {
        assert_eq!(slug("My Project"), "my-project");
        assert_eq!(slug("  rust/day-to-day  "), "rust-day-to-day");
        assert_eq!(slug("ML 3.12 (2)"), "ml-3-12-2");
        // Nothing usable left: still a valid filename rather than a bare ".toml".
        assert_eq!(slug("///"), "workspace");
        assert_eq!(slug(""), "workspace");
    }

    /// Files are hand-editable, so a minimal one (just a name, a root and a layout) must load
    /// instead of being dropped from the list.
    #[test]
    fn a_minimal_hand_written_file_loads() {
        let dir = temp_dir("minimal");
        let text = "name = \"bare\"\nroot = \"/work/project\"\n\n[layout]\n\
                    show_sidebar = true\nshow_terminal = true\nshow_menubar = true\n\
                    sidebar_width = 30\nterminal_pct = 35\nterminal_on_right = false\n\
                    split_view = false\nsplit_pct = 50\n";
        std::fs::write(dir.join("bare.toml"), text).unwrap();
        let ws = load_in(&dir, "bare").expect("minimal file must load");
        assert!(ws.open_files.is_empty() && ws.terminals.is_empty());
        assert_eq!(ws.active_terminal, 0);
        assert!(ws.drawer.is_none(), "a file that says nothing about a drawer describes none");
        assert!(ws.debuggee.is_none(), "and one that names no executable names none");
    }
}
