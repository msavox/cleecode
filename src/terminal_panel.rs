use anyhow::Result;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long the shell's output must be quiet before we consider startup finished and
/// reveal the pane. Quiet means the rc has run and there is a prompt, which is when the
/// banner is scrubbed and any startup command typed — see `flush_pending`.
const STARTUP_IDLE: Duration = Duration::from_millis(250);
/// Safety cap: reveal no matter what after this long, so a shell that keeps its pty busy
/// (e.g. an rc that launches a long-running program) can't stay blank forever.
const STARTUP_MAX: Duration = Duration::from_secs(12);

/// How many lines of scrolled-off output each new shell keeps, set once at startup from
/// `terminal_scrollback` in settings.toml. A global rather than a constructor argument because
/// vt100 fixes the length at `Parser::new` and offers no setter: changing the preference can
/// only ever affect shells opened afterwards, and threading a value through four constructors
/// would suggest otherwise.
static SCROLLBACK_LEN: AtomicUsize = AtomicUsize::new(DEFAULT_SCROLLBACK);

/// A number for each pane CleeCode starts, so their workspace snapshots do not overwrite one
/// another. Only ever compared and never interpreted, so wrapping would be harmless and reaching
/// the wrap is not possible in a session.
fn next_pane_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Matches kitty's default. Worth being deliberate about: vt100 stores whole rows of 32-byte
/// cells at the pane's full width, so the cost is rows x columns x 32 bytes per shell — 16 MB
/// for 2000 lines across a wide 250-column pane, and every tab pays it separately.
pub const DEFAULT_SCROLLBACK: usize = 2000;

pub fn set_scrollback_len(lines: usize) {
    SCROLLBACK_LEN.store(lines, Ordering::Relaxed);
}

/// How many lines have scrolled off the top and are still held.
///
/// vt100 publishes the configured cap and the current offset, but not how much is actually
/// stored. Asking by clamping is exact: `set_scrollback` pins the offset to the real length
/// when handed more than that, so pushing it past the end and reading it back *is* the count.
/// The offset in use is restored before returning, so this stays a read.
fn held_lines(parser: &mut vt100::Parser) -> usize {
    let screen = parser.screen_mut();
    let current = screen.scrollback();
    screen.set_scrollback(usize::MAX);
    let total = screen.scrollback();
    screen.set_scrollback(current);
    total
}

/// Feeds the shell's output to the parser, keeping a reader who has scrolled back where they
/// are.
///
/// vt100 counts the scrollback offset from the live screen and never adjusts it itself, so
/// every line that scrolls off would slide the view up by one: a build printing steadily would
/// drag the page away while it was being read. Counting what got pushed and adding it back pins
/// the view to the same output. Once the buffer is full the oldest lines really are gone, and
/// the `saturating_sub` lets the view drift then rather than pretending otherwise.
fn process_anchored(parser: &mut vt100::Parser, data: &[u8]) {
    let offset = parser.screen().scrollback();
    if offset == 0 {
        parser.process(data);
        return;
    }
    let before = held_lines(parser);
    parser.process(data);
    let pushed = held_lines(parser).saturating_sub(before);
    parser.screen_mut().set_scrollback(offset + pushed);
}

/// Locks a mutex a terminal shares with its reader thread, stepping over poison. If that
/// thread ever panics — on a malformed escape sequence, say — `unwrap()` here would turn one
/// broken terminal into a panic on the main thread, closing the editor and every other shell
/// in it. The data behind the lock is still perfectly readable, so the poison is ignored on
/// purpose: the damage stays inside the terminal it happened in.
pub fn lock_poisoned<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct TerminalPanel {
    master: Box<dyn MasterPty + Send>,
    /// Shared with the reader thread so it can answer terminal queries (DSR/DA) inline,
    /// while the main thread still uses it for user keystrokes.
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child: Box<dyn Child + Send + Sync>,
    pub parser: Arc<Mutex<vt100::Parser>>,
    pub rows: u16,
    pub cols: u16,
    /// Set once the shell's end of the pty closes (process exited, whether via `exit`,
    /// Ctrl+D, a command that dies on Ctrl+C, a crash, or an ssh disconnect).
    pub exited: Arc<AtomicBool>,
    /// When the pane was spawned, and the last moment the shell produced output (millis
    /// since `spawn`, written by the reader thread). Used to hide the noisy startup banner
    /// until the shell settles at a clean prompt.
    spawn: Instant,
    last_output_ms: Arc<AtomicU64>,
    produced_output: Arc<AtomicBool>,
    /// Latches true once the pane has been revealed, so we never hide it again mid-session.
    revealed: bool,
    /// Text selected in this pane, for copying out of the terminal. The app grabs the mouse,
    /// so the host terminal's own selection is unavailable while cleecode runs.
    pub selection: Option<TermSelection>,
    /// A user-given name for this tab, shown in the tab strip (or the window title when it is the
    /// only tab) in place of the default "Terminal N".
    pub name: Option<String>,
    /// A command to run in this shell when the workspace holding it is opened (`claude`,
    /// `octave`, `npm run dev`…). Remembered with the workspace; not re-run on its own.
    pub startup_command: Option<String>,
    /// The startup command, held back until the shell is actually reading. See `run_command`.
    pending_command: Option<String>,
    /// Whether the banner has been scrubbed since this shell started. See `flush_pending`.
    cleared: bool,
    /// When the history was last scrolled through. The scrollbar is a hint rather than
    /// furniture: it appears while it is being used and fades back out, so an idle pane stays
    /// all output and no chrome.
    last_scroll: Option<Instant>,
}

/// A text selection over the terminal's visible screen, in cell coordinates. `anchor` is
/// where it started, `cursor` where it currently ends; either may come first on screen.
/// It flows like text, not as a rectangle: to the end of the first row, then whole rows,
/// then up to the cursor on the last one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TermSelection {
    pub anchor: (u16, u16),
    pub cursor: (u16, u16),
}

impl TermSelection {
    pub fn new(cell: (u16, u16)) -> Self {
        TermSelection { anchor: cell, cursor: cell }
    }

    /// Endpoints in screen order, so callers don't have to care which way it was dragged.
    pub fn ordered(&self) -> ((u16, u16), (u16, u16)) {
        if self.anchor <= self.cursor { (self.anchor, self.cursor) } else { (self.cursor, self.anchor) }
    }

    /// True while the selection has never left its starting cell, which is what a plain click
    /// looks like — the caller drops it instead of treating one character as a selection.
    pub fn is_single_cell(&self) -> bool {
        self.anchor == self.cursor
    }

    pub fn contains(&self, row: u16, col: u16) -> bool {
        let ((start_row, start_col), (end_row, end_col)) = self.ordered();
        if row < start_row || row > end_row {
            return false;
        }
        let after_start = row > start_row || col >= start_col;
        let before_end = row < end_row || col <= end_col;
        after_start && before_end
    }
}

/// Builds the selected text row by row. `cell` yields the character at a position (empty for
/// a blank), which keeps this testable without a real pty. Trailing blanks are trimmed per
/// row — they are padding on screen, not something anyone means to copy — and rows are joined
/// with newlines.
pub fn selected_text(selection: TermSelection, cols: u16, cell: impl Fn(u16, u16) -> String) -> String {
    let ((start_row, start_col), (end_row, end_col)) = selection.ordered();
    let mut rows = Vec::new();
    for row in start_row..=end_row {
        let from = if row == start_row { start_col } else { 0 };
        let to = if row == end_row { end_col } else { cols.saturating_sub(1) };
        let mut line = String::new();
        for col in from..=to.min(cols.saturating_sub(1)) {
            let contents = cell(row, col);
            // vt100 reports the second half of a wide character as empty; a blank cell also
            // comes back empty, and both should read as a space here.
            line.push_str(if contents.is_empty() { " " } else { &contents });
        }
        rows.push(line.trim_end().to_string());
    }
    rows.join("\n")
}

/// End-of-line then kill-line: empties whatever the line editor is holding before a command is
/// typed onto it. Every Unix shell binds both, and `\x15` is the tty's own kill character even
/// for a shell that binds nothing. cmd.exe binds neither, so on Windows the command is typed as
/// it is rather than after two characters it would take literally.
const LINE_RESET: &[u8] = if cfg!(windows) { b"" } else { b"\x05\x15" };

/// The bytes that type `command` into a shell and run it: clear the line, the command, one
/// carriage return. One line goes in and one line comes out, whatever the line editor was
/// holding before — see `flush_pending` for why each piece is there. Any newlines inside the
/// command itself would each submit a line of their own, so they are flattened to spaces.
fn typed_line(command: &str) -> Vec<u8> {
    let mut bytes = LINE_RESET.to_vec();
    bytes.extend(command.replace(['\r', '\n'], " ").as_bytes());
    bytes.push(b'\r');
    bytes
}

/// A wheel notch written the way the program asked to receive it.
///
/// Wheel up is button 64 and wheel down 65, and both are reported as presses with no release —
/// there is no such thing as letting go of a wheel. Coordinates are one-based, counted from the
/// top-left of the pane rather than of the window, since the pane is the whole world as far as
/// the program inside it knows.
///
/// The two older encodings pack each number into one byte with an offset of 32, which is why
/// they cannot describe anything past column 223; a program that asked for one of them gets the
/// events it can express and none of the ones it cannot, rather than a byte that would land it
/// somewhere else entirely. SGR has no such limit and is what anything modern asks for.
fn encode_wheel(
    encoding: vt100::MouseProtocolEncoding,
    up: bool,
    row: u16,
    col: u16,
) -> Vec<u8> {
    let button: u16 = if up { 64 } else { 65 };
    let (row, col) = (row.saturating_add(1), col.saturating_add(1));
    match encoding {
        vt100::MouseProtocolEncoding::Sgr => {
            format!("\x1b[<{button};{col};{row}M").into_bytes()
        }
        vt100::MouseProtocolEncoding::Utf8 => {
            let mut out = b"\x1b[M".to_vec();
            let mut push = |n: u32| {
                let mut buf = [0u8; 4];
                out.extend_from_slice(
                    char::from_u32(n + 32).unwrap_or(' ').encode_utf8(&mut buf).as_bytes(),
                );
            };
            push(u32::from(button));
            push(u32::from(col));
            push(u32::from(row));
            out
        }
        vt100::MouseProtocolEncoding::Default => {
            let byte = |n: u16| u8::try_from(n + 32).unwrap_or(u8::MAX);
            vec![0x1b, b'[', b'M', byte(button), byte(col.min(223)), byte(row.min(223))]
        }
    }
}

/// The interactive shell to spawn in a new terminal pane. Honours `$SHELL` on Unix and
/// `%ComSpec%` on Windows, falling back to `/bin/bash` and `cmd.exe` respectively.
fn default_shell() -> String {
    if cfg!(windows) {
        std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    }
}

/// A terminal capability/status query a program sent us and expects a reply for. Without a
/// reply, tools like fastfetch block for seconds waiting, which is the main cause of slow
/// pane startup inside the embedded terminal.
#[derive(PartialEq, Eq, Debug)]
enum TerminalQuery {
    CursorPosition,
    Status,
    PrimaryDeviceAttributes,
    SecondaryDeviceAttributes,
}

impl TerminalQuery {
    fn response(&self, cursor_row: u16, cursor_col: u16) -> Vec<u8> {
        match self {
            // DSR 6n: report the cursor position (1-based row/col).
            TerminalQuery::CursorPosition => {
                format!("\x1b[{};{}R", cursor_row + 1, cursor_col + 1).into_bytes()
            }
            // DSR 5n: report terminal OK.
            TerminalQuery::Status => b"\x1b[0n".to_vec(),
            // DA1: identify as a VT100 with Advanced Video Option (xterm's basic reply).
            TerminalQuery::PrimaryDeviceAttributes => b"\x1b[?1;2c".to_vec(),
            // DA2: harmless xterm-ish terminal id/version.
            TerminalQuery::SecondaryDeviceAttributes => b"\x1b[>0;10;0c".to_vec(),
        }
    }
}

/// Streaming scanner over the shell's output that recognises CSI queries ending in `n`
/// (DSR) or `c` (DA) and records which replies are needed. Keeps partial-sequence state
/// across reads so a query split across two chunks is still detected.
#[derive(Default)]
struct CsiScanner {
    /// 0 = ground, 1 = saw ESC, 2 = inside a CSI collecting parameter bytes.
    state: u8,
    params: Vec<u8>,
}

impl CsiScanner {
    fn feed(&mut self, data: &[u8], out: &mut Vec<TerminalQuery>) {
        for &b in data {
            match self.state {
                1 => match b {
                    0x1b => {}
                    b'[' => {
                        self.state = 2;
                        self.params.clear();
                    }
                    _ => self.state = 0,
                },
                2 => {
                    if (0x40..=0x7e).contains(&b) {
                        match b {
                            b'n' if self.params == b"6" => out.push(TerminalQuery::CursorPosition),
                            b'n' if self.params == b"5" => out.push(TerminalQuery::Status),
                            b'c' if self.params.first() == Some(&b'>') => {
                                out.push(TerminalQuery::SecondaryDeviceAttributes)
                            }
                            b'c' => out.push(TerminalQuery::PrimaryDeviceAttributes),
                            _ => {}
                        }
                        self.state = 0;
                    } else if self.params.len() < 32 {
                        self.params.push(b);
                    }
                }
                _ => {
                    if b == 0x1b {
                        self.state = 1;
                    }
                }
            }
        }
    }
}

/// A size a terminal grid can actually be driven at. Both floors are there to stop vt100 doing
/// unsigned arithmetic that goes below zero — which panics in debug, wraps to 65535 in release,
/// and either way used to take the whole editor and every shell in it down:
///
/// * **rows ≥ 2** — `Parser::new` sets its scroll region to `rows - 1`; and with a single row
///   every line wrap scrolls, so `col_wrap` runs `prev_pos.row -= scrolled` on row 0.
/// * **cols ≥ 2** — `col_wrap` computes `cols - width`, and `width` is 2 for a double-width
///   character, so one CJK glyph or emoji arriving in a one-column pane is enough.
///
/// The size offered when a terminal is opened or resized comes from the window, which goes
/// degenerate while a frame is collapsed or dragged small, so the floors are applied here rather
/// than trusted from the caller. A 2x2 pane is unusable, but it is *inert* — the point is only
/// that nothing can reach vt100 that makes it subtract past zero.
fn buildable_size(rows: u16, cols: u16) -> (u16, u16) {
    (rows.max(2), cols.max(2))
}

impl TerminalPanel {
    pub fn new(rows: u16, cols: u16, cwd: &Path) -> Result<Self> {
        Self::with_startup(rows, cols, cwd, None)
    }

    /// Spawns a shell, optionally with the command this pane exists to run.
    ///
    /// Nothing is written into the pty here. Queueing input before the shell has read a byte is
    /// a bet on ordering that a shell with its own line editor (zsh, fish) loses: it takes over
    /// the tty part-way through and reads whatever is already waiting as raw typing, so a
    /// carriage return goes missing and two lines land as one — the `clearclaude` this used to
    /// produce. The command goes through `pending_command` like every other injected line, typed
    /// once the shell is genuinely at a prompt. See `flush_pending`.
    pub fn with_startup(rows: u16, cols: u16, cwd: &Path, startup: Option<&str>) -> Result<Self> {
        let (rows, cols) = buildable_size(rows, cols);
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new(default_shell());
        cmd.cwd(cwd);
        // Marks the shell as running inside CleeCode, so a user's rc can skip heavy startup
        // work (e.g. `fastfetch`) here — `if not set -q CLEECODE; fastfetch; end` in fish.
        cmd.env("CLEECODE", "1");
        // Where an interpreter started in this pane should publish its workspace, and where to
        // find the code that does it. Set on every shell, for both languages: a shell that
        // starts neither carries a few unread names, which costs nothing, and the alternative is
        // guessing at spawn time what the user is about to type. Nothing is written to their
        // home directory — the hook lives entirely in the environment.
        for (key, value) in crate::wsnap::shell_env(
            &crate::wsnap::snapshot_dir(),
            next_pane_id(),
            &crate::assets::octave_lib(),
            &crate::assets::python_lib(),
        ) {
            cmd.env(key, value);
        }
        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader()?;
        let writer = Arc::new(Mutex::new(pair.master.take_writer()?));

        let parser =
            Arc::new(Mutex::new(vt100::Parser::new(rows, cols, SCROLLBACK_LEN.load(Ordering::Relaxed))));
        let parser_clone = Arc::clone(&parser);
        let exited = Arc::new(AtomicBool::new(false));
        let exited_clone = Arc::clone(&exited);

        let spawn = Instant::now();
        let last_output_ms = Arc::new(AtomicU64::new(0));
        let produced_output = Arc::new(AtomicBool::new(false));
        let last_output_clone = Arc::clone(&last_output_ms);
        let produced_clone = Arc::clone(&produced_output);
        let writer_clone = Arc::clone(&writer);

        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            let mut scanner = CsiScanner::default();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let data = &buf[..n];
                        {
                            let mut p = lock_poisoned(&parser_clone);
                            process_anchored(&mut p, data);
                        }
                        last_output_clone.store(spawn.elapsed().as_millis() as u64, Ordering::Relaxed);
                        produced_clone.store(true, Ordering::Relaxed);

                        // Answer terminal capability/status queries so probing programs
                        // (fastfetch, vim, etc.) don't stall for seconds waiting for a reply.
                        let mut queries = Vec::new();
                        scanner.feed(data, &mut queries);
                        if !queries.is_empty() {
                            let (crow, ccol) = {
                                let p = lock_poisoned(&parser_clone);
                                p.screen().cursor_position()
                            };
                            if let Ok(mut w) = writer_clone.lock() {
                                for q in &queries {
                                    let resp = q.response(crow, ccol);
                                    let _ = w.write_all(&resp);
                                }
                                let _ = w.flush();
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
            exited_clone.store(true, Ordering::Relaxed);
        });

        Ok(TerminalPanel {
            master: pair.master,
            writer,
            child,
            parser,
            rows,
            cols,
            exited,
            spawn,
            last_output_ms,
            produced_output,
            revealed: false,
            selection: None,
            name: None,
            startup_command: startup.map(str::to_string),
            pending_command: startup.map(str::to_string),
            cleared: false,
            last_scroll: None,
        })
    }

    /// Holds a command to be typed into the shell once it is genuinely reading.
    ///
    /// Nothing is ever written into a pty before the shell has read from it: the input buffer
    /// looks like it would keep a line safe until the shell got there, but a shell with its own
    /// line editor (zsh, fish) takes over the tty part-way through and reads what is already
    /// queued as raw typing, so a carriage return is swallowed and two lines land as one —
    /// `clearclaude`. Waiting for the prompt is the only reliable answer, so the command sits
    /// here until `flush_pending`.
    /// Types one line at this shell's prompt, now.
    ///
    /// Onto an empty line, always: whatever the line editor was holding is cleared first. That
    /// is not a precaution, it is the fix for a bug this file already carries a note about —
    /// a command written onto a line that was not empty arrives glued to what was there, and
    /// `clear` plus `claude` once became `clearclaude`. Anything CleeCode types at a prompt goes
    /// through here for that reason.
    pub fn type_line(&mut self, command: &str) {
        let bytes = typed_line(command);
        self.write_input(&bytes);
    }

    pub fn run_command(&mut self, command: &str) {
        self.pending_command = Some(command.to_string());
    }

    /// Scrubs the startup banner and types the held command, once the shell has settled. Does
    /// nothing until the shell is ready, and each part only ever fires once.
    ///
    /// The shell has been quiet for a quarter of a second, so there is a prompt: a form feed is
    /// what a prompt understands as "clear and redraw yourself". Every interactive shell binds
    /// it, and unlike running `clear` it leaves nothing behind in the history. It also has to
    /// come *before* the command rather than instead of it — a fastfetch that the rc prints
    /// after the pane was spawned survives otherwise, whether or not this pane has a command.
    ///
    /// The command itself goes in through `typed_line`, which empties the line first: whatever
    /// the editor happens to be holding — a leftover from the rc, half a word someone typed
    /// into a pane while it was still starting — goes, so the command can never be glued onto
    /// the end of something else.
    pub fn flush_pending(&mut self) {
        if !self.is_ready() {
            return;
        }
        if !self.cleared {
            self.cleared = true;
            self.write_input(b"\x0c");
        }
        if let Some(command) = self.pending_command.take() {
            self.write_input(&typed_line(&command));
        }
    }

    /// Whether the pane should be shown yet. Stays hidden during the shell's startup
    /// (banner/rc output) and reveals once output has been quiet for `STARTUP_IDLE`, which is
    /// also when `flush_pending` scrubs the banner and leaves a clean prompt.
    /// Latches, so it only ever transitions hidden -> shown once.
    pub fn is_ready(&mut self) -> bool {
        if self.revealed {
            return true;
        }
        let elapsed = self.spawn.elapsed();
        let produced = self.produced_output.load(Ordering::Relaxed);
        let idle = elapsed.saturating_sub(Duration::from_millis(self.last_output_ms.load(Ordering::Relaxed)));
        if elapsed >= STARTUP_MAX || (produced && idle >= STARTUP_IDLE) {
            self.revealed = true;
        }
        self.revealed
    }

    pub fn write_input(&mut self, bytes: &[u8]) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(bytes);
            let _ = w.flush();
        }
    }

    /// pid of the shell process running in this pane, used for best-effort ssh-session detection.
    pub fn child_pid(&self) -> Option<u32> {
        self.child.process_id()
    }

    /// Starts a selection at `cell`, discarding any previous one.
    pub fn begin_selection(&mut self, cell: (u16, u16)) {
        self.selection = Some(TermSelection::new(self.clamp_cell(cell)));
    }

    /// Moves the loose end of the selection, starting one at the terminal's cursor if there
    /// isn't one yet — which is what keyboard selection needs.
    pub fn extend_selection(&mut self, cell: (u16, u16)) {
        let cell = self.clamp_cell(cell);
        match &mut self.selection {
            Some(selection) => selection.cursor = cell,
            None => self.selection = Some(TermSelection::new(cell)),
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// Where the terminal's own cursor is, as the starting point for keyboard selection.
    pub fn cursor_cell(&self) -> (u16, u16) {
        self.parser.lock().map(|p| p.screen().cursor_position()).unwrap_or((0, 0))
    }

    /// The selected text, or `None` when nothing is selected.
    pub fn selection_text(&self) -> Option<String> {
        let selection = self.selection?;
        let parser = self.parser.lock().ok()?;
        let screen = parser.screen();
        let (_, cols) = screen.size();
        Some(selected_text(selection, cols, |row, col| {
            screen.cell(row, col).map(|c| c.contents().to_string()).unwrap_or_default()
        }))
    }

    /// The text of one row as it is on screen, trailing blanks trimmed.
    ///
    /// What a double-click reads before deciding whether the line names a file. Taken from the
    /// screen rather than from the raw output, so a wrapped line reads the way it looks and the
    /// escape sequences that coloured it are already gone.
    pub fn row_text(&self, row: u16) -> Option<String> {
        let parser = self.parser.lock().ok()?;
        let screen = parser.screen();
        let (rows, cols) = screen.size();
        if row >= rows {
            return None;
        }
        let mut text = String::new();
        for col in 0..cols {
            match screen.cell(row, col) {
                Some(cell) => text.push_str(cell.contents()),
                None => text.push(' '),
            }
        }
        Some(text.trim_end().to_string())
    }

    /// How many lines of this shell's output have scrolled off and are still held.
    pub fn scrollback_lines(&self) -> usize {
        held_lines(&mut lock_poisoned(&self.parser))
    }

    /// How far back through those lines the pane is currently looking; 0 is the live output.
    pub fn scrollback_offset(&self) -> usize {
        lock_poisoned(&self.parser).screen().scrollback()
    }

    /// Moves the view through the history. `delta` follows the wheel's sign — negative goes back
    /// in time, positive returns towards the live output — so callers don't have to flip it
    /// against vt100's own count-backwards offset. Both ends clamp.
    pub fn scroll_by(&mut self, delta: isize) {
        {
            let mut parser = lock_poisoned(&self.parser);
            let screen = parser.screen_mut();
            let offset = screen.scrollback() as isize;
            screen.set_scrollback((offset - delta).max(0) as usize);
        }
        self.last_scroll = Some(Instant::now());
    }

    /// Parks the view a given number of lines back from the live output, for a scrollbar being
    /// dragged to an absolute position rather than nudged. Clamped by vt100 at the far end.
    pub fn scroll_to_offset(&mut self, offset: usize) {
        lock_poisoned(&self.parser).screen_mut().set_scrollback(offset);
        self.last_scroll = Some(Instant::now());
    }

    /// Back to the live output. Typing does this: a keystroke is going to the shell, and having
    /// its answer arrive somewhere off-screen is worse than losing your place in the history.
    ///
    /// Deliberately not counted as scrolling: it is a side effect of typing, and flashing the
    /// scrollbar up on every keystroke is exactly the twitch the fade-out exists to avoid.
    pub fn scroll_to_bottom(&self) {
        lock_poisoned(&self.parser).screen_mut().set_scrollback(0);
    }

    /// Whether the history was scrolled within `window`, for deciding if the scrollbar still
    /// has a reason to be on screen.
    pub fn scrolled_within(&self, window: Duration) -> bool {
        self.last_scroll.is_some_and(|at| at.elapsed() < window)
    }

    /// Whether a full-screen program — vim, less, any pager — is in charge of the screen. vt100
    /// gives the alternate screen a zero-length scrollback of its own, so there is nothing to
    /// scroll back to and the wheel belongs to that program instead of to us.
    pub fn alternate_screen(&self) -> bool {
        lock_poisoned(&self.parser).screen().alternate_screen()
    }

    /// A wheel notch as the program in this pane asked to be told about it, at cell `(row, col)`
    /// counted from the pane's top-left. `None` when it never asked, in which case the notch is
    /// ours to spend on the scrollback.
    ///
    /// This is what a terminal emulator does and what this did not: Claude Code, htop, a
    /// mouse-mode vim all turn mouse reporting on, scroll their own view, and draw their own
    /// scrollbar. Dropping the notch — which is what happened here for anything on the alternate
    /// screen — left them no way to be scrolled at all, from the wheel or from anywhere else,
    /// while our own history stayed empty because a full-screen program never fills one.
    pub fn wheel_report(&self, up: bool, row: u16, col: u16) -> Option<Vec<u8>> {
        let screen = lock_poisoned(&self.parser);
        let screen = screen.screen();
        if screen.mouse_protocol_mode() == vt100::MouseProtocolMode::None {
            return None;
        }
        Some(encode_wheel(screen.mouse_protocol_encoding(), up, row, col))
    }

    /// Keeps a cell inside the screen, so a drag that leaves the pane still selects up to the
    /// edge instead of being ignored.
    fn clamp_cell(&self, (row, col): (u16, u16)) -> (u16, u16) {
        (row.min(self.rows.saturating_sub(1)), col.min(self.cols.saturating_sub(1)))
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        // A collapsed or mid-drag pane offers no size worth adopting: reflowing the shell down to
        // the minimum would mangle its output for a frame and gain nothing, so it is left alone.
        if rows == 0 || cols == 0 {
            return;
        }
        // Anything that does get through still has to be a size vt100 can survive.
        let (rows, cols) = buildable_size(rows, cols);
        if rows == self.rows && cols == self.cols {
            return;
        }
        self.rows = rows;
        self.cols = cols;
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        lock_poisoned(&self.parser).screen_mut().set_size(rows, cols);
    }
}

/// Translate a crossterm key event into raw bytes to send to the pty.
pub fn key_to_bytes(key: crossterm::event::KeyEvent) -> Vec<u8> {
    use crossterm::event::{KeyCode, KeyModifiers};

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    // Meta is sent the way terminals have always sent it: ESC, then the key. Without this an
    // Alt chord that CleeCode does not claim reached the shell as a bare letter, so Alt+D in
    // readline (delete-word) typed a "d" instead.
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let meta = |mut bytes: Vec<u8>| {
        if alt {
            bytes.insert(0, 0x1b);
        }
        bytes
    };

    match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                let upper = c.to_ascii_uppercase();
                if upper.is_ascii_alphabetic() {
                    let byte = (upper as u8) - b'A' + 1;
                    return meta(vec![byte]);
                }
                meta(vec![c as u8])
            } else {
                let mut buf = [0u8; 4];
                meta(c.encode_utf8(&mut buf).as_bytes().to_vec())
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => vec![0x1b, b'[', b'Z'],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => vec![0x1b, b'[', b'A'],
        KeyCode::Down => vec![0x1b, b'[', b'B'],
        KeyCode::Right => vec![0x1b, b'[', b'C'],
        KeyCode::Left => vec![0x1b, b'[', b'D'],
        KeyCode::Home => vec![0x1b, b'[', b'H'],
        KeyCode::End => vec![0x1b, b'[', b'F'],
        KeyCode::PageUp => vec![0x1b, b'[', b'5', b'~'],
        KeyCode::PageDown => vec![0x1b, b'[', b'6', b'~'],
        KeyCode::Delete => vec![0x1b, b'[', b'3', b'~'],
        _ => Vec::new(),
    }
}

/// One terminal "window" — a tiled pane in the layout — holding one or more tabbed shells, of
/// which `active` is the one on screen. A window with a single tab looks exactly like the old
/// flat terminal, so the tab strip only appears once a second tab is opened.
pub struct TerminalWindow {
    pub tabs: Vec<TerminalPanel>,
    pub active: usize,
    /// Relative size among the tiled terminal windows; equal by default, shifted when the seam to
    /// a neighbour is dragged. A large base value leaves room for fine adjustment.
    pub weight: u16,
}

/// Default window weight: large enough that a drag can nudge the split in fine steps.
pub const TERMINAL_WEIGHT_DEFAULT: u16 = 1000;

impl TerminalWindow {
    /// A fresh window with a single shell.
    pub fn new(rows: u16, cols: u16, cwd: &Path) -> Result<Self> {
        Ok(TerminalWindow {
            tabs: vec![TerminalPanel::new(rows, cols, cwd)?],
            active: 0,
            weight: TERMINAL_WEIGHT_DEFAULT,
        })
    }

    /// Clamped rather than indexed raw: `active` can arrive from a hand-edited workspace file
    /// or lag behind a tab that has just closed, and this is read on every frame. Falling back
    /// to the last tab shows the wrong shell for one frame; indexing past the end would take the
    /// whole editor down, along with every shell in it.
    pub fn active_tab(&self) -> &TerminalPanel {
        let idx = self.active.min(self.tabs.len().saturating_sub(1));
        &self.tabs[idx]
    }

    pub fn active_tab_mut(&mut self) -> &mut TerminalPanel {
        let idx = self.active.min(self.tabs.len().saturating_sub(1));
        &mut self.tabs[idx]
    }

    /// Adds a tab and focuses it — opening a tab is always to switch to it.
    pub fn add_tab(&mut self, panel: TerminalPanel) {
        self.tabs.push(panel);
        self.active = self.tabs.len() - 1;
    }

    pub fn cycle_tab(&mut self, forward: bool) {
        let n = self.tabs.len();
        if n <= 1 {
            return;
        }
        self.active = if forward { (self.active + 1) % n } else { (self.active + n - 1) % n };
    }

    /// Drops tabs whose shell has exited, keeping `active` in range. Returns whether any tab is
    /// left — an emptied window is removed by the caller.
    pub fn reap_exited(&mut self) -> bool {
        self.tabs.retain(|t| !t.exited.load(Ordering::Relaxed));
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        }
        !self.tabs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4x5 screen whose cell contents encode their position, with a blank column so
    /// trailing-space trimming is exercised.
    fn grid(row: u16, col: u16) -> String {
        if col == 4 { String::new() } else { format!("{row}{col}") }
    }

    /// The regression behind "CleeCode just closed on me, with a session running in a pane":
    /// a terminal opened at zero height panicked inside vt100 and killed the process, shells
    /// and all. The floor is asserted against the real parser, not just the arithmetic, so the
    /// test still fails if vt100 ever gets stricter about what it will accept.
    #[test]
    fn a_terminal_is_never_built_at_a_size_vt100_cannot_take() {
        for (rows, cols) in [(0, 0), (0, 80), (24, 0), (1, 1), (1, 0), (2, 2), (1, 200)] {
            let (r, c) = buildable_size(rows, cols);
            assert!(r >= 2 && c >= 2, "{rows}x{cols} must be floored, got {r}x{c}");

            // Built, then driven: creation trips `rows - 1`, and feeding a double-width glyph
            // trips `cols - width` in `col_wrap`. Both are exercised against the real parser, so
            // the test still fails if vt100 changes what it will tolerate.
            let mut parser = vt100::Parser::new(r, c, 0);
            assert_eq!(parser.screen().size(), (r, c));
            parser.process("日本語テキスト🙂".as_bytes());
            parser.screen_mut().set_size(r, c);
            parser.process("more text that has to wrap somewhere".as_bytes());
        }
        // Anything already usable is passed through untouched.
        assert_eq!(buildable_size(40, 120), (40, 120));
    }

    /// The regression behind "my startup command came out stuck to the `clear`": `clear` and
    /// the command were both handed to the shell as text, and one of them lost its carriage
    /// return on the way in, so `claude` arrived as `clearclaude`. Two things prevent it now,
    /// and this pins both: exactly one line is ever typed, and it is typed onto an empty one.
    #[test]
    fn one_line_goes_in_and_it_is_the_command() {
        let bytes = typed_line("npm run dev");
        assert!(bytes.starts_with(LINE_RESET), "the line has to be cleared before anything is typed");
        assert_eq!(bytes.iter().filter(|b| **b == b'\r').count(), 1, "one line, submitted once");
        assert!(bytes.ends_with(b"npm run dev\r"));

        // A command someone pasted a newline into is still one line, not two.
        assert_eq!(typed_line("a\nb").iter().filter(|b| **b == b'\r').count(), 1);
        assert!(typed_line("a\nb").ends_with(b"a b\r"));
    }

    /// The other half of it: a shell is handed nothing at all until it is at a prompt. Writing
    /// into the pty at spawn is what put a line in front of the shell's own line editor in the
    /// first place, so the command has to be waiting in `pending_command` instead.
    #[test]
    fn a_shell_is_typed_into_only_once_it_is_ready() {
        let mut panel = TerminalPanel::with_startup(24, 80, Path::new("/"), Some("echo hello"))
            .expect("a shell must be spawnable to test one");
        assert_eq!(panel.pending_command.as_deref(), Some("echo hello"));

        // Nothing has been typed yet, and nothing will be until the shell settles.
        panel.revealed = false;
        panel.produced_output.store(false, Ordering::Relaxed);
        panel.flush_pending();
        assert_eq!(panel.pending_command.as_deref(), Some("echo hello"), "not before the prompt");
        assert!(!panel.cleared);
    }

    /// And the whole of it against a real shell, because the failure lived in the line editor
    /// rather than in our arithmetic: spawn one, give it a startup command, ask what ended up
    /// on its screen.
    #[cfg(unix)]
    #[test]
    fn a_startup_command_is_typed_onto_a_line_of_its_own() {
        let marker = "clee-startup-ok";
        let mut panel =
            TerminalPanel::with_startup(24, 80, Path::new("/"), Some(&format!("echo {marker}")))
                .expect("a shell must be spawnable to test one");

        // Generous: this waits out the shell's own startup (up to `STARTUP_MAX`), then the
        // command running. It returns as soon as the output shows up, so the wait is only ever
        // paid in full by a failure.
        let deadline = Instant::now() + STARTUP_MAX + Duration::from_secs(8);
        let mut screen = String::new();
        while Instant::now() < deadline {
            panel.flush_pending();
            screen = lock_poisoned(&panel.parser).screen().contents();
            if screen.lines().any(|l| l.trim() == marker) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        // The echo ran, so the command reached the shell as a command and not as a fragment
        // glued to the end of something else.
        assert!(
            screen.lines().any(|l| l.trim() == marker),
            "the startup command never ran; the shell's screen was:\n{screen}"
        );
        // And nothing was clearing the pane by typing `clear` at the shell, which is what the
        // command used to be concatenated with.
        assert!(
            !screen.contains("clear"),
            "no `clear` should ever be typed into the shell; screen:\n{screen}"
        );
    }

    /// What a program that asked for the mouse expects a wheel notch to look like. Coordinates
    /// are one-based and counted from the pane, which is the only screen the program has.
    #[test]
    fn a_wheel_notch_is_reported_the_way_the_program_asked() {
        use vt100::MouseProtocolEncoding::{Default, Sgr, Utf8};
        // SGR: button 64 up, 65 down, and a press (M) — there is no letting go of a wheel.
        assert_eq!(encode_wheel(Sgr, true, 0, 0), b"\x1b[<64;1;1M".to_vec());
        assert_eq!(encode_wheel(Sgr, false, 11, 29), b"\x1b[<65;30;12M".to_vec());
        // The old encodings offset every number by 32 and spend one byte on each.
        assert_eq!(encode_wheel(Default, true, 0, 0), vec![0x1b, b'[', b'M', 96, 33, 33]);
        assert_eq!(encode_wheel(Default, false, 11, 29), vec![0x1b, b'[', b'M', 97, 62, 44]);
        // Past what one byte can hold, the column is pinned rather than wrapped round to a
        // position on the other side of the pane.
        assert_eq!(encode_wheel(Default, true, 0, 400)[4], 255);
        // UTF-8 puts the same numbers through as characters, so the limit does not apply.
        assert_eq!(encode_wheel(Utf8, true, 0, 0), b"\x1b[M`!!".to_vec());
    }

    /// The regression: a program that turns mouse reporting on scrolls a view of its own — the
    /// wheel has to reach it. It did not, so anything on the alternate screen (Claude Code,
    /// htop, a mouse-mode vim) could not be scrolled at all: our own history is empty there, and
    /// the notch was dropped rather than delivered.
    #[cfg(unix)]
    #[test]
    fn a_program_that_asked_for_the_mouse_gets_the_wheel() {
        let mut panel = TerminalPanel::with_startup(
            10,
            40,
            Path::new("/"),
            // What a full-screen program sends on startup: alternate screen, mouse reporting,
            // SGR encoding. `sleep` keeps it in that state while this looks at it.
            Some("printf '\\033[?1049h\\033[?1000h\\033[?1006h' && sleep 30"),
        )
        .expect("a shell must be spawnable to test one");

        let deadline = Instant::now() + STARTUP_MAX;
        while Instant::now() < deadline {
            panel.flush_pending();
            if panel.wheel_report(true, 0, 0).is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(
            panel.wheel_report(true, 4, 9).as_deref(),
            Some(&b"\x1b[<64;10;5M"[..]),
            "the wheel must be handed to a program that asked for it"
        );
        assert!(panel.alternate_screen(), "and that is exactly where we have no history of our own");
    }

    /// What a shell actually needs to receive. Ctrl+letter is the control byte, and an Alt chord
    /// carries the ESC prefix — without it Alt+D reached readline as a literal "d".
    #[test]
    fn keys_reach_the_shell_the_way_a_terminal_sends_them() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let press = |code, mods| key_to_bytes(KeyEvent::new(code, mods));

        // Ctrl+J is LF and Ctrl+I is Tab — the two the editor used to swallow.
        assert_eq!(press(KeyCode::Char('j'), KeyModifiers::CONTROL), vec![0x0a]);
        assert_eq!(press(KeyCode::Char('i'), KeyModifiers::CONTROL), vec![0x09]);
        assert_eq!(press(KeyCode::Char('c'), KeyModifiers::CONTROL), vec![0x03]);

        assert_eq!(press(KeyCode::Char('d'), KeyModifiers::ALT), vec![0x1b, b'd']);
        assert_eq!(press(KeyCode::Char('b'), KeyModifiers::ALT), vec![0x1b, b'b']);
        // Both modifiers at once: ESC then the control byte.
        assert_eq!(
            press(KeyCode::Char('x'), KeyModifiers::ALT | KeyModifiers::CONTROL),
            vec![0x1b, 0x18]
        );
        // Plain text is untouched.
        assert_eq!(press(KeyCode::Char('d'), KeyModifiers::NONE), vec![b'd']);
        assert_eq!(press(KeyCode::Char('è'), KeyModifiers::NONE), "è".as_bytes().to_vec());
    }

    #[test]
    fn selection_flows_like_text_not_as_a_rectangle() {
        let selection = TermSelection { anchor: (0, 2), cursor: (2, 1) };
        // First row: from the anchor to the end. Middle rows: everything.
        assert!(selection.contains(0, 2) && selection.contains(0, 4));
        assert!(!selection.contains(0, 1), "before the start of the first row");
        assert!(selection.contains(1, 0) && selection.contains(1, 4));
        // Last row: up to the cursor only.
        assert!(selection.contains(2, 1));
        assert!(!selection.contains(2, 2), "past the end of the last row");
        assert!(!selection.contains(3, 0), "outside the row range");
    }

    #[test]
    fn selection_is_direction_agnostic() {
        let forward = TermSelection { anchor: (0, 1), cursor: (1, 3) };
        let backward = TermSelection { anchor: (1, 3), cursor: (0, 1) };
        assert_eq!(forward.ordered(), backward.ordered());
        for row in 0..2 {
            for col in 0..5 {
                assert_eq!(forward.contains(row, col), backward.contains(row, col), "at {row},{col}");
            }
        }
    }

    #[test]
    fn selected_text_joins_rows_and_trims_trailing_blanks() {
        // Two full rows: the blank last column must not survive as trailing whitespace.
        let selection = TermSelection { anchor: (0, 0), cursor: (1, 4) };
        assert_eq!(selected_text(selection, 5, grid), "00010203\n10111213");
    }

    #[test]
    fn selected_text_respects_the_partial_first_and_last_rows() {
        let selection = TermSelection { anchor: (0, 2), cursor: (1, 1) };
        assert_eq!(selected_text(selection, 5, grid), "0203\n1011");
    }

    #[test]
    fn selected_text_of_a_single_cell_is_that_cell() {
        let selection = TermSelection::new((1, 2));
        assert_eq!(selected_text(selection, 5, grid), "12");
        // A blank cell yields nothing rather than a stray space.
        assert_eq!(selected_text(TermSelection::new((1, 4)), 5, grid), "");
    }

    #[test]
    fn selected_text_does_not_read_past_the_screen_width() {
        // A stale selection (e.g. the pane was made narrower) must clamp, not panic.
        let selection = TermSelection { anchor: (0, 0), cursor: (0, 99) };
        assert_eq!(selected_text(selection, 5, grid), "00010203");
    }

    fn scan(data: &[u8]) -> Vec<TerminalQuery> {
        let mut s = CsiScanner::default();
        let mut out = Vec::new();
        s.feed(data, &mut out);
        out
    }

    #[test]
    fn detects_dsr_and_da_queries() {
        assert_eq!(scan(b"\x1b[6n"), vec![TerminalQuery::CursorPosition]);
        assert_eq!(scan(b"\x1b[5n"), vec![TerminalQuery::Status]);
        assert_eq!(scan(b"\x1b[c"), vec![TerminalQuery::PrimaryDeviceAttributes]);
        assert_eq!(scan(b"\x1b[0c"), vec![TerminalQuery::PrimaryDeviceAttributes]);
        assert_eq!(scan(b"\x1b[>c"), vec![TerminalQuery::SecondaryDeviceAttributes]);
    }

    #[test]
    fn ignores_non_query_sequences() {
        // Colours (…m) and cursor moves (…H) are not queries.
        assert!(scan(b"\x1b[31mhello\x1b[0m\x1b[2;3H").is_empty());
    }

    #[test]
    fn handles_query_split_across_reads() {
        let mut s = CsiScanner::default();
        let mut out = Vec::new();
        s.feed(b"\x1b[", &mut out);
        s.feed(b"6n", &mut out);
        assert_eq!(out, vec![TerminalQuery::CursorPosition]);
    }

    #[test]
    fn cursor_position_response_is_one_based() {
        assert_eq!(TerminalQuery::CursorPosition.response(0, 0), b"\x1b[1;1R");
        assert_eq!(TerminalQuery::CursorPosition.response(4, 9), b"\x1b[5;10R");
    }

    /// Feeds `count` numbered lines, the way a shell printing output would.
    fn print_lines(parser: &mut vt100::Parser, from: usize, count: usize) {
        for i in from..from + count {
            parser.process(format!("line {i}\r\n").as_bytes());
        }
    }

    /// The top row on screen, which is what "the view has not moved" has to be checked against.
    fn top_row(parser: &vt100::Parser) -> String {
        let (_, cols) = parser.screen().size();
        (0..cols)
            .filter_map(|c| parser.screen().cell(0, c).map(|cell| cell.contents().to_string()))
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    /// vt100 publishes the configured cap and the current offset but not how much is stored, so
    /// the count is asked for by clamping. It has to be exact, and it has to leave no trace:
    /// it runs on every frame that draws a scrollbar and inside the reader thread.
    #[test]
    fn held_lines_counts_what_is_stored_without_moving_the_view() {
        let mut parser = vt100::Parser::new(4, 20, 100);
        assert_eq!(held_lines(&mut parser), 0, "a fresh screen holds nothing");

        print_lines(&mut parser, 0, 10);
        let held = held_lines(&mut parser);
        assert!(held > 0 && held < 100, "10 lines on a 4-row screen push a few off, got {held}");

        // Asking must not disturb where the reader is looking.
        parser.screen_mut().set_scrollback(3);
        assert_eq!(held_lines(&mut parser), held);
        assert_eq!(parser.screen().scrollback(), 3);

        // Past the cap the count stops at the cap, because the oldest lines are dropped.
        let mut small = vt100::Parser::new(4, 20, 5);
        print_lines(&mut small, 0, 200);
        assert_eq!(held_lines(&mut small), 5);
    }

    /// The point of the whole thing: reading back through a build's output while it is still
    /// printing must not drag the page out from under you.
    #[test]
    fn output_does_not_slide_the_view_of_someone_scrolled_back() {
        let mut parser = vt100::Parser::new(4, 20, 100);
        print_lines(&mut parser, 0, 40);

        parser.screen_mut().set_scrollback(10);
        let parked_on = top_row(&parser);
        assert!(parked_on.starts_with("line "), "expected to be parked on output, saw {parked_on:?}");

        // The shell keeps printing while we read.
        for batch in 0..5 {
            process_anchored(&mut parser, format!("more {batch}\r\n").as_bytes());
            assert_eq!(top_row(&parser), parked_on, "the view moved while output arrived");
        }
        // And the offset really did track the new lines rather than staying put.
        assert_eq!(parser.screen().scrollback(), 15);
    }

    /// At the live end the view is *meant* to follow the output, so anchoring must keep out of
    /// the way — this is the common case, every terminal that nobody has scrolled.
    #[test]
    fn output_still_follows_when_not_scrolled_back() {
        let mut parser = vt100::Parser::new(4, 20, 100);
        print_lines(&mut parser, 0, 40);
        assert_eq!(parser.screen().scrollback(), 0);

        process_anchored(&mut parser, b"newest\r\n");
        assert_eq!(parser.screen().scrollback(), 0, "the live view must stay at the bottom");
    }

    /// A full buffer genuinely loses its oldest lines, so the view cannot be held any longer.
    /// What matters is that it degrades instead of underflowing — `offset + pushed` is computed
    /// from a subtraction that would panic in debug if it were allowed to go negative.
    #[test]
    fn a_full_buffer_drifts_instead_of_underflowing() {
        let mut parser = vt100::Parser::new(4, 20, 5);
        print_lines(&mut parser, 0, 100);
        parser.screen_mut().set_scrollback(5);
        for _ in 0..20 {
            process_anchored(&mut parser, b"flood\r\n");
        }
        assert_eq!(parser.screen().scrollback(), 5, "clamped to what is still held");
    }

    /// A full-screen program gets a scrollback-free grid from vt100, which is what makes
    /// "the wheel belongs to vim, not to us" a fact about the data rather than a guess.
    #[test]
    fn the_alternate_screen_holds_no_history() {
        let mut parser = vt100::Parser::new(4, 20, 100);
        print_lines(&mut parser, 0, 40);
        assert!(held_lines(&mut parser) > 0);

        parser.process(b"\x1b[?1049h"); // enter the alternate screen, as vim/less do
        assert!(parser.screen().alternate_screen());
        print_lines(&mut parser, 0, 40);
        assert_eq!(held_lines(&mut parser), 0);

        parser.process(b"\x1b[?1049l"); // and back out
        assert!(!parser.screen().alternate_screen());
        assert!(held_lines(&mut parser) > 0, "the real screen keeps its history");
    }
}

