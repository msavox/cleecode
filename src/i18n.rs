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
    SettingDiagnostics,
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
    ItemNewFile,
    ItemNewFolder,
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
        (Lang::En, SettingDiagnostics) => "Language server diagnostics",
        (Lang::It, SettingDiagnostics) => "Diagnostici del language server",

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
        Lang::En => "Git — Esc closes · Tab/←→ switch · ↑↓ scroll · R refresh",
        Lang::It => "Git — Esc chiude · Tab/←→ cambia · ↑↓ scorre · R aggiorna",
    }
}

pub fn msg_git_tab(lang: Lang, tab: crate::app::GitTab) -> &'static str {
    use crate::app::GitTab::*;
    match (lang, tab) {
        (Lang::En, Diff) => "Changes",
        (Lang::It, Diff) => "Modifiche",
        (Lang::En, Log) => "History",
        (Lang::It, Log) => "Cronologia",
        (Lang::En, Branches) => "Branches",
        (Lang::It, Branches) => "Branch",
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
