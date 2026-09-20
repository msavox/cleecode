use anyhow::Result;
use ratatui::layout::Rect;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
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

/// How much input a pane will hold on the way to its shell before it starts throwing writes
/// away.
///
/// There has to be a limit, because a pty stops accepting bytes as soon as nobody is reading the
/// other end — a job suspended with Ctrl+Z, a program wedged on something else — and a write into
/// a full one blocks until that changes, which may be never. Doing that on the UI thread froze
/// the whole editor, every pane in it, over one paste into one dead shell.
///
/// 256 messages is far more than a session ever queues while a shell is alive: keystrokes are one
/// message each and a paste is one message however long it is, and a reading shell drains them as
/// fast as they arrive. Reaching the cap therefore means the shell is not reading, and the honest
/// answer then is to lose the input rather than the editor.
const WRITE_QUEUE: usize = 256;

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
    /// The way in to the shell. Everything CleeCode sends — keystrokes, pastes, mouse reports,
    /// the startup command, and the reader thread's answers to terminal queries — is handed to
    /// this queue and written by a thread of this pane's own. Nobody else ever touches the pty's
    /// write end, which is what keeps a shell that has stopped reading from taking the editor
    /// down with it. See `WRITE_QUEUE` and `write_input`.
    input: SyncSender<Vec<u8>>,
    child: Box<dyn Child + Send + Sync>,
    pub parser: Arc<Mutex<vt100::Parser>>,
    pub rows: u16,
    pub cols: u16,
    /// Set once the shell's end of the pty closes (process exited, whether via `exit`,
    /// Ctrl+D, a command that dies on Ctrl+C, a crash, or an ssh disconnect).
    pub exited: Arc<AtomicBool>,
    /// Raised when the pane is torn down, so the reader thread mutes: it keeps draining the
    /// master and throws the bytes away instead of parsing them into a screen that will never
    /// be drawn again.
    ///
    /// Draining, not stopping, is the load-bearing half. A dying shell can have output still in
    /// the pty — fish repaints its prompt on the way out — and the kernel will not let its exit
    /// finish until someone reads it; a reader that stopped early left `shutdown`'s `wait()`
    /// parked on a process forever stuck mid-exit, which was the editor frozen at quit. The
    /// thread ends at EOF, when the last slave fd is truly gone — the way a real terminal reads
    /// until hangup.
    stop: Arc<AtomicBool>,
    /// Counts the times the reader thread has fed the parser. Shared with that thread the same
    /// way the stop flag is, and only ever compared with its own previous value: what it answers
    /// is "has this screen moved since you last looked", which is what lets the frame loop leave
    /// the screen alone while every shell in the window is quiet.
    generation: Arc<AtomicU64>,
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
    /// A line to put at the prompt without running it, held back the same way. See
    /// `queue_line_unsent`.
    pending_unsent: Option<String>,
    /// Whether the banner has been scrubbed since this shell started. See `flush_pending`.
    cleared: bool,
    /// Where the shell in this pane says it is. Written by the reader thread when OSC 7 arrives
    /// and read by the sidebar's shell half; `None` until something has said, which is the honest
    /// answer for a shell nobody has heard from yet.
    cwd: Arc<Mutex<Option<PathBuf>>>,
    /// Whether this shell reports its own folder. Until it has once, the folder is asked of the
    /// process table instead — and after it has, it never is again: a shell that reports is both
    /// exact and free, and the question put to the operating system is neither.
    reports_cwd: Arc<AtomicBool>,
    /// When the history was last scrolled through. The scrollbar is a hint rather than
    /// furniture: it appears while it is being used and fades back out, so an idle pane stays
    /// all output and no chrome.
    last_scroll: Option<Instant>,
    /// Pictures the program in this pane has drawn, posted by the reader thread and taken up
    /// by the next frame. A queue rather than the pictures themselves because the decoding
    /// happens on that thread and the drawing on this one, and the two must not share a list
    /// they both edit.
    graphics: Arc<Mutex<Vec<crate::pane_graphics::Event>>>,
    /// The pictures currently on this pane, each with the drawing it was last built into.
    images: Vec<PaneImage>,
    /// Pictures the reader thread has handed over that this pane has not yet taken up.
    ///
    /// Shared with the reader thread's `Pace`, which stops decoding while it is anything but
    /// zero — a picture nobody has drawn is one the next picture would only replace, so the
    /// next picture is not worth decoding. That is what paces a film to the screen rather than
    /// to a number; see `pane_graphics::Pace`.
    graphics_queued: Arc<AtomicUsize>,
    /// The drawing of the last picture to leave the pane, kept so the next one can have it.
    ///
    /// This is what makes a film possible rather than merely decodable. A drawing carries the
    /// *identity* the host terminal knows the picture by, and a fresh one is a fresh picture
    /// transmitted under a fresh name — so a player, which is a new picture thirty times a
    /// second at the same spot, would hand the host thirty new pictures a second and never tell
    /// it about the twenty-nine it no longer needs. Handing the old drawing on instead means
    /// every frame arrives under the name the one before it had, which is the protocol's own
    /// way of saying "this replaces that".
    spare: Option<Box<ratatui_image::protocol::StatefulProtocol>>,
    /// Something a program tried to draw and could not be. Read and cleared by the app, which
    /// puts it on the status line — a picture that cannot appear should say why rather than
    /// leave a pane looking broken.
    pub graphics_note: Option<GraphicsNote>,
}

/// One picture on a pane, and the drawing of it.
struct PaneImage {
    placement: crate::pane_graphics::Placement,
    /// What it is drawn through, built once.
    ///
    /// Once, and not once a frame, for the reason `preview.rs` gives where it does the same
    /// thing: a protocol carries the identity the host terminal knows the picture by, and a
    /// fresh one every frame is a fresh transmission every frame — the picture redrawn on top
    /// of itself ten times a second, which is not a picture, it is a flicker. The resizing to
    /// whatever rectangle the pane offers is the protocol's own job and it caches it.
    drawn: Option<Box<ratatui_image::protocol::StatefulProtocol>>,
}

/// Why a picture a pane asked for is not there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GraphicsNote {
    /// The picture is in a format this does not read, states a size that cannot be meant, or
    /// named a file or a shared-memory segment that was not there to be read. Worth naming
    /// because the fix is on the user's side: a different flag.
    Unsupported,
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

/// The bytes that type `command` onto a cleared prompt and stop there: the line reset, then the
/// command, and no carriage return. One line is put in front of the user, whatever the line
/// editor was holding before, and the decision to run it stays theirs. Any newlines inside the
/// command itself would each submit a line of their own, so they are flattened to spaces.
fn typed_line_unsent(command: &str) -> Vec<u8> {
    let mut bytes = LINE_RESET.to_vec();
    bytes.extend(command.replace(['\r', '\n'], " ").as_bytes());
    bytes
}

/// The bytes that type `command` into a shell and run it: the same cleared line as
/// [`typed_line_unsent`], with the one carriage return that submits it — see `flush_pending`
/// for why each piece is there.
fn typed_line(command: &str) -> Vec<u8> {
    let mut bytes = typed_line_unsent(command);
    bytes.push(b'\r');
    bytes
}

/// What the pointer just did, in the only three shapes the mouse protocols have a word for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MouseAction {
    Press,
    Release,
    /// The pointer moved into another cell with a button held down.
    Drag,
}

/// Button numbers as the protocols count them: left is 0, middle 1, right 2, and the wheel is
/// numbered from 64 as if it were two more buttons that can only ever be pressed.
pub const BUTTON_LEFT: u16 = 0;
pub const BUTTON_MIDDLE: u16 = 1;
pub const BUTTON_RIGHT: u16 = 2;
const BUTTON_WHEEL_UP: u16 = 64;
const BUTTON_WHEEL_DOWN: u16 = 65;

/// A crossterm button as the mouse protocols number it.
pub fn mouse_button_code(button: crossterm::event::MouseButton) -> u16 {
    use crossterm::event::MouseButton;
    match button {
        MouseButton::Left => BUTTON_LEFT,
        MouseButton::Middle => BUTTON_MIDDLE,
        MouseButton::Right => BUTTON_RIGHT,
    }
}

/// One mouse event written the way the program asked to receive it.
///
/// Coordinates are one-based, counted from the top-left of the pane rather than of the window,
/// since the pane is the whole world as far as the program inside it knows. Motion is not a code
/// of its own but a flag on the button being held, which is what the extra 32 is.
///
/// The two encodings that predate SGR pack each number into one byte with an offset of 32, which
/// is why they cannot describe anything past column 223; a program that asked for one of them
/// gets the events it can express and none of the ones it cannot, rather than a byte that would
/// land it somewhere else entirely. They also have a single code for "a button was released" and
/// never say which one it was — only SGR carries that, in the final character. SGR has neither
/// limit and is what anything modern asks for.
fn encode_mouse(
    encoding: vt100::MouseProtocolEncoding,
    button: u16,
    action: MouseAction,
    row: u16,
    col: u16,
) -> Vec<u8> {
    let held = button + if action == MouseAction::Drag { 32 } else { 0 };
    let (row, col) = (row.saturating_add(1), col.saturating_add(1));
    match encoding {
        vt100::MouseProtocolEncoding::Sgr => {
            let end = if action == MouseAction::Release { 'm' } else { 'M' };
            format!("\x1b[<{held};{col};{row}{end}").into_bytes()
        }
        vt100::MouseProtocolEncoding::Utf8 => {
            let code = if action == MouseAction::Release { 3 } else { held };
            let mut out = b"\x1b[M".to_vec();
            let mut push = |n: u32| {
                let mut buf = [0u8; 4];
                out.extend_from_slice(
                    char::from_u32(n + 32).unwrap_or(' ').encode_utf8(&mut buf).as_bytes(),
                );
            };
            push(u32::from(code));
            push(u32::from(col));
            push(u32::from(row));
            out
        }
        vt100::MouseProtocolEncoding::Default => {
            let code = if action == MouseAction::Release { 3 } else { held };
            let byte = |n: u16| u8::try_from(n + 32).unwrap_or(u8::MAX);
            vec![0x1b, b'[', b'M', byte(code), byte(col.min(223)), byte(row.min(223))]
        }
    }
}

/// A wheel notch. Both directions are reported as presses with no release — there is no such
/// thing as letting go of a wheel.
fn encode_wheel(
    encoding: vt100::MouseProtocolEncoding,
    up: bool,
    row: u16,
    col: u16,
) -> Vec<u8> {
    let button = if up { BUTTON_WHEEL_UP } else { BUTTON_WHEEL_DOWN };
    encode_mouse(encoding, button, MouseAction::Press, row, col)
}

/// Whether a program in this mode wants to hear about this kind of event.
///
/// The modes are a ladder, and a program that asked for a rung near the bottom means it: X10 mode
/// asks for presses and nothing else, and sending it releases it never asked for is how a program
/// ends up acting on a click twice.
fn mode_reports(mode: vt100::MouseProtocolMode, action: MouseAction) -> bool {
    use vt100::MouseProtocolMode as Mode;
    match mode {
        Mode::None => false,
        Mode::Press => action == MouseAction::Press,
        Mode::PressRelease => action != MouseAction::Drag,
        Mode::ButtonMotion | Mode::AnyMotion => true,
    }
}

/// What closes a bracketed paste. Also what has to be taken *out* of anything pasted: a clipboard
/// carrying this sequence would otherwise end the bracket early and leave the rest of its payload
/// arriving as ordinary typing — which, at a shell prompt, means running whatever came after it.
/// That is the whole attack bracketed paste exists to prevent, so the guard is stripped rather
/// than escaped.
const PASTE_END: &str = "\x1b[201~";
const PASTE_START: &str = "\x1b[200~";

/// Pasted text as the program in the pane should receive it.
///
/// When the program has turned bracketed paste on it is asking to be told that what follows was
/// pasted rather than typed, and every program that asks does something with the answer: a shell
/// holds a multi-line paste on the edit line instead of running each line the moment its newline
/// arrives, and vim stops re-indenting. Sending the text bare — which is what happened here —
/// makes a paste indistinguishable from typing, so a copied block of commands executes itself.
///
/// A program that never asked gets exactly what it used to, because to one of those the brackets
/// are not markers, they are six characters of input.
pub fn bracketed_paste(text: &str, bracketed: bool) -> Vec<u8> {
    if !bracketed {
        return text.as_bytes().to_vec();
    }
    let mut out = PASTE_START.as_bytes().to_vec();
    out.extend_from_slice(text.replace(PASTE_END, "").as_bytes());
    out.extend_from_slice(PASTE_END.as_bytes());
    out
}

/// The pty's size, said in both of the units a program may ask it in.
///
/// The pixel half used to be zero, which is not "small" but "unknown": `TIOCGWINSZ` carries it
/// and every program that has to turn pixels into cells reads it there first. Filled in from
/// the host's own cell size, which is the right answer — a pane's cell is the host's cell —
/// and left at zero on a host that never said, because zero is what "unknown" is spelled as
/// and a made-up number would be believed.
fn pty_size(rows: u16, cols: u16) -> PtySize {
    let (width, height) = crate::preview::cell_pixel_size().unwrap_or((0, 0));
    PtySize {
        rows,
        cols,
        pixel_width: cols.saturating_mul(width),
        pixel_height: rows.saturating_mul(height),
    }
}

/// Takes one kitty graphics command off a pane's output and turns it into a picture the pane
/// can draw, a deletion, or a complaint.
///
/// Runs on the reader thread with the parser's lock, because the two questions it has to ask —
/// where the cursor is, and how far the pane has scrolled — are only true at this exact point
/// in the stream. A frame later the shell may have printed three more lines.
fn take_graphics(
    body: &[u8],
    decoder: &mut crate::pane_graphics::Decoder,
    store: &mut crate::pane_graphics::Store,
    pace: &mut crate::pane_graphics::Pace,
    parser: &Mutex<vt100::Parser>,
    inbox: &Mutex<Vec<crate::pane_graphics::Event>>,
    queued: &AtomicUsize,
) {
    use crate::pane_graphics::{Decoded, Event, Placement};

    let (keys, image) = match decoder.feed(body, pace) {
        Decoded::Picture { keys, image } => {
            // `a=t` is "keep this, I will say where later".
            if keys.action == b't' {
                store.keep(keys.id, image);
                return;
            }
            (keys, image)
        }
        Decoded::Show(keys) => match store.get(keys.id) {
            Some(image) => (keys, image.clone()),
            None => return,
        },
        Decoded::Forget(keys) => {
            // `d` names which pictures to forget, and its absence means all of them — which is
            // what `mpv` sends between frames and what anything else sends to tidy up.
            let all = !matches!(keys.delete, b'i' | b'I' | b'n' | b'N');
            store.forget(keys.id, all);
            lock_poisoned(inbox).push(Event::Forget { id: keys.id, all });
            return;
        }
        Decoded::Unsupported => {
            lock_poisoned(inbox).push(Event::Unsupported);
            return;
        }
        Decoded::Nothing => return,
    };

    // How many cells the picture covers. The program usually says (`c` and `r`); when it does
    // not, the host's cell size answers, and where even that is unknown the picture is measured
    // against a plausible cell rather than refused — being roughly the right size is worth more
    // than being absent.
    let (cell_w, cell_h) = crate::preview::cell_pixel_size().unwrap_or((10, 20));
    let in_cells = |pixels: u32, cell: u16| -> u16 {
        let cell = u32::from(cell.max(1));
        pixels.div_ceil(cell).clamp(1, u32::from(u16::MAX)) as u16
    };
    let cols = if keys.cols > 0 { keys.cols } else { in_cells(image.width(), cell_w) };
    let rows = if keys.rows > 0 { keys.rows } else { in_cells(image.height(), cell_h) };

    let mut p = lock_poisoned(parser);
    let (cursor_row, cursor_col) = p.screen().cursor_position();
    let alternate = p.screen().alternate_screen();
    // Counted from the top of everything the pane has ever printed, so the picture travels with
    // the text it was printed among instead of standing still while that scrolls past it.
    let anchor = held_lines(&mut p) + usize::from(cursor_row);

    // Where the cursor ends up. A kitty terminal moves it past the picture unless told not to,
    // and a program printing two pictures in a row — or a newline after one — is relying on
    // that. Sent through the parser as the ordinary moves they are.
    if !keys.keep_cursor {
        let mut moves = Vec::new();
        if rows > 1 {
            moves.extend_from_slice(format!("\x1b[{}B", rows - 1).as_bytes());
        }
        if cols > 0 {
            moves.extend_from_slice(format!("\x1b[{cols}C").as_bytes());
        }
        process_anchored(&mut p, &moves);
    }
    drop(p);

    let mut inbox = lock_poisoned(inbox);
    // A frame that was never taken up is a frame nobody will see: a player draws at the same
    // spot every time, and the one already waiting here would be replaced by this one the
    // moment the two were read together. Dropping it in the queue rather than on the way out
    // of it is what keeps a paused editor from holding a second of video in memory.
    inbox.retain(|waiting| {
        !matches!(waiting, Event::Place(other)
            if other.anchor == anchor && other.col == cursor_col && other.alternate == alternate)
    });
    inbox.push(Event::Place(Box::new(Placement {
        id: keys.id,
        anchor,
        col: cursor_col,
        cols,
        rows,
        alternate,
        image,
    })));
    // Counted under the inbox's own lock, so the pane emptying the queue and the reader thread
    // filling it can never disagree about how much is in it.
    let waiting = inbox.iter().filter(|event| matches!(event, Event::Place(_))).count();
    queued.store(waiting, Ordering::Relaxed);
}

/// Whether a cell has something in it that a picture would be hiding.
///
/// `vt100` counts a space as contents — a cell written with `' '` has a length of one, the
/// same as a cell written with a letter — and that is the difference between a thumbnail that
/// appears and one that does not. `fzf` blanks its preview window before it fills it, so the
/// rows a picture had just been placed on were full of spaces a frame later and the picture was
/// thrown away every time, which is why `ytfzf -t` showed its list and no pictures. Blank is
/// blank however it got that way: what covers a picture is a glyph.
fn covered(cell: &vt100::Cell) -> bool {
    !cell.contents().trim().is_empty()
}

/// Where in the cells it was given a picture actually fits, as (down, across, tall, wide), or
/// `None` when there is nowhere left and the picture has gone.
///
/// Two different things write into a picture's rectangle, and telling them apart is the whole
/// job.
///
/// One is *furniture beside it*: a column taken all the way from the top of the picture to the
/// bottom is not text on the picture, it is something standing next to it that the picture was
/// measured a little too wide for. `fzf` draws a border down the side of its preview window,
/// and a picture that reached it lost every row it had — which is why `ytfzf -t` showed a list
/// and no thumbnails. Those columns are given back at the edges, and only at the edges: one in
/// the middle really does cut the picture in two.
///
/// The other is *text on it*: a line written across the picture, which takes the rows it is on
/// and leaves the rest. `mpv` writes its status straight over the top row of its own frame,
/// every frame, and a picture thrown away for being touched anywhere was thrown away thirty
/// times a second — a film that plays perfectly and shows nothing. Given the rows instead, it
/// plays with its status line above it, which is what it looks like in a terminal that keeps
/// text and pictures in two layers.
///
/// The longest stretch of rows rather than the first, because that status line may land
/// anywhere and taking the first stretch would leave nothing whenever it landed on row nought.
fn fit_into_free_cells(
    rows: u16,
    cols: u16,
    taken: impl Fn(u16, u16) -> bool,
) -> Option<(u16, u16, u16, u16)> {
    if rows == 0 || cols == 0 {
        return None;
    }
    // Furniture first: a column with something in it on every single row of the picture.
    let solid = |col: u16| (0..rows).all(|row| taken(row, col));
    let mut left = 0;
    while left < cols && solid(left) {
        left += 1;
    }
    let mut right = cols;
    while right > left && solid(right - 1) {
        right -= 1;
    }
    if left == right {
        return None;
    }
    // Then the text, a row at a time, across what is left.
    let (mut best, mut run) = ((0u16, 0u16), (0u16, 0u16));
    for row in 0..rows {
        let free = !(left..right).any(|col| taken(row, col));
        run = if free { (if run.1 == 0 { row } else { run.0 }, run.1 + 1) } else { (0, 0) };
        if run.1 > best.1 {
            best = run;
        }
    }
    (best.1 > 0).then_some((best.0, left, best.1, right - left))
}

/// Names the host terminal exports to say which terminal it is, all of which are false inside a
/// pane and all of which are read by something.
///
/// Not an exhaustive list of every terminal on earth — an unknown one's marker does no harm,
/// because a program that recognises it is a program that would have found nothing to use
/// anyway. These are the ones whose presence actually changes a decision: the graphics
/// protocol `chafa`, `ytfzf`, `timg` and `viu` pick, and the keyboard protocol some readline
/// replacements turn on.
const HOST_TERMINAL_ENV: &[&str] = &[
    "TERM_PROGRAM",
    "TERM_PROGRAM_VERSION",
    "TERMINAL_EMULATOR",
    "GHOSTTY_RESOURCES_DIR",
    "GHOSTTY_BIN_DIR",
    "GHOSTTY_SHELL_FEATURES",
    "KITTY_WINDOW_ID",
    "KITTY_PID",
    "KITTY_INSTALLATION_DIR",
    "KITTY_PUBLIC_KEY",
    "WEZTERM_EXECUTABLE",
    "WEZTERM_PANE",
    "WEZTERM_UNIX_SOCKET",
    "WEZTERM_CONFIG_FILE",
    "ITERM_SESSION_ID",
    "ITERM_PROFILE",
    "LC_TERMINAL",
    "LC_TERMINAL_VERSION",
    "VTE_VERSION",
    "KONSOLE_VERSION",
    "ALACRITTY_WINDOW_ID",
    "ALACRITTY_SOCKET",
    "CONTOUR_PROFILE",
    "WT_SESSION",
    "WT_PROFILE_ID",
];

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
    /// `CSI 18 t` — how many rows and columns the text area has.
    TextAreaCells,
    /// `CSI 16 t` — how many pixels one cell is.
    CellPixels,
    /// `CSI 14 t` — how many pixels the whole text area is.
    TextAreaPixels,
}

/// What a pane knows about itself when a program asks. Gathered once per batch of queries
/// rather than per query, because reading the cursor means taking the parser's lock.
#[derive(Clone, Copy)]
struct PaneMetrics {
    /// Row and column, zero-based.
    cursor: (u16, u16),
    /// Rows and columns.
    size: (u16, u16),
    /// One cell's width and height in pixels, as the host reported them. `None` on a host that
    /// never said, and then the size questions go unanswered rather than invented.
    cell: Option<(u16, u16)>,
}

impl TerminalQuery {
    fn response(&self, metrics: PaneMetrics) -> Vec<u8> {
        let (cursor_row, cursor_col) = metrics.cursor;
        let (rows, cols) = metrics.size;
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
            // The three size questions, which a pane could not answer at all until now.
            //
            // `chafa` asks all three before it draws anything, and so does anything else that
            // has to turn pixels into cells. Unanswered they each cost it a timeout and then a
            // guess — square cells, which is why a picture in a pane came out with its
            // proportions wrong even when it came out at all.
            TerminalQuery::TextAreaCells => format!("\x1b[8;{rows};{cols}t").into_bytes(),
            TerminalQuery::CellPixels => match metrics.cell {
                Some((width, height)) => format!("\x1b[6;{height};{width}t").into_bytes(),
                None => Vec::new(),
            },
            TerminalQuery::TextAreaPixels => match metrics.cell {
                Some((width, height)) => format!(
                    "\x1b[4;{};{}t",
                    u32::from(rows) * u32::from(height),
                    u32::from(cols) * u32::from(width)
                )
                .into_bytes(),
                None => Vec::new(),
            },
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
                            // `t` is mostly window *manipulation* — move, raise, retitle — and
                            // a pane has no window to manipulate. Exactly three of its forms
                            // are questions, and those are the three answered here; everything
                            // else ending in `t` is left to be ignored as before.
                            b't' if self.params == b"18" => out.push(TerminalQuery::TextAreaCells),
                            b't' if self.params == b"16" => out.push(TerminalQuery::CellPixels),
                            b't' if self.params == b"14" => {
                                out.push(TerminalQuery::TextAreaPixels)
                            }
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

/// Watches a shell's output for OSC 7 — `ESC ] 7 ; file://host/path ST` — which is how a shell
/// says where it has just moved to.
///
/// It is the one answer that is both exact and free: the shell emits it at every prompt, the
/// bytes are already flowing past this thread, and nothing has to be asked of the operating
/// system.
///
/// Who actually sends it is narrower than it looks. fish sends it unprompted, from its own
/// interactive config. macOS ships the code for zsh and bash in `/etc/zshrc` and `/etc/bashrc`
/// — and gates the whole block on `TERM_PROGRAM` being `Apple_Terminal`, which a pane here is
/// not and must not claim to be. So for a great many shells nothing arrives at all, and
/// `crate::dnd::shell_cwd` asks the process table instead: not a fallback for the exotic case
/// but the ordinary road for anybody whose shell is zsh.
///
/// The host half of the URI is not decoration. Over ssh the shell reporting a path is on another
/// machine, and that path taken for a local one points the sidebar at a folder that either does
/// not exist here or — worse — does, and belongs to something else entirely. A sequence from
/// anywhere but this host is dropped.
#[derive(Default)]
struct Osc7Scanner {
    /// 0 = ground, 1 = saw ESC, 2 = collecting the OSC payload, 3 = saw ESC inside it, which is
    /// the first half of a string terminator.
    state: u8,
    payload: Vec<u8>,
}

/// A cap on the payload. An OSC that never terminates would otherwise grow this buffer for as
/// long as the shell keeps talking; paths are orders of magnitude shorter, and anything longer
/// is not one.
const OSC_MAX: usize = 4096;

impl Osc7Scanner {
    /// Feeds this chunk of output, answering with the folder if the chunk completed an OSC 7
    /// naming one. The last such sequence in the chunk wins: a burst carrying two prompts has
    /// been in two places, and where it ended up is the later one.
    fn feed(&mut self, data: &[u8]) -> Option<PathBuf> {
        let mut found = None;
        for &b in data {
            match self.state {
                1 => match b {
                    b']' => {
                        self.state = 2;
                        self.payload.clear();
                    }
                    0x1b => {}
                    _ => self.state = 0,
                },
                2 => match b {
                    // BEL and `ESC \` both end an OSC string, and shells in the wild use either.
                    0x07 => found = self.finish().or(found),
                    0x1b => self.state = 3,
                    _ => {
                        if self.payload.len() < OSC_MAX {
                            self.payload.push(b);
                        }
                    }
                },
                3 => {
                    if b == b'\\' {
                        found = self.finish().or(found);
                    } else {
                        // An ESC that turned out not to be a terminator still ends the string:
                        // what follows is a new sequence, and a payload carried across it would
                        // be two halves of different messages read as one.
                        self.state = if b == 0x1b { 1 } else { 0 };
                        self.payload.clear();
                    }
                }
                _ => {
                    if b == 0x1b {
                        self.state = 1;
                    }
                }
            }
        }
        found
    }

    /// Ends the string in hand and reads a folder out of it, if it was one of ours.
    fn finish(&mut self) -> Option<PathBuf> {
        self.state = 0;
        let payload = std::mem::take(&mut self.payload);
        let body = payload.strip_prefix(b"7;")?;
        parse_cwd_uri(std::str::from_utf8(body).ok()?)
    }
}

/// The folder an OSC 7 body names, when it names one on this machine.
fn parse_cwd_uri(uri: &str) -> Option<PathBuf> {
    let Some(rest) = uri.strip_prefix("file://") else {
        // Some shells send a bare path instead of a URI. There is no host to disagree with, so
        // it can only mean here.
        return uri.starts_with('/').then(|| PathBuf::from(percent_decoded(uri)));
    };
    // `file://host` with no slash after it names a machine and no folder on it.
    let at = rest.find('/')?;
    let (host, path) = rest.split_at(at);
    if !host_is_this_machine(host) {
        return None;
    }
    let path = percent_decoded(path);
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(drive_path(path)))
}

/// On Windows the URI's path begins with the separator that follows the authority — `/C:/Users`
/// — and the drive letter underneath it is where the real path starts.
#[cfg(windows)]
fn drive_path(path: String) -> String {
    let rest = path.strip_prefix('/').unwrap_or(&path);
    let mut chars = rest.chars();
    let drive = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    if drive && chars.next() == Some(':') {
        return rest.to_string();
    }
    path
}

#[cfg(not(windows))]
fn drive_path(path: String) -> String {
    path
}

/// Whether a host named in an OSC 7 is the machine CleeCode is running on.
///
/// The short name and the fully-qualified one are both this machine, and which of the two a
/// shell writes is not ours to decide: macOS reports itself as `name.local` while the shell
/// says `name`, and on a server it is often the other way round. Both ends are cut to their
/// first label and compared there, which accepts either spelling and still refuses a different
/// machine.
fn host_is_this_machine(host: &str) -> bool {
    if host.is_empty() || host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    static SHORT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let ours = SHORT.get_or_init(|| {
        sysinfo::System::host_name()
            .unwrap_or_default()
            .split('.')
            .next()
            .unwrap_or_default()
            .to_lowercase()
    });
    if ours.is_empty() {
        return false;
    }
    host.split('.').next().is_some_and(|label| label.eq_ignore_ascii_case(ours))
}

/// Percent-decoding, which is what makes a folder with a space in its name arrive as one folder.
/// A `%` that is not followed by two hex digits is left alone rather than dropped: it is a
/// literal in a path that was never encoded, and eating it would rename the folder.
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(byte) = hex_pair(bytes[i + 1], bytes[i + 2]) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_pair(hi: u8, lo: u8) -> Option<u8> {
    Some((hex_digit(hi)? << 4) | hex_digit(lo)?)
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
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
        Self::with_startup_env(rows, cols, cwd, startup, &[])
    }

    /// The same, plus names exported on this one shell and no other.
    ///
    /// The drawer is the only caller that passes any, and the narrowness is the point: two of the
    /// four agents are handed CleeCode's MCP server through an environment variable that names
    /// one of *their own* configuration files, and doing that to every shell would mean an
    /// `opencode` somebody typed in an ordinary pane silently reading a config CleeCode wrote
    /// instead of their own. `CLEE_SESSION` below is set everywhere precisely because it is the
    /// opposite kind of name — ours, unknown to anything that did not come looking for it.
    pub fn with_startup_env(
        rows: u16,
        cols: u16,
        cwd: &Path,
        startup: Option<&str>,
        env: &[(&str, std::path::PathBuf)],
    ) -> Result<Self> {
        let (rows, cols) = buildable_size(rows, cols);
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(pty_size(rows, cols))?;

        let mut cmd = CommandBuilder::new(default_shell());
        cmd.cwd(cwd);
        // Marks the shell as running inside CleeCode, so a user's rc can skip heavy startup
        // work (e.g. `fastfetch`) here — `if not set -q CLEECODE; fastfetch; end` in fish.
        cmd.env("CLEECODE", "1");
        // Where this editor publishes what it knows, for `clee --mcp` to read.
        //
        // Exported here and nowhere else on purpose. An agent run in a pane inherits it, and the
        // MCP server the agent spawns inherits it in turn — so the server is joined to *this*
        // CleeCode by descent rather than by searching for one, which is the only way to get it
        // right when two are open at once. Set on every shell for the same reason as the
        // interpreter variables below: guessing at spawn time what somebody is about to type
        // costs more than an unread name in the environment.
        if let Some(session) = crate::mcp::session_dir() {
            cmd.env(crate::mcp::SESSION_ENV, session);
        }
        // What the program in this pane is actually talking to.
        //
        // Inherited, `TERM` names the terminal CleeCode is *displayed in* — and the pane is not
        // that terminal, it is the vt100 parser below. The two disagree about what is possible:
        // a pane advertised as `xterm-kitty` invites a graphics protocol this parser has never
        // heard of, and one advertised as `xterm-ghostty` names a terminfo entry that only
        // exists where Ghostty was installed.
        //
        // That second half broke a real session on 2026-08-22. CleeCode was running on an Ubuntu
        // box over ssh from Ghostty, ssh carried `TERM=xterm-ghostty` across, and Ubuntu had no
        // such entry — so `clear` cleared nothing, and neither did the form feed this sends to
        // scrub a shell's startup banner, because both go through the same terminfo capability.
        // Nothing in the pane was wrong; it had simply been told it was a terminal that was not
        // there.
        //
        // `xterm-256color` is the honest answer: it is what this parser implements, and every
        // terminfo database on earth has it. `COLORTERM` says the rest — the parser does carry
        // 24-bit colour through to the screen (see `vt100::Color::Rgb` in the renderer), which
        // `xterm-256color` alone would not let a program ask for.
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        // And what it is *not* talking to.
        //
        // Overriding `TERM` was only half the job. A great many programs never ask the terminal
        // anything: they read the environment, find `TERM_PROGRAM=ghostty` or `KITTY_WINDOW_ID`,
        // and take that as proof of a graphics protocol. Those names are inherited from the
        // terminal CleeCode is displayed in, and inside a pane every one of them is a lie —
        // which is the whole of why `mpv --vo=kitty` in a pane could be heard and not seen, and
        // why `chafa` left to itself chose kitty and printed a megabyte into a blank pane.
        //
        // Removed rather than rewritten. There is no value for `TERM_PROGRAM` that describes
        // this honestly, and inventing one only moves the guess somewhere else; a name that is
        // absent is a question the program answers by looking at what it can actually do.
        // `CLEECODE=1`, set above, is the name that *is* true here and is documented as such.
        for name in HOST_TERMINAL_ENV {
            cmd.env_remove(name);
        }
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
        // Last, so a pane that was given a name of its own means it: nothing above is one of
        // these, and a future collision should end with the caller's answer rather than ours.
        for (key, value) in env {
            cmd.env(key, value);
        }
        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader()?;

        // The pty's write end belongs to a thread of its own from here on.
        //
        // Writing into a pty blocks whenever the far end has stopped reading, and that is an
        // ordinary state for a shell to be in — a job suspended, a program waiting on something
        // else — not a fault. Done from the UI thread it stopped the whole editor: one paste into
        // one such pane and every other pane, the file tree and the keyboard went with it. Done
        // from the reader thread, which used to answer terminal queries while holding the writer's
        // mutex, it wedged that lock and took the UI thread down the next time it typed.
        //
        // So there is no shared writer any more, and no lock. Senders queue bytes and this thread
        // is the only thing that ever blocks on them. It ends when the last sender goes: the
        // panel's on `Drop`, the reader thread's when it stops.
        let mut pty_writer = pair.master.take_writer()?;
        let (input, outbox) = sync_channel::<Vec<u8>>(WRITE_QUEUE);
        std::thread::spawn(move || {
            while let Ok(chunk) = outbox.recv() {
                // A write that fails means the far end is gone for good, unlike one that merely
                // blocks, so there is nothing left for this thread to do.
                if pty_writer.write_all(&chunk).is_err() {
                    break;
                }
                let _ = pty_writer.flush();
            }
        });

        let parser =
            Arc::new(Mutex::new(vt100::Parser::new(rows, cols, SCROLLBACK_LEN.load(Ordering::Relaxed))));
        let parser_clone = Arc::clone(&parser);
        let exited = Arc::new(AtomicBool::new(false));
        let exited_clone = Arc::clone(&exited);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);

        let spawn = Instant::now();
        let generation = Arc::new(AtomicU64::new(0));
        let generation_clone = Arc::clone(&generation);
        let last_output_ms = Arc::new(AtomicU64::new(0));
        let produced_output = Arc::new(AtomicBool::new(false));
        let last_output_clone = Arc::clone(&last_output_ms);
        let produced_clone = Arc::clone(&produced_output);
        let input_clone = input.clone();
        let cwd: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
        let cwd_clone = Arc::clone(&cwd);
        let reports_cwd = Arc::new(AtomicBool::new(false));
        let reports_clone = Arc::clone(&reports_cwd);
        let graphics: Arc<Mutex<Vec<crate::pane_graphics::Event>>> = Arc::new(Mutex::new(Vec::new()));
        let graphics_clone = Arc::clone(&graphics);
        let graphics_queued: Arc<AtomicUsize> = Arc::default();
        let queued_clone = Arc::clone(&graphics_queued);
        let queued_reader = Arc::clone(&graphics_queued);

        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            let mut scanner = CsiScanner::default();
            let mut osc = Osc7Scanner::default();
            // The three halves of the graphics road. They live on this thread because the work
            // is per-pane and must not be on the frame's path: decoding a thumbnail is
            // milliseconds, and a pane doing it is a pane, while the main thread doing it is
            // every pane, the editor and the keyboard.
            let mut stream = crate::pane_graphics::PaneStream::default();
            let mut decoder = crate::pane_graphics::Decoder::default();
            let mut store = crate::pane_graphics::Store::default();
            let mut pace = crate::pane_graphics::Pace::sharing(queued_clone);
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        // A closed pane's output is discarded, never left unread. Stopping the
                        // thread here instead — which is what this used to do — deadlocked the
                        // quit: a shell dying with output still in the pty (fish repaints its
                        // prompt on the way out; /bin/sh dies silently, which is why no test
                        // saw it) sits in the kernel's tty drain waiting for a reader, `exit`
                        // never finishes, and the `wait()` in shutdown parks forever. So the
                        // flag mutes the parser and the replies, and only EOF — the far end
                        // truly gone — ends the thread, the way a real terminal reads until
                        // hangup.
                        if stop_clone.load(Ordering::Relaxed) {
                            continue;
                        }
                        let data = &buf[..n];
                        // Split before parsed, because a picture is placed at the cursor and
                        // the cursor is wherever the text before it left off. The pieces come
                        // back in the order they were written for exactly that reason.
                        for piece in stream.feed(data) {
                            match piece {
                                crate::pane_graphics::Piece::Text(text) => {
                                    let mut p = lock_poisoned(&parser_clone);
                                    process_anchored(&mut p, &text);
                                }
                                crate::pane_graphics::Piece::Apc(body) => take_graphics(
                                    &body,
                                    &mut decoder,
                                    &mut store,
                                    &mut pace,
                                    &parser_clone,
                                    &graphics_clone,
                                    &queued_reader,
                                ),
                                crate::pane_graphics::Piece::ApcBroken => decoder.abandon(),
                                crate::pane_graphics::Piece::ClearScreen => {
                                    let alternate =
                                        lock_poisoned(&parser_clone).screen().alternate_screen();
                                    store.forget(0, true);
                                    lock_poisoned(&graphics_clone).push(
                                        crate::pane_graphics::Event::ClearScreen { alternate },
                                    );
                                }
                            }
                        }
                        // Raised after the parser has taken the bytes, so a reader that sees the
                        // new number is looking at a screen that already holds them.
                        generation_clone.fetch_add(1, Ordering::Release);
                        last_output_clone.store(spawn.elapsed().as_millis() as u64, Ordering::Relaxed);
                        produced_clone.store(true, Ordering::Relaxed);

                        // Where the shell says it is, if it said so in this chunk. Read off the
                        // same bytes the screen was drawn from, so the sidebar's shell half and
                        // the prompt underneath it can never be looking at different folders.
                        if let Some(dir) = osc.feed(data) {
                            reports_clone.store(true, Ordering::Relaxed);
                            *lock_poisoned(&cwd_clone) = Some(dir);
                        }

                        // Answer terminal capability/status queries so probing programs
                        // (fastfetch, vim, etc.) don't stall for seconds waiting for a reply.
                        let mut queries = Vec::new();
                        scanner.feed(data, &mut queries);
                        if !queries.is_empty() {
                            let metrics = {
                                let p = lock_poisoned(&parser_clone);
                                PaneMetrics {
                                    cursor: p.screen().cursor_position(),
                                    size: p.screen().size(),
                                    cell: crate::preview::cell_pixel_size(),
                                }
                            };
                            // Queued like anything else the pane sends, and queued as one
                            // message so a burst of queries cannot fill the channel on its own.
                            // `try_send` because this thread must keep draining the shell's
                            // output whatever the state of the write side: blocking here would
                            // freeze the pane's display as well as its input.
                            let mut reply = Vec::new();
                            for q in &queries {
                                reply.extend_from_slice(&q.response(metrics));
                            }
                            let _ = input_clone.try_send(reply);
                        }
                    }
                    Err(_) => break,
                }
            }
            exited_clone.store(true, Ordering::Relaxed);
        });

        Ok(TerminalPanel {
            master: pair.master,
            input,
            child,
            parser,
            rows,
            cols,
            exited,
            stop,
            generation,
            spawn,
            last_output_ms,
            produced_output,
            revealed: false,
            selection: None,
            name: None,
            startup_command: startup.map(str::to_string),
            pending_command: startup.map(str::to_string),
            pending_unsent: None,
            cleared: false,
            last_scroll: None,
            cwd,
            reports_cwd,
            graphics,
            graphics_queued,
            spare: None,
            images: Vec::new(),
            graphics_note: None,
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
        self.suppress_banner_scrub();
    }

    /// Queues one line to be put at the prompt without running it, once the shell is actually
    /// reading — no carriage return follows it, so nothing happens until the user presses Enter.
    /// For a command they have to read before they run it: an install line that pipes a
    /// downloaded script into a shell, above all, where submitting it for them would be running
    /// a remote program because a mouse landed on a row.
    ///
    /// Held back rather than typed on the spot for the reason `run_command` is: the shell chosen
    /// to type into may be one that has only just been opened, still inside its rc and not yet
    /// reading keys, and a line written to it then is half-swallowed — `curl …` arriving as
    /// `cu`. `flush_pending` waits for the prompt and then types it, in the same step as the
    /// banner scrub so the two can never race: the scrub goes first, and the line's own leading
    /// reset clears away anything it left, on a shell that binds the form feed and one that does
    /// not alike.
    pub fn queue_line_unsent(&mut self, command: &str) {
        self.pending_unsent = Some(command.to_string());
    }

    /// Queues one line to be typed *and run* once the shell is reading, which is the twin of
    /// `queue_line_unsent` and exists for the same reason.
    ///
    /// A shell that has only just been opened is not busy, it is not ready yet — and those are
    /// different answers. Refusing it was what an agent got: `open_terminal` followed straight
    /// away by `run_command`, which is the obvious way to use the pair, was turned down every
    /// time on a shell with no line editor, because nothing there ever said "now".
    ///
    /// `false` when a line is already waiting: two lines queued at one prompt would arrive glued
    /// together, and the caller has something truthful to say instead.
    pub fn queue_line(&mut self, command: &str) -> bool {
        if self.pending_command.is_some() || self.pending_unsent.is_some() {
            return false;
        }
        self.pending_command = Some(command.to_string());
        true
    }

    /// Marks the one-time startup banner scrub as done without sending it, because a line has
    /// just been typed here on purpose.
    ///
    /// The scrub is a form feed, fired the moment the shell is first reading keys — which, on a
    /// slow machine, can be after this pane was handed a command rather than before. A shell with
    /// readline turns that form feed into a harmless redraw; `/bin/sh` on Linux is dash, which
    /// has none, so it keeps the form feed as a literal `^L` dropped into the middle of the line
    /// we just typed — `curl …` arriving as `cu^L…`. Whatever the line was for, once it is down
    /// the scrub can only hurt it: the command's own leading line-reset already cleared away
    /// anything the banner left, so there is nothing left for the form feed to do but land in the
    /// wrong place. Marking it done is how it is kept from firing behind us.
    fn suppress_banner_scrub(&mut self) {
        self.cleared = true;
    }

    pub fn run_command(&mut self, command: &str) {
        self.pending_command = Some(command.to_string());
    }

    /// Scrubs the startup banner and types the held command, once the shell has settled. Does
    /// nothing until the shell is ready, and each part only ever fires once.
    ///
    /// A form feed is what a prompt understands as "clear and redraw yourself". Every
    /// interactive shell binds it, and unlike running `clear` it leaves nothing behind in the
    /// history. It also has to come *before* the command rather than instead of it — a fastfetch
    /// that the rc prints after the pane was spawned survives otherwise, whether or not this
    /// pane has a command.
    ///
    /// Both of those want a shell that is *reading*, and this used to settle for a shell that
    /// had gone quiet for a quarter of a second. Those are not the same thing: an rc that runs
    /// `fastfetch` and waits for it looks exactly as quiet as a prompt. Send a form feed then
    /// and no readline is listening — the tty is in cooked mode with echo on, so the character
    /// comes straight back as a literal `^L`, printed above a banner that is still arriving.
    /// That is what a remote Linux box showed: two panes wearing their fastfetch and a `^L` in
    /// the middle of it, while the same rc on a fast Mac finished inside the quarter second and
    /// worked every time.
    ///
    /// `shell_is_reading_keys` asks the question that was actually meant.
    ///
    /// The command itself goes in through `typed_line`, which empties the line first: whatever
    /// the editor happens to be holding — a leftover from the rc, half a word someone typed
    /// into a pane while it was still starting — goes, so the command can never be glued onto
    /// the end of something else.
    pub fn flush_pending(&mut self) {
        if !self.is_ready() {
            return;
        }
        // Nothing typed at a shell that is not reading. Both of these are keystrokes, and a
        // keystroke sent to an rc still running its own commands is either echoed as a control
        // character or eaten by whatever *is* reading. Past the deadline it goes anyway: a shell
        // whose rc never gives the terminal back would otherwise never get its startup command,
        // which is worse than a command typed into something unexpected.
        // The whole question, not half of it. A shell whose rc is waiting on a command it
        // started is exactly as quiet as a shell at its prompt — so quiet alone says "ready"
        // in the middle of somebody's rc, and a form feed sent then is not a command, it is a
        // character echoed back as `^L` above a banner still arriving. What tells the two apart
        // is who holds the terminal: the command the rc started does, and `is_at_prompt` asks.
        if !self.is_at_prompt() && self.spawn.elapsed() < STARTUP_MAX {
            return;
        }
        // The scrub is a form feed, and it means "clear and redraw" only to a shell that binds
        // it. A shell with no line editor binds nothing: it keeps the form feed as a literal
        // `^L` dropped into whatever is on the line — which is why `suppress_banner_scrub`
        // exists at all. So it is not sent to one. The banner stays where it is, which is what
        // it did before there was a scrub, and is a great deal better than a corrupted command.
        //
        // This used to be hidden by the clock rather than decided: the wait above let a shell
        // without an editor through only at `STARTUP_MAX`, twelve seconds in, by which time
        // nothing was being typed for the `^L` to land in the middle of. Letting that shell
        // through as soon as it is quiet — which is the point of the change this sits in — put
        // the form feed back among the keystrokes, and the marks a driven pane types went
        // missing.
        // Marked done only when it is actually sent. The gate above can now let a shell through
        // on a quiet pane rather than on the line discipline, and that is a moment when the
        // shell may not be reading keys yet — setting the flag there spent the one scrub there
        // is on a form feed that was never written, and the banner stayed for good.
        if !self.cleared && self.shell_is_reading_keys() {
            self.cleared = true;
            self.write_input(b"\x0c");
        }
        if let Some(command) = self.pending_command.take() {
            self.write_input(&typed_line(&command));
        }
        // The unsent twin, typed the same way and at the same one moment as the scrub above, so
        // the form feed and the command can never arrive out of order: whatever the scrub left
        // on the line, this line's own reset clears before it types. Left at the prompt, never
        // submitted.
        if let Some(command) = self.pending_unsent.take() {
            self.write_input(&typed_line_unsent(&command));
        }
    }

    /// Whether the pane is back at a prompt: something in it is reading keystrokes rather than
    /// running a command. Used by the run watcher to know when a script it started is over.
    /// Whether a line sent now would reach the shell rather than something it started.
    ///
    /// Two questions, and each one alone gets a case wrong.
    ///
    /// The first is whether the shell itself holds the terminal. A program the shell started in
    /// the foreground owns the terminal's process group while it runs, and a line sent then is
    /// not a command, it is keystrokes for that program. This was the only test tried at first
    /// and it was dropped for the wrong reason: it cannot tell a shell running its own rc from a
    /// shell at its prompt, because `bash` runs rc commands in its own group. True — but that is
    /// one case, and without this test `vim` was another: `vim` turns the line discipline off
    /// exactly as a line editor does, so the test below said "at a prompt" and the line was typed
    /// into the buffer somebody had open.
    ///
    /// The second is whether the shell is ready for a line. A shell with a line editor answers by
    /// turning canonical mode off to read keys one at a time, and that is the sharp signal:
    /// measured on a slow rc, `ICANON` reads on for the whole of it and off within a tick of the
    /// prompt appearing. But a shell without a line editor never turns it off at all — `dash`,
    /// which is `/bin/sh` on Debian and Ubuntu, sits at its prompt in canonical mode forever. Read
    /// as the only signal, that shell was busy for as long as it lived and nothing could ever be
    /// typed into it. So where the line discipline says nothing, a quiet pane says it instead:
    /// output that has stopped for `STARTUP_IDLE` is a shell that has finished what it was doing.
    pub fn is_at_prompt(&self) -> bool {
        self.shell_holds_the_terminal() && self.shell_is_ready_for_a_line()
    }

    /// Whether a program the shell started is in front of it. The honest "busy": a line sent now
    /// would be keystrokes for that program.
    pub fn program_in_front(&self) -> bool {
        !self.shell_holds_the_terminal()
    }

    /// Whether the shell would read a line as a line.
    ///
    /// A shell with a line editor says so by turning canonical mode off. One without never does,
    /// and for it the answer is a pane that has stopped printing: `STARTUP_IDLE` of quiet is a
    /// shell that has finished what it was doing. That second clause can in principle fire on an
    /// rc that pauses a quarter second in the middle — but the alternative for such a shell was
    /// waiting out `STARTUP_MAX` and typing anyway, so this moves an existing risk earlier
    /// rather than inventing one, and it moves twelve seconds of dead waiting with it.
    fn shell_is_ready_for_a_line(&self) -> bool {
        self.shell_is_reading_keys() || self.output_settled()
    }

    /// Whether the shell itself is the terminal's foreground group, rather than something it
    /// started. `true` when it cannot be asked, so a platform without this is no worse off.
    #[cfg(unix)]
    fn shell_holds_the_terminal(&self) -> bool {
        let (Some(fd), Some(pid)) = (self.master.as_raw_fd(), self.child_pid()) else {
            return true;
        };
        // SAFETY: `fd` is the pty master this pane owns and is open for as long as the pane is;
        // `tcgetpgrp` only reads. The shell is a session leader on this pty, so its process
        // group is its own pid.
        let group = unsafe { libc::tcgetpgrp(fd) };
        group < 0 || group == pid as libc::pid_t
    }

    #[cfg(not(unix))]
    fn shell_holds_the_terminal(&self) -> bool {
        true
    }

    /// Whether the pane has stopped printing for long enough to have finished something.
    fn output_settled(&self) -> bool {
        let since_start = self.spawn.elapsed();
        let idle = since_start
            .saturating_sub(Duration::from_millis(self.last_output_ms.load(Ordering::Relaxed)));
        self.produced_output.load(Ordering::Relaxed) && idle >= STARTUP_IDLE
    }

    /// Whether something in the pane is reading keystrokes as keystrokes.
    ///
    /// The terminal's own line discipline answers this. A shell running its rc leaves the pty in
    /// canonical mode with echo on: a form feed sent then is not a command, it is a character,
    /// and it comes straight back as a literal `^L`. A line editor turns both off to read keys
    /// one at a time, and from that moment a form feed means "clear and redraw".
    ///
    /// `true` when it cannot be asked, so a platform without this is no worse off than before.
    #[cfg(unix)]
    fn shell_is_reading_keys(&self) -> bool {
        let Some(fd) = self.master.as_raw_fd() else { return true };
        // SAFETY: `fd` is the pty master this pane owns and is open for as long as the pane is;
        // `tcgetattr` only reads, into a struct sized by libc itself.
        unsafe {
            let mut attrs: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut attrs) != 0 {
                return true;
            }
            attrs.c_lflag & libc::ICANON == 0
        }
    }

    #[cfg(not(unix))]
    fn shell_is_reading_keys(&self) -> bool {
        true
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

    /// Whether this pane is still waiting to be shown. The moment it arrives is decided by the
    /// clock — a quiet quarter second — and a clock cannot announce itself, so while any pane is
    /// hidden the frame loop keeps drawing rather than letting a shell appear a second late.
    pub fn awaiting_reveal(&self) -> bool {
        !self.revealed
    }

    /// Hands input to the shell, without ever waiting for it to be taken.
    ///
    /// `false` says the write was dropped: the queue was full, which means the shell has not read
    /// a byte for a long time, or its end of the pty is gone. Losing a paste is a bad outcome and
    /// the caller is welcome to say so; the outcome it replaces is worse, because the alternative
    /// to dropping is blocking, and this is called from the thread that draws every frame and
    /// reads every key. One unreadable pane used to stop all of them.
    pub fn write_input(&mut self, bytes: &[u8]) -> bool {
        self.input.try_send(bytes.to_vec()).is_ok()
    }

    /// Pasted text, bracketed when the program in this pane asked for that. See
    /// [`bracketed_paste`].
    pub fn paste_bytes(&self, text: &str) -> Vec<u8> {
        bracketed_paste(text, self.holds_a_paste())
    }

    /// Whether the program in this pane turned bracketed paste on, which is the same as asking
    /// whether it can hold a multi-line paste at its prompt instead of running it a line at a
    /// time. Something composing text *for* a pane needs the answer before it composes.
    pub fn holds_a_paste(&self) -> bool {
        lock_poisoned(&self.parser).screen().bracketed_paste()
    }

    /// How many times output has been fed to this pane's screen. Only differences matter.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// pid of the shell process running in this pane, used for best-effort ssh-session detection.
    pub fn child_pid(&self) -> Option<u32> {
        self.child.process_id()
    }

    /// The folder this pane's shell is in, as far as anyone here knows — `None` while nobody
    /// has said and nobody has asked.
    pub fn cwd(&self) -> Option<PathBuf> {
        lock_poisoned(&self.cwd).clone()
    }

    /// Whether this shell reports its own folder, in which case the operating system never has
    /// to be asked.
    pub fn reports_cwd(&self) -> bool {
        self.reports_cwd.load(Ordering::Relaxed)
    }

    /// Records a folder found some way other than the shell saying so. Never raises
    /// `reports_cwd`: that flag means "this shell speaks for itself", and an answer prised out
    /// of the process table is not the shell speaking.
    pub fn note_cwd(&self, dir: PathBuf) {
        *lock_poisoned(&self.cwd) = Some(dir);
    }

    /// Ends the shell in this pane and collects it.
    ///
    /// Dropping the panel closes nothing on its own: the reader thread holds a dup of the pty
    /// master and the writer thread holds its write end, so both stay open and the shell never
    /// notices that its window is gone. A tab closed that way left a shell — and the
    /// `npm run dev` inside it — running invisibly for the rest of the session, holding its port,
    /// and left a thread parsing its output into a screen nobody would ever draw again.
    ///
    /// The two threads need no signal of their own. The reader stops at the flag or at EOF, and
    /// when it does it drops the last copy of the input queue's sender — which is what ends the
    /// writer, since a channel with no senders left cannot be waited on.
    ///
    /// What a real terminal does when its window closes is hang up: the kernel sends SIGHUP to
    /// the foreground process group of the controlling tty. That is done here by hand, because
    /// the dup keeps the kernel from doing it, and it is done to the *group* — signalling only
    /// the shell leaves the job it was running in the foreground orphaned but alive, which is
    /// the whole difference between closing a tab and closing a tab that was running something.
    /// Then the shell itself, and then `wait`, which is what actually reaps it: portable-pty's
    /// child on unix is a plain `std::process::Child`, and dropping one of those leaves a
    /// zombie behind.
    ///
    /// Every step is best effort and every failure is ignored. This runs from `Drop`, including
    /// while the process is on its way out with a dozen panes to close, and a pane whose shell
    /// died an hour ago must go through it as quietly as one that is still running.
    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // A shell that has already exited needs no signal; it only needs collecting.
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            self.hangup_foreground_group();
            // portable-pty's `kill` is not a kill: on unix it sends SIGHUP ("instead of trying
            // to kill the process", in its own words) and nothing ever escalates. /bin/sh dies
            // to that; an interactive fish does not, and `wait()` below on a shell that ignored
            // the hangup is the whole editor frozen at quit, teardown never reached, terminal
            // left on the alternate screen. So the hangup is the offer, and SIGKILL is the
            // deadline: a short grace for the shell to leave on its own terms, then the signal
            // no process can decline — which is what makes the wait below finite.
            let _ = self.child.kill();
            let mut grace = 6u8;
            while grace > 0 && matches!(self.child.try_wait(), Ok(None)) {
                std::thread::sleep(std::time::Duration::from_millis(50));
                grace -= 1;
            }
            #[cfg(unix)]
            if matches!(self.child.try_wait(), Ok(None)) {
                if let Some(pid) = self.child.process_id() {
                    // SAFETY: signalling a pid this pane spawned and has not yet waited, so it
                    // cannot have been recycled.
                    unsafe {
                        libc::kill(pid as libc::pid_t, libc::SIGKILL);
                    }
                }
            }
        }
        let _ = self.child.wait();
    }

    /// Sends SIGHUP to whichever process group has the pane's tty, the way a closing terminal
    /// window does.
    ///
    /// The foreground group is asked of the tty rather than assumed: with `npm run dev` running,
    /// it is the group of `npm`, and that is the one that has to hear about the window closing.
    /// When the tty cannot say — it is being torn down, or the shell already left — the shell's
    /// own pid stands in, which is its group's id since portable-pty gives each pane a session
    /// of its own. Group ids of 0 and 1 are refused on the way past: 0 means "this process's
    /// group", and signalling that would hang up CleeCode itself.
    #[cfg(unix)]
    fn hangup_foreground_group(&self) {
        let group = self
            .master
            .process_group_leader()
            .or_else(|| self.child.process_id().map(|pid| pid as libc::pid_t))
            .filter(|pid| *pid > 1);
        if let Some(group) = group {
            // SAFETY: `killpg` only delivers a signal to a process group id, and the id here is
            // one this pane owns — read from its own tty, or its own child's. A group that has
            // already gone comes back as an error, which is exactly as good an answer.
            unsafe {
                libc::killpg(group, libc::SIGHUP);
            }
        }
    }

    #[cfg(not(unix))]
    fn hangup_foreground_group(&self) {}

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

    /// A button event as the program in this pane asked to be told about it, at cell `(row, col)`
    /// counted from the pane's top-left. `None` when it never asked, or asked for a mode that has
    /// no word for this kind of event.
    ///
    /// The wheel already reached these programs; buttons did not, so htop's function-key bar,
    /// lazygit's panels and a mouse-mode vim's cursor could all be *seen* but not clicked, and
    /// the click silently became the start of a CleeCode text selection instead. Which is still
    /// what it is with Shift held — that is the caller's half of this, and it is the escape hatch
    /// every terminal emulator provides, because a program that has grabbed the mouse otherwise
    /// leaves no way at all to copy text off its screen.
    pub fn mouse_report(
        &self,
        button: u16,
        action: MouseAction,
        row: u16,
        col: u16,
    ) -> Option<Vec<u8>> {
        let parser = lock_poisoned(&self.parser);
        let screen = parser.screen();
        if !mode_reports(screen.mouse_protocol_mode(), action) {
            return None;
        }
        Some(encode_mouse(screen.mouse_protocol_encoding(), button, action, row, col))
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
        let _ = self.master.resize(pty_size(rows, cols));
        lock_poisoned(&self.parser).screen_mut().set_size(rows, cols);
        // A reflow moves every line, and a picture's anchor is a line number. Rather than guess
        // where each one went, they go: a pane that has just changed shape is about to be
        // redrawn by whatever is running in it anyway, and a picture in the wrong place is
        // worse than one that has to be drawn again.
        self.drop_images(|_| false);
    }

    /// Lets go of the pictures the test says are gone, keeping the last one's drawing in
    /// `spare` for whatever is placed next.
    fn drop_images(&mut self, mut keep: impl FnMut(&PaneImage) -> bool) {
        let held = std::mem::take(&mut self.images);
        for image in held {
            if keep(&image) {
                self.images.push(image);
            } else if image.drawn.is_some() {
                // Moved rather than copied: what is wanted is the name the host terminal knows
                // the picture by, and only one picture may hold it at a time.
                self.spare = image.drawn;
            }
        }
    }

    /// Takes up whatever the reader thread has decoded since the last frame.
    pub fn poll_graphics(&mut self) {
        use crate::pane_graphics::Event;
        let events = {
            let mut inbox = lock_poisoned(&self.graphics);
            // Under the lock, with the taking: a picture queued in between would otherwise have
            // its place in the count wiped and the reader thread would run a frame ahead.
            self.graphics_queued.store(0, Ordering::Relaxed);
            std::mem::take(&mut *inbox)
        };
        for event in events {
            match event {
                Event::Place(placement) => {
                    // A picture placed where one already is replaces it rather than stacking on
                    // it. That is how a preview pane showing one thumbnail after another
                    // behaves, and without it the list of pictures would grow with every arrow
                    // key until the oldest were being dropped for no reason.
                    let (anchor, col) = (placement.anchor, placement.col);
                    self.drop_images(|held| {
                        !(held.placement.anchor == anchor && held.placement.col == col)
                    });
                    self.images.push(PaneImage { placement: *placement, drawn: None });
                    // Oldest first, because the oldest is the one nearest the top of the pane
                    // and so the one about to scroll off anyway.
                    while self.images.len() > crate::pane_graphics::MAX_PLACED {
                        let gone = self.images.remove(0);
                        self.spare = gone.drawn.or_else(|| self.spare.take());
                    }
                }
                Event::Forget { id, all } => {
                    self.drop_images(|held| !all && held.placement.id != id);
                }
                Event::ClearScreen { alternate } => {
                    self.drop_images(|held| held.placement.alternate != alternate);
                }
                Event::Unsupported => self.graphics_note = Some(GraphicsNote::Unsupported),
            }
        }
    }

    /// Where this pane's pictures are on screen right now, dropping the ones that no longer
    /// belong there.
    ///
    /// Three things end a picture, and all three are asked here rather than tracked:
    ///
    /// * the text it was placed among scrolled off, so its line number is behind the top of the
    ///   screen — or ahead of the bottom, which a full-screen program switching grids does;
    /// * the pane changed grids. A picture put up by `fzf` on the alternate screen has nothing
    ///   to do with the shell underneath it, and vice versa;
    /// * something was *written* over it. This is the one that matters for a preview pane: a
    ///   kitty terminal keeps a picture until it is told otherwise, but a list being scrolled
    ///   does not tell it anything — it simply draws the next entry over the top. Written, and
    ///   not merely touched: see `covered`.
    ///
    /// That last one is settled cell by cell rather than all at once, because a kitty terminal
    /// keeps text and pictures in two layers and a pane has one grid for both. Where they meet,
    /// the picture gives up what is actually taken and keeps the rest — see `fit_into_free_cells`
    /// for the two shapes that turns out to have, and for the two programs that are the reason
    /// it has to.
    ///
    /// Returns an index into `images` with the rectangle to draw it in, so the caller can let
    /// go of the parser's lock before it draws.
    pub fn graphics_layout(&mut self, content: Rect) -> Vec<(usize, Rect)> {
        if self.images.is_empty() {
            return Vec::new();
        }
        let mut parser = lock_poisoned(&self.parser);
        let held = held_lines(&mut parser);
        let screen = parser.screen();
        let (screen_rows, screen_cols) = screen.size();
        let alternate = screen.alternate_screen();

        let mut rects = Vec::new();
        let mut keep = Vec::with_capacity(self.images.len());
        for (index, held_image) in self.images.iter().enumerate() {
            let placement = &held_image.placement;
            let on_screen = placement.alternate == alternate
                && placement.anchor >= held
                && placement.anchor - held < usize::from(screen_rows);
            if !on_screen {
                    keep.push(false);
                continue;
            }
            let row = (placement.anchor - held) as u16;
            // Clipped to the pane rather than refused by it: a picture wider than the pane is
            // still worth the part of it that fits, which is what a terminal does too.
            let width = placement.cols.min(screen_cols.saturating_sub(placement.col));
            let height = placement.rows.min(screen_rows - row);
            if width == 0 || height == 0 {
                keep.push(false);
                continue;
            }
            // Where the picture actually fits among the cells it was given.
            let Some((down, across, tall, wide)) = fit_into_free_cells(height, width, |r, c| {
                screen.cell(row + r, placement.col + c).is_some_and(covered)
            }) else {
                keep.push(false);
                continue;
            };
            keep.push(true);
            rects.push((
                index,
                Rect {
                    x: content.x + placement.col + across,
                    y: content.y + row + down,
                    width: wide,
                    height: tall,
                },
            ));
        }
        drop(parser);

        if keep.iter().any(|alive| !alive) {
            // Dropping shifts the indices just collected, so they are renumbered rather than
            // recomputed: the rectangles are still right, only their places in the list moved.
            let mut moved = Vec::with_capacity(rects.len());
            let mut next = 0usize;
            let mut map = Vec::with_capacity(keep.len());
            for alive in &keep {
                map.push(if *alive { Some(next) } else { None });
                if *alive {
                    next += 1;
                }
            }
            let mut alive = keep.iter();
            self.drop_images(|_| *alive.next().unwrap_or(&false));
            for (index, rect) in rects {
                if let Some(Some(index)) = map.get(index) {
                    moved.push((*index, rect));
                }
            }
            return moved;
        }
        rects
    }

    /// Draws one of this pane's pictures, building its protocol the first time.
    pub fn draw_graphic(&mut self, f: &mut ratatui::Frame, index: usize, rect: Rect) {
        use ratatui::widgets::StatefulWidget;
        let Some(picker) = crate::preview::pane_picker() else { return };
        // Taken before the picture is borrowed, and put back untouched if it is not wanted.
        let spare = self.spare.take();
        let Some(held) = self.images.get_mut(index) else {
            self.spare = spare;
            return;
        };
        if held.drawn.is_none() {
            let image = held.placement.image.clone();
            held.drawn = Some(match spare {
                // The drawing of a picture that has gone, carrying the name the host terminal
                // knows it by — see `spare`. This is the same swap `preview::redraw_as` makes
                // when a page of a document is replaced by the next one.
                Some(spare) => Box::new(ratatui_image::protocol::StatefulProtocol::new(
                    image,
                    picker.font_size(),
                    spare.background_color(),
                    spare.protocol_type_owned(),
                )),
                None => Box::new(picker.new_resize_protocol(image)),
            });
        } else {
            self.spare = spare;
        }
        let Some(protocol) = held.drawn.as_mut() else { return };
        // `Fit` shrinks the picture to the cells it was given and never enlarges it, which is
        // what the program asking for `c` columns and `r` rows meant.
        ratatui_image::StatefulImage::default()
            .resize(ratatui_image::Resize::Fit(None))
            .render(rect, f.buffer_mut(), protocol.as_mut());
    }
}

/// Closing a pane ends what was running in it — by every route there is.
///
/// `Drop` rather than a call at each close site on purpose. A tab closed, a window closed, a
/// workspace opened over the top of the old one, the whole application quitting: they are all
/// just a `TerminalPanel` going out of scope, and each of them used to leak a shell. Anything
/// added later that drops a panel is covered without knowing it has to be.
impl Drop for TerminalPanel {
    fn drop(&mut self) {
        self.shutdown();
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
                // Ctrl+Space is NUL, and has been since the control codes were laid out around
                // the letters: `@` is 0x40, so Ctrl+@ is 0, and space shares that seat on every
                // keyboard. Falling through to the byte below sent a plain 0x20 instead, which is
                // why tmux's alternate prefix and emacs' set-mark did nothing inside a pane.
                if c == ' ' {
                    return meta(vec![0x00]);
                }
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
        KeyCode::Insert => vec![0x1b, b'[', b'2', b'~'],
        KeyCode::F(n) => function_key(n),
        _ => Vec::new(),
    }
}

/// What xterm sends for F1-F12, and therefore what every terminfo entry expects.
///
/// These were missing entirely, and the reason is worth writing down so it is not undone: CleeCode
/// binds nothing of its own to a function key, because an Italian keyboard makes them awkward to
/// reach. That is a rule about *our* shortcuts, and it had quietly become a rule about the pty as
/// well — so the program in the pane never heard F1-F12 either, which is htop's entire bottom bar,
/// mc's, and half of what a curses program offers a user.
///
/// The first four are the VT100 keypad sequences (`\x1bOP`..`\x1bOS`); the rest are numbered CSI
/// sequences, and the numbering skips 22 for reasons that are now only historical. Anything past
/// F12 is left alone rather than guessed at.
fn function_key(n: u8) -> Vec<u8> {
    match n {
        1..=4 => vec![0x1b, b'O', b'P' + (n - 1)],
        5 => b"\x1b[15~".to_vec(),
        6..=10 => format!("\x1b[{}~", 11 + u16::from(n)).into_bytes(),
        11 => b"\x1b[23~".to_vec(),
        12 => b"\x1b[24~".to_vec(),
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
        // Collected here and not left to `Drop` alone, because this is the path a shell that
        // ended on its own takes — `exit`, Ctrl+D, an ssh that dropped — and the name of the
        // method is a promise about the process table. The reader has already seen EOF by the
        // time the flag is set, so the wait returns at once.
        self.tabs.retain_mut(|t| {
            if !t.exited.load(Ordering::Relaxed) {
                return true;
            }
            let _ = t.child.wait();
            false
        });
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        }
        !self.tabs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture arrives the way chafa sends one — the keys once, the data in chunks — and
    /// lands where the cursor was, not where the cursor ends up.
    #[test]
    fn a_picture_lands_at_the_cursor_and_moves_it_past_itself() {
        use crate::pane_graphics::Event;
        let parser = Mutex::new(vt100::Parser::new(10, 40, 100));
        let inbox = Mutex::new(Vec::new());
        let mut decoder = crate::pane_graphics::Decoder::default();
        let mut store = crate::pane_graphics::Store::default();
        let mut pace = crate::pane_graphics::Pace::default();
        let queued = AtomicUsize::new(0);

        // Two lines of output first, so the cursor is somewhere worth testing.
        lock_poisoned(&parser).process(b"one\r\ntwo\r\n");
        assert_eq!(lock_poisoned(&parser).screen().cursor_position(), (2, 0));

        // A 2x1 RGB picture covering one cell, announced and then sent.
        let feed = |body: &[u8], decoder: &mut _, store: &mut _, pace: &mut _| {
            take_graphics(body, decoder, store, pace, &parser, &inbox, &queued)
        };
        feed(b"Ga=T,f=24,s=2,v=1,c=3,r=2,m=1;", &mut decoder, &mut store, &mut pace);
        feed(b"Gm=1;/wAAAP8A", &mut decoder, &mut store, &mut pace);
        feed(b"Gm=0;", &mut decoder, &mut store, &mut pace);

        let events = lock_poisoned(&inbox);
        let [Event::Place(placement)] = &events[..] else { panic!("expected one placement") };
        assert_eq!(placement.anchor, 2, "the third line the pane ever printed");
        assert_eq!(placement.col, 0);
        assert_eq!((placement.cols, placement.rows), (3, 2));
        assert!(!placement.alternate);
        // Down by one row less than the picture is tall, and right by its width: where a kitty
        // terminal would have left it, so a newline after it lands below the picture.
        assert_eq!(lock_poisoned(&parser).screen().cursor_position(), (3, 3));
    }

    /// `C=1` is a program saying it will do its own positioning.
    #[test]
    fn a_picture_asked_to_keep_the_cursor_keeps_it() {
        let parser = Mutex::new(vt100::Parser::new(10, 40, 100));
        let inbox = Mutex::new(Vec::new());
        let mut decoder = crate::pane_graphics::Decoder::default();
        let mut store = crate::pane_graphics::Store::default();
        let mut pace = crate::pane_graphics::Pace::default();
        let queued = AtomicUsize::new(0);
        take_graphics(
            b"Ga=T,f=24,s=2,v=1,c=3,r=2,C=1;/wAAAP8A",
            &mut decoder,
            &mut store,
            &mut pace,
            &parser,
            &inbox,
            &queued,
        );
        assert_eq!(lock_poisoned(&parser).screen().cursor_position(), (0, 0));
    }

    /// What mpv sends between frames, and what anything else sends to tidy up.
    #[test]
    fn a_delete_without_a_target_forgets_everything() {
        use crate::pane_graphics::Event;
        let parser = Mutex::new(vt100::Parser::new(10, 40, 100));
        let inbox = Mutex::new(Vec::new());
        let mut decoder = crate::pane_graphics::Decoder::default();
        let mut store = crate::pane_graphics::Store::default();
        let mut pace = crate::pane_graphics::Pace::default();
        let queued = AtomicUsize::new(0);
        take_graphics(b"Ga=d", &mut decoder, &mut store, &mut pace, &parser, &inbox, &queued);
        let events = lock_poisoned(&inbox);
        assert!(matches!(events[..], [Event::Forget { all: true, .. }]));
    }

    /// A file that is not there is a picture that could not be drawn — said, rather than left
    /// as a pane that appears to have ignored the command.
    #[test]
    fn a_picture_that_cannot_be_drawn_is_reported_rather_than_ignored() {
        use crate::pane_graphics::Event;
        let parser = Mutex::new(vt100::Parser::new(10, 40, 100));
        let inbox = Mutex::new(Vec::new());
        let mut decoder = crate::pane_graphics::Decoder::default();
        let mut store = crate::pane_graphics::Store::default();
        let mut pace = crate::pane_graphics::Pace::default();
        let queued = AtomicUsize::new(0);
        take_graphics(
            b"Ga=T,f=100,t=f;L3RtcC9waWMucG5n",
            &mut decoder,
            &mut store,
            &mut pace,
            &parser,
            &inbox,
            &queued,
        );
        let events = lock_poisoned(&inbox);
        assert!(matches!(events[..], [Event::Unsupported]));
    }

    /// Where a picture goes when something is drawn into the cells it was given. The two
    /// shapes that matter, and the one that ends it.
    #[test]
    fn a_picture_gives_up_the_cells_that_are_taken_and_keeps_the_rest() {
        // Nothing in the way: all of it, where it was put.
        assert_eq!(fit_into_free_cells(11, 40, |_, _| false), Some((0, 0, 11, 40)));
        // mpv's shape — its status line straight across the top row of its own frame.
        assert_eq!(fit_into_free_cells(11, 40, |row, _| row == 0), Some((1, 0, 10, 40)));
        // And across the bottom, where another player might put it.
        assert_eq!(fit_into_free_cells(11, 40, |row, _| row == 10), Some((0, 0, 10, 40)));
        // A line through the middle leaves the taller half.
        assert_eq!(fit_into_free_cells(11, 40, |row, _| row == 4), Some((5, 0, 6, 40)));
        // fzf's shape — the border down the side of its preview window, which is not text on
        // the picture but furniture beside it. Given back, and the rows all kept.
        assert_eq!(fit_into_free_cells(11, 40, |_, col| col == 39), Some((0, 0, 11, 39)));
        assert_eq!(fit_into_free_cells(11, 40, |_, col| col == 0), Some((0, 1, 11, 39)));
        // Both at once, which is a film inside a bordered pane.
        assert_eq!(fit_into_free_cells(11, 40, |row, col| row == 0 || col == 39), Some((1, 0, 10, 39)));
        // A column standing in the middle is not furniture: it really does cut the picture, and
        // the rows it is on are the rows that go.
        assert_eq!(fit_into_free_cells(11, 40, |_, col| col == 20), None);
        // Covered from end to end: the picture is gone, which is the preview pane's case.
        assert_eq!(fit_into_free_cells(11, 40, |_, _| true), None);
    }

    /// A film, as `mpv --vo=kitty --vo-kitty-use-shm=yes` actually writes one: every frame is a
    /// delete-everything and then one self-contained command naming a shared-memory segment,
    /// marked `m=1` although nothing more is coming. Frame after frame has to arrive as frame
    /// after frame.
    #[cfg(unix)]
    #[test]
    fn a_film_arrives_as_one_placement_per_frame() {
        use base64::Engine;
        use crate::pane_graphics::Event;

        // A 2x1 RGB frame, in a segment named the way mpv names one: created with a leading
        // slash and transmitted without it.
        let name = format!("/clee-film-{}", std::process::id());
        let created = std::ffi::CString::new(name.clone()).unwrap();
        let frame = [0xffu8, 0, 0, 0, 0xff, 0];
        let make = || {
            // SAFETY: a nul-terminated name; the descriptor is closed before the call returns.
            unsafe {
                let fd = libc::shm_open(
                    created.as_ptr(),
                    libc::O_CREAT | libc::O_RDWR,
                    0o600 as libc::c_uint,
                );
                assert!(fd >= 0, "could not make a segment to read back");
                libc::ftruncate(fd, frame.len() as libc::off_t);
                libc::write(fd, frame.as_ptr().cast(), frame.len());
                libc::close(fd);
            }
        };
        let sent = base64::engine::general_purpose::STANDARD.encode(name.trim_start_matches('/'));
        let command = format!("Ga=T,t=s,f=24,s=2,v=1,C=1,q=2,m=1;{sent}");

        let parser = Mutex::new(vt100::Parser::new(10, 40, 100));
        let inbox = Mutex::new(Vec::new());
        let mut decoder = crate::pane_graphics::Decoder::default();
        let mut store = crate::pane_graphics::Store::default();
        let mut pace = crate::pane_graphics::Pace::default();
        let queued = AtomicUsize::new(0);

        for _ in 0..3 {
            make();
            take_graphics(b"Ga=d", &mut decoder, &mut store, &mut pace, &parser, &inbox, &queued);
            take_graphics(
                command.as_bytes(),
                &mut decoder,
                &mut store,
                &mut pace,
                &parser,
                &inbox,
                &queued,
            );
        }

        let events = lock_poisoned(&inbox);
        let deletes = events.iter().filter(|e| matches!(e, Event::Forget { all: true, .. })).count();
        let places = events.iter().filter(|e| matches!(e, Event::Place(_))).count();
        assert_eq!(deletes, 3, "each frame clears the one before it");
        // One placement survives rather than three: the frames are all at the same spot, and a
        // frame nobody took up is replaced in the queue instead of piling up in it.
        assert_eq!(places, 1, "only the newest frame is still waiting");
        assert!(
            !events.iter().any(|e| matches!(e, Event::Unsupported)),
            "a shared-memory frame is a picture a pane can draw"
        );
    }

    /// The three size questions chafa asks before it draws anything. The pixel pair is answered
    /// only where the host said what a cell is; silence is the honest reply otherwise, and a
    /// program that gets it falls back to its own guess rather than to a wrong number.
    #[test]
    fn the_size_questions_are_answered_in_the_units_they_were_asked_in() {
        let known =
            PaneMetrics { cursor: (0, 0), size: (24, 80), cell: Some((9, 19)) };
        assert_eq!(TerminalQuery::TextAreaCells.response(known), b"\x1b[8;24;80t".to_vec());
        assert_eq!(TerminalQuery::CellPixels.response(known), b"\x1b[6;19;9t".to_vec());
        assert_eq!(
            TerminalQuery::TextAreaPixels.response(known),
            format!("\x1b[4;{};{}t", 24 * 19, 80 * 9).into_bytes()
        );

        let unknown = PaneMetrics { cursor: (0, 0), size: (24, 80), cell: None };
        assert_eq!(TerminalQuery::TextAreaCells.response(unknown), b"\x1b[8;24;80t".to_vec());
        assert!(TerminalQuery::CellPixels.response(unknown).is_empty());
        assert!(TerminalQuery::TextAreaPixels.response(unknown).is_empty());
    }

    #[test]
    fn the_size_questions_are_recognised_and_window_commands_are_not() {
        let mut scanner = CsiScanner::default();
        let mut out = Vec::new();
        // The three questions, then a retitle and a raise — commands, not questions.
        scanner.feed(b"\x1b[18t\x1b[16t\x1b[14t\x1b[22;0t\x1b[5t", &mut out);
        assert_eq!(
            out,
            vec![
                TerminalQuery::TextAreaCells,
                TerminalQuery::CellPixels,
                TerminalQuery::TextAreaPixels,
            ]
        );
    }

    /// Everything a shell can put in front of a path on the way to saying where it is.
    #[test]
    fn a_shell_saying_where_it_is_is_understood_however_it_says_it() {
        let cases: [(&[u8], Option<&str>); 6] = [
            // Terminated with BEL, and with `ESC \` — both are string terminators and shells
            // in the wild pick either.
            (b"\x1b]7;file://localhost/tmp/here\x07", Some("/tmp/here")),
            (b"\x1b]7;file://localhost/tmp/here\x1b\\", Some("/tmp/here")),
            // No host at all: `file:///path`, which is what fish sends.
            (b"\x1b]7;file:///usr/local\x07", Some("/usr/local")),
            // A space in a folder name arrives encoded and must come back as one folder.
            (b"\x1b]7;file:///tmp/two%20words\x07", Some("/tmp/two words")),
            // A bare path, sent by shells that skip the URI.
            (b"\x1b]7;/tmp/bare\x07", Some("/tmp/bare")),
            // Another machine's folder is not this machine's folder. See `host_is_this_machine`.
            (b"\x1b]7;file://elsewhere/tmp/there\x07", None),
        ];
        for (input, want) in cases {
            let mut osc = Osc7Scanner::default();
            assert_eq!(
                osc.feed(input).as_deref(),
                want.map(std::path::Path::new),
                "{}",
                String::from_utf8_lossy(input)
            );
        }
    }

    /// The sequence arrives split across reads, because a pty hands over whatever happened to be
    /// in the buffer and a prompt is written in more than one write.
    #[test]
    fn a_folder_split_across_two_reads_still_arrives_whole() {
        let mut osc = Osc7Scanner::default();
        assert_eq!(osc.feed(b"\x1b]7;file:///tmp/sp"), None);
        assert_eq!(osc.feed(b"lit\x07").as_deref(), Some(std::path::Path::new("/tmp/split")));
    }

    /// One read holding two prompts has been in two folders, and the one it ended in is the one
    /// the sidebar should show.
    #[test]
    fn the_last_folder_in_a_burst_is_the_one_that_counts() {
        let mut osc = Osc7Scanner::default();
        let found = osc.feed(b"\x1b]7;file:///one\x07ls\r\n\x1b]7;file:///two\x07");
        assert_eq!(found.as_deref(), Some(std::path::Path::new("/two")));
    }

    /// Other OSC strings go past constantly — OSC 0 and 2 set the window title on every prompt —
    /// and none of them names a folder.
    #[test]
    fn an_osc_that_is_not_a_folder_is_left_alone() {
        let mut osc = Osc7Scanner::default();
        assert_eq!(osc.feed(b"\x1b]0;a title\x07"), None);
        assert_eq!(osc.feed(b"\x1b]2;another\x1b\\"), None);
        // And the scanner is still in working order afterwards.
        assert_eq!(osc.feed(b"\x1b]7;file:///after\x07").as_deref(), Some(std::path::Path::new("/after")));
    }

    /// A `%` in a path that was never encoded is a literal, and eating it would rename the
    /// folder. An OSC left unterminated must not grow the buffer without end either.
    #[test]
    fn a_stray_percent_survives_and_a_runaway_string_is_capped() {
        assert_eq!(percent_decoded("/tmp/100%/x"), "/tmp/100%/x");
        assert_eq!(percent_decoded("/tmp/%zz"), "/tmp/%zz");
        let mut osc = Osc7Scanner::default();
        osc.feed(b"\x1b]7;");
        osc.feed(&vec![b'a'; OSC_MAX * 2]);
        assert_eq!(osc.payload.len(), OSC_MAX);
    }

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

    /// The unsent twin, which the install offer types: the same cleared line, and *no* carriage
    /// return — because the command it carries is a script downloaded and piped into a shell, and
    /// a stray `\r` there would run a remote program on the user's behalf. This pins the one byte
    /// that is the whole difference between offering the line and executing it.
    #[test]
    fn the_unsent_line_never_submits() {
        let danger = "curl -fsSL https://opencode.ai/install | bash";
        let bytes = typed_line_unsent(danger);
        assert!(bytes.starts_with(LINE_RESET), "the line is still cleared before anything is typed");
        assert!(!bytes.contains(&b'\r'), "not one carriage return: the Enter is the user's");
        assert!(bytes.ends_with(danger.as_bytes()), "and the command is all that is left on the line");
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

    /// A pane whose shell ignores the hangup still closes, and in bounded time.
    ///
    /// This is the fish-at-quit freeze: portable-pty's `kill` only ever sends SIGHUP, /bin/sh
    /// dies to it so every other lifecycle test passed, and an interactive fish shrugged it off
    /// — leaving `wait()` in the panel's Drop parked forever, the quit never finishing, and the
    /// terminal stranded on the alternate screen. The pane here execs a shell that traps HUP
    /// away, which is the same situation made deliberate; the drop must still return, because
    /// the shutdown escalates to the signal no process can decline.
    #[cfg(unix)]
    #[test]
    fn a_shell_that_ignores_the_hangup_cannot_hold_the_pane_open() {
        let mut panel = TerminalPanel::with_startup(
            24,
            80,
            Path::new("/"),
            // `exec` keeps the pid the panel will wait on; single quotes read the same to fish
            // and to sh, so the line survives whatever $SHELL the tests run under.
            Some("exec sh -c 'trap \"\" HUP; echo clee-hup-ignored; sleep 300'"),
        )
        .expect("a shell must be spawnable");
        let pid = panel.child_pid().expect("a spawned shell has a pid");

        // Wait until the trap is provably in place — the echo runs after it — otherwise the
        // drop below races the exec and can pass by hanging up the original, un-trapped shell.
        let deadline = Instant::now() + STARTUP_MAX + Duration::from_secs(8);
        let mut screen = String::new();
        while Instant::now() < deadline {
            panel.flush_pending();
            screen = lock_poisoned(&panel.parser).screen().contents();
            if screen.lines().any(|l| l.trim() == "clee-hup-ignored") {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            screen.lines().any(|l| l.trim() == "clee-hup-ignored"),
            "the trap never ran; the shell's screen was:\n{screen}"
        );

        let closing = Instant::now();
        drop(panel);
        assert!(
            closing.elapsed() < Duration::from_secs(5),
            "dropping the pane took {:?}: the wait is unbounded again",
            closing.elapsed()
        );
        assert!(!alive(pid), "the HUP-proof shell outlived its pane");
    }

    /// A program the shell started is not the shell, and a line sent while it runs would be
    /// keystrokes for it.
    ///
    /// Against a real shell, because that is where it went wrong: the line discipline alone said
    /// `vim` was a prompt — `vim` turns canonical mode off exactly as a line editor does — so an
    /// agent asked to run a command typed it into somebody's open buffer. `cat` stands in for
    /// `vim` here: it holds the terminal the same way and needs nothing installed.
    ///
    /// Each step waits for the state it needs rather than for a duration, and the prompt is
    /// waited for *first* — a pane still running its rc is busy too, and a test that took that
    /// for the program would pass without ever starting one.
    #[cfg(unix)]
    #[test]
    fn a_program_running_in_the_pane_is_not_a_prompt() {
        let mut panel = TerminalPanel::new(24, 80, Path::new("/"))
            .expect("a shell must be spawnable to test one");

        let settle = |panel: &mut TerminalPanel, want: bool, what: &str| {
            let deadline = Instant::now() + STARTUP_MAX + Duration::from_secs(10);
            while Instant::now() < deadline {
                panel.flush_pending();
                if panel.is_at_prompt() == want {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            panic!("{what}; the pane's screen was:\n{}", lock_poisoned(&panel.parser).screen().contents());
        };

        settle(&mut panel, true, "the shell never reached a prompt at all");
        panel.type_line("cat");
        settle(&mut panel, false, "a pane running `cat` read as a prompt, so a command would have gone into it");
        panel.write_input(&[0x04]); // ^D at the start of a line: end of input, and cat exits
        settle(&mut panel, true, "the shell never read as a prompt again after the program ended");
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

    /// Whether a pid still names a process — a signal of 0 is delivered to nobody but is still
    /// checked against the process table, which is the cheapest way to ask.
    #[cfg(unix)]
    fn alive(pid: u32) -> bool {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }

    /// The pids running directly under `parent`, read the same way the rest of the app reads the
    /// process table.
    #[cfg(unix)]
    fn children_of(parent: u32) -> Vec<u32> {
        let mut sys = sysinfo::System::new();
        sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::nothing(),
        );
        sys.processes()
            .values()
            .filter(|p| p.parent() == Some(sysinfo::Pid::from_u32(parent)))
            .map(|p| p.pid().as_u32())
            .collect()
    }

    /// Closing a pane has to end the shell in it. It used to end nothing at all: dropping the
    /// panel closed neither end of the pty, because the reader thread holds a dup of the master,
    /// so the shell went on running for the rest of the session with no window to appear in.
    ///
    /// Reaping is half the claim and it is why the pid is checked rather than the exit status: a
    /// child that was signalled but never waited for is still in the process table as a zombie,
    /// and portable-pty's unix child is a plain `std::process::Child`, which does not collect
    /// one when it is dropped.
    #[cfg(unix)]
    #[test]
    fn closing_a_pane_ends_the_shell_and_collects_it() {
        let panel = TerminalPanel::new(24, 80, Path::new("/")).expect("a shell must be spawnable");
        let pid = panel.child_pid().expect("a spawned shell has a pid");
        assert!(alive(pid), "the shell should be running before the pane is closed");

        drop(panel);

        assert!(!alive(pid), "the shell outlived its pane, or was left behind as a zombie");
    }

    /// And the half that matters in practice: what the pane was *running*.
    ///
    /// A shell signalled on its own leaves its foreground job orphaned but alive — the pane
    /// disappears and the dev server it was running keeps its port. A real terminal hangs up the
    /// whole foreground process group when its window closes, and this pins that we do too.
    #[cfg(unix)]
    #[test]
    fn closing_a_pane_takes_down_what_was_running_in_it() {
        let mut panel = TerminalPanel::with_startup(24, 80, Path::new("/"), Some("sleep 300"))
            .expect("a shell must be spawnable");
        let shell = panel.child_pid().expect("a spawned shell has a pid");

        // As generous as the startup-command test next door, and for the same reason: the wait
        // covers the shell's own rc, and it ends the moment the command is actually running.
        let deadline = Instant::now() + STARTUP_MAX + Duration::from_secs(8);
        let mut job = None;
        while Instant::now() < deadline && job.is_none() {
            panel.flush_pending();
            job = children_of(shell).into_iter().next();
            if job.is_none() {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        let job = job.expect("the startup command never started running");

        drop(panel);

        // The hangup reaches the job through its process group, but the job is not our child, so
        // its parent — init, once the shell is gone — does the collecting on its own schedule.
        let deadline = Instant::now() + Duration::from_secs(5);
        while alive(job) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!alive(job), "the command the pane was running kept going after the pane closed");
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

    /// What a program that asked for the mouse expects a *button* to look like — the half that
    /// was missing, so htop's bottom bar and lazygit's panels could be seen but never clicked.
    #[test]
    fn a_button_is_reported_the_way_the_program_asked() {
        use vt100::MouseProtocolEncoding::{Default, Sgr};
        use MouseAction::{Drag, Press, Release};

        // SGR: the button number, one-based coordinates, and a final letter that says which way
        // it went — the only encoding that can tell a release from a press at all.
        assert_eq!(encode_mouse(Sgr, BUTTON_LEFT, Press, 4, 9), b"\x1b[<0;10;5M".to_vec());
        assert_eq!(encode_mouse(Sgr, BUTTON_LEFT, Release, 4, 9), b"\x1b[<0;10;5m".to_vec());
        assert_eq!(encode_mouse(Sgr, BUTTON_MIDDLE, Press, 0, 0), b"\x1b[<1;1;1M".to_vec());
        assert_eq!(encode_mouse(Sgr, BUTTON_RIGHT, Press, 0, 0), b"\x1b[<2;1;1M".to_vec());
        // Motion is the button plus 32, not a code of its own.
        assert_eq!(encode_mouse(Sgr, BUTTON_LEFT, Drag, 1, 2), b"\x1b[<32;3;2M".to_vec());
        assert_eq!(encode_mouse(Sgr, BUTTON_RIGHT, Drag, 1, 2), b"\x1b[<34;3;2M".to_vec());

        // The old encoding offsets every number by 32, and has one code for "something was
        // released" that never says what.
        assert_eq!(encode_mouse(Default, BUTTON_LEFT, Press, 0, 0), vec![0x1b, b'[', b'M', 32, 33, 33]);
        assert_eq!(encode_mouse(Default, BUTTON_RIGHT, Press, 0, 0), vec![0x1b, b'[', b'M', 34, 33, 33]);
        let released = vec![0x1b, b'[', b'M', 35, 33, 33];
        assert_eq!(encode_mouse(Default, BUTTON_LEFT, Release, 0, 0), released);
        assert_eq!(encode_mouse(Default, BUTTON_RIGHT, Release, 0, 0), released);
        assert_eq!(encode_mouse(Default, BUTTON_LEFT, Drag, 0, 0), vec![0x1b, b'[', b'M', 64, 33, 33]);
    }

    /// A program gets the kinds of event it asked for and no others. X10 mode wants presses
    /// alone, and a release it never asked for is how a menu ends up opening twice.
    #[test]
    fn a_program_hears_only_the_events_its_mode_has_a_word_for() {
        use vt100::MouseProtocolMode as Mode;
        use MouseAction::{Drag, Press, Release};
        for action in [Press, Release, Drag] {
            assert!(!mode_reports(Mode::None, action), "nothing asked for the mouse");
        }
        assert!(mode_reports(Mode::Press, Press));
        assert!(!mode_reports(Mode::Press, Release) && !mode_reports(Mode::Press, Drag));
        assert!(mode_reports(Mode::PressRelease, Release));
        assert!(!mode_reports(Mode::PressRelease, Drag), "no motion without asking for it");
        assert!(mode_reports(Mode::ButtonMotion, Drag) && mode_reports(Mode::AnyMotion, Drag));
    }

    /// The regression, against a real parser: a click had nowhere to go. The wheel already
    /// reached a program that asked for the mouse; the buttons were dropped on the floor and
    /// quietly became the start of a CleeCode text selection instead.
    #[cfg(unix)]
    #[test]
    fn a_program_that_asked_for_the_mouse_gets_the_buttons() {
        let mut panel = TerminalPanel::with_startup(
            10,
            40,
            Path::new("/"),
            // Alternate screen, mouse reporting with motion, SGR encoding — what lazygit or a
            // mouse-mode vim turns on. `sleep` holds it there while this looks at it.
            Some("printf '\\033[?1049h\\033[?1002h\\033[?1006h' && sleep 30"),
        )
        .expect("a shell must be spawnable to test one");

        let deadline = Instant::now() + STARTUP_MAX;
        while Instant::now() < deadline {
            panel.flush_pending();
            if panel.mouse_report(BUTTON_LEFT, MouseAction::Press, 0, 0).is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(
            panel.mouse_report(BUTTON_LEFT, MouseAction::Press, 4, 9).as_deref(),
            Some(&b"\x1b[<0;10;5M"[..]),
            "a press must be handed to a program that asked for it"
        );
        assert_eq!(
            panel.mouse_report(BUTTON_LEFT, MouseAction::Release, 4, 9).as_deref(),
            Some(&b"\x1b[<0;10;5m"[..])
        );
        assert_eq!(
            panel.mouse_report(BUTTON_LEFT, MouseAction::Drag, 4, 9).as_deref(),
            Some(&b"\x1b[<32;10;5M"[..])
        );
    }

    /// A pasted block of shell commands must arrive as a paste, not as typing. Bare, every
    /// newline in it submits a line the moment it lands — which is a copied web page running
    /// itself at your prompt.
    #[test]
    fn a_paste_is_bracketed_only_for_a_program_that_asked_for_it() {
        let text = "cd /tmp\nrm -rf junk\n";
        assert_eq!(
            bracketed_paste(text, true),
            format!("\x1b[200~{text}\x1b[201~").into_bytes(),
            "a program that turned the mode on has to be told where the paste ends"
        );
        // Off, it gets exactly what it used to: to a program that never asked, the guards are
        // not markers, they are six characters of input.
        assert_eq!(bracketed_paste(text, false), text.as_bytes().to_vec());

        // A clipboard carrying the end guard cannot use it to break out of the bracket and have
        // the rest of itself read as typing — which is the attack the mode exists to stop.
        let hostile = "harmless\x1b[201~\nrm -rf ~\n";
        let wrapped = String::from_utf8(bracketed_paste(hostile, true)).unwrap();
        assert_eq!(wrapped.matches("\x1b[201~").count(), 1, "one end, and it is ours");
        assert!(wrapped.ends_with("\x1b[201~"));
        assert!(wrapped.contains("harmless\nrm -rf ~\n"), "the text itself survives: {wrapped:?}");
    }

    /// The other regression the parser can be asked about directly: a pane only brackets a paste
    /// while the program in it has the mode on.
    #[test]
    fn the_parser_decides_whether_a_paste_is_bracketed() {
        let mut parser = vt100::Parser::new(4, 20, 0);
        assert!(!parser.screen().bracketed_paste());
        parser.process(b"\x1b[?2004h");
        assert!(parser.screen().bracketed_paste(), "the mode a shell turns on at its prompt");
        parser.process(b"\x1b[?2004l");
        assert!(!parser.screen().bracketed_paste());
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

        // Ctrl+Space is NUL, not a space with a modifier nobody can see. tmux's alternate prefix
        // and emacs' set-mark both live here, and both used to do nothing at all in a pane.
        assert_eq!(press(KeyCode::Char(' '), KeyModifiers::CONTROL), vec![0x00]);
        assert_eq!(press(KeyCode::Char(' '), KeyModifiers::NONE), vec![b' ']);

        // The function keys, which the pty never saw because CleeCode binds none of its own —
        // a rule about our shortcuts that had become a rule about the shell's input. F1-F4 are
        // the VT100 keypad sequences; the rest are numbered, and the numbering skips 22.
        assert_eq!(press(KeyCode::F(1), KeyModifiers::NONE), b"\x1bOP".to_vec());
        assert_eq!(press(KeyCode::F(4), KeyModifiers::NONE), b"\x1bOS".to_vec());
        assert_eq!(press(KeyCode::F(5), KeyModifiers::NONE), b"\x1b[15~".to_vec());
        assert_eq!(press(KeyCode::F(6), KeyModifiers::NONE), b"\x1b[17~".to_vec());
        assert_eq!(press(KeyCode::F(8), KeyModifiers::NONE), b"\x1b[19~".to_vec());
        assert_eq!(press(KeyCode::F(9), KeyModifiers::NONE), b"\x1b[20~".to_vec());
        assert_eq!(press(KeyCode::F(10), KeyModifiers::NONE), b"\x1b[21~".to_vec());
        assert_eq!(press(KeyCode::F(11), KeyModifiers::NONE), b"\x1b[23~".to_vec());
        assert_eq!(press(KeyCode::F(12), KeyModifiers::NONE), b"\x1b[24~".to_vec());
        // Every one of them distinct, which is the whole point of htop's bottom bar.
        let sent: std::collections::HashSet<Vec<u8>> =
            (1..=12).map(|n| press(KeyCode::F(n), KeyModifiers::NONE)).collect();
        assert_eq!(sent.len(), 12);
        // Past F12 nothing is guessed at.
        assert!(press(KeyCode::F(13), KeyModifiers::NONE).is_empty());

        // Insert, which readline and every editor bind and which was also being swallowed.
        assert_eq!(press(KeyCode::Insert, KeyModifiers::NONE), b"\x1b[2~".to_vec());
    }

    /// A pane must survive a shell that has stopped reading its input.
    ///
    /// This used to be a freeze of the whole editor: the write was done on the UI thread, a pty
    /// whose far end is not reading stops accepting bytes, and `write_all` then waits for a
    /// change that may never come. Here the shell is suspended, so nothing will ever drain the
    /// tty — and the test is simply that these calls return.
    #[cfg(unix)]
    #[test]
    fn a_shell_that_has_stopped_reading_cannot_freeze_the_pane() {
        let mut panel =
            TerminalPanel::new(24, 80, Path::new("/")).expect("a shell must be spawnable");
        let pid = panel.child_pid().expect("a spawned shell has a pid") as i32;
        // SAFETY: a signal to this pane's own child, which is alive for the whole test.
        unsafe { libc::kill(pid, libc::SIGSTOP) };

        // Far more than a pty buffer holds, and far more than the queue: the point is that the
        // caller comes back either way, having dropped what it could not deliver.
        let paste = vec![b'x'; 64 * 1024];
        let deadline = Instant::now() + Duration::from_secs(5);
        for _ in 0..WRITE_QUEUE * 4 {
            panel.write_input(&paste);
        }
        assert!(Instant::now() < deadline, "writing into a stopped shell blocked the caller");

        unsafe { libc::kill(pid, libc::SIGCONT) };
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
        let at = |row, col| PaneMetrics { cursor: (row, col), size: (24, 80), cell: None };
        assert_eq!(TerminalQuery::CursorPosition.response(at(0, 0)), b"\x1b[1;1R");
        assert_eq!(TerminalQuery::CursorPosition.response(at(4, 9)), b"\x1b[5;10R");
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


