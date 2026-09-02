use crate::i18n::{self, Key, Lang};

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
    EditKeybindings,
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
    RunSelection,
    SendToAgent,
    ToggleBreakpoint,
    ShowWorkspacePanel,
    InspectVariable,
    ToggleSplitView,
    ToggleHiddenFiles,
    ToggleOpaqueBackground,
    ShowThemes,
    TogglePlotsInTabs,
    Undo,
    Redo,
    ToggleComment,
    DuplicateLine,
    MoveLineUp,
    MoveLineDown,
    Find,
    GotoLine,
    SearchProject,
    ToggleGitPanel,
    GitStatus,
    GitChanges,
    GitHistory,
    GitBranches,
    GitStashes,
    GitFetch,
    GitPull,
    GitPush,
    GitStageFile,
    GitUnstageFile,
    GitDiscardFile,
    GitFileDiff,
    GitCommit,
    GoToDefinition,
    JumpBack,
    FindReferences,
    DocumentSymbols,
    ShowDiagnostics,
    NewFile,
    NewFolder,
    OpenOutside,
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
    // The eleven markdown formatting actions, and the switch that hides the bar they also sit on.
    MdBold,
    MdItalic,
    MdStrike,
    MdCode,
    MdHeading,
    MdBullet,
    MdNumbered,
    MdTask,
    MdLink,
    MdQuote,
    MdFence,
    ToggleMdToolbar,
    ToggleFollowAgentEdits,
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
        MenuAction::EditKeybindings,
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
        MenuAction::RunSelection,
        MenuAction::SendToAgent,
        MenuAction::ToggleBreakpoint,
        MenuAction::ShowWorkspacePanel,
        MenuAction::InspectVariable,
        MenuAction::ToggleSplitView,
        MenuAction::ToggleHiddenFiles,
        MenuAction::ToggleOpaqueBackground,
        MenuAction::ShowThemes,
        MenuAction::TogglePlotsInTabs,
        MenuAction::Undo,
        MenuAction::Redo,
        MenuAction::ToggleComment,
        MenuAction::DuplicateLine,
        MenuAction::MoveLineUp,
        MenuAction::MoveLineDown,
        MenuAction::Find,
        MenuAction::GotoLine,
        MenuAction::SearchProject,
        MenuAction::ToggleGitPanel,
        MenuAction::GitStatus,
        MenuAction::GitChanges,
        MenuAction::GitHistory,
        MenuAction::GitBranches,
        MenuAction::GitStashes,
        MenuAction::GitFetch,
        MenuAction::GitPull,
        MenuAction::GitPush,
        MenuAction::GitStageFile,
        MenuAction::GitUnstageFile,
        MenuAction::GitDiscardFile,
        MenuAction::GitFileDiff,
        MenuAction::GitCommit,
        MenuAction::GoToDefinition,
        MenuAction::JumpBack,
        MenuAction::FindReferences,
        MenuAction::DocumentSymbols,
        MenuAction::ShowDiagnostics,
        MenuAction::NewFile,
        MenuAction::NewFolder,
        MenuAction::OpenOutside,
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
        MenuAction::MdBold,
        MenuAction::MdItalic,
        MenuAction::MdStrike,
        MenuAction::MdCode,
        MenuAction::MdHeading,
        MenuAction::MdBullet,
        MenuAction::MdNumbered,
        MenuAction::MdTask,
        MenuAction::MdLink,
        MenuAction::MdQuote,
        MenuAction::MdFence,
        MenuAction::ToggleMdToolbar,
        MenuAction::ToggleFollowAgentEdits,
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
    /// A caption over the group below it rather than something to pick.
    ///
    /// A rule alone says "these are apart from those", which is enough when the reader can see
    /// what they have in common. Four items that run git and four that do not look alike — they
    /// are all short sentences about the file you right-clicked — so the group is named. Not
    /// selectable, has no shortcut, and never reaches the command palette: there is nothing for
    /// it to do.
    pub header: bool,
}

pub struct MenuDef {
    pub title_key: Key,
    pub items: Vec<MenuItemDef>,
}

/// The parts of the app's state that a menu item reads out on its own right-hand side.
///
/// A menu that only names what an item *does* is fine for "Save" and silent for a switch: "Plots:
/// tabs or windows" was reachable, said what it was about, and left the one question anybody
/// opens it with — which of the two is it right now — unanswered until they flipped it and read
/// the status line.
#[derive(Clone, Copy)]
pub struct MenuStates {
    /// Where a session started now would put its figures. The *effective* destination, not the
    /// setting: on a machine with no screen the setting may say windows while every plot still
    /// arrives as a tab, and the menu would then be reading out a preference nobody is honouring.
    pub plots_in_tabs: bool,
    /// Whether the markdown formatting bar is wanted. The setting, not whether one is on screen
    /// this instant: over a Rust file there is no bar either way, and the menu is answering "is
    /// it switched on", which is the question you turn it off with.
    pub md_toolbar: bool,
    /// Whether files touched from outside open themselves beside your work. The setting, again:
    /// outside a repository it is on and does nothing, and the status line is where that is
    /// said — a menu that read "off" there would be answering a different question.
    pub follow_agent_edits: bool,
}

/// What `action` says about its state, in the column the shortcuts live in. `None` for the
/// items that do something once rather than hold a setting, which is almost all of them.
pub fn item_value(lang: Lang, action: MenuAction, states: MenuStates) -> Option<&'static str> {
    match action {
        MenuAction::TogglePlotsInTabs => Some(i18n::t(
            lang,
            if states.plots_in_tabs { Key::MenuValuePlotsTabs } else { Key::MenuValuePlotsWindows },
        )),
        MenuAction::ToggleMdToolbar => {
            Some(i18n::t(lang, if states.md_toolbar { Key::On } else { Key::Off }))
        }
        MenuAction::ToggleFollowAgentEdits => {
            Some(i18n::t(lang, if states.follow_agent_edits { Key::On } else { Key::Off }))
        }
        _ => None,
    }
}

/// How wide that column has to be for `action`, whatever the state happens to be.
///
/// The widest of the values it can take, so the dropdown is the same size before and after a
/// toggle: a menu that changed width under the cursor would move every other item's shortcut
/// sideways at the moment of pressing Enter.
///
/// Every combination of states is tried rather than one field at a time: an item reads out one
/// of them, but which one is `item_value`'s business, and a loop over a single field would stop
/// measuring the moment a second switch was added — as one was.
pub fn item_value_width(lang: Lang, action: MenuAction) -> usize {
    [true, false]
        .into_iter()
        .flat_map(|plots_in_tabs| {
            [true, false].into_iter().flat_map(move |md_toolbar| {
                [true, false].into_iter().map(move |follow_agent_edits| MenuStates {
                    plots_in_tabs,
                    md_toolbar,
                    follow_agent_edits,
                })
            })
        })
        .filter_map(|states| item_value(lang, action, states))
        .map(|value| value.chars().count())
        .max()
        .unwrap_or(0)
}

fn item(label_key: Key, action: MenuAction, shortcut: Option<&'static str>) -> MenuItemDef {
    MenuItemDef { label_key, action, shortcut, new_group: false, header: false }
}

/// A caption over the group it opens. Carries an action because every item does, and never runs
/// it: everything that can pick an item checks [`MenuItemDef::header`] first.
fn header(label_key: Key, action: MenuAction) -> MenuItemDef {
    MenuItemDef { label_key, action, shortcut: None, new_group: true, header: true }
}

/// Like `item`, but marks the start of a new group so a separator rule is drawn above it.
fn group(label_key: Key, action: MenuAction, shortcut: Option<&'static str>) -> MenuItemDef {
    MenuItemDef { label_key, action, shortcut, new_group: true, header: false }
}

pub fn menu_defs() -> Vec<MenuDef> {
    vec![
        MenuDef {
            title_key: Key::MenuCleeCode,
            items: vec![
                item(Key::ItemCommandPalette, MenuAction::CommandPalette, Some("Ctrl+P")),
                item(Key::ItemOpenSettings, MenuAction::OpenSettings, Some("Ctrl+Shift+O")),
                // Beside the settings panel because it is the other half of the same file, and
                // without a chord of its own on purpose: this is the entry you use once, to
                // find out that the chords are editable and what they are called.
                item(Key::ItemKeybindings, MenuAction::EditKeybindings, None),
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
                // The two the language server adds to moving around. Here rather than in a menu
                // of their own: from where you are sitting they are the same kind of thing as
                // Find and Go to line — ways of arriving somewhere — and which of the three
                // works depends on the file, not on which menu it was found in.
                group(Key::ItemGoToDefinition, MenuAction::GoToDefinition, Some("Ctrl+Shift+J")),
                item(Key::ItemJumpBack, MenuAction::JumpBack, Some("Ctrl+Shift+L")),
                // The three lists the same server can fill, beside the jump it already answers.
                // All of them arrive as a picker rather than as a pane: they are read once, on
                // the way to somewhere, and a panel that stayed open would be a second place to
                // look at the file from.
                item(Key::ItemFindReferences, MenuAction::FindReferences, Some("Ctrl+Shift+Y")),
                item(Key::ItemDocumentSymbols, MenuAction::DocumentSymbols, Some("Ctrl+Shift+V")),
                // No chord: the comfortable ones are spent, and this is the one of the three
                // nobody reaches for mid-keystroke — it is looked at after a build, not while
                // typing a name.
                item(Key::ItemShowDiagnostics, MenuAction::ShowDiagnostics, None),
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
            // Markdown formatting, and only markdown: everything here writes the syntax of one
            // language, which is why it is not in Edit beside the operations that work on any
            // file. The menu is offered whatever is open — an action that says which files it is
            // for is discoverable, and one that appears and disappears with the tab is not.
            //
            // Nothing here has a shortcut. Five comfortable chords are left in the whole
            // application and eleven actions would not fit in them; the bar and this menu are
            // the surface, and the palette reaches both.
            title_key: Key::MenuFormat,
            items: vec![
                item(Key::ItemMdBold, MenuAction::MdBold, None),
                item(Key::ItemMdItalic, MenuAction::MdItalic, None),
                item(Key::ItemMdStrike, MenuAction::MdStrike, None),
                item(Key::ItemMdCode, MenuAction::MdCode, None),
                group(Key::ItemMdHeading, MenuAction::MdHeading, None),
                group(Key::ItemMdBullet, MenuAction::MdBullet, None),
                item(Key::ItemMdNumbered, MenuAction::MdNumbered, None),
                item(Key::ItemMdTask, MenuAction::MdTask, None),
                group(Key::ItemMdLink, MenuAction::MdLink, None),
                item(Key::ItemMdQuote, MenuAction::MdQuote, None),
                item(Key::ItemMdFence, MenuAction::MdFence, None),
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
                // Also a button on the menu bar itself, since the reason to reach for it is
                // that the screen has become hard to read — which is a bad moment to be asked
                // to find a menu in it.
                item(Key::ItemOpaqueBackground, MenuAction::ToggleOpaqueBackground, None),
                // Also a button on the menu bar, for the same reason as the one above it: the
                // moment you want a different theme is the moment the current one is hard to
                // read. Here as well, because a control reachable only by mouse is one the
                // keyboard cannot have.
                item(Key::ItemThemes, MenuAction::ShowThemes, None),
                // The bar teaches the syntax it writes, so it is meant to be switched off once
                // it has: this row reads out which way it is set, because "is it me or is it
                // this file" is the question somebody who cannot see it arrives with.
                item(Key::ItemToggleMdToolbar, MenuAction::ToggleMdToolbar, None),
                // Reads out its own state for the same reason the bar above it does: it is a
                // switch whose effect is that something appears by itself, and "is this on"
                // is the first question anybody arrives with.
                item(Key::ItemFollowAgentEdits, MenuAction::ToggleFollowAgentEdits, None),
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
                item(Key::ItemRunSelection, MenuAction::RunSelection, Some("Ctrl+Shift+X")),
                // Beside the two that run the file themselves, because from where you are sitting
                // it is the same kind of errand: you have a selection, or a cursor on a line, and
                // you are handing it to something that will act on it. The chord was the only way
                // to it, and a feature reachable only by a chord you were told about once is a
                // feature most people never learn they have — the palette is built from these
                // entries, so this row is also the row Ctrl+P finds.
                item(Key::ItemSendToAgent, MenuAction::SendToAgent, Some("Ctrl+Shift+A")),
                // Both belong to a running session rather than to a file, which is why they sit
                // below the separator: they were reachable only from the keyboard before, and a
                // feature nobody can find is a feature nobody has.
                group(Key::ItemToggleBreakpoint, MenuAction::ToggleBreakpoint, Some("Ctrl+Shift+P")),
                item(Key::ItemInspectVariable, MenuAction::InspectVariable, Some("Ctrl+Shift+I")),
                // Where the next session's plots go. Here rather than in View because it is not
                // about how CleeCode looks: it changes what the interpreter is told to do.
                item(Key::ItemPlotsInTabs, MenuAction::TogglePlotsInTabs, None),
                group(Key::ItemRunTarget, MenuAction::RunTarget, None),
            ],
        },
        MenuDef {
            // Git gets a menu because it grew one. It was a single line in Edit — "Git panel" —
            // which was honest while the panel only read: one thing, in the menu for the things
            // you do to the text in front of you. Now it stages, commits, branches, merges,
            // stashes and talks to a server, and a feature whose whole surface is one chord and
            // sixteen single letters inside a modal is a feature you have to be told about.
            title_key: Key::MenuGit,
            items: vec![
                item(Key::ItemGitPanel, MenuAction::ToggleGitPanel, Some("Ctrl+Shift+D")),
                // Each opens the panel already on the tab it names. Which tab you want is the
                // question you arrive with — "what have I changed", "where am I" — and answering
                // it from the menu means never landing on a list you did not come for.
                group(Key::ItemGitStatus, MenuAction::GitStatus, None),
                item(Key::ItemGitChanges, MenuAction::GitChanges, None),
                item(Key::ItemGitHistory, MenuAction::GitHistory, None),
                item(Key::ItemGitBranches, MenuAction::GitBranches, None),
                item(Key::ItemGitStashes, MenuAction::GitStashes, None),
                // Below the line because they are the only three that leave this machine, and
                // the only three that do not happen in the panel at all.
                group(Key::ItemGitFetch, MenuAction::GitFetch, None),
                item(Key::ItemGitPull, MenuAction::GitPull, None),
                item(Key::ItemGitPush, MenuAction::GitPush, None),
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
                // Opening the variables panel belongs here rather than to the two presets that
                // used to be the only way to get one: any layout can want it, and a window
                // nothing can open is a window most people never see.
                group(Key::ItemShowWorkspacePanel, MenuAction::ShowWorkspacePanel, None),
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
        // Asked for with the git half included, because from the palette these act on the
        // tree's selection exactly as Rename and Delete do — and an action reachable only by
        // right-clicking is one somebody working from the keyboard does not have.
        for it in context_items(target, true) {
            // Headers are captions, not commands. One in the palette would be a row that looks
            // like an action and does whichever action it happens to carry.
            if !it.header && !seen.contains(&it.action) {
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
    pub fn new(target: ContextTarget, anchor: (u16, u16), versioned: bool) -> Self {
        let items = context_items(target, versioned);
        // Never opens on a caption. Today the first item is always a real one, and a menu whose
        // first row became a heading would otherwise open with Enter doing nothing.
        let selected = items.iter().position(|i| !i.header).unwrap_or(0);
        ContextMenu { items, selected, anchor }
    }

    /// Moves the cursor, stepping over the captions.
    ///
    /// In the direction of travel, so walking down past a heading lands on the first item under
    /// it and walking up past one lands on the last item above it. A cursor that stopped on a
    /// caption would be a row you can highlight and not choose.
    pub fn move_selection(&mut self, delta: isize) {
        let len = self.items.len() as isize;
        if len == 0 || self.items.iter().all(|i| i.header) {
            return;
        }
        let step = if delta < 0 { -1 } else { 1 };
        let mut at = self.selected as isize;
        for _ in 0..delta.abs().max(1) {
            loop {
                at = ((at + step) % len + len) % len;
                if !self.items[at as usize].header {
                    break;
                }
            }
        }
        self.selected = at as usize;
    }

    /// The action of the row the cursor is on, and `None` if it is on a caption — which the
    /// cursor does not stop on, but Enter must not run one either way.
    pub fn selected_action(&self) -> Option<MenuAction> {
        self.items.get(self.selected).filter(|i| !i.header).map(|i| i.action)
    }
}

/// The action list for each frame's context menu. Groups (separator rules) follow the same
/// `group()` convention as the menu bar.
/// The pop-up's items.
///
/// `versioned` is whether the sidebar row git has something to say about — a file it has never
/// been told about counts, since staging is exactly what you would want to do to one. The git
/// half is left out entirely when it does not, rather than drawn greyed: a right-click on a file
/// in a folder that is not a repository at all would otherwise offer four things that cannot
/// happen, every time.
fn context_items(target: ContextTarget, versioned: bool) -> Vec<MenuItemDef> {
    match target {
        ContextTarget::Sidebar => {
            let mut items = vec![
                // First, and not among the git rows below: a PDF or a .md that CleeCode can only
                // show is exactly what you right-click to send somewhere that can do more with
                // it, and that is a different kind of errand from staging.
                item(Key::ItemOpenOutside, MenuAction::OpenOutside, None),
                group(Key::ItemNewFile, MenuAction::NewFile, Some("n")),
                item(Key::ItemNewFolder, MenuAction::NewFolder, Some("N")),
                // "e" is what the tree actually binds; the hint used to claim F2, which focuses
                // the editor instead.
                group(Key::ItemRename, MenuAction::Rename, Some("e")),
                item(Key::ItemDelete, MenuAction::Delete, Some("Del")),
            ];
            if versioned {
                // Named, because a rule alone would not say what these four have in common: they
                // are the same shape of sentence as Rename and Delete above them and they do a
                // quite different kind of thing.
                items.push(header(Key::HeaderGitFile, MenuAction::GitStageFile));
                // Staging and unstaging happen here and now: they are two commands that change
                // nothing you cannot change back, and walking to a panel to run one on the file
                // you are already pointing at is the long way round.
                items.push(item(Key::ItemGitStageFile, MenuAction::GitStageFile, None));
                items.push(item(Key::ItemGitUnstageFile, MenuAction::GitUnstageFile, None));
                items.push(item(Key::ItemGitFileDiff, MenuAction::GitFileDiff, None));
                // And this one does not: it opens the panel on the file with the question
                // already up. The question, the one letter that answers it and the refusal for a
                // file git has never seen are all tested where they are — a second copy of them
                // out here is a second copy to keep right.
                items.push(item(Key::ItemGitDiscardFile, MenuAction::GitDiscardFile, None));
                // And the ones that are about the repository rather than the file. They are here
                // under a heading of their own instead of only in the Git menu because this is
                // where the sentence ends: you staged a file, and the next thing you want is to
                // commit it — not to close a pop-up and go looking along the menu bar.
                items.push(header(Key::HeaderGitRepo, MenuAction::GitCommit));
                items.push(item(Key::ItemGitCommit, MenuAction::GitCommit, None));
                items.push(item(Key::ItemGitFetch, MenuAction::GitFetch, None));
                items.push(item(Key::ItemGitPull, MenuAction::GitPull, None));
                items.push(item(Key::ItemGitPush, MenuAction::GitPush, None));
            }
            items
        }
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

    /// The switch reads out which way it is set. Without this the Run menu offered "Plots: tabs
    /// or windows" and answered neither — the only way to learn the state was to change it and
    /// read the status line, which is a question you cannot ask without also answering it.
    #[test]
    fn the_plot_item_says_which_of_the_two_it_is() {
        for lang in [Lang::En, Lang::It] {
            let tabs =
                item_value(lang, MenuAction::TogglePlotsInTabs, MenuStates { plots_in_tabs: true, md_toolbar: true, follow_agent_edits: false });
            let windows =
                item_value(lang, MenuAction::TogglePlotsInTabs, MenuStates { plots_in_tabs: false, md_toolbar: true, follow_agent_edits: false });
            assert!(tabs.is_some() && windows.is_some(), "{lang:?}");
            assert_ne!(tabs, windows, "{lang:?}: both states read the same");
            // Wide enough for either, so the dropdown does not resize under the cursor at the
            // moment the item is picked.
            let width = item_value_width(lang, MenuAction::TogglePlotsInTabs);
            assert_eq!(width, tabs.unwrap().chars().count().max(windows.unwrap().chars().count()));
        }
    }

    /// Everything else is something to do once, and a value beside it would be a value about
    /// nothing. Guarded because the column is shared with the shortcuts: an action that grew a
    /// value *and* has a key would draw one over the other.
    #[test]
    fn an_item_never_carries_both_a_shortcut_and_a_state() {
        for def in menu_defs() {
            for item in def.items {
                assert!(
                    item.shortcut.is_none()
                        || item_value(Lang::En, item.action, MenuStates { plots_in_tabs: true, md_toolbar: true, follow_agent_edits: false })
                            .is_none(),
                    "\"{}\" has both a shortcut and a state to read out",
                    i18n::t(Lang::En, item.label_key)
                );
            }
        }
    }

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
