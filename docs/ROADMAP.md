# CleeCode — roadmap

> **Come si legge questo file.** Le prime due parti sono un piano di implementazione per LSP e
> Git scritto da opencode, riportato **invariato**; la terza è la valutazione che lo corregge, e
> la quarta la decisione presa, con lo stato di avanzamento. Se leggi solo una sezione, leggi
> **DECISIONE**: è quella che vale.
>
> Ricostruito il 2026-08-18 dal transcript della sessione del 17/08 — l'originale, mai
> committato, era andato perso dal disco. Si chiamava `PIANO_LSP_GIT.md` e stava nella root.

---

# PIANO: Implementazione LSP e Git Enhancement in CleeCode
*(di opencode, riportato invariato — le correzioni sono più sotto)*

## Contesto

CleeCode è un terminal IDE scritto in Rust (edition 2024) con ratatui. Attualmente ha:
- Editor con syntax highlighting (syntect)
- File tree con indicatori git (solo status colorato)
- Terminali PTY reali
- Workspace persistente

**Obiettivo:** Aggiungere LSP (Language Server Protocol) e Git integration completa.

---

## FASE 1: Git Enhancement

### 1.1 Aggiungere dipendenza `git2`

Aggiungere a `Cargo.toml`:
```toml
git2 = "0.19"
```

`git2` è un binding Rust per libgit2 - più affidabile di chiamare `git` via `std::process::Command`.

### 1.2 Nuovo modulo `src/git.rs` (sostituisce/espande `git_status.rs`)

```rust
// Funzioni da implementare:
pub fn diff_file(repo: &Repository, path: &Path) -> Result<String>  // diff di un file
pub fn commit(repo: &Repository, message: &str, paths: &[PathBuf]) -> Result<Commit>  // commit
pub fn log(repo: &Repository, limit: usize) -> Result<Vec<CommitInfo>>  // history
pub fn branches(repo: &Repository) -> Result<Vec<Branch>>  // lista branch
pub fn checkout(repo: &Repository, branch: &str) -> Result<()>  // switch branch
pub fn stage(repo: &Repository, paths: &[PathBuf]) -> Result<()>  // git add
pub fn unstage(repo: &Repository, paths: &[PathBuf]) -> Result<()>  // git reset
```

### 1.3 Aggiungere campi a `App` in `app.rs`

```rust
pub struct App {
    // ... esistente ...
    pub git_diff: Option<String>,        // diff del file corrente
    pub git_log: Vec<CommitInfo>,        // ultimi N commit
    pub git_branches: Vec<BranchInfo>,   // branch disponibili
    pub git_panel_open: bool,            // visibilità pannello git
    pub git_focus: GitPanelTab,          // quale tab è attivo
}

pub enum GitPanelTab {
    Diff,
    Log,
    Branches,
}
```

### 1.4 Nuovo pannello UI in `ui.rs`

Aggiungere `draw_git_panel()` che mostra:
- **Tab Diff**: diff del file attivo con colori (verde=aggiunto, rosso=rimosso)
- **Tab Log**: lista commit con hash, message, autore
- **Tab Branches**: branch con indicatori (corrente, ahead/behind)

Layout: pannello dedicato nella sidebar o sotto il terminale.

### 1.5 Estendere `MenuAction` in `menu.rs`

```rust
pub enum MenuAction {
    // ... esistente ...
    ToggleGitPanel,
    GitCommit,
    GitStage,
    GitUnstage,
    GitCheckout,
}
```

### 1.6 Aggiungere tasti rapidi

| Tasto | Azione |
|-------|--------|
| `Ctrl+Shift+G` | Toggle Git Panel |
| `Ctrl+Shift+C` | Commit (apre box messaggio) |
| `Ctrl+Shift+A` | Stage file corrente |
| `Ctrl+Shift+U` | Unstage file corrente |

---

## FASE 2: LSP Integration

### 2.1 Aggiungere dipendenze

```toml
tower-lsp = "0.20"
lsp-types = "0.95"
tokio = { version = "1", features = ["full"] }  // async runtime per LSP
```

### 2.2 Nuovo modulo `src/lsp.rs`

```rust
pub struct LspClient {
    pub language: Language,
    pub server: Option<Child>,
    pub rx: Receiver<LspMessage>,
    pub tx: Sender<LspMessage>,
}

pub enum Language {
    Rust,      // rust-analyzer
    Python,    // pyright o pylsp
    TypeScript,// typescript-language-server
    JavaScript,// typescript-language-server
    Go,        // gopls
    // ... espandere gradualmente
}

pub struct LspState {
    pub clients: HashMap<Language, LspClient>,
    pub completions: Vec<CompletionItem>,
    pub diagnostics: Vec<Diagnostic>,
    pub hover_info: Option<HoverInfo>,
}

// Funzioni principali:
pub fn start_server(lang: Language) -> Result<LspClient>
pub fn send_did_open(client: &LspClient, path: &Path, content: &str)
pub fn send_did_change(client: &LspClient, path: &Path, changes: TextEdit)
pub fn request_completion(client: &LspClient, path: &Path, position: Position)
pub fn request_hover(client: &LspClient, path: &Path, position: Position)
pub fn request_definition(client: &LspClient, path: &Path, position: Position)
```

### 2.3 Aggiungere campi a `Editor`

```rust
pub struct Editor {
    // ... esistente ...
    pub lsp_diagnostics: Vec<Diagnostic>,  // errori/warning LSP
    pub lsp_completions: Vec<CompletionItem>,  // completamenti disponibili
    pub show_completions: bool,  // se il menu completamenti è visibile
    pub completion_index: usize,  // indice selezionato
}
```

### 2.4 Integrazione nel ciclo di vita dell'editor

```rust
// In app.rs, quando si apre un file:
if let Some(lang) = detect_language(path) {
    lsp_state.ensure_server(lang)?;
    lsp_client.send_did_open(path, content);
}

// Dopo ogni modifica:
if let Some(client) = lsp_state.get_client(lang) {
    client.send_did_change(path, edits);
}

// Al salvataggio:
if let Some(client) = lsp_state.get_client(lang) {
    client.send_did_save(path);
}
```

### 2.5 UI per LSP

**Autocompletamento** (`draw_completion_menu` in `ui.rs`):
- Mostra sotto il cursore quando si digita
- Navigabile con frecce/focus
- Selezione con `Tab` o `Enter`

**Diagnostics** (barra laterale o inline):
- Errori/warning evidenziati nel margin
- Hover per messaggio completo
- Click per navigare alla posizione

**Hover** (tooltip):
- Mostra tipo/firma quando il mouse è su un simbolo
- Richiede `Ctrl+K Ctrl+I` o simile

### 2.6 Tasti rapidi LSP

| Tasto | Azione |
|-------|--------|
| `Ctrl+Space` | Trigger completion |
| `Ctrl+K Ctrl+I` | Hover info |
| `F12` | Go to definition |
| `Shift+F12` | Find references |
| `Ctrl+.` | Quick fix (code actions) |

---

## ORDINE DI IMPLEMENTAZIONE

### Sprint 1: Git Foundation (2-3 giorni)
1. Aggiungere `git2` a Cargo.toml
2. Creare `src/git.rs` con funzioni base (diff, log, branches)
3. Testare le funzioni in isolamento

### Sprint 2: Git UI (2-3 giorni)
1. Aggiungere campi a `App`
2. Creare `draw_git_panel()` in `ui.rs`
3. Aggiungere `MenuAction` e wiring
4. Collegare al file tree (doppio click per diff)

### Sprint 3: LSP Foundation (3-4 giorni)
1. Aggiungere dipendenze LSP
2. Creare `src/lsp.rs` con client base
3. Implementare `start_server` per rust-analyzer (il più maturo)
4. Testare connessione a rust-analyzer

### Sprint 4: LSP Features (4-5 giorni)
1. Implementare `textDocument/didOpen` e `textDocument/didChange`
2. Implementare `textDocument/completion`
3. Implementare `textDocument/hover`
4. Implementare `textDocument/definition`
5. Aggiungere UI per completamenti e diagnostics

### Sprint 5: Polish (2 giorni)
1. Aggiungere supporto Python (pyright)
2. Aggiungere supporto TypeScript
3. Testing e bug fixing
4. Aggiornare README

---

## NOTE TECNICHE IMPORTANTI

1. **LSP è async**: tower-lsp richiede un async runtime. CleeCode è attualmente sync. Servirà aggiungere `tokio` e gestire il runtime in un thread separato.

2. **Thread safety**: I client LSP girano in thread separati. Usare `Arc<Mutex<LspState>>` per condividere lo stato.

3. **Performance**: Non bloccare il main thread per risposte LSP. Usare canali (mpsc) per comunicare.

4. **Graceful degradation**: Se LSP non è disponibile (server non installato), il programma deve funzionare lo stesso - solo senza intellisense.

5. **Git via git2**: Più affidabile di chiamare `git` via shell. Gestisce errori meglio e non richiede `git` installato.

6. **Memory**: Ogni client LSP consuma ~50-100MB. Limitare a 2-3 server contemporanei.

---

## DIPENDENZE FINALI

```toml
[dependencies]
# ... esistente ...
git2 = "0.19"
tower-lsp = "0.20"
lsp-types = "0.95"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync"] }
```

---
---

# VALUTAZIONE (Claude, 2026-08-17) — cosa tenere e cosa cambiare

Quanto segue è la revisione richiesta del piano qui sopra. Non è di opencode.

## Cosa è messo bene

L'ordine **Git prima di LSP**, la **degradazione se il server manca** (senza language server
il programma deve funzionare lo stesso, solo senza intellisense) e l'uso di **canali invece
del blocco del main thread**. Su questi tre punti il piano ha ragione e restano.

## Tre scelte tecniche da cambiare

### `tower-lsp` è la crate sbagliata

`tower-lsp` serve a **scrivere** un language server, non a parlarci. A CleeCode serve un
*client*: con quella dipendenza si finisce a costruire l'estremità opposta del protocollo.

Serve `lsp-types` per i tipi, e JSON-RPC su stdio scritto a mano — meno lavoro di quanto
sembri: intestazioni `Content-Length`, un thread lettore, un canale.

### `tokio` introduce un secondo modello di concorrenza

Il piano dice "LSP è async, serve tokio". Ma CleeCode **ha già il suo schema** e lo usa tre
volte: thread che lavora, `mpsc` che risponde, `poll_*` nel ciclo a 30 fps. Git status,
decodifica anteprime e scp funzionano tutti così. Un server LSP su stdio è JSON-RPC a righe:
quel modello gli calza. Aggiungere tokio significa due modelli di concorrenza nella stessa
app, e ogni contributore deve sapere quale vale dove.

### `git2` costa più di quanto rende

Il piano dice "più affidabile di chiamare `git`". Non in questo caso:

- **libgit2 è una dipendenza C**, e qui si costruisce per macOS, Linux, Windows MSVC e da
  sorgente via brew. È lo stesso tipo di dipendenza che ha già morso con chafa/pkg-config.
- **Non fa firma GPG, hook, né credential helper.** Un commit fatto con git2 salta l'hook
  pre-commit e non firma: cambio di comportamento silenzioso.
- `git` è già chiamato a shell per lo status, e l'identità del prodotto è "terminali veri".

Per leggere (diff, log, branch) *e* per scrivere, `Command::new("git")` costa meno e si
comporta come il git dell'utente.

## Le scorciatoie violano le regole del progetto

Il piano propone `F12`, `Shift+F12`, `Ctrl+Space`, `Ctrl+K Ctrl+I`, `Ctrl+.`. Il manuale dice,
per scelta motivata: **niente tasti funzione** (su laptop vogliono Fn) e niente simboli che su
layout italiano richiedono già Shift. E collidono con l'esistente: `Ctrl+Shift+G` è il menu
contestuale, `Ctrl+Shift+U` è la modalità ridimensiona. Delle lettere restano libere
`a c d h i j l p q v x y`.

## Due omissioni che diventerebbero bug

**Nessun `didChange` ritardato.** Il piano lo manda "dopo ogni modifica": un messaggio per
tasto premuto, con il testo intero se non si fa sync incrementale. Su un file grande è la
stessa lezione delle anteprime markdown, dove è servito aspettare la pausa nella digitazione.
L'aggancio esiste già: il `revision` di `Editor`.

**Ignora una decisione già presa.** Sul completamento la scelta era fatta: popup tradizionale
sulle parole del buffer, non ghost text, con regole di ranking e non-modalità. Il piano
riparte da zero con il completamento LSP.

## Riordino proposto

1. **Prima Git, tutto a shell, in due passi.** Sola lettura (diff, log, branch), poi le azioni
   (stage, commit). Zero dipendenze nuove, nessun rischio per le build, hook e firma
   preservati.
2. **Prima di LSP, due giorni di debito**: spezzare `handle_key` (294 righe) e `handle_mouse`
   (277) per modalità, e test sulle anteprime (`preview.rs`: 957 righe, 3 test). LSP aggiunge
   una modalità — il popup — proprio a quelle due funzioni.
3. **Poi LSP come release a sé, cominciando dai soli diagnostici.** Un server
   (rust-analyzer), una funzionalità. I diagnostici sono non-modali e non possono corrompere
   un buffer: se il server muore, si perdono delle sottolineature. Il completamento tocca il
   testo mentre scrivi ed è dove un difetto costa caro — va dopo, seguendo la decisione già
   presa sul popup a parole di buffer.

Punto di partenza consigliato: **Git a sola lettura**. È piccolo, si vede subito, e non tocca
il ciclo di input.

---

# DECISIONE (2026-08-18)

Discusso e deciso con l'utente. Il piano di opencode qui sopra **non si esegue come scritto**:
le tre correzioni tecniche (niente `tower-lsp`, niente `tokio`, niente `git2`) valgono, e
l'ordine diventa questo.

## L'intuizione che riordina il piano

Il completamento **non è una funzionalità LSP: è un pezzo di UI**. Il popup — lista sotto il
cursore, frecce, Tab/Invio, non-modale — è lo stesso lavoro sia che i candidati vengano dalle
parole del buffer sia che arrivino da rust-analyzer. Quindi il popup si costruisce **una volta
sola**, alimentato dalle parole del buffer (la decisione già presa, che funziona in qualsiasi
linguaggio e anche in un file di config); quando arriva l'LSP non è una funzionalità nuova da
disegnare, è **una seconda sorgente** che si innesta in un popup già collaudato.

## Le release

**0.6 — Cercare e confrontare** ← *fatta, non ancora rilasciata*
1. ✅ `find.rs`: regex + maiuscole/minuscole. Motore `fancy-regex` (già nel binario via syntect:
   +1 riga di lock, zero crate nuovi). `Ctrl+U` maiuscole, `Ctrl+N` regex — **non** D e T, che
   sono già presi da "chiudi tab" e dal terminale. Gruppi `$1` nella sostituzione.
   Default cambiato: la ricerca ora ignora le maiuscole.
2. ✅ Ricerca nel progetto (`Ctrl+Shift+H`), `src/search.rs`. **Camminata nostra, non `rg`**:
   scelta rivista in corsa perché due dialetti di "pattern" — uno per il file, uno per il
   progetto — sono esattamente il difetto contestato a `tokio` nel piano di opencode.
   `find::compile` è ora l'unico posto dove una query diventa un pattern. Thread + `mpsc` +
   poll; risultati in un picker normale (si filtrano scrivendo).
3. ✅ Pannello Git in sola lettura (`Ctrl+Shift+D`), `src/git.rs`. Diff vs `HEAD` (stage e non
   stage insieme), 50 commit, branch con `[ahead/behind]`. Modale, non un quarto frame.

Zero dipendenze nuove a parte fancy-regex, che era già compilata. Nessuna nuova modalità nel
ciclo di input.

**0.7 — Completamento** ← *fatta, non ancora rilasciata*
1. ~~Spezzare `handle_key` (294 righe) e `handle_mouse` (277) per modalità.~~ **Non serviva, e la
   premessa era sbagliata.** Riguardato prima di toccarlo: `handle_key` è già un dispatcher a tre
   stadi — la fila di modali che delegano ciascuna al proprio `handle_*_key`, i chord globali, e
   il dispatch sul fuoco — ed è cresciuto di 16 righe in tutta la 0.6, non del doppio. Il popup
   non si aggancia lì comunque: essendo non-modale non entra nella fila dei modali, entra in
   `handle_editor_key` (197 righe), che una modalità in più non l'ha ingrossata. Il refactor non
   si ripagava: si sarebbe pagato per virtù, che è esattamente ciò che la valutazione qui sopra
   rimprovera al piano di opencode.
2. ✅ Popup di completamento sulle parole del buffer, `src/complete.rs`. Le decisioni del design
   sono tutte rispettate: ranking a quattro livelli (prefisso esatto → prefisso ignorando le
   maiuscole → fuzzy → parole chiave sempre ultime), `fuzzy_score` confinato al terzo livello,
   indice costruito una volta all'apertura e poi solo filtrato, accettazione in un solo passo di
   undo, Tab che accetta col popup aperto e indenta altrimenti. Il seam `Source` c'è dall'inizio:
   nella 0.8 rust-analyzer diventa una terza variante, non un secondo popup.

   **La non-modalità è una funzione pura**, `complete::key_action`, con un test che elenca i
   cinque tasti presi e verifica che tutto il resto cada nell'editor — inclusi ↑ e Invio *con* un
   modificatore, che restano dell'editor. Un `App` non è costruibile in un test (due PTY veri più
   le impostazioni dell'utente da disco), quindi le regole che contano stanno in funzioni pure:
   `key_action`, `opens_on`, `rank`, `prefix_at`, `completion_rect`. Il disegno è verificato per
   davvero, renderizzando il popup in un `TestBackend` e rileggendo le parole dal buffer.

   Due cose emerse strada facendo, tenute: una finestra di 4000 righe attorno al cursore quando
   si costruisce l'indice (è la stessa distanza che il ranking già usa, applicata prima del
   lavoro invece che dopo), e l'impostazione `completion` per spegnerlo, perché una lista che si
   apre da sola non è un aiuto per tutti.

   **Corretta una divergenza latente**: `editor_mut()` risolve il buffer con
   `active_editor_index()`, che ignora il pannello destro se lo split è chiuso, mentre
   `pane_editor_index()` no. Oggi `close_split` riporta il fuoco a sinistra e le due non
   divergono mai — ma l'accettazione di una parola prende gli offset da un buffer e li scrive con
   `editor_mut()`, e appoggiare *quella* scrittura su un invariante mantenuto a mano altrove è il
   modo di corrompere un file più avanti. Il completamento chiede l'indice come lo chiede
   `editor_mut()`.

3. ✅ **Chiuse le due code della 0.6.** Le schede del pannello Git si cambiano col click:
   `ui::git_tab_slots` dice dove sono, e il disegno ci renderizza dentro invece di rifare il
   calcolo per conto suo — è lo stesso motivo per cui esiste `tab_strip_layout`, e un hit-test
   che rifà il conto è un hit-test che un giorno non sarà d'accordo con quello che si vede.
   "Sostituisci tutti" ha l'anteprima: `FindState::preview` mostra l'occorrenza corrente coi
   gruppi già risolti e quante altre farebbero la stessa fine, perché con un pattern `$1` è
   indistinguibile da un dollaro letterale finché non lo si risolve — e a quel punto il file è
   già cambiato.

4. ✅ **Verificato guidando il binario vero**, non solo coi test unitari. Uno script apre `clee`
   in uno pseudo-terminale, rende l'output con `pyte` e rilegge la griglia di caratteri: il
   popup si apre a due lettere, offre le parole del buffer *e* le parole chiave di Rust perché
   il file è `.rs`, si restringe scrivendo, mette per prima la parola più vicina al cursore, si
   chiude con Esc senza toccare il testo, riappare scrivendo ancora, accetta con Tab e torna
   indietro in un solo passo di undo. Quindici controlli, tutti verdi.

   Da sapere se lo si rifà: CleeCode interroga il terminale (device attributes, protocolli
   kitty) e disegna il primo fotogramma solo dopo che quelle domande vanno in timeout, quindi un
   driver deve aspettare una *condizione*, non un tempo fisso. E `▶` da solo non basta a trovare
   il popup: c'è anche nel pulsante `▶ Run` della barra.

**0.8 — LSP** ← *fatta, non ancora rilasciata*

Client JSON-RPC scritto a mano su stdio in `src/lsp.rs`, `lsp-types` per i tipi, **un solo
server** (rust-analyzer) e **solo diagnostici**, come deciso. Niente `tower-lsp`, niente `tokio`:
il client usa lo schema che c'era già — thread, `mpsc`, `poll_lsp` nel ciclo a 30 fps accanto
agli altri poll. `didChange` ritardato di 400 ms sul `revision` di `Editor`, e manda il file
intero: la sincronizzazione incrementale vorrebbe una seconda descrizione di cos'è una modifica,
tenuta in pari con la rope a mano, e il risparmio non la paga visto che parte solo a pausa fatta.

**Due crate nuove: `serde_json` e `lsp-types`** (cinque in tutto col loro seguito, tutte Rust
puro). Coerente con il rifiuto di `git2`, che era per la dipendenza C e per hook e firma saltati,
non per il numero. Su `lsp-types` la valutazione è stata rifatta coi numeri: costa 3 crate, e
`fluent-uri` sembrava ripagarla da sola — poi si scopre che **`lsp_types::Uri` è solo un parser,
`FromStr` e basta, senza `from_file_path`**. La codifica percorso→`file://` è comunque a mano,
qui con i test che merita: spazi, accenti e lettera di unità Windows. `lsp-types` resta per le
struct del protocollo, perché l'handshake con rust-analyzer è pignolo.

**Cosa si vede:** sottolineatura colorata dove il server indica, numero di riga dello stesso
colore, e il messaggio della riga del cursore a destra nella barra di stato — accanto al
messaggio di stato, non al suo posto. Impostazione `diagnostics` per spegnere tutto.

**Verificato senza rust-analyzer**, che qui non è installato (e non c'è rustup). Tre livelli:
funzioni pure per framing, URI e conversione delle posizioni; il thread lettore guidato da una
trascrizione finta in memoria; e `Client` contro un **processo vero**, `scripts/lsp_stub.py`, che
riecheggia l'URI ricevuto invece di inventarne uno — uno stub con l'URI in conserva passerebbe
anche con la codifica rotta. Poi `scripts/drive_lsp.py` mette lo stub sul PATH col nome che
CleeCode cerca e guida l'editor vero: pyte tiene colore e attributi per cella, quindi
"sottolineato" e "rosso" si controllano davvero. Nove controlli.

**Il bug che solo l'end-to-end poteva trovare.** Tutti i test unitari usavano percorsi assoluti.
Un progetto aperto come `.` tiene le tab su `./src/main.rs`, e `uri_for` ci metteva davanti uno
slash producendo `file:///./src/main.rs`: un URI che si compila, parte, torna indietro dal server
e non nomina niente a nessuno dei due capi. Niente sottolineature e niente che dicesse perché.
Ora un percorso relativo **non ha URI** e lo dice, e l'app risolve il percorso una volta sola
(`canonicalize`, perché il server scioglie i symlink e su macOS `/tmp` torna indietro come
`/private/tmp`) tenendo la corrispondenza fra il percorso risolto e quello della tab.

Un dettaglio che vale per il prossimo che scrive codice Windows qui: la radice di un percorso si
chiede al *testo*, non a `Path::is_absolute()`, che risponde per la piattaforma su cui è
compilato — `C:\src` risulta relativo a una build Unix, e chi scrive quel codice non può provarlo.

Poi rust-analyzer diventa una sorgente in più per il popup della 0.7, innestandosi sul `Source`
che è lì apposta dalla 0.7.

**0.9 — Modalità IDE per Octave e Python**

Proposta arrivata il 2026-08-20 da una sessione parallela, con prototipi funzionanti e tre
documenti di handoff in `~/cleecode-octave-ws/` (fuori dal repo, non sotto git). Pannello del
workspace dal vivo, figure come tab di anteprima navigabili, preset `clee -w octave` e
`clee -w pylab`. **Non ancora portata nel repo**: qui c'è la valutazione, non il lavoro.

*L'inquadratura da preservare* — ed è la parte più preziosa dell'handoff: **una funzionalità
con due backend, non due lavori**. Ogni voce della lista vale per entrambi i linguaggi, e tutta
la superficie specifica del linguaggio sono quindici one-liner. Costruita come due
implementazioni parallele "che poi convergono" diventa due mezze funzionalità che non
convergono mai. Entrambi i prototipi emettono già lo stesso JSON con un campo `lang`.

**Ricontrollato qui, non preso sulla parola.** Quattro conferme e due correzioni.

Confermati:
- I quattro agganci nel repo sono esatti: `main.rs:286` (`is_default` prima di `load`),
  `terminal_panel.rs:365` (`cmd.env("CLEECODE", "1")`), `app.rs:668` (`DragTarget`),
  `preview.rs` che disegna PNG via `ratatui_image`.
- `add_input_event_hook` misurato in un PTY: **mediana 105 ms** (101–106), **zero** scatti
  durante un comando bloccante di 2 s, e niente stampato nella trascrizione dell'utente. Il
  numero dell'handoff è giusto al millisecondo.
- La trappola di `jsonencode`: una struct array 1x1 serializza a `{...}`, una 2x1 a `[{...}]`.
  `vars` è un cell array apposta — verificato che il cell array 1x1 dà `[{...}]`.
- Il bug dei workspace built-in che ombreggiano silenziosamente quelli dell'utente esiste
  davvero: `is_default(n)` risponde prima che `load(n)` guardi il file, e il messaggio d'errore
  di `save_in` nomina sempre `DEFAULT_NAME`. Oggi è latente — solo "Default layout" è
  riservato — e diventa reale nel momento in cui `octave` diventa un nome built-in. Va corretto
  **insieme** ai preset, non aggirato scegliendo nomi scomodi.

Corretti:
- **`sys.ps1` come oggetto non scatta una volta per statement nel PyREPL nuovo.**
  `HANDOFF-PYTHON.md` riga 27 dice "fires once per statement in **both** Python 3.13 REPLs".
  Misurato qui, stesso PTY, stessa Python 3.13.14, tre statement:

  | REPL | righe intere | carattere per carattere |
  |---|---|---|
  | basico (`PYTHON_BASIC_REPL=1`) | 4 | 4 |
  | PyREPL (predefinito) | **60** | **60** |

  Nel REPL basico l'affermazione regge esattamente (4 = i tre statement più la `print` finale).
  Nel PyREPL — che è il predefinito in un terminale vero, cioè quello che CleeCode avrà
  *sempre* — sono una **ventina di chiamate per statement**: PyREPL stringifica il prompt più
  volte per ridisegno, e non cambia niente se il testo arriva tutto insieme o digitato.

  Quindi il titolo "niente polling, non serve l'apparato di rilevamento delle modifiche di
  Octave, costo a riposo esattamente zero" **non regge come scritto**: uno `_snapshot()` messo
  dentro `__str__` girerebbe venti volte per comando. Il meccanismo resta utilizzabile, ma
  serve o un guardiano dentro `__str__` (ed è di nuovo rilevamento delle modifiche, cioè la
  cosa che l'handoff dice giustamente di non appiattire fra i due linguaggi), o un aggancio
  diverso: `post_run_cell` di IPython, che l'handoff già cita come più solido, o un audit hook
  su `exec`. Va deciso **prima** di costruirci sopra, perché è una delle tre cose che l'handoff
  stesso indica come da non pareggiare fra Octave e Python — solo che la differenza va nel
  verso opposto a quello scritto.
- **Il numero di `paperposition` non è una costante, ed è il foglio a esserlo.** Il meccanismo è
  confermato — una figura `position [0 0 800 600]` non stampa un PNG 800x600 — ma qui viene
  **739x554** con `-r100`, mentre l'handoff riporta 709x532 con `-r96`. I due numeri concordano:
  739/100 = 7,39 e 709/96 = 7,385, cioè lo stesso foglio di ~7,39x5,54 pollici, e i pixel sono
  pollici per DPI. Quindi la correzione dell'handoff (§6: forzare `paperposition` in pollici) è
  quella giusta; è solo la coppia di numeri in pixel che non va citata come costante.

Non verificato: la fingerprint O(n²) (7,46 ms/tick a 200 variabili) — è un dettaglio interno al
prototipo, plausibile, e non cambia la forma della cosa.

*Da sistemare nell'handoff prima di portarlo:* `HANDOFF-SHARED.md` si contraddice sul nome —
riga 11 dice `clee -w octavelab`, riga 77 dice `clee -w octave` e la sezione Naming argomenta
contro `octavelab`. Vale la seconda.

*Ordine di costruzione proposto* (dall'handoff, e mi convince): eseguire selezione e celle `%%`
dall'editor per prima, perché è quella che trasforma la cosa da visualizzatore in IDE; poi
traceback cliccabili, ispettore di variabili come tab, pannello history, completamento dalla
sessione viva (si innesta come terza sorgente in `complete.rs`, esattamente come l'LSP della
0.8), export delle figure. Il debugger è un livello a sé, sugli stessi canali.

*Aperto:* dove vivono i `.m` e i `.py` (`assets/octave/`, `assets/python/`), e i tre documenti
di handoff vanno portati in `docs/` insieme al codice — il loro valore è il ragionamento dietro
scelte che da fuori sembrano arbitrarie, ed è quello che un lettore futuro "sistemerebbe"
rompendole. `test/pty_strict.py` è l'unico test della rilevazione delle modifiche e serve una
casa anche a lui. Finché la copia in `~/cleecode-octave-ws/` esiste fuori dal repo ci sono due
copie divergenti della stessa cosa, e fra un mese nessuna delle due sembrerà quella buona.

**Dopo:** azioni Git che scrivono (stage, commit), altri server LSP. Le azioni che scrivono
stanno in fondo di proposito: leggere ha rischio zero, scrivere no, e il commit si fa già nel
terminale accanto — che è il punto del prodotto.

## Scorciatoie

Libere: `Ctrl+Shift+` `A C D H I J L P Q V X Y Z`. Occupate: `B E F G K M N O R S T U W`.
Restano vietati i tasti funzione e i simboli che il layout italiano mette già sotto Shift.
