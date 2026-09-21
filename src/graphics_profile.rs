//! Where a pane's video actually spends its time, counted rather than guessed.
//!
//! A pane playing a film through `mpv --vo=kitty` costs more CPU in CleeCode than it does in
//! `mpv`, and the reason is a road that is walked twice: the frame arrives in a shared-memory
//! segment, is copied out of it, copied again on its way to the drawing, turned from RGB into
//! RGBA, base64-encoded and written to the host terminal as a megabyte of escape sequence. The
//! obvious idea is to stop touching the pixels at all — put the frame in a segment of our own
//! and hand the host the four hundred bytes of name, which is exactly how `mpv` hands it to us.
//!
//! That idea is worth building only if the stages it removes are where the time goes, and
//! nobody knows whether they are. This module is how that question gets an answer instead of an
//! opinion. Each stage of the road opens a `Span` and closes it with the bytes it read and
//! wrote; a counter per stage accumulates calls, nanoseconds and bytes; a ring of the most
//! recent durations is kept so the report can state a p95 as well as a mean, because one slow
//! frame in thirty is what a viewer sees as a stutter and a mean hides it.
//!
//! Two things make the numbers mean something, and both are recorded here beside the stages.
//! The first is the denominator: `getrusage` at the start and at the end says how much CPU the
//! whole process spent, so the report can say what fraction of it the instrumented stages
//! account for — and, more usefully, how much they do not. A profile that accounts for a tenth
//! of the time has not found where the time goes. The second is the picture's own dimensions at
//! each end of the road: `s=` and `v=` as they arrived, against the pixel size of the rectangle
//! they were drawn into, which is what says whether the resampling in the middle is doing
//! anything at all.
//!
//! Off unless `CLEE_GRAPHICS_PROFILE` names a file to write the report to, and off means off:
//! every entry point begins with one relaxed atomic load and returns, so no clock is read and
//! no counter is touched on a pane that is not being measured. The same kill-switch shape as
//! `CLEE_UPDATE_CHECK` and `CLEE_DRIVE_SHELL`, and for the same reason — a measurement hook
//! that cost something when idle would be a permanent tax to answer one question.
//!
//! What is deliberately *not* instrumented is `Pace`. It decides whether a frame is worth
//! decoding by reading a clock, so a hook that read one of its own inside it would be measuring
//! a pacing that only exists while it is being measured. The refusals are counted — an atomic
//! increment, after the decision, on the path that has already returned `false` — and nothing
//! else there is touched.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The stages of the road, in the order a frame walks them.
///
/// The first three are the reader thread's, the next three the drawing thread's, and the last
/// two are the host's — the bytes leaving through the terminal's own writer. They are numbered
/// because the counters are a plain array indexed by the number, which is what keeps the hot
/// path to an index rather than a lookup.
#[derive(Clone, Copy)]
pub enum Stage {
    /// Scanning everything a pane writes for escape sequences. Charged for all of the pane's
    /// output, not only its pictures — which is the point of measuring it.
    Split = 0,
    /// Opening, mapping, copying out of and unlinking the segment the frame arrived in.
    ShmRead = 1,
    /// Decoding the base64 a frame arrived *in*, on the road the segment is not used — which is
    /// `mpv`'s own default, `--vo-kitty-use-shm` being off unless it is asked for. Nothing a
    /// relay does to the way out changes this, so it is here to be excluded from the case for
    /// one rather than to be counted towards it.
    Base64 = 2,
    /// Turning the frame's bytes into an `image::DynamicImage`. Meant to be a move for the raw
    /// formats and a decode only for PNG; the numbers say which it was.
    Wrap = 3,
    /// The second full copy of the frame, taken because the drawing owns what it draws.
    Clone = 4,
    /// Building the protocol around the picture. Cheaper-looking than it is: `ratatui-image`
    /// hashes the whole frame here to know whether it has changed.
    Build = 5,
    /// The drawing itself, which inside `ratatui-image` is the resample, the RGB-to-RGBA
    /// conversion and the base64 — the three stages the shared-memory relay would remove.
    Render = 6,
    /// `write` on the terminal's own handle, below the frame buffer, so this is the real cost
    /// of handing the bytes to the host rather than the cost of buffering them.
    Write = 7,
    /// `flush` on the same handle.
    Flush = 8,
}

/// How many stages there are, which is how long the counter array is.
const STAGES: usize = 9;

/// What each stage is called in the report. Same order as `Stage`.
const NAMES: [&str; STAGES] = [
    "splitter",
    "shm_read",
    "base64_decode",
    "wrap",
    "clone",
    "protocol_build",
    "render",
    "stdout_write",
    "stdout_flush",
];

/// How many recent durations per stage are kept for the percentile.
///
/// A ring rather than a growing list: twenty seconds of video is some six hundred frames and
/// fits many times over, while the splitter — called once per read off the pty — can run to
/// tens of thousands, and a measurement that allocated without bound would eventually be
/// measuring its own allocator. When more arrive than fit, the percentile is over the most
/// recent `SAMPLES` of them and the report says so by printing the sample count beside it.
const SAMPLES: usize = 1 << 16;

/// The one relaxed flag the hot path reads. Separate from the profile itself so that the check
/// is a load and a branch rather than a `OnceLock` deref.
static ON: AtomicBool = AtomicBool::new(false);

static PROFILE: OnceLock<Option<Profile>> = OnceLock::new();

/// Everything one stage accumulates.
struct Totals {
    calls: AtomicU64,
    /// Wall nanoseconds, which for a stage that blocks is mostly not this process's doing.
    nanos: AtomicU64,
    /// Nanoseconds of CPU on the thread that ran the stage. The one that can be compared with
    /// `getrusage`, and the only one the decision should be read off: a write that waited four
    /// hundred milliseconds for a terminal to catch up spent none of them computing.
    cpu_nanos: AtomicU64,
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
    /// The ring of recent durations, and how many have been written into it ever. The index is
    /// the count masked, so writers never wait on each other.
    samples: Vec<AtomicU64>,
    taken: AtomicUsize,
}

impl Default for Totals {
    fn default() -> Self {
        Totals {
            calls: AtomicU64::new(0),
            nanos: AtomicU64::new(0),
            cpu_nanos: AtomicU64::new(0),
            bytes_in: AtomicU64::new(0),
            bytes_out: AtomicU64::new(0),
            samples: (0..SAMPLES).map(|_| AtomicU64::new(0)).collect(),
            taken: AtomicUsize::new(0),
        }
    }
}

/// One drawn frame of the editor, for telling the cost of a picture from the cost of a screen.
///
/// The bytes are what left through stdout between this frame and the one before it, and
/// `pictures` is how many pane pictures were rendered into it. Frames with none are the
/// baseline: whatever the editor writes for its own cells, the status line and the cursor.
struct FrameRow {
    bytes: u64,
    pictures: u32,
}

/// A picture as it arrived against the rectangle it was drawn into, both in pixels.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Fit {
    src: (u32, u32),
    dst: (u32, u32),
}

struct Profile {
    path: std::path::PathBuf,
    started: Instant,
    /// When the report was last written out. See `frame_drawn`.
    flushed: Mutex<Instant>,
    /// User and system seconds at startup, subtracted from the same pair at exit. Not zero: the
    /// process has already loaded a workspace by the time this is taken.
    rusage_start: (f64, f64),
    stages: Vec<Totals>,
    /// Pictures the decoder produced, whatever became of them afterwards.
    decoded: AtomicU64,
    /// Pictures `Pace` refused to decode because the screen had not taken up the last one.
    paced_out: AtomicU64,
    /// Protocols built, which for video is one per frame that reached the screen.
    built: AtomicU64,
    /// Editor frames drawn, with and without a picture in them.
    frames: AtomicU64,
    /// Every stdout byte counted so far, read at each frame boundary to charge the difference
    /// to that frame.
    stdout_total: AtomicU64,
    rows: std::sync::Mutex<Vec<FrameRow>>,
    fits: std::sync::Mutex<std::collections::BTreeMap<Fit, u64>>,
    /// The `s=` and `v=` pairs seen, by format, with a count each.
    arrivals: std::sync::Mutex<std::collections::BTreeMap<(u32, u32, u32), u64>>,
}

/// Reads the switch and, if it is set, prepares the counters. Called once from `main`, before
/// anything that could be measured has run.
pub fn init() {
    let path = std::env::var_os("CLEE_GRAPHICS_PROFILE").filter(|p| !p.is_empty());
    let profile = path.map(|path| Profile {
        path: std::path::PathBuf::from(path),
        started: Instant::now(),
        flushed: Mutex::new(Instant::now()),
        rusage_start: rusage(),
        stages: (0..STAGES).map(|_| Totals::default()).collect(),
        decoded: AtomicU64::new(0),
        paced_out: AtomicU64::new(0),
        built: AtomicU64::new(0),
        frames: AtomicU64::new(0),
        stdout_total: AtomicU64::new(0),
        rows: std::sync::Mutex::new(Vec::new()),
        fits: std::sync::Mutex::new(std::collections::BTreeMap::new()),
        arrivals: std::sync::Mutex::new(std::collections::BTreeMap::new()),
    });
    let running = profile.is_some();
    let _ = PROFILE.set(profile);
    // Raised last, so that no thread can see the flag before the counters it indexes exist.
    ON.store(running, Ordering::Relaxed);
}

/// Whether anything is being measured. One relaxed load, which is the whole cost of this module
/// to a session that never asked for it.
#[inline]
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

fn profile() -> Option<&'static Profile> {
    PROFILE.get()?.as_ref()
}

/// One measured stretch of work.
///
/// Opened before the work and closed after it with what it read and what it produced. When the
/// switch is off the clock is never read and closing does nothing, which is why this is a value
/// carried across the work rather than a guard with a `Drop`: a `Drop` would have to decide
/// what the byte counts were, and they are only known at the end.
pub struct Span {
    stage: Stage,
    /// The two clocks, or nothing at all when the switch is off. Both are needed and neither
    /// answers for the other: the wall clock is what a stutter is made of, and the thread's CPU
    /// clock is what `getrusage` adds up at the end.
    start: Option<(Instant, u64)>,
}

impl Span {
    #[inline]
    pub fn open(stage: Stage) -> Span {
        Span { stage, start: on().then(|| (Instant::now(), thread_cpu_nanos())) }
    }

    /// Closes the span, charging it with the bytes it took in and the bytes it gave back.
    #[inline]
    pub fn close(self, bytes_in: u64, bytes_out: u64) {
        self.close_with(bytes_in, || bytes_out);
    }

    /// The same, where working out the bytes produced is itself work — walking a list of
    /// pieces, say — and must not happen on a session that is not measuring.
    #[inline]
    pub fn close_with(self, bytes_in: u64, bytes_out: impl FnOnce() -> u64) {
        let Some((start, cpu)) = self.start else { return };
        let nanos = start.elapsed().as_nanos() as u64;
        let cpu_nanos = thread_cpu_nanos().saturating_sub(cpu);
        let Some(profile) = profile() else { return };
        let totals = &profile.stages[self.stage as usize];
        totals.calls.fetch_add(1, Ordering::Relaxed);
        totals.nanos.fetch_add(nanos, Ordering::Relaxed);
        totals.cpu_nanos.fetch_add(cpu_nanos, Ordering::Relaxed);
        totals.bytes_in.fetch_add(bytes_in, Ordering::Relaxed);
        totals.bytes_out.fetch_add(bytes_out(), Ordering::Relaxed);
        let slot = totals.taken.fetch_add(1, Ordering::Relaxed) & (SAMPLES - 1);
        totals.samples[slot].store(nanos, Ordering::Relaxed);
    }
}

/// A picture came out of the decoder, at this size and in this format.
pub fn decoded(width: u32, height: u32, format: u32) {
    if !on() {
        return;
    }
    let Some(profile) = profile() else { return };
    profile.decoded.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut arrivals) = profile.arrivals.lock() {
        *arrivals.entry((width, height, format)).or_insert(0) += 1;
    }
}

/// `Pace` refused a picture. Counted after the decision and never before it: see the note at
/// the top of this file about not measuring the thing into existence.
pub fn paced_out() {
    if !on() {
        return;
    }
    if let Some(profile) = profile() {
        profile.paced_out.fetch_add(1, Ordering::Relaxed);
    }
}

/// A protocol was built around a picture.
pub fn built() {
    if !on() {
        return;
    }
    if let Some(profile) = profile() {
        profile.built.fetch_add(1, Ordering::Relaxed);
    }
}

/// A picture was drawn: how big it was, and how big the hole it went into was, both in pixels.
///
/// The pair is the whole of the resampling question. `Resize::Fit` is asked for on every frame,
/// and if the two sizes are already equal then what it does is a copy dressed up as a resize —
/// which a relay could skip. If they differ, the relay has to scale the frame itself or ask the
/// player for a different size.
pub fn drew(src: (u32, u32), dst: (u32, u32)) {
    if !on() {
        return;
    }
    let Some(profile) = profile() else { return };
    if let Ok(mut fits) = profile.fits.lock() {
        *fits.entry(Fit { src, dst }).or_insert(0) += 1;
    }
}

/// One frame of the editor has been drawn and flushed.
///
/// Called after the flush so that the stdout bytes of that frame are already counted, and the
/// difference since the previous call is what this frame cost the host.
pub fn frame_drawn(pictures: u32) {
    if !on() {
        return;
    }
    let Some(profile) = profile() else { return };
    profile.frames.fetch_add(1, Ordering::Relaxed);
    let total = profile.stages[Stage::Write as usize].bytes_out.load(Ordering::Relaxed);
    let previous = profile.stdout_total.swap(total, Ordering::Relaxed);
    if let Ok(mut rows) = profile.rows.lock() {
        // Bounded for the same reason the sample rings are: a long session must not turn a
        // measurement into a memory leak. Twenty thousand frames is some ten minutes of video.
        if rows.len() < 20_000 {
            rows.push(FrameRow { bytes: total.saturating_sub(previous), pictures });
        }
    }
    // And written out every so often, not only on the way out of `main`.
    //
    // The report used to exist only if the editor was quit the ordinary way, which made every
    // measurement depend on a human pressing the right keys in the right order and cost several
    // real runs to a session that was killed instead. It also rules out driving a measurement
    // from outside, where the only way to end a session running in somebody else's terminal is
    // to kill it. Written from here because this is the one place already called once a frame
    // and never from a stage's hot path; the cost is a clock read and a comparison, and the
    // write itself happens once every few seconds against a file nobody is reading yet.
    if let Ok(mut flushed) = profile.flushed.try_lock() {
        if flushed.elapsed() >= FLUSH_EVERY {
            *flushed = Instant::now();
            write_report(profile);
        }
    }
}

/// How often the report is written while the session is still running. Long enough that a
/// twelve-second run writes it a handful of times, short enough that a session ended by a
/// signal loses only the tail.
const FLUSH_EVERY: Duration = Duration::from_secs(2);

/// Renders the report and puts it where it was asked for, creating the directory if need be.
fn write_report(profile: &Profile) {
    let report = render_report(profile);
    if let Some(parent) = profile.path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&profile.path, report);
}

/// How many pictures have been rendered in total, so a caller can tell how many went into one
/// frame by taking the difference across it.
pub fn renders() -> u64 {
    if !on() {
        return 0;
    }
    profile().map_or(0, |p| p.stages[Stage::Render as usize].calls.load(Ordering::Relaxed))
}

/// The terminal's own writer, counted.
///
/// Sits *below* the frame buffer rather than above it, which is the difference between
/// measuring the bytes as they are handed to the host and measuring them as they are copied
/// into a four-megabyte buffer. The second number would be the same and the timing would be a
/// `memcpy`; the first is the write the host has to read.
pub struct CountingStdout(std::io::Stdout);

impl CountingStdout {
    pub fn new() -> CountingStdout {
        CountingStdout(std::io::stdout())
    }
}

impl Write for CountingStdout {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let span = Span::open(Stage::Write);
        let written = self.0.write(buf);
        let bytes = *written.as_ref().unwrap_or(&0) as u64;
        span.close(buf.len() as u64, bytes);
        written
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let span = Span::open(Stage::Flush);
        let flushed = self.0.flush();
        span.close(0, 0);
        flushed
    }
}

/// The calling thread's CPU time so far, in nanoseconds.
///
/// Per-thread rather than per-process because the road runs on two of them — the pane's reader
/// and the one that draws — and a process-wide clock read from either would count the other's
/// work as this stage's. Zero where the platform has no such clock, which makes the stage read
/// as free rather than as wrong; the report says which clock it is quoting.
#[cfg(unix)]
fn thread_cpu_nanos() -> u64 {
    // SAFETY: `spec` is plain data, written by `clock_gettime` before it is read.
    let mut spec: libc::timespec = unsafe { std::mem::zeroed() };
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut spec) } != 0 {
        return 0;
    }
    (spec.tv_sec as u64) * 1_000_000_000 + spec.tv_nsec as u64
}

/// See the unix spelling above.
#[cfg(not(unix))]
fn thread_cpu_nanos() -> u64 {
    0
}

/// What one measured stage costs to measure, in nanoseconds, found by measuring nothing a
/// thousand times over.
///
/// Printed in the report beside the stage totals so that a stage called ten thousand times can
/// be read with its own instrument's weight in mind. The splitter is the one this matters for:
/// it runs once per read off the pty, and a hook that cost a microsecond would be inventing
/// milliseconds of work that a session without the switch never does.
fn span_overhead_nanos() -> u64 {
    const ROUNDS: u64 = 1000;
    let start = Instant::now();
    for _ in 0..ROUNDS {
        let _ = (Instant::now(), thread_cpu_nanos());
    }
    // Twice, because a span reads both clocks at each end.
    (start.elapsed().as_nanos() as u64 / ROUNDS) * 2
}

/// The process's own CPU time so far, as user and system seconds.
///
/// The denominator of the whole exercise. Without it a table of stage totals says only that the
/// stages took as long as they took; with it the report can say that they are a fifth of what
/// the process spent, and that four fifths are somewhere this did not look.
#[cfg(unix)]
fn rusage() -> (f64, f64) {
    // SAFETY: `usage` is plain data, written entirely by `getrusage` before it is read, and the
    // call touches nothing else.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
        return (0.0, 0.0);
    }
    let seconds = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    (seconds(usage.ru_utime), seconds(usage.ru_stime))
}

/// Windows has no `getrusage`, and no shared-memory road for a frame to arrive by either, so
/// the question this module exists to answer cannot be asked there.
#[cfg(not(unix))]
fn rusage() -> (f64, f64) {
    (0.0, 0.0)
}

/// Writes the report, if one was asked for. Called once on the way out of `main`.
pub fn finish() {
    if !on() {
        return;
    }
    let Some(profile) = profile() else { return };
    // Lowered first: a stage closing while the report is being written would be counted into a
    // total that has already been printed, and a thread is still draining its pane at this
    // point.
    ON.store(false, Ordering::Relaxed);
    write_report(profile);
}

/// The p95 and the mean of a stage's sample ring, in nanoseconds.
fn percentiles(totals: &Totals) -> (u64, usize) {
    let taken = totals.taken.load(Ordering::Relaxed);
    let held = taken.min(SAMPLES);
    if held == 0 {
        return (0, 0);
    }
    let mut samples: Vec<u64> =
        totals.samples[..held].iter().map(|s| s.load(Ordering::Relaxed)).collect();
    samples.sort_unstable();
    let index = ((held as f64) * 0.95).ceil() as usize;
    (samples[index.saturating_sub(1).min(held - 1)], held)
}

fn render_report(profile: &Profile) -> String {
    let wall = profile.started.elapsed().as_secs_f64();
    let (user, system) = rusage();
    let cpu_user = user - profile.rusage_start.0;
    let cpu_system = system - profile.rusage_start.1;
    let cpu = cpu_user + cpu_system;

    let mut stages = String::new();
    let mut instrumented = 0u64;
    for (index, name) in NAMES.iter().enumerate() {
        let totals = &profile.stages[index];
        let calls = totals.calls.load(Ordering::Relaxed);
        let nanos = totals.nanos.load(Ordering::Relaxed);
        let cpu_nanos = totals.cpu_nanos.load(Ordering::Relaxed);
        // The splitter is on the reader thread and the render on the drawing thread; summing
        // them is still the right total, because `getrusage` sums every thread's CPU too.
        instrumented += cpu_nanos;
        let (p95, held) = percentiles(totals);
        let mean = nanos.checked_div(calls).unwrap_or(0);
        let cpu_mean = cpu_nanos.checked_div(calls).unwrap_or(0);
        if index > 0 {
            stages.push_str(",\n");
        }
        stages.push_str(&format!(
            "    {{\"stage\": {:?}, \"calls\": {calls}, \"wall_ns\": {nanos}, \
             \"wall_mean_ns\": {mean}, \"wall_p95_ns\": {p95}, \"samples\": {held}, \
             \"cpu_ns\": {cpu_nanos}, \"cpu_mean_ns\": {cpu_mean}, \"bytes_in\": {}, \
             \"bytes_out\": {}, \"percent_of_process_cpu\": {:.2}}}",
            name,
            totals.bytes_in.load(Ordering::Relaxed),
            totals.bytes_out.load(Ordering::Relaxed),
            if cpu > 0.0 { (cpu_nanos as f64 / 1e9) / cpu * 100.0 } else { 0.0 },
        ));
    }

    let rows = profile.rows.lock().map(|rows| {
        let split = |with: bool| {
            let mut bytes: Vec<u64> = rows
                .iter()
                .filter(|row| (row.pictures > 0) == with)
                .map(|row| row.bytes)
                .collect();
            bytes.sort_unstable();
            let count = bytes.len();
            let total: u64 = bytes.iter().sum();
            let mean = if count == 0 { 0 } else { total / count as u64 };
            let p95 = if count == 0 {
                0
            } else {
                bytes[(((count as f64) * 0.95).ceil() as usize).saturating_sub(1).min(count - 1)]
            };
            format!("{{\"frames\": {count}, \"total_bytes\": {total}, \"mean_bytes\": {mean}, \"p95_bytes\": {p95}}}")
        };
        (split(true), split(false))
    });
    let (with_picture, without_picture) = rows.unwrap_or_else(|_| ("{}".into(), "{}".into()));

    let fits = profile
        .fits
        .lock()
        .map(|fits| {
            fits.iter()
                .map(|(fit, count)| {
                    format!(
                        "    {{\"src\": [{}, {}], \"dst\": [{}, {}], \"count\": {count}, \"same\": {}}}",
                        fit.src.0,
                        fit.src.1,
                        fit.dst.0,
                        fit.dst.1,
                        fit.src == fit.dst
                    )
                })
                .collect::<Vec<_>>()
                .join(",\n")
        })
        .unwrap_or_default();

    let arrivals = profile
        .arrivals
        .lock()
        .map(|arrivals| {
            arrivals
                .iter()
                .map(|((w, h, format), count)| {
                    format!("    {{\"s\": {w}, \"v\": {h}, \"f\": {format}, \"count\": {count}}}")
                })
                .collect::<Vec<_>>()
                .join(",\n")
        })
        .unwrap_or_default();

    let accounted = if cpu > 0.0 { (instrumented as f64 / 1e9) / cpu * 100.0 } else { 0.0 };
    let font = crate::preview::cell_pixel_size().unwrap_or((0, 0));
    format!(
        "{{\n  \"wall_seconds\": {wall:.3},\n  \"cpu_user_seconds\": {cpu_user:.3},\n  \
         \"cpu_system_seconds\": {cpu_system:.3},\n  \"cpu_total_seconds\": {cpu:.3},\n  \
         \"instrumented_cpu_seconds\": {:.3},\n  \"instrumented_percent_of_cpu\": {accounted:.2},\n  \
         \"span_overhead_ns\": {},\n  \
         \"protocol\": {:?},\n  \"cell_pixels\": [{}, {}],\n  \
         \"pictures_decoded\": {},\n  \"pictures_paced_out\": {},\n  \"protocols_built\": {},\n  \
         \"editor_frames\": {},\n  \"frames_with_picture\": {with_picture},\n  \
         \"frames_without_picture\": {without_picture},\n  \"stages\": [\n{stages}\n  ],\n  \
         \"fits\": [\n{fits}\n  ],\n  \"arrivals\": [\n{arrivals}\n  ]\n}}\n",
        instrumented as f64 / 1e9,
        span_overhead_nanos(),
        crate::preview::protocol_name(),
        font.0,
        font.1,
        profile.decoded.load(Ordering::Relaxed),
        profile.paced_out.load(Ordering::Relaxed),
        profile.built.load(Ordering::Relaxed),
        profile.frames.load(Ordering::Relaxed),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole contract of the switch: nothing is measured and nothing is recorded until a
    /// path is named. Every other test in the tree runs with it off, which is what makes this
    /// one worth having — it pins the default the rest of the suite silently relies on.
    #[test]
    fn nothing_is_measured_unless_a_path_is_named() {
        assert!(!on());
        let span = Span::open(Stage::Render);
        assert!(span.start.is_none());
        span.close(1, 1);
        // And the recording entry points are all no-ops rather than panics on an uninitialised
        // profile, which is how they behave in every unit test in the tree.
        decoded(640, 480, 24);
        paced_out();
        built();
        drew((640, 480), (640, 480));
        frame_drawn(1);
        assert_eq!(renders(), 0);
    }

    /// A stage's percentile comes out of the ring even when the ring has wrapped, and a stage
    /// nobody walked reports nothing rather than a division by zero.
    #[test]
    fn a_percentile_survives_an_empty_stage_and_a_full_ring() {
        let totals = Totals::default();
        assert_eq!(percentiles(&totals), (0, 0));
        for nanos in 1..=100u64 {
            let slot = totals.taken.fetch_add(1, Ordering::Relaxed) & (SAMPLES - 1);
            totals.samples[slot].store(nanos, Ordering::Relaxed);
        }
        let (p95, held) = percentiles(&totals);
        assert_eq!(held, 100);
        assert_eq!(p95, 95);
    }
}
