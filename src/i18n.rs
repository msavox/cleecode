#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Lang {
    En,
    It,
}

impl Lang {
    pub fn label(&self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::It => "Italiano",
        }
    }

    pub fn next(&self) -> Lang {
        match self {
            Lang::En => Lang::It,
            Lang::It => Lang::En,
        }
    }
}

impl Default for Lang {
    fn default() -> Self {
        Lang::En
    }
}

#[derive(Clone, Copy)]
pub enum Key {
    MenuCleeCode,
    MenuFile,
    MenuEdit,
    MenuFormat,
    MenuView,
    MenuTerminal,
    ItemSave,
    ItemSaveAs,
    ItemSaveAll,
    ItemQuit,
    ItemToggleSidebar,
    ItemToggleTerminal,
    ItemToggleDrawer,
    /// The drawer's border title while the launcher is showing — before there is an agent to
    /// name it after.
    DrawerTitle,
    /// Said beside an agent that is not on this machine. Honest and short: the name stays in
    /// the list, because the empty drawer is also where you learn what CleeCode can run.
    DrawerNotInstalled,
    /// The two keys the launcher answers to, on its bottom row.
    DrawerHint,
    ItemToggleMenuBar,
    ItemOpenMenuBar,
    ItemColumnSelection,
    ItemConvertLineEndings,
    /// The word the status bar puts beside the encoding chip on a buffer in the declared
    /// large-file mode. One word, because it shares a slot with the encoding and the line
    /// ending and is the last of the three to fit — its job is to keep the fact on screen after
    /// the sentence that explained it has scrolled away, not to explain it again.
    StatusLargeFile,
    // The Format menu, and the View entry that hides the bar carrying the same eleven actions.
    ItemMdBold,
    ItemMdItalic,
    ItemMdStrike,
    ItemMdCode,
    ItemMdHeading,
    ItemMdBullet,
    ItemMdNumbered,
    ItemMdTask,
    ItemMdLink,
    ItemMdQuote,
    ItemMdFence,
    ItemToggleMdToolbar,
    ItemFollowAgentEdits,
    MsgMdOnlyMarkdown,
    MsgMdCantHere,
    MdLinkPlaceholder,
    WorkspaceBadge,
    ItemOpenSettings,
    ItemKeybindings,
    ItemNewTerminal,
    ItemNewTerminalTab,
    ItemCloseTerminalTab,
    ItemRenameTerminal,
    ItemCloseTerminal,
    ItemAbout,
    ItemCopy,
    ItemCut,
    ItemPaste,
    ItemSelectAll,
    ItemIndent,
    ItemOutdent,
    ItemToggleFold,
    ItemCloseFile,
    ItemNextTab,
    ItemPrevTab,
    ItemNextTerminal,
    ItemPrevTerminal,
    MenuLayout,
    ItemLayoutClassic,
    ItemLayoutWide,
    ItemLayoutTriple,
    ItemToggleTerminalSide,
    ItemToggleResizeMode,
    ResizeModeHint,
    MenuRun,
    MenuGit,
    ItemRunFile,
    ItemRunSelection,
    ItemSendToAgent,
    ItemToggleBreakpoint,
    MenuDebug,
    ItemDebugStart,
    ItemDebugStop,
    ItemDebugContinue,
    ItemDebugStepOver,
    ItemDebugStepIn,
    ItemDebugStepOut,
    ItemDebugPanel,
    PanelDebug,
    DebugFrames,
    DebugVariables,
    DebugWatches,
    DebugOutput,
    DebugRunning,
    DebugAsking,
    DebugNoWatches,
    ItemShowWorkspacePanel,
    ItemInspectVariable,
    ItemRunTarget,
    RunMenuTitle,
    VenvRegisterItem,
    VenvBrowseItem,
    ItemToggleSplitView,
    ItemToggleHiddenFiles,
    ItemTransparentBackground,
    ItemThemes,
    ItemPlotsInTabs,
    MenuValuePlotsTabs,
    MenuValuePlotsWindows,
    ToolbarRun,
    ToolbarRefresh,
    ToolbarVenvNone,
    ToolbarRunNone,
    PanelFile,
    SettingsTitle,
    AboutTitle,
    AboutTagline,
    AboutAuthor,
    AboutRepo,
    AboutCloseHint,
    SplashTagline,
    SplashSubtitle,
    SplashHint,
    SettingLineNumbers,
    SettingSyntaxHighlighting,
    SettingWordWrap,
    SettingTabSize,
    SettingInsertSpaces,
    SettingShowWhitespace,
    SettingAutoIndent,
    SettingCompletion,
    SettingLanguageServer,
    SettingFollowAgentEdits,
    /// Whether an agent may change a buffer with unsaved work in it. Three answers rather than a
    /// switch, so each value says what it does instead of leaving the reader to guess what "off"
    /// would mean here.
    SettingAgentEdits,
    SettingAgentEditsAsk,
    SettingAgentEditsAllow,
    SettingAgentEditsDeny,
    SettingAutosaveRecovery,
    SettingPlotsInTabs,
    SettingPlotsNoDisplay,
    SettingPlotsTabs,
    SettingPlotsWindows,
    /// The agent drawer's two modes. The values name what each one does to the rest of the
    /// screen, because that is the whole of the difference between them.
    SettingDrawerMode,
    SettingDrawerPinned,
    SettingDrawerAutocollapse,
    SettingSplash,
    SettingMouseEnabled,
    SettingLanguage,
    On,
    Off,
    UntitledFile,
    StatusHelp,
    ItemUndo,
    ItemRedo,
    ItemToggleComment,
    ItemDuplicateLine,
    ItemMoveLineUp,
    ItemMoveLineDown,
    ItemFind,
    ItemGotoLine,
    ItemSearchProject,
    ItemReplaceProject,
    ItemGitPanel,
    ItemGoToDefinition,
    ItemJumpBack,
    ItemFindReferences,
    ItemDocumentSymbols,
    ItemRenameSymbol,
    ItemFormatDocument,
    ItemCodeActions,
    ItemExpandSelection,
    ItemShrinkSelection,
    ItemShowDiagnostics,
    ItemGitStatus,
    ItemGitChanges,
    ItemGitHistory,
    ItemGitBranches,
    ItemGitStashes,
    ItemGitFetch,
    ItemGitPull,
    ItemGitPush,
    ItemGitStageFile,
    ItemGitUnstageFile,
    ItemGitDiscardFile,
    ItemGitFileDiff,
    ItemGitCommit,
    HeaderGitFile,
    HeaderGitRepo,
    ItemNewFile,
    ItemNewFolder,
    ItemOpenOutside,
    ItemRename,
    ItemDelete,
    ItemCommandPalette,
    ItemOpenFilePicker,
    MsgNothingToUndo,
    MsgNothingToRedo,
    MsgNoCommentSyntax,
    ItemNextTerminalTab,
    ItemPrevTerminalTab,
    ItemFocusFileTree,
    ItemFocusEditor,
    ItemFocusTerminal,
    MenuWorkspace,
    ItemSaveWorkspace,
    ItemOpenWorkspace,
    ItemDeleteWorkspace,
    MenuHelp,
    ItemShowManual,
    ManualTitle,
    ManualHint,
    MsgNoWorkspaces,
    PickerCommands,
    PickerOpenFile,
    PickerOpenFileCapped,
    PickerSearchResults,
    PickerReferences,
    PickerSymbols,
    PickerDiagnostics,
    PickerCodeActions,
    PickerVariables,
    PickerVenvBrowse,
    PickerWorkspaceOpen,
    PickerWorkspaceDelete,
    PickerRecovery,
    // The frames the modals hang in. A box whose title is in one language and whose prompt is
    // in another reads as an unfinished translation, which is what it was.
    ModalDelete,
    ModalUnsaved,
    ModalRename,
    ModalTerminalForm,
    ModalSearchProject,
    ModalNewFolder,
    ModalNewFile,
    ModalSaveWorkspace,
    ModalSaveAs,
    ModalAddVenvPath,
    ModalAddVenvNickname,
    ModalFindReplace,
    FindLabel,
    ReplaceLabel,
    FindNoMatches,
    // The workspace viewer. It runs as its own process and draws its own table, but it is the
    // same program and answers to the same language setting.
    WsWaiting,
    WsWaitingWhere,
    WsWaitingQuiet,
    WsEmpty,
    WsRecent,
    WsColName,
    WsColSize,
    WsColClass,
    WsColMin,
    WsColMax,
    WsColMean,
    WsColValue,
}

pub fn t(lang: Lang, key: Key) -> &'static str {
    use Key::*;
    match (lang, key) {
        (Lang::En, MenuCleeCode) => "CleeCode",
        (Lang::It, MenuCleeCode) => "CleeCode",

        (Lang::En, MenuFile) => "File",
        (Lang::It, MenuFile) => "File",

        (Lang::En, MenuView) => "View",
        (Lang::It, MenuView) => "Vista",

        (Lang::En, MenuTerminal) => "Terminal",
        (Lang::It, MenuTerminal) => "Terminale",

        (Lang::En, MenuEdit) => "Edit",
        (Lang::It, MenuEdit) => "Modifica",

        (Lang::En, ItemSave) => "Save",
        (Lang::It, ItemSave) => "Salva",

        (Lang::En, ItemSaveAs) => "Save As...",
        (Lang::It, ItemSaveAs) => "Salva come...",

        (Lang::En, ItemSaveAll) => "Save All",
        (Lang::It, ItemSaveAll) => "Salva tutto",

        (Lang::En, ItemQuit) => "Quit",
        (Lang::It, ItemQuit) => "Esci",

        (Lang::En, ItemToggleSidebar) => "File sidebar",
        (Lang::It, ItemToggleSidebar) => "Sidebar file",

        (Lang::En, ItemToggleTerminal) => "Terminal panel",
        (Lang::It, ItemToggleTerminal) => "Pannello terminale",

        (Lang::En, ItemToggleDrawer) => "Agent drawer",
        (Lang::It, ItemToggleDrawer) => "Cassetto agente",
        (Lang::En, DrawerTitle) => "agent",
        (Lang::It, DrawerTitle) => "agente",
        (Lang::En, DrawerNotInstalled) => "not installed",
        (Lang::It, DrawerNotInstalled) => "non installato",
        (Lang::En, DrawerHint) => "\u{2191}\u{2193} choose \u{00b7} Enter starts it",
        (Lang::It, DrawerHint) => "\u{2191}\u{2193} scegli \u{00b7} Invio lo avvia",

        (Lang::En, ItemToggleMenuBar) => "Menu bar",
        (Lang::It, ItemToggleMenuBar) => "Barra dei menu",
        (Lang::En, ItemOpenMenuBar) => "Open the menu bar",
        (Lang::It, ItemOpenMenuBar) => "Apri la barra dei menu",
        (Lang::En, ItemColumnSelection) => "Column selection",
        (Lang::It, ItemColumnSelection) => "Selezione verticale",
        (Lang::En, ItemConvertLineEndings) => "Convert line endings",
        (Lang::It, ItemConvertLineEndings) => "Converti fine riga",
        (Lang::En, StatusLargeFile) => "large",
        (Lang::It, StatusLargeFile) => "grande",

        (Lang::En, MenuFormat) => "Format",
        (Lang::It, MenuFormat) => "Formato",
        (Lang::En, ItemMdBold) => "Bold",
        (Lang::It, ItemMdBold) => "Grassetto",
        (Lang::En, ItemMdItalic) => "Italic",
        (Lang::It, ItemMdItalic) => "Corsivo",
        (Lang::En, ItemMdStrike) => "Strikethrough",
        (Lang::It, ItemMdStrike) => "Barrato",
        (Lang::En, ItemMdCode) => "Inline code",
        (Lang::It, ItemMdCode) => "Codice inline",
        (Lang::En, ItemMdHeading) => "Heading (cycle)",
        (Lang::It, ItemMdHeading) => "Titolo (cicla)",
        (Lang::En, ItemMdBullet) => "Bullet list",
        (Lang::It, ItemMdBullet) => "Elenco puntato",
        (Lang::En, ItemMdNumbered) => "Numbered list",
        (Lang::It, ItemMdNumbered) => "Elenco numerato",
        (Lang::En, ItemMdTask) => "Task list",
        (Lang::It, ItemMdTask) => "Elenco attività",
        (Lang::En, ItemMdLink) => "Link",
        (Lang::It, ItemMdLink) => "Collegamento",
        (Lang::En, ItemMdQuote) => "Quote",
        (Lang::It, ItemMdQuote) => "Citazione",
        (Lang::En, ItemMdFence) => "Code block",
        (Lang::It, ItemMdFence) => "Blocco di codice",
        (Lang::En, ItemToggleMdToolbar) => "Formatting bar",
        (Lang::It, ItemToggleMdToolbar) => "Barra di formattazione",
        (Lang::En, ItemFollowAgentEdits) => "Follow edits made outside",
        (Lang::It, ItemFollowAgentEdits) => "Segui le modifiche da fuori",
        // The menu offers the eleven actions on every file, so the refusal has to say what kind
        // of file they are for rather than only that nothing happened.
        (Lang::En, MsgMdOnlyMarkdown) => "Formatting applies to Markdown files",
        (Lang::It, MsgMdOnlyMarkdown) => "La formattazione vale per i file Markdown",
        // And this one is for the right file in the wrong place: a rectangular selection, or a
        // run of text spanning lines that the markers would fall inside.
        (Lang::En, MsgMdCantHere) => "Can't format here",
        (Lang::It, MsgMdCantHere) => "Qui non si può formattare",
        // What a link gets for a label when there was nothing selected to make into one.
        (Lang::En, MdLinkPlaceholder) => "text",
        (Lang::It, MdLinkPlaceholder) => "testo",
        // Short on purpose: it shares the menu bar row with the menu titles.
        (Lang::En, WorkspaceBadge) => "workspace:",
        (Lang::It, WorkspaceBadge) => "workspace:",

        (Lang::En, ItemOpenSettings) => "Settings...",
        (Lang::It, ItemOpenSettings) => "Impostazioni...",

        // Opens settings.toml itself rather than a panel: the chords are a table in a file, and
        // the entry says so by taking you to the file with that table already written out.
        (Lang::En, ItemKeybindings) => "Keybindings...",
        (Lang::It, ItemKeybindings) => "Scorciatoie...",

        (Lang::En, ItemNewTerminal) => "New terminal window",
        (Lang::It, ItemNewTerminal) => "Nuova finestra terminale",
        (Lang::En, ItemNewTerminalTab) => "New terminal tab",
        (Lang::It, ItemNewTerminalTab) => "Nuovo tab terminale",
        (Lang::En, ItemCloseTerminalTab) => "Close terminal tab",
        (Lang::It, ItemCloseTerminalTab) => "Chiudi tab terminale",
        (Lang::En, ItemRenameTerminal) => "Rename terminal...",
        (Lang::It, ItemRenameTerminal) => "Rinomina terminale...",

        (Lang::En, ItemCloseTerminal) => "Close terminal",
        (Lang::It, ItemCloseTerminal) => "Chiudi terminale",

        (Lang::En, ItemAbout) => "About CleeCode",
        (Lang::It, ItemAbout) => "Informazioni su CleeCode",

        (Lang::En, ItemCopy) => "Copy",
        (Lang::It, ItemCopy) => "Copia",

        (Lang::En, ItemCut) => "Cut",
        (Lang::It, ItemCut) => "Taglia",

        (Lang::En, ItemPaste) => "Paste",
        (Lang::It, ItemPaste) => "Incolla",

        (Lang::En, ItemSelectAll) => "Select All",
        (Lang::It, ItemSelectAll) => "Seleziona tutto",

        (Lang::En, ItemIndent) => "Indent",
        (Lang::It, ItemIndent) => "Aumenta rientro",

        (Lang::En, ItemOutdent) => "Outdent",
        (Lang::It, ItemOutdent) => "Riduci rientro",

        (Lang::En, ItemToggleFold) => "Fold/Unfold",
        (Lang::It, ItemToggleFold) => "Comprimi/Espandi blocco",

        (Lang::En, ItemCloseFile) => "Close file",
        (Lang::It, ItemCloseFile) => "Chiudi file",

        (Lang::En, ItemNextTab) => "Next tab",
        (Lang::It, ItemNextTab) => "Tab successiva",

        (Lang::En, ItemPrevTab) => "Previous tab",
        (Lang::It, ItemPrevTab) => "Tab precedente",

        (Lang::En, ItemNextTerminal) => "Next terminal",
        (Lang::It, ItemNextTerminal) => "Terminale successivo",

        (Lang::En, ItemPrevTerminal) => "Previous terminal",
        (Lang::It, ItemPrevTerminal) => "Terminale precedente",

        (Lang::En, MenuLayout) => "Layout",
        (Lang::It, MenuLayout) => "Layout",

        (Lang::En, ItemLayoutClassic) => "Classic",
        (Lang::It, ItemLayoutClassic) => "Classico",

        (Lang::En, ItemLayoutWide) => "Wide (2 columns)",
        (Lang::It, ItemLayoutWide) => "Ampio (2 colonne)",

        (Lang::En, ItemLayoutTriple) => "Triple (3 columns)",
        (Lang::It, ItemLayoutTriple) => "Triplo (3 colonne)",

        (Lang::En, ItemToggleTerminalSide) => "Terminal on right",
        (Lang::It, ItemToggleTerminalSide) => "Terminale a destra",

        (Lang::En, ItemToggleResizeMode) => "Resize mode",
        (Lang::It, ItemToggleResizeMode) => "Modalita ridimensiona",

        (Lang::En, ResizeModeHint) => "Resize mode: arrows grow the focused frame, Shift+arrow shrinks, Esc/Enter to exit",
        (Lang::It, ResizeModeHint) => "Modalita ridimensiona: le frecce allargano il frame sotto focus, Shift+freccia restringe, Esc/Invio per uscire",

        (Lang::En, MenuRun) => "Run",
        (Lang::It, MenuRun) => "Esegui",
        // Its own menu rather than one line buried in Edit, which is where the panel used to be
        // reachable from. Everything git can do here is now something you can find by looking,
        // instead of a chord you have to have been told about.
        (Lang::En, MenuGit) => "Git",
        (Lang::It, MenuGit) => "Git",

        (Lang::En, ItemRunFile) => "Run current file",
        (Lang::It, ItemRunFile) => "Esegui file corrente",
        (Lang::En, ItemRunSelection) => "Run selection or cell",
        (Lang::It, ItemRunSelection) => "Esegui selezione o cella",
        // "To the agent's prompt", not "ask the agent": nothing is submitted and nothing is
        // asked. The text arrives where you type, and the question stays yours to press Enter on.
        (Lang::En, ItemSendToAgent) => "Send where you are to the agent",
        (Lang::It, ItemSendToAgent) => "Manda dove sei all'agente",
        (Lang::En, ItemShowWorkspacePanel) => "Show session variables",
        (Lang::It, ItemShowWorkspacePanel) => "Mostra le variabili della sessione",
        (Lang::En, ItemToggleBreakpoint) => "Breakpoint on this line",
        (Lang::It, ItemToggleBreakpoint) => "Breakpoint su questa riga",

        // Its own menu, beside Run rather than inside it: running a file and debugging a compiled
        // program are two errands, and the six rows below would have doubled the Run menu to say
        // so. Not one of them has a chord — see the comment on the menu itself.
        (Lang::En, MenuDebug) => "Debug",
        (Lang::It, MenuDebug) => "Debug",
        // "Start" and not "Run": what it starts is a debugger attached to this project's
        // executable, and Run already means handing a file to an interpreter.
        (Lang::En, ItemDebugStart) => "Start debugging",
        (Lang::It, ItemDebugStart) => "Avvia il debug",
        (Lang::En, ItemDebugStop) => "Stop debugging",
        (Lang::It, ItemDebugStop) => "Ferma il debug",
        (Lang::En, ItemDebugContinue) => "Continue",
        (Lang::It, ItemDebugContinue) => "Continua",
        (Lang::En, ItemDebugStepOver) => "Step over",
        (Lang::It, ItemDebugStepOver) => "Passo sopra",
        (Lang::En, ItemDebugStepIn) => "Step into",
        (Lang::It, ItemDebugStepIn) => "Passo dentro",
        // "Step out", and not "finish": the word on the wire is `stepOut` and the word in gdb is
        // `finish`, and a menu that used the second would be naming a command nobody types here.
        (Lang::En, ItemDebugStepOut) => "Step out",
        (Lang::It, ItemDebugStepOut) => "Passo fuori",
        // The one row of the menu that is about the screen rather than about the program: the
        // panel opens by itself when a session starts, and this is how somebody who put it away
        // gets it back without stopping what they were debugging.
        (Lang::En, ItemDebugPanel) => "Debug panel",
        (Lang::It, ItemDebugPanel) => "Pannello di debug",

        // The panel's own frame title and its four headings. Headings and not columns: the panel
        // is a narrow column beside the editor, and four sections stacked read where four columns
        // side by side would not fit at all.
        (Lang::En, PanelDebug) => "Debug",
        (Lang::It, PanelDebug) => "Debug",
        (Lang::En, DebugFrames) => "Frames",
        (Lang::It, DebugFrames) => "Frame",
        (Lang::En, DebugVariables) => "Variables",
        (Lang::It, DebugVariables) => "Variabili",
        // "Watches" is the debugger's word in English and has no settled Italian one; the phrase
        // Italian debuggers do use is "espressioni", so that is what it says rather than an
        // invented calque nobody would recognise.
        (Lang::En, DebugWatches) => "Watches",
        (Lang::It, DebugWatches) => "Espressioni",
        (Lang::En, DebugOutput) => "Output",
        (Lang::It, DebugOutput) => "Output",
        // What stands where the rows would be while the program is moving. One dim line, because
        // frames, variables and watches are all answers about a place the program is no longer
        // at, and leaving the old ones on screen would be the panel lying quietly.
        (Lang::En, DebugRunning) => "running…",
        (Lang::It, DebugRunning) => "in esecuzione…",
        // And the moment in between: stopped, but the adapter has not answered yet.
        (Lang::En, DebugAsking) => "asking the adapter…",
        (Lang::It, DebugAsking) => "sto chiedendo all'adapter…",
        // Doubles as the instruction, because an empty list with no hint under it is a section
        // nobody ever finds out how to fill.
        (Lang::En, DebugNoWatches) => "w adds one",
        (Lang::It, DebugNoWatches) => "w ne aggiunge una",
        (Lang::En, ItemInspectVariable) => "Look inside a variable...",
        (Lang::It, ItemInspectVariable) => "Guarda dentro una variabile...",

        (Lang::En, ItemRunTarget) => "How this file runs...",
        (Lang::It, ItemRunTarget) => "Come si esegue questo file...",

        (Lang::En, RunMenuTitle) => "Run",
        (Lang::It, RunMenuTitle) => "Esecuzione",

        (Lang::En, VenvRegisterItem) => "Add a venv from elsewhere on disk...",
        (Lang::It, VenvRegisterItem) => "Aggiungi un venv da un altro percorso...",
        (Lang::En, VenvBrowseItem) => "Browse for a venv folder...",
        (Lang::It, VenvBrowseItem) => "Sfoglia una cartella venv...",

        (Lang::En, ItemToggleSplitView) => "Split editor",
        (Lang::It, ItemToggleSplitView) => "Editor affiancati",

        (Lang::En, ItemToggleHiddenFiles) => "Hidden files",
        (Lang::It, ItemToggleHiddenFiles) => "File nascosti",

        // Named for what pressing it asks for, not for what the screen is doing: every theme
        // paints its own surface now, so the thing the user reaches for here is seeing through
        // it.
        (Lang::En, ItemTransparentBackground) => "Transparent background",
        (Lang::It, ItemTransparentBackground) => "Sfondo trasparente",

        (Lang::En, ItemThemes) => "Theme\u{2026}",
        (Lang::It, ItemThemes) => "Tema\u{2026}",

        // Named for where the file goes rather than for the machinery: "with the default
        // application" is how a settings dialog would say it, and this is a right-click on a
        // PDF.
        (Lang::En, ItemOpenOutside) => "Open outside CleeCode",
        (Lang::It, ItemOpenOutside) => "Apri fuori da CleeCode",

        (Lang::En, ItemPlotsInTabs) => "Plots: tabs or windows",
        (Lang::It, ItemPlotsInTabs) => "Grafici: schede o finestre",

        // Where a session started right now would put its plots, read out on the right of the
        // menu item that changes it. One word, because the label beside it has already asked
        // the question and the column is the same one the shortcuts use — and because the
        // settings row's own wording ("the interpreter's own windows") would make the Run menu
        // half again as wide for a value that is only ever one of two things.
        (Lang::En, MenuValuePlotsTabs) => "tabs",
        (Lang::It, MenuValuePlotsTabs) => "schede",
        (Lang::En, MenuValuePlotsWindows) => "windows",
        (Lang::It, MenuValuePlotsWindows) => "finestre",

        (Lang::En, ToolbarRun) => "Run",
        (Lang::It, ToolbarRun) => "Esegui",

        (Lang::En, ToolbarRefresh) => "Refresh",
        (Lang::It, ToolbarRefresh) => "Aggiorna",

        (Lang::En, ToolbarVenvNone) => "no venv",
        (Lang::It, ToolbarVenvNone) => "no venv",

        (Lang::En, ToolbarRunNone) => "no command",
        (Lang::It, ToolbarRunNone) => "nessun comando",

        (Lang::En, PanelFile) => "Files",
        (Lang::It, PanelFile) => "File",

        (Lang::En, SettingsTitle) => "Settings (Esc closes, Enter/arrows change value)",
        (Lang::It, SettingsTitle) => "Impostazioni (Esc chiude, Invio/frecce cambiano valore)",

        (Lang::En, AboutTitle) => "About",
        (Lang::It, AboutTitle) => "Informazioni",

        (Lang::En, AboutTagline) => {
            "A terminal IDE: micro-style editor with a file tree sidebar, integrated terminals and syntax highlighting."
        }
        (Lang::It, AboutTagline) => {
            "Una IDE da terminale: editor stile micro con sidebar file, terminali integrati ed evidenziazione sintattica."
        }

        (Lang::En, AboutAuthor) => "Created by Matteo Savoia",
        (Lang::It, AboutAuthor) => "Creato da Matteo Savoia",

        (Lang::En, AboutRepo) => "github.com/msavox/cleecode",
        (Lang::It, AboutRepo) => "github.com/msavox/cleecode",

        (Lang::En, AboutCloseHint) => "Press Esc or click anywhere to close",
        (Lang::It, AboutCloseHint) => "Premi Esc o clicca per chiudere",

        (Lang::En, SplashTagline) => "🐢  steady wins the race",
        (Lang::It, SplashTagline) => "🐢  chi va piano va lontano",

        (Lang::En, SplashSubtitle) => "a terminal IDE",
        (Lang::It, SplashSubtitle) => "una IDE da terminale",

        (Lang::En, SplashHint) => "press any key to continue",
        (Lang::It, SplashHint) => "premi un tasto per continuare",

        (Lang::En, SettingLineNumbers) => "Line numbers",
        (Lang::It, SettingLineNumbers) => "Numeri di riga",

        (Lang::En, SettingSyntaxHighlighting) => "Syntax highlighting",
        (Lang::It, SettingSyntaxHighlighting) => "Syntax highlighting",

        (Lang::En, SettingWordWrap) => "Word wrap",
        (Lang::It, SettingWordWrap) => "A capo automatico (word wrap)",

        (Lang::En, SettingTabSize) => "Tab size",
        (Lang::It, SettingTabSize) => "Ampiezza tab",

        (Lang::En, SettingInsertSpaces) => "Insert spaces instead of tabs",
        (Lang::It, SettingInsertSpaces) => "Inserisci spazi invece di tab",

        (Lang::En, SettingShowWhitespace) => "Show whitespace",
        (Lang::It, SettingShowWhitespace) => "Mostra spazi/tab (whitespace)",

        (Lang::En, SettingAutoIndent) => "Auto-indent",
        (Lang::It, SettingAutoIndent) => "Indentazione automatica",
        (Lang::En, SettingCompletion) => "Word completion",
        (Lang::It, SettingCompletion) => "Completamento parole",
        (Lang::En, SettingLanguageServer) => "Language server (diagnostics, completion)",
        (Lang::It, SettingLanguageServer) => "Language server (diagnostici, completamento)",
        // Named after what it does to the screen rather than after the agent, because a `sed`
        // in one of the terminal panes trips it in exactly the same way.
        (Lang::En, SettingFollowAgentEdits) => "Follow edits made outside (open the files)",
        (Lang::It, SettingFollowAgentEdits) => "Segui le modifiche da fuori (apri i file)",

        // The row names the buffers it is about, because that is the whole of why it exists: a
        // file with no unsaved work in it is one an agent edits on disk without asking anybody,
        // and CleeCode reloads it. This is only ever about text that is on screen and nowhere
        // else. Each value is a sentence in the first person, because the reader is deciding
        // what happens to their own work rather than setting a mode.
        (Lang::En, SettingAgentEdits) => "Agents editing unsaved buffers",
        (Lang::It, SettingAgentEdits) => "Agenti che modificano buffer non salvati",
        (Lang::En, SettingAgentEditsAsk) => "ask me each time",
        (Lang::It, SettingAgentEditsAsk) => "chiedimelo ogni volta",
        (Lang::En, SettingAgentEditsAllow) => "let them, without asking",
        (Lang::It, SettingAgentEditsAllow) => "lasciali fare, senza chiedere",
        (Lang::En, SettingAgentEditsDeny) => "never",
        (Lang::It, SettingAgentEditsDeny) => "mai",

        // Says what it keeps rather than what it is called: "autosave" reads as "your file is
        // written for you", and this never writes your file. It writes a copy elsewhere.
        (Lang::En, SettingAutosaveRecovery) => "Keep recovery copies of unsaved files",
        (Lang::It, SettingAutosaveRecovery) => "Tieni copie di ripristino dei file non salvati",

        // Named for the question rather than for one of its two answers. As "Plots as tabs"
        // with an on/off beside it, the row said nothing about what "off" meant — and "off"
        // meant the interpreter's own windows, which is a whole other way of working and the
        // thing somebody hunting for this row is usually hunting for.
        (Lang::En, SettingPlotsInTabs) => "Where plots open",
        (Lang::It, SettingPlotsInTabs) => "Dove si aprono i grafici",

        (Lang::En, SettingPlotsNoDisplay) => "tabs — no display here",
        (Lang::It, SettingPlotsNoDisplay) => "schede — qui non c'è un display",
        (Lang::En, SettingPlotsTabs) => "tabs, inside CleeCode",
        (Lang::It, SettingPlotsTabs) => "schede, dentro CleeCode",
        (Lang::En, SettingPlotsWindows) => "the interpreter's own windows",
        (Lang::It, SettingPlotsWindows) => "finestre dell'interprete",

        (Lang::En, SettingDrawerMode) => "Agent drawer",
        (Lang::It, SettingDrawerMode) => "Cassetto agente",
        (Lang::En, SettingDrawerPinned) => "pinned, part of the layout",
        (Lang::It, SettingDrawerPinned) => "fisso, parte del layout",
        (Lang::En, SettingDrawerAutocollapse) => "autocollapse, over the frames",
        (Lang::It, SettingDrawerAutocollapse) => "a scomparsa, sopra i frame",
        (Lang::En, SettingSplash) => "Splash screen at startup",
        (Lang::It, SettingSplash) => "Schermata iniziale all'avvio",

        (Lang::En, SettingMouseEnabled) => "Mouse enabled",
        (Lang::It, SettingMouseEnabled) => "Mouse abilitato",

        (Lang::En, SettingLanguage) => "Language",
        (Lang::It, SettingLanguage) => "Lingua",

        (Lang::En, On) => "on",
        (Lang::It, On) => "on",

        (Lang::En, Off) => "off",
        (Lang::It, Off) => "off",

        (Lang::En, UntitledFile) => "[untitled]",
        (Lang::It, UntitledFile) => "[senza nome]",

        (Lang::En, StatusHelp) => {
            "^⇧M manual · ^⇧B menu · ^P commands · ^⇧O settings · ^Tab frames · ^S save · ^Q quit"
        }
        (Lang::It, StatusHelp) => {
            "^⇧M manuale · ^⇧B menu · ^P comandi · ^⇧O impostazioni · ^Tab frame · ^S salva · ^Q esci"
        }

        (Lang::En, ItemUndo) => "Undo",
        (Lang::It, ItemUndo) => "Annulla",

        (Lang::En, ItemRedo) => "Redo",
        (Lang::It, ItemRedo) => "Ripeti",

        (Lang::En, ItemToggleComment) => "Toggle comment",
        (Lang::It, ItemToggleComment) => "Commenta/decommenta",

        (Lang::En, ItemDuplicateLine) => "Duplicate line",
        (Lang::It, ItemDuplicateLine) => "Duplica riga",

        (Lang::En, ItemMoveLineUp) => "Move line up",
        (Lang::It, ItemMoveLineUp) => "Sposta riga su",

        (Lang::En, ItemMoveLineDown) => "Move line down",
        (Lang::It, ItemMoveLineDown) => "Sposta riga giù",

        (Lang::En, ItemFind) => "Find / Replace...",
        (Lang::It, ItemFind) => "Trova / Sostituisci...",

        (Lang::En, ItemGotoLine) => "Go to line...",
        (Lang::It, ItemGotoLine) => "Vai alla riga...",
        (Lang::En, ItemSearchProject) => "Search in project...",
        (Lang::It, ItemSearchProject) => "Cerca nel progetto...",

        (Lang::En, ItemReplaceProject) => "Replace in project...",
        (Lang::It, ItemReplaceProject) => "Sostituisci nel progetto...",
        (Lang::En, ItemGitPanel) => "Git panel",
        (Lang::It, ItemGitPanel) => "Pannello Git",
        (Lang::En, ItemGoToDefinition) => "Go to definition",
        (Lang::It, ItemGoToDefinition) => "Vai alla definizione",
        (Lang::En, ItemJumpBack) => "Back where you were",
        (Lang::It, ItemJumpBack) => "Torna dov'eri",
        (Lang::En, ItemFindReferences) => "Find references",
        (Lang::It, ItemFindReferences) => "Trova i riferimenti",
        (Lang::En, ItemDocumentSymbols) => "Symbols in this file",
        (Lang::It, ItemDocumentSymbols) => "Simboli del file",
        // The one thing in this group that writes. It is a question first — the box, then the
        // preview — so the label says the action and not the consequence.
        (Lang::En, ItemRenameSymbol) => "Rename symbol",
        (Lang::It, ItemRenameSymbol) => "Rinomina simbolo",
        (Lang::En, ItemFormatDocument) => "Format document",
        (Lang::It, ItemFormatDocument) => "Formatta il documento",
        (Lang::En, ItemCodeActions) => "What can be done here",
        (Lang::It, ItemCodeActions) => "Cosa si può fare qui",
        // Named for what happens on screen rather than for the protocol's word for it: nobody
        // opens a menu looking for "selection range", and everybody knows what a selection that
        // grows is.
        (Lang::En, ItemExpandSelection) => "Widen the selection",
        (Lang::It, ItemExpandSelection) => "Allarga la selezione",
        (Lang::En, ItemShrinkSelection) => "Narrow the selection",
        (Lang::It, ItemShrinkSelection) => "Restringi la selezione",
        (Lang::En, ItemShowDiagnostics) => "Everything that is wrong",
        (Lang::It, ItemShowDiagnostics) => "Tutto quello che non va",

        // Each opens the panel on the tab it names. The panel's own tab strip does the same
        // thing in one keypress once it is open — these are for finding it in the first place.
        (Lang::En, ItemGitStatus) => "What has changed",
        (Lang::It, ItemGitStatus) => "Cosa è cambiato",
        (Lang::En, ItemGitChanges) => "The diff",
        (Lang::It, ItemGitChanges) => "Le modifiche",
        (Lang::En, ItemGitHistory) => "History and branches drawn",
        (Lang::It, ItemGitHistory) => "Cronologia e branch disegnati",
        (Lang::En, ItemGitBranches) => "Branches",
        (Lang::It, ItemGitBranches) => "Branch",
        (Lang::En, ItemGitStashes) => "Stashes",
        (Lang::It, ItemGitStashes) => "Stash",
        // Named for where they run, because that is the surprising part: the panel is not
        // involved, and the command appears at a prompt where it can ask you for a password.
        (Lang::En, ItemGitFetch) => "Fetch, in the terminal",
        (Lang::It, ItemGitFetch) => "Fetch, nel terminale",
        (Lang::En, ItemGitPull) => "Pull, in the terminal",
        (Lang::It, ItemGitPull) => "Pull, nel terminale",
        (Lang::En, ItemGitPush) => "Push, in the terminal",
        (Lang::It, ItemGitPush) => "Push, nel terminale",

        // On the right-click of a file the tree has a git mark against. Each names the file
        // rather than saying "this": the pop-up is anchored at the pointer and the row it came
        // from is behind it.
        (Lang::En, ItemGitStageFile) => "Stage this file",
        (Lang::It, ItemGitStageFile) => "Metti in stage questo file",
        (Lang::En, ItemGitUnstageFile) => "Take it back out of the index",
        (Lang::It, ItemGitUnstageFile) => "Toglilo dall'index",
        (Lang::En, ItemGitFileDiff) => "What changed in it",
        (Lang::It, ItemGitFileDiff) => "Cosa è cambiato dentro",
        (Lang::En, ItemGitDiscardFile) => "Throw its changes away...",
        (Lang::It, ItemGitDiscardFile) => "Butta via le sue modifiche...",
        (Lang::En, ItemGitCommit) => "Commit what is staged...",
        (Lang::It, ItemGitCommit) => "Committa quello che è in stage...",

        // The two captions over the git half of a file's right-click. They say which of the two
        // things below them is about — the file you clicked, or the repository it is in — because
        // "Stage this file" and "Push" are the same shape of sentence and a very different reach.
        (Lang::En, HeaderGitFile) => "Git — this file",
        (Lang::It, HeaderGitFile) => "Git — questo file",
        (Lang::En, HeaderGitRepo) => "Git — the repository",
        (Lang::It, HeaderGitRepo) => "Git — il repository",

        (Lang::En, ItemNewFile) => "New file...",
        (Lang::It, ItemNewFile) => "Nuovo file...",

        (Lang::En, ItemNewFolder) => "New folder...",
        (Lang::It, ItemNewFolder) => "Nuova cartella...",
        (Lang::En, ItemRename) => "Rename...",
        (Lang::It, ItemRename) => "Rinomina...",
        (Lang::En, ItemDelete) => "Delete...",
        (Lang::It, ItemDelete) => "Elimina...",

        (Lang::En, ItemCommandPalette) => "Command palette...",
        (Lang::It, ItemCommandPalette) => "Palette comandi...",

        (Lang::En, ItemOpenFilePicker) => "Open file...",
        (Lang::It, ItemOpenFilePicker) => "Apri file...",

        (Lang::En, MsgNothingToUndo) => "Nothing to undo",
        (Lang::It, MsgNothingToUndo) => "Niente da annullare",

        (Lang::En, MsgNothingToRedo) => "Nothing to redo",
        (Lang::It, MsgNothingToRedo) => "Niente da ripetere",

        (Lang::En, MsgNoCommentSyntax) => "No line-comment syntax for this file type",
        (Lang::It, MsgNoCommentSyntax) => "Nessuna sintassi di commento per questo tipo di file",

        (Lang::En, ItemNextTerminalTab) => "Next terminal tab",
        (Lang::It, ItemNextTerminalTab) => "Tab terminale successivo",
        (Lang::En, ItemPrevTerminalTab) => "Previous terminal tab",
        (Lang::It, ItemPrevTerminalTab) => "Tab terminale precedente",

        (Lang::En, ItemFocusFileTree) => "Focus file tree",
        (Lang::It, ItemFocusFileTree) => "Focus albero file",
        (Lang::En, ItemFocusEditor) => "Focus editor",
        (Lang::It, ItemFocusEditor) => "Focus editor",
        (Lang::En, ItemFocusTerminal) => "Focus terminal",
        (Lang::It, ItemFocusTerminal) => "Focus terminale",

        // Untranslated on purpose: "workspace" is what the concept is called in Italian too,
        // and it keeps the Alt+W mnemonic identical in both languages.
        (Lang::En, MenuWorkspace) => "Workspace",
        (Lang::It, MenuWorkspace) => "Workspace",

        (Lang::En, ItemSaveWorkspace) => "Save workspace...",
        (Lang::It, ItemSaveWorkspace) => "Salva workspace...",
        (Lang::En, ItemOpenWorkspace) => "Open workspace...",
        (Lang::It, ItemOpenWorkspace) => "Apri workspace...",
        (Lang::En, ItemDeleteWorkspace) => "Delete workspace...",
        (Lang::It, ItemDeleteWorkspace) => "Elimina workspace...",

        (Lang::En, MenuHelp) => "Help",
        (Lang::It, MenuHelp) => "Aiuto",

        (Lang::En, ItemShowManual) => "Manual...",
        (Lang::It, ItemShowManual) => "Manuale...",

        (Lang::En, ManualTitle) => "CleeCode manual",
        (Lang::It, ManualTitle) => "Manuale CleeCode",

        (Lang::En, ManualHint) => "↑↓ section · Space/⇧Space page · digit jumps · Home/End · Esc closes",
        (Lang::It, ManualHint) => "↑↓ sezione · Spazio/⇧Spazio pagina · cifra salta · Home/Fine · Esc chiude",

        (Lang::En, MsgNoWorkspaces) => "No saved workspaces yet — use Workspace ▸ Save workspace",
        (Lang::It, MsgNoWorkspaces) => "Nessun workspace salvato — usa Workspace ▸ Salva workspace",

        // The line across the top of each overlay list. It is the only thing that says what
        // Enter is about to do, and on the two workspace lists that is the difference between
        // opening a workspace and deleting one, so both name their verb.
        (Lang::En, PickerCommands) => "Command palette",
        (Lang::It, PickerCommands) => "Palette dei comandi",

        (Lang::En, PickerOpenFile) => "Open file (type / or ~ to browse)",
        (Lang::It, PickerOpenFile) => "Apri file (digita / o ~ per sfogliare)",

        (Lang::En, PickerOpenFileCapped) => "Open file (first 8000 only — type / or ~ to browse)",
        (Lang::It, PickerOpenFileCapped) => "Apri file (solo i primi 8000 — digita / o ~ per sfogliare)",

        (Lang::En, PickerSearchResults) => "Search results",
        (Lang::It, PickerSearchResults) => "Risultati della ricerca",

        (Lang::En, PickerReferences) => "Where that is used",
        (Lang::It, PickerReferences) => "Dove è usato",

        (Lang::En, PickerSymbols) => "Symbols in this file",
        (Lang::It, PickerSymbols) => "Simboli del file",

        // Named for what the list actually holds: the servers only speak about the files that
        // are open, so this is never the whole project however much it looks like it.
        (Lang::En, PickerDiagnostics) => "What is wrong in the open files",
        (Lang::It, PickerDiagnostics) => "Cosa non va nei file aperti",
        (Lang::En, PickerCodeActions) => "What the server offers to do here",
        (Lang::It, PickerCodeActions) => "Cosa il server si offre di fare qui",

        (Lang::En, PickerVariables) => "Variables",
        (Lang::It, PickerVariables) => "Variabili",

        (Lang::En, PickerVenvBrowse) => "Browse for a venv (type / or ~ to go elsewhere)",
        (Lang::It, PickerVenvBrowse) => "Cerca un venv (digita / o ~ per andare altrove)",

        (Lang::En, PickerWorkspaceOpen) => "Open workspace (Enter opens)",
        (Lang::It, PickerWorkspaceOpen) => "Apri workspace (Invio apre)",

        (Lang::En, PickerWorkspaceDelete) => "Delete workspace (Enter deletes)",
        (Lang::It, PickerWorkspaceDelete) => "Elimina workspace (Invio elimina)",

        // Honest about all three things at once: this is work CleeCode never saved, restoring it
        // does not save it either, and nothing is lost by pressing Esc. A title that only said
        // "recovered files" would leave somebody guessing at every one of those.
        (Lang::En, PickerRecovery) => {
            "Unsaved work from a session that ended (Enter restores it, unsaved; Esc keeps it for later)"
        }
        (Lang::It, PickerRecovery) => {
            "Lavoro non salvato di una sessione finita (Invio lo ripristina, non salvato; Esc lo tiene per dopo)"
        }

        // Modal titles. Written without the spaces that pad them against the border: the border
        // is the drawing's business, and a title with its padding baked in cannot be measured.
        (Lang::En, ModalDelete) => "Delete?",
        (Lang::It, ModalDelete) => "Eliminare?",

        (Lang::En, ModalUnsaved) => "Unsaved changes",
        (Lang::It, ModalUnsaved) => "Modifiche non salvate",

        (Lang::En, ModalRename) => "Rename",
        (Lang::It, ModalRename) => "Rinomina",

        (Lang::En, ModalTerminalForm) => "Terminal name & startup command",
        (Lang::It, ModalTerminalForm) => "Nome del terminale e comando di avvio",

        (Lang::En, ModalSearchProject) => "Search in project",
        (Lang::It, ModalSearchProject) => "Cerca nel progetto",

        (Lang::En, ModalNewFolder) => "New folder",
        (Lang::It, ModalNewFolder) => "Nuova cartella",

        (Lang::En, ModalNewFile) => "New file",
        (Lang::It, ModalNewFile) => "Nuovo file",

        (Lang::En, ModalSaveWorkspace) => "Save workspace",
        (Lang::It, ModalSaveWorkspace) => "Salva workspace",

        (Lang::En, ModalSaveAs) => "Save as",
        (Lang::It, ModalSaveAs) => "Salva come",

        // The two steps are numbered so it is plain the box will be back with a second question.
        (Lang::En, ModalAddVenvPath) => "Add venv (1/2)",
        (Lang::It, ModalAddVenvPath) => "Aggiungi venv (1/2)",

        (Lang::En, ModalAddVenvNickname) => "Add venv (2/2)",
        (Lang::It, ModalAddVenvNickname) => "Aggiungi venv (2/2)",

        (Lang::En, ModalFindReplace) => "Find / Replace",
        (Lang::It, ModalFindReplace) => "Trova / Sostituisci",

        // The two field labels of that box. The caret is placed from whichever is wider, so
        // neither may carry padding of its own.
        (Lang::En, FindLabel) => "Find:",
        (Lang::It, FindLabel) => "Trova:",

        (Lang::En, ReplaceLabel) => "Replace:",
        (Lang::It, ReplaceLabel) => "Sostituisci:",

        (Lang::En, FindNoMatches) => "(no matches)",
        (Lang::It, FindNoMatches) => "(nessun risultato)",

        // The viewer's first screen: it is on show for the whole of a session that never starts,
        // so it says both what is missing and that nothing is expected of the prompt.
        (Lang::En, WsWaiting) => "Waiting for a session…",
        (Lang::It, WsWaiting) => "In attesa di una sessione…",

        (Lang::En, WsWaitingWhere) => "Start an interpreter in one of the terminals and this fills in.",
        (Lang::It, WsWaitingWhere) => "Avvia un interprete in uno dei terminali e questo si riempie.",

        (Lang::En, WsWaitingQuiet) => "Nothing is typed at your prompt to ask it.",
        (Lang::It, WsWaitingQuiet) => "Non viene digitato niente al tuo prompt per chiederlo.",

        (Lang::En, WsEmpty) => "The workspace is empty.",
        (Lang::It, WsEmpty) => "Il workspace è vuoto.",

        (Lang::En, WsRecent) => "Recent",
        (Lang::It, WsRecent) => "Recenti",

        // Column headings, kept short: the viewer's pane is a window beside the editor, and a
        // heading wider than its column would be cut in the middle of a word.
        (Lang::En, WsColName) => "Name",
        (Lang::It, WsColName) => "Nome",

        (Lang::En, WsColSize) => "Size",
        (Lang::It, WsColSize) => "Dim.",

        (Lang::En, WsColClass) => "Class",
        (Lang::It, WsColClass) => "Classe",

        (Lang::En, WsColMin) => "Min",
        (Lang::It, WsColMin) => "Min",

        (Lang::En, WsColMax) => "Max",
        (Lang::It, WsColMax) => "Max",

        (Lang::En, WsColMean) => "Mean",
        (Lang::It, WsColMean) => "Media",

        (Lang::En, WsColValue) => "Value",
        (Lang::It, WsColValue) => "Valore",
    }
}

/// How many results a chooser is offering, on the line across its top.
pub fn msg_picker_matches(lang: Lang, count: usize) -> String {
    match lang {
        Lang::En => format!("({count} matches)"),
        Lang::It => format!("({count} risultati)"),
    }
}

/// What the viewer could not fit, and what to do about it.
///
/// Two forms because the advice is worth a line only when the line has room for it: cut
/// mid-word it reads like the bug it exists to prevent, so at the narrow end the count goes
/// alone.
pub fn msg_ws_more(lang: Lang, hidden: usize) -> String {
    match lang {
        Lang::En => format!("… {hidden} more — make this pane taller"),
        Lang::It => format!("… altre {hidden} — allarga questo pannello in altezza"),
    }
}

pub fn msg_ws_more_short(lang: Lang, hidden: usize) -> String {
    match lang {
        Lang::En => format!("… {hidden} more"),
        Lang::It => format!("… altre {hidden}"),
    }
}

/// Where the session is stopped, above the variables it is stopped among.
pub fn msg_ws_stopped(lang: Lang, name: &str, line: usize) -> String {
    match lang {
        Lang::En => format!("stopped in {name} at line {line}"),
        Lang::It => format!("fermo in {name} alla riga {line}"),
    }
}

pub fn msg_ws_called_from(lang: Lang, name: &str, line: usize) -> String {
    match lang {
        Lang::En => format!("called from {name} at line {line}"),
        Lang::It => format!("chiamato da {name} alla riga {line}"),
    }
}

pub fn terminal_title(lang: Lang, index: usize) -> String {
    match lang {
        Lang::En => format!(" Terminal {} ", index + 1),
        Lang::It => format!(" Terminale {} ", index + 1),
    }
}

pub fn terminal_starting(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "starting shell…",
        Lang::It => "avvio shell…",
    }
}

pub fn msg_unsaved_count(lang: Lang, count: usize) -> String {
    match lang {
        Lang::En => format!("{count} file(s) with unsaved changes"),
        Lang::It => format!("{count} file con modifiche non salvate"),
    }
}

pub fn msg_unsaved_question(lang: Lang, detail: &str) -> String {
    match lang {
        Lang::En => format!("Unsaved changes in {detail}."),
        Lang::It => format!("Modifiche non salvate in {detail}."),
    }
}

pub fn msg_unsaved_choices(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "S = save & continue · Y/Enter = discard · Esc = cancel",
        Lang::It => "S = salva e continua · Y/Invio = scarta · Esc = annulla",
    }
}

pub fn msg_find_hint(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Enter/↑↓ next/prev · Tab field · Ctrl+R replace · Ctrl+A all · Esc close",
        Lang::It => "Invio/↑↓ succ/prec · Tab campo · Ctrl+R sostituisci · Ctrl+A tutti · Esc chiudi",
    }
}

/// The two switches that decide how the query is read, drawn as the state they are in rather
/// than as things to do: which reading is in force is what you need to know when a search comes
/// back with a count you did not expect.
pub fn msg_find_flags(lang: Lang, case_sensitive: bool, regex: bool) -> String {
    let mark = |on: bool| if on { "on" } else { "off" };
    let segno = |on: bool| if on { "sì" } else { "no" };
    match lang {
        Lang::En => format!(
            "Ctrl+U case {} · Ctrl+N regex {}",
            mark(case_sensitive),
            mark(regex)
        ),
        Lang::It => format!(
            "Ctrl+U maiuscole {} · Ctrl+N regex {}",
            segno(case_sensitive),
            segno(regex)
        ),
    }
}

pub fn msg_git_panel_title(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Git — Esc closes · Tab/←→ switch · ↑↓ move · R refresh",
        Lang::It => "Git — Esc chiude · Tab/←→ cambia · ↑↓ scorre · R aggiorna",
    }
}

/// The keys the tab you are on actually has, drawn along its bottom.
///
/// Written out rather than left to the manual. Every one of these is a single letter with no
/// modifier, which is only safe because the panel takes the whole keyboard while it is up — and
/// a key that does something on one tab and nothing on the next has to say which is which,
/// or the way to find out is to press it.
pub fn msg_git_keys(lang: Lang, tab: crate::app::GitTab) -> &'static str {
    use crate::app::GitTab::*;
    match (lang, tab) {
        (Lang::En, Status) => "S stage · U unstage · A all · C commit · E amend · Z stash · X discard · Enter open",
        (Lang::It, Status) => "S in stage · U toglie · A tutto · C commit · E emenda · Z stash · X scarta · Invio apre",
        (Lang::En, Graph) => "Enter show · B branch here · T tag · K cherry-pick · V revert · H reset --hard",
        (Lang::It, Graph) => "Invio mostra · B branch qui · T tag · K cherry-pick · V revert · H reset --hard",
        (Lang::En, Branches) => "Enter switch · N new · D delete · M merge here · F fetch · L pull · P push",
        (Lang::It, Branches) => "Invio passa · N nuovo · D elimina · M unisce qui · F fetch · L pull · P push",
        (Lang::En, Stashes) => "Enter apply · O pop · D drop · Z stash what is here now",
        (Lang::It, Stashes) => "Invio applica · O pop · D elimina · Z mette via quello che c'è ora",
        (Lang::En, Diff) => "PgUp/PgDn a page · Home the top",
        (Lang::It, Diff) => "PgSu/PgGiù una pagina · Home in cima",
    }
}

/// The one line that says a command stopped half-way, and the one key that gets out of it.
///
/// Drawn only while there is one to get out of. A panel that always said "no merge in progress"
/// would be spending a row on the answer to a question nobody asked.
pub fn msg_git_unfinished(lang: Lang, what: crate::git::Unfinished) -> String {
    use crate::git::Unfinished::*;
    let name = match (lang, what) {
        (Lang::En, Merge) => "A merge",
        (Lang::It, Merge) => "Un merge",
        (Lang::En, CherryPick) => "A cherry-pick",
        (Lang::It, CherryPick) => "Un cherry-pick",
        (Lang::En, Revert) => "A revert",
        (Lang::It, Revert) => "Un revert",
        (Lang::En, Rebase) => "A rebase",
        (Lang::It, Rebase) => "Un rebase",
    };
    match lang {
        Lang::En => format!("{name} stopped part-way — stage the files it marked and commit, or press Q to put it back"),
        Lang::It => format!("{name} si è fermato a metà — metti in stage i file che ha segnato e committa, oppure Q per tornare indietro"),
    }
}

/// The run-target row that means "the prompt that is already open".
/// What the four arrows on a figure's bar do, in a word.
///
/// The buttons carry their own keys, so this does not list them — that was the duplication the
/// bar already had once and lost. What a button cannot say is which of the two things it does:
/// the same arrow slides a flat plot and turns a solid one, and only the axes knows which.
/// What a menu is called on the bar.
///
/// One function, because three things ask: the drawing, the hit-test that says which title a
/// click landed on, and the mnemonic that opens a menu by its first letter. A title that were
/// one length to the drawing and another to the hit-test would put every click on the wrong
/// menu.
pub fn menu_title(lang: Lang, key: Key) -> &'static str {
    t(lang, key)
}

pub fn msg_figure_keys(lang: Lang, is3d: bool) -> &'static str {
    match (lang, is3d) {
        (Lang::En, false) => "arrows pan  ",
        (Lang::It, false) => "le frecce spostano  ",
        (Lang::En, true) => "arrows turn it  ",
        (Lang::It, true) => "le frecce ruotano  ",
    }
}

pub fn msg_run_session_row(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "The session that is open",
        Lang::It => "La sessione già aperta",
    }
}

/// Whether there is one right now. The row means two different things either way — with nothing
/// open it is a preference for next time rather than a destination — so it says which.
pub fn msg_run_session_detail(lang: Lang, open: bool) -> &'static str {
    match (lang, open) {
        (Lang::En, true) => "the file runs at that prompt and it keeps what the file made",
        (Lang::It, true) => "il file gira a quel prompt e la sessione tiene quello che fa",
        (Lang::En, false) => "none open — Run starts one in a shell until there is",
        (Lang::It, false) => "nessuna aperta — finché non c'è, Run ne avvia una in una shell",
    }
}

pub fn msg_run_session_chosen(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Run will hand the file to the interpreter already at a prompt",
        Lang::It => "Run consegnerà il file all'interprete già al prompt",
    }
}

pub fn msg_git_needs_a_name(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "That needs a name",
        Lang::It => "Serve un nome",
    }
}

/// Said in place of running `git commit` with nothing in the box. Git refuses it too, but its
/// refusal is a paragraph about the commit template and the editor it would have opened, none of
/// which is true here — the box was right there and empty.
pub fn msg_git_needs_a_message(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "That needs a message",
        Lang::It => "Serve un messaggio",
    }
}

pub fn msg_git_nothing_to_amend(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "There is no commit yet to amend",
        Lang::It => "Non c'è ancora un commit da emendare",
    }
}

pub fn msg_git_nothing_to_stash(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Nothing to put away — git only stashes files it knows about",
        Lang::It => "Niente da mettere via — git mette in stash solo i file che conosce",
    }
}

/// Said instead of asking. git refuses to delete the branch you are standing on, and it names
/// the branch rather than the reason — which is the one thing you already knew.
pub fn msg_git_branch_is_current(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("{name} is the branch you are on — move to another one first"),
        Lang::It => format!("{name} è il branch su cui sei — passa prima a un altro"),
    }
}

pub fn msg_git_merge_into_itself(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "That is the branch you are on — pick the one to merge into it",
        Lang::It => "È il branch su cui sei — scegli quello da unire a questo",
    }
}

pub fn msg_git_nothing_to_abort(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Nothing is half-done — Q puts back a merge, a pick, a revert or a rebase",
        Lang::It => "Non c'è niente a metà — Q annulla un merge, un pick, un revert o un rebase",
    }
}

/// Fetch, pull and push are typed into a shell rather than run behind the panel, and the message
/// says so — because the panel vanishing and a command appearing at a prompt is otherwise a
/// thing that happened *to* you rather than the thing you asked for.
pub fn msg_git_in_terminal(lang: Lang, command: &str) -> String {
    match lang {
        Lang::En => format!("{command} — running in the terminal, where it can ask you for a password"),
        Lang::It => format!("{command} — gira nel terminale, dove può chiederti una password"),
    }
}

pub fn msg_git_file_unchanged(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "That file is already what the last commit says it is",
        Lang::It => "Quel file è già come dice l'ultimo commit",
    }
}

pub fn msg_git_no_commits(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "No commits yet",
        Lang::It => "Ancora nessun commit",
    }
}

pub fn msg_git_no_stashes(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Nothing put away — Z stashes what is changed right now",
        Lang::It => "Niente messo via — Z mette in stash quello che è cambiato adesso",
    }
}

/// The heading over a box you type into.
pub fn msg_git_text_prompt(lang: Lang, kind: &crate::app::GitText, staged: usize) -> String {
    use crate::app::GitText::*;
    match (lang, kind) {
        (_, Commit) => msg_git_commit_prompt(lang, staged),
        (Lang::En, Amend) => format!("Rewrite the last commit — {staged} staged go into it · Enter · Esc cancels"),
        (Lang::It, Amend) => format!("Riscrive l'ultimo commit — ci finiscono {staged} in stage · Invio · Esc annulla"),
        (Lang::En, Branch { at: Some(at) }) => format!("Name for a branch starting at {at} · Enter · Esc cancels"),
        (Lang::It, Branch { at: Some(at) }) => format!("Nome del branch che parte da {at} · Invio · Esc annulla"),
        (Lang::En, Branch { at: None }) => "Name for a branch starting here · Enter · Esc cancels".to_string(),
        (Lang::It, Branch { at: None }) => "Nome del branch che parte da qui · Invio · Esc annulla".to_string(),
        (Lang::En, Tag { at }) => format!("Name for a tag on {at} · Enter · Esc cancels"),
        (Lang::It, Tag { at }) => format!("Nome del tag su {at} · Invio · Esc annulla"),
        (Lang::En, Stash) => "A name for what you are putting away — or none, and git writes one · Enter".to_string(),
        (Lang::It, Stash) => "Un nome per quello che metti via — o nessuno, e lo scrive git · Invio".to_string(),
    }
}

/// The heading over a question that takes one letter.
///
/// Every one of them ends in the same two letters as the discard question, and for the same
/// reason: the key that answers has to be in the text that asks, or a box that reads "S / N" and
/// answers only to `y` looks broken while working exactly as written.
pub fn msg_git_confirm_prompt(lang: Lang, confirm: &crate::app::GitConfirm) -> String {
    use crate::app::GitConfirm::*;
    let yes = yes_key(lang).to_ascii_uppercase();
    let no = match lang {
        Lang::En => 'N',
        Lang::It => 'N',
    };
    let body = match (lang, confirm) {
        (_, Discard(change)) => return msg_git_discard_prompt(lang, &change.path.display().to_string()),
        (Lang::En, DeleteBranch(name)) => format!(
            "Delete the branch {name}? Anything on it and nowhere else is only in the reflog afterwards."
        ),
        (Lang::It, DeleteBranch(name)) => format!(
            "Elimino il branch {name}? Quello che c'è sopra e da nessun'altra parte resta solo nel reflog."
        ),
        (Lang::En, ResetHard { hash, subject }) => format!(
            "Move this branch back to {hash} ({subject}) and make the files match? Commits after it stay in the reflog; changes you have not committed do not."
        ),
        (Lang::It, ResetHard { hash, subject }) => format!(
            "Riporto il branch a {hash} ({subject}) e allineo i file? I commit dopo restano nel reflog; le modifiche non committate no."
        ),
        (Lang::En, DropStash(name)) => format!("Throw away {name}? A dropped stash is not in any branch."),
        (Lang::It, DropStash(name)) => format!("Butto via {name}? Uno stash eliminato non è in nessun branch."),
    };
    format!("{body}  {yes} / {no}")
}

pub fn msg_git_clean(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Nothing changed — the working tree matches the last commit",
        Lang::It => "Niente di modificato — l'albero di lavoro è come l'ultimo commit",
    }
}

/// The box that takes a commit message. Says what will be committed, because "everything staged"
/// is not the same as "everything changed" and the difference is the whole reason staging exists.
pub fn msg_git_commit_prompt(lang: Lang, staged: usize) -> String {
    match lang {
        Lang::En => format!("Commit message — {staged} staged · Enter commits · Esc cancels"),
        Lang::It => format!("Messaggio del commit — {staged} in stage · Invio conferma · Esc annulla"),
    }
}

pub fn msg_git_nothing_staged(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Nothing is staged — S puts a file in, A puts everything in",
        Lang::It => "Non c'è niente in stage — S ce ne mette uno, A mette tutto",
    }
}

/// The one question in the panel that has to be answered rather than dismissed.
///
/// It names the file and it says where the work goes, because it does not go anywhere: this is
/// the only action here that is not in some reflog afterwards.
pub fn msg_git_discard_prompt(lang: Lang, file: &str) -> String {
    match lang {
        Lang::En => format!("Throw away every change to {file}? It is in no commit and no stash, and nothing brings it back.  Y / N"),
        Lang::It => format!("Butto via tutte le modifiche a {file}? Non sono in nessun commit né stash, e non le riporta indietro niente.  S / N"),
    }
}

/// The one letter that means yes, in the language the question was asked in.
///
/// Its own function so the key and the text of the question cannot drift apart: a box that reads
/// "S / N" and only answers to `y` is a box that looks broken while it is working exactly as
/// written.
pub fn yes_key(lang: Lang) -> char {
    match lang {
        Lang::En => 'y',
        Lang::It => 's',
    }
}

pub fn msg_git_working(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "git is working…",
        Lang::It => "git sta lavorando…",
    }
}

/// What to say when git did the thing and said nothing about it, which is most of them: `add`
/// and `reset` are silent when they work.
pub fn msg_git_done(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Done",
        Lang::It => "Fatto",
    }
}

pub fn msg_git_tab(lang: Lang, tab: crate::app::GitTab) -> &'static str {
    use crate::app::GitTab::*;
    match (lang, tab) {
        // Not "Files": the file tree's own frame is titled that, and two things called the same
        // on one screen is one of them being read as the other.
        (Lang::En, Status) => "Status",
        (Lang::It, Status) => "Stato",
        (Lang::En, Diff) => "Changes",
        (Lang::It, Diff) => "Modifiche",
        (Lang::En, Graph) => "History",
        (Lang::It, Graph) => "Cronologia",
        (Lang::En, Branches) => "Branches",
        (Lang::It, Branches) => "Branch",
        (Lang::En, Stashes) => "Stashes",
        (Lang::It, Stashes) => "Stash",
    }
}

pub fn msg_git_loading(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Asking git…",
        Lang::It => "Chiedo a git…",
    }
}

/// An empty diff is an answer, and which file it is an answer about matters: "nothing changed"
/// means something different for one file than for the whole tree.
pub fn msg_git_no_changes(lang: Lang, file: Option<&str>) -> String {
    match (lang, file) {
        (Lang::En, Some(f)) => format!("No changes in {f} since the last commit"),
        (Lang::It, Some(f)) => format!("Nessuna modifica in {f} dall'ultimo commit"),
        (Lang::En, None) => "Nothing changed since the last commit".to_string(),
        (Lang::It, None) => "Niente di modificato dall'ultimo commit".to_string(),
    }
}

/// The lines drawn in an editor frame with nothing open in it.
///
/// A list rather than one sentence because the second line is the way out: a frame that only
/// said "no file open" would be a dead end, and the tab you closed used to be the way back.
pub fn msg_no_file_open(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::En => &["No file open", "", "Ctrl+O to open one  ·  Ctrl+P for a command"],
        Lang::It => &["Nessun file aperto", "", "Ctrl+O per aprirne uno  ·  Ctrl+P per un comando"],
    }
}

pub fn msg_search_prompt(lang: Lang) -> String {
    match lang {
        Lang::En => "Search the project for:".to_string(),
        Lang::It => "Cerca nel progetto:".to_string(),
    }
}

/// Said while the walk is running. A search across a tree is the one thing here that can take a
/// visible moment, and silence would read as nothing having happened.
pub fn msg_search_running(lang: Lang, query: &str) -> String {
    match lang {
        Lang::En => format!("Searching for \"{query}\"…"),
        Lang::It => format!("Cerco \"{query}\"…"),
    }
}

pub fn msg_search_done(lang: Lang, hits: usize, files: usize, truncated: bool) -> String {
    let more_en = if truncated { " (stopped at the limit)" } else { "" };
    let more_it = if truncated { " (fermata al limite)" } else { "" };
    match lang {
        Lang::En => format!("{hits} line(s) in {files} file(s){more_en}"),
        Lang::It => format!("{hits} righe in {files} file{more_it}"),
    }
}

pub fn msg_search_none(lang: Lang, query: &str, files: usize) -> String {
    match lang {
        Lang::En => format!("\"{query}\" is nowhere in {files} file(s)"),
        Lang::It => format!("\"{query}\" non c'è in nessuno dei {files} file"),
    }
}

// ---- Replacing across the project ------------------------------------------------------------
//
// The same box as the search above, with a second field, and therefore the same family of
// refusals as the rename: this is the only thing in CleeCode that writes files nobody has open.
// Every sentence below says what was *not* done as well as why, because a sweep that stopped
// halfway and said nothing would leave a project nobody can reason about.

/// The second field of the search box. Empty it is a search, filled it is a sweep, and the box
/// has to say which of the two Enter is about to do.
pub fn msg_search_replace_prompt(lang: Lang) -> String {
    match lang {
        Lang::En => "Replace with (Tab switches, empty = just search):".to_string(),
        Lang::It => "Sostituisci con (Tab cambia, vuoto = cerca e basta):".to_string(),
    }
}

/// Said while the walk is running, which is the same walk the search runs — so it says the same
/// thing, plus what the hits are going to be offered as.
pub fn msg_replace_running(lang: Lang, query: &str, replacement: &str) -> String {
    match lang {
        Lang::En => format!("Searching for \"{query}\" to replace with \"{replacement}\"…"),
        Lang::It => format!("Cerco \"{query}\" da sostituire con \"{replacement}\"…"),
    }
}

/// The search stopped at its own limit, so what came back is part of the project rather than the
/// project. A preview built on part of it would show a sweep that is honest about what it draws
/// and silent about the rest, and the rest is what a replace-all is for.
pub fn msg_replace_refused_truncated(lang: Lang, limit: usize) -> String {
    match lang {
        Lang::En => format!(
            "More than {limit} lines match — more than one sweep can hold, narrow the query"
        ),
        Lang::It => format!(
            "Più di {limit} righe corrispondono — più di quanto una passata regga, restringi la ricerca"
        ),
    }
}

/// One of the files it would write to is open in a tab that cannot be typed in.
pub fn msg_replace_refused_read_only(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "One of the files this would change is open read-only — nothing was changed",
        Lang::It => "Uno dei file da cambiare è aperto in sola lettura — non ho cambiato niente",
    }
}

/// Every file the search named has stopped matching between the search and the preview.
///
/// Not an error on its own — a file rewritten by a build, a formatter or an agent in the second
/// the walk took is dropped from the preview without comment, because the search *is* a moment
/// old and that is not news. It only becomes a sentence when there is nothing left at all, since
/// then the preview would be an empty box asking to be agreed to.
pub fn msg_replace_nothing_left(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Nothing left to replace — the files changed since the search",
        Lang::It => "Non c'è più niente da sostituire — i file sono cambiati dopo la ricerca",
    }
}

/// The text moved between the preview being built and the key being pressed: a buffer's revision
/// bumped, or a file's timestamp on disk. Checked for every file before any of them is written,
/// so this refusal always arrives with nothing half done behind it.
pub fn msg_replace_refused_moved(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "A file changed under the preview — nothing was changed, search again",
        Lang::It => "Un file è cambiato sotto l'anteprima — non ho cambiato niente, ricerca di nuovo",
    }
}

/// The preview's title: what becomes what, and how much of it there is.
pub fn msg_replace_preview_title(
    lang: Lang,
    query: &str,
    replacement: &str,
    edits: usize,
    files: usize,
) -> String {
    match lang {
        Lang::En => format!("{query} → {replacement}  ·  {edits} change(s) in {files} file(s)"),
        Lang::It => format!("{query} → {replacement}  ·  {edits} modifiche in {files} file"),
    }
}

/// What happened, counted the way it will have to be taken back.
///
/// The two halves are named separately because they are not the same promise. The buffers took
/// one step of undo each and one Ctrl+Z puts any of them back. The files on disk have no undo at
/// all — the preview was the consent, and saying "N rewritten on disk" out loud is the whole of
/// the honesty available afterwards.
pub fn msg_replace_applied(lang: Lang, edits: usize, buffers: usize, disk: usize) -> String {
    match lang {
        Lang::En => format!(
            "{edits} replacement(s): {buffers} open buffer(s), one undo each · {disk} file(s) rewritten on disk"
        ),
        Lang::It => format!(
            "{edits} sostituzioni: {buffers} buffer aperti, un annulla ciascuno · {disk} file riscritti su disco"
        ),
    }
}

/// A file on disk refused the write — read-only, a full disk, a directory that has gone. Named,
/// because the count on its own would leave the reader guessing which sweep is now half done.
pub fn msg_replace_write_failed(lang: Lang, path: &str, detail: &str) -> String {
    match lang {
        Lang::En => format!("Could not write {path}: {detail} — the sweep stopped there"),
        Lang::It => format!("Non riesco a scrivere {path}: {detail} — la passata si è fermata lì"),
    }
}

pub fn msg_replace_cancelled(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Replace cancelled — nothing was changed",
        Lang::It => "Sostituzione annullata — non ho cambiato niente",
    }
}

/// Said when a pattern will not compile, or gave up. Without it a half-typed pattern is
/// indistinguishable from one that simply matches nothing.
pub fn msg_find_pattern_error(lang: Lang, detail: &str) -> String {
    match lang {
        Lang::En => format!("pattern: {detail}"),
        Lang::It => format!("pattern: {detail}"),
    }
}

/// The same box asks for a line in a buffer and a page in a document, because it is the same
/// question — a number — and a second box for it would be a second thing to learn. It does have
/// to say which, though: being asked for a line while looking at a PDF is a small betrayal.
pub fn msg_goto_prompt(lang: Lang, pages: bool) -> &'static str {
    match (lang, pages) {
        (Lang::En, false) => "Go to line:",
        (Lang::It, false) => "Vai alla riga:",
        (Lang::En, true) => "Go to page:",
        (Lang::It, true) => "Vai alla pagina:",
    }
}

pub fn goto_title(lang: Lang, pages: bool) -> &'static str {
    match (lang, pages) {
        (Lang::En, false) => "Go to line",
        (Lang::It, false) => "Vai alla riga",
        (Lang::En, true) => "Go to page",
        (Lang::It, true) => "Vai alla pagina",
    }
}

pub fn msg_new_entry_prompt(lang: Lang, is_dir: bool) -> &'static str {
    match (lang, is_dir) {
        (Lang::En, false) => "New file name:",
        (Lang::En, true) => "New folder name:",
        (Lang::It, false) => "Nome nuovo file:",
        (Lang::It, true) => "Nome nuova cartella:",
    }
}

/// How many matches Ctrl+A would change, said beside the preview of the first one. Silent at one
/// match, where the preview already shows the whole of what would happen.
pub fn msg_replace_all_count(lang: Lang, count: usize) -> String {
    if count <= 1 {
        return String::new();
    }
    match lang {
        Lang::En => format!("   ({count} in all)"),
        Lang::It => format!("   ({count} in tutto)"),
    }
}

/// What went to which prompt. Says the *count* rather than echoing the code: the code is already
/// on screen in the editor it came from, and the number is the thing you cannot see — it is how
/// you notice you sent one line when you meant a cell.
pub fn msg_run_piece(
    lang: Lang,
    what: crate::session::Piece,
    language: &str,
    lines: usize,
    terminal: usize,
) -> String {
    let selection = what == crate::session::Piece::Selection;
    match (lang, selection) {
        (Lang::En, true) => format!("Selection ({lines} lines) → {language}, terminal {}", terminal + 1),
        (Lang::En, false) => format!("Cell ({lines} lines) → {language}, terminal {}", terminal + 1),
        (Lang::It, true) => format!("Selezione ({lines} righe) → {language}, terminale {}", terminal + 1),
        (Lang::It, false) => format!("Cella ({lines} righe) → {language}, terminale {}", terminal + 1),
    }
}

/// Said after a plot is asked to move. Names the move rather than echoing the command: the
/// command is Octave's business, and what the user did was press an arrow.
/// Said after a double-click in a terminal took you somewhere. Names the file and the line,
/// because the row that was clicked has usually scrolled by the time you look back at it.
pub fn msg_jumped_to(lang: Lang, name: &str, line: usize) -> String {
    match lang {
        Lang::En => format!("{name}, line {line}"),
        Lang::It => format!("{name}, riga {line}"),
    }
}

/// How old a recovery copy is, for the row that offers it.
///
/// Coarse on purpose, and coarser the further back it goes. The question the row answers is "is
/// this the work I remember losing", and to that "4 minutes ago" and "4 minutes and 12 seconds
/// ago" are the same answer — while the second one invites reading a precision into it that a
/// five-second tick does not have.
pub fn msg_recovery_age(lang: Lang, seconds: u64) -> String {
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    match (lang, seconds, minutes, hours) {
        (Lang::En, s, _, _) if s < 60 => "moments ago".to_string(),
        (Lang::It, s, _, _) if s < 60 => "poco fa".to_string(),
        (Lang::En, _, m, _) if m < 60 => format!("{m} min ago"),
        (Lang::It, _, m, _) if m < 60 => format!("{m} min fa"),
        (Lang::En, _, _, h) if h < 24 => format!("{h} h ago"),
        (Lang::It, _, _, h) if h < 24 => format!("{h} h fa"),
        (Lang::En, _, _, _) => format!("{days} d ago"),
        (Lang::It, _, _, _) => format!("{days} g fa"),
    }
}

/// Said once a copy is back in a buffer. The second half is the whole point and is not decoration:
/// nothing has been written to the file, so the work is exactly as unsaved as it was before the
/// session ended, and Ctrl+Z is still holding what is on disk.
pub fn msg_recovery_restored(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("{name} restored, unsaved — Ctrl+S to keep it, Ctrl+Z for the file on disk"),
        Lang::It => {
            format!("{name} ripristinato, non salvato — Ctrl+S per tenerlo, Ctrl+Z per il file su disco")
        }
    }
}

/// Said when a recovery copy could not be written: a full disk, a config directory that is not
/// writable, a home directory that is not there. Once, not every five seconds — see
/// `App::poll_autosave` — and it names the directory, because the fix is always something about
/// that directory and a message that did not say which one would send the user looking.
pub fn msg_recovery_failed(lang: Lang, path: &str, detail: &str) -> String {
    match lang {
        Lang::En => format!("Cannot keep recovery copies in {path}: {detail}"),
        Lang::It => format!("Non riesco a tenere le copie di ripristino in {path}: {detail}"),
    }
}

/// Said when the copy is there but the buffer it belongs in will not take it — a file that has
/// become unreadable, or binary, since the session that was editing it ended.
pub fn msg_recovery_refused(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("{name} cannot hold text any more — the copy is left where it is"),
        Lang::It => format!("{name} non può più contenere testo — la copia resta dov'è"),
    }
}

/// Said when the output named a file that is not in this project. Better than a double-click
/// that quietly does nothing, and better than opening a file of that name from somewhere else.
pub fn msg_jump_not_found(lang: Lang, path: &str) -> String {
    match lang {
        Lang::En => format!("\"{path}\" is not in this project"),
        Lang::It => format!("\"{path}\" non è in questo progetto"),
    }
}

pub fn msg_figure_nav(lang: Lang, nav: crate::session::Nav, is3d: bool) -> String {
    use crate::session::Nav;
    let en = match (nav, is3d) {
        (Nav::In, _) => "Zooming in",
        (Nav::Out, _) => "Zooming out",
        (Nav::Reset, _) => "Back to the whole plot",
        (_, true) => "Turning it",
        (_, false) => "Panning",
    };
    let it = match (nav, is3d) {
        (Nav::In, _) => "Ingrandisco",
        (Nav::Out, _) => "Allargo",
        (Nav::Reset, _) => "Torno al grafico intero",
        (_, true) => "Lo giro",
        (_, false) => "Sposto",
    };
    // Redrawn by the session, so the numbers on the axes are redrawn with it.
    match lang {
        Lang::En => format!("{en} — the session is redrawing it"),
        Lang::It => format!("{it} — la sessione lo sta ridisegnando"),
    }
}

/// Says the name rather than the whole path: the file lands beside the project, which is where
/// you were going to look for it anyway.
pub fn msg_figure_exported(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("Written to {name}, in the project folder"),
        Lang::It => format!("Scritto in {name}, nella cartella del progetto"),
    }
}

/// Said when there is nothing to inspect. Not an error: an editor with no interpreter running
/// is the ordinary case, and the sentence says what would make it possible.
/// Set or cleared, and where. Says the line because the cursor may have moved on by the time
/// you look, and a breakpoint you cannot find is one you will trip over later.
pub fn msg_breakpoint(lang: Lang, on: bool, name: &str, line: usize) -> String {
    match (lang, on) {
        (Lang::En, true) => format!("Breakpoint at {name}:{line}"),
        (Lang::En, false) => format!("Breakpoint cleared at {name}:{line}"),
        (Lang::It, true) => format!("Breakpoint a {name}:{line}"),
        (Lang::It, false) => format!("Breakpoint tolto a {name}:{line}"),
    }
}

pub fn msg_break_unsaved(lang: Lang) -> String {
    match lang {
        Lang::En => "Save the file first — a breakpoint is set in a file, not in a buffer",
        Lang::It => "Salva prima il file — un breakpoint sta in un file, non in un buffer",
    }
    .to_string()
}

/// The key hint a menu shows, in the words printed on the reader's keyboard.
///
/// Nearly every shortcut here is Ctrl and a letter, which is the same key everywhere. The
/// odd one out is the delete key: an Italian keyboard has `Canc` written on it and no `Del`
/// anywhere, and this project already bends its bindings around that layout — telling
/// somebody to press a key their keyboard does not have is the same mistake in smaller type.
pub fn shortcut_label(lang: Lang, shortcut: &str) -> &str {
    match (lang, shortcut) {
        (Lang::It, "Del") => "Canc",
        _ => shortcut,
    }
}

/// What went wrong in `[keys]`, said on the status line at startup rather than raised as an
/// error. Each of these leaves the default chord where it was, so the sentence is about a
/// setting that did not take effect, not about an editor that failed to start.
///
/// The offending text is quoted back verbatim in every one of them. A message that only said
/// "unknown action" would leave the reader looking for it in a file they have written by hand.
pub fn msg_keys_unknown_action(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("[keys]: no action is called \"{name}\" — see Keybindings... for the list"),
        Lang::It => {
            format!("[keys]: nessuna azione si chiama \"{name}\" — la lista è in Scorciatoie...")
        }
    }
}

pub fn msg_keys_bad_chord(lang: Lang, name: &str, chord: &str) -> String {
    match lang {
        Lang::En => format!("[keys]: \"{chord}\" is not a chord — {name} keeps its usual key"),
        Lang::It => format!("[keys]: \"{chord}\" non è una corda — {name} tiene il tasto di sempre"),
    }
}

/// Two actions on one chord. Which one wins is not a coin toss and the message says so, because
/// the other one has silently stopped working and that is the thing worth knowing.
pub fn msg_keys_conflict(lang: Lang, chord: &str, winner: &str, loser: &str) -> String {
    match lang {
        Lang::En => format!("[keys]: {chord} is on both {winner} and {loser} — {winner} wins"),
        Lang::It => format!("[keys]: {chord} sta su {winner} e su {loser} — vince {winner}"),
    }
}

pub fn msg_keys_reloaded(lang: Lang) -> String {
    match lang {
        Lang::En => "Keybindings reloaded from settings.toml".to_string(),
        Lang::It => "Scorciatoie ricaricate da settings.toml".to_string(),
    }
}

pub fn msg_keys_no_config_dir(lang: Lang) -> String {
    match lang {
        Lang::En => "No config directory to keep settings.toml in".to_string(),
        Lang::It => "Nessuna cartella di configurazione dove tenere settings.toml".to_string(),
    }
}

/// The comment written above the `[keys]` table CleeCode seeds into settings.toml. Lines, not
/// one string, because each has to come back out with a `#` in front of it and a hard-wrapped
/// paragraph is the only way a comment block stays inside eighty columns in both languages.
pub fn keys_section_header(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::En => &[
            "",
            "# Keyboard chords. Uncomment a line and change its chord to move that action; every",
            "# action left commented out keeps the key CleeCode ships with. Modifiers are ctrl,",
            "# shift and alt, joined to the key with +, and the key may be a letter, a digit,",
            "# F1 to F12, an arrow (left/right/up/down), enter, tab, esc or space.",
            "#",
            "# Two actions on one chord is allowed and reported: the one listed first here wins,",
            "# and the other stops firing. Changes take effect when this file is saved.",
        ],
        Lang::It => &[
            "",
            "# Corde da tastiera. Togli il commento a una riga e cambiale la corda per spostare",
            "# quell'azione; ogni azione lasciata commentata tiene il tasto con cui CleeCode",
            "# arriva. I modificatori sono ctrl, shift e alt, uniti al tasto con +, e il tasto",
            "# può essere una lettera, una cifra, da F1 a F12, una freccia (left/right/up/down),",
            "# enter, tab, esc o space.",
            "#",
            "# Due azioni sulla stessa corda si può, e viene segnalato: vince quella elencata",
            "# per prima qui sotto, l'altra smette di scattare. Le modifiche valgono al salvataggio.",
        ],
    }
}

pub fn msg_workspace_panel(lang: Lang) -> String {
    match lang {
        Lang::En => "Watching for a session — start octave or python in a terminal".to_string(),
        Lang::It => "In ascolto di una sessione — avvia octave o python in un terminale".to_string(),
    }
}

/// Says where, and what to type next. The stepping commands are the one part of this the
/// editor does not drive — `dbstep` from inside Octave's hook returns without an error and
/// without moving, measured, and Python's stepping belongs to pdb's own prompt — so the
/// sentence points at the prompt where they do work.
///
/// Which words it points at depends on the language, because they are different words and a
/// message telling a Python user to type `dbstep` is worse than no message.
pub fn msg_debug_stopped(lang: Lang, name: &str, line: usize, steps: &str) -> String {
    match lang {
        Lang::En => format!("Stopped in {name}, line {line} — {steps} at the prompt"),
        Lang::It => format!("Fermo in {name}, riga {line} — {steps} al prompt"),
    }
}

pub fn msg_debug_running(lang: Lang) -> String {
    match lang {
        Lang::En => "Running again",
        Lang::It => "Riparte",
    }
    .to_string()
}

// ---- The debug adapter ---------------------------------------------------------------------
//
// Separate from the four sentences above, which belong to the interpreter debuggers, because
// they are said about a different kind of session — a compiled program under lldb-dap or gdb.
// `msg_debug_running` is the one they share, and shared on purpose: a program that has started
// moving again is the same news whichever debugger is watching it.

/// What to install, in the words of the platform the reader is on.
///
/// `os` is `std::env::consts::OS` at the call site rather than read here, so all three sentences
/// can be tested on one machine — a message that is only ever seen on the platform nobody
/// building this has is exactly the message that goes wrong.
///
/// The list of adapters comes from [`crate::dap::ADAPTERS_WANTED`], which is the list
/// `find_adapter` actually looks for: two lists would drift, and the one that drifted would be
/// this one.
pub fn msg_debugger_no_adapter(lang: Lang, os: &str) -> String {
    let wanted = crate::dap::ADAPTERS_WANTED.join(", ");
    match (lang, os) {
        (Lang::En, "macos") => format!(
            "No debug adapter ({wanted}): lldb-dap comes with the Xcode command-line tools, xcode-select --install"
        ),
        (Lang::En, "linux") => format!(
            "No debug adapter ({wanted}): install lldb-dap from your LLVM packages, or a gdb 14 or newer"
        ),
        (Lang::En, "windows") => format!(
            "No debug adapter ({wanted}): gdb from MSYS2 for GNU-toolchain binaries, lldb-dap from the LLVM installer for MSVC ones"
        ),
        // Anything else is a machine nobody here has held. It gets the honest version: the names
        // to look for, and where to write down whatever it is called there.
        (Lang::En, _) => format!(
            "No debug adapter ({wanted}): put one on PATH, or name yours as debug_adapter in settings.toml"
        ),
        (Lang::It, "macos") => format!(
            "Nessun debug adapter ({wanted}): lldb-dap arriva con gli strumenti da riga di comando di Xcode, xcode-select --install"
        ),
        (Lang::It, "linux") => format!(
            "Nessun debug adapter ({wanted}): installa lldb-dap dai pacchetti LLVM, oppure un gdb 14 o piu recente"
        ),
        (Lang::It, "windows") => format!(
            "Nessun debug adapter ({wanted}): gdb da MSYS2 per i binari GNU, lldb-dap dall'installer LLVM per quelli MSVC"
        ),
        (Lang::It, _) => format!(
            "Nessun debug adapter ({wanted}): mettine uno nel PATH, o scrivi il tuo come debug_adapter in settings.toml"
        ),
    }
}

/// The guess pointed at something that is not a file. Names it, because the whole point of a
/// filled-in guess is that the reader can see what it guessed.
pub fn msg_debugger_no_debuggee(lang: Lang, program: &str) -> String {
    match lang {
        Lang::En => format!("Nothing to debug at {program} — build it first, or set debuggee in the workspace file"),
        Lang::It => format!("Niente da debuggare in {program} — compilalo, o scrivi debuggee nel file di workspace"),
    }
}

/// The adapter would not start at all: it is not where it was said to be, or it died on the
/// handshake. Whatever it said comes through unedited.
pub fn msg_debugger_adapter_failed(lang: Lang, reason: &str) -> String {
    match lang {
        Lang::En => format!("The debug adapter would not start: {reason}"),
        Lang::It => format!("Il debug adapter non parte: {reason}"),
    }
}

pub fn msg_debugger_started(lang: Lang, adapter: &str, program: &str) -> String {
    match lang {
        Lang::En => format!("Debugging {program} with {adapter}"),
        Lang::It => format!("Debug di {program} con {adapter}"),
    }
}

/// Why it stopped, in the adapter's own word — "breakpoint", "step", "exception" — or in its own
/// sentence where it wrote one.
pub fn msg_debugger_stopped(lang: Lang, why: &str) -> String {
    match lang {
        Lang::En => format!("Stopped: {why}"),
        Lang::It => format!("Fermo: {why}"),
    }
}

pub fn msg_debugger_exited(lang: Lang, code: i64) -> String {
    match lang {
        Lang::En => format!("The program exited with {code}"),
        Lang::It => format!("Il programma è uscito con {code}"),
    }
}

/// The session ended without the program having reported a status of its own — which is what a
/// `terminated` with no `exited` in front of it means.
pub fn msg_debugger_over(lang: Lang) -> String {
    match lang {
        Lang::En => "The debug session is over",
        Lang::It => "La sessione di debug è finita",
    }
    .to_string()
}

pub fn msg_debugger_ended(lang: Lang, program: &str) -> String {
    match lang {
        Lang::En => format!("Stopped debugging {program}"),
        Lang::It => format!("Debug di {program} fermato"),
    }
}

/// A request the adapter refused, named with the request so that "not available" is attached to
/// the thing that is not available.
pub fn msg_debugger_refused(lang: Lang, command: &str, message: &str) -> String {
    match lang {
        Lang::En => format!("The adapter refused {command}: {message}"),
        Lang::It => format!("L'adapter ha rifiutato {command}: {message}"),
    }
}

/// The adapter stopped talking. Not a failure of the editor and not phrased as one: a debug
/// session ending is the ordinary end of every debug session there has ever been.
pub fn msg_debugger_dead(lang: Lang, reason: &str) -> String {
    match lang {
        Lang::En => format!("The debug session ended: {reason}"),
        Lang::It => format!("La sessione di debug è finita: {reason}"),
    }
}

pub fn msg_debugger_no_session(lang: Lang) -> String {
    match lang {
        Lang::En => "Nothing is being debugged — Debug ▸ Start debugging first",
        Lang::It => "Non c'è niente in debug — prima Debug ▸ Avvia il debug",
    }
    .to_string()
}

/// Asked to step a program that is running. The refusal says what would make it work, because
/// "not stopped" on its own reads as a fault rather than as a state.
pub fn msg_debugger_not_stopped(lang: Lang) -> String {
    match lang {
        Lang::En => "The program is running — it steps once it stops at a breakpoint",
        Lang::It => "Il programma sta girando — si avanza quando si ferma a un breakpoint",
    }
    .to_string()
}

/// The title on the box *Debug ▸ Start debugging* now opens, and the line inside it.
///
/// Two pieces of text rather than one because that is the shape every other single-line box in
/// this editor has — a title saying which question this is, and the question itself over the
/// answer. The title repeats the menu row's own words so that the box is visibly the thing the
/// row opened, and the prompt says *program*, which is the word for what a debugger is pointed at.
pub fn debuggee_title(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Start debugging",
        Lang::It => "Avvia il debug",
    }
}

/// Says what emptying the box does, for the same reason [`msg_run_command_prompt`] does: the
/// answer is prefilled, so the only way to find out what an empty one means is to be told.
pub fn msg_debuggee_prompt(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Program to debug (empty to go back to the guess):",
        Lang::It => "Programma da debuggare (vuoto per tornare alla proposta):",
    }
}

/// The other box the panel opens: one watch expression, in the debuggee's own language.
pub fn watch_title(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Watch",
        Lang::It => "Espressione",
    }
}

pub fn msg_watch_prompt(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Expression to watch:",
        Lang::It => "Espressione da sorvegliare:",
    }
}

/// The row of letters along the bottom of the panel.
///
/// Written out where the hands are, the way the git panel writes its own keys along the bottom of
/// itself: these letters only do anything while this frame has the focus, so the frame is the only
/// place they can honestly be advertised. The verbs are gdb's own spellings, which is the whole
/// reason they were chosen — see the design's "Keys" section.
pub fn msg_debug_panel_keys(lang: Lang) -> String {
    match lang {
        Lang::En => "c run  n over  s into  o out  w watch  d drop  x stop",
        Lang::It => "c va  n sopra  s dentro  o fuori  w espr  d togli  x ferma",
    }
    .to_string()
}

pub fn msg_debug_panel_toggled(lang: Lang, open: bool) -> String {
    match (lang, open) {
        (Lang::En, true) => "Debug panel shown",
        (Lang::En, false) => "Debug panel hidden",
        (Lang::It, true) => "Pannello di debug mostrato",
        (Lang::It, false) => "Pannello di debug nascosto",
    }
    .to_string()
}

pub fn msg_watch_added(lang: Lang, expression: &str) -> String {
    match lang {
        Lang::En => format!("Watching {expression}"),
        Lang::It => format!("Sorveglio {expression}"),
    }
}

pub fn msg_watch_removed(lang: Lang, expression: &str) -> String {
    match lang {
        Lang::En => format!("Stopped watching {expression}"),
        Lang::It => format!("Non sorveglio più {expression}"),
    }
}

/// One session at a time. See [`crate::app::DebugSession`] for why that is a decision.
pub fn msg_debugger_already_running(lang: Lang) -> String {
    match lang {
        Lang::En => "A debug session is already running — stop it first",
        Lang::It => "C'è già una sessione di debug — fermala prima",
    }
    .to_string()
}

pub fn msg_inspect_waiting(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Asking the session…",
        Lang::It => "Sto chiedendo alla sessione…",
    }
}

pub fn msg_inspect_hint(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "arrows page · Home to the corner · R ask again · Esc close",
        Lang::It => "frecce sfogliano · Home all'angolo · R richiede · Esc chiude",
    }
}

pub fn msg_inspect_no_session(lang: Lang) -> String {
    match lang {
        Lang::En => "No live session — start Octave or Python in a terminal first".to_string(),
        Lang::It => "Nessuna sessione viva — avvia prima Octave o Python in un terminale".to_string(),
    }
}

pub fn msg_figure_no_session(lang: Lang, language: &str) -> String {
    match lang {
        Lang::En => format!("The {language} session that drew this is gone — the picture stays"),
        Lang::It => format!("La sessione {language} che l'ha disegnato non c'è più — resta l'immagine"),
    }
}

pub fn msg_run_piece_unsaved(lang: Lang) -> String {
    match lang {
        Lang::En => "Save the file first — an interpreter is pointed at a file, not at a buffer",
        Lang::It => "Salva prima il file — a un interprete si indica un file, non un buffer",
    }
    .to_string()
}

/// Said after context went to an agent's prompt. It names what was sent and where it went, and
/// it says the part that matters: nothing has been submitted, and Enter is the user's to press.
/// Where the context landed: a numbered terminal, or the drawer.
///
/// The drawer is named rather than numbered because it has no number — it is not one of the
/// terminal panel's windows, and calling it "terminal 3" would send the reader looking for a
/// third terminal that is not there.
pub fn msg_agent_sent_to_terminal(lang: Lang, reference: &str, agent: &str, terminal: usize) -> String {
    match lang {
        Lang::En => {
            format!("{reference} → {agent}, terminal {} — Enter sends it", terminal + 1)
        }
        Lang::It => {
            format!("{reference} → {agent}, terminale {} — Enter lo manda", terminal + 1)
        }
    }
}

pub fn msg_agent_sent_to_drawer(lang: Lang, reference: &str, agent: &str) -> String {
    match lang {
        Lang::En => format!("{reference} → {agent}, in the drawer — Enter sends it"),
        Lang::It => format!("{reference} → {agent}, nel cassetto — Enter lo manda"),
    }
}

/// What `Ctrl+Shift+A` says when there was nobody to talk to. The key does not fail: it opens
/// the drawer on its launcher, which is where an agent is chosen, so the message says what
/// just happened rather than what did not.
pub fn msg_drawer_summoned(lang: Lang) -> String {
    match lang {
        Lang::En => "No agent running — the drawer is open: choose one and press Enter",
        Lang::It => "Nessun agente in esecuzione — cassetto aperto: scegline uno e premi Invio",
    }
    .to_string()
}

/// The drawer opened or closed from the menu. Says that closing is not killing, because that is
/// the part nobody can see.
pub fn msg_drawer_toggled(lang: Lang, open: bool) -> String {
    match (lang, open) {
        (Lang::En, true) => "Agent drawer open",
        (Lang::En, false) => "Agent drawer hidden — the agent is still running in it",
        (Lang::It, true) => "Cassetto agente aperto",
        (Lang::It, false) => "Cassetto agente nascosto — l'agente dentro continua a girare",
    }
    .to_string()
}

pub fn msg_drawer_started(lang: Lang, agent: &str) -> String {
    match lang {
        Lang::En => format!("{agent} started in the drawer"),
        Lang::It => format!("{agent} avviato nel cassetto"),
    }
}

/// The agent in the drawer has exited. Nothing takes its place: a shell appearing where an agent
/// was looks exactly like the agent still being there, so the launcher comes back instead.
pub fn msg_drawer_agent_ended(lang: Lang, agent: &str) -> String {
    match lang {
        Lang::En => format!("{agent} has ended — the drawer is back to the list"),
        Lang::It => format!("{agent} è terminato — il cassetto torna alla lista"),
    }
}

/// Said when `clee -w NAME` is typed with one of the four retired agent names — `claude` and its
/// three siblings, which opened a preset before 0.16 and open nothing now. A hard cut, not a
/// phase-out: there was no deprecation release for these to have lived through, so the first and
/// only word about it is this one, at the moment someone reaches for the command that used to
/// work. Two facts, one line: the name is gone, and what replaced it.
pub fn msg_agent_preset_retired(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("{name} is no longer a preset — Ctrl+Shift+A opens the agent drawer"),
        Lang::It => format!("{name} non è più un preset — Ctrl+Shift+A apre il cassetto agente"),
    }
}

pub fn msg_drawer_start_error(lang: Lang, agent: &str, error: &str) -> String {
    match lang {
        Lang::En => format!("Could not start {agent} in the drawer: {error}"),
        Lang::It => format!("Impossibile avviare {agent} nel cassetto: {error}"),
    }
}

// ---- What an agent does to the editor, said on the status line -------------------------------

// Everything below is the corner-of-the-eye half of the MCP bridge: an agent that opened a file,
// rendered one, said something or changed a buffer, reported in one line while the user goes on
// working. All of them name the file, because "the agent did something" is not information, and
// none of them is longer than a line — the status bar has exactly one.

/// One line from the agent itself. Marked as coming from it and not from the editor, because
/// everything else in this bar is CleeCode speaking and the difference matters: what follows is
/// text a language model wrote.
pub fn msg_agent_says(lang: Lang, text: &str) -> String {
    match lang {
        Lang::En => format!("agent: {text}"),
        Lang::It => format!("agente: {text}"),
    }
}

/// A file an agent asked to be shown. Names the line when it named one, since that is the part
/// that says *why* the file appeared.
pub fn msg_agent_opened(lang: Lang, path: &str, line: Option<usize>) -> String {
    match (lang, line) {
        (Lang::En, Some(line)) => format!("agent opened {path}:{line}"),
        (Lang::En, None) => format!("agent opened {path}"),
        (Lang::It, Some(line)) => format!("l'agente ha aperto {path}:{line}"),
        (Lang::It, None) => format!("l'agente ha aperto {path}"),
    }
}

pub fn msg_agent_previewed(lang: Lang, path: &str) -> String {
    match lang {
        Lang::En => format!("agent is showing {path}"),
        Lang::It => format!("l'agente sta mostrando {path}"),
    }
}

/// The question asked before an agent may write into unsaved work.
///
/// It names the file and the size of the change, because "one line for two" and "eighty lines for
/// none" are not the same question, and it prints its three keys for the reason [`yes_key`] gives:
/// a prompt whose letters are not in its text is a prompt that looks broken while working exactly
/// as written. `A` is the one that is neither yes nor no — it says yes to everything the agent
/// asks for the rest of this session, and stops when CleeCode does.
pub fn msg_agent_edit_confirm(lang: Lang, path: &str, added: usize, removed: usize) -> String {
    let yes = yes_key(lang).to_ascii_uppercase();
    match lang {
        Lang::En => format!(
            "The agent wants to change {path} (+{added}/-{removed} lines) — unsaved work. \
             {yes} once · A this whole session · N no"
        ),
        Lang::It => format!(
            "L'agente vuole modificare {path} (+{added}/-{removed} righe) — lavoro non salvato. \
             {yes} una volta · A tutta la sessione · N no"
        ),
    }
}

/// Said after the change lands, naming where it landed and that the file on disk has not moved.
/// The second half is the part the user has to be told: their buffer and their file now disagree,
/// and nothing but them saving will settle it.
pub fn msg_agent_edited(lang: Lang, path: &str, line: usize) -> String {
    match lang {
        Lang::En => format!("agent edited {path}:{line} — not saved, Ctrl+Z undoes it"),
        Lang::It => format!("l'agente ha modificato {path}:{line} — non salvato, Ctrl+Z annulla"),
    }
}

/// Said when the question is answered with anything but yes, so a refusal visibly did nothing
/// rather than invisibly doing something. The same reason `msg_scp_cancelled` exists.
pub fn msg_agent_edit_declined(lang: Lang, path: &str) -> String {
    match lang {
        Lang::En => format!("{path} left alone — the agent has been told"),
        Lang::It => format!("{path} lasciato com'era — l'agente è stato avvisato"),
    }
}

/// A buffer with no file behind it. There is nothing to point an agent at, and the honest
/// answer is to say what is missing rather than to send it a name that means nothing.
pub fn msg_agent_unsaved(lang: Lang) -> String {
    match lang {
        Lang::En => "Save the file first — an agent is pointed at a path, not at a buffer",
        Lang::It => "Salva prima il file — a un agente si indica un percorso, non un buffer",
    }
    .to_string()
}

pub fn msg_run_piece_no_language(lang: Lang, ext: &str) -> String {
    match lang {
        Lang::En => format!("No live session for .{ext} — this works in Octave and Python"),
        Lang::It => format!("Nessuna sessione viva per .{ext} — funziona con Octave e Python"),
    }
}

pub fn msg_run_piece_empty(lang: Lang) -> String {
    match lang {
        Lang::En => "Nothing to run there",
        Lang::It => "Non c'è niente da eseguire lì",
    }
    .to_string()
}

pub fn msg_run_piece_no_scratch(lang: Lang) -> String {
    match lang {
        Lang::En => "Could not write the piece to a temporary file",
        Lang::It => "Non sono riuscito a scrivere il pezzo in un file temporaneo",
    }
    .to_string()
}

/// Said once, when a server a file would have used turns out not to be installed. Worded as a
/// fact about the machine rather than as an error, because it is not one: the editor works
/// exactly as it did before, minus the underlines.
pub fn msg_lsp_missing(lang: Lang, program: &str) -> String {
    match lang {
        Lang::En => format!("{program} is not installed — editing without diagnostics"),
        Lang::It => format!("{program} non è installato — si modifica senza diagnostici"),
    }
}

pub fn msg_lsp_needs_saving(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Save the file first — a server can only look at what is on disk",
        Lang::It => "Salva prima il file — un server guarda solo quello che è su disco",
    }
}

/// Said when the key is pressed in a file no server serves — a `.txt`, or a language nothing is
/// installed for. Named as a fact about this file rather than as a failure: nothing went wrong.
pub fn msg_lsp_none_here(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "No language server for this file",
        Lang::It => "Nessun language server per questo file",
    }
}

pub fn msg_lsp_looking(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Asking where that is defined…",
        Lang::It => "Chiedo dov'è definito…",
    }
}

/// An answer, and worth saying out loud. A key that does nothing and says nothing is a key you
/// press again harder — and this is the common case on a keyword, in a comment, or on a name the
/// server has not finished indexing.
pub fn msg_lsp_no_definition(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "The server knows of no definition for that",
        Lang::It => "Il server non conosce nessuna definizione per quello",
    }
}

pub fn msg_lsp_looking_references(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Asking where that is used…",
        Lang::It => "Chiedo dov'è usato…",
    }
}

pub fn msg_lsp_looking_symbols(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Asking what is in this file…",
        Lang::It => "Chiedo cosa c'è in questo file…",
    }
}

/// The same kind of answer as [`msg_lsp_no_definition`], and said for the same reason: on a
/// keyword, in a comment, or before the server has finished indexing, an empty list is what
/// comes back and an empty picker would look like a bug.
pub fn msg_lsp_no_references(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "The server knows of nowhere that uses it",
        Lang::It => "Il server non conosce nessun posto che lo usa",
    }
}

pub fn msg_lsp_no_symbols(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "The server sees no names in this file",
        Lang::It => "Il server non vede nomi in questo file",
    }
}

/// Said when the diagnostics list is asked for and there is nothing in it.
///
/// Worded to say which files it looked at, because that is the honest scope: a server only
/// speaks about the files it was told about, and those are the ones with a tab.
pub fn msg_lsp_no_diagnostics(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Nothing to report — no diagnostics on the open files",
        Lang::It => "Niente da segnalare — nessun diagnostico sui file aperti",
    }
}

pub fn msg_lsp_nowhere_back(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "You have not jumped anywhere to come back from",
        Lang::It => "Non sei saltato da nessuna parte da cui tornare",
    }
}

// ---- Renaming a name ------------------------------------------------------------------------
//
// The wordiest family of messages in this file, and on purpose. Every one of them below the
// prompt is a *refusal*: the rename was asked for, the server answered, and the editor is not
// going to do what it said. A refusal that does not say which of half a dozen reasons it was
// leaves the reader with nothing to change, and the one thing worse than a rename that will not
// happen is a rename that will not happen for no stated reason.

/// The box: what is being renamed, and the two keys that end it.
pub fn msg_rename_symbol_prompt(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("Rename '{name}' to (Enter = ask, Esc = cancel):"),
        Lang::It => format!("Rinomina '{name}' in (Invio = chiedi, Esc = annulla):"),
    }
}

/// Said when the key is pressed somewhere there is no name: in the indentation, on a bracket, on
/// a blank line. Nothing opens, because there would be nothing to put in the box.
pub fn msg_rename_nothing_here(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "No name under the cursor to rename",
        Lang::It => "Nessun nome sotto il cursore da rinominare",
    }
}

pub fn msg_rename_asking(lang: Lang, old_name: &str, new_name: &str) -> String {
    match lang {
        Lang::En => format!("Asking what it takes to rename {old_name} to {new_name}…"),
        Lang::It => format!("Chiedo cosa serve per rinominare {old_name} in {new_name}…"),
    }
}

/// The server answered, and its answer was that nothing would change. An answer, and said out
/// loud for the same reason the absent definition is.
pub fn msg_rename_no_changes(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "The server would change nothing for that rename",
        Lang::It => "Il server non cambierebbe niente per quel rename",
    }
}

/// The server wanted files created, moved or deleted as well as edited.
///
/// Refused whole rather than in part. Applying the edits and skipping the file operations would
/// carry out half of what the server asked for and report it as all of it — and the half left
/// out is the half that moves files around on disk.
pub fn msg_rename_refused_file_ops(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "That rename would also create, move or delete files — nothing was changed",
        Lang::It => "Quel rename creerebbe, sposterebbe o cancellerebbe file — non ho cambiato niente",
    }
}

/// One of the edits covers more than one line, which this cannot show and therefore will not do.
pub fn msg_rename_refused_multiline(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "One of the server's edits spans several lines — nothing was changed",
        Lang::It => "Una delle modifiche del server copre più righe — non ho cambiato niente",
    }
}

/// Two of the edits cover the same text, so their order would decide the result.
pub fn msg_rename_refused_overlap(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Two of the server's edits overlap — nothing was changed",
        Lang::It => "Due modifiche del server si sovrappongono — non ho cambiato niente",
    }
}

/// The rename reaches files no tab holds. The count is the whole message: it is what turns a
/// refusal into an instruction, since opening those files and pressing the key again works.
pub fn msg_rename_refused_outside(lang: Lang, count: usize) -> String {
    match lang {
        Lang::En => format!("{count} file(s) this rename touches are not open — open them and try again"),
        Lang::It => format!("{count} file toccati da questo rename non sono aperti — aprili e riprova"),
    }
}

/// One of the tabs it would write to cannot be typed in — a preview, or a file opened read-only.
pub fn msg_rename_refused_read_only(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "One of the files this rename touches is open read-only — nothing was changed",
        Lang::It => "Uno dei file toccati da questo rename è aperto in sola lettura — non ho cambiato niente",
    }
}

/// The text moved between the preview being built and the key being pressed.
///
/// The one refusal that happens after something was already shown on screen, which is why it
/// says so: a file reloaded from disk under an open preview would have the edits applied at
/// character offsets measured against text that is no longer there.
pub fn msg_rename_refused_moved(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "The text changed under the preview — nothing was changed, ask again",
        Lang::It => "Il testo è cambiato sotto l'anteprima — non ho cambiato niente, richiedi",
    }
}

/// The preview's title: what is being renamed to what, and how much of it there is.
///
/// An empty `old_name` is a preview that is not a rename — a code action reaching more than one
/// buffer comes up in the same box, and there is no name it is turning into another. What is left
/// is the server's own title for what it is about to do, which is the whole caption: an arrow with
/// nothing on its left would be a sentence with a word missing.
pub fn msg_rename_preview_title(
    lang: Lang,
    old_name: &str,
    new_name: &str,
    edits: usize,
    files: usize,
) -> String {
    let what = if old_name.is_empty() {
        new_name.to_string()
    } else {
        format!("{old_name} → {new_name}")
    };
    match lang {
        Lang::En => format!("{what}  ·  {edits} change(s) in {files} file(s)"),
        Lang::It => format!("{what}  ·  {edits} modifiche in {files} file"),
    }
}

/// One file's header row in a preview, written the way a diff writes one so it reads as one.
///
/// Not `msg_rename_*`, because the rename is no longer the only thing that shows a file's worth
/// of changes before making them: the sweep across the project draws the same rows. One header,
/// so two previews cannot come to look like two different features.
pub fn msg_preview_file_header(lang: Lang, path: &str, count: usize) -> String {
    match lang {
        Lang::En => format!("--- {path}  ({count} change(s))"),
        Lang::It => format!("--- {path}  ({count} modifiche)"),
    }
}

/// A preview's footer, shared by both of them. Spelled out for the same reason the git panel's
/// is: these are bare keys, safe only while the box owns the keyboard, and discoverable only if
/// the box says so.
pub fn msg_preview_keys(lang: Lang) -> String {
    let yes = yes_key(lang).to_ascii_uppercase();
    match lang {
        Lang::En => format!("Enter or {yes} = apply   Esc = cancel   ↑↓ PgUp/PgDn = scroll"),
        Lang::It => format!("Invio o {yes} = applica   Esc = annulla   ↑↓ PgSu/PgGiù = scorri"),
    }
}

/// What happened, in the same numbers the preview showed, so the two can be compared.
///
/// An empty `old_name` means the same here as it does in the title above, and reads as what it is:
/// an action was carried out, and it has a name of the server's own rather than two of ours.
pub fn msg_rename_applied(
    lang: Lang,
    old_name: &str,
    new_name: &str,
    edits: usize,
    files: usize,
) -> String {
    if old_name.is_empty() {
        return match lang {
            Lang::En => format!("{new_name}: {edits} change(s) in {files} file(s)"),
            Lang::It => format!("{new_name}: {edits} modifiche in {files} file"),
        };
    }
    match lang {
        Lang::En => format!("Renamed {old_name} to {new_name}: {edits} change(s) in {files} file(s)"),
        Lang::It => format!("Rinominato {old_name} in {new_name}: {edits} modifiche in {files} file"),
    }
}

pub fn msg_rename_cancelled(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Rename cancelled — nothing was changed",
        Lang::It => "Rename annullato — non ho cambiato niente",
    }
}

/// Esc over the same box when what it was showing was not a rename. Its own sentence rather than
/// the one above, because the one above names the thing that did not happen — and telling somebody
/// a rename was cancelled when they cancelled a quick fix is telling them about a different key.
pub fn msg_edit_preview_cancelled(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Cancelled — nothing was changed",
        Lang::It => "Annullato — non ho cambiato niente",
    }
}

// ---- Laying the file out ---------------------------------------------------------------------
//
// Shorter than the family above, and that is the difference between the two features rather than
// an oversight: a format has no preview to cancel, no second file to be outside, and no name to
// say twice. What is left is the line that says the question went out, the one that says nothing
// needed doing, the one that says what was done, and the three refusals.

pub fn msg_format_asking(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Asking the server how this file should be laid out…",
        Lang::It => "Chiedo al server come va impaginato questo file…",
    }
}

/// The server answered with no edits at all, which is an answer: the file is already laid out the
/// way it would lay it out.
///
/// Said out loud, and it is the message in this family that most needed writing. A key that does
/// nothing visible is a key you press again, and "nothing happened" and "nothing needed to happen"
/// look identical from the outside.
pub fn msg_format_already(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "The server would change nothing — already formatted",
        Lang::It => "Il server non cambierebbe niente — già formattato",
    }
}

/// What was done, in the server's own count of edits, and the promise that it is one step.
pub fn msg_format_applied(lang: Lang, edits: usize) -> String {
    match lang {
        Lang::En => format!("Formatted: {edits} change(s), one Ctrl+Z takes them all back"),
        Lang::It => format!("Formattato: {edits} modifiche, un Ctrl+Z le rimette tutte a posto"),
    }
}

/// The buffer cannot be typed in — a preview, or a file opened read-only. Said when the key is
/// pressed rather than after a round trip, so it does not read as the server having refused.
pub fn msg_format_read_only(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "This file is open read-only — nothing was changed",
        Lang::It => "Questo file è aperto in sola lettura — non ho cambiato niente",
    }
}

/// Two of the edits cover the same text, so their order would decide the result.
pub fn msg_format_refused_overlap(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Two of the server's edits overlap — nothing was changed",
        Lang::It => "Due modifiche del server si sovrappongono — non ho cambiato niente",
    }
}

/// The buffer moved between the question and the answer: closed, reloaded from disk, or edited
/// into a shape with fewer lines than the server is describing.
///
/// One message for all three, unlike the rename's two, because the reader's move is the same in
/// every one of them — press the key again — and a refusal that distinguished them would be
/// distinguishing causes nobody can act on differently.
pub fn msg_format_refused_moved(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "The text changed while the server was answering — nothing was changed, ask again",
        Lang::It => "Il testo è cambiato mentre il server rispondeva — non ho cambiato niente, richiedi",
    }
}

// ---- What the server offers to do about it ----------------------------------------------------
//
// Shorter again than the format's family, and for the same kind of reason: what could go wrong
// here mostly goes wrong somewhere that already has words for it. An action that reaches more than
// one buffer is refused by the rename's sentences, and one that reaches a single buffer by the
// format's — this is the handful that belong to the question itself, which is asked, answered with
// a list, and then answered again for the row that was picked.

pub fn msg_code_actions_asking(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Asking what can be done here…",
        Lang::It => "Chiedo cosa si può fare qui…",
    }
}

/// The server answers this request for nobody. Said the moment the row is picked rather than after
/// a round trip, because a server that never claimed the request would answer with a
/// method-not-found — and the status line would print it as though it were news.
pub fn msg_code_actions_unsupported(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "This server does not offer code actions",
        Lang::It => "Questo server non offre azioni sul codice",
    }
}

/// The server answered, and its answer was that there is nothing to do here.
///
/// The commonest answer of all — in the middle of a line that is not wrong there usually is
/// nothing — and the one that most needed writing down: a list that did not open and said nothing
/// is indistinguishable from a menu row that is broken.
pub fn msg_code_actions_none(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "The server has nothing to offer here",
        Lang::It => "Il server non ha niente da offrire qui",
    }
}

/// One action was picked and the server has not yet said what it would change.
///
/// Named, because the wait is the server computing the whole refactoring: the titles come back at
/// once and the edits are worked out for the one that was chosen, so this is the line that sits
/// there while that happens.
pub fn msg_code_action_asking(lang: Lang, title: &str) -> String {
    match lang {
        Lang::En => format!("Asking the server what \"{title}\" would change…"),
        Lang::It => format!("Chiedo al server cosa cambierebbe \"{title}\"…"),
    }
}

/// The action turned out to change nothing, which is an answer. Said out loud for the reason
/// [`msg_format_already`] is: a row that did nothing in silence is a row somebody presses again.
pub fn msg_code_action_no_changes(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "That action would change nothing",
        Lang::It => "Quell'azione non cambierebbe niente",
    }
}

/// What was done, named the way the server named it, and the promise that it is one step.
pub fn msg_code_action_applied(lang: Lang, title: &str, edits: usize) -> String {
    match lang {
        Lang::En => format!("{title}: {edits} change(s), one Ctrl+Z takes them all back"),
        Lang::It => format!("{title}: {edits} modifiche, un Ctrl+Z le rimette tutte a posto"),
    }
}

// ---- Widening and narrowing the selection -----------------------------------------------------
//
// The shortest family here, and deliberately: this is a chord pressed several times in a row, and
// every one of these sentences is something the reader sees *instead* of the selection moving. So
// each says which of the four possible reasons it was — no server, a server that does not do this,
// a caret with nothing around it, and the two ends of the ladder — and none of them says more.

pub fn msg_selection_asking(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Asking what encloses this…",
        Lang::It => "Chiedo cosa racchiude questo…",
    }
}

/// The server answers this request for nobody. Said before the question goes out, for the reason
/// [`msg_code_actions_unsupported`] is.
pub fn msg_selection_unsupported(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "This server does not widen selections",
        Lang::It => "Questo server non allarga le selezioni",
    }
}

/// The server answered and named nothing at all around the caret — a blank line, the inside of a
/// comment, a file it has not finished reading.
pub fn msg_selection_nothing_here(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "The server knows of nothing around the cursor",
        Lang::It => "Il server non conosce niente attorno al cursore",
    }
}

/// The top of the ladder. Worded as a fact about what the server can see rather than about the
/// file, because that is what it is: the outermost range a server names is the item it parsed, and
/// on most languages that is one function or one declaration and not the whole document.
pub fn msg_selection_widest(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Already the widest thing the server sees around this",
        Lang::It => "È già la cosa più larga che il server vede attorno a questo",
    }
}

/// The bottom of it, which is where the caret was when the walk started.
pub fn msg_selection_narrowest(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Back to where the widening started",
        Lang::It => "Sei tornato da dove è partito l'allargamento",
    }
}

/// Narrowing with nothing to narrow back into. Not a failure and not a server's fault: shrinking is
/// the undo of a widening, and there is no such thing as the inside of a selection nobody grew.
pub fn msg_selection_nothing_to_shrink(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Nothing to narrow back into — widen the selection first",
        Lang::It => "Niente in cui restringere — allarga prima la selezione",
    }
}

pub fn msg_lsp_ready(lang: Lang, program: &str) -> String {
    match lang {
        Lang::En => format!("{program} is answering"),
        Lang::It => format!("{program} risponde"),
    }
}

pub fn msg_lsp_stopped(lang: Lang) -> String {
    match lang {
        Lang::En => "The language server stopped — the underlines are gone, the file is not",
        Lang::It => "Il language server si è fermato — via le sottolineature, non il file",
    }
    .to_string()
}

pub fn msg_replaced_all(lang: Lang, count: usize) -> String {
    match lang {
        Lang::En => format!("Replaced {count} occurrence(s)"),
        Lang::It => format!("Sostituite {count} occorrenze"),
    }
}

pub fn msg_created_entry(lang: Lang, path: &str) -> String {
    match lang {
        Lang::En => format!("Created: {path}"),
        Lang::It => format!("Creato: {path}"),
    }
}

pub fn msg_create_entry_error(lang: Lang, err: &str) -> String {
    match lang {
        Lang::En => format!("Could not create: {err}"),
        Lang::It => format!("Impossibile creare: {err}"),
    }
}

pub fn msg_opened_read_only(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("Opened {name} (read-only: binary or non-UTF-8)"),
        Lang::It => format!("Aperto {name} (sola lettura: binario o non-UTF-8)"),
    }
}

/// Said once, on opening a file over the large-file line: the size that decided it, then the
/// three things that are not there. Said in that order because the size is the only part the
/// user can check, and it is what makes the rest a rule rather than a failure.
///
/// The undo depth is a number and not "shallow": a user about to make a large edit needs to
/// know how far back they can walk, and "shallow" is exactly the word that makes them find out
/// the hard way. Not modal — an editor that stops to be agreed with about a file it *is*
/// opening has interrupted for nothing.
pub fn msg_opened_large(lang: Lang, name: &str, megabytes: u64, undo_depth: usize) -> String {
    match lang {
        Lang::En => format!(
            "Opened {name} — {megabytes} MB: no highlighting, no word completion, \
             undo keeps {undo_depth} steps"
        ),
        Lang::It => format!(
            "Aperto {name} — {megabytes} MB: niente colori, niente completamento dalle parole, \
             undo a {undo_depth} passi"
        ),
    }
}

pub fn msg_opened(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("Opened: {name}"),
        Lang::It => format!("Aperto: {name}"),
    }
}

pub fn msg_open_error(lang: Lang, err: &str) -> String {
    match lang {
        Lang::En => format!("Error opening file: {err}"),
        Lang::It => format!("Errore apertura file: {err}"),
    }
}

pub fn msg_saved(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("Saved: {name}"),
        Lang::It => format!("Salvato: {name}"),
    }
}

pub fn msg_save_error(lang: Lang, err: &str) -> String {
    match lang {
        Lang::En => format!("Error saving: {err}"),
        Lang::It => format!("Errore salvataggio: {err}"),
    }
}

pub fn msg_saved_all(lang: Lang, count: usize) -> String {
    match lang {
        Lang::En if count == 0 => "Nothing to save".to_string(),
        Lang::En => format!("Saved {count} file(s)"),
        Lang::It if count == 0 => "Niente da salvare".to_string(),
        Lang::It => format!("Salvati {count} file"),
    }
}

pub fn msg_save_all_errors(lang: Lang, saved: usize, errors: &str) -> String {
    match lang {
        Lang::En => format!("Saved {saved} file(s), errors: {errors}"),
        Lang::It => format!("Salvati {saved} file, errori: {errors}"),
    }
}

pub fn msg_new_terminal(lang: Lang, count: usize) -> String {
    match lang {
        Lang::En => format!("New terminal window ({count} total)"),
        Lang::It => format!("Nuova finestra terminale ({count} totali)"),
    }
}

pub fn msg_new_terminal_tab(lang: Lang, tabs: usize) -> String {
    match lang {
        Lang::En => format!("New terminal tab ({tabs} in this window)"),
        Lang::It => format!("Nuovo tab terminale ({tabs} in questa finestra)"),
    }
}

pub fn msg_terminal_create_error(lang: Lang, err: &str) -> String {
    match lang {
        Lang::En => format!("Error creating terminal: {err}"),
        Lang::It => format!("Errore creazione terminale: {err}"),
    }
}

pub fn msg_min_one_terminal(lang: Lang) -> String {
    match lang {
        Lang::En => "At least one terminal must remain".to_string(),
        Lang::It => "Deve rimanere almeno un terminale".to_string(),
    }
}

pub fn msg_project_folder(lang: Lang, path: &str) -> String {
    match lang {
        Lang::En => format!("Project folder: {path}"),
        Lang::It => format!("Cartella progetto: {path}"),
    }
}

pub fn msg_externally_modified_kept(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("{name} changed on disk (unsaved local changes kept)"),
        Lang::It => format!("{name} modificato esternamente (modifiche locali non salvate mantenute)"),
    }
}

pub fn msg_externally_reloaded(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("{name} reloaded (changed on disk)"),
        Lang::It => format!("{name} ricaricato (modificato esternamente)"),
    }
}

/// Said when follow mode is switched on somewhere that has no repository to watch. It is not an
/// error and nothing is refused — the setting stays where it was put — but a switch that does
/// nothing has to say so, or it reads as broken the first time a file fails to appear.
pub fn msg_follow_needs_a_repo(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Follow mode watches git status, and this folder is not in a repository",
        Lang::It => "Il modo segui guarda git status, e questa cartella non è in un repository",
    }
}

pub fn msg_follow_mode(lang: Lang, on: bool) -> &'static str {
    match (lang, on) {
        (Lang::En, true) => "Follow mode on: files touched from outside open beside your work",
        (Lang::En, false) => "Follow mode off",
        (Lang::It, true) => "Modo segui acceso: i file toccati da fuori si aprono di fianco",
        (Lang::It, false) => "Modo segui spento",
    }
}

/// Said once, when follow mode has opened as many tabs as it is allowed to. It closes none of
/// them by itself — a tab you were reading is not the editor's to take away — so the way on is
/// to close the ones that are done with.
pub fn msg_follow_full(lang: Lang, limit: usize) -> String {
    match lang {
        Lang::En => format!("Follow mode has opened its {limit} tabs; close some to see more"),
        Lang::It => format!("Il modo segui ha aperto le sue {limit} schede; chiudine per vederne altre"),
    }
}

pub fn msg_copied(lang: Lang, chars: usize) -> String {
    match lang {
        Lang::En => format!("Copied {chars} characters"),
        Lang::It => format!("Copiati {chars} caratteri"),
    }
}

pub fn msg_cut(lang: Lang, chars: usize) -> String {
    match lang {
        Lang::En => format!("Cut {chars} characters"),
        Lang::It => format!("Tagliati {chars} caratteri"),
    }
}

pub fn msg_copied_files(lang: Lang, count: usize, dest: &str) -> String {
    match lang {
        Lang::En => format!("Copied {count} item(s) to {dest}"),
        Lang::It => format!("Copiati {count} elemento/i in {dest}"),
    }
}

pub fn msg_copy_failed(lang: Lang, dest: &str, err: &str) -> String {
    match lang {
        Lang::En => format!("Copy to {dest} failed: {err}"),
        Lang::It => format!("Copia in {dest} fallita: {err}"),
    }
}

/// Asked before anything leaves this machine.
///
/// A paste into a pane that has an ssh session in it used to *upload* whatever paths it named,
/// with no question and no undo — and a paste is not always a drag: the text of a path is a thing
/// people copy, and the path of a private key is a thing people copy while looking for it. So the
/// host is named, because it is the answer to "where would this go", and the count is named,
/// because "3 items" is how you notice you selected a folder you did not mean to. Ends in the
/// same two letters as every other one-letter question here, for the reason `yes_key` gives.
pub fn msg_scp_confirm(lang: Lang, count: usize, target: &str) -> String {
    let yes = yes_key(lang).to_ascii_uppercase();
    match lang {
        Lang::En => format!("Upload {count} item(s) to {target} with scp? They leave this machine.  {yes} / N"),
        Lang::It => format!("Carico {count} elemento/i su {target} con scp? Escono da questa macchina.  {yes} / N"),
    }
}

/// Said when the question is answered with anything but yes, so the paste visibly did nothing
/// rather than invisibly doing something.
pub fn msg_scp_cancelled(lang: Lang) -> String {
    match lang {
        Lang::En => "Upload cancelled; nothing left this machine".to_string(),
        Lang::It => "Upload annullato; non è uscito niente da questa macchina".to_string(),
    }
}

pub fn msg_scp_started(lang: Lang, count: usize, target: &str) -> String {
    match lang {
        Lang::En => format!("Uploading {count} item(s) to {target} via scp…"),
        Lang::It => format!("Upload di {count} elemento/i su {target} via scp…"),
    }
}

pub fn msg_confirm_delete(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("Delete '{name}'? Enter = yes, any other key = no"),
        Lang::It => format!("Eliminare '{name}'? Invio = si, un altro tasto = no"),
    }
}

pub fn msg_deleted(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("Deleted {name}"),
        Lang::It => format!("Eliminato {name}"),
    }
}

pub fn msg_delete_failed(lang: Lang, name: &str, err: &str) -> String {
    match lang {
        Lang::En => format!("Failed to delete {name}: {err}"),
        Lang::It => format!("Eliminazione di {name} fallita: {err}"),
    }
}

pub fn msg_rename_prompt(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("Rename '{name}' to (Enter = confirm, Esc = cancel):"),
        Lang::It => format!("Rinomina '{name}' in (Invio = conferma, Esc = annulla):"),
    }
}

pub fn msg_renamed(lang: Lang, old_name: &str, new_name: &str) -> String {
    match lang {
        Lang::En => format!("Renamed {old_name} to {new_name}"),
        Lang::It => format!("Rinominato {old_name} in {new_name}"),
    }
}

pub fn msg_rename_failed(lang: Lang, name: &str, err: &str) -> String {
    match lang {
        Lang::En => format!("Failed to rename {name}: {err}"),
        Lang::It => format!("Rinomina di {name} fallita: {err}"),
    }
}

pub fn msg_delete_cancelled(lang: Lang) -> String {
    match lang {
        Lang::En => "Delete cancelled".to_string(),
        Lang::It => "Eliminazione annullata".to_string(),
    }
}

/// Shown when something panicked and was contained rather than allowed to close the editor.
/// It names the log deliberately: the status line is one transient line, and a bug worth
/// reporting deserves somewhere to read the details back from.
/// `clee -w` was given a name that is not on disk. Deliberately not a fallback to something
/// else: opening a different workspace than the one asked for is worse than opening none.
/// Someone tried to save over the built-in workspace. It is not a file, and the point of it is
/// to be the one layout that is always there to go back to — so it says no, with the reason.
/// Column selection has no visible switch of its own, so the status line is where it says
/// whether it is on — otherwise the only clue is the shape the next drag happens to make.
pub fn msg_column_selection(lang: Lang, on: bool) -> String {
    match (lang, on) {
        (Lang::En, true) => "Column selection on — Shift+arrows draw the rectangle".to_string(),
        (Lang::En, false) => "Column selection off".to_string(),
        (Lang::It, true) => "Selezione verticale attiva — Shift+frecce disegnano il rettangolo".to_string(),
        (Lang::It, false) => "Selezione verticale disattivata".to_string(),
    }
}

/// Said after "Convert line endings" flips how the buffer will be saved. Named old → new
/// rather than just the new state: the status line is the only place this is confirmed at
/// all, and "now CRLF" leaves open whether that is what changed or what it always was.
pub fn msg_line_endings_converted(lang: Lang, to_crlf: bool) -> String {
    match (lang, to_crlf) {
        (Lang::En, true) => "Line endings: LF → CRLF (written on next save)".to_string(),
        (Lang::En, false) => "Line endings: CRLF → LF (written on next save)".to_string(),
        (Lang::It, true) => "Fine riga: LF → CRLF (scritti al prossimo salvataggio)".to_string(),
        (Lang::It, false) => "Fine riga: CRLF → LF (scritti al prossimo salvataggio)".to_string(),
    }
}

/// Said when the background is handed back to the terminal, and when it is taken again. Worth a
/// line: the change is a whole screen repainting, and it should be obvious that it was a setting
/// rather than a glitch — and obvious that the same button undoes it.
///
/// Renamed with the setting it reports. `on` here means transparency, not fill: a message saying
/// "solid background off" about a flag called transparent would have to be read backwards every
/// time somebody checked which way the two agreed.
pub fn msg_transparent_background(lang: Lang, on: bool) -> String {
    match (lang, on) {
        (Lang::En, true) => "Transparent background on — the terminal shows through again".to_string(),
        (Lang::En, false) => "Transparent background off — the theme paints its own surface".to_string(),
        (Lang::It, true) => "Sfondo trasparente attivo — il terminale torna a trasparire".to_string(),
        (Lang::It, false) => "Sfondo trasparente disattivato — il tema dipinge la propria superficie".to_string(),
    }
}

/// Why the background cannot be handed back. A theme that brings its own surface has no
/// terminal colours left to reveal: turning the fill off would not make it translucent, it would
/// leave light text on whatever the terminal happens to be. Better to say so than to offer a
/// switch that does nothing.
pub fn msg_background_owned_by_theme(lang: Lang, theme: &str) -> String {
    match lang {
        Lang::En => format!("{theme} brings its own background — it cannot be made translucent"),
        Lang::It => format!("{theme} porta il proprio sfondo — non può essere reso trasparente"),
    }
}

/// The plot destination, and — when the machine has no screen to put a window on — why the
/// choice did not take. Saying nothing there would leave the menu showing "off" and the plots
/// still arriving as tabs, which reads as a broken toggle rather than as the only thing that
/// could have happened.
pub fn msg_plots_in_tabs(lang: Lang, in_tabs: bool, can_open_a_window: bool) -> String {
    match (lang, in_tabs, can_open_a_window) {
        (Lang::En, true, _) => "Plots open as tabs — from the next Octave or Python you start".to_string(),
        (Lang::En, false, true) => {
            "Plots open in the interpreter's own windows — from the next one you start".to_string()
        }
        (Lang::En, false, false) => {
            "No display here, so plots stay as tabs — a window would have nowhere to open"
                .to_string()
        }
        (Lang::It, true, _) => "I grafici si aprono nelle tab — dal prossimo Octave o Python che avvii".to_string(),
        (Lang::It, false, true) => {
            "I grafici si aprono nelle finestre dell'interprete — dal prossimo che avvii".to_string()
        }
        (Lang::It, false, false) => {
            "Qui non c'è un display: i grafici restano nelle tab, una finestra non avrebbe dove aprirsi"
                .to_string()
        }
    }
}

/// Handed to the desktop. Named, because a right-click menu is a place where the wrong row is
/// easy to hit and the window may take a moment to appear in front of the terminal.
pub fn msg_opened_outside(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("{name} handed to the desktop"),
        Lang::It => format!("{name} passato al desktop"),
    }
}

/// Why it could not be. Over ssh there is no desktop this side of the connection to hand it to,
/// which is a different thing from an opener that failed and is said differently.
pub fn msg_open_outside_failed(lang: Lang, name: &str, err: &str) -> String {
    match (lang, err) {
        (Lang::En, "over ssh") => {
            format!("{name} stays here: over ssh there is no desktop to open it on")
        }
        (Lang::It, "over ssh") => {
            format!("{name} resta qui: via ssh non c'è un desktop su cui aprirlo")
        }
        (Lang::En, _) => format!("Could not open {name} outside: {err}"),
        (Lang::It, _) => format!("Impossibile aprire {name} fuori: {err}"),
    }
}

/// Handed to the browser. A double-click on a URL should say where it went, the way a jump to
/// a traceback does.
pub fn msg_opened_url(lang: Lang, url: &str) -> String {
    match lang {
        Lang::En => format!("Opening {url} in the browser"),
        Lang::It => format!("Apertura di {url} nel browser"),
    }
}

/// Why a URL could not be handed over. Over ssh there is no browser on this side either.
pub fn msg_open_url_failed(lang: Lang, url: &str, err: &str) -> String {
    match (lang, err) {
        (Lang::En, "over ssh") => {
            format!("{url} stays here: over ssh there is no browser to open it on")
        }
        (Lang::It, "over ssh") => {
            format!("{url} resta qui: via ssh non c'è un browser su cui aprirlo")
        }
        (Lang::En, _) => format!("Could not open {url}: {err}"),
        (Lang::It, _) => format!("Impossibile aprire {url}: {err}"),
    }
}

pub fn msg_workspace_readonly(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => {
            format!("\"{name}\" is built in and cannot be changed — save under another name")
        }
        Lang::It => {
            format!("\"{name}\" è predefinito e non si può modificare — salvalo con un altro nome")
        }
    }
}

pub fn msg_workspace_unknown(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("No workspace called \"{name}\" — `clee -w` lists the saved ones"),
        Lang::It => format!("Nessun workspace \"{name}\" — `clee -w` elenca quelli salvati"),
    }
}

pub fn msg_internal_error(lang: Lang, detail: &str) -> String {
    match lang {
        Lang::En => format!("Internal error (session kept, see panic.log): {detail}"),
        Lang::It => format!("Errore interno (sessione mantenuta, vedi panic.log): {detail}"),
    }
}

pub fn msg_scp_result(lang: Lang, ok: usize, failed: usize, target: &str) -> String {
    match lang {
        Lang::En if failed == 0 => format!("scp: {ok} item(s) uploaded to {target}"),
        Lang::En => format!("scp: {ok} uploaded, {failed} failed (to {target})"),
        Lang::It if failed == 0 => format!("scp: {ok} elemento/i caricati su {target}"),
        Lang::It => format!("scp: {ok} caricati, {failed} falliti (su {target})"),
    }
}

/// Names the protocol the picture is being drawn with. Worth saying once on opening: "kitty"
/// and "half-blocks" look very different on screen, and knowing which one you got is the
/// difference between "my terminal cannot do better" and "something is wrong".
pub fn msg_preview_opened(lang: Lang, protocol: &str) -> String {
    match lang {
        Lang::En => format!("Preview drawn with {protocol}"),
        Lang::It => format!("Anteprima disegnata con {protocol}"),
    }
}

pub fn msg_page_of(lang: Lang, page: usize, total: usize) -> String {
    match lang {
        Lang::En => format!(" page {page} of {total} "),
        Lang::It => format!(" pagina {page} di {total} "),
    }
}

/// When nothing available could say how many pages there are. Paging still works, so the label
/// states what it knows rather than hiding.
pub fn msg_page(lang: Lang, page: usize) -> String {
    match lang {
        Lang::En => format!(" page {page} "),
        Lang::It => format!(" pagina {page} "),
    }
}

pub fn msg_markdown_preview(lang: Lang, as_document: bool) -> String {
    match (lang, as_document) {
        (Lang::En, true) => "Markdown preview: rendered as a document".to_string(),
        (Lang::It, true) => "Anteprima markdown: resa come documento".to_string(),
        (Lang::En, false) => {
            "Markdown preview: styled text (install pandoc, and a terminal with graphics, for a document)"
                .to_string()
        }
        (Lang::It, false) => {
            "Anteprima markdown: testo con stili (per il documento servono pandoc e un terminale con grafica)"
                .to_string()
        }
    }
}

/// A file drop whose files are not on this machine. Over ssh that is the whole explanation and
/// worth giving; locally it is stranger, and the honest answer is just that they are not there.
pub fn msg_drop_not_here(lang: Lang, over_ssh: bool) -> String {
    match (lang, over_ssh) {
        (Lang::En, true) => {
            "Those files are on the machine you connected from; CleeCode is running here, and cannot reach them"
                .to_string()
        }
        (Lang::It, true) => {
            "Quei file sono sulla macchina da cui ti sei collegato; CleeCode gira qui e non può raggiungerli"
                .to_string()
        }
        (Lang::En, false) => "Nothing to copy: those paths do not exist on this machine".to_string(),
        (Lang::It, false) => "Niente da copiare: quei percorsi non esistono su questa macchina".to_string(),
    }
}

pub fn msg_preview_refreshed(lang: Lang) -> String {
    match lang {
        Lang::En => "Preview refreshed".to_string(),
        Lang::It => "Anteprima aggiornata".to_string(),
    }
}

/// While a picture is being decoded on a background thread.
pub fn msg_preview_loading(lang: Lang) -> String {
    match lang {
        Lang::En => "Loading the picture...".to_string(),
        Lang::It => "Carico l'immagine...".to_string(),
    }
}

/// An animation whose frames would not all fit in memory. The tab shows the first frame, which
/// is the honest half of the file it can afford; this says so, with the numbers that decided it.
pub fn msg_animation_too_large(lang: Lang, width: u32, height: u32, frames: usize) -> String {
    match (lang, frames) {
        (Lang::En, 1) => format!(
            "Too large to animate ({width}x{height}, more than one frame): showing the first frame"
        ),
        (Lang::En, _) => format!(
            "Too large to animate ({width}x{height}, more than {frames} frames): showing the first frame"
        ),
        (Lang::It, 1) => format!(
            "Troppo grande da animare ({width}x{height}, più di un fotogramma): mostro il primo"
        ),
        (Lang::It, _) => format!(
            "Troppo grande da animare ({width}x{height}, più di {frames} fotogrammi): mostro il primo"
        ),
    }
}

/// The mark the preview's bar keeps for that file, for as long as the tab is open. The status
/// message above is taken by the next gesture, and without this the tab would go on looking
/// like an ordinary still picture with nothing left on screen to say why.
pub fn label_first_frame(lang: Lang) -> String {
    match lang {
        Lang::En => " first frame ".to_string(),
        Lang::It => " primo fotogramma ".to_string(),
    }
}

pub fn msg_preview_failed(lang: Lang, reason: &str) -> String {
    match lang {
        Lang::En => format!("Cannot show this file: {reason}"),
        Lang::It => format!("Non riesco a mostrare questo file: {reason}"),
    }
}

pub fn msg_run_no_file(lang: Lang) -> String {
    match lang {
        Lang::En => "Save the file first to run it".to_string(),
        Lang::It => "Salva il file per poterlo eseguire".to_string(),
    }
}

pub fn msg_run_no_command(lang: Lang, ext: &str) -> String {
    match lang {
        Lang::En => format!("No run command for .{ext} files — set one from the button left of Run"),
        Lang::It => format!("Nessun comando per i file .{ext} — impostalo dal pulsante a sinistra di Esegui"),
    }
}

/// The toolbar button on a file with no extension, which is the one thing a run command can't
/// be keyed on.
pub fn msg_run_no_ext(lang: Lang) -> String {
    match lang {
        Lang::En => "Run commands are set per file extension, and this file has none".to_string(),
        Lang::It => "I comandi di esecuzione si impostano per estensione, e questo file non ne ha"
            .to_string(),
    }
}

pub fn msg_run_command_row(lang: Lang, ext: &str) -> String {
    match lang {
        Lang::En => format!("Run command for .{ext}..."),
        Lang::It => format!("Comando per i file .{ext}..."),
    }
}

pub fn msg_run_command_unset_row(lang: Lang, ext: &str) -> String {
    match lang {
        Lang::En => format!("Set a run command for .{ext}..."),
        Lang::It => format!("Imposta un comando per i file .{ext}..."),
    }
}

/// Deliberately not naming the extension again: it is already in the row above and in the
/// menu's own title, and what this row adds is the *scope*, which is the whole distinction.
pub fn msg_run_command_project_row(lang: Lang) -> String {
    match lang {
        Lang::En => "...only in this project (.cleecode.toml)".to_string(),
        Lang::It => "...solo in questo progetto (.cleecode.toml)".to_string(),
    }
}

pub fn msg_run_command_project_set(lang: Lang, ext: &str, command: &str) -> String {
    match lang {
        Lang::En => format!(".{ext} runs with, in this project only: {command}"),
        Lang::It => format!("I file .{ext}, solo in questo progetto, si eseguono con: {command}"),
    }
}

pub fn msg_run_command_project_cleared(lang: Lang, ext: &str) -> String {
    match lang {
        Lang::En => format!(".{ext} is back to the run command every project shares"),
        Lang::It => format!("I file .{ext} tornano al comando condiviso da tutti i progetti"),
    }
}

/// What emptying the box does differs by scope, and it is the one thing about this box worth
/// saying: globally it restores the built-in default, in a project it drops the override and
/// hands the extension back to the shared command.
pub fn msg_run_command_prompt(lang: Lang, scope: crate::app::RunScope) -> String {
    match (lang, scope) {
        (Lang::En, crate::app::RunScope::Global) => "Command (empty to go back to the default):",
        (Lang::It, crate::app::RunScope::Global) => "Comando (vuoto per tornare al predefinito):",
        (Lang::En, crate::app::RunScope::Project) => {
            "Command for this project (empty to use the shared one):"
        }
        (Lang::It, crate::app::RunScope::Project) => {
            "Comando per questo progetto (vuoto per usare quello condiviso):"
        }
    }
    .to_string()
}

/// Spelled out in the box itself: the placeholders are the whole reason a command can do more
/// than "interpreter, then file", and nobody would go looking for them in the manual.
pub fn msg_run_command_placeholders(lang: Lang) -> String {
    match lang {
        Lang::En => "{file} full path  ·  {dir} its folder  ·  {name} file name  ·  {stem} name without extension"
            .to_string(),
        Lang::It => "{file} percorso  ·  {dir} sua cartella  ·  {name} nome file  ·  {stem} nome senza estensione"
            .to_string(),
    }
}

pub fn msg_run_command_set(lang: Lang, ext: &str, command: &str) -> String {
    match lang {
        Lang::En => format!(".{ext} runs with: {command}"),
        Lang::It => format!("I file .{ext} si eseguono con: {command}"),
    }
}

pub fn msg_run_command_cleared(lang: Lang, ext: &str) -> String {
    match lang {
        Lang::En => format!(".{ext} files have no run command now"),
        Lang::It => format!("I file .{ext} non hanno più un comando di esecuzione"),
    }
}

pub fn msg_save_as_prompt(lang: Lang) -> String {
    match lang {
        Lang::En => "Save as (name or path, relative to the project root):".to_string(),
        Lang::It => "Salva come (nome o percorso, relativo alla radice del progetto):".to_string(),
    }
}

pub fn msg_save_as_exists(lang: Lang, path: &str) -> String {
    match lang {
        Lang::En => format!("{path} already exists — choose another name"),
        Lang::It => format!("{path} esiste già — scegli un altro nome"),
    }
}

pub fn msg_saved_all_unnamed(lang: Lang, saved: usize, unnamed: usize) -> String {
    match lang {
        Lang::En => format!("Saved {saved} file(s); {unnamed} still need a name (Ctrl+S to name one)"),
        Lang::It => format!("Salvati {saved} file; {unnamed} senza nome (Ctrl+S per dargliene uno)"),
    }
}

pub fn msg_venv_selected(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("Python venv: {name}"),
        Lang::It => format!("Venv Python: {name}"),
    }
}

pub fn msg_venv_cleared(lang: Lang) -> String {
    match lang {
        Lang::En => "Python venv: none (system python)".to_string(),
        Lang::It => "Venv Python: nessuno (python di sistema)".to_string(),
    }
}

pub fn msg_not_a_venv(lang: Lang, path: &str) -> String {
    match lang {
        Lang::En => format!("{path} is not a virtualenv (no bin/activate) — not added"),
        Lang::It => format!("{path} non e un virtualenv (manca bin/activate) — non aggiunto"),
    }
}

pub fn msg_terminal_rename_prompt(lang: Lang) -> String {
    match lang {
        Lang::En => "Name (empty to reset):".to_string(),
        Lang::It => "Nome (vuoto per azzerare):".to_string(),
    }
}

pub fn msg_terminal_startup_prompt(lang: Lang) -> String {
    match lang {
        Lang::En => "Startup command, run when the workspace opens (e.g. claude, octave):".to_string(),
        Lang::It => "Comando di avvio, eseguito all'apertura del workspace (es. claude, octave):".to_string(),
    }
}

pub fn msg_terminal_form_hint(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Tab switches field · Enter confirms · Esc cancels",
        Lang::It => "Tab cambia campo · Invio conferma · Esc annulla",
    }
}

pub fn msg_terminal_renamed(lang: Lang, name: &str, startup: Option<&str>) -> String {
    let label = if name.is_empty() {
        match lang {
            Lang::En => "default name".to_string(),
            Lang::It => "nome predefinito".to_string(),
        }
    } else {
        format!("\"{name}\"")
    };
    match (lang, startup) {
        (Lang::En, Some(cmd)) => format!("Terminal: {label} · startup command: {cmd} (runs when the workspace opens)"),
        (Lang::En, None) => format!("Terminal: {label}"),
        (Lang::It, Some(cmd)) => {
            format!("Terminale: {label} · comando di avvio: {cmd} (eseguito all'apertura del workspace)")
        }
        (Lang::It, None) => format!("Terminale: {label}"),
    }
}

pub fn msg_workspace_save_prompt(lang: Lang) -> String {
    match lang {
        Lang::En => "Workspace name (an existing one is overwritten):".to_string(),
        Lang::It => "Nome del workspace (se esiste viene sovrascritto):".to_string(),
    }
}

pub fn msg_workspace_saved(lang: Lang, name: &str, terminals: usize) -> String {
    match lang {
        Lang::En => format!("Workspace \"{name}\" saved ({terminals} terminal window(s))"),
        Lang::It => format!("Workspace \"{name}\" salvato ({terminals} finestre terminale)"),
    }
}

/// Said when a workspace of the user's own has the same name as one CleeCode ships. Theirs is
/// what opened — a saved workspace cannot be had again and a built-in can — and the sentence
/// exists so that fact is on screen rather than being a preset that mysteriously does nothing.
pub fn msg_workspace_shadows(lang: Lang, built_in: &str) -> String {
    match lang {
        Lang::En => format!("Opened yours — \"{built_in}\" is also built in; rename yours to reach it"),
        Lang::It => format!("Aperto il tuo — \"{built_in}\" è anche un preset; rinomina il tuo per averlo"),
    }
}

pub fn msg_workspace_loaded(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("Workspace \"{name}\" opened"),
        Lang::It => format!("Workspace \"{name}\" aperto"),
    }
}

/// Said when changing project folder steps out of the workspace in use. Both halves matter: the
/// folder is what was asked for, and the workspace leaving is what would otherwise be noticed
/// only later, by finding it pointing somewhere else.
pub fn msg_workspace_left(lang: Lang, name: &str, path: &str) -> String {
    match lang {
        Lang::En => format!("Left workspace \"{name}\" — project folder: {path}"),
        Lang::It => format!("Uscito dal workspace \"{name}\" — cartella progetto: {path}"),
    }
}

pub fn msg_workspace_deleted(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("Workspace \"{name}\" deleted"),
        Lang::It => format!("Workspace \"{name}\" eliminato"),
    }
}

pub fn msg_workspace_error(lang: Lang, err: &str) -> String {
    match lang {
        Lang::En => format!("Could not save the workspace: {err}"),
        Lang::It => format!("Impossibile salvare il workspace: {err}"),
    }
}

pub fn msg_resize_edge(lang: Lang) -> String {
    match lang {
        Lang::En => "That border is the window edge — nothing to resize there".to_string(),
        Lang::It => "Quel bordo è il bordo della finestra — niente da ridimensionare lì".to_string(),
    }
}

pub fn msg_venv_path_prompt(lang: Lang) -> String {
    match lang {
        Lang::En => "Path to the venv (absolute, or ~/... ):".to_string(),
        Lang::It => "Percorso del venv (assoluto, oppure ~/... ):".to_string(),
    }
}

pub fn msg_venv_nickname_prompt(lang: Lang) -> String {
    match lang {
        Lang::En => "Short name to show in the selector (Enter to skip):".to_string(),
        Lang::It => "Nome breve da mostrare nel selettore (Invio per saltare):".to_string(),
    }
}

pub fn msg_copied_chars(lang: Lang, chars: usize) -> String {
    match lang {
        Lang::En => format!("Copied {chars} characters from the terminal"),
        Lang::It => format!("Copiati {chars} caratteri dal terminale"),
    }
}

pub fn msg_run_started(lang: Lang, terminal_index: usize, command: &str) -> String {
    match lang {
        Lang::En => format!("Running in Terminal {}: {command}", terminal_index + 1),
        Lang::It => format!("Eseguo nel Terminale {}: {command}", terminal_index + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one sentence somebody with no debugger installed ever sees, on all three platforms and
    /// in both languages.
    ///
    /// Tested for every operating system rather than for this one, because a message that can
    /// only be read on a machine nobody here has is exactly the message that goes wrong: the
    /// Windows wording would be checked by a Windows user, once, after it had shipped.
    ///
    /// What it must carry is what to look for — the adapters `find_adapter` actually looks for,
    /// taken from the same list — and one sentence about where they come from here. The unknown
    /// platform is the one that has to mention the setting: there is no package to name, so the
    /// escape has to be.
    #[test]
    fn the_missing_adapter_sentence_names_what_to_install_on_every_platform() {
        for lang in [Lang::En, Lang::It] {
            for os in ["macos", "linux", "windows", "freebsd"] {
                let said = msg_debugger_no_adapter(lang, os);
                for wanted in crate::dap::ADAPTERS_WANTED {
                    assert!(said.contains(wanted), "{lang:?}/{os}: {said:?} never names {wanted}");
                }
            }
            assert!(msg_debugger_no_adapter(lang, "macos").contains("xcode-select"), "{lang:?}");
            assert!(msg_debugger_no_adapter(lang, "linux").contains("LLVM"), "{lang:?}");
            assert!(msg_debugger_no_adapter(lang, "windows").contains("MSYS2"), "{lang:?}");
            assert!(
                msg_debugger_no_adapter(lang, "plan9").contains("debug_adapter"),
                "{lang:?}: with no package to name, the setting is the whole answer"
            );
            // And no two of them are the same sentence, which is the failure this would have:
            // one arm written and three copies of it.
            let all = ["macos", "linux", "windows", "plan9"].map(|os| msg_debugger_no_adapter(lang, os));
            for (i, one) in all.iter().enumerate() {
                for other in &all[i + 1..] {
                    assert_ne!(one, other, "{lang:?}: two platforms are told the same thing");
                }
            }
        }
    }

    /// The four refusals a debug verb can give have to be four different sentences.
    ///
    /// They are the whole of what somebody gets back from a menu row that did nothing, and two of
    /// them reading alike is two states nobody can tell apart: "there is no session" and "the
    /// program is running" want opposite next moves.
    #[test]
    fn every_debug_refusal_says_something_of_its_own() {
        for lang in [Lang::En, Lang::It] {
            let said = [
                msg_debugger_no_session(lang),
                msg_debugger_not_stopped(lang),
                msg_debugger_already_running(lang),
                msg_debugger_no_debuggee(lang, "target/debug/clee"),
            ];
            for (i, one) in said.iter().enumerate() {
                assert!(!one.is_empty(), "{lang:?}: a refusal that says nothing");
                for other in &said[i + 1..] {
                    assert_ne!(one, other, "{lang:?}: two refusals read the same");
                }
            }
            assert!(
                said[3].contains("target/debug/clee"),
                "{lang:?}: the guess that failed has to be named, or nobody can correct it"
            );
        }
    }

    /// The question and the key that answers it are two pieces of text a long way apart, and
    /// nothing else would notice them drifting: a box reading "S / N" that only answers to `y`
    /// looks broken while working exactly as written, and the user's way of finding out is to
    /// press the key and watch nothing happen — in front of the one action that cannot be undone.
    #[test]
    fn the_discard_question_names_the_key_that_answers_it() {
        for lang in [Lang::En, Lang::It] {
            let question = msg_git_discard_prompt(lang, "src/main.rs");
            let key = yes_key(lang).to_ascii_uppercase();
            assert!(
                question.contains(&format!("{key} / N")),
                "{lang:?}: {question:?} does not offer {key}"
            );
            assert!(question.contains("src/main.rs"), "{lang:?}: the file has to be named");
        }
    }

    /// The same property for the upload question, which is asked on the status line rather than
    /// in a box: there is even less room there for the reader to guess, and saying yes sends
    /// files to another machine.
    #[test]
    fn the_upload_question_names_the_key_the_host_and_the_count() {
        for lang in [Lang::En, Lang::It] {
            let question = msg_scp_confirm(lang, 3, "build-box");
            let key = yes_key(lang).to_ascii_uppercase();
            assert!(question.contains(&format!("{key} / N")), "{lang:?}: {question:?} does not offer {key}");
            assert!(question.contains("build-box"), "{lang:?}: the host has to be named");
            assert!(question.contains('3'), "{lang:?}: how many files has to be said");
        }
    }

    /// Every tab says which keys it has, and the two that can be acted on have to name the
    /// action rather than only the movement — that is the whole reason the row is drawn.
    #[test]
    fn each_tab_says_what_can_be_done_to_it() {
        for lang in [Lang::En, Lang::It] {
            for tab in crate::app::GitTab::ALL {
                assert!(!msg_git_keys(lang, tab).is_empty(), "{lang:?} {tab:?}");
            }
            let status = msg_git_keys(lang, crate::app::GitTab::Status);
            for key in ["S", "U", "A", "C", "X"] {
                assert!(status.contains(key), "{lang:?}: the status keys leave out {key}");
            }
        }
    }
}
