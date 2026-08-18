use crate::i18n::Key;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    ToggleSidebar,
    ToggleTerminal,
    NewTerminalTab,
    CloseTerminalTab,
    RenameTerminal,
    ToggleMenuBar,
    OpenMenuBar,
    ColumnSelection,
    OpenSettings,
    SaveAll,
    NewTerminal,
    CloseTerminal,
    Save,
    SaveAs,
    RunTarget,
    Quit,
    ShowAbout,
    Copy,
    Cut,
    Paste,
    SelectAll,
    Indent,
    Outdent,
    ToggleFold,
    CloseFile,
    NextTab,
    PrevTab,
    NextTerminal,
    PrevTerminal,
    LayoutClassic,
    LayoutWide,
    LayoutTriple,
    ToggleTerminalSide,
    ToggleResizeMode,
    RunFile,
    ToggleSplitView,
    ToggleHiddenFiles,
    Undo,
    Redo,
    ToggleComment,
    DuplicateLine,
    MoveLineUp,
    MoveLineDown,
    Find,
    GotoLine,
    SearchProject,
    NewFile,
    NewFolder,
    CommandPalette,
    OpenFilePicker,
    Rename,
    Delete,
    NextTerminalTab,
    PrevTerminalTab,
    SaveWorkspace,
    OpenWorkspace,
    DeleteWorkspace,
    ShowManual,
    FocusFileTree,
    FocusEditor,
    FocusTerminal,
}

impl MenuAction {
    /// Every action the app can perform. The palette is built from the menus, and
    /// `every_action_is_reachable` checks this list against it — so an action that exists but
    /// sits in no menu (as `RenameTerminal` once did, reachable only by right-click) fails the
    /// test instead of quietly being mouse-only.
    #[allow(dead_code, reason = "the checklist the reachability tests are written against")]
    pub const ALL: &'static [MenuAction] = &[
        MenuAction::ToggleSidebar,
        MenuAction::ToggleTerminal,
        MenuAction::NewTerminalTab,
        MenuAction::CloseTerminalTab,
        MenuAction::RenameTerminal,
        MenuAction::ToggleMenuBar,
        MenuAction::OpenMenuBar,
        MenuAction::ColumnSelection,
        MenuAction::OpenSettings,
        MenuAction::SaveAll,
        MenuAction::NewTerminal,
        MenuAction::CloseTerminal,
        MenuAction::Save,
        MenuAction::SaveAs,
        MenuAction::RunTarget,
        MenuAction::Quit,
        MenuAction::ShowAbout,
        MenuAction::Copy,
        MenuAction::Cut,
        MenuAction::Paste,
        MenuAction::SelectAll,
        MenuAction::Indent,
        MenuAction::Outdent,
        MenuAction::ToggleFold,
        MenuAction::CloseFile,
        MenuAction::NextTab,
        MenuAction::PrevTab,
        MenuAction::NextTerminal,
        MenuAction::PrevTerminal,
        MenuAction::LayoutClassic,
        MenuAction::LayoutWide,
        MenuAction::LayoutTriple,
        MenuAction::ToggleTerminalSide,
        MenuAction::ToggleResizeMode,
        MenuAction::RunFile,
        MenuAction::ToggleSplitView,
        MenuAction::ToggleHiddenFiles,
        MenuAction::Undo,
        MenuAction::Redo,
        MenuAction::ToggleComment,
        MenuAction::DuplicateLine,
        MenuAction::MoveLineUp,
        MenuAction::MoveLineDown,
        MenuAction::Find,
        MenuAction::GotoLine,
        MenuAction::SearchProject,
        MenuAction::NewFile,
        MenuAction::NewFolder,
        MenuAction::CommandPalette,
        MenuAction::OpenFilePicker,
        MenuAction::Rename,
        MenuAction::Delete,
        MenuAction::NextTerminalTab,
        MenuAction::PrevTerminalTab,
        MenuAction::SaveWorkspace,
        MenuAction::OpenWorkspace,
        MenuAction::DeleteWorkspace,
        MenuAction::ShowManual,
        MenuAction::FocusFileTree,
        MenuAction::FocusEditor,
        MenuAction::FocusTerminal,
    ];
}

pub struct MenuItemDef {
    pub label_key: Key,
    pub action: MenuAction,
    /// Keyboard shortcut hint shown right-aligned in the dropdown, if any.
    /// Key names, not human language, so this is never translated.
    pub shortcut: Option<&'static str>,
    /// When true, a separator rule is drawn above this item, opening a new
    /// visual group. Purely cosmetic: the item stays selectable and keyboard
    /// navigation is unaffected.
    pub new_group: bool,
}

pub struct MenuDef {
    pub title_key: Key,
    pub items: Vec<MenuItemDef>,
}

fn item(label_key: Key, action: MenuAction, shortcut: Option<&'static str>) -> MenuItemDef {
    MenuItemDef { label_key, action, shortcut, new_group: false }
}

/// Like `item`, but marks the start of a new group so a separator rule is drawn above it.
fn group(label_key: Key, action: MenuAction, shortcut: Option<&'static str>) -> MenuItemDef {
    MenuItemDef { label_key, action, shortcut, new_group: true }
}

pub fn menu_defs() -> Vec<MenuDef> {
    vec![
        MenuDef {
            title_key: Key::MenuCleeCode,
            items: vec![
                item(Key::ItemCommandPalette, MenuAction::CommandPalette, Some("Ctrl+P")),
                item(Key::ItemOpenSettings, MenuAction::OpenSettings, Some("Ctrl+Shift+O")),
                group(Key::ItemQuit, MenuAction::Quit, Some("Ctrl+Q")),
            ],
        },
        MenuDef {
            title_key: Key::MenuFile,
            items: vec![
                item(Key::ItemOpenFilePicker, MenuAction::OpenFilePicker, Some("Ctrl+O")),
                item(Key::ItemNewFile, MenuAction::NewFile, Some("n")),
                item(Key::ItemNewFolder, MenuAction::NewFolder, Some("N")),
                // Act on the file tree's selection, hence the tree-scoped key hints.
                item(Key::ItemRename, MenuAction::Rename, Some("e")),
                item(Key::ItemDelete, MenuAction::Delete, Some("Del")),
                group(Key::ItemSave, MenuAction::Save, Some("Ctrl+S")),
                item(Key::ItemSaveAs, MenuAction::SaveAs, None),
                item(Key::ItemSaveAll, MenuAction::SaveAll, Some("Ctrl+Shift+S")),
                group(Key::ItemCloseFile, MenuAction::CloseFile, Some("Ctrl+W")),
                group(Key::ItemNextTab, MenuAction::NextTab, Some("Ctrl+Shift+→")),
                item(Key::ItemPrevTab, MenuAction::PrevTab, Some("Ctrl+Shift+←")),
            ],
        },
        MenuDef {
            title_key: Key::MenuEdit,
            items: vec![
                item(Key::ItemUndo, MenuAction::Undo, Some("Ctrl+Z")),
                item(Key::ItemRedo, MenuAction::Redo, Some("Ctrl+Y")),
                group(Key::ItemCopy, MenuAction::Copy, Some("Ctrl+C")),
                item(Key::ItemCut, MenuAction::Cut, Some("Ctrl+X")),
                item(Key::ItemPaste, MenuAction::Paste, Some("Ctrl+V")),
                item(Key::ItemSelectAll, MenuAction::SelectAll, Some("Ctrl+A")),
                group(Key::ItemFind, MenuAction::Find, Some("Ctrl+F")),
                item(Key::ItemGotoLine, MenuAction::GotoLine, Some("Ctrl+G")),
                item(Key::ItemSearchProject, MenuAction::SearchProject, Some("Ctrl+Shift+H")),
                group(Key::ItemToggleComment, MenuAction::ToggleComment, Some("Ctrl+K")),
                item(Key::ItemDuplicateLine, MenuAction::DuplicateLine, Some("Alt+Shift+↓")),
                item(Key::ItemMoveLineUp, MenuAction::MoveLineUp, Some("Alt+↑")),
                item(Key::ItemMoveLineDown, MenuAction::MoveLineDown, Some("Alt+↓")),
                group(Key::ItemIndent, MenuAction::Indent, Some("Tab")),
                item(Key::ItemOutdent, MenuAction::Outdent, Some("Shift+Tab")),
                group(Key::ItemToggleFold, MenuAction::ToggleFold, Some("Ctrl+Shift+F")),
                // No key of its own: the comfortable chords are spent, and Alt+drag is the
                // gesture people already reach for. From here it is still keyboard-reachable,
                // and Shift+arrows then build the rectangle.
                item(Key::ItemColumnSelection, MenuAction::ColumnSelection, None),
            ],
        },
        MenuDef {
            title_key: Key::MenuView,
            items: vec![
                item(Key::ItemToggleSidebar, MenuAction::ToggleSidebar, Some("Ctrl+E")),
                item(Key::ItemToggleTerminal, MenuAction::ToggleTerminal, Some("Ctrl+J")),
                item(Key::ItemToggleMenuBar, MenuAction::ToggleMenuBar, Some("Ctrl+B")),
                // Reachable from the palette on purpose: Ctrl+Shift+B is the only key that
                // opens the menus, and a terminal without disambiguated key reporting sends
                // it as plain Ctrl+B — which hides the bar instead. Without this entry the
                // menus would be mouse-only there.
                item(Key::ItemOpenMenuBar, MenuAction::OpenMenuBar, Some("Ctrl+Shift+B")),
                group(Key::ItemToggleHiddenFiles, MenuAction::ToggleHiddenFiles, Some("H")),
                group(Key::ItemFocusFileTree, MenuAction::FocusFileTree, Some("Ctrl+Alt+←")),
                item(Key::ItemFocusEditor, MenuAction::FocusEditor, Some("Ctrl+Tab")),
                item(Key::ItemFocusTerminal, MenuAction::FocusTerminal, Some("Ctrl+Alt+↓")),
            ],
        },
        MenuDef {
            title_key: Key::MenuLayout,
            items: vec![
                item(Key::ItemLayoutClassic, MenuAction::LayoutClassic, None),
                item(Key::ItemLayoutWide, MenuAction::LayoutWide, None),
                item(Key::ItemLayoutTriple, MenuAction::LayoutTriple, None),
                group(Key::ItemToggleTerminalSide, MenuAction::ToggleTerminalSide, None),
                item(Key::ItemToggleResizeMode, MenuAction::ToggleResizeMode, Some("Ctrl+Shift+U")),
                item(Key::ItemToggleSplitView, MenuAction::ToggleSplitView, Some("Ctrl+L")),
            ],
        },
        MenuDef {
            title_key: Key::MenuRun,
            items: vec![
                item(Key::ItemRunFile, MenuAction::RunFile, Some("Ctrl+Shift+R")),
                group(Key::ItemRunTarget, MenuAction::RunTarget, None),
            ],
        },
        MenuDef {
            title_key: Key::MenuTerminal,
            items: vec![
                item(Key::ItemNewTerminal, MenuAction::NewTerminal, Some("Ctrl+Shift+N")),
                item(Key::ItemNewTerminalTab, MenuAction::NewTerminalTab, Some("Ctrl+Shift+T")),
                item(Key::ItemRenameTerminal, MenuAction::RenameTerminal, Some("Ctrl+Shift+E")),
                group(Key::ItemCloseTerminalTab, MenuAction::CloseTerminalTab, Some("Ctrl+Shift+K")),
                item(Key::ItemCloseTerminal, MenuAction::CloseTerminal, None),
                group(Key::ItemNextTerminal, MenuAction::NextTerminal, Some("Ctrl+Shift+↓")),
                item(Key::ItemPrevTerminal, MenuAction::PrevTerminal, Some("Ctrl+Shift+↑")),
                item(Key::ItemNextTerminalTab, MenuAction::NextTerminalTab, Some("Ctrl+Shift+→")),
                item(Key::ItemPrevTerminalTab, MenuAction::PrevTerminalTab, Some("Ctrl+Shift+←")),
            ],
        },
        MenuDef {
            title_key: Key::MenuWorkspace,
            items: vec![
                item(Key::ItemOpenWorkspace, MenuAction::OpenWorkspace, None),
                item(Key::ItemSaveWorkspace, MenuAction::SaveWorkspace, Some("Ctrl+Shift+W")),
                group(Key::ItemDeleteWorkspace, MenuAction::DeleteWorkspace, None),
            ],
        },
        MenuDef {
            title_key: Key::MenuHelp,
            items: vec![
                item(Key::ItemShowManual, MenuAction::ShowManual, Some("Ctrl+Shift+M")),
                group(Key::ItemAbout, MenuAction::ShowAbout, None),
            ],
        },
    ]
}

/// Every action offered anywhere, as (owning menu title, item): the menu bar's entries first,
/// then any context-menu entry the menu bar doesn't already carry. The command palette is built
/// from this, so a context-only action stays reachable without a mouse.
pub fn command_entries() -> Vec<(Key, MenuItemDef)> {
    let mut seen: Vec<MenuAction> = Vec::new();
    let mut out = Vec::new();
    for def in menu_defs() {
        for it in def.items {
            seen.push(it.action);
            out.push((def.title_key, it));
        }
    }
    for target in [ContextTarget::Sidebar, ContextTarget::Editor, ContextTarget::Terminal] {
        let group_key = match target {
            ContextTarget::Sidebar => Key::PanelFile,
            ContextTarget::Editor => Key::MenuEdit,
            ContextTarget::Terminal => Key::MenuTerminal,
        };
        for it in context_items(target) {
            if !seen.contains(&it.action) {
                seen.push(it.action);
                out.push((group_key, it));
            }
        }
    }
    out
}

/// Which frame a context menu was raised over, so the right item set is offered.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ContextTarget {
    Sidebar,
    Editor,
    Terminal,
}

/// A right-click / Ctrl+Space pop-up: a short, context-specific action list anchored where it was
/// opened. Reuses `MenuItemDef` (labels, shortcuts, group separators) and runs through the same
/// `run_menu_action` path as the menu bar.
pub struct ContextMenu {
    pub items: Vec<MenuItemDef>,
    pub selected: usize,
    /// Top-left cell the pop-up hangs from.
    pub anchor: (u16, u16),
}

impl ContextMenu {
    pub fn new(target: ContextTarget, anchor: (u16, u16)) -> Self {
        ContextMenu { items: context_items(target), selected: 0, anchor }
    }

    pub fn move_selection(&mut self, delta: isize) {
        let len = self.items.len() as isize;
        if len == 0 {
            return;
        }
        self.selected = (((self.selected as isize + delta) % len + len) % len) as usize;
    }

    pub fn selected_action(&self) -> Option<MenuAction> {
        self.items.get(self.selected).map(|i| i.action)
    }
}

/// The action list for each frame's context menu. Groups (separator rules) follow the same
/// `group()` convention as the menu bar.
fn context_items(target: ContextTarget) -> Vec<MenuItemDef> {
    match target {
        ContextTarget::Sidebar => vec![
            item(Key::ItemNewFile, MenuAction::NewFile, Some("n")),
            item(Key::ItemNewFolder, MenuAction::NewFolder, Some("N")),
            // "e" is what the tree actually binds; the hint used to claim F2, which focuses
            // the editor instead.
            group(Key::ItemRename, MenuAction::Rename, Some("e")),
            item(Key::ItemDelete, MenuAction::Delete, Some("Del")),
        ],
        ContextTarget::Editor => vec![
            item(Key::ItemCut, MenuAction::Cut, Some("Ctrl+X")),
            item(Key::ItemCopy, MenuAction::Copy, Some("Ctrl+C")),
            item(Key::ItemPaste, MenuAction::Paste, Some("Ctrl+V")),
            item(Key::ItemSelectAll, MenuAction::SelectAll, Some("Ctrl+A")),
            group(Key::ItemToggleComment, MenuAction::ToggleComment, Some("Ctrl+K")),
            group(Key::ItemFind, MenuAction::Find, Some("Ctrl+F")),
            item(Key::ItemGotoLine, MenuAction::GotoLine, Some("Ctrl+G")),
        ],
        ContextTarget::Terminal => vec![
            item(Key::ItemCopy, MenuAction::Copy, Some("Ctrl+C")),
            item(Key::ItemPaste, MenuAction::Paste, Some("Ctrl+V")),
            group(Key::ItemNewTerminal, MenuAction::NewTerminal, Some("Ctrl+Shift+N")),
            item(Key::ItemNewTerminalTab, MenuAction::NewTerminalTab, Some("Ctrl+Shift+T")),
            item(Key::ItemRenameTerminal, MenuAction::RenameTerminal, Some("Ctrl+Shift+E")),
            group(Key::ItemCloseTerminalTab, MenuAction::CloseTerminalTab, Some("Ctrl+Shift+K")),
            item(Key::ItemCloseTerminal, MenuAction::CloseTerminal, None),
        ],
    }
}

pub struct MenuBar {
    pub active: bool,
    pub menu_index: usize,
    pub item_index: usize,
    pub defs: Vec<MenuDef>,
}

impl MenuBar {
    pub fn new() -> Self {
        MenuBar {
            active: false,
            menu_index: 0,
            item_index: 0,
            defs: menu_defs(),
        }
    }

    pub fn open(&mut self) {
        self.active = true;
        self.item_index = 0;
    }

    pub fn close(&mut self) {
        self.active = false;
    }

    pub fn move_menu(&mut self, delta: isize) {
        let len = self.defs.len() as isize;
        let mut idx = self.menu_index as isize + delta;
        idx = ((idx % len) + len) % len;
        self.menu_index = idx as usize;
        self.item_index = 0;
    }

    pub fn move_item(&mut self, delta: isize) {
        let len = self.defs[self.menu_index].items.len() as isize;
        if len == 0 {
            return;
        }
        let mut idx = self.item_index as isize + delta;
        idx = ((idx % len) + len) % len;
        self.item_index = idx as usize;
    }

    pub fn selected_action(&self) -> Option<MenuAction> {
        self.defs[self.menu_index]
            .items
            .get(self.item_index)
            .map(|i| i.action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rule this whole audit exists to enforce: nothing is mouse-only. Every action the app
    /// knows must appear in the command palette, which is built from `command_entries`.
    #[test]
    fn every_action_is_reachable_from_the_palette() {
        let entries = command_entries();
        for action in MenuAction::ALL {
            assert!(
                entries.iter().any(|(_, it)| it.action == *action),
                "an action is missing from every menu, so it can only be reached with a mouse"
            );
        }
    }

    /// And the other direction: an action offered somewhere but absent from `ALL` would slip
    /// past the check above.
    #[test]
    fn the_palette_offers_each_action_once_and_only_known_ones() {
        let entries = command_entries();
        for (_, it) in &entries {
            assert!(MenuAction::ALL.contains(&it.action), "menus offer an action missing from MenuAction::ALL");
        }
        let mut actions: Vec<usize> = entries
            .iter()
            .map(|(_, it)| MenuAction::ALL.iter().position(|a| *a == it.action).unwrap())
            .collect();
        let before = actions.len();
        actions.sort_unstable();
        actions.dedup();
        assert_eq!(before, actions.len(), "the palette lists the same action twice");
    }

    /// No advertised shortcut may be Alt plus a letter or a digit. macOS sends Option as Meta
    /// only on US keyboard layouts, so on an Italian or German one those chords never reach the
    /// application — and a menu that promises a key which quietly does nothing is worse than a
    /// menu that promises none. Alt with an *arrow* is fine and deliberately still used: Option
    /// with an arrow produces no printable character, so it arrives as Meta on every layout.
    ///
    /// Function keys are barred for a different reason: on a laptop they sit behind Fn.
    /// Column selection has no key of its own, so the palette is the only way to it from the
    /// keyboard. If it ever falls out of the menus it becomes mouse-only in silence.
    #[test]
    fn column_selection_is_reachable_without_the_mouse() {
        let labels: Vec<String> = command_entries()
            .into_iter()
            .map(|(menu, it)| {
                format!("{}: {}", crate::i18n::t(crate::i18n::Lang::En, menu), crate::i18n::t(crate::i18n::Lang::En, it.label_key))
            })
            .collect();
        assert!(
            labels.iter().any(|l| l.contains("Column selection")),
            "the palette should list it; it lists: {labels:?}"
        );
    }

    #[test]
    fn no_shortcut_is_advertised_that_some_keyboards_cannot_send() {
        for (_, it) in command_entries() {
            let Some(sc) = it.shortcut else { continue };
            // The key itself is the last part of the chord; everything before it is modifiers,
            // so "Alt+Shift+↓" is an arrow and not, as a first-character test would have it, S.
            let pressed = sc.rsplit('+').next().unwrap_or(sc);
            let is_letter_or_digit = pressed.chars().count() == 1
                && pressed.chars().next().is_some_and(|c| c.is_ascii_alphanumeric());
            assert!(
                !(sc.contains("Alt+") && is_letter_or_digit),
                "{sc} needs Option-as-Meta, which macOS does not give non-US layouts"
            );
            let f_key =
                pressed.strip_prefix('F').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
            assert!(!f_key, "{sc} is a function key, which needs Fn on a laptop");
            assert!(!sc.contains("PgUp") && !sc.contains("PgDn"), "{sc} needs Fn on a laptop");
        }
    }
}
