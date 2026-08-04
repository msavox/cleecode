use anyhow::Result;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long the shell's output must be quiet before we consider startup finished and
/// reveal the pane. The injected `clear` runs as the shell's first interactive command
/// (after its rc/banner), so by the time output idles the screen is already clean.
const STARTUP_IDLE: Duration = Duration::from_millis(250);
/// Safety cap: reveal no matter what after this long, so a shell that keeps its pty busy
/// (e.g. an rc that launches a long-running program) can't stay blank forever.
const STARTUP_MAX: Duration = Duration::from_secs(12);

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

impl TerminalPanel {
    pub fn new(rows: u16, cols: u16, cwd: &Path) -> Result<Self> {
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
        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader()?;
        let writer = Arc::new(Mutex::new(pair.master.take_writer()?));
        // Queued in the pty's input buffer; the shell consumes it as soon as it starts
        // reading interactively, clearing any startup banner (e.g. fastfetch/neofetch).
        let clear = if cfg!(windows) { b"cls\r".as_slice() } else { b"clear\r".as_slice() };
        if let Ok(mut w) = writer.lock() {
            let _ = w.write_all(clear);
            let _ = w.flush();
        }

        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 0)));
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
                            let mut p = parser_clone.lock().unwrap();
                            p.process(data);
                        }
                        last_output_clone.store(spawn.elapsed().as_millis() as u64, Ordering::Relaxed);
                        produced_clone.store(true, Ordering::Relaxed);

                        // Answer terminal capability/status queries so probing programs
                        // (fastfetch, vim, etc.) don't stall for seconds waiting for a reply.
                        let mut queries = Vec::new();
                        scanner.feed(data, &mut queries);
                        if !queries.is_empty() {
                            let (crow, ccol) = {
                                let p = parser_clone.lock().unwrap();
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
        })
    }

    /// Whether the pane should be shown yet. Stays hidden during the shell's startup
    /// (banner/rc output) and reveals once output has been quiet for `STARTUP_IDLE` — by
    /// which point the injected `clear` has scrubbed the banner and left a clean prompt.
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

    /// Keeps a cell inside the screen, so a drag that leaves the pane still selects up to the
    /// edge instead of being ignored.
    fn clamp_cell(&self, (row, col): (u16, u16)) -> (u16, u16) {
        (row.min(self.rows.saturating_sub(1)), col.min(self.cols.saturating_sub(1)))
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        if rows == 0 || cols == 0 || (rows == self.rows && cols == self.cols) {
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
        self.parser.lock().unwrap().screen_mut().set_size(rows, cols);
    }
}

/// Translate a crossterm key event into raw bytes to send to the pty.
pub fn key_to_bytes(key: crossterm::event::KeyEvent) -> Vec<u8> {
    use crossterm::event::{KeyCode, KeyModifiers};

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                let upper = c.to_ascii_uppercase();
                if upper.is_ascii_alphabetic() {
                    let byte = (upper as u8) - b'A' + 1;
                    return vec![byte];
                }
                vec![c as u8]
            } else {
                let mut buf = [0u8; 4];
                c.encode_utf8(&mut buf).as_bytes().to_vec()
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

    pub fn active_tab(&self) -> &TerminalPanel {
        &self.tabs[self.active]
    }

    pub fn active_tab_mut(&mut self) -> &mut TerminalPanel {
        &mut self.tabs[self.active]
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
}
