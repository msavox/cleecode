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
//! It also stopped resizing, which was not part of the original idea and turned out to be the
//! larger half of it. `ratatui-image` resamples every picture to the exact pixel size of the
//! cells it will cover, because its transmission says nothing about cells and the host has to
//! infer them; the transmission written here says `c` and `r`, so the host is told the box and
//! scales the picture into it on the way to the screen. What the editor then does to a frame
//! between `mpv` and the host is nothing at all: it is base64-encoded and written. See
//! `encode`, where those two keys go, for what that changes about how a picture looks.
//!
//! The other half of what this saves does not show up on the wire. On the crate's road every
//! frame is a fresh `StatefulProtocol`, which owns its picture — so the frame is cloned — and
//! which hashes the whole picture on the way in to decide whether it changed. Here the encoder
//! borrows the frame and is told when it changed, so both of those go: per frame, two full
//! passes over a megabyte that produced nothing.
//!
//! Since then the frame has stopped travelling by value at all, where the host will have it the
//! other way. Measured in a real Ghostty at thirty frames a second, a pane playing a film handed
//! over 2.1 MB an escape and spent 66.8 % of everything CleeCode's CPU went on inside one
//! `write`; another 27.6 % was ratatui diffing a cell whose symbol was that same two megabytes,
//! which moves with the same number. So the picture now goes into a POSIX shared-memory segment
//! CleeCode owns and the escape carries only its name — `t=s`, sixty bytes rather than two
//! million — which is the same road `mpv` already uses to hand frames *to* a pane. See `stage`
//! for the ring of segments and who owns one after it has been read, `by_name` for the single
//! question that decides whether the road is taken, and the note on `encode_by` for the shape of
//! the command. Where anything at all stands in the way — a host that never said it could, a
//! picture in a layout that would need converting, Windows, a create or a map that failed on
//! this one frame — what goes out is the `t=d` below, unchanged, and nobody can tell but the
//! clock.
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

/// Whether the frame may be handed to the host by *name* rather than by value.
///
/// One question and it has already been answered: `detect_host_abilities` asked the terminal at
/// startup whether it would take a picture out of a POSIX shared-memory segment, and recorded
/// what it said. Asking again here would be asking a terminal that no longer has stdin to
/// itself — the trap the probe's own note spends a paragraph on — so this is a read of a fact,
/// not a question, and every doubt has already resolved in the direction of `Direct`.
///
/// `Medium::File` is a road the probe can report and this does not take. It would be a write
/// through the filesystem per frame where `Shared` is a copy into a mapping, so it is a
/// different trade with a different answer and it deserves its own measurement rather than
/// being folded in here on the grounds that both are "by name". It falls through to `t=d` for
/// now, which is what it did before any of this existed.
fn by_name() -> bool {
    matches!(crate::preview::host_abilities().medium, crate::preview::Medium::Shared)
}

/// How many segments the ring holds.
///
/// Four, and the number is the answer to two different questions at once. The first is how many
/// frames may be in flight: the escape naming a segment is written into a cell, the cell reaches
/// the host when ratatui flushes, and the host reads it some unknown moment later — so a ring of
/// one would be CleeCode writing frame N+1 into the pages the host is reading frame N out of,
/// which is tearing, intermittently, in exactly the way no test catches. The second is how many
/// pictures may be on screen: the ring is the process's and not the pane's, so two panes each
/// playing a film take two slots of every editor frame, and four leaves each of them a full
/// frame of slack at thirty frames a second.
///
/// It does not want to be larger. A segment is as big as the picture — a megabyte for a film in
/// a reasonable pane — so the ring is the resident cost of having played a video at all, and
/// eight would be eight megabytes bought to insure against a host that is further behind than a
/// host that is still drawing has any business being. It does not want to be smaller either:
/// three would leave two simultaneous films half an editor frame apart, which is the shape of a
/// bug nobody would find until they had two films open.
#[cfg(unix)]
const FRAMES: usize = 4;

/// One slot of the ring: where its segment is mapped, and how long it is.
///
/// The address is an integer rather than a pointer, and that is not squeamishness. The ring is a
/// `static` behind a `Mutex` so that the whole of it is one thing rather than one per pane, and
/// a raw pointer is not `Send`; what actually crosses between threads is an address that is only
/// ever turned back into a pointer by the thread holding the lock, over a mapping that lives
/// until this slot is next rebuilt. A length of zero means the slot has never been used.
#[cfg(unix)]
#[derive(Clone, Copy)]
struct Frame {
    addr: usize,
    len: usize,
}

/// The ring, and which slot the next frame takes.
#[cfg(unix)]
struct Ring {
    next: usize,
    slots: [Frame; FRAMES],
}

#[cfg(unix)]
static RING: Mutex<Ring> = Mutex::new(Ring { next: 0, slots: [Frame { addr: 0, len: 0 }; FRAMES] });

/// What one slot of the ring is called.
///
/// The hard limit is macOS's and it is not generous: `PSHMNAMLEN` is **31 characters including
/// the leading slash**, and `shm_open` answers `ENAMETOOLONG` past it — which arrives here as a
/// segment that was never created, and reads exactly like a host that said no. `preview.rs` has
/// the same note over its probe fixtures and the same arithmetic. This name is
/// `/clee-frame-` (twelve) plus a pid (seven at the very widest anything writes one) plus a dash
/// and one digit of slot: twenty-one, with ten to spare, and there is a test that says so.
///
/// The leading slash is part of the name and stays part of what is transmitted. `mpv` strips it
/// and gets away with it because glibc looks a name up spelled either way; macOS does not, and
/// the note on `read_shared` in `pane_graphics.rs` is the story of finding that out from the
/// receiving end. Sent whole, both hosts look up the same thing.
#[cfg(unix)]
fn frame_name(slot: usize) -> String {
    format!("/clee-frame-{}-{slot}", std::process::id())
}

/// Puts a frame in the next segment of the ring and returns the name to send, or `None` when
/// anything at all went wrong — which the caller reads as "send this one frame by value".
///
/// This is the mirror of `read_shared` and `read_mapping` in `pane_graphics.rs` and is written
/// to look like them, because between them they are the whole of the road: one side creates,
/// truncates, maps and copies in, the other opens, maps, copies out and unlinks.
///
/// **The receiver owns the segment**, which is the one thing about this that is not obvious. The
/// protocol makes the terminal responsible for the object the moment it has read it, and a host
/// doing its job unlinks — so by the time the ring comes round again the name is usually *gone*,
/// and a slot that assumed otherwise would transmit a name resolving to nothing and show a black
/// pane. So the name is created afresh every turn, with `O_CREAT | O_EXCL`, and the interesting
/// case is the one where that *fails*: `EEXIST` means the host did not unlink, the object under
/// that name is still the one this slot holds mapped, and the frame is written straight into it.
/// Bounded either way, which is the whole point of a ring rather than a fresh segment per frame
/// — a host that unlinks costs one create and one map per frame, a host that does not costs
/// neither, and neither of them costs a segment that nobody ever removes.
///
/// The one copy left on this road is the one at the bottom. It could go: the frame arrives from
/// `mpv` in a segment of its own, and reading that segment straight into this one would put the
/// pixels in the right place once instead of twice. It is not done here because the fallback
/// roads — `t=d`, and every terminal that draws with half-blocks or sixel or nothing at all —
/// need the decoded picture in memory, so removing the copy means knowing which road the frame
/// will take before it has been read. That is the next thing available to anyone who comes
/// looking, and `shm_read` says what it is worth: 188 µs a frame, 2.7 % of CPU.
#[cfg(unix)]
fn stage(bytes: &[u8]) -> Option<String> {
    let want = bytes.len();
    if want == 0 {
        return None;
    }
    let mut ring = RING.lock().ok()?;
    let slot = ring.next;
    ring.next = (ring.next + 1) % FRAMES;
    let name = frame_name(slot);
    let spelling = std::ffi::CString::new(name.as_str()).ok()?;
    let held = ring.slots[slot];

    // SAFETY: a nul-terminated name, which is all `shm_open_create` reads.
    let mut fd = unsafe { crate::preview::shm_open_create(spelling.as_ptr()) };
    if fd < 0 {
        let taken = std::io::Error::last_os_error().raw_os_error() == Some(libc::EEXIST);
        // The name is still there and this slot's mapping is still the right size for the
        // picture, so the object under that name is the one already mapped here and the host
        // simply has not unlinked it. Nothing to create and nothing to map: the frame goes
        // straight into the pages, and the name that was sent last time is sent again.
        if taken && held.len == want {
            // SAFETY: `held.len == want` bytes were mapped writable at this address by a turn of
            // this ring that has not been undone, and nothing unmaps a slot except the code
            // below, which holds the same lock this does.
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), held.addr as *mut u8, want) };
            return Some(name);
        }
        // Either the name is held at the wrong length — a pane resized around a film — or
        // something is there that should not be, such as a segment left behind by a previous
        // CleeCode killed on the same pid. Removing it is removing our own name and nobody
        // else's, since the pid in it is this process's.
        // SAFETY: a nul-terminated name.
        unsafe { libc::shm_unlink(spelling.as_ptr()) };
        // SAFETY: as above.
        fd = unsafe { crate::preview::shm_open_create(spelling.as_ptr()) };
    }
    if fd < 0 {
        return None;
    }
    // A fresh object under this name, so whatever this slot used to hold is neither wanted nor
    // reachable any more. Dropped before the new mapping is made rather than after, so that a
    // film playing for an hour holds `FRAMES` mappings and not one per frame.
    if held.len != 0 {
        // SAFETY: the address and length a previous turn of this ring mapped, unmapped once; the
        // slot is cleared in the same breath so no later turn can unmap it again.
        unsafe { libc::munmap(held.addr as *mut libc::c_void, held.len) };
        ring.slots[slot] = Frame { addr: 0, len: 0 };
    }
    // SAFETY: `fd` came back from `shm_open_create` above.
    let sized = unsafe { libc::ftruncate(fd, want as libc::off_t) } == 0;
    // SAFETY: a writable shared mapping of exactly the length just truncated to.
    let map = match sized {
        true => unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                want,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        },
        false => libc::MAP_FAILED,
    };
    // The descriptor has done its work: a mapping outlives the descriptor it was made from, and
    // an unlink later on does not disturb it either.
    // SAFETY: the same descriptor, closed exactly once.
    unsafe { libc::close(fd) };
    if map == libc::MAP_FAILED {
        // Nothing is mapped, so the name would be one the host could not read anything useful
        // out of. Taken away again rather than left for the next turn to trip over.
        // SAFETY: a nul-terminated name, and nothing of ours is mapped under it.
        unsafe { libc::shm_unlink(spelling.as_ptr()) };
        return None;
    }
    // SAFETY: `want` bytes were just mapped writable at this address, and `bytes` is at least
    // that long because `want` is its length.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), map.cast::<u8>(), want) };
    ring.slots[slot] = Frame { addr: map as usize, len: want };
    Some(name)
}

/// Windows has no POSIX shared memory, so a frame there goes the way it always has.
///
/// The same single refusal `load_medium` makes on the receiving side rather than half a feature:
/// `t=d` is not a degraded road, it is the road every terminal in existence understands.
#[cfg(not(unix))]
fn stage(_bytes: &[u8]) -> Option<String> {
    None
}

/// Unmaps and unlinks the whole ring, on the way out of the process.
///
/// Called from `main`'s teardown, beside `graphics_profile::finish`, and it exists for the host
/// that did not do its half. A terminal that reads a segment unlinks it, and after that this
/// finds nothing to remove — which is the good outcome and not an error. A host that read
/// nothing, though, leaves `FRAMES` segments of a megabyte each sitting in the kernel for as
/// long as the machine is up, and "as long as the machine is up" is the part that matters: a
/// driven session with the medium forced, which is a host that never reads by construction,
/// would otherwise cost four megabytes a run forever.
///
/// A session that is killed rather than closed still leaks them. That is the same hole
/// `preview.rs` leaves behind its probe fixtures and it is bounded the same way — `FRAMES`
/// names, all carrying this pid, all of which the next CleeCode on that pid removes on its first
/// turn of the ring.
pub fn release_frames() {
    #[cfg(unix)]
    {
        let Ok(mut ring) = RING.lock() else { return };
        for slot in 0..FRAMES {
            let held = ring.slots[slot];
            if held.len != 0 {
                // SAFETY: the address and length this ring mapped, unmapped exactly once — the
                // slot is cleared below so nothing can reach it again.
                unsafe { libc::munmap(held.addr as *mut libc::c_void, held.len) };
                ring.slots[slot] = Frame { addr: 0, len: 0 };
            }
            if let Ok(spelling) = std::ffi::CString::new(frame_name(slot)) {
                // SAFETY: a nul-terminated name, and nothing of ours is mapped under it now.
                unsafe { libc::shm_unlink(spelling.as_ptr()) };
            }
        }
    }
}

/// Whether this picture can go out on this road, and the cell box it is placed across if it can.
///
/// This used to be a list of refusals about size, and all of them came from one omission: the
/// transmission carried no `c` and `r`, so the host had to work out how many rows and columns the
/// picture covered from its pixels and the cell size, and the picture therefore had to be a whole
/// number of cells in both directions and no bigger than the rectangle. A frame that was five
/// pixels short of the grid — which is nearly every frame of a film that honours its aspect
/// ratio — failed that test and took the crate's road instead.
///
/// The transmission now says `c` and `r`, which is the protocol's own way of saying "this
/// picture belongs in that many cells", and a host that is given them scales the picture into the
/// box in both directions with filtering. So the size refusals are gone: the box is the rectangle
/// the picture was laid out into, and whatever arrived goes out at whatever size it arrived.
///
/// What is left are the two refusals that were never about fitting. A layout the protocol cannot
/// carry as it stands has to be converted first, and a conversion is the cost this road exists to
/// avoid; and a picture is placed by naming each of its rows with a combining character, of which
/// the protocol defines 297, so a box taller than that has rows no character can name. The box is
/// clamped there rather than refused, because `place` clamps to the same number and the two must
/// agree — a placement that asked for more rows than it filled would have the host scale the
/// picture to a box whose bottom is never drawn.
pub fn placement(image: &DynamicImage, area: Rect) -> Option<(u16, u16)> {
    raw(image)?;
    if image.width() == 0 || image.height() == 0 {
        return None;
    }
    let rows = area.height.min(DIACRITICS.len() as u16);
    (area.width > 0 && rows > 0).then_some((area.width, rows))
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
    /// The size in pixels of the picture the sequence below carries and the cell box it was
    /// transmitted for, or `None` when the slot has just been handed a new picture and does not
    /// yet carry it.
    ///
    /// This is the whole of the "has it changed?" test, and it is sound for a reason worth saying
    /// out loud: within one placement the picture's bytes never change after the first frame, and
    /// a *new* picture always arrives as a new placement, which clears this. So the question is
    /// never "are these the same pixels" — which would mean reading them all, the very hash this
    /// road exists to skip — but "is this still the same placement".
    ///
    /// The cell box is half of the answer because it travels *inside* the transmission, as `c`
    /// and `r`: a pane resized around a picture that has not changed a pixel is a picture the
    /// host is now scaling into the wrong box, and only sending it again puts that right.
    encoded: Option<(u32, u32, u16, u16)>,
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
        self.encode(image, cells);
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
    ///
    /// `c` and `r` are the other half of why. They say how many columns and rows the placement
    /// occupies, and a host given them scales the picture into that box — which is what lets the
    /// frame go out at whatever size it arrived at, with nothing resampled here. Two consequences
    /// follow and are deliberate. A frame that comes back 580x435 for a 580x440 box used to be
    /// letterboxed with five black rows; it is now stretched by about one per cent, because "fit
    /// into these cells" is what the keys mean, and at that ratio it cannot be seen. And a
    /// picture *smaller* than its box is now enlarged into it rather than sitting small in the
    /// corner, which is what a real kitty terminal does with the same escape sequence and what a
    /// program that asked for `c` columns and `r` rows was asking for.
    fn encode(&mut self, image: &DynamicImage, cells: (u16, u16)) {
        self.encode_by(image, cells, by_name());
    }

    /// The encoding itself, with the road it takes handed in rather than asked for.
    ///
    /// Split from `encode` for one reason, and it is a test's: the gate is a fact about the
    /// session recorded in a `OnceLock` at startup, which under `cargo test` is never set and
    /// always reads `Direct`. A test that wanted to see the other road would have to reach into
    /// a global every other test in the process shares. So the question is asked once, at the
    /// top, and everything below it is a function of the answer.
    ///
    /// **`t=s`: the frame by name.** The picture goes into a segment of CleeCode's own and the
    /// escape carries the name of it — which is some sixty bytes where the same frame by value
    /// is two million, and the whole of why any of this was written. Two things about the shape
    /// of that command are not arbitrary. The payload is the base64 of the *name*, not the name,
    /// because that is what the protocol says and what CleeCode's own receiving side decodes
    /// before it opens anything. And there is no `m` key at all: `m` is the protocol's chunking
    /// and chunking is a thing a direct transmission does, so a command carrying a name is
    /// complete by arriving — `pane_graphics` says the same thing from the other end, where
    /// reading `m` on this road would gather every frame of a film into one transmission that
    /// never ends, because `mpv` marks every single frame `m=1` and never closes one.
    ///
    /// **A failure here costs one frame and not the pane.** `stage` answers `None` for anything
    /// that went wrong — the ring's lock poisoned, a name the platform will not take, a create
    /// or a map that failed — and `None` falls through to the road below, which works on every
    /// terminal there is. Nothing is remembered about the failure and nothing is switched off:
    /// the next frame asks again.
    fn encode_by(&mut self, image: &DynamicImage, cells: (u16, u16), by_name: bool) {
        let size = (image.width(), image.height(), cells.0, cells.1);
        if self.encoded == Some(size) {
            return;
        }
        let Some((bytes, format)) = raw(image) else { return };
        let (id, (columns, rows)) = (self.id, cells);
        let (width, height) = (image.width(), image.height());
        let (start, escape, end) = tmux_wrapping();
        let keys = format!("i={id},a=T,U=1,f={format}");
        let box_ = format!("s={width},v={height},c={columns},r={rows}");

        let data = &mut self.transmit;
        data.clear();
        // Asked only where the host said it could take one, so a terminal that answered nothing
        // — which is most of them, and every one reached over `ssh` — never creates a segment at
        // all. The copy into it is charged to the `Render` stage along with everything else this
        // function does, which keeps a profile taken before this existed comparable with one
        // taken after.
        if let Some(name) = by_name.then(|| stage(bytes)).flatten() {
            data.reserve(name.len() * 2 + keys.len() + box_.len() + 32);
            data.push_str(start);
            write!(data, "{escape}_Gq=2,{keys},t=s,{box_};").unwrap();
            base64_simd::STANDARD.encode_append(name.as_bytes(), data);
            write!(data, "{escape}\\").unwrap();
            data.push_str(end);
            self.encoded = Some(size);
            self.pending = true;
            return;
        }

        // 4096 base64 characters is the protocol's limit for one command, and four characters
        // carry three bytes.
        const CHARS_PER_CHUNK: usize = 4096;
        const CHUNK: usize = (CHARS_PER_CHUNK / 4) * 3;

        let chunks = bytes.chunks(CHUNK);
        let count = chunks.len();
        // The keys of the first chunk and the per-chunk framing, so that a frame's worth of
        // base64 lands in one allocation that is then kept for the next frame.
        data.reserve(count * (CHARS_PER_CHUNK + 16 + escape.len() * 2 + end.len()) + 64);
        for (index, chunk) in chunks.enumerate() {
            data.push_str(start);
            write!(data, "{escape}_Gq=2,").unwrap();
            if index == 0 {
                write!(data, "{keys},t=d,{box_},").unwrap();
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
/// happen here, since `placement` clamps the box to the table's height, and is written the
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
        slot.encode(&image, (2, 1));
        let header = slot.transmit.split(';').next().unwrap();
        assert!(header.contains("f=24"), "{header}");
        assert!(header.contains("a=T,U=1"), "{header}");
        assert!(header.contains("t=d,s=20,v=20"), "{header}");
        // The box the host is to scale it into, which is what makes the size above free to be
        // whatever the picture happened to arrive at.
        assert!(header.contains("c=2,r=1"), "{header}");
        assert!(header.contains("q=2"), "{header}");
        assert!(header.contains(&format!("i={}", slot.id)), "{header}");
        assert_eq!(decoded(&slot.transmit), image.as_bytes());
    }

    #[test]
    fn a_picture_with_alpha_still_goes_out_as_rgba() {
        let image = rgba(20, 20);
        let mut slot = PaneKitty::new().unwrap();
        slot.encode(&image, (2, 1));
        assert!(slot.transmit.split(';').next().unwrap().contains("f=32"));
        assert_eq!(decoded(&slot.transmit), image.as_bytes());
    }

    #[test]
    fn chunks_are_the_protocols_size_and_the_last_one_says_so() {
        let image = rgb(100, 100);
        let mut slot = PaneKitty::new().unwrap();
        slot.encode(&image, (10, 5));
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
        slot.encode(&image, (2, 1));
        slot.place(Rect::new(0, 0, 2, 1), &mut Buffer::empty(Rect::new(0, 0, 4, 4)), (2, 1));
        assert!(!slot.pending, "placing it means the host now has it");
        slot.encode(&image, (2, 1));
        assert!(!slot.pending, "the same placement's picture is already there");
        slot.adopt();
        slot.encode(&image, (2, 1));
        assert!(slot.pending, "a new placement's picture has to be sent again");
    }

    /// The cell box travels inside the transmission, so a pane resized around a picture that
    /// has not changed a pixel is a picture the host must be given again — otherwise it goes on
    /// scaling it into the box it was told about the first time.
    #[test]
    fn the_same_picture_in_a_new_box_is_sent_again() {
        let image = rgb(20, 20);
        let mut slot = PaneKitty::new().unwrap();
        slot.encode(&image, (2, 1));
        slot.place(Rect::new(0, 0, 2, 1), &mut Buffer::empty(Rect::new(0, 0, 8, 8)), (2, 1));
        assert!(!slot.pending);
        slot.encode(&image, (4, 2));
        assert!(slot.pending, "a different box is a different placement");
        assert!(slot.transmit.split(';').next().unwrap().contains("c=4,r=2"));
    }

    /// What this road refuses now, and what it no longer does. The size refusals are gone with
    /// the resize they existed for: the host is told the box and scales into it, so a picture
    /// that lands short of the cell grid, or well over it, is placed just the same.
    #[test]
    fn only_a_layout_the_protocol_cannot_carry_is_refused() {
        let area = Rect::new(0, 0, 10, 10);
        assert_eq!(placement(&rgb(20, 40), area), Some((10, 10)), "the box is the rectangle");
        // A height that stops short of the cell grid — nearly every frame of a film that
        // honours its aspect ratio, and the case that used to send the whole road home.
        assert_eq!(placement(&rgb(580, 435), area), Some((10, 10)));
        // Bigger in pixels than the rectangle, which the host shrinks.
        assert_eq!(placement(&rgb(2000, 40), area), Some((10, 10)));
        // Greyscale would have to be converted before it could be sent.
        assert_eq!(placement(&DynamicImage::ImageLuma8(image::GrayImage::new(20, 40)), area), None);
        // Nothing to transmit: `s=0,v=0` is not a picture.
        assert_eq!(placement(&rgb(0, 0), area), None);
        // No cells to place it in.
        assert_eq!(placement(&rgb(20, 40), Rect::new(0, 0, 0, 5)), None);
        // A box taller than the diacritics that name rows is clamped to them, so that what is
        // asked for is what is drawn.
        let tall = Rect::new(0, 0, 4, 400);
        assert_eq!(placement(&rgb(20, 40), tall), Some((4, DIACRITICS.len() as u16)));
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

    /// A picture of a given size with a given colouring, so that two frames the same shape are
    /// still two different pictures — which is the only way to tell a segment that was rewritten
    /// from one that merely still holds what it held.
    #[cfg(unix)]
    fn tinted(width: u32, height: u32, tint: u8) -> DynamicImage {
        let bytes: Vec<u8> = (0..width * height * 3).map(|i| (i % 251) as u8 ^ tint).collect();
        DynamicImage::ImageRgb8(image::RgbImage::from_raw(width, height, bytes).unwrap())
    }

    /// The ring is one thing for the whole process, and `cargo test` runs its tests in threads
    /// of one process — so two tests rotating it at once would each see the other's turns, and
    /// "the slot four frames from now" would be nobody's slot. The tests that care which segment
    /// comes up next take this first. Poison is stepped over rather than unwrapped: a test that
    /// panicked while holding it has already failed, and taking the rest down with it would
    /// report the wrong name.
    #[cfg(unix)]
    static ROTATING: Mutex<()> = Mutex::new(());

    /// The name the host is given for a frame, out of a sequence built with `t=s`.
    #[cfg(unix)]
    fn named(sequence: &str) -> String {
        let payload = payloads(sequence);
        assert_eq!(payload.len(), 1, "a picture by name is one command, never chunked");
        let name = base64::engine::general_purpose::STANDARD.decode(payload[0]).unwrap();
        String::from_utf8(name).unwrap()
    }

    /// The whole of the change, in one measurement: the same frame that costs two million bytes
    /// of escape by value costs sixty by name, and the pixels the host would find under that
    /// name are the ones that went in.
    ///
    /// Read back through `pane_graphics::read_shared`, which is the receiving side of this exact
    /// protocol and which unlinks what it reads — so this test also plays the host's part of the
    /// contract, and leaves the slot in the state a real terminal leaves it in.
    #[cfg(unix)]
    #[test]
    fn a_frame_the_host_will_take_by_name_travels_as_a_name() {
        let _rotating = ROTATING.lock().unwrap_or_else(|held| held.into_inner());
        let image = rgb(200, 100);
        let mut slot = PaneKitty::new().unwrap();
        slot.encode_by(&image, (20, 10), true);

        let header = slot.transmit.split(';').next().unwrap();
        assert!(header.contains("t=s"), "{header}");
        assert!(!header.contains("t=d"), "{header}");
        // Everything the road by value already said about the picture is still said here: the
        // format, the size, the cell box the host scales it into, and the id it replaces.
        assert!(header.contains("f=24"), "{header}");
        assert!(header.contains("a=T,U=1"), "{header}");
        assert!(header.contains("s=200,v=100"), "{header}");
        assert!(header.contains("c=20,r=10"), "{header}");
        assert!(header.contains(&format!("i={}", slot.id)), "{header}");
        // `m` is the protocol's chunking, and chunking belongs to a transmission that carries
        // the picture. A command carrying a name is complete by arriving.
        assert!(!header.contains("m="), "{header}");

        let name = named(&slot.transmit);
        assert!(name.starts_with("/clee-frame-"), "{name}");
        // The escape is now a name and its framing, where the same frame by value is 80,000
        // bytes of base64 — and that ratio is the whole feature.
        assert!(slot.transmit.len() < 128, "{} bytes of escape", slot.transmit.len());

        let want = image.as_bytes().len();
        let read = crate::pane_graphics::read_shared(name.as_bytes(), 0, want);
        assert_eq!(read.as_deref(), Some(image.as_bytes()), "the host would find the frame");
        release_frames();
    }

    /// A picture the host cannot take by name, or one this was not asked to send that way, goes
    /// out exactly as it always did — which is the promise the whole road rests on.
    #[test]
    fn a_frame_that_cannot_go_by_name_still_goes_by_value() {
        let image = rgb(20, 20);
        let mut slot = PaneKitty::new().unwrap();
        slot.encode_by(&image, (2, 1), false);
        let header = slot.transmit.split(';').next().unwrap();
        assert!(header.contains("t=d"), "{header}");
        assert_eq!(decoded(&slot.transmit), image.as_bytes());
    }

    /// Why the ring is a ring. The host reads a segment some unknown moment after the escape
    /// naming it was written, so consecutive frames must not be written into the same pages —
    /// and `FRAMES` of them later the names come round again, which is what bounds the whole
    /// thing to four segments rather than one per frame of a film.
    #[cfg(unix)]
    #[test]
    fn consecutive_frames_take_different_segments_and_the_ring_comes_round() {
        let _rotating = ROTATING.lock().unwrap_or_else(|held| held.into_inner());
        let mut slot = PaneKitty::new().unwrap();
        let mut names = Vec::new();
        for frame in 0..FRAMES + 1 {
            // A different size each time, so the encoder treats each as a new picture rather
            // than as the one it already carries.
            let image = rgb(20, 20 + frame as u32);
            slot.encode_by(&image, (2, 1), true);
            names.push(named(&slot.transmit));
        }
        let ring: std::collections::BTreeSet<&String> = names[..FRAMES].iter().collect();
        assert_eq!(ring.len(), FRAMES, "no two frames in flight share a segment: {names:?}");
        assert_eq!(names[FRAMES], names[0], "and then the ring comes round");
        for name in ring {
            let _ = crate::pane_graphics::read_shared(name.as_bytes(), 0, 0);
        }
        release_frames();
    }

    /// What happens when the host does its half. The protocol makes the terminal the owner of
    /// the segment once it has read it, so by the time the ring comes round the name is gone —
    /// and the slot must build it again rather than transmit a name that resolves to nothing,
    /// which on screen is a pane that has stopped.
    #[cfg(unix)]
    #[test]
    fn a_segment_the_host_unlinked_is_made_again() {
        let _rotating = ROTATING.lock().unwrap_or_else(|held| held.into_inner());
        let image = tinted(30, 30, 0);
        let mut slot = PaneKitty::new().unwrap();
        slot.encode_by(&image, (3, 3), true);
        let name = named(&slot.transmit);
        // The host reading it, unlink and all.
        assert!(crate::pane_graphics::read_shared(name.as_bytes(), 0, 0).is_some());
        assert!(
            crate::pane_graphics::read_shared(name.as_bytes(), 0, 0).is_none(),
            "the read above is also the unlink the protocol asks for"
        );

        // Round the ring until that slot comes up again. The pictures are the same shape and
        // differently coloured, so the segment is the right size to be reused and the only thing
        // that can tell a rebuilt one from a stale one is what is in it.
        let mut found = None;
        for frame in 0..FRAMES {
            let next = tinted(30, 30, frame as u8 + 1);
            slot.encode_by(&next, (3, 4 + frame as u16), true);
            if named(&slot.transmit) == name {
                found = Some(next);
                break;
            }
        }
        let expected = found.expect("the ring never came back to that slot");
        let read = crate::pane_graphics::read_shared(name.as_bytes(), 0, expected.as_bytes().len());
        assert_eq!(read.as_deref(), Some(expected.as_bytes()), "the segment was made again");
        // Nothing in the kernel outlives the test that made it. Safe to call with the
        // rotation lock held, which is the only thing that could be part-way through a turn.
        release_frames();
    }

    /// And what happens when the host does *not* do its half — a terminal that reads the frame
    /// and leaves the object behind, or the driven harness, whose host never reads at all. The
    /// slot is reused rather than recreated: the name is the same, the pages are the same, and
    /// what the host would find there is the newest frame. Bounded either way, which is the
    /// point of a ring.
    #[cfg(unix)]
    #[test]
    fn a_host_that_never_unlinks_has_its_slot_reused() {
        let _rotating = ROTATING.lock().unwrap_or_else(|held| held.into_inner());
        let first = tinted(40, 40, 0);
        let mut slot = PaneKitty::new().unwrap();
        slot.encode_by(&first, (4, 4), true);
        let name = named(&slot.transmit);

        // Nothing reads and nothing unlinks, which is the harness's host and a terminal that
        // does not keep its half of the bargain. The same pixel count every time, so the slot's
        // mapping is still the right length and the create that comes round again finds the name
        // taken — the arm that writes straight into the pages it already holds.
        let mut latest = None;
        for frame in 0..FRAMES {
            let next = tinted(40, 40, frame as u8 + 1);
            slot.encode_by(&next, (4, 5 + frame as u16), true);
            if named(&slot.transmit) == name {
                latest = Some(next);
                break;
            }
        }
        let expected = latest.expect("the ring never came back to that slot");
        let read = crate::pane_graphics::read_shared(name.as_bytes(), 0, expected.as_bytes().len());
        assert_eq!(read.as_deref(), Some(expected.as_bytes()));
        // Nothing in the kernel outlives the test that made it. Safe to call with the
        // rotation lock held, which is the only thing that could be part-way through a turn.
        release_frames();
    }

    /// macOS caps a shared-memory name at 31 characters including the leading slash and answers
    /// `ENAMETOOLONG` past it — a failure that arrives as a segment that was never created and
    /// reads exactly like a host saying no. The same arithmetic `preview.rs` pins for its probe
    /// fixtures, here for the ring, at the widest a pid is ever written.
    #[cfg(unix)]
    #[test]
    fn a_segment_name_fits_what_the_platform_will_take() {
        let widest = format!("/clee-frame-{}-{}", u32::MAX, FRAMES - 1);
        assert!(widest.len() <= 31, "{widest} is {} characters", widest.len());
        assert!(frame_name(0).starts_with("/clee-frame-"));
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
    /// pty, a four-by-three clip, and the 580x435 `mpv` produces for a 580x440 hole — five rows
    /// short of the grid, which is the shape that used to be padded here and is now stretched by
    /// the host instead.
    #[test]
    fn a_film_frame_is_placed_as_the_whole_rectangle_at_the_size_it_arrived() {
        let area = Rect::new(0, 3, 58, 22);
        let image = rgb(580, 435);
        let cells = placement(&image, area).unwrap();
        assert_eq!(cells, (58, 22));
        let mut buf = Buffer::empty(Rect { x: 0, y: 0, width: 120, height: 40 });
        let mut slot = PaneKitty::new().unwrap();
        slot.draw(&image, area, &mut buf, cells);
        assert_eq!(Size::from(buf.area), Size::new(120, 40));
        // Three bytes a pixel on the wire, not four, and the pixels are the ones that arrived:
        // nothing between the player and the host resampled the frame.
        assert_eq!(decoded(&slot.transmit).len(), 580 * 435 * 3);
        let header = slot.transmit.split(';').next().unwrap();
        assert!(header.contains("s=580,v=435"), "{header}");
        assert!(header.contains("c=58,r=22"), "{header}");
    }
}
