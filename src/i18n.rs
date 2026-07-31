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
    ItemSaveAll,
    ItemQuit,
    ItemToggleSidebar,
    ItemToggleTerminal,
    ItemOpenSettings,
    ItemNewTerminal,
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
    ItemToggleSplitView,
    ItemToggleHiddenFiles,
    ToolbarRun,
    ToolbarVenvNone,
    PanelFile,
    SettingsTitle,
    AboutTitle,
    AboutTagline,
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

        (Lang::En, ItemSaveAll) => "Save All",
        (Lang::It, ItemSaveAll) => "Salva tutto",

        (Lang::En, ItemQuit) => "Quit",
        (Lang::It, ItemQuit) => "Esci",

        (Lang::En, ItemToggleSidebar) => "File sidebar",
        (Lang::It, ItemToggleSidebar) => "Sidebar file",

        (Lang::En, ItemToggleTerminal) => "Terminal panel",
        (Lang::It, ItemToggleTerminal) => "Pannello terminale",

        (Lang::En, ItemOpenSettings) => "Settings...",
        (Lang::It, ItemOpenSettings) => "Impostazioni...",

        (Lang::En, ItemNewTerminal) => "New terminal",
        (Lang::It, ItemNewTerminal) => "Nuovo terminale",

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

        (Lang::En, ResizeModeHint) => "Resize mode: ←/→ sidebar width, ↑/↓ terminal size, Esc/Enter to exit",
        (Lang::It, ResizeModeHint) => "Modalita ridimensiona: ←/→ larghezza sidebar, ↑/↓ dimensione terminale, Esc/Invio per uscire",

        (Lang::En, MenuRun) => "Run",
        (Lang::It, MenuRun) => "Esegui",

        (Lang::En, ItemRunFile) => "Run current file",
        (Lang::It, ItemRunFile) => "Esegui file corrente",

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
            "F9 menu · F1/F2/F3 focus · F4 settings · F5/F6 terminals · Ctrl+S save · Ctrl+Q quit"
        }
        (Lang::It, StatusHelp) => {
            "F9 menu · F1/F2/F3 focus · F4 impostazioni · F5/F6 terminali · Ctrl+S salva · Ctrl+Q esci"
        }
    }
}

pub fn terminal_title(lang: Lang, index: usize) -> String {
    match lang {
        Lang::En => format!(" Terminal {} ", index + 1),
        Lang::It => format!(" Terminale {} ", index + 1),
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
        Lang::En => format!("New terminal ({count} total)"),
        Lang::It => format!("Nuovo terminale ({count} totali)"),
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

pub fn msg_delete_cancelled(lang: Lang) -> String {
    match lang {
        Lang::En => "Delete cancelled".to_string(),
        Lang::It => "Eliminazione annullata".to_string(),
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

pub fn msg_run_started(lang: Lang, terminal_index: usize, command: &str) -> String {
    match lang {
        Lang::En => format!("Running in Terminal {}: {command}", terminal_index + 1),
        Lang::It => format!("Eseguo nel Terminale {}: {command}", terminal_index + 1),
    }
}
