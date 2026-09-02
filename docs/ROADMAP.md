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

### L'audit dei driver, e i due bug che teneva nascosti (2026-08-22)

Dopo i tre controlli vacui trovati a mano, un agente ha passato tutti gli undici driver con una
regola sola: un controllo sospetto va **dimostrato** vacuo — stampando cosa legge davvero, o
rompendone la premessa e verificando che passi lo stesso — non dichiarato tale leggendolo. Ne ha
confermati quattordici. La maggior parte erano innocui; due nascondevano bug veri, ed è per quelli
che l'esercizio valeva.

**L'ispettore Python non rispondeva mai.** Il controllo aspettava `35` da qualche parte sullo
schermo, e 35 è il Max di `arange(36)`, già scritto nella riga di riepilogo del pannello prima che
all'ispettore fosse chiesto niente. Irrigidito a una terna di valori della matrice, ha mostrato
"Asking the session…" per sempre. La causa: il lato Python risponde da `sys.ps1` e dal debugger, e
**nessuno dei due gira mentre nessuno digita** — che è esattamente il momento in cui l'ispettore
viene aperto. Octave non ha il problema perché `add_input_event_hook` scatta da fermo;
`_slice_watcher` è quell'hook, ricostruito con quello che Python ha.

**Le figure non si ridisegnavano su nessuna macchina con uno schermo.** `zoom`, pan e reset
mandavano il comando, la sessione zoomava per davvero — xlim da 0..100 a 25..75, misurato — e la
tab continuava a mostrare la prima immagine. Octave marca `__modified__` solo sotto gnuplot; con
qt non lo imposta mai, nemmeno dopo un replot o un `title`. La funzione era stata misurata su una
macchina headless, dove gnuplot viene scelto e il flag funziona, ed era rotta ovunque altro da
quando è nata. Il gemello Python era lo specchio: `fig.stale` non veniva mai *azzerato*, quindi
ogni figura veniva ri-renderizzata a ogni prompt.

**E un terzo, trovato accendendo un controllo che non era mai stato acceso.** La gamba matplotlib
di `drive_python` chiedeva a `sys.executable` se matplotlib fosse importabile — l'interprete che
esegue il driver, non il `python3` che la sessione avvia. Su questa macchina sono due python
diversi. Chiesto a quello giusto, il driver è fallito tre controlli prima di arrivare alle figure:
il Python di Homebrew su macOS è un framework build e il processo si chiama `Python` con la
maiuscola, il confronto era case-sensitive, e mandare una cella a una sessione viva scriveva il
comando di **shell** `python3 file.py` al prompt di Python. Lo stesso bug della 0.9.1 con un altro
cappello — lì era `octave-cli-11.3.0`, qui è una lettera maiuscola.

*La forma ricorrente,* ormai la terza volta in tre release: un controllo che guarda troppo largo,
o nel riquadro sbagliato, trasforma un bug in un PASS. E ce n'è una peggiore, comparsa qui: un
controllo che verifica un'**assenza** guardando nel posto sbagliato è un PASS gratis che non può
fallire mai. Il rimedio non è la diffidenza generica ma `Session.frame_of`, che cammina fino ai
bordi del riquadro in cui il testo è davvero disegnato, più l'abitudine di stampare nella nota
*cosa* è stato letto.

*Due controlli chiedevano il comportamento sbagliato,* e uno era il caso raro che vale di più:
`drive_presets` pretendeva il prompt di fianco all'editor su finestra larga, cosa deliberatamente
tolta nella 0.9.2 — e falliva da allora senza che nessuno lo leggesse. Il suo compagno a 92
colonne passava contro un layout identico a quello largo, quindi non dimostrava nessuna
adattabilità: sembrava la prova di un comportamento che era stato rimosso.

---

## 0.10.1 e 0.10.2 — quello che è uscito subito dopo (2026-08-22)

Due patch nate dallo stesso posto delle quattro della 0.9.1: una sessione vera su una Ubuntu
remota. `clear` in un pane, le figure che perdevano le etichette, una sessione `ssh -X` che
moriva; e poi `grid minor` che via ssh non stampava niente — sembrava Linux ed era il toolkit
grafico, perché senza display si sceglie gnuplot e gnuplot non sa mettere tacche minori fra
tacche che gli arrivano una per una.

## 0.11 — git per intero, il grafo, e il server che risponde a più di una domanda (2026-08-23)

### Il pannello Git fa tutto quello che un pannello Git può fare

**Cinque schede invece di quattro,** e Cronologia non è più una lista: è un **grafo di tutti i
branch insieme**, disegnato in ASCII. `src/git_graph.rs` è un'assegnazione di lane in una
passata sola, funzione pura della lista di commit, senza accesso al repository e senza disegno
dentro — quindi le forme scomode (un merge octopus, due storie senza antenato comune, un grafo
tagliato al limite con genitori che non arrivano mai) sono casi in un file di test invece che
repository che qualcuno deve costruire.

*Perché ASCII.* I sei caratteri di `git log --graph` — `*`, `|`, `/`, `\`, `-` — ce li ha ogni
terminale e li spazia allo stesso modo. Box-drawing e braille fanno un grafo più bello dove il
font ce l'ha, e via ssh verso una console qualsiasi diventano quadratini o mezze colonne di
disallineamento. **Un grafo che sbaglia a dire quale linea si unisce a quale è peggio di nessun
grafo.**

*Una scelta contro `git log --graph`:* **le lane non si compattano mai a sinistra.** git compatta
e ottiene un disegno più stretto al prezzo di una diagonale ogni volta che una lane si chiude più
a sinistra. In una finestra che tiene anche un editor, le linee che restano nella loro colonna
sono quello che rende la forma leggibile a colpo d'occhio, e le diagonali che restano sono le due
che significano qualcosa: un branch che parte e un branch che rientra.

*Due attese sbagliate, trovate dai test.* Avevo trascritto l'output di `git log --graph` senza
pensarci per l'octopus (`|\ \`) e per tre branch che rientrano insieme (`|/|/|/`). Il layout
produce `|\-\` e `|/-/-/`, e ha ragione lui: il tratto orizzontale dice *da dove viene* la linea,
e le lane attraversate non proseguono, quindi non devono disegnare una barra. La regola che conta
— una lane viva attraversata **tiene la sua `|`**, solo i vuoti prendono `-` — ha un test suo,
perché senza un merge da una parte all'altra del grafo si disegnerebbe dritto attraverso due
branch estranei e si leggerebbe come un'unione a tre.

**Quello che il pannello scrive adesso:** stage, unstage, tutto, commit, **amend**, discard,
**stash** (crea, applica, pop, elimina), **branch** (crea da HEAD o da un commit del grafo,
elimina, checkout, **merge**), **tag**, **cherry-pick**, **revert**, **reset --hard**, e
**l'uscita da un merge / pick / revert / rebase fermo a metà** — offerta solo quando ce n'è uno da
cui uscire, letta dal filesystem (`rebase-merge`, `MERGE_HEAD`, …) e non dalla prosa di
`git status`, che cambia fra versioni.

**Fetch, pull e push ci sono, e girano nel terminale.** È la risposta alla cosa che li ha tenuti
fuori per tre release, non un cambio di idea: possono fermarsi a chiedere una password, un codice
a due fattori o una host key, e un pannello modale non ha dove mettere quella domanda. Un
terminale è esattamente la cosa che può farla, e CleeCode ne ha di veri a un tasto di distanza.
Quindi il pannello si chiude, la shell prende il fuoco, e il comando appare a un prompt.

*Le grafie restano quelle vecchie,* per la ragione già scritta nella 0.10: `stash save` e non
`stash push`, che è del 2017 — questo è il comando che serve di più sul git vecchio di un server
raggiunto via ssh, ed è l'unico posto dove una grafia deprecata vale più di una corrente.

*Una domanda in rosso e una in giallo, e la differenza è tutta la sicurezza qui.* Rosso solo dove
il sì distrugge qualcosa che non è in nessun commit, stash o reflog: scartare un file, `reset
--hard`, eliminare uno stash. Eliminare un branch **non** è in quella lista — i suoi commit
restano nel reflog novanta giorni — ed è la distinzione, non una dimenticanza: **rosso su tutto è
rosso su niente.**

### Dove si trova, che era metà del problema

Il pannello esisteva solo dietro `Ctrl+Shift+D` e una riga sepolta nel menu Modifica. Adesso:

- un **menu Git** nella barra, che apre direttamente la scheda che vuoi, più i tre remoti;
- il **tasto destro su un file versionato** dell'albero ha le azioni git per quel file, sotto due
  intestazioni di sezione — *Git — questo file* e *Git — il repository* — perché una riga sola non
  direbbe che «Metti in stage questo file» e «Rinomina...» sono due tipi di cosa diversi;
- lo scarto dal menu contestuale **non riscrive la domanda**: apre il pannello sul file con la
  domanda già su. Le sue regole sono la cosa scritta con più cura del pannello e le uniche davanti
  a un'azione che niente annulla; una seconda copia sarebbe una seconda copia da tenere giusta, e
  quella che sbaglierebbe è quella che nessuno guarda.

### Tre bug veri, e uno era invisibile da sempre

- **I pallini git nell'albero non comparivano mai** aprendo il progetto come `.` — cioè nel modo
  normale. `git_status` archiviava per percorso assoluto (`toplevel/rel`) e l'albero cerca con la
  grafia della root che gli è stata data, che lanciando `clee` in una cartella è `./main.rs`. Ogni
  lookup mancava, nessuna riga prendeva il pallino, e **niente da nessuna parte lo diceva**: un
  editor senza marcatori git è identico a un repository senza modifiche. È la stessa forma del bug
  `file:///./src/main.rs` dell'LSP nella 0.8 — un percorso giusto, in una grafia che l'altro capo
  non usa.
- **Incollare in una casella modale scriveva nel file dietro.** `handle_paste` conosceva quattro
  riquadri su venti e per tutti gli altri cadeva sull'editor: un messaggio di commit incollato
  finiva nel sorgente aperto sotto il pannello, in silenzio, con la casella lì che continuava a
  chiederlo. Adesso c'è **una funzione sola** che dice se un riquadro possiede la tastiera, ed è il
  *cancello* davanti alla catena invece che la catena stessa — quindi un riquadro aggiunto a una e
  non all'altra o non prende nessun tasto o li mangia tutti, sbagliato **la prima volta che lo
  apri** invece che solo per un incolla.
- **`git show --stat` non stampa il patch.** L'ha trovato il driver: il lettore di un commit
  mostrava i nomi dei file e sotto niente. `--stat` *sostituisce* il patch invece di aggiungersi,
  e nessuno lo dice — la finestra si apre, ha del contenuto, e il diff semplicemente non c'è.
- E una fragilità: gli snapshot in volo non avevano un ordine. Adesso ognuno porta il numero
  dell'interrogazione, e tutto quello che non è l'ultima viene buttato.

### LSP: più server, definizione, hover

**La tabella dei server** ha dodici voci (rust-analyzer, pyright, tsserver, gopls, clangd,
lua-language-server, zls, solargraph, bash-language-server, json, taplo, texlab) e — quello che
conta di più — **una tabella dell'utente in `settings.toml`** che vince su quella predefinita:
`[language_servers]` con `estensione = "riga di comando"`. Una release non deve essere il modo di
raggiungere un language server nuovo, o il fork che uno si tiene in `~/bin`. Una voce messa a `""`
ne spegne una predefinita.

**Un processo per programma**, non per linguaggio: clangd serve sette estensioni e uno per
estensione sarebbero sette clangd a indicizzare lo stesso progetto. Avviato alla prima apertura di
un file che serve, e a ognuno vengono annunciati solo i suoi file — un `.py` detto a rust-analyzer
è una pagina di errori su un file che lui legge come Rust. Un server che non parte è ricordato
**per programma**: una macchina con gopls e senza clangd continua ad avere Go.

**`Ctrl+Shift+J` va alla definizione, `Ctrl+Shift+L` torna indietro** — una pila, perché seguire un
nome dentro un nome dentro un nome è il modo normale di leggere codice che non conosci, e uno slot
solo ti lascerebbe a due file da dove sei partito. La risposta si legge in tutte e tre le forme che
un server può mandare (`Location`, un array, `LocationLink`): sono tutte corrette e sceglierne una
significherebbe perdere silenziosamente le altre.

**L'hover non ha un tasto, ed è una scelta.** Un hover è la risposta a una domanda che non hai
proprio fatto — cos'è questo, cosa torna. Quello che va chiesto lo chiede chi già lo sa. Quindi
arriva da solo quando il cursore si ferma su una parola, nell'unica riga che il server ha già:
quella di stato, a destra. **Il diagnostico vince quello spazio quando c'è**, e non è vicina: un
errore su questa riga è una notizia, un tipo no — e finché c'è qualcosa che non va, il tipo è molto
probabilmente il motivo.

*Il reader del client non indovina più.* Teneva un insieme di id di completamento, con un commento
che diceva che «la prima risposta è l'handshake e tutto il resto è un completamento» sarebbe stato
vero quel giorno e silenziosamente falso il giorno di un terzo tipo di richiesta. Quel giorno è
arrivato due volte insieme: adesso ogni id porta scritto **cosa stava chiedendo**.

*Un bug che solo il driver poteva trovare, di nuovo:* l'hover chiedeva di un file che il server non
aveva ancora ricevuto. Rispondeva — su un documento che non ha mai visto, cioè su niente — e la
risposta veniva ricordata come quella di quel file. Era intermittente e restava nascosta dietro un
predicato lento del driver: la prima versione del controllo falliva e nel fallire *dava tempo* alla
sincronizzazione. Sistemato il predicato, il bug è comparso.

### La griglia di gnuplot, finalmente uguale a qt

La 0.10.2 disegnava la griglia **minore** che gnuplot rifiuta, e lasciava la **maggiore** al
toolkit «perché almeno c'è». Messe le due stampe della stessa figura una accanto all'altra, era
palesemente la scelta sbagliata: qt disegna la maggiore come `gridcolor` a `gridalpha`, cioè il 15%
di un quasi-nero su bianco — un grigio *attraverso* il quale si legge il grafico. gnuplot la
disegna nera piena, ed è la cosa più forte della figura: **il dato è una curva sottile dietro una
gabbia nera.** Correggerne una sola era lo stato peggiore dei tre — due grigi e due pesi in una
figura sola, che si legge come uno sbaglio anche a chi la versione qt non l'ha mai vista.

Adesso sono entrambe nostre, misurate: i grigi dell'immagine stampata sono **222 e 199**, gli
stessi identici numeri di qt.

*E la cosa che si vede solo guardandola davvero,* segnalata dall'utente e confermata al pixel: **le
linee maggiori si fermano una tacca prima del bordo.** qt disegna le sue per tutta l'altezza e poi
ci passa sopra le tacche, perché lì la decorazione degli assi sta davanti. In gnuplot sta dietro:
disegna bordo e tacche per primi e i dati — che è quello che le nostre linee sono, per lui — sopra.
Una linea che arriva al bordo **copre la tacca su cui dovrebbe poggiare**, e il risultato è una
griglia che fluttua staccata da un asse senza tacche. Che è esattamente quello che sembrava, ed
esattamente la prima cosa che si nota confrontando le due figure.

*Il lavoro sulla griglia è in due file suoi* (`cleecode_grid.m`, `cleecode_grid_undo.m`) invece che
sottofunzioni di `cleecode_figs`, perché così si può chiamare, stampare e guardare da sola — che è
come il difetto della griglia maggiore è stato trovato. E il controllo che il codice interprete
viaggi dentro al binario **non ha più una lista scritta a mano**: legge le chiamate dal codice. Una
lista a mano fallisce solo per una funzione che qualcuno si è ricordato di aggiungerci, e il caso
da cui protegge è quello che nessuno si è ricordato — cioè esattamente questi due file.

### Run entra nella sessione, anche per Python

Trovato dall'utente in tre segnalazioni di fila che erano lo stesso bug: nessun plot, nessuna
variabile, pannello vuoto. Run per Python avviava sempre una shell nuova, quindi lo script girava
in un processo che finiva subito — le variabili sparivano prima che il pannello potesse vederle e
le figure le disegnava qualcosa che non esisteva più. Octave consegna il file alla sessione viva
dalla 0.9; adesso lo fa anche Python, con `exec(open(...).read())`, che gira nel namespace del
prompt. Poi puoi continuare a scrivere comandi che usano quello che il file ha lasciato — che è
il modo in cui `clee -w pylab` somiglia al foglio che sembra.

*La motivazione contraria era scritta e diceva:* un REPL Python aperto di lato mentre editi una
web application non è dove deve girare `manage.py`. Descrive una sessione vera e manca quella per
cui CleeCode spedisce un preset. E la scelta adesso non è più nostra: **la tendina del target di
Run ha una riga in cima, "la sessione già aperta"**, e scegliere un venv significa scegliere di
avviare un interprete — la stessa domanda detta dall'altro capo. La spunta segue quello che Run
farebbe *davvero*: preferenza attiva ma nessun prompt aperto, e la spunta sta sul venv.

*E prima ancora, il bug che si vedeva:* con un prompt Python aperto, Run scriveva `python3 file.py`
lì dentro, cioè un `NameError` nella trascrizione dell'utente con il suo nome sopra. La regola che
lo produceva diceva "se nessuna shell è libera usa quella in cui eri" — e quella in cui eri è
esattamente l'interprete in cui stavi lavorando. Terza incarnazione dello stesso errore, dopo
`octave-cli-11.3.0` nella 0.9.1 e una `P` maiuscola nell'audit della 0.10. Adesso, se non c'è una
shell libera, se ne apre una: **Run deve eseguire**, e un comando scritto dove non può girare è
peggio di un pannello che nessuno ha chiesto.

### I controlli delle figure c'erano e non si vedevano

Segnalati come mancanti — pan, rotazione 3-D — ed erano lì dalla 0.9: sulla scheda di una figura
le frecce spostano un grafico piatto e **girano** un asse tridimensionale, `+`/`−` avvicinano, `r`
rimette la vista di partenza, `e` esporta. Il comando va alla sessione, che ridisegna, e per
questo le etichette degli assi restano vere.

Quello che mancava era che la barra lo dicesse. Offriva zoom, fit e invert — i controlli
dell'*immagine* — e di pan e rotazione non c'era traccia, quindi l'unico modo di scoprirli era
premere una freccia a caso. Adesso la barra lo scrive, e scrive quale dei due sta facendo, perché
sono gli stessi quattro tasti su due cose diverse. Con `examples/plot3d.m` e `examples/plot3d.py`
per provarla, e `examples/plot.py` per il resto del lato pylab.

**Quello che ancora non c'è, ed è il prossimo passo:** il **mouse**. Trascinare per spostare, e
trascinare un rettangolo per zoomare dentro quello. Il pezzo che manca non è il gesto ma la
conversione: da pixel della scheda a coordinate dei dati serve sapere dove sta il riquadro degli
assi *dentro* l'immagine, e quello lo sa solo la sessione — quindi lo snapshot deve portarselo
dietro, come già porta `xlim`, `ylim` e `view`.

**Dopo:** il mouse sulle figure, un picker quando le definizioni sono più di una, e il resto di
quello che un language server sa dire (rename, format, i simboli del documento).

---

## 0.11.1 — perché si aprivano due grafici (2026-08-23)

Segnalato subito dopo la 0.11.0: su un Mac, con una figura come scheda, si apriva **anche** una
finestra Qt. La causa è una riga sola, e non era dove la cercavo.

**`figure(n)` su una figura che esiste già la *alza*** — e alzare, in Octave, vuol dire rimettere
`visible` a `"on"`. `defaultfigurevisible`, che la sessione tiene a `off` proprio perché nessuna
finestra si apra mai, decide solo come una figura *nasce*. Quindi un `figure(n)` la disfa per
sempre. E ogni comando di navigazione che CleeCode mandava cominciava così: `figure(2); view(60,
30);`. Premere una freccia su una scheda faceva comparire una finestra vera e ce la lasciava.

Misurato in Octave 11.3.0: `set(0, 'currentfigure', n)` seleziona **senza** alzare, e `xlim()`
subito dopo lavora sulla figura giusta. È quello che CleeCode manda adesso, e l'export non
seleziona affatto — l'handle di una figura numerata *è* il suo numero, quindi `print(n, ...)`
basta.

*E il rimedio dall'altro capo,* perché la lista dei modi in cui una figura può tornare visibile
non la finisce nessuno: il tick la rimette a `off` a ogni giro. Lo script dell'utente che disegna
due volte nella stessa figura la alza esattamente allo stesso modo, ed è codice suo, legittimo.

**Il controllo che lo tiene chiuso chiede alla sessione, non allo schermo:** una finestra Qt si
apre fuori dal disegno di CleeCode, quindi dal terminale non se ne può leggere niente. Il driver
scrive `printf('VISIBLE=%s\n', get(1,'visible'))` al prompt dopo aver premuto tutti i tasti che
muovono il grafico. Dimostrato non vacuo rimettendo il bug: fallisce.

### La scheda si era fermata al primo fotogramma

Trovato tirando lo stesso filo, ed è peggio del primo. `fingerprint` decideva se ristampare
guardando **geometria**: dimensioni, limiti, vista, posizione, numero di figli, titolo. Un
`set(h, "ydata", ...)` — cioè qualunque animazione, e qualunque aggiornamento in place — non
tocca niente di tutto ciò con gli assi fissati da `axis(...)`. La fingerprint veniva identica, la
figura non veniva mai ristampata, e la scheda mostrava il primo fotogramma **per il resto della
sessione**. Senza dire niente: era una foto vera di una figura vera, solo non più di quella.

Adesso la fingerprint campiona anche i dati — al massimo trentadue punti per array, così una
superficie da un milione di numeri costa come una linea da cento.

### E le animazioni si possono guardare

Il pannello si aggiorna quando l'interprete **aspetta un comando**: misurato, `add_input_event_hook`
non scatta nemmeno una volta durante un comando che dura due secondi. Giusto per un pannello di
variabili, e vuol dire che un ciclo è invisibile mentre gira.

Quindi un ciclo che vuole essere guardato lo dice: `cleecode_frame()` in Octave,
`_cleecode_pyws.frame()` in Python. Fuori da CleeCode non fanno niente, così lo script resta
eseguibile dovunque. Costo di un fotogramma, misurato qui:

| | ms per fotogramma | |
|---|---|---|
| Octave, toolkit qt | 28 (linea), 35 (superficie 60×60) | ~30 al secondo |
| Octave, gnuplot (headless, via ssh) | 148 | ~7 al secondo |
| matplotlib | 12 | più di quanto un terminale disegni |

Con `examples/anima.m` e `examples/anima.py` per provarlo.

### E un easter egg

Il primo menu della barra prende il nome della sessione: **Ottavio** con il workspace Octave,
**Pitone** con pylab. È uno scherzo ed è anche il segno più chiaro di quale dei due preset sei
dentro — cosa che altrimenti si legge nel grigetto in fondo alla barra. Una funzione sola decide
il nome, perché a chiederlo sono in tre — il disegno, il click e il mnemonico — e "Ottavio" è una
colonna più corto di "CleeCode": una larghezza diversa fra disegno e hit-test manderebbe ogni
click sul menu sbagliato, e solo a chi usa un preset.

## 0.11.2 — chi ha chiamato octave-gui (2026-08-23)

L'utente: «ho visto crashare octave-gui — lo hai chiamato tu?». Sì, indirettamente, e questa è
quasi certamente la causa vera dei due grafici, più ancora del `figure(n)` della 0.11.1.

Il **preset** avvia `octave --no-gui --persist`. Il comando di default del **pulsante Run** era
`octave --persist {file}`, senza `--no-gui`, e i due non sono andati d'accordo per tutto il tempo
in cui sono esistiti entrambi. Su una macchina con uno schermo, `octave` liscio avvia l'Octave
**grafico**: un IDE intero in una finestra sua, col suo editor e le sue finestre di figure. Quindi
premere Run su un `.m` senza una sessione aperta apriva un secondo IDE accanto a questo, il
grafico finiva nella sua finestra invece che nella scheda, e su questo Mac `octave-gui` è caduto.

Adesso il default dice `--no-gui`, e — la metà che conta — **la stringa vecchia viene riscritta**
in `settings.toml`: chi ha eseguito un `.m` anche una volta ce l'ha salvata, e correggere solo il
default avrebbe corretto solo le installazioni nuove. Riconosciuta alla lettera e solo fra le
stringhe che CleeCode ha scritto: un comando modificato dall'utente è suo, qualunque cosa dica.

## 0.12 — le animazioni che non sfarfallano, e una figura che resta la stessa (2026-08-23)

L'utente: «le animazioni vanno ma flickerano di brutto». Erano quattro cose diverse, tre nostre.

**La scheda si svuotava fra un fotogramma e l'altro.** Rileggere una figura rimetteva la
scheda in stato di caricamento — una scritta in mezzo a un riquadro vuoto — mentre il PNG
veniva decodificato su un thread. Dieci volte al secondo: immagine, buco, immagine. Quello *è*
il flicker. `rerender_preview` teneva su quello che c'era dai tempi dello zoom; la strada da
cui passa davvero una figura no. Adesso sì, e una decodifica già in volo fa aspettare il
fotogramma dopo invece di far partire un secondo thread sullo stesso file. Il fotogramma
saltato non si perde: il timestamp viene registrato solo quando una lettura è cominciata.

**Ogni fotogramma era un'immagine nuova per il terminale.** `new_resize_protocol` sceglie un id
kitty a caso, e l'id è scritto *dentro le celle* come colore: un protocollo nuovo per fotogramma
cambiava ogni cella del riquadro, diceva al terminale di dimenticare un'immagine e di piazzarne
un'altra sull'intera area, e quello che si vedeva in mezzo era il riquadro. Ora la figura nuova
va al protocollo che c'è già, sotto l'id che il terminale conosce: le celle restano identiche e
lui si limita a ridipingere. Smette anche di perdere un id kitty per fotogramma per tutta la
durata dell'animazione. Un test disegna due fotogrammi e rilegge l'id dalle celle, che è dove lo
legge anche il terminale; invertendo la fix, fallisce.

**Niente teneva fermo lo schermo mentre il fotogramma veniva scritto.** Adesso ogni frame esce
dentro un synchronized update (DEC 2026): Ghostty, kitty, WezTerm, iTerm2, foot e tmux tengono
su l'ultimo fotogramma completo finché questo non è arrivato tutto, e un terminale che non
conosce quella modalità la ignora. Il ritorno di quadro che una TUI non ha mai avuto. Verificato
sul flusso grezzo del pty: 27 aperture, 27 chiusure, bilanciate.

**E l'immagine veniva letta mentre veniva scritta.** Tutti e due gli hook stampavano dritti sul
nome che l'editor sorveglia, quindi un fotogramma preso a metà scrittura si decodificava come
`unexpected end of file` e la scheda diceva di non riuscire a leggere un file che un millisecondo
dopo è perfetto — beccato tre volte in cinque secondi dal driver. Ora scrivono di fianco e
rinominano, che dentro una cartella è atomico. `rename` e non `movefile`: movefile lancia `mv`,
e un fork per fotogramma è un terzo del budget di un'animazione. Come cintura oltre alle
bretelle, una decodifica fallita non porta più via l'immagine che è già su.

**«Su octave mi sa che manco partono, scatta un frame ogni tanto»** era una quinta cosa, e
diversa. La scheda veniva riletta solo quando cambiava lo *snapshot*, e lo snapshot lo scrive un
hook che gira mentre l'interprete aspetta un comando — un ciclo non aspetta mai. `cleecode_frame`
ristampa le figure, e non deve ricostruire uno snapshot sessanta volte al secondo, quindi
un'animazione Octave si muoveva solo quando il ciclo lasciava entrare l'hook per caso. Il
timestamp della figura è ciò che dice che è stata ridisegnata, e costa uno `stat` per figura
aperta per tick. Il `frame()` di Python lo snapshot lo scrive: ecco perché pylab animava e Octave
no.

**Le figure duplicate.** Octave e matplotlib scrivevano `fig1.png` nella stessa cartella —
entrambi numerano da uno — quindi due sessioni si sovrascrivevano il grafico e le frecce sulla
scheda chiedevano il ridisegno all'interprete sbagliato. Una cartella per linguaggio. Non per
pannello, che era l'altro modo: il percorso *è* la scheda, e una cartella per pannello aprirebbe
una seconda scheda per la figura 1 a ogni sessione riavviata.

E rilanciare uno script accumulava schede — due grafici diventavano quattro al secondo giro e sei
al terzo — perché tutte e due le lingue danno il primo numero libero, quindi `plt.subplots()` e un
`figure()` nudo creano figure nuove ogni volta. Ora ▶ Run chiude le figure che *quel file* aveva
aperto l'ultima volta, e solo quelle: un grafico fatto a mano al prompt non appartiene a nessun
run e sopravvive. Quali fossero si guarda dal momento in cui il comando viene battuto fino al
ritorno del prompt, letto dalla disciplina di linea del pty invece che indovinato con un timeout.

**E i grafici devono funzionare senza i preset**, che servono a preparare un layout e una
sessione viva, non a cambiare come si vedono i plot. Octave lo faceva già, col PKG_ADD sul suo
load path. Python no: `PYTHONSTARTUP` lo legge solo un interprete interattivo, quindi
`python3 plot.py` — cioè quello che fa Run quando non c'è un prompt aperto — non installava
niente, mentre `MPLBACKEND` puntava già al backend senza finestre. La figura veniva disegnata per
nessuno e `plt.show()` non apriva niente: il grafico non esisteva da nessuna parte. Lo risolve un
`sitecustomize` sulla cartella della libreria, e la consegna è registrata **dentro il modulo del
backend**, non lì: gli handler `atexit` girano in ordine inverso e pyplot registra
`Gcf.destroy_all` mentre viene importato, quindi un hook messo prima che matplotlib fosse mai
usato trova tutte le figure già distrutte. Misurato: `get_fignums()` lì risponde `[]` ogni volta.
Il backend viene importato quando matplotlib lo sceglie, che è per forza dopo. Un millisecondo
sull'avvio di un Python che non disegna.

**Il primo menu ha ripreso il suo nome** — via l'easter egg Ottavio/Pitone, su richiesta — e con
lui sparisce il parametro `workspace` dalle tre cose che chiedevano il titolo: il disegno, il
click e il mnemonico, che dovevano concordare su una larghezza. **I tre workspace predefiniti**
— layout di default, octave e pylab — non si potevano cancellare (l'elenco di cancellazione sono
i file, e loro non sono file) ma niente lo diceva: nella lista sembravano roba tua da buttare.
Ora sono del colore che il selettore usa già per le parti che appartengono all'app.

**Il rumore che si può zittire e quello che no.** La riga `qt.qpa.fonts: Populating font family
aliases…` è logging Qt e se ne va con una regola sola, una categoria, solo i warning. La riga
`FALLBACK (log once): Fallback to SW vertex processing` è il driver OpenGL di Apple e non
risponde a nessuna regola: misurato, la stampa il `print` — quello che chiede CleeCode, non il
`plot` dell'utente — una volta per sessione, e stampare col renderer vettoriale la evita al
prezzo di 190 ms a fotogramma contro 37.

**Un esempio si chiamava `plot.m`.** Octave cerca le funzioni fra i file della cartella in cui
lavora *prima* che nella sua libreria, quindi un `plot.m` lì dentro diventa "il" plot per ogni
script lanciato da quella cartella — e quello aveva pure una parentesi di troppo. Risultato:
`anima.m` falliva con l'errore di sintassi di un file che non nomina. Ora si chiama `grafico.m`,
e `anima.m` gira fino in fondo anche fuori da CleeCode, dove `cleecode_frame` non è inerte: non
esiste proprio.

Nei driver quattro controlli nuovi, tutti falliti prima della loro correzione: un'animazione
campionata venticinque volte al secondo non sbianca mai e non mostra mai la scritta di
caricamento; tre giri di uno script con figure generiche lasciano la sessione con le stesse
quattro figure; le due che non ha aperto sono ancora lì; e uno script lanciato in una shell
qualsiasi, senza nessuna sessione, consegna il suo grafico. Uno di loro passava a vuoto appena
scritto — lo script che doveva lanciare non partiva — e adesso lo dice.


## 0.12.1 — le impostazioni che non si vedevano (2026-08-23)

Partita da una domanda: come si disattiva la cattura dei plot. La risposta era in due posti, e
uno dei due non esisteva sullo schermo.

**Tre impostazioni disegnate fuori dal riquadro.** `SETTINGS_COUNT` valeva 9 dal primo commit —
la modale si dimensiona su quel numero e il cursore ci fa il modulo — mentre `rows()` nel
frattempo era arrivata a dodici. *Dove si aprono i grafici*, *Mouse abilitato* e *Lingua* erano
sotto il bordo: invisibili, irraggiungibili con le frecce, e nemmeno cliccabili, perché anche il
click confronta l'indice con la stessa costante. Ecco perché la destinazione dei plot si poteva
cambiare solo dal menu Esegui o dal file su disco, e la lingua solo dal file. Il riquadro adesso
si dimensiona sulle righe che ha davvero, e un test fallisce se i due numeri divergono di nuovo.
Un secondo test prova che ogni riga cambia il *suo* valore e nessun altro: `activate` è un match
sull'indice, quindi una riga inserita in mezzo senza rinumerare quelle sotto ribalterebbe la
vicina in silenzio — che è esattamente come si aggiungeva la riga dello splash.

**Il valore contro il bordo destro.** A colonna fissa, `Language server (diagnostici,
completamento)` — 44 caratteri contro 34 — si portava via il posto del suo `on`, che finiva
attaccato alla parentesi. Ora l'etichetta spinge il valore invece di essere invasa da lui, e la
modale è larga quanto la riga più lunga che deve disegnare: in italiano *schede — qui non c'è un
display* è una frase, non una parola.

**Una modifica fatta lì dentro adesso ha effetto, e resta.** Scegliere una riga scriveva la
struct e basta. La destinazione dei plot vive però anche in un atomico, che è da dove partono le
shell — una shell nasce fuori dal thread dell'app e non può leggerla — quindi la riga che nessuno
poteva raggiungere non avrebbe comunque funzionato. E niente veniva scritto su disco fino a
un'uscita pulita: modifica nella modale più terminale chiuso con la X, e le due si annullavano. Il
toggle del menu faceva entrambe le cose da sempre; sono le stesse impostazioni.

**L'interruttore dice da che parte sta.** *Grafici: schede o finestre* poneva la domanda e non
rispondeva: per sapere com'era messo bisognava ribaltarlo e leggere la barra di stato, cioè
rispondere per poter chiedere. Adesso scrive `schede` o `finestre` a destra, nella colonna delle
scorciatoie — nessuna voce ha entrambe, e un test lo tiene vero — e scrive la destinazione
*effettiva*: su una macchina senza schermo dice `schede`, perché è lì che i grafici vanno
qualunque cosa dica l'impostazione.

**Lo splash è un'impostazione.** Un argomento sulla riga di comando lo salta da sempre, quindi
`clee src/main.rs` va dritto al lavoro e `clee` nudo mostra la tartaruga per un secondo e otto
decimi. *Schermata iniziale all'avvio*, spenta, lo salta anche lì.

Verificato pilotando il binario in uno pseudo-terminale, non solo con `cargo test`: la voce di
menu che legge `tabs`, le tredici righe tutte disegnate, il file su disco che cambia appena si
preme Invio, e lo splash che compare o no secondo l'impostazione. Il primo tentativo di quella
verifica passava a vuoto per un motivo che vale la pena scrivere: il fixture appendeva
`show_splash = false` in fondo a un `settings.toml` che finisce con `[language_servers]`, quindi
la chiave finiva dentro la tabella, `toml::from_str` falliva e `Settings::load` tornava ai
default in silenzio. Una preferenza ignorata e un file che non si legge hanno lo stesso aspetto.


## 0.12.2 — l'immagine che rompeva l'anteprima (2026-08-23)

Segnalato come "alcuni md li vedo, il README di clee solo in modalità testo": la differenza fra
i due era una figura.

**typst e i percorsi assoluti.** pandoc estrae le immagini di un documento in una directory
temporanea e passa al motore percorsi assoluti — e typst legge un percorso assoluto come relativo
alla sua *root*, non al filesystem. Cercava quindi `/private/var/…/media/docs/demo.gif` sotto la
directory di lavoro e diceva che non c'era. Adesso il motore riceve `--root`, ed è la root su cui
sta la temporanea, perché è lì che puntano quei percorsi. Riguardava ogni markdown con
un'immagine dentro; il ripiego a testo stilato è silenzioso per scelta, ed è per questo che si
leggeva come "l'anteprima grafica non va più". Il test lo prova end-to-end dove pandoc e un motore
ci sono: invertita la correzione, fallisce con l'errore vero.

**E quando fallisce, la barra di stato dice perché.** Mostrava l'ultima riga di pandoc — "Error
producing PDF." — vera e inutile. Il motore la spiega sopra: typst apre con `error:`, TeX con
`!`, e adesso è quella a comparire.

**Apri fuori da CleeCode.** Prima voce del menu contestuale dell'albero, e nella palette: passa il
file al programma che il sistema associa a quel tipo — un PDF al lettore, un `.md` al browser,
tutto quello che CleeCode può solo mostrare. `open` su macOS, `xdg-open` su Linux, `start` su
Windows, dove il primo argomento fra virgolette è il *titolo della finestra* e non il percorso —
il bug classico di quella funzione da tre righe, qui evitato e testato. Via ssh rifiuta e lo dice:
l'apertura avverrebbe sulla macchina in fondo alla connessione, su un desktop dove non è seduto
nessuno.


## 0.12.3 — perché le animazioni Octave erano lente (2026-08-24)

Segnalato da una sessione via ssh: "octave a fare animazioni è più lento rispetto a python".

**Non era ssh e non era il toolkit.** Misurato qui, stessa figura (linea di 500 punti, 800×600 a
96 dpi): `print` sotto gnuplot 155 ms a fotogramma, sotto qt 161 ms, `savefig` di matplotlib
11 ms. Il PNG di Octave è pure il più piccolo — 7 kB contro 28 — quindi il trasferimento kitty
sulla connessione lo favorisce, non lo penalizza. E il costo non è nei pixel: 150 ms a 400×300 e
159 ms a metà risoluzione. È la macchina di `print`, che copia la figura in una nascosta e rifà
tutto il giro del toolkit a ogni fotogramma.

**La scorciatoia.** Sotto gnuplot `drawnow (TERM, FILE)` passa la figura al terminale png del
toolkit senza niente in mezzo. Attraverso l'hook vero, con la griglia e tutto il resto al loro
posto:

| | prima | ora |
|---|---|---|
| linea animata 800×600 | 243 ms | 55 ms |
| superficie 3-D con colorbar 640×480 | 345 ms | 74 ms |
| immagine 500×400 | 233 ms | 47 ms |

La figura resta la stessa: la griglia la disegna CleeCode come oggetti linea veri (vedi
cleecode_grid), quindi sopravvive al cambio di strada. Confrontate le due immagini una accanto
all'altra — linea, superficie, colorbar — non si distinguono.

**Due controlli, perché la strada è stretta.** Sotto qt l'argomento del terminale viene ignorato e
quello che finisce su disco è PostScript con estensione `.png` — verificato sui magic bytes
(`%!PS-Ado`), ed è per questo che il file viene riletto invece che dato per buono. E la
dimensione: `print` riceve la misura dalla paperposition, `drawnow` no, quindi una figura più
piccola del pavimento che il chiamante applica uscirebbe con un numero di pixel diverso da quello
che il pannello dichiara — e ogni coordinata del mouse mappata sopra sarebbe sbagliata in
silenzio. Si leggono i primi 24 byte: firma, larghezza e altezza dall'IHDR. Se uno dei due
controlli non torna, si stampa come prima.

Nota di misura, per onestà: fuori dall'hook `drawnow` rispettava anche `linewidth`, che `print`
attraverso il percorso eps ignora. Dentro l'hook — con paperposition impostata — quel guadagno
non si vede: le due immagini hanno la stessa linea sottile. Resta la velocità, che era il punto.


## 0.12.4 — l'interruttore che si sentiva solo dopo un riavvio (2026-08-24)

Tre cose trovate usando l'editor, non leggendolo.

**"Lo switch di preferenza non viene cagato se non riavvio clee".** Mezzo vero, ed è la metà
interessante. Misurato pilotando il binario: una shell aperta *dopo* il toggle riceve già
`CLEECODE_PLOTS=windows` senza riavviare niente. Quella che era già aperta no, e non può: l'ambiente
di un processo è una copia fatta quando è partito, e da fuori non si cambia. Quindi l'`octave` che
digiti dopo, al prompt in cui stavi lavorando da un'ora, continua a fare quello che diceva
l'impostazione vecchia — e l'unico gesto che rimetteva tutto d'accordo era far ripartire l'editor.

Adesso la risposta sta anche in un file, che i due hook leggono *quando parte l'interprete*:
`cleecode_boot` prima di scegliere il toolkit, `sync_plots` in `sitecustomize` prima che qualcuno
possa importare matplotlib — che il backend lo sceglie all'import e poi non riguarda più la
variabile. La variabile resta come ripiego. Verificato nei due sensi su entrambi i linguaggi: con
la variabile che dice `tabs` e il file `windows` Octave svuota `CLEECODE_OCTAVE_FIGS`, e Python in
una shell mai riavviata passa da `tabs/module://cleecode_mpl` a `windows/None`. Una sessione già
avviata tiene quello con cui è nata, che non è una politica ma un fatto sui processi — e il
messaggio adesso lo dice: "dal prossimo Octave o Python che avvii".

**I pulsanti della figura che non potevano funzionare sembravano funzionanti.** I sei di sinistra
non toccano il quadro: chiedono alla sessione che l'ha disegnata di ridisegnare. Se quella non c'è
più — ed è lo stato normale di una figura nata da `▶ Run`, la cui shell finisce con lo script —
l'unica risposta era una riga nella barra di stato, la cosa più facile da non vedere. Ora sono
spenti quando dietro non c'è nessun interprete. Con Octave vivo, verificato: il clic sulla freccia
risponde "Panning — the session is redrawing it".

**E la barra si lascia colpire.** Nessun pulsante è più stretto di cinque celle — `+` ne aveva tre,
un bersaglio grande come un punto fermo — e la colonna di spazio fra due pulsanti adesso appartiene
a uno dei due invece che a nessuno. Sullo schermo non si è mosso niente: sono cresciuti i bersagli.


## 0.12.5 — la velocizzazione tolta il giorno stesso (2026-08-24)

La strada `drawnow` della 0.12.3 è tornata indietro. Su una sessione Linux via ssh scriveva nel
transcript dell'utente la chiacchiera di gnuplot — `multiplot> set style increment default;` e
`line 0: warning: deprecated command`, una volta per fotogramma di animazione — e su quella
macchina una finestra al posto della scheda.

Il perché è istruttivo e va scritto, perché la misura era giusta e la conclusione no. `drawnow`
disegna attraverso lo stream *vivo* di gnuplot della figura: il suo stderr è quello della shell, e
il suo terminale è quello che il display offre. `print` invece lancia un gnuplot suo, gli passa uno
script e non dice niente a nessuno. Sulla macchina di sviluppo — un Mac, dove il toolkit vero è qt
e quello stream non si è mai nemmeno aperto: `numel (get (f, "__plot_stream__")) == 0` — la
differenza non poteva manifestarsi. Tutti i controlli passavano: PNG veri, dimensione esatta,
linea, superficie con colorbar e immagine indistinguibili dalle stampate.

Quindi non è "print è lento" a essere sbagliato — quello resta vero e misurato, 243 ms contro 55
attraverso l'hook. È sbagliato aver considerato sufficiente una verifica fatta dove il caso
critico non esiste. La nota in `cleecode_figs.m` dice cosa è costata e cosa andrebbe dimostrato
prima di riprovarci: una Linux con il display inoltrato via ssh, che è esattamente la
configurazione che si è rotta.

Il transcript è dell'utente. Un frame rate non vale una riga scritta lì dentro.

## 0.13.1 — il quit congelato, e il lettore che aveva smesso di leggere (2026-08-24)

Segnalato dall'utente a minuti dalla 0.13.0: Quit — dal menu o con Ctrl+Q — congelava l'editor,
terminale mai restituito. Due cause, una sopra l'altra, entrambe invisibili ai test perché i
test muoiono con /bin/sh e il bug vuole una fish al prompt.

**La prima: `kill` di portable-pty non uccide.** Su unix manda SIGHUP e basta — "instead of
trying to kill the process", parole sue — e niente scala mai. /bin/sh a un SIGHUP muore, ed è
per questo che ogni test del ciclo di vita passava; una fish interattiva lo ignora, e il
`wait()` nel Drop aspettava un processo che non sarebbe morto mai. Ora la hangup è l'offerta e
SIGKILL è la scadenza: sei decimi di grazia, poi il segnale che nessun processo può rifiutare.

**La seconda, quella vera: il lettore usciva allo stop invece di drenare.** Con la escalation a
posto il quit si bloccava ancora, e `ps` mostrava una fish in stato `E` — non viva, non zombie:
*ferma a metà dell'exit*. Il kernel non lascia finire l'uscita di un processo finché l'output
rimasto nel suo pty non viene letto, e fish morendo ridipinge il prompt; il nostro thread
lettore, visto lo stop flag, era già uscito. Nessuno a leggere, exit mai completata, `wait4`
parcheggiata per sempre: il flag adesso ammutolisce — i byte si drenano e si buttano — e il
thread finisce all'EOF, come un terminale vero che legge fino all'hangup.

Diagnosi fatta campionando il processo congelato dell'utente (lo stack diceva `Drop → wait4`) e
poi con lldb su una riproduzione pilotata con fish nei pane. Il test nuovo fissa la metà
fissabile — una shell che ignora SIGHUP non può tenere aperto il pane — e la nota qui fissa
l'altra: un ciclo di vita dei pty non è provato finché non è provato con la shell che l'utente
usa davvero.

## 0.13.2 — la barra che insegna il markdown (2026-08-25)

Idea dell'utente: chi non sa la sintassi markdown a memoria si merita una toolbar da editor
WYSIWYG — che però scrive la sintassi vera nel buffer, sotto gli occhi. Educativa per
costruzione: si clicca B, si vedono comparire gli asterischi, e dopo un po' li si scrive da
soli e si spegne la barra da *Vista → Barra di formattazione* (persistito nei settings).

Undici azioni semantiche in `editor.rs`, ognuna un solo passo di undo: i toggle inline
contano i run di `*` ai bordi (corsivo presente ⇔ run dispari, così il corsivo su un
grassetto dà `***x***` invece di romperlo), i prefissi di riga seguono il modello
tutte-o-nessuna di `toggle_comment`, il link lascia selezionato il segnaposto `url` così la
prima cosa digitata è l'indirizzo. La barra ricalca la nav bar della preview — funzione di
layout pura condivisa fra renderer e hit-test del mouse, `pane_areas` come unica porta così i
nove call site non possono divergere — e sparisce da sola sotto le sei righe di pane. Nuovo
menu Formato (sempre visibile: un'azione che dice per quali file è si scopre, una che appare
e scompare col tab no), stato On/Off leggibile nella voce di Vista, niente scorciatoie nuove:
restano cinque lettere Ctrl+Shift libere in tutta l'applicazione e undici azioni non ci
starebbero. Specifica scritta a tavolino dopo una ricognizione del codice, implementazione
delegata, 17 test nuovi (418 in tutto).

Ripagato anche un piccolo debito trovato per strada: la 0.13.0 e la 0.13.1 erano uscite senza
aggiornare né la man page (ferma a "CleeCode 0.12.5") né `.github/release-notes.md` — il
guard della release confronta il tag col solo `Cargo.toml`. Le due sezioni mancanti sono
state scritte a posteriori insieme a quella nuova.

## 0.13.3 — il doppio click apre anche gli URL (2026-09-01)

Prima contribuzione da fuori: [PR #1](https://github.com/msavox/cleecode/pull/1) di
[@rikhza](https://github.com/rikhza), arrivata non richiesta su un repo che aveva appena
superato le trenta stelle. Il doppio click su una riga di terminale che dice `percorso:riga`
apriva già il file; ora una riga che dice `https://…` apre il browser, con la stessa consegna
al desktop di *Apri fuori da CleeCode* e lo stesso rifiuto via ssh, dove il desktop è di chi
sta seduto dall'altra parte. Codice pulito, nelle convenzioni della casa — commenti che dicono
il perché, test con nomi che sono frasi, i18n EN+IT, manuale integrato aggiornato in entrambe
le lingue — e con i rilievi di Copilot già chiusi dall'autore prima della review.

Sopra ci sono andate tre correzioni, tutte sullo stesso tema: un URL non è una stringa che
arriva da noi. Una riga di terminale è quello che ci hanno stampato un log di build, un `cat` o
un `curl`.

La prima è quella che conta. Su Windows l'opener dei file è `cmd /C start`, cioè una shell, e
una shell legge la `&` di una query string come fine di un comando e inizio del successivo:
`?a=1&b=2` avrebbe aperto mezza pagina e poi eseguito `b=2`. L'escaping degli argomenti di Rust
non aiuta — protegge il parsing del programma chiamato, non una seconda lettura fatta da `cmd`
— quindi non era solo un URL rotto, era esecuzione di comandi da una riga di output. Gli URL
adesso vanno a `explorer`, avviato direttamente, senza niente in mezzo che rilegga
l'argomento; e `open_url` rifiuta un URL che porti un carattere che un URI non può contenere
prima di avviare qualsiasi cosa, su tutte le piattaforme.

La seconda: un URL finisce dove un URL non può proseguire — spazio, carattere di controllo, o
uno di quelli che la RFC 3986 esclude — così `<https://x>` e `href="https://x"` lasciano fuori
il markup e una sequenza ANSI dopo l'indirizzo non ci finisce dentro. Una parentesi chiusa se
ne va solo se dentro l'URL non c'era niente ad aprirla: `…/wiki/Rust_(programming_language)` e
`http://[::1]:8080` si tengono la loro, `(vedi https://x)` cede la sua. Il non-ASCII resta:
`…/wiki/Perù` è un indirizzo vero. Regola classica dei linkificatori, che la versione
originale — `trim_end_matches` secco — non aveva.

La terza è una riga che nomina insieme un file che non c'è e un URL: adesso apre l'URL invece
di lamentarsi del file. E `find_url_start` è tornato a `str::find` invece di una scansione di
byte scritta a mano: gli offset che restituisce sono su un confine di carattere per
costruzione, e sparisce insieme il guard `is_char_boundary` che era stato aggiunto per
difendersi da sé stessa.

Dieci test nuovi (428 in tutto).

---

# L'ASTICELLA (2026-08-24) — da progetto serio a IDE quotidiano

> Scritta dopo una review completa del codice (sei passate indipendenti, finding verificati
> riga per riga) e dopo la campagna di correzioni che ne è seguita — sei ondate, dalla perdita
> di dati all'emulazione dei terminali. Questa sezione guarda avanti: cosa manca perché un
> programmatore C, C++, Rust, TypeScript o Java possa *viverci dentro* una giornata di lavoro,
> e in che ordine costruirlo. Octave e pylab restano il capitolo che nessun altro ha — il
> differenziatore, non l'identità.

## Il posizionamento, prima delle release

CleeCode non vince contro Zed o VS Code sul loro terreno, e non deve provarci. Il terreno suo
è un altro: **l'IDE che funziona via ssh a configurazione zero**. Un binario, batterie incluse,
il mouse funziona, l'output del compilatore si clicca. Contro VS Code Remote (che vuole il suo
server sulla macchina in fondo) e contro Neovim (che è un progetto di configurazione prima che
un editor), quella frase regge già oggi. Le release qui sotto servono a renderla vera per una
giornata intera, non solo per la prima ora.

Tre vincoli che non cambiano, qualunque release: niente dipendenze C (la lezione di git2);
un solo modello di concorrenza — thread, `mpsc`, `poll_*` nel ciclo dei frame — niente tokio;
e ogni funzionalità verificata guidando il binario vero, con controlli che dimostrano di
guardare nel posto giusto (la lezione pagata tre volte fra 0.9.1 e 0.10).

## 0.13 — Il debito ripagato ← *il lavoro c'è, va cotto*

Le sei ondate della review: le quattro strade che perdevano modifiche non salvate, le
scritture atomiche ovunque, il Drop dei terminali (SIGHUP al gruppo, kill, wait), lo stack
overflow del walk, l'evidenziazione incrementale, il ciclo eventi che disegna solo quando
serve, le posizioni LSP nelle unità giuste, il server che riparte, il pannello Git sul
repository appena nato, bracketed paste e mouse dentro i pane, il paste in tutte le caselle.

**Quello che la 0.13 chiede prima di uscire non è codice: è uso.** Una-due settimane di lavoro
vero, incluso il ramo che ha trovato i bug di ogni release passata — la sessione ssh su Linux.
Le modifiche al ciclo eventi e al writer dei pty sono profonde e i test unitari non bastano a
battezzarle: la roadmap di questo stesso file insegna che i bug veri arrivano dall'uso, non da
`cargo test`.

E due cose piccole che la review ha segnato:
- **i driver pty entrano in CI** (`scripts/drive_*.py`). Girano su ubuntu-latest contro il
  binario vero; sono l'unica verifica dell'interfaccia interattiva e oggi girano solo a mano.
- **il guard tag↔versione nella release**: una riga che confronta il tag con `Cargo.toml`
  prima di pubblicare binari che direbbero una versione sbagliata.

## 0.14 — L'editor con un agente dentro

Deciso il 2026-08-24, su indicazione dell'utente: prima dei temi e prima dell'LSP completo,
perché nel 2026 è la prima domanda che un utente nuovo fa a un editor — e perché qui la
risposta costa poco. **Claude Code, opencode e codex sono programmi da terminale, e CleeCode
ospita terminali veri.** Nessun editor GUI parte da questa posizione: loro devono *incorporare*
un agente, noi dobbiamo solo presentargli bene la casa. Le ondate della review hanno appena
reso vero il prerequisito senza saperlo: mouse reporting, bracketed paste, tasti funzione e
glifi wide dentro i pane sono esattamente ciò che serve a una TUI come Claude Code per essere
usabile in un riquadro.

L'integrazione è in quattro pezzi, dal più economico al più profondo:

**1. I preset.** `clee -w claude`, `clee -w opencode`, `clee -w codex`: editor a sinistra,
l'agente in una scheda del terminale, una shell semplice nell'altra — la stessa macchina dei
preset octave/pylab, che esiste apposta. Un preset è una promessa su cosa compare quando scrivi
il suo nome, e i driver la verificano scrivendolo.

**2. Un seam solo, N agenti.** La lezione dell'handoff numerico — *una funzionalità con due
backend, non due lavori* — vale identica: un adattatore `Agent` accanto a `Language` in
`session.rs` che risponde alle stesse domande (quali nomi di programma sono quell'agente, come
si nomina un file al suo prompt, come gli si consegna un blocco di testo senza incollarlo riga
per riga). Su quel seam: **mandare contesto con un tasto** — il file corrente, la selezione,
o il diagnostico sotto il cursore — scritto al prompt dell'agente come riferimento
`percorso:riga`, via file d'appoggio dove serve, con la stessa disciplina di `Ctrl+Shift+X`.
Il ritorno esiste già ed è gratis: gli agenti stampano `file:riga` di continuo, e il doppio
click di `locate.rs` li apre da sempre.

**3. I file sotto le mani dell'agente, in diretta.** Il punto di partenza è capire cosa fa
davvero un agente: non digita — **scrive il file intero a ogni edit**. Quindi "live" non è
streaming, è reagire bene a una sequenza di scritture atomiche, che è ciò che un editor sa
osservare. Dalla 0.13 le modifiche esterne si rilevano su *tutti* i buffer: un file aperto si
aggiorna già da solo mentre l'agente ci lavora. Sopra quella base, tre pezzi:
- *le righe cambiate si vedono*: al reload il rope vecchio è ancora in mano — un diff di
  righe (funzione pura) e le righe nuove si accendono nel gutter, che già disegna breakpoint
  e diagnostici, finché un tasto non le spegne;
- *il modo segui*, spento di default: i file che l'agente tocca e che non hai aperto si
  aprono **a fianco, senza prendere la tastiera** — la regola delle figure della 0.9, "mostra
  senza prendere". E il rilevamento non vuole un watcher nuovo: `git status` gira già ogni
  700 ms, e la differenza fra due snapshot consecutivi *è* la lista dei file toccati — gratis
  e agnostica sull'agente (vale per Claude, codex, opencode e per un sed in una shell). Fuori
  da un repo git il modo segui semplicemente non c'è, e lo dice;
- *la regola di sicurezza che c'è già*: un buffer sporco non si auto-ricarica mai — il lavoro
  dell'utente vince su quello dell'agente, sempre.
Il pannello Git resta la review: lo stato mostra cosa ha toccato la sessione, il diff cosa ha
scritto, scartare è già una domanda in rosso. Raffinamento opzionale, se l'uso lo chiede: un
hook PostToolUse di Claude Code che scrive il percorso toccato in un file di contratto (il
pattern wsnap) — zucchero per-agente sopra un meccanismo che regge senza.

**4. Il canale profondo: un server MCP, non tre integrazioni.** Tutti e tre gli agenti parlano
MCP; CleeCode può *essere* un server MCP su stdio — `clee --mcp` — che espone quello che solo
l'editor sa: i file aperti e quello attivo, la selezione, i diagnostici del language server,
e un piccolo set di azioni (apri questo file a questa riga). Una implementazione, tre
consumatori; la configurazione per-agente è una riga nel suo config, documentata nel manuale.
È JSON-RPC su stdio, cioè esattamente la macchina già scritta due volte (client LSP, stub dei
test) — niente tokio, niente dipendenze nuove, il modello thread+mpsc+poll che c'è già.
Stessa regola del pannello Git per le azioni: leggere ha rischio zero e si espone largo,
scrivere passa dal consenso dell'utente.

> **Fatto.** `clee --mcp` in `src/mcp.rs`: NDJSON su stdio (non il framing LSP), quattro tool —
> `open_files`, `selection`, `diagnostics`, `open_file`. Il ponte è file-based come wsnap: la dir
> di sessione `~/.config/cleecode/sessions/<pid>/`, esportata alle shell come `CLEE_SESSION` e
> quindi ereditata dall'agente e dal suo server; `state.json` scritto atomico e throttlato,
> le richieste in `requests/`. `open_file` apre **a fianco senza prendere la tastiera**. Nessun
> tool scrive in un buffer: il consenso dell'utente è UI di un'iterazione successiva.

**Cosa deliberatamente NON si fa:** nessuna chat ricostruita dentro l'editor, nessuna chiave
API custodita da CleeCode, nessun agente reimplementato. Gli agenti esistono, sono bravi, e
sono TUI: il valore di CleeCode è essere il posto migliore dove farli girare — con l'editor,
il compilatore cliccabile e il pannello Git attorno — non una copia peggiore di uno di loro.
Chi ha l'agente ce l'ha già; chi non ce l'ha ha un editor completo senza.

## 0.15 — Presentabile a un estraneo

Le due cose che ogni nuovo utente incontra nei primi cinque minuti, nell'ordine in cui le
incontra.

**I temi.** Oggi ogni colore è cablato per un fondo quasi nero: su un terminale chiaro la barra
di stato e l'albero sono ai limiti dell'illeggibile, ed è la prima impressione. Il lavoro è in
due metà che vanno insieme: i colori della UI raccolti in una palette (una struct, non un file
di configurazione — prima si separa, poi eventualmente si espone), e il tema syntect scelto di
conseguenza — quelli chiari sono già compilati nel binario, `base16-ocean.light` non costa un
byte. Un'impostazione a tre valori: scuro, chiaro, auto. L'auto legge il terminale dove si può
(OSC 11 con timeout, come già si interrogano device attributes e kitty) e ripiega su scuro.

> **L'ordine cambia (2026-09-01).** La metà meccanica dei temi passa *davanti* al lavoro sugli
> agenti. Il motivo non è che sia più urgente, è che la palette tocca ogni punto di disegno
> dell'applicazione: la 0.14 porta UI nuova — le righe cambiate accese nel gutter, il modo segui —
> e scriverla contro una palette che esiste costa meno che scriverla coi colori cablati e
> riconvertirla il mese dopo. Chi arriva secondo paga il doppio, quindi la palette arriva prima.
> I numeri seguono l'ordine: i temi sono la 0.14, gli agenti la 0.15.

Il lavoro si spezza in due pezzi rilasciabili separatamente. **Il primo è meccanico e senza
rischi**: la struct della palette, la scelta scuro/chiaro a mano, il tema syntect di conseguenza.
Nessun protocollo, nessuna interrogazione, ogni modifica locale e verificabile. **Il secondo è
l'`auto`** con OSC 11 e il suo timeout — l'unico pezzo che può andare storto, perché un terminale
che non risponde non deve poter ritardare l'avvio, e va scritto quando il primo è già in uso.

> **Fatti tutti e due.** Il primo pezzo è uscito con la 0.14; l'`auto` è dentro dal 2026-09-02.
> `theme = "auto"` chiede lo sfondo con OSC 11 accanto alle altre domande d'avvio, prima della
> cattura del mouse, e la domanda si fa solo se il tema è già `auto`: chi ne ha scelto uno per
> nome non paga niente. Attesa 150 ms su `poll`, non un thread parcheggiato in `read` che poi si
> mangerebbe il primo tasto; un terminale muto ottiene il tema scuro. La risposta vale per la
> sessione — scegliere Auto dalla tendina risolve su quella, e se all'avvio non era `auto`
> dipinge scuro fino al riavvio, perché a mouse catturato la domanda non si può rifare.

Le misure, prese sul codice della 0.13.3: 239 occorrenze di `Color::` in tutto il sorgente, 224
in `ui.rs` e 13 in `preview.rs`. Di quelle in `ui.rs`, 34 sono i colori delle icone dei file
(colori di marca, giustamente fissi) più quelli del disegno della finestra Informazioni. Restano
circa duecento punti da instradare sulla palette, e nei test ci sono solo sei asserzioni di
colore: un diff grosso ma piatto, con una superficie di regressione pari a tutta la UI e un
rischio per singola riga vicino a zero. Il lato syntect è invece un punto solo, `highlight.rs`,
più un ri-highlight dei buffer aperti quando il tema cambia.

**Il pezzo difficile c'è già.** `paint_background` riempie ogni cella rimasta sul colore del
terminale, fatto come passata sul frame finito apposta perché i modali che fanno `Clear` non
aprano buchi trasparenti. CleeCode sa già smettere di essere trasparente e dipingere la propria
superficie: un tema con un campo opaco è quel meccanismo con un altro colore dentro.

**Il tema attuale si chiama CleeCode** ed è il default; tutti gli altri sono aggiunte. Per i temi
importati la palette della UI si può *derivare*, perché un tema syntect porta nei suoi `settings`
sfondo, primo piano, selezione e gutter; scritte a mano restano solo le palette dei temi nostri.

**Turbo, il collaudo.** Un tema ispirato all'IDE di Turbo Pascal e Turbo C — blu EGA, cornice
grigia, la lettera acceleratrice in rosso — va scritto *secondo*, subito dopo il default, perché
è il caso che mette sotto sforzo la palette come nessun altro tema scuro farebbe: chiede un
colore di campo distinto da quello di cornice, un accento usato come sfondo e non come testo, e
chiaro-su-scuro e scuro-su-chiaro dentro lo stesso tema. Se la struct regge Turbo Pascal regge
qualunque cosa le chiederemo dopo, ed è meglio scoprirlo mentre cambiarla è ancora gratis. Si
chiama *Turbo* e non *Borland* per la stessa regola sulle licenze qui sotto. Perimetro: v1 solo
colori. I bordi doppi e l'ombra dei dialoghi sono glifi, non colori — eventualmente un campo
separato più avanti, se l'effetto senza non convince.

**Sette temi sono già dentro il binario** e oggi ne buttiamo via sei: `load_defaults()` di syntect
porta `base16-ocean.dark` (l'attuale), `base16-eighties.dark`, `base16-mocha.dark` e
`Solarized (dark)` fra gli scuri, `base16-ocean.light`, `Solarized (light)` e `InspiredGitHub`
fra i chiari. Il bundle scuro-e-chiaro esiste già: basta smettere di ignorarlo.

**La regola sulle licenze, per il bundle.** Un tema è un file di colori e copiarlo è banale, ma
nome e file sono due questioni diverse: il nome è marchio, il file è copyright. Rinominare serve
contro il primo e non estingue il secondo — un file MIT copiato, rinominato e privato della nota
di copyright non è una scappatoia, è la violazione. Quindi: **licenza permissiva, nome vero,
`themes/NOTICE`** con autore e licenza di ciascuno; oppure **palette nostra e nome nostro**, come
Turbo. Niente file altrui con la targhetta cambiata. I nomi delicati (Darcula è di JetBrains,
Dracula ha marchio e linee guida, Monokai ha una variante commerciale) non valgono la fatica,
perché il terreno cromatico è coperto da roba pulita: Catppuccin per lo scuro violaceo, Gruvbox
per il grigio caldo, One Dark per il grigio-blu. Da aggiungere ai sette di casa, tutti MIT:
Nord, Gruvbox Dark e Light, One Dark e One Light, Catppuccin Mocha e Latte.

**L'importazione, dopo.** I `.tmTheme` costano quasi zero — syntect è un motore TextMate e
`ThemeSet::get_theme` legge già un file da `~/.config/cleecode/themes/`. Il formato
d'interscambio giusto per noi è però **base16**: sedici colori con una mappatura definita, nato
per i terminali, e l'unico che copre con un file solo sia la palette della UI sia la sintassi.
Il JSON di VS Code resta rimandato: i `tokenColors` sono scope TextMate e si convertono, ma le
centinaia di chiavi `workbench.*` sono pensate per una GUI e ne useremmo otto.

**La scelta rapida** è una tendina accanto al pulsante dello sfondo, all'estremità destra della
barra dei menu: il pulsante e la sua zona di clic esistono già, e la tendina si ancora come
quelle dei menu. Vale la stessa regola del pulsante — è fra i primi a cedere quando la finestra
è stretta. Una domanda da decidere lì: se scegliere un tema a fondo opaco debba accendere da sé
il flag dello sfondo, o se i due interruttori restino indipendenti e liberi di contraddirsi.

✅ **I keybinding rimappabili.** I vincoli di questo progetto — niente F-key, niente Alt+lettera,
niente Ctrl+freccia — sono sacrosanti *per un layout italiano su macOS* e arbitrari per
chiunque altro. La forma: una tabella `[keys]` in `settings.toml`, azione = corda, che
sovrascrive i default uno alla volta; i default non si toccano. Il test del manuale
(`every_advertised_key_is_written_down`) deve imparare che una scorciatoia pubblicizzata può
essere stata rimappata — il manuale mostra quella effettiva. Non è un sistema di keymap alla
vim: è la possibilità di spostare una corda che sul tuo layout non esiste.

> Fatto in `src/keymap.rs`: ventiquattro azioni con nome kebab-case, i default intatti, avviso
> sulla status line per nome o corda illeggibile e per due azioni sulla stessa corda (vince la
> prima dichiarata). **CleeCode ▸ Scorciatoie...** semina in `settings.toml` la sezione `[keys]`
> con ogni azione commentata sul tasto di oggi — generata dalla tabella, quindi un'azione nuova ci
> compare da sé — e apre il file; salvarlo ricarica le corde.

## 0.16 — Il refactoring quotidiano

La release che decide se un professionista ci resta. Due fronti, entrambi già appoggiati su
seam esistenti.

**Il language server per intero.** Diagnostici, definizione, hover e completamento sono il 60%;
il 40% che manca è quello che si usa ogni ora:
- *references* (`Ctrl+Shift+?` da decidere sulla tabella delle lettere libere) in un picker,
  come i risultati di ricerca;
- *rename* — il primo comando LSP che **scrive**, e la disciplina è quella del pannello Git:
  anteprima di cosa cambia e dove, applicazione atomica file per file (le scritture atomiche
  della 0.13 sono il prerequisito, ed è per questo che il rename arriva dopo di loro), un solo
  passo di undo per i buffer aperti, rifiuto onesto per i file fuori dai buffer che il server
  vuole toccare e non stanno in nessuna tab;
- *format* on demand (non on-save: on-save è una politica, e le politiche arrivano dopo i
  meccanismi) via `textDocument/formatting`, applicato come un edit unico;
- *document symbols* in un picker — l'outline è una tendina, non un pannello;
- *trigger characters*: il popup che si apre su `.` e `::` è esattamente il punto in cui la
  sorgente LSP supera le parole del buffer, e oggi non scatta;
- la *lista dei diagnostici* di progetto, un `PickerKind` in più sui dati che già arrivano.

**Sostituisci nel progetto.** La ricerca c'è; manca la metà che scrive. Stessa disciplina del
rename perché *è* lo stesso problema: anteprima raggruppata per file, `find::compile` resta
l'unico posto dove una query diventa un pattern (la decisione della 0.6 vale ancora),
applicazione atomica, buffer aperti aggiornati in un passo di undo ciascuno, file su disco
riscritti con la strada temp+rename.

## 0.17 — La potenza sotto le dita

- **La selezione a colonna scrive su ogni riga.** È il multi-cursor nella forma che il progetto
  ha già mezzo costruito: digitare con un blocco attivo inserisce su tutte le righe del blocco,
  Backspace idem. Il multi-cursor arbitrario (Ctrl+D "prossima occorrenza") viene dopo, se
  viene: la colonna copre l'80% dei casi con il 20% della macchina.
- **Autosave dei buffer sporchi.** Lo scudo tiene vivo il processo, ma un SIGKILL o uno stack
  overflow perdono ancora tutto quello che non era salvato. Una copia dei dirty ogni pochi
  secondi in una cartella di recovery, offerta al riavvio se più recente del file. Chiude per
  davvero la promessa scritta nel README: *it does not close on you* — e quando succede lo
  stesso, non ti è costato niente.
- **La barra di stato dice cosa stai editando**: encoding ed EOL accanto a riga:colonna, con
  la conversione CRLF↔LF a un comando. Piccolo, ma è il genere di assenza che un utente Windows
  o un file legacy trasformano in sfiducia.
- **Un modo large-file dichiarato**: sopra una soglia (50 MB?) niente highlighting, niente
  indice del completamento, undo a profondità ridotta — e la barra che lo dice. Meglio un
  editor che dichiara i suoi limiti di uno che li scopre congelandosi.

## La vetrina — il marchio, il sito, i pacchetti (2026-09-02)

Senza numero di release apposta: non è una release di funzionalità, è il lavoro che rende
trovabile e installabile quello che le release costruiscono, e si fa a pezzi accanto a loro.
Tre pezzi, in ordine di dipendenza:

- **Il marchio Marunja.** La gerarchia, detta una volta per tutte: Marunja è la casa,
  msavox (Matteo Savoia) l'autore, CleeCode il marchio dell'applicazione. CleeCode si
  dichiara un prodotto Marunja nella finestra About, nel README e nel piè di pagina del
  sito — un'apposizione sobria, non un rebranding: l'editor si chiama CleeCode e continua
  a chiamarsi così. Ovunque serva un copyright (il sito madre già fa così) la firma è
  © msavox.
- **Il sito: `cleecode.marunja.com`.** Una vetrina alla maniera del sito di VS Code: le
  feature principali mostrate (gli screenshot dei temi e le tape demo esistono già in
  `docs/` e si rigenerano da script — il sito li riusa, non li duplica), e i pulsanti di
  download per sistema operativo. Il selling point in testa è quello della 0.15: il tuo
  agente — Claude Code, codex, opencode — con l'abbonamento che hai già, dentro l'editor,
  e i suoi edit visibili live nei buffer; nessuna chiave API da configurare, da custodire
  o da pagare a consumo. Nessun editor GUI può dire questa frase intera, e la vetrina la
  dice per prima. Statico, su una pagina Cloudflare; il DNS è del dominio
  marunja.com e ci arriva con una redirezione. Il sito dice anche la strada che c'è già:
  `brew install` dal tap per macOS. E il sito madre ricambia: su marunja.com un piccolo
  banner — o una card, come quella che Maestrino ha già — che manda a
  `cleecode.marunja.com`; il sito madre è VitePress nel monorepo marunja-suite, e la card
  di Maestrino è il precedente da copiare.
- **I pacchetti Linux: `.deb` come minimo.** Oggi Linux scarica un binario dalla release;
  un utente Debian/Ubuntu si aspetta `apt install ./clee.deb`. Il pacchetto entra nella CI
  guidata dai tag accanto ai binari esistenti (il guard tag↔versione della 0.13 vale anche
  per lui), con dentro binario, man page e desktop entry. Gli altri formati — rpm,
  AppImage — dopo, se qualcuno li chiede: un formato di pacchetto è una promessa di
  manutenzione, e le promesse si fanno una alla volta.

## 1.0 — la definizione, non una data

La 1.0 non è una release di funzionalità: è un elenco di frasi che devono essere vere.

1. Una giornata di lavoro C/Rust/TS — edit, build, test, commit, refactor — senza uscire
   dall'editor e senza incontrare un limite non dichiarato.
2. Le stesse cose, via ssh su una Ubuntu, con la stessa esperienza.
3. Un utente su layout tedesco con terminale chiaro non deve toccare il TOML per avere colori
   leggibili e scorciatoie raggiungibili.
4. Un crash — di qualunque tipo, scudo o non scudo — non costa più di qualche secondo di
   lavoro.
5. I driver pty girano in CI su ogni push, e nessun controllo "guarda nel posto sbagliato".
6. `clee -w claude` (o opencode, o codex) apre un posto di lavoro completo: l'agente in un
   pane usabile, il contesto dell'editor a un tasto, i suoi edit visibili nei buffer e nel
   pannello Git senza toccare niente.

## Cosa resta fuori, e perché è una decisione

- **Il debugging DAP** (gdb/lldb per C e Rust). La risposta dichiarata resta: il debugger gira
  nel terminale accanto, che è vero e onesto per il pubblico ssh. DAP è una release intera a
  sé — protocollo, UI dei frame, watch — e farla a metà produrrebbe la cosa peggiore: un
  debugger che sembra esserci. Il giorno che si fa, il pannello del workspace numerico è il
  precedente da seguire (breakpoint nel gutter, frame nel pannello — la forma c'è già).
- **Tree-sitter.** syntect con la scala di stati incrementale della 0.13 basta per
  l'evidenziazione; tree-sitter porterebbe una dipendenza C per grammatica e un secondo
  modello del testo da tenere in pari con la rope. Se ne riparla solo se il folding semantico
  o la selezione strutturale diventano priorità — non prima.
- **Un sistema di plugin.** Le due tabelle (`run_commands`, `language_servers`) più il
  contratto file del workspace (versionato, documentato in `docs/design/`) sono già
  un'estensibilità in embrione. Formalizzarla è lavoro da dopo-1.0: un'API si può aggiungere,
  una sbagliata non si può togliere.
- **tokio, git2, e ogni seconda copia di un modello che c'è già.** Le ragioni scritte nella
  valutazione del 2026-08-17 non sono invecchiate di un giorno.
