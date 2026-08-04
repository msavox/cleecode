//! The built-in manual: the text shown by Help ▸ Manual (`Ctrl+Shift+M`), as a list of sections a
//! reader moves between rather than one long scroll. Plain text with ASCII diagrams, so it
//! renders identically wherever CleeCode does — no font, no colour, no markdown.
//!
//! Body lines are kept under ~76 columns: the reading pane is the modal width minus the
//! section list, and wrapping mid-diagram would ruin the pictures.

use crate::i18n::Lang;

pub struct Section {
    pub title: &'static str,
    pub body: &'static [&'static str],
}

/// Where the reader is: which section, and how far down it. Held by `App` while the manual
/// is open.
pub struct ManualState {
    pub section: usize,
    pub scroll: usize,
}

impl ManualState {
    pub fn new() -> Self {
        ManualState { section: 0, scroll: 0 }
    }

    /// Moves to another section, always landing at its top — a scroll offset carried over
    /// from a longer section would open the next one part-way down.
    pub fn select(&mut self, index: usize, count: usize) {
        if count == 0 {
            return;
        }
        self.section = index.min(count - 1);
        self.scroll = 0;
    }

    pub fn cycle(&mut self, delta: isize, count: usize) {
        if count == 0 {
            return;
        }
        let len = count as isize;
        self.section = (((self.section as isize + delta) % len + len) % len) as usize;
        self.scroll = 0;
    }

    /// Scrolls within `len` lines shown `height` at a time, stopping at the last screenful
    /// rather than scrolling the text off the top.
    pub fn scroll_by(&mut self, delta: isize, len: usize, height: usize) {
        let max = len.saturating_sub(height.max(1));
        self.scroll = (self.scroll as isize + delta).clamp(0, max as isize) as usize;
    }
}

pub fn sections(lang: Lang) -> Vec<Section> {
    match lang {
        Lang::En => EN.iter().map(|(title, body)| Section { title, body }).collect(),
        Lang::It => IT.iter().map(|(title, body)| Section { title, body }).collect(),
    }
}

type Page = (&'static str, &'static [&'static str]);

const EN: &[Page] = &[
    ("Overview", &[
        "CleeCode is a terminal IDE: an editor, a file tree, and real terminals",
        "in one window. Everything is reachable with the keyboard alone; the",
        "mouse is an alternative, never the only way.",
        "",
        "  ┌─ menu bar ────────────────────────────────────────────────┐",
        "  │ CleeCode File Edit View Layout Run Terminal Workspace     │",
        "  ├──────────┬────────────────────────────────────────────────┤",
        "  │ Files    │ main.rs x  notes.md x        venv v    > Run   │",
        "  │  src/    ├────────────────────────────────────────────────┤",
        "  │   main.rs│  1  fn main() {                                │",
        "  │   ui.rs  │  2      println!(\"hello\");                     │",
        "  │  README  │  3  }                                          │",
        "  ├──────────┴─────────────────────┬──────────────────────────┤",
        "  │ Terminal 1                     │ claude                   │",
        "  │ $                              │ $ claude                 │",
        "  ├────────────────────────────────┴──────────────────────────┤",
        "  │ status message                                            │",
        "  └───────────────────────────────────────────────────────────┘",
        "",
        "Three frames take focus in turn: file tree, editor, terminals.",
        "  Ctrl+Alt+←↑↓→           go to the frame that lies that way",
        "  Ctrl+Tab                or cycle through them, like Cmd+Tab",
        "",
        "The focused frame is the one with the coloured border. Most keys",
        "are read by it, so the same key can mean different things in the",
        "editor and in a shell — which is why the terminal keeps almost all",
        "of them for the program running inside it.",
        "",
        "Two ways to find anything without remembering a shortcut:",
        "  Ctrl+P         command palette — every action, fuzzy-searched",
        "  Ctrl+Shift+B   the menu bar, then arrows and Enter",
    ]),
    ("Layout", &[
        "The layout is yours to shape, and it is remembered between runs.",
        "",
        "Show and hide frames:",
        "  Ctrl+E   file tree sidebar",
        "  Ctrl+J   terminal panel",
        "  Ctrl+B   menu bar (Ctrl+Shift+B still opens it while hidden)",
        "  Ctrl+L   split the editor into two panes",
        "",
        "Resize with the keyboard — Ctrl+Shift+U enters resize mode:",
        "  arrows         grow the focused frame on that side",
        "  Shift+arrow    shrink it",
        "  Esc / Enter    leave resize mode (sizes are saved)",
        "",
        "  ┌────────┬──────────────────┐   Editor focused, resize, ←",
        "  │ tree   ║ editor           │   moves the sidebar seam:",
        "  │        ║                  │   the editor grows, the tree",
        "  ├────────╨──────────────────┤   gives up columns.",
        "  │ terminals                 │",
        "  └───────────────────────────┘   A border that is the window",
        "                                  edge has nothing to move, and",
        "                                  says so in the status line.",
        "",
        "With the terminal focused and more than one terminal window open,",
        "the same arrows move the seam between the windows themselves —",
        "left/right when they sit side by side, up/down when stacked.",
        "",
        "With the mouse: drag any inner border. Same seams, same limits.",
        "",
        "Presets live in the Layout menu:",
        "  Classic   tree left, terminals as a strip below",
        "  Wide      no tree, terminals as a column on the right",
        "  Triple    tree left, editor centre, terminals right",
        "and 'Terminal on right' flips the terminal panel between the two",
        "orientations without touching anything else.",
    ]),
    ("File tree", &[
        "The sidebar lists the project root. Icons come from a Nerd Font",
        "(clee --install-font installs the bundled one), and the dot on the",
        "right is git status: yellow modified, green added, red deleted,",
        "cyan renamed, grey untracked.",
        "",
        "With the tree focused (Ctrl+Alt+← from the editor):",
        "  ↑ ↓        move",
        "  → ←        expand / collapse a folder",
        "  Enter      open a file, or make a folder the project root",
        "  ..         the first row walks up to the parent folder",
        "  n / N      new file / new folder, in the selected directory",
        "  e          rename the selection",
        "  Delete     delete it (with a confirmation)",
        "  H          show or hide dotfiles",
        "",
        "A single click on a folder expands it, a double click reroots the",
        "project there. Right-click (or Ctrl+Shift+G) opens the same actions",
        "as a context menu.",
        "",
        "Files dragged onto the tree from elsewhere are copied into the",
        "selected directory. Dropped onto a terminal that is inside an ssh",
        "session, they are uploaded with scp instead.",
    ]),
    ("Editor", &[
        "Tabs across the top of the editor, one per open file; a * marks",
        "unsaved changes. Ctrl+O opens the fuzzy quick-open — start the query",
        "with / ~ ./ or ../ and it browses the filesystem instead of the",
        "project.",
        "",
        "  Ctrl+S / ^⇧S        save / save all",
        "  Ctrl+W              close the tab (asks about unsaved changes)",
        "  Ctrl+Shift+← / →   previous / next tab",
        "  Ctrl+Z / Ctrl+Y     undo / redo",
        "  Ctrl+C X V A        copy, cut, paste, select all",
        "  Ctrl+F / Ctrl+G     find and replace / go to line",
        "  Ctrl+K              comment or uncomment the line",
        "  Alt+↑ / Alt+↓       move the line up / down",
        "  Alt+Shift+↓         duplicate the line",
        "  Tab / Shift+Tab     indent / outdent the selection",
        "  Ctrl+Shift+F        fold or unfold the block under the cursor",
        "  Alt+←/→             move by word (Shift extends the selection)",
        "  Ctrl+Backspace      delete the word before the cursor",
        "",
        "Split view (Ctrl+L) gives two panes, each with its own tab strip:",
        "",
        "  ┌───────────────┬───────────────┐  Alt+← / Alt+→ move between",
        "  │ main.rs       │ notes.md      │  the panes. Both run their own",
        "  │               ║               │  file with ▶ Run; the",
        "  │               ║               │  ║ seam is draggable too.",
        "  └───────────────┴───────────────┘",
        "",
        "Files changed on disk are reloaded automatically unless the buffer",
        "has unsaved changes, in which case the local version is kept and the",
        "status line says so.",
    ]),
    ("Terminals", &[
        "The terminals are real ptys running your $SHELL, so ssh, vim, tmux",
        "and claude all behave as they would anywhere else.",
        "",
        "There are two levels, and it is worth keeping them apart:",
        "",
        "  window   a tiled pane. Ctrl+Shift+N opens one.",
        "  tab      a shell inside a window. Ctrl+Shift+T opens one, and",
        "           it. Tabs only show a strip once a window has two.",
        "",
        "  ┌ Terminal 1 ─────────────┬ claude ─┬ octave ──┬───────────────┐",
        "  │ $                       │ the second window has three tabs,  │",
        "  │                         │ each one its own shell; one of them│",
        "  │                         │ is on screen at a time.            │",
        "  └─────────────────────────┴────────────────────────────────────┘",
        "",
        "  Ctrl+Shift+↑ / ↓       previous / next window",
        "  Ctrl+Shift+← / →       previous / next tab in this window",
        "  Ctrl+Shift+E            name this terminal, and give it a",
        "                          startup command",
        "",
        "Ctrl+Shift+E opens a small form, two fields (Tab switches):",
        "",
        "  Name              claude",
        "  Startup command   claude",
        "",
        "The name replaces 'Terminal N' in the title or tab strip. The",
        "startup command is remembered with the workspace and run in this",
        "shell whenever that workspace is opened — see the Workspaces",
        "section. Setting it does not run it on the spot.",
        "",
        "Selecting text: drag with the mouse, or hold Shift and use the",
        "arrows. Either way the selection goes straight to the system",
        "clipboard, and Esc clears it. Paste with the context menu (the",
        "shell needs Ctrl+V for itself).",
        "",
        "A shell that exits closes its tab, and a window with no tabs left",
        "disappears. There is always at least one terminal.",
    ]),
    ("Workspaces", &[
        "A workspace is a saved snapshot of a whole set-up:",
        "",
        "  · the project root and the files open in tabs",
        "  · frame sizes, which frames are shown, split view",
        "  · every terminal window and tab, with its name, its startup",
        "    command and its share of the space",
        "  · the selected Python venv",
        "",
        "  Workspace ▸ Save workspace...    Ctrl+Shift+W, then a name",
        "  Workspace ▸ Open workspace...    pick from the saved ones",
        "  Workspace ▸ Delete workspace...  same list, removes one",
        "",
        "Opening one rebuilds the terminals it describes and runs each",
        "startup command in its own shell. A workspace with a shell named",
        "'claude' whose startup command is 'claude', next to a plain one and",
        "an 'octave' tab, comes back exactly like that every time:",
        "",
        "  ┌ claude ──────────────┬ Terminal 2 ──┬ octave ──────────────┐",
        "  │ $ claude             │ $            │ $ octave             │",
        "  │ ▌                    │              │ octave:1>            │",
        "  └──────────────────────┴──────────────┴──────────────────────┘",
        "",
        "Buffers with unsaved changes are never closed by opening a",
        "workspace — they stay open alongside its files.",
        "",
        "The workspace in use is written back when CleeCode exits, so",
        "renaming a terminal or nudging a seam is still there next time, and",
        "a bare `clee` reopens it. Launching with a path argument starts",
        "from that path instead and leaves the workspace alone.",
        "",
        "Each workspace is a plain TOML file under the config directory",
        "(workspaces/<name>.toml), so they can be edited, copied between",
        "machines, or kept in a dotfiles repository.",
    ]),
    ("Running code", &[
        "▶ Run in the toolbar, or Ctrl+Shift+R, runs the focused editor's file",
        "pane: the command is typed into an idle terminal, so its output,",
        "prompts and errors are just shell output you can scroll and copy.",
        "",
        "Which command is used comes from [run_commands] in settings.toml,",
        "keyed by extension, with {file} standing in for the file's path:",
        "",
        "  [run_commands]",
        "  py = \"python3 {file}\"",
        "  m  = \"octave --persist {file}\"",
        "",
        "Two special cases save a lot of annoyance:",
        "",
        "  · An interpreter installed outside PATH goes under",
        "    [interpreter_paths] (program name -> absolute path). Octave on",
        "    Windows is found automatically in Program Files.",
        "  · If an Octave prompt is already open in a terminal, a .m file is",
        "    handed to that session with run(...) instead of starting a",
        "    second interpreter — the variables and plot windows survive.",
        "",
        "Python virtualenvs have their own selector, in the toolbar and in",
        "Run ▸ Python venv: the venvs found in the project root, plus any you",
        "register from elsewhere on disk (by path, or with the folder",
        "browser). The selected one replaces python/python3 in the run",
        "command. It is remembered per workspace.",
    ]),
    ("Keys", &[
        "Frame navigation is Ctrl+Alt and an arrow, not plain Ctrl: macOS gives",
        "Ctrl with every arrow to Mission Control and to switching Spaces, and",
        "takes them before any terminal sees them.",
        "",
        "There are no function keys and no PageUp/PageDown here: on a laptop",
        "both need Fn. Ctrl+Shift+<letter> is the application's own layer, and",
        "it is safe in a terminal — no terminal can send Ctrl+Shift to the",
        "program running in a pane, so nothing there is listening for it.",
        "",
        "General",
        "  Ctrl+Shift+M            this manual",
        "  Ctrl+Shift+B            menu bar",
        "  Ctrl+Shift+O            settings",
        "  Ctrl+Shift+G            context menu for the focused frame",
        "  Ctrl+P / Ctrl+O         command palette / quick open",
        "  Ctrl+Q                  quit",
        "",
        "Focus and layout",
        "  Ctrl+Alt+←↑↓→           go to the frame in that direction",
        "  Ctrl+Tab                or cycle them, like Cmd+Tab",
        "  Ctrl+Shift+← / →        previous / next tab of this frame",
        "  Ctrl+E / Ctrl+J         sidebar / terminal panel",
        "  Ctrl+B / Ctrl+L         menu bar / split editor",
        "  Alt+← / Alt+→           move between split panes",
        "  Ctrl+Shift+U            resize mode",
        "",
        "Files and editing",
        "  Ctrl+S / Ctrl+Shift+S   save / save all",
        "  Ctrl+W / Ctrl+D         close tab",
        "  Ctrl+Z / Ctrl+Y         undo / redo",
        "  Ctrl+F / Ctrl+G         find and replace / go to line",
        "  Ctrl+K                  toggle comment",
        "  Alt+↑ / Alt+↓           move line       Alt+Shift+↓  duplicate",
        "  Ctrl+Shift+F            fold / unfold",
        "",
        "Terminals",
        "  Ctrl+Shift+T            new tab",
        "  Ctrl+Shift+K            close this shell (and its window if last)",
        "  Ctrl+Shift+N            new window",
        "  Ctrl+Shift+↑ / ↓        previous / next window",
        "  Ctrl+Shift+E            name and startup command",
        "  Shift+arrows            select text (copies as you go)",
        "",
        "Workspaces and running",
        "  Ctrl+Shift+R            run the current file",
        "  Ctrl+Shift+W            save the workspace",
        "",
        "In the file tree: n / N new file / folder, e rename, Delete remove,",
        "H hidden files, Enter open or reroot.",
        "",
        "The only Alt chords left are the arrows, and they work everywhere:",
        "an Option arrow makes no printable character, so every terminal",
        "forwards it as Meta whatever the keyboard layout is.",
    ]),
    ("Files", &[
        "Everything CleeCode keeps lives in one directory:",
        "",
        "  ~/.config/cleecode/            (macOS and Linux)",
        "  %APPDATA%\\cleecode\\             (Windows)",
        "",
        "    settings.toml                preferences, layout, run commands,",
        "                                 registered venvs, last session",
        "    workspaces/<name>.toml       one file per saved workspace",
        "",
        "settings.toml is meant to be edited by hand. The parts that have no",
        "panel in the app are all there: [run_commands], [interpreter_paths],",
        "registered_venvs, auto_pairs.",
        "",
        "The settings panel (Ctrl+Shift+O) covers line numbers, highlighting,",
        "word wrap, tab size, spaces versus tabs, whitespace marks,",
        "auto-indent, mouse capture and the interface language. Changes take",
        "effect at once and are saved on exit.",
        "",
        "Command line:",
        "  clee                  resume the last workspace or project",
        "  clee <directory>      open it as the project root",
        "  clee <file>           open the file in the current directory",
        "  clee --install-font   install the bundled Nerd Font",
        "  clee --help           usage, --version",
    ]),
];

const IT: &[Page] = &[
    ("Panoramica", &[
        "CleeCode è una IDE da terminale: editor, albero dei file e terminali",
        "veri in una sola finestra. Tutto è raggiungibile da tastiera; il",
        "mouse è un'alternativa, mai l'unica strada.",
        "",
        "  ┌─ barra dei menu ──────────────────────────────────────────┐",
        "  │ CleeCode File Modifica Vista Layout Esegui Terminale Aiuto│",
        "  ├──────────┬────────────────────────────────────────────────┤",
        "  │ File     │ main.rs x  note.md x       venv v    > Esegui  │",
        "  │  src/    ├────────────────────────────────────────────────┤",
        "  │   main.rs│  1  fn main() {                                │",
        "  │   ui.rs  │  2      println!(\"ciao\");                      │",
        "  │  README  │  3  }                                          │",
        "  ├──────────┴─────────────────────┬──────────────────────────┤",
        "  │ Terminale 1                    │ claude                   │",
        "  │ $                              │ $ claude                 │",
        "  ├────────────────────────────────┴──────────────────────────┤",
        "  │ messaggio di stato                                        │",
        "  └───────────────────────────────────────────────────────────┘",
        "",
        "Tre frame prendono il focus a turno: albero, editor, terminali.",
        "  Ctrl+Alt+←↑↓→           va al frame in quella direzione",
        "  Ctrl+Alt+←↑↓→           va al frame in quella direzione",
        "  Ctrl+Tab                oppure li scorre a turno",
        "",
        "Il frame con il bordo colorato è quello che ha il focus: quasi tutti",
        "i tasti li legge lui, quindi lo stesso tasto può voler dire cose",
        "diverse nell'editor e in una shell — per questo il terminale lascia",
        "passare quasi tutto al programma che ci gira dentro.",
        "",
        "Due modi per trovare qualsiasi cosa senza ricordare scorciatoie:",
        "  Ctrl+P         palette comandi — ogni azione, ricerca fuzzy",
        "  Ctrl+Shift+B   la barra dei menu, poi frecce e Invio",
    ]),
    ("Layout", &[
        "Il layout si modella come si vuole e viene ricordato fra un avvio",
        "e l'altro.",
        "",
        "Mostrare e nascondere i frame:",
        "  Ctrl+E   sidebar dei file",
        "  Ctrl+J   pannello terminali",
        "  Ctrl+B   barra dei menu (Ctrl+Shift+B la apre comunque)",
        "  Ctrl+L   divide l'editor in due pannelli",
        "",
        "Da tastiera — Ctrl+Shift+U entra in modalità ridimensiona:",
        "  frecce         allargano il frame sotto focus da quel lato",
        "  Shift+freccia  lo restringono",
        "  Esc / Invio    esce (le dimensioni vengono salvate)",
        "",
        "  ┌────────┬──────────────────┐   Editor sotto focus, poi ←",
        "  │ albero ║ editor           │   muove la giunzione con la",
        "  │        ║                  │   sidebar: l'editor cresce,",
        "  ├────────╨──────────────────┤   l'albero cede colonne.",
        "  │ terminali                 │",
        "  └───────────────────────────┘   Un bordo che è il bordo della",
        "                                  finestra non si muove, e la",
        "                                  status line lo dice.",
        "",
        "Con il terminale sotto focus e più finestre terminale aperte, le",
        "stesse frecce muovono la giunzione fra le finestre: sinistra/destra",
        "se sono affiancate, su/giù se impilate.",
        "",
        "Col mouse: trascina un bordo interno. Stesse giunzioni, stessi",
        "limiti.",
        "",
        "I preset stanno nel menu Layout:",
        "  Classico   albero a sinistra, terminali in fascia sotto",
        "  Ampio      niente albero, terminali in colonna a destra",
        "  Triplo     albero a sinistra, editor al centro, terminali a destra",
        "e 'Terminale a destra' ribalta il pannello fra i due orientamenti.",
    ]),
    ("Albero file", &[
        "La sidebar mostra la radice del progetto. Le icone vengono da un",
        "Nerd Font (clee --install-font installa quello incluso) e il pallino",
        "a destra è lo stato git: giallo modificato, verde aggiunto, rosso",
        "eliminato, ciano rinominato, grigio non tracciato.",
        "",
        "Con l'albero sotto focus (Ctrl+Alt+← dall'editor):",
        "  ↑ ↓        muove",
        "  → ←        espande / chiude una cartella",
        "  Invio      apre un file, o rende una cartella la radice",
        "  ..         la prima riga risale alla cartella superiore",
        "  n / N      nuovo file / nuova cartella nella directory scelta",
        "  e          rinomina",
        "  Canc       elimina (con conferma)",
        "  H          mostra o nasconde i file nascosti",
        "",
        "Un clic su una cartella la espande, un doppio clic ci sposta la",
        "radice del progetto. Il tasto destro (o Ctrl+Shift+G) apre le stesse",
        "azioni come menu contestuale.",
        "",
        "I file trascinati sull'albero vengono copiati nella directory",
        "selezionata; se il drop avviene su un terminale dentro una sessione",
        "ssh, vengono invece caricati con scp.",
    ]),
    ("Editor", &[
        "In cima all'editor una tab per file aperto; un * segnala le",
        "modifiche non salvate. Ctrl+O apre la ricerca rapida fuzzy — se la",
        "query inizia con / ~ ./ o ../ diventa un browser del filesystem.",
        "",
        "  Ctrl+S / ^⇧S        salva / salva tutto",
        "  Ctrl+W              chiude la tab (chiede se ci sono modifiche)",
        "  Ctrl+Shift+← / →   tab precedente / successiva",
        "  Ctrl+Z / Ctrl+Y     annulla / ripeti",
        "  Ctrl+C X V A        copia, taglia, incolla, seleziona tutto",
        "  Ctrl+F / Ctrl+G     trova e sostituisci / vai alla riga",
        "  Ctrl+K              commenta o decommenta la riga",
        "  Alt+↑ / Alt+↓       sposta la riga su / giù",
        "  Alt+Shift+↓         duplica la riga",
        "  Tab / Shift+Tab     aumenta / riduce il rientro",
        "  Ctrl+Shift+F        comprime o espande il blocco",
        "  Alt+←/→             si muove per parole (Shift estende)",
        "  Ctrl+Backspace      cancella la parola precedente",
        "",
        "La vista divisa (Ctrl+L) dà due pannelli, ognuno con le sue tab:",
        "",
        "  ┌───────────────┬───────────────┐  Alt+← / Alt+→ passano da un",
        "  │ main.rs       │ note.md       │  pannello all'altro. Entrambi",
        "  │               ║               │  eseguono il proprio file con",
        "  │               ║               │  ▶ Esegui; la giunzione",
        "  └───────────────┴───────────────┘  ║ si trascina anche.",
        "",
        "I file modificati fuori vengono ricaricati da soli, a meno che il",
        "buffer abbia modifiche non salvate: in quel caso resta la versione",
        "locale e la status line lo segnala.",
    ]),
    ("Terminali", &[
        "I terminali sono pty veri che eseguono la tua $SHELL: ssh, vim,",
        "tmux e claude si comportano come ovunque altrove.",
        "",
        "Ci sono due livelli, e conviene tenerli distinti:",
        "",
        "  finestra   un riquadro affiancato nel layout. Ctrl+Shift+N",
        "             ne apre una.",
        "  tab        una shell dentro una finestra. Ctrl+Shift+T ne apre",
        "             una. La striscia delle tab compare solo",
        "             quando una finestra ne ha due.",
        "",
        "  ┌ Terminale 1 ────────────┬ claude ─┬ octave ──┬───────────────┐",
        "  │ $                       │ la seconda finestra ha tre tab,    │",
        "  │                         │ ognuna con la sua shell; una alla  │",
        "  │                         │ volta è sullo schermo.             │",
        "  └─────────────────────────┴────────────────────────────────────┘",
        "",
        "  Ctrl+PgSu / Ctrl+PgGiù   finestra precedente / successiva",
        "  Ctrl+Shift+← / →         tab precedente / successiva",
        "  Ctrl+Shift+E             dà un nome al terminale e un comando",
        "                           di avvio",
        "",
        "Ctrl+Shift+E apre un form con due campi (Tab cambia campo):",
        "",
        "  Nome                claude",
        "  Comando di avvio    claude",
        "",
        "Il nome sostituisce 'Terminale N' nel titolo o nella tab. Il comando",
        "di avvio viene ricordato insieme al workspace ed eseguito in questa",
        "shell ogni volta che quel workspace viene aperto — vedi la sezione",
        "Workspace. Impostarlo non lo esegue subito.",
        "",
        "Per selezionare testo: trascina col mouse, oppure tieni Shift e usa",
        "le frecce. In entrambi i casi la selezione finisce negli appunti di",
        "sistema, ed Esc la annulla. Per incollare usa il menu contestuale",
        "(Ctrl+V serve alla shell).",
        "",
        "Una shell che esce chiude la sua tab, e una finestra rimasta senza",
        "tab sparisce. Un terminale resta sempre aperto.",
    ]),
    ("Workspace", &[
        "Un workspace è la fotografia salvata di un intero assetto:",
        "",
        "  · la radice del progetto e i file aperti nelle tab",
        "  · dimensioni dei frame, quali sono visibili, vista divisa",
        "  · ogni finestra e tab di terminale, con nome, comando di avvio",
        "    e quota di spazio",
        "  · il venv Python selezionato",
        "",
        "  Workspace ▸ Salva workspace...   Ctrl+Shift+W, poi un nome",
        "  Workspace ▸ Apri workspace...     scegli fra quelli salvati",
        "  Workspace ▸ Elimina workspace...  stessa lista, ne toglie uno",
        "",
        "Aprirne uno ricostruisce i terminali che descrive ed esegue ogni",
        "comando di avvio nella sua shell. Un workspace con una shell",
        "chiamata 'claude' e comando 'claude', accanto a una normale e a una",
        "tab 'octave', torna ogni volta esattamente così:",
        "",
        "  ┌ claude ──────────────┬ Terminale 2 ─┬ octave ──────────────┐",
        "  │ $ claude             │ $            │ $ octave             │",
        "  │ ▌                    │              │ octave:1>            │",
        "  └──────────────────────┴──────────────┴──────────────────────┘",
        "",
        "I buffer con modifiche non salvate non vengono mai chiusi",
        "dall'apertura di un workspace: restano aperti accanto ai suoi file.",
        "",
        "Il workspace in uso viene riscritto all'uscita, così un terminale",
        "rinominato o una giunzione spostata si ritrovano al prossimo avvio,",
        "e un `clee` senza argomenti lo riapre. Lanciare clee con un percorso",
        "parte invece da lì e lascia stare il workspace.",
        "",
        "Ogni workspace è un file TOML nella cartella di configurazione",
        "(workspaces/<nome>.toml): si può modificare a mano, copiare su",
        "un'altra macchina o tenere in un repo di dotfile.",
    ]),
    ("Esecuzione", &[
        "▶ Esegui nella toolbar, o Ctrl+Shift+R, esegue il file dell'editor",
        "sotto focus: il comando viene scritto in un terminale libero, quindi",
        "output, prompt ed errori sono normale output di shell, scorribile e",
        "copiabile.",
        "",
        "Il comando arriva da [run_commands] in settings.toml, per estensione,",
        "con {file} al posto del percorso del file:",
        "",
        "  [run_commands]",
        "  py = \"python3 {file}\"",
        "  m  = \"octave --persist {file}\"",
        "",
        "Due casi particolari fanno risparmiare parecchi fastidi:",
        "",
        "  · Un interprete installato fuori dal PATH va in",
        "    [interpreter_paths] (nome programma -> percorso assoluto).",
        "    Octave su Windows viene trovato da solo in Program Files.",
        "  · Se un prompt Octave è già aperto in un terminale, un file .m",
        "    viene passato a quella sessione con run(...) invece di avviare",
        "    un secondo interprete: variabili e finestre dei grafici restano.",
        "",
        "I virtualenv Python hanno un selettore dedicato, nella toolbar e in",
        "Esegui ▸ Venv Python: quelli trovati nella radice del progetto più",
        "quelli registrati da altrove (per percorso o col browser di",
        "cartelle). Quello scelto sostituisce python/python3 nel comando di",
        "esecuzione e viene ricordato nel workspace.",
    ]),
    ("Tasti", &[
        "La navigazione fra frame è Ctrl+Alt più una freccia, non solo Ctrl:",
        "macOS assegna Ctrl con ogni freccia a Mission Control e al cambio di",
        "Spazio, e le intercetta prima che il terminale le veda.",
        "",
        "Niente tasti funzione e niente PagSu/PagGiù: su un portatile vogliono",
        "tutti il tasto Fn. Il layer dell'applicazione è Ctrl+Shift+<lettera>,",
        "ed è sicuro dentro un terminale — nessun terminale sa mandare",
        "Ctrl+Shift al programma in esecuzione, quindi lì non lo ascolta nessuno.",
        "",
        "Generali",
        "  Ctrl+Shift+M            questo manuale",
        "  Ctrl+Shift+B            barra dei menu",
        "  Ctrl+Shift+O            impostazioni",
        "  Ctrl+Shift+G            menu contestuale del frame sotto focus",
        "  Ctrl+P / Ctrl+O         palette comandi / apertura rapida",
        "  Ctrl+Q                  esci",
        "",
        "Focus e layout",
        "  Ctrl+Alt+←↑↓→           va al frame in quella direzione",
        "  Ctrl+Tab                oppure li scorre, come Cmd+Tab",
        "  Ctrl+Shift+← / →        tab precedente / successiva del frame",
        "  Ctrl+E / Ctrl+J         sidebar / pannello terminali",
        "  Ctrl+B / Ctrl+L         barra dei menu / editor affiancati",
        "  Alt+← / Alt+→           fra i due pannelli",
        "  Ctrl+Shift+U            modalità ridimensiona",
        "",
        "File e modifica",
        "  Ctrl+S / Ctrl+Shift+S   salva / salva tutto",
        "  Ctrl+W / Ctrl+D         chiude la tab",
        "  Ctrl+Z / Ctrl+Y         annulla / ripeti",
        "  Ctrl+F / Ctrl+G         trova e sostituisci / vai alla riga",
        "  Ctrl+K                  commenta",
        "  Alt+↑ / Alt+↓           sposta riga    Alt+Shift+↓  duplica",
        "  Ctrl+Shift+F            comprimi / espandi",
        "",
        "Terminali",
        "  Ctrl+Shift+T            nuova tab",
        "  Ctrl+Shift+K            chiude questa shell (e la finestra se sola)",
        "  Ctrl+Shift+N            nuova finestra",
        "  Ctrl+Shift+↑ / ↓        finestra precedente / successiva",
        "  Ctrl+Shift+E            nome e comando di avvio",
        "  Shift+frecce            seleziona testo (copia mentre selezioni)",
        "",
        "Workspace ed esecuzione",
        "  Ctrl+Shift+R            esegue il file corrente",
        "  Ctrl+Shift+W            salva il workspace",
        "",
        "Nell'albero: n / N nuovo file / cartella, e rinomina, Canc elimina,",
        "H file nascosti, Invio apre o cambia radice.",
        "",
        "Gli unici Alt rimasti sono le frecce, e funzionano ovunque: Option",
        "con una freccia non produce nessun carattere, quindi ogni terminale",
        "la inoltra come Meta qualunque sia il layout.",
    ]),
    ("File", &[
        "Tutto quello che CleeCode conserva sta in una sola cartella:",
        "",
        "  ~/.config/cleecode/            (macOS e Linux)",
        "  %APPDATA%\\cleecode\\             (Windows)",
        "",
        "    settings.toml                preferenze, layout, comandi di",
        "                                 esecuzione, venv registrati,",
        "                                 ultima sessione",
        "    workspaces/<nome>.toml       un file per workspace salvato",
        "",
        "settings.toml è pensato per essere modificato a mano. Le parti che",
        "non hanno un pannello nell'app sono tutte lì: [run_commands],",
        "[interpreter_paths], registered_venvs, auto_pairs.",
        "",
        "Le impostazioni (Ctrl+Shift+O) coprono numeri di riga, evidenziazione,",
        "a capo automatico, ampiezza tab, spazi o tab, marcatori degli spazi,",
        "indentazione automatica, mouse e lingua dell'interfaccia. Le",
        "modifiche valgono subito e vengono salvate all'uscita.",
        "",
        "Riga di comando:",
        "  clee                  riprende l'ultimo workspace o progetto",
        "  clee <cartella>       la apre come radice del progetto",
        "  clee <file>           apre il file nella cartella corrente",
        "  clee --install-font   installa il Nerd Font incluso",
        "  clee --help           uso, --version",
    ]),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The reading pane is the modal minus the section list; lines longer than that would be
    /// clipped mid-diagram, which is exactly what the ASCII pictures can't survive.
    #[test]
    fn every_line_fits_the_reading_pane() {
        const MAX: usize = 76;
        for lang in [Lang::En, Lang::It] {
            for section in sections(lang) {
                assert!(section.title.chars().count() <= 14, "section title is too long for the list");
                for line in section.body {
                    assert!(
                        line.chars().count() <= MAX,
                        "manual line is {} columns, over the {MAX} the pane can show: {line}",
                        line.chars().count()
                    );
                }
            }
        }
    }

    /// The diagrams are drawn by hand, and a single stray column makes a box visibly crooked —
    /// which is how a double-width emoji got into one of them. Within a run of box-drawing
    /// lines, the left edges must share a column and the right edges (the lines that close with
    /// a box character, rather than trailing into prose) must share a width.
    #[test]
    fn diagram_boxes_line_up() {
        const EDGES: [char; 8] = ['┌', '┐', '└', '┘', '├', '┤', '│', '╔'];
        let starts_box = |l: &&str| l.trim_start().starts_with(EDGES);
        for lang in [Lang::En, Lang::It] {
            for section in sections(lang) {
                let mut block: Vec<&str> = Vec::new();
                // A trailing blank flushes the last block without duplicating the check.
                for line in section.body.iter().copied().chain(std::iter::once("")) {
                    if starts_box(&line) {
                        block.push(line);
                        continue;
                    }
                    if block.len() > 1 {
                        let indent = |l: &str| l.chars().take_while(|c| *c == ' ').count();
                        let left = indent(block[0]);
                        let closed: Vec<&&str> =
                            block.iter().filter(|l| l.ends_with(EDGES)).collect();
                        let width = closed.first().map(|l| l.chars().count()).unwrap_or(0);
                        for l in &block {
                            assert_eq!(indent(l), left, "diagram line starts in the wrong column: {l}");
                        }
                        for l in closed {
                            assert_eq!(l.chars().count(), width, "diagram box is not rectangular: {l}");
                        }
                    }
                    block.clear();
                }
            }
        }
    }

    /// Both languages must offer the same sections, or switching language mid-read would land
    /// the reader somewhere else entirely.
    #[test]
    fn both_languages_have_the_same_sections() {
        assert_eq!(sections(Lang::En).len(), sections(Lang::It).len());
        assert!(!sections(Lang::En).is_empty());
    }

    #[test]
    fn scrolling_stops_at_the_last_screenful_and_at_the_top() {
        let mut state = ManualState::new();
        // 40 lines in a 10-row pane: the furthest down is line 30.
        state.scroll_by(100, 40, 10);
        assert_eq!(state.scroll, 30);
        state.scroll_by(-100, 40, 10);
        assert_eq!(state.scroll, 0);
        // Shorter than the pane: nothing to scroll.
        state.scroll_by(5, 4, 10);
        assert_eq!(state.scroll, 0);
    }

    #[test]
    fn changing_section_returns_to_its_top() {
        let mut state = ManualState::new();
        state.scroll_by(5, 40, 10);
        state.cycle(1, 3);
        assert_eq!((state.section, state.scroll), (1, 0));
        // Wraps in both directions.
        state.cycle(-1, 3);
        assert_eq!(state.section, 0);
        state.cycle(-1, 3);
        assert_eq!(state.section, 2);
        // Out-of-range clicks land on the last section rather than panicking.
        state.select(99, 3);
        assert_eq!(state.section, 2);
    }
}
