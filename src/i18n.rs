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
    ItemToggleMenuBar,
    ItemOpenMenuBar,
    ItemColumnSelection,
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
    MsgMdOnlyMarkdown,
    MsgMdCantHere,
    MdLinkPlaceholder,
    WorkspaceBadge,
    ItemOpenSettings,
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
    ItemToggleBreakpoint,
    ItemShowWorkspacePanel,
    ItemInspectVariable,
    ItemRunTarget,
    RunMenuTitle,
    VenvRegisterItem,
    VenvBrowseItem,
    ItemToggleSplitView,
    ItemToggleHiddenFiles,
    ItemOpaqueBackground,
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
    SettingPlotsInTabs,
    SettingPlotsNoDisplay,
    SettingPlotsTabs,
    SettingPlotsWindows,
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
    ItemGitPanel,
    ItemGoToDefinition,
    ItemJumpBack,
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
    PickerVariables,
    PickerVenvBrowse,
    PickerWorkspaceOpen,
    PickerWorkspaceDelete,
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

        (Lang::En, ItemToggleMenuBar) => "Menu bar",
        (Lang::It, ItemToggleMenuBar) => "Barra dei menu",
        (Lang::En, ItemOpenMenuBar) => "Open the menu bar",
        (Lang::It, ItemOpenMenuBar) => "Apri la barra dei menu",
        (Lang::En, ItemColumnSelection) => "Column selection",
        (Lang::It, ItemColumnSelection) => "Selezione verticale",

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
        (Lang::En, ItemShowWorkspacePanel) => "Show session variables",
        (Lang::It, ItemShowWorkspacePanel) => "Mostra le variabili della sessione",
        (Lang::En, ItemToggleBreakpoint) => "Breakpoint on this line",
        (Lang::It, ItemToggleBreakpoint) => "Breakpoint su questa riga",
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

        (Lang::En, ItemOpaqueBackground) => "Solid background",
        (Lang::It, ItemOpaqueBackground) => "Sfondo pieno",

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
        (Lang::En, ItemGitPanel) => "Git panel",
        (Lang::It, ItemGitPanel) => "Pannello Git",
        (Lang::En, ItemGoToDefinition) => "Go to definition",
        (Lang::It, ItemGoToDefinition) => "Vai alla definizione",
        (Lang::En, ItemJumpBack) => "Back where you were",
        (Lang::It, ItemJumpBack) => "Torna dov'eri",

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

        (Lang::En, PickerVariables) => "Variables",
        (Lang::It, PickerVariables) => "Variabili",

        (Lang::En, PickerVenvBrowse) => "Browse for a venv (type / or ~ to go elsewhere)",
        (Lang::It, PickerVenvBrowse) => "Cerca un venv (digita / o ~ per andare altrove)",

        (Lang::En, PickerWorkspaceOpen) => "Open workspace (Enter opens)",
        (Lang::It, PickerWorkspaceOpen) => "Apri workspace (Invio apre)",

        (Lang::En, PickerWorkspaceDelete) => "Delete workspace (Enter deletes)",
        (Lang::It, PickerWorkspaceDelete) => "Elimina workspace (Invio elimina)",

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

pub fn msg_workspace_panel(lang: Lang) -> String {
    match lang {
        Lang::En => "Watching for a session — start octave or python in a terminal".to_string(),
        Lang::It => "In ascolto di una sessione — avvia octave o python in un terminale".to_string(),
    }
}

pub fn msg_break_no_language(lang: Lang, ext: &str) -> String {
    match lang {
        Lang::En => format!("No debugger for .{ext} — this works in Octave and Python"),
        Lang::It => format!("Nessun debugger per .{ext} — funziona con Octave e Python"),
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

pub fn msg_lsp_nowhere_back(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "You have not jumped anywhere to come back from",
        Lang::It => "Non sei saltato da nessuna parte da cui tornare",
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

/// Said when the background is taken over, and when it is handed back. Worth a line: the change
/// is a whole screen repainting, and it should be obvious that it was a setting rather than a
/// glitch — and obvious that the same button undoes it.
pub fn msg_opaque_background(lang: Lang, on: bool) -> String {
    match (lang, on) {
        (Lang::En, true) => "Solid background on — the terminal no longer shows through".to_string(),
        (Lang::En, false) => "Solid background off — the terminal's own background is back".to_string(),
        (Lang::It, true) => "Sfondo pieno attivo — il terminale non traspare più".to_string(),
        (Lang::It, false) => "Sfondo pieno disattivato — torna lo sfondo del terminale".to_string(),
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
