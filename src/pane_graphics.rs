//! What a program inside a terminal pane writes that is not text.
//!
//! Two things happen here, and they are one pass over the same bytes because both have to be
//! decided *before* `vt100` sees them.
//!
//! The first is a repair. `vt100` implements CUP (`ESC [ y ; x H`) and not HVP
//! (`ESC [ y ; x f`), which the standard says are the same instruction. Most programs use the
//! first; `btop` uses only the second — five hundred and fifty-six times in three seconds of
//! running — and so every absolute move it made was dropped on the floor, which is why its
//! panels arrived as one long wrapped stream with the box-drawing characters scattered through
//! the numbers. `mpv` positions the same way. The final byte is rewritten in place on the way
//! past: same length, same parameters, an instruction the parser already has.
//!
//! The second is the kitty graphics protocol. Those arrive as APC strings — `ESC _ G ... ESC \`
//! — and `vte`, which `vt100` parses with, swallows an APC in its state machine without ever
//! offering it to a callback: there is no trait method to implement, which is why this has to
//! be a splitter over the stream rather than a parser extension. The payload is decoded to a
//! picture here and handed to the pane, and the pane's drawing puts it back on screen through
//! `ratatui-image` — the same road `preview.rs` already takes for a picture opened in a tab.
//! Drawing it ourselves rather than forwarding the escapes is what lets it keep working where
//! the host terminal has no graphics protocol at all, and over `ssh`.
//!
//! What is deliberately not here is video. `mpv --vo=kitty` writes 7.7 MB a second at 320x240
//! — every frame a whole picture, base64-encoded — and a pane that decoded and re-encoded that
//! at twenty-five frames a second would spend the editor's entire budget on one pane. So there
//! is a flood guard: past `FLOOD_PLACEMENTS` pictures in a second the pane stops decoding and
//! says so, once, instead of melting. See `Flood`.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use base64::Engine;
use image::DynamicImage;

/// The most base64 one picture may carry before it is abandoned. Generous for a thumbnail —
/// eight megabytes of base64 is a six-megabyte image — and a hard stop for a program that
/// opens a transmission and never closes it, which would otherwise grow this buffer for as
/// long as it kept talking.
const MAX_PAYLOAD: usize = 8 * 1024 * 1024;

/// The most pixels a decoded picture may have. The protocol states the dimensions before the
/// data arrives, so an absurd `s=` and `v=` pair can be refused without allocating for it.
const MAX_PIXELS: u64 = 16_000_000;

/// How many pictures a pane may have on it at once. Older placements are dropped first: they
/// are the ones already scrolled towards the top.
pub const MAX_PLACED: usize = 16;

/// How many transmitted-but-not-yet-placed pictures a pane keeps for a later `a=p`. Small on
/// purpose — the store exists so a program *may* split transmission from placement, not so it
/// can use the pane as an image cache.
const MAX_STORED: usize = 8;

/// Pictures per second above which a pane stops decoding.
///
/// This is the line between a thumbnail and a video, and it is drawn here rather than argued
/// about: eight pictures a second is far more than any preview pane asks for and far less than
/// any player wants, so nothing legitimate is near it from either side.
const FLOOD_PLACEMENTS: usize = 8;

/// How long a pane stays shut after a flood, so a player that is still running does not get a
/// picture through every time the window happens to slide.
const FLOOD_COOLDOWN: Duration = Duration::from_secs(2);

/// One step of a pane's output, on its way to the parser.
#[derive(Debug, PartialEq, Eq)]
pub enum Piece {
    /// Ordinary output, with any HVP already rewritten as CUP. Hand it to `vt100`.
    Text(Vec<u8>),
    /// The body of an APC string — everything between `ESC _` and its terminator. Kitty's
    /// commands all begin with `G`; anything else is passed on here and ignored by the reader,
    /// because guessing at somebody else's private sequence is how you corrupt it.
    Apc(Vec<u8>),
    /// The screen was wiped — `CSI 2 J`, or `CSI 3 J` which takes the scrollback with it.
    ///
    /// Reported rather than worked out, because working it out is not possible from the cells.
    /// A picture is otherwise kept for as long as the cells beneath it are empty, and a wipe
    /// makes them *more* empty, not less: `clear` at a prompt would leave the picture sitting
    /// there over a blank screen, which is the one thing everybody notices.
    ClearScreen,
}

/// Where the splitter is in the stream. Kept across reads, because a pty hands over whatever
/// happened to be in the buffer and an escape sequence is split across two of them often
/// enough that a scanner which forgot in between would drop one every few seconds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// Ordinary bytes.
    Ground,
    /// Saw `ESC`, waiting to find out what it introduces.
    Escape,
    /// Inside `ESC [ ... final`. `plain` tracks whether every parameter byte so far has been a
    /// digit or a semicolon, which is what an HVP looks like: a `?`, `>`, `<` or `=` means a
    /// private sequence, and an intermediate byte means something else again, and neither may
    /// have its final byte rewritten.
    Csi { plain: bool },
    /// Inside an APC string, collecting its body.
    Apc,
    /// Saw `ESC` inside an APC string — the first half of `ESC \`, the terminator.
    ApcEscape,
    /// Inside an OSC or DCS string. Not interpreted here (OSC 7 has its own scanner, and
    /// `vt100` handles what it handles), but tracked so that the bytes of somebody's window
    /// title cannot be mistaken for an escape sequence of their own.
    String,
    /// Saw `ESC` inside an OSC or DCS string.
    StringEscape,
}

/// Splits a pane's output into text and graphics, repairing HVP on the way past.
pub struct PaneStream {
    state: State,
    /// Text accumulated since the last piece was emitted.
    text: Vec<u8>,
    /// The body of the APC being collected.
    apc: Vec<u8>,
    /// The parameter bytes of the CSI being collected, so that `CSI 2 J` can be told from the
    /// `CSI 0 J` a prompt writes several times a second. Bounded: a parameter list longer than
    /// this is not one of the two being looked for.
    params: Vec<u8>,
}

impl Default for PaneStream {
    fn default() -> Self {
        PaneStream {
            state: State::Ground,
            text: Vec::new(),
            apc: Vec::new(),
            params: Vec::new(),
        }
    }
}

/// The most of an APC body that is collected before the rest is thrown away. One kitty chunk
/// is capped at 4096 bytes of base64 by the protocol, so this is an order of magnitude of
/// headroom and still a bound on a sequence that never terminates.
const MAX_APC: usize = 64 * 1024;

impl PaneStream {
    /// Takes a chunk as it came off the pty and returns it in order: text to be parsed, and
    /// graphics commands to be read.
    ///
    /// Order is the whole point. A picture is placed at the cursor, and the cursor is wherever
    /// the text *before* the command put it — so the two cannot be separated into two passes
    /// and reunited afterwards.
    pub fn feed(&mut self, data: &[u8]) -> Vec<Piece> {
        let mut out = Vec::new();
        for &b in data {
            match self.state {
                State::Ground => {
                    if b == 0x1b {
                        self.state = State::Escape;
                    }
                    self.text.push(b);
                }
                State::Escape => {
                    self.text.push(b);
                    self.state = match b {
                        b'[' => {
                            self.params.clear();
                            State::Csi { plain: true }
                        }
                        // APC, and the two that behave like it: SOS and PM are string
                        // sequences with the same shape, and collecting one costs nothing.
                        b'_' | b'X' | b'^' => {
                            // The `ESC _` itself never reaches the parser: a sequence being
                            // taken out of the stream has to be taken out whole.
                            self.text.truncate(self.text.len().saturating_sub(2));
                            self.apc.clear();
                            State::Apc
                        }
                        b']' | b'P' => State::String,
                        // Another ESC restarts the sequence rather than ending it.
                        0x1b => State::Escape,
                        _ => State::Ground,
                    };
                }
                State::Csi { plain } => {
                    if (0x40..=0x7e).contains(&b) {
                        // HVP and CUP are the same instruction; only one of them is
                        // implemented downstream, so this is where the other becomes it.
                        self.text.push(if b == b'f' && plain { b'H' } else { b });
                        self.state = State::Ground;
                        if b == b'J' && matches!(self.params.as_slice(), b"2" | b"3") {
                            // After the sequence, not before: the parser has to perform the
                            // wipe, and the pane reads which screen it was on afterwards.
                            flush(&mut out, &mut self.text);
                            out.push(Piece::ClearScreen);
                        }
                    } else {
                        self.text.push(b);
                        if self.params.len() < 16 {
                            self.params.push(b);
                        }
                        // 0x30..=0x3b is digits and `;`. Anything else — a private marker or
                        // an intermediate — means this is not an HVP and must be left alone.
                        self.state = State::Csi { plain: plain && (0x30..=0x3b).contains(&b) };
                    }
                }
                State::Apc => match b {
                    0x1b => self.state = State::ApcEscape,
                    // A bare BEL ends a string sequence in plenty of terminals, and some
                    // programs use it here too.
                    0x07 => {
                        flush(&mut out, &mut self.text);
                        out.push(Piece::Apc(std::mem::take(&mut self.apc)));
                        self.state = State::Ground;
                    }
                    _ => {
                        if self.apc.len() < MAX_APC {
                            self.apc.push(b);
                        }
                    }
                },
                State::ApcEscape => {
                    if b == b'\\' {
                        flush(&mut out, &mut self.text);
                        out.push(Piece::Apc(std::mem::take(&mut self.apc)));
                        self.state = State::Ground;
                    } else {
                        // Not a terminator after all: the ESC belonged to the body.
                        if self.apc.len() < MAX_APC {
                            self.apc.push(0x1b);
                        }
                        self.state = State::Apc;
                        if b == 0x1b {
                            self.state = State::ApcEscape;
                        } else if self.apc.len() < MAX_APC {
                            self.apc.push(b);
                        }
                    }
                }
                State::String => {
                    self.text.push(b);
                    match b {
                        0x1b => self.state = State::StringEscape,
                        0x07 => self.state = State::Ground,
                        _ => {}
                    }
                }
                State::StringEscape => {
                    self.text.push(b);
                    self.state = if b == 0x1b { State::StringEscape } else { State::Ground };
                }
            }
        }
        flush(&mut out, &mut self.text);
        out
    }
}

/// Hands over whatever text has piled up, so that a graphics command emitted next lands after
/// the output that positioned the cursor for it rather than before.
fn flush(out: &mut Vec<Piece>, text: &mut Vec<u8>) {
    if !text.is_empty() {
        out.push(Piece::Text(std::mem::take(text)));
    }
}

/// The control keys of one kitty graphics command, as far as this reads them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Keys {
    /// `a` — what to do: `T` transmit and show, `t` transmit and keep, `p` show a kept one,
    /// `d` forget one (or all).
    pub action: u8,
    /// `f` — 24 for RGB, 32 for RGBA, 100 for a PNG. Anything else is refused.
    pub format: u32,
    /// `t` — where the data is: `d` in the payload. A file or a shared-memory segment is
    /// refused on purpose; see `Decoded::Unsupported`.
    pub medium: u8,
    /// `s`, `v` — the picture's size in pixels, needed for the raw formats and ignored for PNG.
    pub width: u32,
    pub height: u32,
    /// `c`, `r` — how many cells the picture is meant to cover.
    pub cols: u16,
    pub rows: u16,
    /// `i` — the number the program knows this picture by.
    pub id: u32,
    /// `m=1` — more chunks follow.
    pub more: bool,
    /// `C=1` — leave the cursor where it is. Without it the cursor ends up past the picture,
    /// which is what a program printing one after another relies on.
    pub keep_cursor: bool,
    /// `d` — which pictures a delete is about.
    pub delete: u8,
}

impl Keys {
    /// Reads the `key=value,key=value` half of a command.
    fn parse(control: &[u8]) -> Keys {
        let mut keys = Keys::default();
        for field in control.split(|&b| b == b',') {
            let mut halves = field.splitn(2, |&b| b == b'=');
            let (Some(name), Some(value)) = (halves.next(), halves.next()) else { continue };
            let [name] = name else { continue };
            let number = || std::str::from_utf8(value).ok().and_then(|v| v.parse::<u32>().ok());
            let letter = || value.first().copied().unwrap_or(0);
            match name {
                b'a' => keys.action = letter(),
                b'f' => keys.format = number().unwrap_or(0),
                b't' => keys.medium = letter(),
                b's' => keys.width = number().unwrap_or(0),
                b'v' => keys.height = number().unwrap_or(0),
                b'c' => keys.cols = number().unwrap_or(0).min(u32::from(u16::MAX)) as u16,
                b'r' => keys.rows = number().unwrap_or(0).min(u32::from(u16::MAX)) as u16,
                b'i' => keys.id = number().unwrap_or(0),
                b'm' => keys.more = number() == Some(1),
                b'C' => keys.keep_cursor = number() == Some(1),
                b'd' => keys.delete = letter(),
                _ => {}
            }
        }
        keys
    }
}

/// What a finished command turned out to be.
pub enum Decoded {
    /// A picture, and what the command asked to be done with it.
    Picture { keys: Keys, image: DynamicImage },
    /// Show a picture that was transmitted earlier.
    Show(Keys),
    /// Forget one picture, or all of them.
    Forget(Keys),
    /// A command this understands the shape of but cannot carry out: a picture living in a file
    /// or a shared-memory segment, or compressed. Named rather than ignored so the pane can say
    /// so once instead of appearing to do nothing.
    Unsupported,
    /// Nothing to do — a chunk in the middle of a transmission, or a command for a part of the
    /// protocol this does not implement.
    Nothing,
}

/// Collects the chunks of one transmission and decodes the picture at the end of it.
#[derive(Default)]
pub struct Decoder {
    /// The command that opened the transmission, and the base64 gathered since. Kitty sends the
    /// control keys once and then repeats `m=1` with payload only, so the first set has to be
    /// kept until `m=0` closes it.
    pending: Option<(Keys, Vec<u8>)>,
}

impl Decoder {
    /// Reads one APC body.
    pub fn feed(&mut self, body: &[u8]) -> Decoded {
        // Kitty's commands, and only kitty's, begin with `G`.
        let Some(rest) = body.strip_prefix(b"G") else { return Decoded::Nothing };
        let (control, payload) = match rest.iter().position(|&b| b == b';') {
            Some(at) => (&rest[..at], &rest[at + 1..]),
            None => (rest, &[][..]),
        };
        let keys = Keys::parse(control);

        // A chunk in the middle carries only `m=`, so the keys that matter are the ones the
        // transmission opened with.
        if let Some((open, buffer)) = &mut self.pending {
            if buffer.len() + payload.len() > MAX_PAYLOAD {
                self.pending = None;
                return Decoded::Nothing;
            }
            buffer.extend_from_slice(payload);
            if keys.more {
                return Decoded::Nothing;
            }
            let (open, buffer) = (*open, std::mem::take(buffer));
            self.pending = None;
            return finish(open, &buffer);
        }

        match keys.action {
            b'd' => return Decoded::Forget(keys),
            b'p' => return Decoded::Show(keys),
            b'T' | b't' | 0 => {}
            // `a=q` is a capability question, and the honest answer is the one a pane already
            // gives by saying nothing: this is not a kitty terminal, it is a grid of cells.
            // `a=a` is animation, and `a=c` composition; neither is implemented.
            _ => return Decoded::Nothing,
        }

        // Anything but a direct transmission is refused. A file path or a shared-memory name
        // is written from the far end of a pty that may be on another machine entirely, and
        // opening whatever it happens to name on *this* one is not a feature.
        if keys.medium != 0 && keys.medium != b'd' {
            return Decoded::Unsupported;
        }
        if keys.more {
            self.pending = Some((keys, payload.to_vec()));
            return Decoded::Nothing;
        }
        finish(keys, payload)
    }
}

/// Turns a complete base64 payload into a picture.
fn finish(keys: Keys, payload: &[u8]) -> Decoded {
    let pixels = u64::from(keys.width) * u64::from(keys.height);
    if matches!(keys.format, 24 | 32) && (pixels == 0 || pixels > MAX_PIXELS) {
        return Decoded::Unsupported;
    }
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(payload) else {
        return Decoded::Nothing;
    };
    let image = match keys.format {
        24 => image::RgbImage::from_raw(keys.width, keys.height, bytes).map(DynamicImage::ImageRgb8),
        32 => {
            image::RgbaImage::from_raw(keys.width, keys.height, bytes).map(DynamicImage::ImageRgba8)
        }
        // A PNG states its own size, so `s` and `v` are not consulted.
        100 => image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok(),
        _ => return Decoded::Unsupported,
    };
    match image {
        Some(image) => Decoded::Picture { keys, image },
        None => Decoded::Unsupported,
    }
}

/// A picture on a pane, and where it sits.
pub struct Placement {
    /// What the program calls it, so a later delete can name it. Zero when it never said.
    pub id: u32,
    /// Which line of everything the pane has ever printed the picture's top row sits on.
    ///
    /// Not the screen row. A screen row slides upwards as output arrives, and a picture pinned
    /// to one would climb the pane while the text it belongs to stayed put. Counting from the
    /// top of the scrollback instead means the two move together, and that the picture leaves
    /// the screen exactly when its text does.
    pub anchor: usize,
    pub col: u16,
    pub cols: u16,
    pub rows: u16,
    /// Whether it was placed on the alternate screen. A picture put up by a full-screen program
    /// has nothing to do with the one underneath it, so the two sets never mix.
    pub alternate: bool,
    pub image: DynamicImage,
}

/// What the reader thread has to tell the pane about.
pub enum Event {
    Place(Box<Placement>),
    Forget { id: u32, all: bool },
    /// The screen was wiped. Only the pictures on the grid it happened on go: a full-screen
    /// program clearing the alternate screen has nothing to say about the shell underneath it.
    ClearScreen { alternate: bool },
    /// A picture arrived that cannot be drawn — a file, a shared-memory segment, a format this
    /// does not read. Reported so the pane can say so rather than stay blank.
    Unsupported,
    /// Pictures are arriving faster than a pane is for. Said once per flood.
    Flood,
}

/// Counts recent placements, so a pane can tell a preview from a film.
pub struct Flood {
    recent: VecDeque<Instant>,
    /// When the gate closed. Until it reopens, nothing is decoded.
    shut: Option<Instant>,
}

impl Default for Flood {
    fn default() -> Self {
        Flood { recent: VecDeque::new(), shut: None }
    }
}

impl Flood {
    /// Whether a picture may be decoded now, and whether this is the moment the gate shut.
    pub fn admit(&mut self) -> Admission {
        let now = Instant::now();
        if let Some(since) = self.shut {
            if now.duration_since(since) < FLOOD_COOLDOWN {
                return Admission::Refused;
            }
            self.shut = None;
            self.recent.clear();
        }
        while self.recent.front().is_some_and(|t| now.duration_since(*t) > Duration::from_secs(1)) {
            self.recent.pop_front();
        }
        self.recent.push_back(now);
        if self.recent.len() > FLOOD_PLACEMENTS {
            self.shut = Some(now);
            self.recent.clear();
            return Admission::JustShut;
        }
        Admission::Allowed
    }
}

/// The answer to "may this picture be decoded".
#[derive(PartialEq, Eq, Debug)]
pub enum Admission {
    Allowed,
    /// The first refusal of this flood — worth telling the user about, once.
    JustShut,
    /// A later refusal of the same flood. Silent.
    Refused,
}

/// Pictures kept for a later `a=p`, oldest dropped first.
#[derive(Default)]
pub struct Store {
    images: Vec<(u32, DynamicImage)>,
}

impl Store {
    pub fn keep(&mut self, id: u32, image: DynamicImage) {
        self.images.retain(|(kept, _)| *kept != id);
        self.images.push((id, image));
        while self.images.len() > MAX_STORED {
            self.images.remove(0);
        }
    }

    pub fn get(&self, id: u32) -> Option<&DynamicImage> {
        self.images.iter().find(|(kept, _)| *kept == id).map(|(_, image)| image)
    }

    pub fn forget(&mut self, id: u32, all: bool) {
        if all {
            self.images.clear();
        } else {
            self.images.retain(|(kept, _)| *kept != id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(pieces: &[Piece]) -> Vec<u8> {
        pieces
            .iter()
            .filter_map(|p| match p {
                Piece::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .concat()
    }

    /// The whole of btop's problem, in one line: HVP becomes CUP and nothing else moves.
    #[test]
    fn hvp_is_rewritten_as_cup() {
        let mut stream = PaneStream::default();
        assert_eq!(text(&stream.feed(b"\x1b[3;5fhi")), b"\x1b[3;5Hhi".to_vec());
    }

    #[test]
    fn a_private_sequence_ending_in_f_is_left_alone() {
        let mut stream = PaneStream::default();
        assert_eq!(text(&stream.feed(b"\x1b[?3;5f")), b"\x1b[?3;5f".to_vec());
        let mut stream = PaneStream::default();
        assert_eq!(text(&stream.feed(b"\x1b[3 f")), b"\x1b[3 f".to_vec());
    }

    #[test]
    fn a_sequence_split_across_reads_is_still_repaired() {
        let mut stream = PaneStream::default();
        let mut out = text(&stream.feed(b"\x1b[3;"));
        out.extend(text(&stream.feed(b"5f")));
        out.extend(text(&stream.feed(b"x")));
        assert_eq!(out, b"\x1b[3;5Hx".to_vec());
    }

    #[test]
    fn an_f_in_ordinary_text_is_left_alone() {
        let mut stream = PaneStream::default();
        assert_eq!(text(&stream.feed(b"of course")), b"of course".to_vec());
    }

    /// The graphics come out whole and the text around them keeps its order, which is what
    /// puts the picture at the right cursor position.
    #[test]
    fn graphics_are_taken_out_in_order() {
        let mut stream = PaneStream::default();
        let pieces = stream.feed(b"before\x1b_Ga=T,f=24;AAAA\x1b\\after");
        assert_eq!(
            pieces,
            vec![
                Piece::Text(b"before".to_vec()),
                Piece::Apc(b"Ga=T,f=24;AAAA".to_vec()),
                Piece::Text(b"after".to_vec()),
            ]
        );
    }

    #[test]
    fn an_apc_split_across_reads_survives() {
        let mut stream = PaneStream::default();
        assert_eq!(stream.feed(b"\x1b_Ga=T"), vec![]);
        assert_eq!(stream.feed(b",f=24;AA"), vec![]);
        assert_eq!(stream.feed(b"AA\x1b"), vec![]);
        assert_eq!(stream.feed(b"\\"), vec![Piece::Apc(b"Ga=T,f=24;AAAA".to_vec())]);
    }

    /// An OSC carrying a window title must not have its bytes read as escapes — the title is
    /// somebody's text, and `[` and `f` are ordinary characters in it.
    #[test]
    fn a_string_sequence_passes_through_untouched() {
        let mut stream = PaneStream::default();
        let title = b"\x1b]0;a[3;5f title\x07rest";
        assert_eq!(text(&stream.feed(title)), title.to_vec());
    }

    /// `clear` at a prompt, which empties the cells a picture was sitting over — so the
    /// emptiness cannot be what keeps it there.
    #[test]
    fn a_screen_wipe_is_reported() {
        let mut stream = PaneStream::default();
        let pieces = stream.feed(b"\x1b[H\x1b[2J\x1b[3J");
        assert!(pieces.iter().filter(|p| **p == Piece::ClearScreen).count() == 2);
        // And the sequences still reach the parser, which is the half that does the wiping.
        assert_eq!(text(&pieces), b"\x1b[H\x1b[2J\x1b[3J".to_vec());
    }

    /// The erases a prompt writes several times a second are not wipes, and a pane that
    /// treated them as such would lose a picture the moment anything else was printed.
    #[test]
    fn an_ordinary_erase_is_not_a_wipe() {
        let mut stream = PaneStream::default();
        let pieces = stream.feed(b"\x1b[0J\x1b[J\x1b[1J\x1b[K");
        assert!(!pieces.iter().any(|p| *p == Piece::ClearScreen));
    }

    #[test]
    fn keys_are_read_off_a_command() {
        let keys = Keys::parse(b"a=T,f=32,s=18,v=19,c=2,r=1,m=1,q=2");
        assert_eq!(keys.action, b'T');
        assert_eq!(keys.format, 32);
        assert_eq!((keys.width, keys.height), (18, 19));
        assert_eq!((keys.cols, keys.rows), (2, 1));
        assert!(keys.more);
        assert!(!keys.keep_cursor);
    }

    /// chafa's shape: the keys arrive once with an empty payload, the data follows in chunks,
    /// and an empty `m=0` closes it.
    #[test]
    fn a_chunked_transmission_is_reassembled() {
        let mut decoder = Decoder::default();
        // A 2x1 RGB picture is six bytes, "/wAAAP8A" in base64.
        assert!(matches!(decoder.feed(b"Ga=T,f=24,s=2,v=1,c=1,r=1,m=1;"), Decoded::Nothing));
        assert!(matches!(decoder.feed(b"Gm=1;/wAA"), Decoded::Nothing));
        assert!(matches!(decoder.feed(b"Gm=1;AP8A"), Decoded::Nothing));
        let done = decoder.feed(b"Gm=0;");
        let Decoded::Picture { keys, image } = done else { panic!("expected a picture") };
        assert_eq!((image.width(), image.height()), (2, 1));
        assert_eq!((keys.cols, keys.rows), (1, 1));
    }

    #[test]
    fn an_unchunked_transmission_decodes_at_once() {
        let mut decoder = Decoder::default();
        let done = decoder.feed(b"Ga=T,f=24,s=2,v=1;/wAAAP8A");
        assert!(matches!(done, Decoded::Picture { .. }));
    }

    #[test]
    fn a_file_or_shared_memory_picture_is_refused() {
        let mut decoder = Decoder::default();
        assert!(matches!(decoder.feed(b"Ga=T,f=24,t=f,s=2,v=1;L3RtcC94"), Decoded::Unsupported));
        let mut decoder = Decoder::default();
        assert!(matches!(decoder.feed(b"Ga=T,f=24,t=s,s=2,v=1;L3RtcC94"), Decoded::Unsupported));
    }

    #[test]
    fn a_delete_is_recognised() {
        let mut decoder = Decoder::default();
        assert!(matches!(decoder.feed(b"Ga=d"), Decoded::Forget(_)));
    }

    #[test]
    fn a_non_kitty_apc_is_ignored() {
        let mut decoder = Decoder::default();
        assert!(matches!(decoder.feed(b"somebody-elses-sequence"), Decoded::Nothing));
    }

    #[test]
    fn a_picture_too_large_to_be_meant_is_refused() {
        let mut decoder = Decoder::default();
        assert!(matches!(decoder.feed(b"Ga=T,f=24,s=60000,v=60000;AAAA"), Decoded::Unsupported));
    }

    /// The line between a preview and a film.
    #[test]
    fn a_flood_shuts_the_gate_once_and_stays_shut() {
        let mut flood = Flood::default();
        for _ in 0..FLOOD_PLACEMENTS {
            assert_eq!(flood.admit(), Admission::Allowed);
        }
        assert_eq!(flood.admit(), Admission::JustShut);
        assert_eq!(flood.admit(), Admission::Refused);
        assert_eq!(flood.admit(), Admission::Refused);
    }

    #[test]
    fn the_store_keeps_the_newest_and_forgets_on_request() {
        let mut store = Store::default();
        for id in 1..=(MAX_STORED as u32 + 2) {
            store.keep(id, DynamicImage::new_rgb8(1, 1));
        }
        assert!(store.get(1).is_none(), "the oldest was dropped");
        assert!(store.get(MAX_STORED as u32 + 2).is_some());
        store.forget(0, true);
        assert!(store.get(MAX_STORED as u32 + 2).is_none());
    }
}
