use std::fs;
use std::path::PathBuf;

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
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let is_dir = path.is_dir();
        FileNode {
            path,
            name,
            is_dir,
            expanded: false,
            loaded: false,
            children: Vec::new(),
        }
    }

    fn load_children(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let mut entries: Vec<PathBuf> = match fs::read_dir(&self.path) {
            Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.path()).collect(),
            Err(_) => Vec::new(),
        };
        entries.sort_by(|a, b| {
            let a_dir = a.is_dir();
            let b_dir = b.is_dir();
            match (a_dir, b_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase()
                    .cmp(&b.file_name().unwrap_or_default().to_string_lossy().to_lowercase()),
            }
        });
        self.children = entries.into_iter().map(FileNode::new).collect();
    }

    /// Expands the node (loading its children on first use). No-op if already expanded.
    fn expand(&mut self) {
        if !self.is_dir || self.expanded {
            return;
        }
        self.load_children();
        self.expanded = true;
    }
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
}

impl FileTree {
    pub fn new(root_path: PathBuf) -> Self {
        let mut root = FileNode::new(root_path);
        root.load_children();
        root.expanded = true;
        let mut tree = FileTree {
            root,
            selected: 0,
            visible: Vec::new(),
        };
        tree.rebuild_visible();
        tree
    }

    pub fn has_parent(&self) -> bool {
        self.root.path.parent().is_some()
    }

    pub fn parent_dir(&self) -> Option<PathBuf> {
        self.root.path.parent().map(|p| p.to_path_buf())
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
        Self::walk(&self.root, 0, &mut stack, &mut self.visible);
        if self.selected >= self.visible.len() && !self.visible.is_empty() {
            self.selected = self.visible.len() - 1;
        }
    }

    fn walk(node: &FileNode, depth: usize, path: &mut Vec<usize>, out: &mut Vec<VisibleEntry>) {
        // root itself is not shown as a row; only its children are shown starting at depth 0
        for (i, child) in node.children.iter().enumerate() {
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
                Self::walk(child, depth + 1, path, out);
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
        let mut tree = FileTree::new(dir.clone());
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
        let mut tree = FileTree::new(dir.clone());
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
        let tree = FileTree::new(dir.clone());
        assert!(tree.has_parent());
        assert!(tree.visible[0].is_up);
        assert_eq!(tree.parent_dir(), dir.parent().map(|p| p.to_path_buf()));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
