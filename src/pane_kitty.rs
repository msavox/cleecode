//! Handing a pane's picture to a kitty host ourselves, instead of through `ratatui-image`.
//!
//! This exists for one number. While a film plays in a pane, CleeCode spends thirteen per cent
//! of a core, and sixty-nine per cent of that is the `write` that puts the frame on the host's
//! terminal: 1.36 MB of escape sequence, thirty times a second. The picture `mpv` hands over is
//! already three bytes a pixel, and the kitty graphics protocol will take it that way — `f=24`
//! says "raw RGB", and it is the format the protocol was given for exactly this case.
//! `ratatui-image` does not offer it. Its kitty protocol calls `to_rgba8()` on whatever it is
//! given and writes `f=32`, so every frame pays a full conversion of the picture to add an alpha
//! channel that is then base64-encoded and sent down the wire: a third more bytes for a quarter
//! of the payload that carries nothing. Nothing in the crate can be asked for the other format —
//! `StatefulProtocolTrait` is private and the format is a literal inside `transmit_virtual` — so
//! the transmission is written here.
//!
//! What is written here is *only* the transmission. The placement is the crate's, copied rather
//! than reinvented, because it is subtle and it works: the picture is transmitted once with
//! `a=T,U=1` — a virtual placement, belonging to no cell — and then *placed* by writing Unicode
//! placeholder characters (U+10EEEE) into the cells, each carrying the image's id in its
//! foreground colour and its row in a combining diacritic. That indirection is why a picture in
//! a pane survives ratatui's buffer diffing at all: the escape sequence lives inside a cell's
//! symbol, so a frame that redraws nothing moves no pixels, and a frame that redraws the row
//! carries the picture with it. Break the placement and the pictures stop working; only the
//! bytes between `_G` and `\` are ours.
//!
//! The other half of what this saves does not show up on the wire. On the crate's road every
//! frame is a fresh `StatefulProtocol`, which owns its picture — so the frame is cloned — and
//! which hashes the whole picture on the way in to decide whether it changed. Here the encoder
//! borrows the frame and is told when it changed, so both of those go: per frame, two full
//! passes over a megabyte that produced nothing.
//!
//! What it measured, said plainly, because half of it is not what was expected. On the profiler's
//! film (`scripts/profile_video.py`, four runs of each binary, interleaved) a frame that draws a
//! picture leaves as 1,014,000 bytes instead of 1,358,000 — a quarter less, exactly the alpha
//! channel — and the three drawing stages fall from 158 ms to 55 ms in a twelve-second window:
//! the clone and the hash go to nothing and the encode itself is a third cheaper. The editor's
//! *total* CPU does not move, 13.0-13.2 % of a core before and 12.4-13.1 % after, and the reason
//! is that seventy per cent of it is the `write` onto the pty, which that harness charges by the
//! call and not by the byte: every run ever taken with this profiler, at anything from 548 KB to
//! 743 KB a call, costs about 2 ms of CPU a call. The bytes are genuinely gone — a host that
//! base64-decodes them and uploads a texture does a quarter less work — but this harness's host
//! is a python loop that throws them away, and it cannot show that. See the note at the head of
//! `profile_video.py`, which says the same thing about itself.

use std::fmt::Write as _;
use std::num::NonZeroU16;
use std::sync::{Mutex, OnceLock};

use image::DynamicImage;
use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui_image::FontSize;

/// The lowest image id this hands out, and how many follow it.
///
/// The range has to be one that cannot be mistaken for a picture opened in a tab, because those
/// go out through `ratatui-image` on the same terminal and an id is the *only* thing telling the
/// host which picture is which: two pictures sharing an id are one picture, and the second
/// transmission silently replaces the first. The crate picks its ids with `rand::random::<u32>()`
/// over the whole 32-bit space (`picker.rs`, `new_resize_protocol`), so no range is provably
/// disjoint from it and claiming otherwise would be a lie. What can be said is this: a collision
/// needs a random draw to land on one of the ids *live at that moment*, which is one per pane
/// picture on screen — a handful — so the odds are a handful in four billion per picture opened,
/// and a single redrawn thumbnail is the whole of the damage. The range is high and odd-looking
/// on purpose as well, because the other plausible way for the crate to pick ids is a counter
/// from zero, and a counter would have to hand out three billion pictures before it reached
/// here.
///
/// Ids are recycled rather than consumed (see `Ids`), so the ceiling is the most pane pictures
/// ever on screen at one time and not the number ever drawn. A session that somehow passed it
/// falls back to the crate's road instead of wrapping onto an id somebody else is using.
const BAND_FIRST: u32 = 0xC1EE_0000;
const BAND_LEN: u32 = 4096;

/// The ids handed out and the ids given back.
///
/// A free list rather than a counter because an id is a name the host terminal holds a picture
/// under, and a name that is never reused means the host's image store grows for as long as the
/// session lasts — which is how a player ends up evicting the very picture it is drawing. A slot
/// returns its id when it is dropped, which is when its picture has genuinely left the pane; a
/// slot handed on to the *next* picture in the same place is moved, not dropped, and keeps the
/// name it had. That is the whole trick, and `TerminalPanel::spare` explains why it matters.
struct Ids {
    handed_out: u32,
    returned: Vec<u32>,
}

static IDS: Mutex<Ids> = Mutex::new(Ids { handed_out: 0, returned: Vec::new() });

/// Takes an id out of the band, preferring one that has been given back.
fn take_id() -> Option<u32> {
    let mut ids = IDS.lock().ok()?;
    if let Some(id) = ids.returned.pop() {
        return Some(id);
    }
    if ids.handed_out >= BAND_LEN {
        return None;
    }
    ids.handed_out += 1;
    Some(BAND_FIRST + ids.handed_out - 1)
}

/// Gives an id back to the band.
fn give_back(id: u32) {
    if let Ok(mut ids) = IDS.lock() {
        ids.returned.push(id);
    }
}

/// Whether the host terminal is being spoken to through tmux, and the wrapping that needs.
///
/// The same question `ratatui-image` asks of the environment, asked again here because its
/// answer is kept in a private field of the picker. tmux does not pass an unknown escape
/// sequence through to the terminal underneath it unless it is wrapped in a passthrough of its
/// own, with every escape byte doubled; the three pieces are the opening, the byte to use for
/// `ESC` inside, and the closing.
fn tmux_wrapping() -> (&'static str, &'static str, &'static str) {
    static INSIDE: OnceLock<bool> = OnceLock::new();
    let inside = *INSIDE.get_or_init(|| {
        std::env::var("TERM").is_ok_and(|term| term.starts_with("tmux"))
            || std::env::var("TERM_PROGRAM").is_ok_and(|program| program == "tmux")
    });
    match inside {
        false => ("", "\x1b", ""),
        true => ("\x1bPtmux;", "\x1b\x1b", "\x1b\\"),
    }
}

/// The picture's bytes as the protocol will carry them, and the number that says how to read
/// them.
///
/// The point of the whole exercise is that nothing happens here: a frame from `mpv` arrives as
/// `RgbImage` and leaves as the same bytes, and a picture that already has an alpha channel
/// leaves as its own bytes too. Any other layout — a palette, sixteen bits a channel, greyscale —
/// would have to be converted first, and a conversion is the cost this road exists to avoid, so
/// those are refused and go out through `ratatui-image` as they always did.
fn raw(image: &DynamicImage) -> Option<(&[u8], u32)> {
    match image {
        DynamicImage::ImageRgb8(buffer) => Some((buffer.as_raw(), 24)),
        DynamicImage::ImageRgba8(buffer) => Some((buffer.as_raw(), 32)),
        _ => None,
    }
}

/// Whether this picture can go out on this road, and how many cells it covers if it can.
///
/// The placement carries no size of its own: the host works out how many rows and columns the
/// picture occupies from its pixels and the cell size, so the picture has to be a whole number of
/// cells in both directions or the last row and column would be a fraction of one. That is not a
/// restriction in the case this exists for — a player scales to the rectangle it was given, and
/// `terminal_panel::padded_to` has already made up whatever it left short — and everything else
/// is a picture `ratatui-image` would have resampled anyway, so it is handed back to it untouched
/// rather than resampled here.
///
/// The height is also capped at the number of diacritics the protocol defines, which is the same
/// clamp the crate applies: a row beyond the last one has no character to name it.
pub fn placement(image: &DynamicImage, font: FontSize, area: Rect) -> Option<(u16, u16)> {
    raw(image)?;
    let (cell_width, cell_height) = (u32::from(font.width), u32::from(font.height));
    if cell_width == 0 || cell_height == 0 {
        return None;
    }
    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 || width % cell_width != 0 || height % cell_height != 0 {
        return None;
    }
    let columns = u16::try_from(width / cell_width).ok()?;
    let rows = u16::try_from(height / cell_height).ok()?;
    if columns > area.width || rows > area.height || usize::from(rows) > DIACRITICS.len() {
        return None;
    }
    Some((columns, rows))
}

/// One pane picture's place in the host terminal's image store, kept across frames.
///
/// The slot is the id and the buffers that go with it, and it outlives the pictures that pass
/// through it: a film is a new picture thirty times a second in the same spot, and the host must
/// be told "this replaces that" rather than handed thirty pictures a second it will never be told
/// to forget. Transmitting under an id the host already holds is the protocol's own way of saying
/// it, which is why this is a struct at all and not a function that takes a picture.
pub struct PaneKitty {
    id: u32,
    /// The escape that sets a cell's foreground to the low three bytes of the id. Built once
    /// because it is written on every row of every frame.
    id_color: String,
    /// The top byte of the id, which travels as the third diacritic on each placeholder rather
    /// than in the colour — a foreground colour has only three bytes to carry.
    id_extra: u16,
    /// The size in pixels of the picture the sequence below carries, or `None` when the slot has
    /// just been handed a new picture and does not yet carry it.
    ///
    /// This is the whole of the "has it changed?" test, and it is sound for a reason worth saying
    /// out loud: within one placement the picture's bytes never change after the first frame
    /// padded them, and a *new* picture always arrives as a new placement, which clears this. So
    /// the question is never "are these the same pixels" — which would mean reading them all, the
    /// very hash this road exists to skip — but "is this still the same placement".
    encoded: Option<(u32, u32)>,
    /// The transmit-and-place sequence for the picture on this slot, kept between frames so the
    /// megabyte of base64 is written into an allocation that already exists. A slot that once
    /// held a film keeps a film-sized buffer for as long as it lives, which is the trade.
    transmit: String,
    /// Whether that sequence still has to reach the host. Cleared once it has been written into
    /// the cell buffer, for the same reason the crate tracks it: the picture is transmitted once
    /// and then merely pointed at, and re-sending it every frame would undo the point.
    pending: bool,
    /// One row of placeholders, reused down the picture and across frames.
    row: String,
}

impl Drop for PaneKitty {
    fn drop(&mut self) {
        give_back(self.id);
    }
}

impl PaneKitty {
    /// A slot with an id of its own, or `None` when the band is exhausted — which means the
    /// caller should take the crate's road, not that it should invent an id.
    pub fn new() -> Option<PaneKitty> {
        let id = take_id()?;
        let [id_extra, r, g, b] = id.to_be_bytes();
        Some(PaneKitty {
            id,
            id_color: format!("\x1b[38;2;{r};{g};{b}m"),
            id_extra: u16::from(id_extra),
            encoded: None,
            transmit: String::new(),
            pending: false,
            row: String::new(),
        })
    }

    /// Takes the slot over for a new picture, keeping the id and forgetting what it held.
    ///
    /// Called when the slot is passed from the picture that has gone to the one that replaced it.
    /// Forgetting is what makes the next frame transmit: the id stays, so the host replaces the
    /// picture rather than gaining one.
    pub fn adopt(&mut self) {
        self.encoded = None;
    }

    /// Puts the picture on screen: transmits it if the host has not been given it yet, and writes
    /// the placeholders that place it.
    pub fn draw(&mut self, image: &DynamicImage, area: Rect, buf: &mut Buffer, cells: (u16, u16)) {
        self.encode(image);
        self.place(area, buf, cells);
    }

    /// Builds the transmit sequence for a picture, unless this slot already carries it.
    ///
    /// The shape is the protocol's: an APC string per chunk, at most 4096 base64 characters in
    /// each because that is the most a single command may carry, `m=1` on every chunk but the
    /// last to say more is coming, and `q=2` throughout so the host answers none of it — there is
    /// nobody here to read an answer, and an unread reply would arrive in the editor's own input.
    /// Only the first chunk carries the keys that describe the picture, which is where `f=24`
    /// goes and why this function exists.
    fn encode(&mut self, image: &DynamicImage) {
        let size = (image.width(), image.height());
        if self.encoded == Some(size) {
            return;
        }
        let Some((bytes, format)) = raw(image) else { return };
        let (id, (width, height)) = (self.id, size);
        let (start, escape, end) = tmux_wrapping();

        // 4096 base64 characters is the protocol's limit for one command, and four characters
        // carry three bytes.
        const CHARS_PER_CHUNK: usize = 4096;
        const CHUNK: usize = (CHARS_PER_CHUNK / 4) * 3;

        let chunks = bytes.chunks(CHUNK);
        let count = chunks.len();
        let data = &mut self.transmit;
        data.clear();
        // The keys of the first chunk and the per-chunk framing, so that a frame's worth of
        // base64 lands in one allocation that is then kept for the next frame.
        data.reserve(count * (CHARS_PER_CHUNK + 16 + escape.len() * 2 + end.len()) + 64);
        for (index, chunk) in chunks.enumerate() {
            data.push_str(start);
            write!(data, "{escape}_Gq=2,").unwrap();
            if index == 0 {
                write!(data, "i={id},a=T,U=1,f={format},t=d,s={width},v={height},").unwrap();
            }
            let more = u8::from(count > index + 1);
            write!(data, "m={more};").unwrap();
            base64_simd::STANDARD.encode_append(chunk, data);
            write!(data, "{escape}\\").unwrap();
            data.push_str(end);
        }
        self.encoded = Some(size);
        self.pending = true;
    }

    /// Writes the Unicode placeholders that put the picture in `area`.
    ///
    /// This is `ratatui_image::protocol::kitty::render`, copied because it cannot be called, and
    /// it deserves an explanation rather than a reference. Each row of the picture is one cell in
    /// the buffer whose *symbol* is the whole row: the cursor position is saved, the foreground is
    /// set to the image id, a placeholder character is written carrying the row and column in its
    /// diacritics, the rest of the row follows as bare placeholders which inherit those numbers,
    /// and then the cursor is put back where the terminal expects it to be after a cell of that
    /// width. Every other cell of the row is marked `Skip` so that ratatui's diff does not write
    /// over the middle of the sequence, and the one that carries it is forced to a width of one so
    /// the diff does not decide the row is wider than it is. The transmission, when there is one,
    /// rides in front of the first row's symbol — it is a single cell's worth of text as far as
    /// the buffer is concerned, however many megabytes it happens to be.
    fn place(&mut self, area: Rect, buf: &mut Buffer, cells: (u16, u16)) {
        const UNIT_WIDTH: CellDiffOption = CellDiffOption::ForcedWidth(NonZeroU16::new(1).unwrap());

        let width = area.width.min(cells.0);
        let height = area.height.min(cells.1).min(DIACRITICS.len() as u16);
        if width == 0 || height == 0 {
            return;
        }
        let PaneKitty { id_color, id_extra, transmit, pending, row, .. } = self;
        let filler: String = std::iter::repeat_n('\u{10EEEE}', usize::from(width - 1)).collect();
        // Back to where the terminal would have left the cursor if it had drawn the whole area,
        // since everything above happened between a save and a restore.
        let (right, down) = (area.width - 1, area.height - 1);
        let restore = format!("\x1b[u\x1b[{right}C\x1b[{down}B");

        for y in 0..height {
            row.clear();
            write!(
                row,
                "\x1b[s{id_color}\u{10EEEE}{}{}{}",
                diacritic(y),
                diacritic(0),
                diacritic(*id_extra)
            )
            .unwrap();
            row.push_str(&filler);
            row.push_str(&restore);

            for x in 1..width {
                if let Some(cell) = buf.cell_mut((area.left() + x, area.top() + y)) {
                    cell.set_diff_option(CellDiffOption::Skip);
                }
            }
            if let Some(cell) = buf.cell_mut((area.left(), area.top() + y)) {
                if y == 0 && *pending {
                    // Appended to the transmission and then cut back off again, rather than the
                    // other way round: the transmission is the megabyte, and copying the row onto
                    // the end of it costs a few hundred bytes where copying it onto the front of
                    // the row would cost the megabyte.
                    let end = transmit.len();
                    transmit.push_str(row);
                    cell.set_symbol(transmit).set_diff_option(UNIT_WIDTH);
                    transmit.truncate(end);
                } else {
                    cell.set_symbol(row).set_diff_option(UNIT_WIDTH);
                }
            }
        }
        *pending = false;
    }
}

/// The row and column diacritics, from the kitty graphics protocol's own table.
///
/// Copied from <https://sw.kovidgoyal.net/kitty/_downloads/1792bad15b12979994cd6ecc54c967a6/rowcolumn-diacritics.txt>
/// — the same 297 characters `ratatui-image` carries, and they have to be these characters in
/// this order because the host reads a placeholder's row and column out of the *index* of the
/// diacritic that follows it. See
/// <https://sw.kovidgoyal.net/kitty/graphics-protocol/#unicode-placeholders>.
static DIACRITICS: [char; 297] = [
    '\u{305}',
    '\u{30D}',
    '\u{30E}',
    '\u{310}',
    '\u{312}',
    '\u{33D}',
    '\u{33E}',
    '\u{33F}',
    '\u{346}',
    '\u{34A}',
    '\u{34B}',
    '\u{34C}',
    '\u{350}',
    '\u{351}',
    '\u{352}',
    '\u{357}',
    '\u{35B}',
    '\u{363}',
    '\u{364}',
    '\u{365}',
    '\u{366}',
    '\u{367}',
    '\u{368}',
    '\u{369}',
    '\u{36A}',
    '\u{36B}',
    '\u{36C}',
    '\u{36D}',
    '\u{36E}',
    '\u{36F}',
    '\u{483}',
    '\u{484}',
    '\u{485}',
    '\u{486}',
    '\u{487}',
    '\u{592}',
    '\u{593}',
    '\u{594}',
    '\u{595}',
    '\u{597}',
    '\u{598}',
    '\u{599}',
    '\u{59C}',
    '\u{59D}',
    '\u{59E}',
    '\u{59F}',
    '\u{5A0}',
    '\u{5A1}',
    '\u{5A8}',
    '\u{5A9}',
    '\u{5AB}',
    '\u{5AC}',
    '\u{5AF}',
    '\u{5C4}',
    '\u{610}',
    '\u{611}',
    '\u{612}',
    '\u{613}',
    '\u{614}',
    '\u{615}',
    '\u{616}',
    '\u{617}',
    '\u{657}',
    '\u{658}',
    '\u{659}',
    '\u{65A}',
    '\u{65B}',
    '\u{65D}',
    '\u{65E}',
    '\u{6D6}',
    '\u{6D7}',
    '\u{6D8}',
    '\u{6D9}',
    '\u{6DA}',
    '\u{6DB}',
    '\u{6DC}',
    '\u{6DF}',
    '\u{6E0}',
    '\u{6E1}',
    '\u{6E2}',
    '\u{6E4}',
    '\u{6E7}',
    '\u{6E8}',
    '\u{6EB}',
    '\u{6EC}',
    '\u{730}',
    '\u{732}',
    '\u{733}',
    '\u{735}',
    '\u{736}',
    '\u{73A}',
    '\u{73D}',
    '\u{73F}',
    '\u{740}',
    '\u{741}',
    '\u{743}',
    '\u{745}',
    '\u{747}',
    '\u{749}',
    '\u{74A}',
    '\u{7EB}',
    '\u{7EC}',
    '\u{7ED}',
    '\u{7EE}',
    '\u{7EF}',
    '\u{7F0}',
    '\u{7F1}',
    '\u{7F3}',
    '\u{816}',
    '\u{817}',
    '\u{818}',
    '\u{819}',
    '\u{81B}',
    '\u{81C}',
    '\u{81D}',
    '\u{81E}',
    '\u{81F}',
    '\u{820}',
    '\u{821}',
    '\u{822}',
    '\u{823}',
    '\u{825}',
    '\u{826}',
    '\u{827}',
    '\u{829}',
    '\u{82A}',
    '\u{82B}',
    '\u{82C}',
    '\u{82D}',
    '\u{951}',
    '\u{953}',
    '\u{954}',
    '\u{F82}',
    '\u{F83}',
    '\u{F86}',
    '\u{F87}',
    '\u{135D}',
    '\u{135E}',
    '\u{135F}',
    '\u{17DD}',
    '\u{193A}',
    '\u{1A17}',
    '\u{1A75}',
    '\u{1A76}',
    '\u{1A77}',
    '\u{1A78}',
    '\u{1A79}',
    '\u{1A7A}',
    '\u{1A7B}',
    '\u{1A7C}',
    '\u{1B6B}',
    '\u{1B6D}',
    '\u{1B6E}',
    '\u{1B6F}',
    '\u{1B70}',
    '\u{1B71}',
    '\u{1B72}',
    '\u{1B73}',
    '\u{1CD0}',
    '\u{1CD1}',
    '\u{1CD2}',
    '\u{1CDA}',
    '\u{1CDB}',
    '\u{1CE0}',
    '\u{1DC0}',
    '\u{1DC1}',
    '\u{1DC3}',
    '\u{1DC4}',
    '\u{1DC5}',
    '\u{1DC6}',
    '\u{1DC7}',
    '\u{1DC8}',
    '\u{1DC9}',
    '\u{1DCB}',
    '\u{1DCC}',
    '\u{1DD1}',
    '\u{1DD2}',
    '\u{1DD3}',
    '\u{1DD4}',
    '\u{1DD5}',
    '\u{1DD6}',
    '\u{1DD7}',
    '\u{1DD8}',
    '\u{1DD9}',
    '\u{1DDA}',
    '\u{1DDB}',
    '\u{1DDC}',
    '\u{1DDD}',
    '\u{1DDE}',
    '\u{1DDF}',
    '\u{1DE0}',
    '\u{1DE1}',
    '\u{1DE2}',
    '\u{1DE3}',
    '\u{1DE4}',
    '\u{1DE5}',
    '\u{1DE6}',
    '\u{1DFE}',
    '\u{20D0}',
    '\u{20D1}',
    '\u{20D4}',
    '\u{20D5}',
    '\u{20D6}',
    '\u{20D7}',
    '\u{20DB}',
    '\u{20DC}',
    '\u{20E1}',
    '\u{20E7}',
    '\u{20E9}',
    '\u{20F0}',
    '\u{2CEF}',
    '\u{2CF0}',
    '\u{2CF1}',
    '\u{2DE0}',
    '\u{2DE1}',
    '\u{2DE2}',
    '\u{2DE3}',
    '\u{2DE4}',
    '\u{2DE5}',
    '\u{2DE6}',
    '\u{2DE7}',
    '\u{2DE8}',
    '\u{2DE9}',
    '\u{2DEA}',
    '\u{2DEB}',
    '\u{2DEC}',
    '\u{2DED}',
    '\u{2DEE}',
    '\u{2DEF}',
    '\u{2DF0}',
    '\u{2DF1}',
    '\u{2DF2}',
    '\u{2DF3}',
    '\u{2DF4}',
    '\u{2DF5}',
    '\u{2DF6}',
    '\u{2DF7}',
    '\u{2DF8}',
    '\u{2DF9}',
    '\u{2DFA}',
    '\u{2DFB}',
    '\u{2DFC}',
    '\u{2DFD}',
    '\u{2DFE}',
    '\u{2DFF}',
    '\u{A66F}',
    '\u{A67C}',
    '\u{A67D}',
    '\u{A6F0}',
    '\u{A6F1}',
    '\u{A8E0}',
    '\u{A8E1}',
    '\u{A8E2}',
    '\u{A8E3}',
    '\u{A8E4}',
    '\u{A8E5}',
    '\u{A8E6}',
    '\u{A8E7}',
    '\u{A8E8}',
    '\u{A8E9}',
    '\u{A8EA}',
    '\u{A8EB}',
    '\u{A8EC}',
    '\u{A8ED}',
    '\u{A8EE}',
    '\u{A8EF}',
    '\u{A8F0}',
    '\u{A8F1}',
    '\u{AAB0}',
    '\u{AAB2}',
    '\u{AAB3}',
    '\u{AAB7}',
    '\u{AAB8}',
    '\u{AABE}',
    '\u{AABF}',
    '\u{AAC1}',
    '\u{FE20}',
    '\u{FE21}',
    '\u{FE22}',
    '\u{FE23}',
    '\u{FE24}',
    '\u{FE25}',
    '\u{FE26}',
    '\u{10A0F}',
    '\u{10A38}',
    '\u{1D185}',
    '\u{1D186}',
    '\u{1D187}',
    '\u{1D188}',
    '\u{1D189}',
    '\u{1D1AA}',
    '\u{1D1AB}',
    '\u{1D1AC}',
    '\u{1D1AD}',
    '\u{1D242}',
    '\u{1D243}',
    '\u{1D244}',
];

/// The character that names a row, or the first one for a row beyond the table — which cannot
/// happen here, since `placement` refuses a picture taller than the table, and is written the
/// crate's way rather than as a panic all the same.
#[inline]
fn diacritic(row: u16) -> char {
    *DIACRITICS.get(usize::from(row)).unwrap_or(&DIACRITICS[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use ratatui::layout::Size;

    const FONT: FontSize = FontSize { width: 10, height: 20 };

    fn rgb(width: u32, height: u32) -> DynamicImage {
        let bytes: Vec<u8> = (0..width * height * 3).map(|i| (i % 251) as u8).collect();
        DynamicImage::ImageRgb8(image::RgbImage::from_raw(width, height, bytes).unwrap())
    }

    fn rgba(width: u32, height: u32) -> DynamicImage {
        let bytes: Vec<u8> = (0..width * height * 4).map(|i| (i % 241) as u8).collect();
        DynamicImage::ImageRgba8(image::RgbaImage::from_raw(width, height, bytes).unwrap())
    }

    /// Everything between `m=<n>;` and the escape that ends the chunk, chunk by chunk.
    fn payloads(sequence: &str) -> Vec<&str> {
        sequence
            .split("\x1b_G")
            .skip(1)
            .map(|chunk| {
                let body = chunk.split_once(';').unwrap().1;
                body.split_once('\x1b').unwrap().0
            })
            .collect()
    }

    fn decoded(sequence: &str) -> Vec<u8> {
        payloads(sequence)
            .iter()
            .flat_map(|chunk| base64::engine::general_purpose::STANDARD.decode(chunk).unwrap())
            .collect()
    }

    #[test]
    fn rgb_goes_out_as_itself() {
        let image = rgb(20, 20);
        let mut slot = PaneKitty::new().unwrap();
        slot.encode(&image);
        let header = slot.transmit.split(';').next().unwrap();
        assert!(header.contains("f=24"), "{header}");
        assert!(header.contains("a=T,U=1"), "{header}");
        assert!(header.contains("t=d,s=20,v=20"), "{header}");
        assert!(header.contains("q=2"), "{header}");
        assert!(header.contains(&format!("i={}", slot.id)), "{header}");
        assert_eq!(decoded(&slot.transmit), image.as_bytes());
    }

    #[test]
    fn a_picture_with_alpha_still_goes_out_as_rgba() {
        let image = rgba(20, 20);
        let mut slot = PaneKitty::new().unwrap();
        slot.encode(&image);
        assert!(slot.transmit.split(';').next().unwrap().contains("f=32"));
        assert_eq!(decoded(&slot.transmit), image.as_bytes());
    }

    #[test]
    fn chunks_are_the_protocols_size_and_the_last_one_says_so() {
        let image = rgb(100, 100);
        let mut slot = PaneKitty::new().unwrap();
        slot.encode(&image);
        let chunks = payloads(&slot.transmit);
        assert!(chunks.len() > 2, "a picture this size is more than one chunk");
        assert!(chunks.iter().all(|chunk| chunk.len() <= 4096));
        assert!(chunks.iter().take(chunks.len() - 1).all(|chunk| chunk.len() == 4096));
        let more: Vec<&str> = slot
            .transmit
            .split("\x1b_G")
            .skip(1)
            .map(|chunk| chunk.split_once(';').unwrap().0)
            .map(|keys| if keys.ends_with("m=1") { "more" } else { "last" })
            .collect();
        assert_eq!(more.last(), Some(&"last"));
        assert!(more.iter().take(more.len() - 1).all(|flag| *flag == "more"));
    }

    #[test]
    fn a_picture_of_the_same_size_is_not_encoded_twice() {
        let image = rgb(20, 20);
        let mut slot = PaneKitty::new().unwrap();
        slot.encode(&image);
        slot.place(Rect::new(0, 0, 2, 1), &mut Buffer::empty(Rect::new(0, 0, 4, 4)), (2, 1));
        assert!(!slot.pending, "placing it means the host now has it");
        slot.encode(&image);
        assert!(!slot.pending, "the same placement's picture is already there");
        slot.adopt();
        slot.encode(&image);
        assert!(slot.pending, "a new placement's picture has to be sent again");
    }

    #[test]
    fn only_whole_cells_take_this_road() {
        let area = Rect::new(0, 0, 10, 10);
        assert_eq!(placement(&rgb(20, 40), FONT, area), Some((2, 2)));
        // A height that stops short of the cell grid, which is what `padded_to` is for.
        assert_eq!(placement(&rgb(20, 35), FONT, area), None);
        // Bigger than the rectangle it was given.
        assert_eq!(placement(&rgb(200, 40), FONT, area), None);
        // Greyscale would have to be converted before it could be sent.
        assert_eq!(
            placement(&DynamicImage::ImageLuma8(image::GrayImage::new(20, 40)), FONT, area),
            None
        );
        // No cell size at all, which is a terminal that never answered.
        assert_eq!(placement(&rgb(20, 40), FontSize { width: 0, height: 0 }, area), None);
    }

    #[test]
    fn placeholders_carry_the_id_and_the_rows() {
        let image = rgb(20, 40);
        let mut slot = PaneKitty::new().unwrap();
        let area = Rect::new(1, 1, 2, 2);
        let mut buf = Buffer::empty(Rect { x: 0, y: 0, width: 4, height: 4 });
        slot.draw(&image, area, &mut buf, (2, 2));

        let first = buf[(1, 1)].symbol().to_string();
        assert!(first.starts_with("\x1b_G"), "the transmission rides in front of the first row");
        assert!(first.contains(&slot.id_color));
        assert!(first.contains(diacritic(0)));
        assert_eq!(buf[(2, 1)].diff_option, CellDiffOption::Skip);

        let second = buf[(1, 2)].symbol().to_string();
        assert!(!second.contains("\x1b_G"), "only the first row carries it");
        assert!(second.contains(diacritic(1)), "the second row names itself");

        // A second frame of the same placement points at the picture without sending it again.
        let mut next = Buffer::empty(Rect { x: 0, y: 0, width: 4, height: 4 });
        slot.draw(&image, area, &mut next, (2, 2));
        assert!(!next[(1, 1)].symbol().contains("\x1b_G"));
    }

    #[test]
    fn ids_come_out_of_the_band_and_go_back_into_it() {
        let first = PaneKitty::new().unwrap();
        let second = PaneKitty::new().unwrap();
        for slot in [&first, &second] {
            assert!((BAND_FIRST..BAND_FIRST + BAND_LEN).contains(&slot.id));
            // The low three bytes are the foreground colour, and a black foreground would be
            // read as no id at all.
            assert_ne!(slot.id & 0x00FF_FFFF, 0);
            // The top byte rides as a diacritic, so it has to name one.
            assert!(((slot.id >> 24) as usize) < DIACRITICS.len());
        }
        assert_ne!(first.id, second.id, "two live pictures never share a name");
        let returned = second.id;
        drop(second);
        assert!(
            IDS.lock().unwrap().returned.contains(&returned),
            "a picture that has gone gives its name back"
        );
    }

    /// The size a pane hands over for a film, straight out of the profiler's own run: a 120x40
    /// pty, a four-by-three clip, and `padded_to` having made up the five rows `mpv` left short.
    #[test]
    fn a_film_frame_is_placed_as_the_whole_rectangle() {
        let area = Rect::new(0, 3, 58, 22);
        let image = rgb(580, 440);
        let cells = placement(&image, FONT, area).unwrap();
        assert_eq!(cells, (58, 22));
        let mut buf = Buffer::empty(Rect { x: 0, y: 0, width: 120, height: 40 });
        let mut slot = PaneKitty::new().unwrap();
        slot.draw(&image, area, &mut buf, cells);
        assert_eq!(Size::from(buf.area), Size::new(120, 40));
        // Three bytes a pixel on the wire, not four: the whole point of the road.
        assert_eq!(decoded(&slot.transmit).len(), 580 * 440 * 3);
    }
}
