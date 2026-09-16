use std::fs;
use std::path::{Path, PathBuf};

pub struct FileNode {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub expanded: bool,
    pub loaded: bool,
    pub children: Vec<FileNode>,
}

impl FileNode {
    fn new(path: PathBuf) -> Self {
        let is_dir = path.is_dir();
        Self::of_kind(path, is_dir)
    }

    /// The same node, built where the caller has already been told what the path is. A listing
    /// knows that from the directory entry, and asking the disk again per node is a stat the
    /// answer to which is already in hand.
    fn of_kind(path: PathBuf, is_dir: bool) -> Self {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        FileNode {
            path,
            name,
            is_dir,
            expanded: false,
            loaded: false,
            children: Vec::new(),
        }
    }

    /// A directory's entries, folders first and then by name, with each entry's kind carried
    /// alongside it.
    ///
    /// The kind is read once per entry, off the listing, rather than asked of the disk from
    /// inside the comparator. A comparator runs O(n log n) times, so a directory of a thousand
    /// files used to cost ten thousand stats to put in order — and it is sorted again on every
    /// refresh, twice a second, for every folder left open.
    fn read_sorted_entries(dir: &std::path::Path) -> Vec<(PathBuf, bool)> {
        // The name it sorts under is folded once per entry too, for the same reason.
        let mut entries: Vec<(PathBuf, bool, String)> = match fs::read_dir(dir) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .map(|e| {
                    // `file_type` comes off the directory entry, so it is not another look at the
                    // disk — but it describes the link rather than its target, and a link to a
                    // folder should still open like one. Only then is the target asked about.
                    let is_dir = match e.file_type() {
                        Ok(kind) if !kind.is_symlink() => kind.is_dir(),
                        _ => e.path().is_dir(),
                    };
                    let key = e.file_name().to_string_lossy().to_lowercase();
                    (e.path(), is_dir, key)
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        entries.sort_by(|(_, a_dir, a_key), (_, b_dir, b_key)| match (a_dir, b_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a_key.cmp(b_key),
        });
        entries.into_iter().map(|(path, is_dir, _)| (path, is_dir)).collect()
    }

    fn load_children(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        self.children = Self::read_sorted_entries(&self.path)
            .into_iter()
            .map(|(path, is_dir)| FileNode::of_kind(path, is_dir))
            .collect();
    }

    /// Expands the node (loading its children on first use). No-op if already expanded.
    fn expand(&mut self) {
        if !self.is_dir || self.expanded {
            return;
        }
        self.load_children();
        self.expanded = true;
    }

    /// Re-reads this node's directory listing if it was already loaded, keeping
    /// existing children (and recursing into expanded ones) so external changes on
    /// disk (files created/removed by another process) show up without disturbing
    /// the user's current expand/collapse state.
    fn refresh(&mut self) {
        if !self.loaded {
            return;
        }
        let entries = Self::read_sorted_entries(&self.path);
        let mut old_children = std::mem::take(&mut self.children);
        let mut new_children = Vec::with_capacity(entries.len());
        for (path, is_dir) in entries {
            if let Some(pos) = old_children.iter().position(|c| c.path == path) {
                let mut kept = old_children.remove(pos);
                if kept.expanded {
                    kept.refresh();
                }
                new_children.push(kept);
            } else {
                new_children.push(FileNode::of_kind(path, is_dir));
            }
        }
        self.children = new_children;
    }
}

/// `path` written relative to `root`, when it is inside it — the question "is this folder part
/// of the project", answered with the way down to it.
///
/// Tried plainly first and then with both sides resolved, because two spellings of one folder are
/// ordinary on macOS: a shell sitting in `/tmp` reports `/private/tmp`, `/tmp` being a symlink to
/// it, and a plain prefix test would call the project's own folder somewhere else. Resolving
/// asks the disk, so it is the second question rather than the first.
pub fn relative_to(root: &Path, path: &Path) -> Option<PathBuf> {
    if let Ok(rest) = path.strip_prefix(root) {
        return Some(rest.to_path_buf());
    }
    let root = fs::canonicalize(root).ok()?;
    let path = fs::canonicalize(path).ok()?;
    path.strip_prefix(&root).ok().map(|rest| rest.to_path_buf())
}

pub struct VisibleEntry {
    pub depth: usize,
    pub name: String,
    pub is_dir: bool,
    pub expanded: bool,
    pub node_index: Vec<usize>,
    /// True for the synthetic ".." row used to walk up past the current root.
    pub is_up: bool,
}

/// What Enter should do with the currently selected row.
pub enum Activation {
    OpenFile(PathBuf),
    SetRoot(PathBuf),
    NavigateUp,
}

pub struct FileTree {
    pub root: FileNode,
    pub selected: usize,
    pub visible: Vec<VisibleEntry>,
    pub show_hidden: bool,
}

impl FileTree {
    /// `show_hidden` is a required argument rather than a default the caller patches
    /// afterwards: the tree is rebuilt from several places (root change, refresh after a file
    /// operation) and any one of them forgetting to reapply the preference made hidden files
    /// silently reappear.
    pub fn new(root_path: PathBuf, show_hidden: bool) -> Self {
        let mut root = FileNode::new(root_path);
        root.load_children();
        root.expanded = true;
        let mut tree = FileTree {
            root,
            selected: 0,
            visible: Vec::new(),
            show_hidden,
        };
        tree.rebuild_visible();
        tree
    }

    /// Re-reads directory contents on disk for the root and every currently
    /// expanded/loaded folder, then recomputes the visible row list.
    pub fn refresh(&mut self) {
        self.root.refresh();
        self.rebuild_visible();
    }

    /// Absolute path for each row in `visible`, in the same order (None for the
    /// synthetic ".." row). Lets callers look up per-file annotations (e.g. git status)
    /// without holding two overlapping borrows of `FileTree` at once.
    pub fn visible_paths(&self) -> Vec<Option<PathBuf>> {
        self.visible
            .iter()
            .map(|entry| if entry.is_up { None } else { Some(self.node_at(&entry.node_index).path.clone()) })
            .collect()
    }

    pub fn has_parent(&self) -> bool {
        self.parent_dir().is_some()
    }

    /// The folder one level up, or None at the top — and None, too, for a root whose spelling has
    /// no level up to name. `Path::parent` answers the empty path for a relative root of a single
    /// component (`.`, `src`), which is not a directory anyone can read: offered as a ".." row it
    /// gave a tree with no root and no way back. Asked through here by `has_parent`, so the row
    /// only ever appears where it leads somewhere.
    pub fn parent_dir(&self) -> Option<PathBuf> {
        self.root
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf())
    }

    pub fn rebuild_visible(&mut self) {
        self.visible.clear();
        if self.has_parent() {
            self.visible.push(VisibleEntry {
                depth: 0,
                name: "..".to_string(),
                is_dir: true,
                expanded: false,
                node_index: Vec::new(),
                is_up: true,
            });
        }
        let mut stack: Vec<usize> = Vec::new();
        Self::walk(&self.root, 0, &mut stack, &mut self.visible, self.show_hidden);
        if self.selected >= self.visible.len() && !self.visible.is_empty() {
            self.selected = self.visible.len() - 1;
        }
    }

    /// Applies the caller's preference, rather than flipping a copy of it, so the tree can't
    /// end up disagreeing with the setting it is supposed to reflect.
    pub fn set_show_hidden(&mut self, show_hidden: bool) {
        self.show_hidden = show_hidden;
        self.rebuild_visible();
    }

    /// Opens the tree down to `path` and puts the selection on it, so somewhere inside the
    /// project can be shown without the root moving.
    ///
    /// This is the cheap half of "go there". Changing the root is a change of *project* —
    /// `App::set_root` reloads the project's settings, restarts its git baseline and lets go of
    /// the workspace — and none of that has any business happening because somebody wanted to
    /// look in `src/`. Revealing touches only which rows are on screen, and is undone by
    /// collapsing them again.
    ///
    /// Answers whether it landed on the path itself. It can fail to: a folder whose name starts
    /// with a dot has no row at all while hidden files are off, and the honest thing then is to
    /// stop at the nearest ancestor that *is* on screen and say so, rather than to move the
    /// selection somewhere arbitrary and report success.
    pub fn reveal(&mut self, path: &Path) -> bool {
        let Some(rest) = relative_to(&self.root.path, path) else {
            return false;
        };
        let mut index: Vec<usize> = Vec::new();
        for name in rest.iter() {
            // Walked from the top each time rather than held as a borrow: the depth of a path is
            // a handful of steps, and the alternative is a mutable borrow that has to outlive
            // the loop it is being reseated in.
            let parent = self.node_at_mut(&index);
            // A folder on the way down has to be open for the one below it to exist as a node.
            // Already-open folders and files alike take this as a no-op.
            parent.expand();
            let Some(at) = parent.children.iter().position(|child| child.path.file_name() == Some(name)) else {
                // The path names something this tree does not have — deleted since, or never
                // there. What was opened on the way stays open; it is a folder of the project
                // either way.
                self.rebuild_visible();
                return false;
            };
            index.push(at);
        }
        // The folder that was asked for is opened too, because revealing a folder and not
        // showing what is in it answers a question nobody asked.
        let target = self.node_at_mut(&index);
        if target.is_dir {
            target.expand();
        }
        self.rebuild_visible();
        if let Some(row) = self.row_of(&index) {
            self.selected = row;
            return true;
        }
        // Hidden, and hidden files are off. Land as close to it as the tree can actually show.
        while index.pop().is_some() {
            if let Some(row) = self.row_of(&index) {
                self.selected = row;
                break;
            }
        }
        false
    }

    /// Which visible row a node sits on, if it is on screen at all.
    fn row_of(&self, index: &[usize]) -> Option<usize> {
        if index.is_empty() {
            return None;
        }
        self.visible.iter().position(|entry| !entry.is_up && entry.node_index == index)
    }

    fn walk(node: &FileNode, depth: usize, path: &mut Vec<usize>, out: &mut Vec<VisibleEntry>, show_hidden: bool) {
        // root itself is not shown as a row; only its children are shown starting at depth 0
        for (i, child) in node.children.iter().enumerate() {
            if !show_hidden && child.name.starts_with('.') {
                continue;
            }
            path.push(i);
            out.push(VisibleEntry {
                depth,
                name: child.name.clone(),
                is_dir: child.is_dir,
                expanded: child.expanded,
                node_index: path.clone(),
                is_up: false,
            });
            if child.is_dir && child.expanded {
                Self::walk(child, depth + 1, path, out, show_hidden);
            }
            path.pop();
        }
    }

    fn node_at_mut(&mut self, index: &[usize]) -> &mut FileNode {
        let mut node = &mut self.root;
        for &i in index {
            node = &mut node.children[i];
        }
        node
    }

    fn node_at(&self, index: &[usize]) -> &FileNode {
        let mut node = &self.root;
        for &i in index {
            node = &node.children[i];
        }
        node
    }

    /// Path of the currently selected row, or None if nothing real is selected (e.g. the ".." row).
    pub fn selected_path(&self) -> Option<PathBuf> {
        let entry = self.visible.get(self.selected)?;
        if entry.is_up {
            return None;
        }
        Some(self.node_at(&entry.node_index).path.clone())
    }

    /// Directory new items (e.g. drag-and-dropped files) should land in: the selected
    /// directory itself, the parent of a selected file, or the project root as a fallback.
    pub fn selected_dir(&self) -> PathBuf {
        let Some(entry) = self.visible.get(self.selected) else {
            return self.root.path.clone();
        };
        if entry.is_up {
            return self.root.path.clone();
        }
        let node = self.node_at(&entry.node_index);
        if entry.is_dir {
            node.path.clone()
        } else {
            node.path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| self.root.path.clone())
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.visible.is_empty() {
            return;
        }
        let len = self.visible.len() as isize;
        let mut new_sel = self.selected as isize + delta;
        if new_sel < 0 {
            new_sel = 0;
        } else if new_sel >= len {
            new_sel = len - 1;
        }
        self.selected = new_sel as usize;
    }

    /// Right arrow: expands the selected directory. Never collapses or toggles.
    pub fn expand_selected(&mut self) {
        let Some(entry) = self.visible.get(self.selected) else { return };
        if entry.is_up || !entry.is_dir || entry.expanded {
            return;
        }
        let entry_index = entry.node_index.clone();
        self.node_at_mut(&entry_index).expand();
        self.rebuild_visible();
    }

    /// Expands a collapsed folder or collapses an expanded one — what a single mouse click on
    /// a folder does. Unlike `collapse_selected` it never walks the selection up to the
    /// parent: a click should only affect the row that was clicked.
    pub fn toggle_selected(&mut self) {
        let Some(entry) = self.visible.get(self.selected) else { return };
        if entry.is_up || !entry.is_dir {
            return;
        }
        let was_expanded = entry.expanded;
        let entry_index = entry.node_index.clone();
        if was_expanded {
            self.node_at_mut(&entry_index).expanded = false;
        } else {
            self.node_at_mut(&entry_index).expand();
        }
        self.rebuild_visible();
    }

    /// Enter: open a file, make a directory the new tree root, or walk up via "..".
    pub fn activate_selected(&mut self) -> Option<Activation> {
        let entry = self.visible.get(self.selected)?;
        if entry.is_up {
            return Some(Activation::NavigateUp);
        }
        let node = self.node_at(&entry.node_index);
        if entry.is_dir {
            Some(Activation::SetRoot(node.path.clone()))
        } else {
            Some(Activation::OpenFile(node.path.clone()))
        }
    }

    /// Left arrow: collapses the selected directory, or jumps to its parent row.
    pub fn collapse_selected(&mut self) {
        let Some(entry) = self.visible.get(self.selected) else { return };
        if entry.is_up {
            return;
        }
        if entry.is_dir && entry.expanded {
            let entry_index = entry.node_index.clone();
            let node = self.node_at_mut(&entry_index);
            node.expanded = false;
            self.rebuild_visible();
        } else if entry.depth > 0 {
            // move selection to parent directory row
            let parent_depth = entry.depth - 1;
            let mut i = self.selected;
            while i > 0 {
                i -= 1;
                if self.visible[i].depth == parent_depth {
                    self.selected = i;
                    break;
                }
            }
        }
    }
}

/// How many names the shell half will list. A folder can hold thousands — `node_modules`, a
/// build directory — and reading all of them on every `cd`, to draw a pane eight rows tall, is a
/// cost nobody asked for. What is past the cap is counted and said, never silently dropped.
const SHELL_LIST_MAX: usize = 500;

/// One row of the shell half of the sidebar.
pub struct ShellRow {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    /// True for the ".." row, which is how walking up looks here as well.
    pub is_up: bool,
}

/// What one folder holds, listed flat: the sidebar's answer to the `ls` somebody types to see
/// where they are.
///
/// Not a `FileTree` with a single level open, and deliberately not. A tree carries which of its
/// folders are expanded, and that state has no meaning in a pane whose folder moves out from
/// under it — every `cd` would invalidate it, and the pane would spend its life forgetting
/// things the user never opened. So this holds exactly what the question is worth: the names in
/// one folder, folders first, re-read when the folder changes.
pub struct ShellList {
    /// The folder being shown, or `None` while no shell has said where it is.
    pub dir: Option<PathBuf>,
    pub rows: Vec<ShellRow>,
    pub selected: usize,
    /// How many names the folder held beyond the ones listed. See `SHELL_LIST_MAX`.
    pub overflow: usize,
    show_hidden: bool,
}

impl ShellList {
    /// `show_hidden` is required rather than defaulted for the same reason `FileTree::new`
    /// requires it: the preference is one thing, and two panes disagreeing about it is a bug
    /// that looks like a missing file.
    pub fn new(show_hidden: bool) -> Self {
        ShellList {
            dir: None,
            rows: Vec::new(),
            selected: 0,
            overflow: 0,
            show_hidden,
        }
    }

    /// Shows another folder, from the top. The selection does not travel: it belonged to the
    /// folder that has just been left.
    pub fn show(&mut self, dir: PathBuf) {
        self.dir = Some(dir);
        self.selected = 0;
        self.reread();
    }

    /// Re-reads the folder in place, keeping the selection where it can. What this catches is
    /// the same thing the tree's refresh catches: files another process wrote while the pane sat
    /// there, which for this pane is most of what it is for.
    pub fn refresh(&mut self) {
        self.reread();
    }

    pub fn set_show_hidden(&mut self, show_hidden: bool) {
        self.show_hidden = show_hidden;
        self.reread();
    }

    fn reread(&mut self) {
        self.rows.clear();
        self.overflow = 0;
        let Some(dir) = self.dir.clone() else {
            self.selected = 0;
            return;
        };
        // The way up, offered the way the tree offers it — and only where it leads somewhere.
        if let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) {
            self.rows.push(ShellRow {
                name: "..".to_string(),
                path: parent.to_path_buf(),
                is_dir: true,
                is_up: true,
            });
        }
        for (path, is_dir) in FileNode::read_sorted_entries(&dir) {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string());
            if !self.show_hidden && name.starts_with('.') {
                continue;
            }
            if self.rows.len() >= SHELL_LIST_MAX {
                self.overflow += 1;
                continue;
            }
            self.rows.push(ShellRow {
                name,
                path,
                is_dir,
                is_up: false,
            });
        }
        if self.selected >= self.rows.len() {
            self.selected = self.rows.len().saturating_sub(1);
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() as isize - 1;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
    }

    pub fn selected_row(&self) -> Option<&ShellRow> {
        self.rows.get(self.selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("clicode_tree_test_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        std::fs::write(dir.join("sub").join("b.txt"), "b").unwrap();
        dir
    }

    /// Revealing walks the tree open to a folder deeper in the project and lands the selection
    /// on it — without the root moving, which is the whole point of it existing.
    #[test]
    fn revealing_opens_the_way_down_and_leaves_the_root_alone() {
        let dir = setup_dir("reveal_down");
        std::fs::create_dir_all(dir.join("sub").join("deep")).unwrap();
        std::fs::write(dir.join("sub").join("deep").join("c.txt"), "c").unwrap();
        let mut tree = FileTree::new(dir.clone(), true);

        assert!(tree.reveal(&dir.join("sub").join("deep")));
        assert_eq!(tree.root.path, dir, "revealing is not a change of project");
        let row = &tree.visible[tree.selected];
        assert_eq!(row.name, "deep");
        assert!(row.expanded, "the folder revealed shows what is in it");
        assert_eq!(tree.selected_path().as_deref(), Some(dir.join("sub").join("deep").as_path()));
    }

    /// A file is revealed the same way a folder is: what you asked to see is the row, and the
    /// folders above it are only the way there.
    #[test]
    fn revealing_a_file_selects_the_file() {
        let dir = setup_dir("reveal_file");
        let mut tree = FileTree::new(dir.clone(), true);
        assert!(tree.reveal(&dir.join("sub").join("b.txt")));
        assert_eq!(tree.visible[tree.selected].name, "b.txt");
    }

    /// Outside the project there is nothing to reveal — that is `set_root`'s errand, and it is a
    /// different one. Nothing moves, and the answer says so.
    #[test]
    fn revealing_refuses_anything_outside_the_project() {
        let dir = setup_dir("reveal_outside");
        let mut tree = FileTree::new(dir.join("sub"), true);
        let before = tree.selected;
        assert!(!tree.reveal(&dir));
        assert!(!tree.reveal(std::path::Path::new("/definitely/not/here")));
        assert_eq!(tree.root.path, dir.join("sub"));
        assert_eq!(tree.selected, before);
    }

    /// A folder whose name starts with a dot has no row while hidden files are off. Revealing
    /// into one lands on the nearest row that is actually on screen and reports that it did not
    /// get all the way — the alternative is a selection somewhere arbitrary, reported as success.
    #[test]
    fn revealing_into_a_hidden_folder_stops_where_the_tree_can_show_it() {
        let dir = setup_dir("reveal_hidden");
        std::fs::create_dir_all(dir.join("sub").join(".secret")).unwrap();
        let mut tree = FileTree::new(dir.clone(), false);

        assert!(!tree.reveal(&dir.join("sub").join(".secret")));
        assert_eq!(tree.visible[tree.selected].name, "sub", "as close as it can be shown");

        // With hidden files on, the same call arrives.
        tree.set_show_hidden(true);
        assert!(tree.reveal(&dir.join("sub").join(".secret")));
        assert_eq!(tree.visible[tree.selected].name, ".secret");
    }

    /// The shell half lists one folder flat, folders first, with the way up offered the way the
    /// tree offers it — and it obeys the hidden-files preference like everything else.
    #[test]
    fn the_shell_half_lists_one_folder_the_way_ls_does() {
        let dir = setup_dir("shell_list");
        std::fs::write(dir.join(".rc"), "x").unwrap();
        let mut list = ShellList::new(false);
        list.show(dir.clone());

        let names: Vec<&str> = list.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["..", "sub", "a.txt"]);
        assert!(list.rows[0].is_up);
        assert_eq!(list.rows[0].path, dir.parent().unwrap());

        list.set_show_hidden(true);
        let names: Vec<&str> = list.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["..", "sub", ".rc", "a.txt"]);
    }

    /// Moving to another folder is a fresh listing, not the old one scrolled: the selection
    /// belonged to the folder that has been left.
    #[test]
    fn the_shell_half_starts_from_the_top_in_a_new_folder() {
        let dir = setup_dir("shell_moves");
        let mut list = ShellList::new(true);
        list.show(dir.clone());
        list.move_selection(2);
        assert_eq!(list.selected, 2);

        list.show(dir.join("sub"));
        assert_eq!(list.selected, 0);
        assert_eq!(list.selected_row().map(|r| r.name.as_str()), Some(".."));
        assert_eq!(list.dir.as_deref(), Some(dir.join("sub").as_path()));
    }

    /// A folder with more names than the pane will list says how many it is not showing, rather
    /// than quietly ending the list early.
    #[test]
    fn a_crowded_folder_says_how_much_it_is_not_showing() {
        let dir = setup_dir("shell_crowded");
        let many = dir.join("many");
        std::fs::create_dir_all(&many).unwrap();
        for i in 0..SHELL_LIST_MAX + 10 {
            std::fs::write(many.join(format!("f{i:04}")), "x").unwrap();
        }
        let mut list = ShellList::new(true);
        list.show(many);
        assert_eq!(list.rows.len(), SHELL_LIST_MAX);
        assert_eq!(list.overflow, 11, "the ten past the cap, plus the one the way-up row took");
    }

    /// A single click on a folder toggles it, and must not move the selection the way
    /// `collapse_selected` does when a row isn't an expanded folder.
    #[test]
    fn toggle_selected_opens_and_closes_a_folder_in_place() {
        let dir = setup_dir("toggle_folder");
        let mut tree = FileTree::new(dir.clone(), true);
        let off = up_row_offset(&tree);
        tree.selected = off; // the "sub" folder
        assert_eq!(tree.visible[off].name, "sub");

        tree.toggle_selected();
        assert_eq!(tree.visible[off + 1].name, "b.txt", "should have expanded");
        assert_eq!(tree.selected, off, "selection must stay on the clicked row");

        tree.toggle_selected();
        assert_eq!(tree.visible[off + 1].name, "a.txt", "should have collapsed again");
        assert_eq!(tree.selected, off);

        // A file row is not a folder: toggling it does nothing at all.
        tree.selected = off + 1;
        let before = tree.visible.len();
        tree.toggle_selected();
        assert_eq!(tree.visible.len(), before);
        assert_eq!(tree.selected, off + 1);
    }

    /// Hidden files must stay hidden across every operation that rebuilds the tree — a
    /// refresh, or navigating to another folder — which is what made them reappear before.
    #[test]
    fn hidden_files_stay_hidden_across_rebuilds() {
        let dir = setup_dir("hidden_persists");
        std::fs::create_dir_all(dir.join(".hidden_dir")).unwrap();
        std::fs::write(dir.join(".hidden_file"), "x").unwrap();

        let mut tree = FileTree::new(dir.clone(), false);
        let names = |t: &FileTree| t.visible.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
        assert!(!names(&tree).iter().any(|n| n.starts_with('.') && n != ".."), "{:?}", names(&tree));

        tree.refresh();
        assert!(!names(&tree).iter().any(|n| n.starts_with('.') && n != ".."), "refresh revealed them");

        // Descending into a subfolder is a fresh tree, and it must inherit the preference.
        let sub = FileTree::new(dir.join("sub"), false);
        assert!(!names(&sub).iter().any(|n| n.starts_with('.') && n != ".."));

        // And the toggle still works, in both directions.
        tree.set_show_hidden(true);
        assert!(names(&tree).contains(&".hidden_file".to_string()));
        tree.set_show_hidden(false);
        assert!(!names(&tree).contains(&".hidden_file".to_string()));
    }

    fn up_row_offset(tree: &FileTree) -> usize {
        if tree.has_parent() {
            1
        } else {
            0
        }
    }

    #[test]
    fn lists_dirs_before_files_and_expands() {
        let dir = setup_dir("lists_dirs");
        let mut tree = FileTree::new(dir.clone(), true);
        let off = up_row_offset(&tree);
        assert_eq!(tree.visible.len(), 2 + off);
        assert!(tree.visible[off].is_dir);
        assert_eq!(tree.visible[off].name, "sub");
        assert_eq!(tree.visible[off + 1].name, "a.txt");

        // Right expands "sub" without opening/rerooting
        tree.selected = off;
        tree.expand_selected();
        assert_eq!(tree.visible.len(), 3 + off);
        assert_eq!(tree.visible[off + 1].name, "b.txt");
        assert_eq!(tree.visible[off + 1].depth, 1);

        // Enter on a file opens it
        tree.selected = off + 1;
        match tree.activate_selected() {
            Some(Activation::OpenFile(p)) => assert_eq!(p, dir.join("sub").join("b.txt")),
            _ => panic!("expected OpenFile"),
        }

        // Enter on a directory asks to make it the new root
        tree.selected = off;
        match tree.activate_selected() {
            Some(Activation::SetRoot(p)) => assert_eq!(p, dir.join("sub")),
            _ => panic!("expected SetRoot"),
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn collapse_resets_visible_list() {
        let dir = setup_dir("collapse");
        let mut tree = FileTree::new(dir.clone(), true);
        let off = up_row_offset(&tree);
        tree.selected = off;
        tree.expand_selected();
        assert_eq!(tree.visible.len(), 3 + off);
        tree.selected = off;
        tree.collapse_selected();
        assert_eq!(tree.visible.len(), 2 + off);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn up_row_present_when_not_at_filesystem_root() {
        let dir = setup_dir("up_row");
        let tree = FileTree::new(dir.clone(), true);
        assert!(tree.has_parent());
        assert!(tree.visible[0].is_up);
        assert_eq!(tree.parent_dir(), dir.parent().map(|p| p.to_path_buf()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `.` and a bare folder name have the empty path as their parent, which reads as no
    /// directory at all. The row offering to walk up there must not be drawn: following it
    /// emptied the drawer and left no way back into the project.
    #[test]
    fn no_up_row_for_a_relative_root_with_nothing_above_it() {
        for root in [".", "./", "src"] {
            let tree = FileTree::new(PathBuf::from(root), true);
            assert!(!tree.has_parent(), "{root:?} claims a parent");
            assert_eq!(tree.parent_dir(), None, "{root:?}");
            assert!(!tree.visible.iter().any(|e| e.is_up), "{root:?} drew a \"..\" row");
            assert!(!tree.visible.is_empty(), "{root:?} listed nothing");
        }
    }
}
