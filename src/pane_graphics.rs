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
//! Video is here too, which it was not at first. `mpv --vo=kitty` writes 7.7 MB a second at
//! 320x240 — every frame a whole picture, base64-encoded — and a pane that decoded and
//! re-encoded all of that would spend the editor's entire budget on one pane, so the first
//! answer was a flood guard that shut the pane after eight pictures in a second. The protocol's
//! own answer is better and is now implemented instead: `t=s` puts the frame in a shared-memory
//! segment and sends only its name, some four hundred bytes a frame rather than five megabytes,
//! and reading it is a page-mapping and a copy. What remains is a ceiling — `MAX_DECODES_PER_SEC`
//! — and it is the screen's, not the protocol's: the editor draws thirty frames a second at the
//! very most, so a thirty-first picture decoded in the same second is one that would be replaced
//! before anybody saw it. See `Pace`.

use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom};
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

/// A ceiling on decoding that no player is anywhere near, as a backstop for one that is.
///
/// The pacing that actually matters is `Pace::queued` below and needs no number at all. This
/// is only here so that a program which somehow hands over pictures faster than the screen is
/// ever drained cannot keep the reader thread decoding without pause.
const MAX_DECODES_PER_SEC: usize = 120;

/// The most a picture read out of a file or a shared-memory segment may be. Four bytes a pixel
/// at the pixel ceiling — the largest thing that would be accepted afterwards anyway, said here
/// so that a name pointing at something enormous costs a `stat` rather than the read.
const MAX_RAW: usize = (MAX_PIXELS * 4) as usize;

/// One step of a pane's output, on its way to the parser.
#[derive(Debug, PartialEq, Eq)]
pub enum Piece {
    /// Ordinary output, with any HVP already rewritten as CUP. Hand it to `vt100`.
    Text(Vec<u8>),
    /// The body of an APC string — everything between `ESC _` and its terminator. Kitty's
    /// commands all begin with `G`; anything else is passed on here and ignored by the reader,
    /// because guessing at somebody else's private sequence is how you corrupt it.
    Apc(Vec<u8>),
    /// A graphics command was cut off part-way: an `ESC` arrived inside it that was not its
    /// terminator, so the string ended where it stood and what had been collected is gone.
    ///
    /// Reported rather than passed over in silence, because a chunked transmission is only
    /// half in this splitter — the other half is in `Decoder`, which is still holding the
    /// chunks that came before and still waiting for the `m=0` that will now never come. Left
    /// waiting, it reads the *next* frame's opening command as more of the frame that broke,
    /// and the one after that, and a film stops after a fraction of a second while the status
    /// line says a picture could not be drawn. One broken frame should cost one frame.
    ApcBroken,
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
    /// Bytes of a broken graphics command's payload still to be thrown away rather than
    /// printed. See `LITTER`.
    litter: usize,
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
            litter: 0,
            params: Vec::new(),
        }
    }
}

/// How much of a broken graphics command's payload is thrown away rather than printed.
///
/// `mpv` writes its picture to stdout and its status line to stderr, and those are two buffers
/// in front of one pty — so every so often the status line lands in the middle of a frame.
/// Eight times in five hundred megabytes, measured in a plain pty with no CleeCode anywhere
/// near it: this is `mpv`'s own doing and every terminal sees it.
///
/// What every terminal then does is print the rest of the frame, because base64 is printable
/// text and the command that framed it is gone. That is the litter people see around a picture
/// in a terminal, and in a pane it is worse than litter: a pane draws its pictures into the
/// same cells as its text, and a screen full of base64 is a screen with nowhere left to put a
/// picture. One corrupted frame ended the film, and every frame after it was decoded perfectly
/// and dropped for want of an empty cell.
///
/// So the tail of a broken transmission is treated as what it is, which is not text. Thrown
/// away until the `ESC \` that was going to end it turns up, and bounded by this — twice the
/// largest chunk the protocol allows — so that a command broken with nothing following it
/// cannot swallow a shell prompt.
const LITTER: usize = 8 * 1024;

/// How much of an APC body must have arrived before its tail counts as payload rather than as
/// somebody's output. `mpv` signs off with `ESC _ G a=d ;` and no terminator at all, and what
/// follows *that* is the escapes that leave the alternate screen — which must not be thrown
/// away. See the test that pins it.
const LITTER_AFTER: usize = 64;

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
            // The tail of a broken graphics command is counted down a byte at a time, whatever
            // state the byte is handled in, so that a stream of escapes cannot keep the pane
            // swallowing output. `LITTER` says what this is for.
            let littering = self.litter > 0;
            self.litter = self.litter.saturating_sub(1);
            // One byte can need handling twice: an ESC that ends an unterminated string
            // sequence is also the start of whatever comes next, and that next thing has to be
            // read rather than dropped. Only `ApcEscape` ever asks for the second pass, and the
            // state it leaves behind consumes the byte, so this cannot spin.
            let mut again = true;
            while std::mem::take(&mut again) {
                match self.state {
                    State::Ground => {
                        if b == 0x1b {
                            self.state = State::Escape;
                        } else if littering {
                            // The tail of a command that broke: payload, not text.
                            continue;
                        }
                        self.text.push(b);
                    }
                    State::Escape => {
                        // Only two things end the swallowing, and an ordinary escape is
                        // neither. The interruption that broke the command is itself an escape
                        // — that is how it broke it — and it is usually a player repainting a
                        // status line, escapes and text together, with the rest of the picture
                        // still to come after it. Stopping there would put the rest of the
                        // picture back on the screen, which is the whole thing being avoided.
                        if littering {
                            match b {
                                // The terminator the broken command never got to use. It
                                // belongs to that command, so it is not printed either.
                                b'\\' => {
                                    self.litter = 0;
                                    self.text.pop();
                                    self.state = State::Ground;
                                    continue;
                                }
                                // A new command of its own: whatever was left of the old one is
                                // not coming.
                                b'_' | b'X' | b'^' => self.litter = 0,
                                _ => {}
                            }
                        }
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
                            // An ESC inside a string sequence that turns out not to be its
                            // terminator *ends* the sequence. That is what `vte` does — the parser
                            // the rest of a pane runs on — and it is not a nicety. `mpv` signs off
                            // with `ESC _ G a=d ;` and no terminator at all, followed straight away
                            // by the escapes that leave the alternate screen and bring the cursor
                            // back. Reading those as more of the string, which is what this used to
                            // do, swallowed every one of them: the pane stayed on the alternate
                            // screen with the last frame of the video stuck over it for good, and
                            // no amount of typing got it back.
                            //
                            // The half-collected command is dropped rather than guessed at. A kitty
                            // payload is base64 and never contains an ESC, so nothing well-formed
                            // is lost by ending it here.
                            let broken = !self.apc.is_empty();
                            // Long enough to have been carrying a picture, so what follows the
                            // interruption is the rest of that picture and not output.
                            if self.apc.len() > LITTER_AFTER {
                                self.litter = LITTER;
                            }
                            self.apc.clear();
                            if broken {
                                flush(&mut out, &mut self.text);
                                out.push(Piece::ApcBroken);
                            }
                            self.text.push(0x1b);
                            self.state = State::Escape;
                            again = true;
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
    /// `t` — where the data is: `d` in the payload itself, `f` a file, `t` a file to be
    /// deleted once read, `s` a POSIX shared-memory object. For all but `d` the payload is the
    /// *name* rather than the picture; see `load_medium`.
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
    /// `O` — how far into the file or segment the picture starts. Zero, nearly always.
    pub offset: u32,
    /// `S` — how many bytes of it to read. Zero means "to the end", which for a raw format is
    /// worked out from `s` and `v` instead, because those are the exact answer.
    pub size: u32,
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
                b'O' => keys.offset = number().unwrap_or(0),
                b'S' => keys.size = number().unwrap_or(0),
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
    /// A command this understands the shape of but cannot carry out: a format this does not
    /// read, dimensions that cannot be meant, or a file or shared-memory segment that was not
    /// there to be read. Named rather than ignored so the pane can say so once instead of
    /// appearing to do nothing.
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
    ///
    /// A `None` buffer is a transmission being *swallowed*: the pane already has as many
    /// pictures this second as it can show, so the payload is dropped as it arrives rather than
    /// gathered and then thrown away. The chunks still have to be read past to find the end,
    /// which is the whole reason the state is kept at all.
    pending: Option<(Keys, Option<Vec<u8>>)>,
}

impl Decoder {
    /// Lets go of a transmission that is not going to be finished.
    ///
    /// Called when the splitter reports a command cut off part-way. Without it the chunks
    /// gathered so far sit here waiting for a terminator that has been thrown away, and every
    /// command after them is read as more of them.
    pub fn abandon(&mut self) {
        self.pending = None;
    }

    /// Reads one APC body.
    pub fn feed(&mut self, body: &[u8], pace: &mut Pace) -> Decoded {
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
            if let Some(buffer) = buffer {
                if buffer.len() + payload.len() > MAX_PAYLOAD {
                    self.pending = None;
                    return Decoded::Nothing;
                }
                buffer.extend_from_slice(payload);
            }
            if keys.more {
                return Decoded::Nothing;
            }
            let (open, buffer) = (*open, buffer.take());
            self.pending = None;
            return match buffer {
                Some(buffer) => finish(open, &buffer),
                None => Decoded::Nothing,
            };
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

        // Dimensions that cannot be meant are refused here, before anything is read or mapped:
        // the protocol states the size ahead of the data precisely so that it can be.
        if absurd(&keys) {
            return Decoded::Unsupported;
        }

        // A picture that lives somewhere else. The payload is its *name*, not its bytes, and
        // the protocol's chunking does not apply: kitty honours `m` for a direct transmission
        // only, and `mpv` depends on that — it marks every single frame `m=1` and never sends
        // the `m=0` that would close one. Reading `m` here the way the direct path does would
        // gather every frame of a film into one transmission that never ends.
        if keys.medium != 0 && keys.medium != b'd' {
            if !pace.admit() {
                return Decoded::Nothing;
            }
            let Ok(name) = base64::engine::general_purpose::STANDARD.decode(payload) else {
                return Decoded::Nothing;
            };
            let Some(bytes) = load_medium(&keys, &name) else { return Decoded::Unsupported };
            return decode(keys, bytes);
        }

        if !pace.admit() {
            // The chunks of the refused transmission still have to be counted past, or the one
            // after it would be read as more of this.
            if keys.more {
                self.pending = Some((keys, None));
            }
            return Decoded::Nothing;
        }
        if keys.more {
            self.pending = Some((keys, Some(payload.to_vec())));
            return Decoded::Nothing;
        }
        finish(keys, payload)
    }
}

/// Whether the dimensions a command states are ones to refuse before any of its data is looked
/// at. Only the raw formats state their size; a PNG carries its own.
fn absurd(keys: &Keys) -> bool {
    let pixels = u64::from(keys.width) * u64::from(keys.height);
    matches!(keys.format, 24 | 32) && (pixels == 0 || pixels > MAX_PIXELS)
}

/// How many bytes one pixel of a raw format takes, or `None` where the format is not raw.
fn bytes_per_pixel(format: u32) -> Option<usize> {
    match format {
        24 => Some(3),
        32 => Some(4),
        _ => None,
    }
}

/// Turns a complete base64 payload into a picture.
fn finish(keys: Keys, payload: &[u8]) -> Decoded {
    if absurd(&keys) {
        return Decoded::Unsupported;
    }
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(payload) else {
        return Decoded::Nothing;
    };
    decode(keys, bytes)
}

/// Turns the picture's own bytes — however they arrived — into the picture.
fn decode(keys: Keys, bytes: Vec<u8>) -> Decoded {
    let image =
        match keys.format {
            24 => image::RgbImage::from_raw(keys.width, keys.height, bytes)
                .map(DynamicImage::ImageRgb8),
            32 => image::RgbaImage::from_raw(keys.width, keys.height, bytes)
                .map(DynamicImage::ImageRgba8),
            // A PNG states its own size, so `s` and `v` are not consulted.
            100 => image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok(),
            _ => return Decoded::Unsupported,
        };
    match image {
        Some(image) => Decoded::Picture { keys, image },
        None => Decoded::Unsupported,
    }
}

/// Reads a picture out of wherever the command said it lives.
///
/// This used to be refused outright, and the reason given was that a name written from the far
/// end of a pty may have been written on another machine, so opening whatever it happens to
/// name on *this* one is not a feature. That reason does not survive being looked at: the
/// program in the pane runs as the user who is reading the screen, so there is nothing here it
/// could not have read itself and sent down the direct road instead. What it actually cost was
/// video. The protocol has no compression, so a frame of any size is megabytes of base64 —
/// `mpv --vo=kitty` measures at some twenty megabytes a second into a small pane — and `t=s`
/// exists exactly so that it need not be: the frame goes into a shared-memory segment and the
/// escape carries four hundred bytes of name. That is the road that makes a pane able to play
/// a film at all, and it was the one closed.
///
/// What is guarded instead is the shape of what is opened: a regular file and nothing else, so
/// that a name pointing at a fifo or a character device cannot park the reader thread in a read
/// that never returns, and a ceiling on the length so that one pointing at something enormous
/// costs a `stat` rather than the read.
///
/// And the deleting is guarded harder than the reading, because the two are not the same risk.
/// Reading a name a remote program sent shows *you* a file of your own, which you could have
/// opened anyway; deleting one destroys it, and the far end of an `ssh` could not have done
/// that itself. So `t=t` — "read this and throw it away" — throws away only what is in a
/// temporary directory, which is the only place anything made for a single escape sequence has
/// any business being. Kitty draws the same line in the same place.
///
/// `None` where it could not be read, which the caller reports as a picture that could not be
/// drawn rather than as silence.
#[cfg(unix)]
fn load_medium(keys: &Keys, name: &[u8]) -> Option<Vec<u8>> {
    // For a raw format the exact length is known from `s` and `v`, and knowing it is worth more
    // than `S`: a segment is often rounded up to a page and reading the padding back would make
    // the picture the wrong length. A PNG states its own size, so there `S` — or the whole of
    // the file — is the answer.
    let want = match bytes_per_pixel(keys.format) {
        Some(bytes) => {
            (keys.width as usize).checked_mul(keys.height as usize)?.checked_mul(bytes)?
        }
        None => keys.size as usize,
    };
    let offset = u64::from(keys.offset);
    match keys.medium {
        // `t=t` is a file the terminal is expected to delete once it has read it; `t=f` is one
        // it must leave alone.
        b'f' | b't' => {
            use std::os::unix::ffi::OsStrExt;
            let path = std::path::Path::new(std::ffi::OsStr::from_bytes(name));
            read_file(path, offset, want, keys.medium == b't' && temporary(path))
        }
        b's' => read_shared(name, offset, want),
        _ => None,
    }
}

/// Windows reads a picture out of the escape and nowhere else, for now.
///
/// The file half would port — a path is a path — but the segment half is POSIX shared memory
/// and has no counterpart, and the programs this road exists for (`mpv`, `ueberzugpp`, kitty's
/// own `icat`) are not ones anybody runs there. Left as one refusal rather than half a feature.
#[cfg(not(unix))]
fn load_medium(_keys: &Keys, _name: &[u8]) -> Option<Vec<u8>> {
    None
}

/// Whether a path is somewhere a file made for one escape sequence would live, and so
/// somewhere a `t=t` may be taken at its word. Compared as text rather than resolved, because
/// what is being asked is where the program *said* the file was.
#[cfg(unix)]
fn temporary(path: &std::path::Path) -> bool {
    let temp = std::env::temp_dir();
    [temp.as_path(), std::path::Path::new("/tmp"), std::path::Path::new("/var/tmp")]
        .iter()
        .any(|root| path.starts_with(root))
}

/// Reads `want` bytes from `offset` in a file, and deletes it afterwards if asked to.
#[cfg(unix)]
fn read_file(path: &std::path::Path, offset: u64, want: usize, remove: bool) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(path).ok()?;
    // Regular files only. A fifo would block the reader thread until whoever wrote the name
    // decided to say something, which is a pane that stops taking output.
    if !meta.is_file() {
        return None;
    }
    let start = offset.min(meta.len());
    let available = usize::try_from(meta.len() - start).ok()?;
    let want = if want == 0 { available } else { want.min(available) };
    if want == 0 || want > MAX_RAW {
        return None;
    }
    let mut file = std::fs::File::open(path).ok()?;
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = vec![0u8; want];
    let read = file.read_exact(&mut bytes).ok();
    if remove {
        // Deleted whether or not the read worked: `t=t` means the file was made for this one
        // command, and leaving it behind because the read failed is how a temp directory fills.
        let _ = std::fs::remove_file(path);
    }
    read.map(|()| bytes)
}

/// Opens a shared-memory object for reading, spelled the way each platform declares it.
///
/// macOS gives `shm_open` as the variadic C function it is, so the mode is simply not passed;
/// Linux declares the three-argument form and the mode is required, then ignored because
/// `O_CREAT` is not among the flags. Two arguments compiled on the machine this was written on
/// and on no other, which is the sort of thing only a build somewhere else finds.
///
/// # Safety
/// `name` must point at a nul-terminated string.
#[cfg(all(unix, target_os = "macos"))]
unsafe fn shm_open_read(name: *const libc::c_char) -> libc::c_int {
    unsafe { libc::shm_open(name, libc::O_RDONLY) }
}

/// See the macOS spelling above.
///
/// # Safety
/// `name` must point at a nul-terminated string.
#[cfg(all(unix, not(target_os = "macos")))]
unsafe fn shm_open_read(name: *const libc::c_char) -> libc::c_int {
    unsafe { libc::shm_open(name, libc::O_RDONLY, 0) }
}

/// Reads `want` bytes from `offset` in a POSIX shared-memory object, and unlinks it.
///
/// Two things here are not obvious. The first is the unlink: the protocol makes the terminal
/// responsible for the segment the moment it has read it, and a player that is handed its name
/// back frame after frame relies on that — `mpv` recreates it each time, so a terminal that
/// never unlinked would leave one segment per film sitting in the kernel for as long as the
/// machine is up.
///
/// The second is the leading slash. `mpv` creates its segment as `shm_open("/mpv-kitty-0x…")`
/// and then transmits the name with the slash stripped off. glibc's `shm_open` accepts a name
/// spelled either way, so on Linux nobody has ever noticed; macOS does not, and answers
/// `ENOENT` to every frame — which is sound with no picture. So the name is tried as it was
/// sent and then again with the slash put back, and the film plays on both.
#[cfg(unix)]
fn read_shared(name: &[u8], offset: u64, want: usize) -> Option<Vec<u8>> {
    let mut spellings = vec![name.to_vec()];
    if !name.starts_with(b"/") {
        let mut slashed = vec![b'/'];
        slashed.extend_from_slice(name);
        spellings.push(slashed);
    }
    for spelling in spellings {
        let Ok(spelling) = std::ffi::CString::new(spelling) else { continue };
        // SAFETY: a nul-terminated name, which is all `shm_open_read` reads.
        let fd = unsafe { shm_open_read(spelling.as_ptr()) };
        if fd < 0 {
            continue;
        }
        let bytes = read_mapping(fd, offset, want);
        // SAFETY: `fd` came back from `shm_open` above and is closed exactly once.
        unsafe {
            libc::close(fd);
            libc::shm_unlink(spelling.as_ptr());
        }
        return bytes;
    }
    None
}

/// Maps a descriptor and copies the wanted stretch of it out.
#[cfg(unix)]
fn read_mapping(fd: libc::c_int, offset: u64, want: usize) -> Option<Vec<u8>> {
    // SAFETY: `stat` is plain data and is written entirely by `fstat` before it is read.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut stat) } != 0 {
        return None;
    }
    let len = usize::try_from(stat.st_size).ok()?;
    let start = usize::try_from(offset).ok()?.min(len);
    let want = if want == 0 { len - start } else { want.min(len - start) };
    if len == 0 || want == 0 || want > MAX_RAW {
        return None;
    }
    // SAFETY: a read-only mapping of the whole object, unmapped below with the same length.
    let map =
        unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_SHARED, fd, 0) };
    if map == libc::MAP_FAILED {
        return None;
    }
    // SAFETY: `start + want <= len`, which is the length just mapped, and the copy is finished
    // before the mapping goes.
    let bytes = unsafe { std::slice::from_raw_parts(map.cast::<u8>().add(start), want) }.to_vec();
    // SAFETY: the same address and length `mmap` returned, unmapped exactly once.
    unsafe { libc::munmap(map, len) };
    Some(bytes)
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
    Forget {
        id: u32,
        all: bool,
    },
    /// The screen was wiped. Only the pictures on the grid it happened on go: a full-screen
    /// program clearing the alternate screen has nothing to say about the shell underneath it.
    ClearScreen {
        alternate: bool,
    },
    /// A picture arrived that cannot be drawn — a format this does not read, a size that cannot
    /// be meant, a file or segment that was not there. Reported so the pane can say so rather
    /// than stay blank.
    Unsupported,
}

/// Keeps a pane's decoding down to what its screen actually takes up.
///
/// The measure is the screen itself rather than a number of pictures a second, and the
/// difference is visible. A fixed ceiling has to be set *somewhere*, and wherever it is set it
/// beats against the players that are near it: at thirty a second against a thirty-frame film,
/// the window filled up once a second and threw away the three frames it took to slide, which
/// on screen is a smooth film that hitches once a second — worse to watch than a slower one
/// that never does.
///
/// So the question asked instead is "is there already a picture waiting that nobody has seen".
/// If there is, this one would only replace it, and replacing it costs the whole decode for
/// nothing; if there is not, the screen is ready and the picture is worth having. A pane
/// therefore decodes exactly as often as it draws, whatever that turns out to be, and needs to
/// be told nothing about either.
pub struct Pace {
    recent: VecDeque<Instant>,
    /// Pictures decoded and handed over that the screen has not taken up yet. Written by the
    /// pane as it drains its queue, read here.
    queued: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Default for Pace {
    fn default() -> Self {
        Pace::sharing(std::sync::Arc::default())
    }
}

impl Pace {
    /// A pace that watches this counter of pictures still waiting to be drawn.
    pub fn sharing(queued: std::sync::Arc<std::sync::atomic::AtomicUsize>) -> Pace {
        Pace { recent: VecDeque::new(), queued }
    }

    /// Whether a picture may be decoded now.
    pub fn admit(&mut self) -> bool {
        if self.queued.load(std::sync::atomic::Ordering::Relaxed) > 0 {
            return false;
        }
        let now = Instant::now();
        while self.recent.front().is_some_and(|t| now.duration_since(*t) > Duration::from_secs(1)) {
            self.recent.pop_front();
        }
        if self.recent.len() >= MAX_DECODES_PER_SEC {
            return false;
        }
        self.recent.push_back(now);
        true
    }
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

    /// How `mpv` signs off, byte for byte: a delete-everything command left open, and then the
    /// escapes that leave the alternate screen and bring the cursor back. Collected into the
    /// string instead of ending it, those escapes never reach the parser — and the pane is left
    /// on the alternate screen with a frame of video over it that nothing can shift.
    #[test]
    fn an_unterminated_command_ends_at_the_next_escape_instead_of_eating_it() {
        let mut stream = PaneStream::default();
        let tail = b"\x1b_Ga=d;\x1b[?25h\x1b[?1003l\x1b[?1049l\x1b>\x1b[?25h";
        let pieces = stream.feed(tail);
        let text = text(&pieces);
        assert_eq!(
            text,
            b"\x1b[?25h\x1b[?1003l\x1b[?1049l\x1b>\x1b[?25h".to_vec(),
            "everything after the unterminated command has to reach the parser"
        );
        assert!(
            !pieces.iter().any(|p| matches!(p, Piece::Apc(_))),
            "and the half-written command is dropped, not guessed at"
        );
    }

    /// The same, split across reads the way a pty hands it over.
    #[test]
    fn an_unterminated_command_split_across_reads_still_releases_what_follows() {
        let mut stream = PaneStream::default();
        let mut text_out = text(&stream.feed(b"\x1b_Ga=d;"));
        text_out.extend(text(&stream.feed(b"\x1b")));
        text_out.extend(text(&stream.feed(b"[?1049l")));
        assert_eq!(text_out, b"\x1b[?1049l".to_vec());
    }

    /// A well-formed command still ends where it says it does, and an ESC inside the payload
    /// is not something a base64 body can contain anyway.
    #[test]
    fn a_terminated_command_is_unaffected() {
        let mut stream = PaneStream::default();
        let pieces = stream.feed(b"\x1b_Ga=T,f=24;AAAA\x1b\\rest");
        assert_eq!(
            pieces,
            vec![Piece::Apc(b"Ga=T,f=24;AAAA".to_vec()), Piece::Text(b"rest".to_vec())]
        );
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
        let pace = &mut Pace::default();
        // A 2x1 RGB picture is six bytes, "/wAAAP8A" in base64.
        assert!(matches!(decoder.feed(b"Ga=T,f=24,s=2,v=1,c=1,r=1,m=1;", pace), Decoded::Nothing));
        assert!(matches!(decoder.feed(b"Gm=1;/wAA", pace), Decoded::Nothing));
        assert!(matches!(decoder.feed(b"Gm=1;AP8A", pace), Decoded::Nothing));
        let done = decoder.feed(b"Gm=0;", pace);
        let Decoded::Picture { keys, image } = done else { panic!("expected a picture") };
        assert_eq!((image.width(), image.height()), (2, 1));
        assert_eq!((keys.cols, keys.rows), (1, 1));
    }

    #[test]
    fn an_unchunked_transmission_decodes_at_once() {
        let mut decoder = Decoder::default();
        let done = decoder.feed(b"Ga=T,f=24,s=2,v=1;/wAAAP8A", &mut Pace::default());
        assert!(matches!(done, Decoded::Picture { .. }));
    }

    /// `t=f`: the payload names a file, and the picture comes out of it.
    #[cfg(unix)]
    #[test]
    fn a_picture_in_a_file_is_read_from_it() {
        let dir = std::env::temp_dir().join(format!("clee-graphics-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("six-bytes.rgb");
        std::fs::write(&path, [0xff, 0, 0, 0, 0xff, 0]).unwrap();
        let name = base64::engine::general_purpose::STANDARD.encode(path.to_str().unwrap());

        let mut decoder = Decoder::default();
        let command = format!("Ga=T,f=24,t=f,s=2,v=1;{name}");
        let done = decoder.feed(command.as_bytes(), &mut Pace::default());
        let Decoded::Picture { image, .. } = done else { panic!("expected a picture") };
        assert_eq!((image.width(), image.height()), (2, 1));
        assert!(path.exists(), "`t=f` leaves the file where it found it");

        // `t=t` is the same read, and then the file is gone.
        let command = format!("Ga=T,f=24,t=t,s=2,v=1;{name}");
        let done = Decoder::default().feed(command.as_bytes(), &mut Pace::default());
        assert!(matches!(done, Decoded::Picture { .. }));
        assert!(!path.exists(), "`t=t` means the terminal is to delete it");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A `t=t` naming something outside a temporary directory is still read and still drawn —
    /// and the file is still there afterwards. The far end of an `ssh` does not get to delete
    /// your files by asking for a picture.
    #[cfg(unix)]
    #[test]
    fn a_delete_after_reading_is_only_obeyed_in_a_temporary_directory() {
        let dir =
            std::env::current_dir().unwrap().join(format!("clee-keep-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("six-bytes.rgb");
        std::fs::write(&path, [0xff, 0, 0, 0, 0xff, 0]).unwrap();
        let name = base64::engine::general_purpose::STANDARD.encode(path.to_str().unwrap());

        let command = format!("Ga=T,f=24,t=t,s=2,v=1;{name}");
        let done = Decoder::default().feed(command.as_bytes(), &mut Pace::default());
        assert!(matches!(done, Decoded::Picture { .. }), "it is still a picture");
        assert!(path.exists(), "and it is still a file");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `t=s`: how `mpv` sends a film. The picture comes out of the segment, and the segment is
    /// gone afterwards — including when the name arrives without the leading slash it was
    /// created with, which is exactly what `mpv` does.
    #[cfg(unix)]
    #[test]
    fn a_picture_in_shared_memory_is_read_and_the_segment_unlinked() {
        for bare in [false, true] {
            let slashed = format!("/clee-graphics-{}-{}", std::process::id(), u8::from(bare));
            let created = std::ffi::CString::new(slashed.clone()).unwrap();
            // SAFETY: a nul-terminated name; the descriptor is closed below.
            let fd = unsafe {
                libc::shm_open(
                    created.as_ptr(),
                    libc::O_CREAT | libc::O_RDWR,
                    0o600 as libc::c_uint,
                )
            };
            assert!(fd >= 0, "could not make a segment to read back");
            let frame = [0xffu8, 0, 0, 0, 0xff, 0];
            // SAFETY: the object was just made and is sized before it is written.
            unsafe {
                libc::ftruncate(fd, frame.len() as libc::off_t);
                libc::write(fd, frame.as_ptr().cast(), frame.len());
                libc::close(fd);
            }

            let sent =
                if bare { slashed.trim_start_matches('/').to_string() } else { slashed.clone() };
            let name = base64::engine::general_purpose::STANDARD.encode(&sent);
            let command = format!("Ga=T,f=24,t=s,s=2,v=1,C=1,q=2,m=1;{name}");
            let done = Decoder::default().feed(command.as_bytes(), &mut Pace::default());
            let Decoded::Picture { image, .. } = done else {
                panic!("expected a picture from the segment named {sent}")
            };
            assert_eq!((image.width(), image.height()), (2, 1));

            // SAFETY: a nul-terminated name, and unlinking one that is already gone is an error
            // and nothing more.
            let again = unsafe { super::shm_open_read(created.as_ptr()) };
            assert!(again < 0, "the segment should have been unlinked after it was read");
        }
    }

    /// `mpv`'s shape, and the reason `m` cannot be read on this road: every frame says `m=1`
    /// and no frame ever says `m=0`. Each one has to stand on its own.
    #[cfg(unix)]
    #[test]
    fn a_shared_memory_command_is_complete_even_though_it_says_more_follows() {
        let name = base64::engine::general_purpose::STANDARD.encode("clee-graphics-absent");
        let command = format!("Ga=T,t=s,f=24,s=512,v=384,C=1,q=2,m=1;{name}");
        let mut decoder = Decoder::default();
        // Unsupported, not Nothing: the segment is not there, which is a picture that could not
        // be drawn rather than a chunk of one still arriving...
        assert!(matches!(
            decoder.feed(command.as_bytes(), &mut Pace::default()),
            Decoded::Unsupported
        ));
        // ...and nothing was left half-open, so the sign-off that follows is still a delete.
        assert!(matches!(decoder.feed(b"Ga=d", &mut Pace::default()), Decoded::Forget(_)));
    }

    #[test]
    fn a_delete_is_recognised() {
        let mut decoder = Decoder::default();
        assert!(matches!(decoder.feed(b"Ga=d", &mut Pace::default()), Decoded::Forget(_)));
    }

    #[test]
    fn a_non_kitty_apc_is_ignored() {
        let mut decoder = Decoder::default();
        assert!(matches!(
            decoder.feed(b"somebody-elses-sequence", &mut Pace::default()),
            Decoded::Nothing
        ));
    }

    #[test]
    fn a_picture_too_large_to_be_meant_is_refused() {
        let mut decoder = Decoder::default();
        assert!(matches!(
            decoder.feed(b"Ga=T,f=24,s=60000,v=60000;AAAA", &mut Pace::default()),
            Decoded::Unsupported
        ));
    }

    /// A picture nobody has drawn yet is a picture the next one would only replace, so the next
    /// one is not decoded at all. This is the whole of the pacing.
    #[test]
    fn nothing_is_decoded_while_a_picture_is_still_waiting_to_be_drawn() {
        let waiting = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut pace = Pace::sharing(std::sync::Arc::clone(&waiting));
        assert!(pace.admit(), "an empty queue means the screen is ready");
        waiting.store(1, std::sync::atomic::Ordering::Relaxed);
        assert!(!pace.admit());
        assert!(!pace.admit());
        waiting.store(0, std::sync::atomic::Ordering::Relaxed);
        assert!(pace.admit(), "and it is ready again once the picture has gone up");
    }

    /// The backstop, for a program that hands over pictures faster than anything drains them.
    #[test]
    fn decoding_has_a_ceiling_even_when_the_screen_never_falls_behind() {
        let mut pace = Pace::default();
        for _ in 0..MAX_DECODES_PER_SEC {
            assert!(pace.admit());
        }
        assert!(!pace.admit());
        assert!(!pace.admit());
    }

    /// A refused frame must not swallow the one after it: its chunks are read past, and then
    /// the next transmission is a transmission again.
    #[test]
    fn a_refused_transmission_is_counted_past_rather_than_gathered() {
        let waiting = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(1));
        let mut pace = Pace::sharing(waiting);
        let mut decoder = Decoder::default();
        assert!(matches!(decoder.feed(b"Ga=T,f=24,s=2,v=1,m=1;/wAA", &mut pace), Decoded::Nothing));
        assert!(matches!(decoder.feed(b"Gm=0;AP8A", &mut pace), Decoded::Nothing));
        // The gate is still shut, but the decoder is back at the start of a command rather than
        // in the middle of one.
        assert!(matches!(decoder.feed(b"Ga=d", &mut pace), Decoded::Forget(_)));
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
