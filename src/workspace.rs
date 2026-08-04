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
    #[serde(default)]
    pub active_terminal: usize,
    pub layout: WorkspaceLayout,
    #[serde(default)]
    pub terminals: Vec<WorkspaceTerminal>,
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
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let text = toml::to_string_pretty(ws).map_err(|e| e.to_string())?;
    let path = file_in(dir, &ws.name);
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn load_in(dir: &Path, name: &str) -> Option<Workspace> {
    let text = std::fs::read_to_string(file_in(dir, name)).ok()?;
    toml::from_str(&text).ok()
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
    }
}
