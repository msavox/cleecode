use crate::i18n::Key;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    ToggleSidebar,
    ToggleTerminal,
    NewTerminalTab,
    CloseTerminalTab,
    ToggleMenuBar,
    OpenSettings,
    SaveAll,
    NewTerminal,
    CloseTerminal,
    Save,
    SaveAs,
    SelectVenv,
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
    NewFile,
    NewFolder,
    CommandPalette,
    OpenFilePicker,
    Rename,
    Delete,
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
                item(Key::ItemAbout, MenuAction::ShowAbout, None),
                group(Key::ItemCommandPalette, MenuAction::CommandPalette, Some("Ctrl+P")),
                item(Key::ItemOpenSettings, MenuAction::OpenSettings, Some("F4")),
                group(Key::ItemQuit, MenuAction::Quit, Some("Ctrl+Q")),
            ],
        },
        MenuDef {
            title_key: Key::MenuFile,
            items: vec![
                item(Key::ItemOpenFilePicker, MenuAction::OpenFilePicker, Some("Ctrl+O")),
                item(Key::ItemNewFile, MenuAction::NewFile, Some("n")),
                item(Key::ItemNewFolder, MenuAction::NewFolder, Some("N")),
                group(Key::ItemSave, MenuAction::Save, Some("Ctrl+S")),
                item(Key::ItemSaveAs, MenuAction::SaveAs, None),
                item(Key::ItemSaveAll, MenuAction::SaveAll, Some("Alt+S")),
                group(Key::ItemCloseFile, MenuAction::CloseFile, Some("Ctrl+W")),
                group(Key::ItemNextTab, MenuAction::NextTab, Some("Alt+.")),
                item(Key::ItemPrevTab, MenuAction::PrevTab, Some("Alt+,")),
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
                group(Key::ItemToggleComment, MenuAction::ToggleComment, Some("Ctrl+/")),
                item(Key::ItemDuplicateLine, MenuAction::DuplicateLine, Some("Alt+Shift+↓")),
                item(Key::ItemMoveLineUp, MenuAction::MoveLineUp, Some("Alt+↑")),
                item(Key::ItemMoveLineDown, MenuAction::MoveLineDown, Some("Alt+↓")),
                group(Key::ItemIndent, MenuAction::Indent, Some("Tab")),
                item(Key::ItemOutdent, MenuAction::Outdent, Some("Shift+Tab")),
                group(Key::ItemToggleFold, MenuAction::ToggleFold, Some("F7")),
            ],
        },
        MenuDef {
            title_key: Key::MenuView,
            items: vec![
                item(Key::ItemToggleSidebar, MenuAction::ToggleSidebar, Some("Ctrl+E")),
                item(Key::ItemToggleTerminal, MenuAction::ToggleTerminal, Some("Ctrl+J")),
                item(Key::ItemToggleMenuBar, MenuAction::ToggleMenuBar, Some("Ctrl+B")),
                group(Key::ItemToggleHiddenFiles, MenuAction::ToggleHiddenFiles, Some("H")),
            ],
        },
        MenuDef {
            title_key: Key::MenuLayout,
            items: vec![
                item(Key::ItemLayoutClassic, MenuAction::LayoutClassic, None),
                item(Key::ItemLayoutWide, MenuAction::LayoutWide, None),
                item(Key::ItemLayoutTriple, MenuAction::LayoutTriple, None),
                group(Key::ItemToggleTerminalSide, MenuAction::ToggleTerminalSide, None),
                item(Key::ItemToggleResizeMode, MenuAction::ToggleResizeMode, Some("F8")),
                item(Key::ItemToggleSplitView, MenuAction::ToggleSplitView, Some("Ctrl+L")),
            ],
        },
        MenuDef {
            title_key: Key::MenuRun,
            items: vec![
                item(Key::ItemRunFile, MenuAction::RunFile, Some("F10")),
                group(Key::ItemSelectVenv, MenuAction::SelectVenv, None),
            ],
        },
        MenuDef {
            title_key: Key::MenuTerminal,
            items: vec![
                item(Key::ItemNewTerminal, MenuAction::NewTerminal, Some("F5")),
                item(Key::ItemNewTerminalTab, MenuAction::NewTerminalTab, Some("Ctrl+T")),
                item(Key::ItemCloseTerminal, MenuAction::CloseTerminal, Some("F6")),
                group(Key::ItemNextTerminal, MenuAction::NextTerminal, Some("Ctrl+PgDn")),
                item(Key::ItemPrevTerminal, MenuAction::PrevTerminal, Some("Ctrl+PgUp")),
            ],
        },
    ]
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
            group(Key::ItemRename, MenuAction::Rename, Some("F2")),
            item(Key::ItemDelete, MenuAction::Delete, Some("Del")),
        ],
        ContextTarget::Editor => vec![
            item(Key::ItemCut, MenuAction::Cut, Some("Ctrl+X")),
            item(Key::ItemCopy, MenuAction::Copy, Some("Ctrl+C")),
            item(Key::ItemPaste, MenuAction::Paste, Some("Ctrl+V")),
            item(Key::ItemSelectAll, MenuAction::SelectAll, Some("Ctrl+A")),
            group(Key::ItemToggleComment, MenuAction::ToggleComment, Some("Ctrl+/")),
            group(Key::ItemFind, MenuAction::Find, Some("Ctrl+F")),
            item(Key::ItemGotoLine, MenuAction::GotoLine, Some("Ctrl+G")),
        ],
        ContextTarget::Terminal => vec![
            item(Key::ItemCopy, MenuAction::Copy, Some("Ctrl+C")),
            item(Key::ItemPaste, MenuAction::Paste, Some("Ctrl+V")),
            group(Key::ItemNewTerminal, MenuAction::NewTerminal, Some("F5")),
            item(Key::ItemNewTerminalTab, MenuAction::NewTerminalTab, Some("Ctrl+T")),
            group(Key::ItemCloseTerminalTab, MenuAction::CloseTerminalTab, None),
            item(Key::ItemCloseTerminal, MenuAction::CloseTerminal, Some("F6")),
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

    pub fn open_at(&mut self, menu_index: usize) {
        self.active = true;
        self.menu_index = menu_index;
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
