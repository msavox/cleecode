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
    ItemSelectVenv,
    VenvPickerTitle,
    VenvRegisterItem,
    VenvBrowseItem,
    ItemToggleSplitView,
    ItemToggleHiddenFiles,
    ToolbarRun,
    ToolbarVenvNone,
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

        (Lang::En, ItemSelectVenv) => "Python venv...",
        (Lang::It, ItemSelectVenv) => "Venv Python...",

        (Lang::En, VenvPickerTitle) => "Python venv",
        (Lang::It, VenvPickerTitle) => "Venv Python",

        (Lang::En, VenvRegisterItem) => "Add a venv from elsewhere on disk...",
        (Lang::It, VenvRegisterItem) => "Aggiungi un venv da un altro percorso...",
        (Lang::En, VenvBrowseItem) => "Browse for a venv folder...",
        (Lang::It, VenvBrowseItem) => "Sfoglia una cartella venv...",

        (Lang::En, ItemToggleSplitView) => "Split editor",
        (Lang::It, ItemToggleSplitView) => "Editor affiancati",

        (Lang::En, ItemToggleHiddenFiles) => "Hidden files",
        (Lang::It, ItemToggleHiddenFiles) => "File nascosti",

        (Lang::En, ToolbarRun) => "Run",
        (Lang::It, ToolbarRun) => "Esegui",

        (Lang::En, ToolbarVenvNone) => "no venv",
        (Lang::It, ToolbarVenvNone) => "no venv",

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

        (Lang::En, ManualHint) => "↑↓ scroll · Tab/←→ section · PgUp/PgDn page · Home/End · Esc closes",
        (Lang::It, ManualHint) => "↑↓ scorri · Tab/←→ sezione · PgSu/PgGiù pagina · Home/Fine · Esc chiude",

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

pub fn msg_goto_prompt(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Go to line:",
        Lang::It => "Vai alla riga:",
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

pub fn msg_run_no_file(lang: Lang) -> String {
    match lang {
        Lang::En => "Save the file first to run it".to_string(),
        Lang::It => "Salva il file per poterlo eseguire".to_string(),
    }
}

pub fn msg_run_no_command(lang: Lang, ext: &str) -> String {
    match lang {
        Lang::En => format!("No run command configured for .{ext} files (edit run_commands in settings.toml)"),
        Lang::It => format!("Nessun comando configurato per i file .{ext} (modifica run_commands in settings.toml)"),
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

pub fn msg_workspace_loaded(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("Workspace \"{name}\" opened"),
        Lang::It => format!("Workspace \"{name}\" aperto"),
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
