use crate::menu::MenuAction;
use std::path::PathBuf;

/// What a picked entry does when confirmed.
pub enum PickAction {
    Command(MenuAction),
    OpenFile(PathBuf),
    /// A directory in the venv browser: registered if it's a venv, otherwise descended into.
    VenvDir(PathBuf),
    /// A saved workspace, by name: opened, or deleted, depending on the picker's kind.
    Workspace(String),
    /// A variable in a live session, to look inside.
    Inspect(String),
    /// A line found by the project search: the file to open and where in it to land. The column
    /// travels with the line so the cursor arrives on the word rather than beside it.
    FileLine(PathBuf, usize, usize),
}

pub struct PickItem {
    pub label: String,
    /// Keyboard shortcut shown right-aligned, so the command palette doubles as a
    /// cheatsheet. Not part of the fuzzy-matched text.
    pub shortcut: Option<String>,
    pub action: PickAction,
}

/// Which chooser this is, so the owner knows whose list to rebuild as the query changes.
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum PickerKind {
    Commands,
    Files,
    /// Browsing the disk for a directory to register as a venv.
    VenvBrowse,
    /// Saved workspaces, to open one.
    Workspaces,
    /// The same list, but Enter deletes instead of opening. A separate kind rather than a flag
    /// so nothing can confuse the two — one of them destroys a file.
    WorkspaceDelete,
    /// Lines found across the project. The list is fixed once the search is done, and the query
    /// typed here narrows it further — which is how a search that found too much is salvaged
    /// without running it again.
    SearchResults,
    /// The names a live interpreter is holding, to pick one to look inside.
    Variables,
    /// Everywhere the language server says a name is used. A fixed list, like the search
    /// results, and narrowed the same way — which is what makes a name used in eighty places
    /// still usable: type the file you meant.
    References,
    /// What the open file contains, in document order. The outline is a chooser and not a pane:
    /// it is opened to go somewhere, and it closes when you get there.
    Symbols,
    /// Everything the servers have said is wrong, across the files that are open.
    Diagnostics,
}

/// A fuzzy-filtered chooser shared by the command palette and the file quick-open. Holds
/// the full item list plus the indices (best-scoring first) that match the current query.
pub struct Picker {
    pub title: &'static str,
    pub kind: PickerKind,
    pub query: String,
    pub items: Vec<PickItem>,
    pub filtered: Vec<usize>,
    pub selected: usize,
    /// Matched against instead of `query` when they differ. The file picker browsing the disk
    /// types a whole path, of which only the trailing fragment should filter the listing.
    pub filter_override: Option<String>,
    /// True while the query is being read as a filesystem path rather than a project-file
    /// search, so the owner only rebuilds the list when the mode actually flips.
    pub path_mode: bool,
    /// Where this list was asked from, for the lists that are asked from somewhere in
    /// particular: confirming a row pushes it onto the jump stack, so the key that comes back
    /// from a definition also comes back from a reference.
    ///
    /// Held here rather than on the application because it dies with the picker. A slot beside
    /// the others would outlive the list it belongs to, and the next Enter in an unrelated
    /// chooser would jump back to somewhere nobody had left.
    pub origin: Option<(PathBuf, usize, usize)>,
}

impl Picker {
    pub fn new(title: &'static str, kind: PickerKind, items: Vec<PickItem>) -> Self {
        let mut p = Picker {
            title,
            kind,
            query: String::new(),
            items,
            filtered: Vec::new(),
            selected: 0,
            filter_override: None,
            path_mode: false,
            origin: None,
        };
        p.refilter();
        p
    }

    /// Swaps the candidate list, keeping the query, for a picker whose contents depend on what
    /// has been typed (browsing directories).
    pub fn set_items(&mut self, items: Vec<PickItem>) {
        self.items = items;
        self.refilter();
    }

    pub fn refilter(&mut self) {
        let needle = self.filter_override.clone().unwrap_or_else(|| self.query.clone());
        if needle.is_empty() {
            self.filtered = (0..self.items.len()).collect();
        } else {
            let mut scored: Vec<(i32, usize)> = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(i, it)| fuzzy_score(&needle, &it.label).map(|s| (s, i)))
                .collect();
            // Higher score first; ties keep original order for stability.
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            self.filtered = scored.into_iter().map(|(_, i)| i).collect();
        }
        self.selected = 0;
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let len = self.filtered.len() as isize;
        let mut idx = self.selected as isize + delta;
        idx = ((idx % len) + len) % len;
        self.selected = idx as usize;
    }

    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.refilter();
    }

    pub fn selected_action(&self) -> Option<&PickAction> {
        self.filtered.get(self.selected).and_then(|&i| self.items.get(i)).map(|it| &it.action)
    }
}

/// Case-insensitive subsequence fuzzy score, or None if `query` isn't a subsequence of
/// `text`. Rewards consecutive matches and matches at word starts so "of" ranks
/// "Open file" above an incidental "o…f…" deeper in a string.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    let q: Vec<char> = query.to_lowercase().chars().collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let mut qi = 0usize;
    let mut score = 0i32;
    let mut last_match: Option<usize> = None;
    for (ti, &tc) in t.iter().enumerate() {
        if qi < q.len() && tc == q[qi] {
            score += 1;
            if let Some(lm) = last_match {
                if ti == lm + 1 {
                    score += 3; // consecutive
                }
            }
            if ti == 0 || !t[ti - 1].is_alphanumeric() {
                score += 2; // word start
            }
            last_match = Some(ti);
            qi += 1;
        }
    }
    if qi == q.len() {
        // Mild preference for shorter labels among equal matches.
        Some(score - (t.len() as i32) / 20)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsequence_matches_and_non_matches() {
        assert!(fuzzy_score("of", "Open file").is_some());
        assert!(fuzzy_score("xyz", "Open file").is_none());
    }

    #[test]
    fn consecutive_and_word_start_rank_higher() {
        let contiguous = fuzzy_score("open", "Open file").unwrap();
        let scattered = fuzzy_score("open", "on pen").unwrap();
        assert!(contiguous > scattered);
    }

    #[test]
    fn refilter_orders_by_score() {
        let items = vec![
            PickItem { label: "File: Save".into(), shortcut: None, action: PickAction::Command(MenuAction::Save) },
            PickItem { label: "File: Save All".into(), shortcut: None, action: PickAction::Command(MenuAction::SaveAll) },
            PickItem { label: "Edit: Copy".into(), shortcut: None, action: PickAction::Command(MenuAction::Copy) },
        ];
        let mut p = Picker::new("Commands", PickerKind::Commands, items);
        p.query = "save".into();
        p.refilter();
        // Both "Save" entries match; "Copy" doesn't.
        assert_eq!(p.filtered.len(), 2);
    }
}
