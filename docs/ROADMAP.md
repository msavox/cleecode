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

  Quindi il titolo "niente polling, costo a riposo esattamente zero" non reggeva **come
  scritto**: uno `_snapshot()` dentro `__str__` girava venti volte per comando, e siccome
  `_snapshot()` chiama `_figures()`, che chiama `savefig()`, con una figura aperta voleva dire
  riscrivere il PNG venti volte per comando.

  **Deciso e risolto il 2026-08-20** — e il titolo alla fine regge, gli mancava un secondo
  meccanismo. Il prompt è il segnale sbagliato da solo ma è il *momento* giusto, perché viene
  disegnato dopo che lo statement è finito. Mancava qualcosa che dicesse che uno statement è
  girato, e un **audit hook su `exec`** lo dice esattamente una volta *se guarda il nome del
  file* del code object: il REPL compila ogni cosa che scrivi come `<python-input-7>`, o
  `<stdin>` in quello basico. Quindi l'hook marca e il prompt raccoglie.

  Misurato su una sessione: 65 ristringificazioni diventano **5 snapshot per 5 statement**,
  ognuno che vede il namespace come lo statement l'ha lasciato, e 16 tasti mai eseguiti non ne
  producono nessuno. `scripts/ide/python_cadence_test.py` è la prova di non regressione, e si
  rifiuta apposta di girare sul REPL basico.

  Le due risposte più semplici sono morte sotto PyREPL, misurate e non supposte:
  `readline.get_current_history_length()` — l'analogo diretto del trucco `numel(history())` su
  cui si regge il lato Octave — restituisce **0**, perché PyREPL tiene una history sua; e un
  audit hook che non guarda il nome del file vede **52 exec per 4 statement**. Tenere l'hook
  installato non costa niente di misurabile: lavoro numerico, cicli puri e duemila `open`+`write`
  vengono identici a tre decimali con e senza.

  Il confronto con Octave regge quindi ancora, e con più margine di quanto sembrasse: Octave
  campiona a 10 Hz e ha bisogno di una fingerprint per sapere se qualcosa si è mosso, Python ha
  una callback esatta per statement e gratis. Solo che per costruirla servono due meccanismi,
  uno per *quale* statement e uno per *quando è finito* — ed è anche il motivo per cui nessuno
  dei due da solo bastava.

  Resta aperto un solo ramo: **IPython non usa `sys.ps1`**, quindi vuole il suo
  `ip.events.register("post_run_cell", ...)`. L'handoff lo diceva già ed è più ufficiale di
  entrambe le metà di questo; va rilevato quale REPL sta girando invece di indovinare.
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

**Passo 1 fatto** il 2026-08-20: **eseguire selezione e celle `%%`**, `Ctrl+Shift+X`. Il seam
c'è dal primo commit come chiedeva l'handoff — `src/session.rs`, un `Language` con due
adattatori: quali nomi di programma sono quell'interprete, cosa gli si dice per eseguire un
file, come si cita un percorso per il suo prompt, che estensione vuole il file di appoggio. La
parte Octave era già cablata in `dnd.rs` per il pulsante Run (passare un `.m` all'Octave già
aperto invece di avviarne un secondo): è la stessa domanda, quindi è migrata lì e ha messo una
seconda risposta.

Il marcatore di cella è **uno solo per entrambi**: `%%`, con davanti il carattere di commento
del linguaggio — `%% titolo` in Octave, `# %%` in Python. Sono le due cose che quei mondi già
scrivono, quindi nessuno deve imparare una convenzione di CleeCode per usare la funzionalità. Un
file senza marcatori è una cella sola, che è la lettura onesta di "esegui questa cella" in uno
script che nessuno ha diviso.

Passa da un file temporaneo, mai incollando al prompt: l'handoff aveva ragione, un blocco
indentato incollato fa rispondere `IndentationError` a Python, e a un prompt Octave viene
riecheggiato riga per riga nella trascrizione dell'utente.

*Una riga tirata di proposito:* il **pulsante Run non cambia** comportamento per Python. La
macchina sotto adesso conosce entrambi i linguaggi, ma un prompt Octave è quasi sempre *il*
posto dove si sta lavorando, mentre un REPL Python aperto in un terminale di lato mentre editi
un'applicazione web non è dove `manage.py` deve girare. Mandare un file intero in una sessione
viva è un cambio di comportamento, e appartiene alla funzionalità che lo chiede — eseguire una
selezione — non a un pulsante che significava già altro.

*Scorciatoia:* `Ctrl+Shift+X`, eXecute. **Non** Shift+Invio, che è quello che usano tutti i
notebook e che un terminale non sa consegnare: la codifica non ha spazio per lo Shift dal VT100,
quindi funzionerebbe in due emulatori e non farebbe niente in silenzio in tutti gli altri.

*Provato contro interpreti veri*, non contro stub, perché il punto è che la **sessione** si
tiene quello che la cella ha fatto e uno stub proverebbe solo che una stringa è arrivata a un
prompt. `scripts/drive_cells.py` avvia un Octave vero e un Python vero dentro i terminali di
CleeCode, esegue una cella in ciascuno e poi chiede *alla sessione* la variabile: 12 controlli,
compreso che l'**altra** cella non è girata — altrimenti "esegui cella" è un Run travestito.

Per farlo l'imbracatura ha dovuto imparare a **rispondere alle domande del terminale**. Tutte le
scorciatoie dell'applicazione sono chord Ctrl+Shift, che esistono solo se CleeCode abilita il
protocollo kitty, e lo abilita solo se il terminale risponde a `CSI ? u`. Ora `pty_drive.py`
risponde a quella e alle altre due, il che rende provabile ogni scorciatoia dell'app — e come
effetto collaterale il primo fotogramma arriva in **1 secondo invece di 15**, perché non si
aspettano più i timeout.

**Passo 2 fatto**: i **preset**, `clee -w octave` e `clee -w pylab`, con il nome asimmetrico
deciso dall'handoff e per il motivo che ci dà — *Python* è ambiguo, un layout Django e uno
numerico non hanno niente in comune, mentre `octave` è già lo strumento di calcolo con i grafici
e un `octavelab` non disambiguerebbe niente.

**Una finestra terminale, due schede** — l'interprete e una shell semplice per git e pip. Una
seconda *finestra* toglierebbe schermo all'editor per sempre per tenerci una cosa che si usa un
minuto alla volta; una scheda è a un tasto e non costa niente. La scheda 1 è l'interprete, che è
dove atterra `Ctrl+Shift+X` e quindi è quello che devi avere davanti. `pylab` avvia il python del
venv attivo, se ce n'è uno: altrimenti aprirebbe un prompt senza i pacchetti del progetto, che è
l'unica cosa che quel preset esiste per evitare.

**Il layout si adatta alla finestra.** Sopra le 150 colonne il prompt va *accanto* all'editor,
che è dove lo mettono i desktop di Octave e MATLAB e per lo stesso motivo: l'output numerico è
largo e una riga di matrice in un pannello stretto va a capo diventando illeggibile. Sotto, va
sotto — non è un compromesso, è la risposta giusta, perché così entrambe le cornici hanno tutta
la larghezza. Un layout giusto a 200 colonne e inutilizzabile a 80 non è il layout migliore, è il
layout migliore per chi l'ha scritto.

**Corretto il bug dei nomi built-in**, come diceva l'handoff: nel codice, non aggirandolo con
nomi scomodi. Prima `is_default(n)` rispondeva *prima* che il disco fosse consultato, quindi un
workspace salvato con quel nome smetteva di aprirsi in silenzio. Adesso `workspace::resolve`
guarda prima il file e **il file vince**: salvare sotto un nome built-in è rifiutato, quindi un
file che collide può solo essere più vecchio del preset che ha preso il nome — ed è
insostituibile, dove il built-in è documentato e riproducibile. L'utente se lo sente dire, con
scritto come riavere il preset. Il rifiuto del salvataggio ora nomina il built-in giusto invece
di dire sempre `DEFAULT_NAME`, che era vero con uno solo ed è diventato una bugia con tre. E
`clee -w` elenca i built-in marcando quelli ombreggiati.

**Passo 3 fatto**: il **workspace dal vivo**, ed è una **finestra** e non una scheda — scelta
dell'utente, e ha ragione: una scheda è un posto dove vai, una finestra è una cosa che guardi di
sfuggita. Esegui una cella e alzi lo sguardo, che è il motivo per cui i desktop di Octave e
MATLAB lo tengono agganciato.

*Il visualizzatore è CleeCode stesso*, `clee --watch-workspace <dir>`, avviato dal preset in
quella finestra. Niente da installare, nessuno script esterno, e la tabella la disegniamo noi.
Non usa schermo alternato né modalità raw: Ctrl+C lo chiude come qualsiasi programma, il
pannello si ridimensiona senza che debba saperne niente, e se muore non ha sequestrato il
terminale. È un programma che stampa una tabella.

*L'hook si installa senza toccare la home dell'utente*, ed è meglio di quanto proponeva §5
dell'handoff. Non serve il blocco in `~/.octaverc`: il comando di avvio del preset è
`octave --no-gui --persist --path "$CLEECODE_OCTAVE_LIB" --eval cleecode_boot`, e le variabili
d'ambiente sono già lì. Verificato in un PTY: lo snapshot compare all'avvio, `seq` sale a ogni
comando, e la trascrizione dell'utente resta **pulita** — nemmeno una riga in più. Il `.octaverc`
resta la strada per chi digita `octave` da sé, e quando servirà sarà un comando esplicito come
`--install-app`, non una cosa che un preset fa di nascosto. Python non ha nemmeno quel problema:
`PYTHONSTARTUP` è una variabile d'ambiente e basta.

*I `.m` e i `.py` viaggiano dentro il binario* (`src/assets.rs`, `include_str!`) e vengono
scritti nella cartella temporanea all'occorrenza. Un binario installato da Homebrew non ha il
repository accanto, quindi un percorso che si risolve in sviluppo e non dopo sarebbe una
funzionalità che funziona solo per chi l'ha compilata.

*Una terza correzione all'handoff:* `HANDOFF-SHARED.md` diceva "entrambi i linguaggi emettono già
lo stesso JSON con un campo `lang`". Octave non lo emetteva — il campo c'era solo nel prototipo
Python. Aggiunto, così la vista sa di chi sta parlando invece di chiamarlo genericamente
"workspace".

*Provato contro un Octave vero*, `scripts/drive_workspace.py`: 11 controlli, dalla finestra che
c'è prima che qualcosa sia girato fino alla `magic(4)` riassunta, al NaN segnalato e alla
statistica che per un char è un trattino e non uno zero. E che l'**altra** cella non compare.

*Provato aprendo i preset davvero*, `scripts/drive_presets.py`: 14 controlli, entrambi i
linguaggi, in finestra larga e stretta — l'interprete è al prompt senza che nessuno abbia
digitato niente, la shell è accanto nella stessa finestra, e le cornici si spostano quando la
finestra si stringe. Un preset è una promessa su cosa compare quando scrivi il suo nome, e
l'unico modo di verificarla è scriverlo.

**Passo 4 fatto**: le **figure come tab**. Una finestra Qt viva non si può spostare dentro un
terminale, quindi alla sessione viene detto di non aprirne (`defaultfigurevisible` a off) e
consegna un'immagine: l'hook stampa su PNG a fine comando e `preview.rs` la disegna, che già
sapeva fare. Ridisegna e la tab cambia; una seconda figura è una seconda tab; e non toglie mai la
tastiera a quello che stavi scrivendo — apre a fianco, dividendo l'editor se c'è spazio.

*Misurato contro Octave vero, e i numeri dell'handoff reggono con una differenza:* la correzione
di `paperposition` dà **800x600 esatti**, la ristampa dopo `xlim` costa **37 ms** e una `surf`
3-D 93 ms. La prima stampa è 341 ms qui, non 813 — la macchina o la versione.

*Una cosa che l'handoff lasciava aperta è risolta.* §6 diceva "non ancora fatto: nella sonda le
figure sono ristampate a ogni tick, riusare il trigger del confine di comando". Fatto — la
stampa sta dentro `write_snapshot`, che gira solo quando la fingerprint cambia — **e c'è di
meglio**: Octave marca una figura `__modified__` quando qualcosa in lei si muove, e lo si può
azzerare. Quindi si ristampa solo la figura toccata. Con due figure aperte e un comando che non
ne tocca nessuna il costo passa da 130 ms a **1 ms**, che è la differenza fra un prompt che
risponde e uno che esita.

**Passo 5 fatto**: la **navigazione**, e va *dentro* l'interprete. Su una tab-figura `+`/`-`
avvicinano e allargano, le frecce spostano — o girano, se è una superficie — e `r` rimette il
grafico intero. Nessuno di questi tocca i pixel: viene mandato un comando al prompt, la sessione
ridisegna, e la tab raccoglie il PNG nuovo come raccoglie qualsiasi cambiamento al suo file.

*Perché non lo zoom raster:* ingrandire i pixel lascia le etichette degli assi a descrivere un
intervallo che non è più sullo schermo — il grafico dice da 0 a 100 mentre mostra da 25 a 75 — e
nessuna nitidezza sistema un numero sbagliato. Con la ridisegnatura a 37 ms la risposta onesta è
anche quella veloce.

*Comandi verificati contro Octave vero prima di scriverli:* `zoom(2)` porta `[0 100]` a `[25 75]`,
il pan sposta di un quarto della finestra, `axis auto` reimposta, e `view(az+15, el)` gira una
superficie. Il sidecar geometrico — emesso al passo 4 senza che niente lo usasse — adesso serve
davvero: `is3d` decide se una freccia sposta o gira, e `view` dà l'angolo da cui partire, perché
non esiste una forma relativa per nominarlo.

Le forme Python ci sono e sono nella stessa `Language`, quindi quando matplotlib verrà agganciato
non c'è una seconda implementazione da scrivere. Ancora non provate contro un interprete vero:
è l'unica parte di questo passo che non ho misurato.

**Passo 6 fatto**: **traceback cliccabili**, `src/locate.rs`. Doppio click su una riga di output
di un terminale che nomina un file e una riga e si apre lì, col cursore su quella riga.

*Non è solo per i traceback*, ed è la cosa migliore che ha: qualunque cosa stampi
`percorso:riga` funziona — cargo, gcc, eslint, pytest, `grep -n`. È nata per gli interpreti e
serve a tutto il resto dell'editor.

*Ogni formato è preso da output reale, non dalla memoria*, e i test lo citano: Python dice
`File "…", line 2`; Octave dice `boom at line 3 column 3` nominando la *funzione* quando ce l'ha
e il file quando non ce l'ha — quindi un nome nudo è una pista e non una posizione, ed è marcato
come tale prima di cercare `boom.m` nel progetto.

*Il bug trovato guidando, non leggendo:* `grep -n` stampa `file:riga:**testo**`, non
`file:riga:colonna`. Il riconoscitore voleva un numero anche nel terzo campo, e un doppio click
su un risultato di grep non portava da nessuna parte. Adesso i due punti si dividono da sinistra
prendendo il primo posto dove quello che precede somiglia a un percorso — che è anche ciò che
tiene insieme `C:\src\main.rs:12:5`, perché dividere alla lettera di unità lascia `C`, che non
somiglia a niente.

**Passo 7 fatto**: il **completamento dalla sessione viva**, innestato sul seam `Source` messo
nella 0.7 apposta per questo — una variante in più, non un secondo popup. In un file il cui
linguaggio ha una sessione aperta, i nomi che quella sessione tiene sono offerti in verde, a
distanza zero: una variabile che esiste nell'interprete è vicina quanto una parola può essere, e
più di una scritta quaranta righe più su.

*Il punto è quello che nessun'altra sorgente può dare:* una variabile creata al prompt non sta in
nessun file, quindi niente che legga i buffer potrebbe suggerirla per quanto a lungo la sessione
la tenga. Provato esattamente così — `calibrazione_lunga` creata solo al prompt e poi completata
in un file dove non compare.

*Quello che ancora non fa:* i nomi delle funzioni della sessione. `completion_matches` di Octave
li darebbe, ma va valutato nell'interprete, e valutare qualcosa per ogni prefisso digitato vuol
dire un canale di richiesta e risposta che oggi non c'è — lo snapshot va in una direzione sola.
Le funzioni dei file aperti le dà già la sorgente delle parole del buffer.

**Passi 8 e 9 fatti**: **export delle figure** (`e` su una tab-figura scrive un PDF nella cartella
del progetto) e **history** sotto le variabili nella finestra workspace.

L'export lo chiede alla sessione, non converte il PNG a schermo: l'interprete ha ancora la
figura e la può disegnare a qualsiasi dimensione, mentre un PDF fatto da una bitmap è una bitmap
in una busta. PDF perché un grafico esce dall'editor per entrare in un documento, e lì lo vuole
vettoriale; il PNG per chi vuole i pixel è già su disco.

La history era "quasi gratis" come diceva l'handoff — l'hook Octave legge già `history()` per il
rilevamento delle modifiche — **ma sarebbe stata inutile senza una cosa in più**: conterrebbe
anche i comandi che CleeCode inietta, e un elenco di comandi recenti pieno di
`figure(1); zoom(2);` è l'elenco di quello che ha fatto CleeCode, che nessuno ha chiesto di
vedere. Adesso ogni comando che CleeCode digita finisce con un commento che lo marca. Due lavori
con una riga: nella trascrizione dice *chi* ha scritto quella riga, e nella history è ciò che
permette di escluderla. Marcato con un commento e non con una convenzione sulla forma, così chi
scrive `figure(2)` di suo non viene mai scambiato per noi.

Python non ha ancora la history: sotto PyREPL `readline` restituisce 0 (misurato quando si
decideva la cadenza dell'hook), quindi vuole la strada di `_pyrepl` o quella di IPython. Detto
qui perché è esattamente il tipo di differenza fra i due linguaggi che l'handoff dice di non
appiattire.

**Passo 10 fatto**: l'**ispettore di variabili**, `Ctrl+Shift+I`. Offre i nomi che la sessione
tiene e apre quello scelto: i valori, uno schermo alla volta, con righe e colonne numerate.

*Il giro di andata e ritorno che l'handoff descrive, ma senza toccare il prompt.* Lo snapshot
dice cosa una variabile *è*; non può dire cosa *contiene*, perché una matrice 2000x2000 sono
quattro milioni di numeri che nessuno vuole su disco dieci volte al secondo. Quindi lo schermo si
chiede — e la prima versione lo chiedeva **digitando al prompt**, che è esattamente ciò che il
principio numero uno di questo progetto dice di non fare.

*E il bug l'ha detto prima ancora del principio.* Scritture **identiche byte per byte** allo
stesso terminale venivano eseguite la seconda volta e ignorate la prima, con qualsiasi attesa in
mezzo e da qualsiasi percorso di codice — verificato registrando i byte davvero scritti sul pty.
Invece di continuare a inseguirlo ho cambiato canale: CleeCode scrive un file di *richiesta*,
l'hook che gira già a ogni momento di inattività la legge e risponde. Niente prompt, niente riga
nella trascrizione dell'utente, e niente da azzeccare con un line editor. Al primo colpo, sempre.

*Quello che ancora non fa:* modificare le celle. La direzione di scrittura è una riga —
`a(3,4) = 7;` — ma è una riga che **cambia i dati dell'utente**, e vuole una modalità di
inserimento con un cursore nella griglia. Le operazioni che leggono hanno rischio zero, quelle
che scrivono no: è la stessa riga tracciata nella 0.6 per il pannello Git, e vale ancora.

**Passo 11 fatto**: il **debugger** — breakpoint nel gutter, dove sei fermo, e il workspace del
frame nel pannello. `Ctrl+Shift+P` mette o toglie un breakpoint sulla riga del cursore.

*Quattro misure prese prima di scrivere una riga di codice, e sono loro a dettare la forma:*

| domanda | risposta misurata |
|---|---|
| l'hook continua a scattare al prompt `debug>`? | **sì** — quindi essere fermi è una cosa che CleeCode *vede*, non che l'utente deve dire |
| `dbstop` funziona da dentro l'hook? | **sì**, via `evalin` — quindi mettere un breakpoint non lascia una riga nella trascrizione |
| l'hook può vedere le variabili del frame fermo? | **sì**, con `evalin("caller", ...)`; `evalin("base", ...)` dà quelle di fuori |
| `dbstep` funziona da dentro l'hook? | **no** — non dà errore e non si muove |

Quindi lo stepping resta una cosa che si scrive al prompt di debug, e il manuale lo dice invece
di offrire un tasto che non farebbe niente in silenzio. La metà che funziona da qui — mettere i
breakpoint, seguire dove sei, mostrare cosa c'è in quel frame — è quella che vale, perché è
quella che a mano è scomoda.

*Due bug trovati guidandolo:*
- **I nomi venivano dal frame e i valori dalla base**, quindi ogni lettura falliva con
  "undefined" e il `try/catch` del tick si mangiava l'intero snapshot. Sintomo unico: un pannello
  che smetteva di aggiornarsi. È la seconda volta che quel catch silenzioso costa un'ora, quindi
  adesso `CLEECODE_DBG_LOG` fa scrivere il motivo da qualche parte.
- **Quando la sessione si fermava, CleeCode si prendeva la tastiera** per mostrare la riga —
  proprio mentre stavi per scrivere `dbstep` al prompt. Adesso mostra senza prendere, come fa
  una figura che si apre.

**Con questo l'ordine di costruzione dell'handoff è finito**, debugger compreso.

---

**Passo 12 fatto**: **Python in pari con Octave**. Erano quattro le cose che mancavano —
history, ispettore, debugger, figure — e adesso ci sono tutte, guidate dallo stesso driver che
guida Octave (`scripts/drive_python.py`, gli stessi controlli in una passata sola).

Il senso del seam si vede qui: il lato Rust ha cambiato *quattro variabili d'ambiente e un
campo JSON*. Tutto il resto è stato lavoro dentro `cleecode_pyws.py`, perché i due interpreti
rispondono alle stesse tre domande scritte negli stessi tre file.

*Ogni pezzo ha voluto una strada sua, e nessuna somigliava a quella di Octave:*

| cosa | Octave | Python |
|---|---|---|
| history | `history()` | `readline` ne riporta **zero** — misurato; sta nel reader di PyREPL, 105 voci |
| variabili del frame | `evalin("caller", …)` | `frame.f_locals`, dallo stesso frame in cui pdb si è fermato |
| breakpoint | `dbstop` dentro l'hook | `pdb`, con il tracing acceso per la durata **di una sola istruzione** |
| stepping | `dbstep` al prompt | `n` e `c` al `(Pdb)` — e la riga di stato adesso dice quale delle due |
| figure | ristampa se `__modified__` | risalva se `fig.stale` — la stessa idea con l'attrezzo di matplotlib |

Il tracing acceso per una sola istruzione è la scelta che conta. Il prompt di Python **non è
fermo**: PyREPL ridisegna la riga a ogni tasto premuto, quindi una `settrace` lasciata accesa lì
pagherebbe una chiamata per riga di codice per ogni *carattere digitato*, e non prenderebbe
niente. L'audit hook scatta una volta sola, subito prima dell'istruzione dell'utente, che è
l'unica finestra in cui un breakpoint può essere raggiunto. Il prompt poi la rispegne.

*Tre bug, e due erano miei modi di perdere tempo:*
- **bdb senza `botframe` si ferma dappertutto.** `set_continue()` vuol dire "corri fino a un
  breakpoint" solo se `botframe` è impostato; senza, la prima riga che vede è quella di PyREPL e
  si ferma lì. Impostarlo dà anche la velocità: una chiamata dentro un file senza breakpoint non
  viene più tracciata riga per riga.
- **`/var` contro `/private/var`.** CleeCode scrive il path risolto, Python importa quello con il
  link; `bdb` li confronta dopo `abspath`, che i link non li segue. Stesso file, due nomi, e la
  sessione passava oltre il breakpoint **senza un errore da nessuna parte**. Risolto una volta
  sola sovrascrivendo `canonic`, che è il punto da cui passano entrambi i lati del confronto.
- **Due volte ho inseguito un bug in un binario vecchio.** Prima il `include_str!` non
  ricostruito, poi — peggio — i driver che usano `target/debug` mentre io ricostruivo solo
  `--release`. Il codice era giusto da un pezzo. Da qui in avanti: prima di credere a un FAIL,
  controllare *quale* binario ha appena girato.

E `CLEECODE_DBG_LOG` adesso c'è anche per Python, per lo stesso motivo per cui c'è per Octave.

*Quello che resta dichiarato, non nascosto:* l'ispettore mostra e non modifica, e le figure
vogliono matplotlib installato **nello stesso python del terminale** — su questa macchina
`brew install python-matplotlib` l'ha messo per il python3.14 di brew, mentre `python3` è pyenv
3.13 e non lo vede. Il driver lo dichiara come SKIP invece di far finta di aver controllato. Il debugger è un livello a
sé, sugli stessi canali.

*Il materiale è nel repo* e la copia fuori è stata cancellata: i `.m` in `assets/octave/`, i
`.py` in `assets/python/`, le imbracature in `scripts/ide/`, e i tre documenti in
`docs/design/ide-mode*.md` — quelli valgono quanto il codice, perché sono il ragionamento dietro scelte
che da fuori sembrano arbitrarie ed è esattamente quello che un lettore futuro "sistemerebbe"
rompendole.

**Dopo:** azioni Git che scrivono (stage, commit), altri server LSP. Le azioni che scrivono
stanno in fondo di proposito: leggere ha rischio zero, scrivere no, e il commit si fa già nel
terminale accanto — che è il punto del prodotto.

## Scorciatoie

Libere: `Ctrl+Shift+` `A C D H I J L P Q V X Y Z`. Occupate: `B E F G K M N O R S T U W`.
Restano vietati i tasti funzione e i simboli che il layout italiano mette già sotto Shift.

---

## 0.9.1 — quattro bug trovati dall'uso vero (2026-08-21)

**La lezione, prima dei bug:** ho misurato tutto il lato numerico su un Mac locale e ho
rilasciato dicendo che Linux è supportato, senza mai chiedere **dove gira davvero Octave**. La
risposta era "un server remoto via ssh", che è il posto dove un IDE da terminale serve di più ed
è l'unico che non avevo provato. Tre dei quattro bug qui sotto erano invisibili da qui.

| bug | perché nessuno dei dieci driver lo ha preso |
|---|---|
| **il pannello si svuotava a intermittenza** | `newest_in` prendeva il `.json` più recente, e nella cartella ce ne sono quattro tipi — snapshot, domanda dell'ispettore, risposta, breakpoint. Ogni driver usa **o** il pannello **o** un file di richiesta, mai i due insieme, e il vuoto dura fino al tick dopo |
| **i breakpoint smettevano di funzionare** | stessa causa, conseguenza peggiore: quel `Watch` è anche da dove `publish_breakpoints` ricava il path, quindi scriveva in `break-break-0.json` |
| **i plot si aprivano in finestre vere** | l'hook lo installava solo il `--eval` del preset. La funzione funzionava **esattamente nel caso che i test pilotano**, che è la disposizione più lusinghiera possibile |
| **su una macchina headless non si poteva disegnare** | qui c'è sempre un display |

*Le correzioni, e cosa le ha decise:*

- **`newest_in` guarda solo gli snapshot**, e il prefisso è una costante invece di una stringa
  scritta in due punti — distinguerli a occhio è quello che è andato storto.
- **`PKG_ADD` più `OCTAVE_PATH`**: Octave esegue un file `PKG_ADD` quando una cartella entra nel
  load path, e `OCTAVE_PATH` ce la mette per **qualunque** Octave. Meccanismo di Octave, non
  nostro, e il comando del preset torna a leggersi come una riga che scriverebbe una persona.
  Si antepone al path dell'utente invece di sostituirlo: perdergli le sue cartelle sarebbe un
  bug molto peggiore di quello che stavo correggendo.
- **gnuplot quando non c'è un display.** Una sessione che CleeCode pilota non mostra mai una
  finestra, quindi al toolkit basta saper *stampare*. Misurato: gnuplot e qt producono la stessa
  figura, stessa dimensione, stessa geometria degli assi — 451 ms contro 298. Lento e presente
  batte veloce e assente. Senza nessun toolkit, il pannello lo dice dal primo snapshot invece di
  lasciare che `figure()` fallisca dentro lo script dell'utente.
- **Il pannello si apre da qualsiasi layout.** Esisteva solo dentro i due preset, quindi un
  workspace salvato tuo — cioè quello in cui sta davvero chi usa CleeCode da un po' — non ne
  aveva e non poteva chiederne uno.

*E una cosa che vale più dei quattro bug:* `drive_inspect` **asseriva che il bug fosse il
comportamento giusto**. Chiudeva l'ispettore e aspettava che "6x6" sparisse da **tutto lo
schermo** — cosa che succedeva perché nello stesso momento si stava svuotando la tabella. Un
test che guarda troppo largo trasforma un bug in un PASS. Adesso guarda la cornice
dell'ispettore, e separatamente che la tabella sia rimasta.

---

## 0.9.2 — la cattura dei plot diventa una scelta (2026-08-21)

**La lezione:** la cattura delle figure era nata come l'unico modo in cui il plotting poteva
funzionare — una finestra Qt viva non si può spostare dentro un terminale — e da lì è rimasta
l'unico comportamento possibile, senza che nessuno l'avesse mai deciso. È un default giusto e una
regola sbagliata: su un desktop una finestra vera ha zoom, pan e rotazione fatti dal toolkit, e
chi li voleva non aveva modo di chiederli. Un default diventa una prigione nel momento in cui
smetti di poterlo spegnere.

*Le decisioni, e cosa le ha guidate:*

- **La domanda è "c'è uno schermo", non "sono in ssh".** La prima versione rifiutava le finestre
  via ssh per principio, che è giusto per un `ssh host` nudo e sbagliato per `ssh -X` con XQuartz
  dall'altra parte, dove il plot si apre sullo schermo davanti a cui l'utente è davvero seduto.
  Lento su un link sottile, e affare suo. Quindi si guarda solo `DISPLAY`/`WAYLAND_DISPLAY`, più
  l'eccezione di macOS e Windows che hanno un window server e nessuna variabile che lo nomini.
- **Dove la scelta non c'è, la riga lo dice invece di sparire.** Su una macchina senza schermo
  "finestre" significa nessun grafico: l'impostazione resta a `on — nessun display` e non prende
  Enter. Un interruttore che legge "off" mentre le tab continuano ad arrivare è un interruttore
  rotto; una riga assente è la domanda "perché non posso spegnerlo" senza risposta.
- **Una variabile sola, `CLEECODE_PLOTS`, per tutti e due i linguaggi.** Un Octave che tiene le
  sue finestre qt e un matplotlib che tiene le proprie sono la stessa risposta alla stessa
  domanda. `MPLBACKEND` viene messo solo in modalità tab: non messo, matplotlib fa quello che fa
  ovunque, che è tutto il punto dell'impostazione.
- **Vale dalla sessione dopo, e lo dice.** Un interprete sceglie il backend una volta sola,
  all'avvio, e una finestra già a schermo non si convince a diventare un'immagine.

*Tre bug trovati strada facendo, tutti dallo stesso posto — un Octave headless su Linux:*

| bug | perché era invisibile da qui |
|---|---|
| **una tab di figura chiusa tornava** | lo snapshot elenca le figure che la sessione *tiene*, e il poll lo leggeva come "mostra queste". Con una figura sola non si vede mai: serve chiuderne una e disegnare nell'altra |
| **Run scriveva un comando di shell al prompt di Octave** | `/usr/bin/octave` esegue `octave-cli-11.3.0` sulle build senza Qt, e Linux taglia il nome a quindici caratteri. Su un Mac l'eseguibile si chiama `octave` e basta |
| **l'avviso di Octave su gnuplot arrivava al primo plot** | qui gnuplot non viene mai scelto, perché il display c'è |

*E il layout:* i preset mettono il prompt sotto a ogni larghezza. Di fianco si legge bene finché
non si apre una figura — poi l'editor si divide per mettere il grafico accanto al codice che lo ha
disegnato, e con tre colonne ogni metà è un terzo di finestra. Un plot largo un terzo di finestra
è una miniatura.

---

## 0.10 — il language server entra nella lista che c'era già (2026-08-22)

**La decisione presa nella 0.7 ha pagato qui.** Il popup era stato costruito con dentro un
`Source` che aveva una variante sola vera, e la motivazione scritta allora era: quando arriverà
l'LSP non sarà una funzionalità nuova da disegnare, sarà una seconda sorgente che si innesta.
È andata esattamente così — il disegno, i cinque tasti, la non-modalità, l'accettazione in un
passo di undo non sono stati toccati. Quello che è servito è una variante in più, un colore, e
il modo di far entrare dei candidati **in ritardo**.

*Il ritardo è tutto il problema.* Le parole del buffer si raccolgono nel momento in cui il popup
si apre; la risposta del server arriva qualche frame dopo, in un editor che nel frattempo può
essersi mosso. Da qui le tre scelte che contano:

- **La lista non aspetta mai.** Si apre sulle parole del file e i nomi del server ci cadono
  dentro dopo. Un popup che aspettasse rust-analyzer sarebbe un popup che a volte non c'è, ed è
  peggio di uno che a volte è più povero.
- **`absorb` ri-ordina, non appende.** Un suggerimento che merita la prima riga deve poterci
  arrivare, altrimenti la seconda sorgente è una nota a piè di pagina.
- **Una riga scelta e una riga di default non sono la stessa cosa.** Questa l'ha trovata un test,
  scritto mentre pensavo di documentare il comportamento giusto: tenevo la selezione *sempre*, e
  così il nome migliore del server arrivava **sotto l'evidenziazione** e Invio scriveva ancora la
  parola di prima. La riga zero prima che le frecce siano toccate è dove il popup si è aperto,
  non un posto scelto da qualcuno. Ora `touched` distingue le due cose, e `refilter` lo rimette a
  zero: una parola diversa è una lista diversa.

*Quattro decisioni di protocollo, e la ragione di ciascuna:*

- **La richiesta scavalca `QUIET` di proposito.** Il debounce esiste perché il server non
  rianalizzi un file che stai ancora scrivendo; una richiesta di completamento è una domanda su
  *questo* testo, e una risposta sul testo di quattrocento millisecondi fa non è una risposta
  giusta più lenta, è una risposta sbagliata. Quindi `didChange` parte prima della domanda. Costa
  un messaggio per parola scritta — gli editor che ne mandano uno per tasto ne mandano di più.
- **L'id è l'unica guardia.** Il set delle richieste in volo è condiviso con il thread lettore,
  invece della scorciatoia "la prima risposta è l'handshake, tutto il resto è completamento": vera
  oggi e silenziosamente falsa il giorno che si aggiunge un terzo tipo di richiesta.
- **Si dichiara `snippetSupport: false` a voce.** È il default, ed è proprio il default su cui ci
  si appoggia: un server a cui non hai detto niente può mandare snippet lo stesso, e `${1:self}`
  dentro un buffer è roba che l'utente deve disfare a mano. `word_of` regge comunque il caso.
- **La risposta si legge a mano, non con `CompletionResponse`.** `CompletionList` ha `isIncomplete`
  obbligatorio: un server che lo omette costerebbe **l'intera lista**, cioè un popup vuoto,
  indistinguibile da un server che non aveva niente da dire. Stesso motivo per cui un item
  illeggibile è una riga persa e non la lista.

*Il ranking non è raddoppiato.* Niente tier tutto suo per l'LSP: i candidati del server entrano
negli stessi tier di prefisso, ordinati per `sortText` — che è il giudizio del server su cosa sia
più pertinente **in quel punto**, cioè la stessa domanda a cui risponde `distance`. Quindi
l'ordine del server *è* la distanza, e la prima proposta compete con una parola scritta sulla riga
del cursore. Due dialetti di "quanto è pertinente" sarebbero lo stesso difetto contestato a
`tokio` e a `rg` nelle release precedenti.

*E un dettaglio che si vede solo a schermo:* un item si riduce alla parola che scriverebbe.
rust-analyzer etichetta una funzione `push_str(…)` e una macro `println!(…)` perché quelle
etichette sono scritte **per essere lette**; inserirle com'è lascia parentesi nel file che
l'utente deve tornare a togliere. La lista completa *una parola*, e questo è il punto in cui lo
dice.

**L'impostazione ha cambiato nome:** `diagnostics` → `language_server`, con l'alias che continua a
leggere la vecchia. Adesso governa due cose e chiamarla come una sola sarebbe stato un nome che
mente.

*Come è stato provato,* visto che rust-analyzer qui non c'è: `scripts/lsp_stub.py` adesso risponde
anche a `textDocument/completion`, e **costruisce la risposta con la domanda** — l'etichetta dice
la riga e la colonna su cui è stato interrogato e la parola che ha trovato lì nel testo che gli è
stato mandato. Così `du_line4_col2` sullo schermo è la prova che il file è arrivato, che la
posizione è contata come la conta lui, e che la risposta è tornata nel popup che l'aveva chiesta.
Una lista inventata dallo stub passerebbe con tre di queste quattro cose rotte.

**E un PASS per il motivo sbagliato, preso al volo:** il controllo del colore guardava la riga in
cima alla lista, che è quella *selezionata* — nera su ciano per via dell'evidenziazione. Passava
qualunque colore avessero le sorgenti, e avrebbe continuato a passare togliendo il colore del
tutto. Adesso guarda una riga non selezionata. È la stessa lezione di `drive_inspect` nella 0.9.1,
e ha fatto in tempo a ripresentarsi in due release.

### L'ultima scheda si può chiudere (2026-08-22)

Chiudere l'ultima scheda rimetteva al suo posto un buffer senza nome, e quindi l'ultima scheda era
l'unica che non si potesse chiudere: chiedevi che sparisse e si sedeva lì una cosa identica. Il
commento nel codice diceva perché — "il resto dell'app dà per scontato che ci sia sempre qualcosa
da mostrare" — ed è vero: gli ottanta e passa posti che chiedono "il file corrente" non possono
crescere ognuno un ramo per il caso in cui non ce n'è uno.

La risposta non è farglielo crescere: è `scratch`, un buffer vero che nessuna scheda indica e che
nessuno disegna. `editor()` e `editor_mut()` ci ripiegano invece di indicizzare una lista vuota, e
non ci arriva niente in uso normale perché l'editor smette di prendere tasti quando il suo
riquadro è vuoto e il renderer disegna lo stato vuoto invece di un buffer. Sta lì perché un
chiamante che chiede lo stesso ottenga un buffer e non un panic.

*E lì accanto c'era un bug latente*, che non era mio ma è saltato fuori guardando: `.min(len - 1)`
va sotto zero se la lista si è svuotata, e si svuota — chiudere un file chiude anche l'anteprima
che era una vista di quel file, quindi le ultime *due* schede se ne vanno con un tasto solo. Ora
c'è un posto solo, `nothing_open`, e ci si arriva da entrambe le strade.

### Il pannello Git scrive (2026-08-22)

Era in fondo alla coda da tre release, con una motivazione scritta: leggere ha rischio zero,
scrivere no, e il commit si fa già nel terminale accanto. La motivazione regge ancora — ed è
proprio lei che ha deciso *cosa* entra e cosa no, invece di essere un motivo per non fare niente.

**Quello che c'è:** una scheda Stato, prima di tutte, con ogni file modificato e davanti le due
lettere di git. `S` mette in stage la riga sotto il cursore, `U` la toglie, `A` mette tutto, `C`
chiede un messaggio e committa, `Invio` apre il file. Su Branch, `Invio` ci passa. `X` butta via
le modifiche a un file, dietro domanda.

**Quello che non c'è, e perché:** push e pull possono fermarsi a chiedere una password, e in un
pannello non c'è nessun terminale in cui chiederla — resterebbero appesi e si vedrebbero solo
fallire. Merge e rebase vogliono risolvere conflitti, che è una funzione dell'editor tutta sua.

*Le decisioni che contano, e cosa le ha guidate:*

- **Le due lettere di git restano due lettere.** Un enum nostro avrebbe dovuto crescere un ramo di
  ripiego — `U` per un conflitto di merge, `T` per un file diventato symlink — e un ramo di
  ripiego è dove finiscono gli stati a cui nessuno ha pensato, per essere mostrati sbagliati.
  Sono anche quello che mostra qualsiasi altro strumento git e quello che spiegano le man page.
  `MM` è un file aggiunto e poi cambiato ancora: una lettera sola ne perderebbe metà.
- **Le grafie sono quelle vecchie.** `reset HEAD --`, `checkout HEAD --`, `checkout <branch>` e non
  `restore`/`switch`, che sono di git 2.23 (2019). Abbastanza recenti da mancare su un server di
  lunga vita — e un server di lunga vita raggiunto via ssh è esattamente il posto dove un editor
  da terminale serve. La stessa lezione della 0.9.1, applicata prima di prenderla in faccia.
- **I percorsi partono dalla cima dell'albero.** `--porcelain` li stampa relativi al top del
  repository, mentre il pannello gira nella cartella in cui CleeCode è stato aperto. Da una root
  due livelli più giù, `src/app.rs` sarebbe un file che non c'è e git risponderebbe che lo
  pathspec non corrisponde a niente — vero e inutile. Quindi lo snapshot si porta dietro `top` e
  le azioni passano percorsi assoluti.
- **`-z`, e non è un dettaglio.** Senza, git *quota* un percorso con uno spazio o un accento —
  `"src/prova nuova.rs"` — e ogni chiamante dovrebbe saper togliere le virgolette. Con, i percorsi
  arrivano come sono su disco, separati da un byte che dentro non ci può stare. Un rename porta
  con sé il nome di prima come campo a parte: letto come una voce diventerebbe un file chiamato
  `rc/old.rs` con lettere di stato `sr`, quindi la camminata lo salta.
- **Una scrittura per volta, su un thread.** `git commit` fa girare gli hook, e un pre-commit che
  lancia una suite di test fermerebbe il frame loop — editor, terminali e orologio insieme. E due
  `git add` in corsa per il lock dell'index sono un errore che si legge come un tasto ignorato.

**La domanda dello scarto è l'unica cosa da cui non si torna indietro,** e le regole sono tutte
lì per questo: prende *una* lettera — quella della lingua in cui è scritta la domanda — e legge
ogni altro tasto come un no, compresi `s`, `u`, `a` e `c`, che sulla lista dietro fanno qualcosa.
Un file di cui git non sa niente viene rifiutato *prima* della domanda invece che dopo la
risposta: non c'è nessuna versione precedente a cui tornare, e l'unico modo di onorare la parola
sarebbe cancellare il file, che è `rm` nel terminale. Un test in `i18n.rs` verifica che il testo
della domanda nomini il tasto che la risponde, perché sono due pezzi di testo lontani e un
riquadro che dice "S / N" e risponde solo a `y` sembra rotto mentre funziona esattamente come
scritto — davanti all'unica azione irreversibile.

*Come è stato provato:* `scripts/drive_git.py` fa un repository vero, lo cambia, e dopo **ogni**
azione chiede al repository com'è messo — non al pannello. Un pannello che disegnasse "in stage"
senza mettere niente in stage passerebbe qualsiasi controllo fatto sullo schermo, e tutto il punto
del lato scrittura è che sia successo qualcosa su disco. Il controllo più utile dei diciannove è
quello in mezzo allo scarto: preme `s` mentre la domanda è aperta e verifica che il file sia
ancora modificato.

**Tre bug nel driver, tutti dello stesso tipo — guardare troppo largo:** `git status --porcelain`
messo in `.strip()` perde lo spazio iniziale, e `" M main.rs"` diventa indistinguibile da un file
in stage; aspettare che `main.rs` compaia a schermo si accontentava dell'**albero dei file dietro
al pannello**, quindi il test proseguiva su un pannello che stava ancora chiedendo a git; e
cercare il colore della riga selezionata per nome (`cyan`) non trovava mai niente, perché ratatui
scrive i colori come codici a 256 e pyte li restituisce in esadecimale. Nessuno dei tre era un bug
del programma, e tutti e tre avrebbero potuto passare per uno.

### Tre controlli che non controllavano niente (2026-08-22)

`drive_workspace` falliva su "and the other cell's are not", ed era il driver ad avere torto: la
finestra workspace mostrava esattamente `b`, `nn`, `s` e nessun `first`. `workspace_pane`
tagliava la riga sull'**ultimo** `││`, che è anche il bordo fra albero dei file ed editor, quindi
si portava dentro le righe dello script — e `first` era scritto a riga 2 del file aperto.

*E tirando quel filo ne sono venuti fuori altri due, peggiori.* Lo stesso taglio in
`drive_inspect` e `drive_python` serviva a dire "al prompt dell'utente non è stato scritto
niente". Sul serio leggeva **una colonna di bordo vuoto**: il riquadro più a destra su quelle
righe non è il terminale, è la finestra workspace accanto. Quei due controlli passavano a
prescindere — sarebbero passati anche se CleeCode avesse scritto l'intera richiesta al prompt.
Un controllo che verifica un'**assenza** e guarda nel posto sbagliato è un PASS gratis, ed è più
insidioso di uno che guarda troppo largo: quello almeno può fallire.

Adesso `Session.frame_of(needle)` cammina fino ai bordi del riquadro in cui il testo è davvero
disegnato, e le note dei due controlli stampano cosa hanno letto — perché la prova che un
controllo guardi nel punto giusto deve stare nel suo output, non nella fiducia di chi l'ha
scritto. È la terza volta in tre release che salta fuori la stessa forma di errore (`drive_inspect`
nella 0.9.1, il colore della riga selezionata nella 0.10, questi tre): quando un controllo passa,
vale la pena guardare *cosa* ha guardato.
