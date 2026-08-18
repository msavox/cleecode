//! Files there is nothing to edit in, shown as themselves in an editor tab.
//!
//! A picture opened from the tree used to give a blank read-only buffer: the binary guard had
//! already emptied it, so the tab said nothing and could do nothing. This draws the file
//! instead.
//!
//! The pixels come from CleeCode's own output, not from a program inside a terminal pane. That
//! is not a preference — a pane is parsed into a grid of cells by `vt100` and repainted, and
//! `vt100` handles neither DCS nor APC, so the graphics escapes a program writes there are
//! recognised and dropped. Written from here they go straight down the same stdout ratatui
//! already uses, which is also why this keeps working over ssh.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::OnceLock;

use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;

/// What the host terminal turned out to be able to draw. Asked once, at startup, because it
/// costs a query-and-wait on stdout that must not happen mid-frame; a global for the same
/// reason the scrollback length is one — it is a fact about the session, fixed for its life.
static PICKER: OnceLock<Picker> = OnceLock::new();

/// Asks the terminal what it can draw. Safe to call once the alternate screen is up: the query
/// has its own timeout inside `ratatui-image` and falls back to half-blocks, so a terminal that
/// answers nothing costs a moment and a coarser picture rather than a hang.
pub fn detect_terminal() {
    if let Ok(picker) = Picker::from_query_stdio() {
        let _ = PICKER.set(picker);
    }
}

fn picker() -> Option<&'static Picker> {
    PICKER.get()
}

/// How the picture will actually be drawn, for saying so in the tab.
pub fn protocol_name() -> &'static str {
    match picker().map(Picker::protocol_type) {
        Some(ratatui_image::picker::ProtocolType::Kitty) => "kitty",
        Some(ratatui_image::picker::ProtocolType::Iterm2) => "iTerm2",
        Some(ratatui_image::picker::ProtocolType::Sixel) => "sixel",
        _ => "half-blocks",
    }
}

/// Whether the terminal draws real pixels rather than coloured cells. A page of prose reduced
/// to half-blocks is unreadable — a 150dpi page is some 200 times more pixels than a pane has
/// cells — so where there is no graphics protocol the *text* rendering is the better answer,
/// not a degraded picture.
pub fn has_real_pixels() -> bool {
    !matches!(
        picker().map(Picker::protocol_type),
        None | Some(ratatui_image::picker::ProtocolType::Halfblocks)
    )
}

/// Whether pandoc is on the PATH, asked once. Without it markdown can still be shown, as styled
/// text; with it, it can be shown as a document, pictures and all.
pub fn has_pandoc() -> bool {
    static FOUND: OnceLock<bool> = OnceLock::new();
    *FOUND.get_or_init(|| {
        std::process::Command::new("pandoc")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// Whether a markdown preview can be a real document rather than styled text. Both halves have
/// to hold: a document needs pandoc to make it and pixels to show it.
pub fn markdown_as_document() -> bool {
    has_real_pixels() && has_pandoc()
}

/// Extensions shown as a picture rather than opened as text.
///
/// A list rather than "whatever isn't text", because being unreadable as text is not a reason
/// to believe something is an image: a .zip is binary too, and guessing wrong would replace a
/// tab that admits it can't help with one that pretends it can.
pub fn is_previewable(ext: &str) -> bool {
    matches!(ext, "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tiff" | "tif")
}

/// Extensions with a rendered view alongside the source, rather than instead of it.
///
/// Different in kind from a picture or a PDF: those have no editable text form, so the tab *is*
/// the file. Markdown is something you write, so its preview is a second view of a buffer that
/// stays open and editable — never a second copy of the text, which could only ever diverge
/// from the first.
pub fn is_renderable(ext: &str) -> bool {
    matches!(ext, "md" | "markdown" | "mdown" | "mkd")
}

/// What a preview tab is showing at this moment.
pub enum State {
    /// Being produced on a background thread. A large photo takes long enough to decode, and a
    /// PDF page long enough to rasterise, that doing either on the main thread would stall the
    /// whole window, terminals included.
    Loading,
    /// Ready to draw. The protocol holds the picture and, once drawn, the version of it fitted
    /// to the pane.
    ///
    /// The crate offers a threaded protocol for this, and it was tried: the resize does happen
    /// during the draw. But once the filter is chosen to suit what is being shown the work is
    /// twenty-odd milliseconds, and moving it off the frame bought less than the extra frame of
    /// latency it added — while the hand-off of a finished resize back to the tab that asked for
    /// it had a way of losing one, leaving a pane blank with nothing to make it ask again.
    Ready(Box<StatefulProtocol>),
    /// Unreadable, unrenderable, or too big. Shown as a message in the tab: a preview that
    /// cannot be produced should say so where it was expected, not leave an empty frame.
    Failed(String),
    /// Markdown, rendered to styled lines. Text rather than pixels, so it needs no graphics
    /// protocol, no external tool and no subprocess: it draws the same everywhere and it can
    /// keep up with typing.
    Rendered {
        lines: Vec<ratatui::text::Line<'static>>,
        /// Which revision of the source these lines came from. Kept beside them so a stale
        /// render is recognisable as one rather than merely looking wrong.
        #[allow(dead_code)]
        revision: u64,
    },
}

/// What a preview tab is a view of.
///
/// Worth a name of its own: the three differ in what the navigation bar may offer them, and
/// spelling the question out as `pages.is_none() && source.is_none()` in every place that asks
/// is how a photograph ended up with a dark mode that turned it into a negative.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A picture file: one image, shown as it is.
    Picture,
    /// A PDF: pages, rasterised one at a time.
    Document,
    /// The rendered view of a markdown buffer — a document when pandoc can make one, styled
    /// text otherwise, and either way a second view of something being edited.
    Markdown,
}

/// How a page is sized against the pane it is shown in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fit {
    /// The whole page, shrunk until it fits. Nothing is hidden, and on a tall pane the text
    /// ends up small.
    Page,
    /// The page's width fills the pane and the rest of it is scrolled to. What you want for
    /// reading, which is why a document opens this way.
    Width,
}

/// A preview tab: what is on screen, and — for a document — which page of how many.
pub struct Preview {
    pub state: State,
    /// The buffer this is a view of, for a rendered preview. The preview owns no text of its
    /// own — one copy, so the two can never disagree about what the file says — and holding the
    /// path is how it finds the buffer to render each time that buffer changes.
    pub source: Option<PathBuf>,
    /// When the source last stopped changing, and at which revision. A document takes about
    /// half a second to make, so it follows the pauses rather than the keystrokes; without this
    /// every character typed would start a render that the next character made pointless.
    pub settled: Option<(u64, std::time::Instant)>,
    /// The revision what is currently on screen was made from.
    pub shown_revision: u64,
    /// How tall the pane was, in cells, the last time it was drawn. See `area_cols`.
    pub area_rows: u16,
    /// How wide the pane was, in cells, the last time it was drawn. Recorded by the renderer
    /// because it is the only place that knows, and needed by the *next* render, which has to
    /// ask a rasteriser for a number of pixels before it has a pane to look at.
    pub area_cols: u16,
    /// 1.0 is the page fitted to the pane; above that it is rasterised larger and scrolled.
    pub zoom: f32,
    /// Whether the page is shown inverted, for reading a white document on a dark screen.
    pub inverted: bool,
    /// Whole page, or page width filling the pane.
    pub fit: Fit,
    /// The rendered page, kept whole so scrolling can cut a new window out of it without going
    /// back to the rasteriser. A page is a few megabytes; re-rendering one is a subprocess.
    pub full: Option<image::DynamicImage>,
    /// Where the visible window sits on the rendered page, in the page's own pixels. Two axes,
    /// because zooming past the pane's width makes sideways travel real as well.
    pub scroll_px: u32,
    pub scroll_x: u32,
    /// Set once making a document out of this has failed. The tab then stays on the styled-text
    /// rendering rather than spending half a second failing the same way after every pause —
    /// the reason it failed will not have changed by itself. \u{25b6} Refresh clears it and tries again.
    pub document_failed: bool,
    /// `None` for a picture, which is one image with nothing to page through. `Some` for a PDF,
    /// even before the page count is known: paging has to work while that is still being asked
    /// for, and on a file whose count cannot be established at all.
    pub pages: Option<Pages>,
    /// Markdown only: show it as styled text even where a document could be made. The rendered
    /// document is the prettier of the two and the text one is the faster — it follows the
    /// keystrokes, needs no pandoc and no graphics — so which is wanted is a matter of what is
    /// being done, not of what the machine can manage. The `text` button on the bar sets it.
    pub text_only: bool,
}

pub struct Pages {
    /// One-based, the way a document is numbered and the way the page is named on screen.
    pub current: usize,
    /// `None` while unknown, or when nothing could tell us. Paging still works without it —
    /// the far end announces itself by failing to produce a page.
    pub total: Option<usize>,
}

impl Preview {
    pub fn picture() -> Self {
        Preview { state: State::Loading, pages: None, source: None, settled: None, shown_revision: 0, document_failed: false, area_cols: 0, area_rows: 0, zoom: 1.0, inverted: false, fit: Fit::Page, full: None, scroll_px: 0, scroll_x: 0, text_only: false }
    }

    pub fn document(page: usize) -> Self {
        Preview {
            state: State::Loading,
            pages: Some(Pages { current: page, total: None }),
            source: None,
            settled: None,
            shown_revision: 0,
            document_failed: false,
            area_cols: 0,
            area_rows: 0,
            zoom: 1.0,
            inverted: false,
            // A document is for reading, so it opens at the width that makes it readable.
            fit: Fit::Width,
            full: None,
            scroll_px: 0,
            scroll_x: 0,
            text_only: false,
        }
    }

    /// A live view of an open buffer. Starts empty; the first frame fills it.
    pub fn rendered(source: PathBuf) -> Self {
        Preview {
            state: State::Rendered { lines: Vec::new(), revision: u64::MAX },
            // A document made from markdown has pages like any other; styled text is one long
            // scroll. Which it is only becomes known when the first render is asked for.
            pages: markdown_as_document().then(|| Pages { current: 1, total: None }),
            source: Some(source),
            settled: None,
            shown_revision: u64::MAX,
            document_failed: false,
            area_cols: 0,
            area_rows: 0,
            zoom: 1.0,
            inverted: false,
            // A document is for reading, so it opens at the width that makes it readable.
            fit: Fit::Width,
            full: None,
            scroll_px: 0,
            scroll_x: 0,
            text_only: false,
        }
    }

    /// Whether the buffer has moved since what is on screen was made.
    pub fn stale(&self, revision: u64) -> bool {
        self.shown_revision != revision
    }

    /// What this tab is a view of. A markdown preview keeps its `source` whichever of its two
    /// renderings is up, so that is the question asked first.
    pub fn kind(&self) -> Kind {
        match (self.source.is_some(), self.pages.is_some()) {
            (true, _) => Kind::Markdown,
            (false, true) => Kind::Document,
            (false, false) => Kind::Picture,
        }
    }

    /// Switches a markdown preview between the rendered document and the styled text, and makes
    /// sure the next pass over the buffers remakes it: one has pages and the other is a single
    /// scroll, so what is on screen cannot be reused for the other.
    pub fn set_text_only(&mut self, text_only: bool) {
        if self.text_only == text_only {
            return;
        }
        self.text_only = text_only;
        self.pages = (!text_only && markdown_as_document()).then(|| Pages { current: 1, total: None });
        self.settled = None;
        self.scroll_px = 0;
        self.scroll_x = 0;
        // No revision can equal this, so the buffer counts as moved and the view is made again.
        self.shown_revision = u64::MAX;
    }

    /// Whether this is showing styled text rather than a page of pixels — markdown that cannot
    /// be made into a document, or that was asked to stay as text. Zoom, fit and dark mode are
    /// all properties of a rasterised page, so a bar over a text view offers none of them.
    pub fn text_view(&self) -> bool {
        self.kind() == Kind::Markdown && (self.text_only || !markdown_as_document())
    }

    /// The box a picture is scaled into for the pane it last had, at the zoom in force. Zero
    /// when the pane has never been drawn, which `scale_picture` reads as "leave it alone".
    ///
    /// Capped at the same pixel budget a rasterised page gets: four times a large pane is tens
    /// of megapixels, and every one of them would be resampled on a zoom step.
    pub fn picture_box(&self) -> (u32, u32) {
        if self.area_cols == 0 || self.area_rows == 0 {
            return (0, 0);
        }
        let (pane_w, pane_h) = pane_pixels(self.area_cols, self.area_rows);
        let zoom = self.zoom.max(0.1);
        let (w, h) = ((pane_w as f32 * zoom) as u32, (pane_h as f32 * zoom) as u32);
        let (w, h) = (w.max(1), h.max(1));
        let pixels = u64::from(w) * u64::from(h);
        if pixels <= u64::from(pixel_budget()) {
            return (w, h);
        }
        // Both sides shrink together, so the reduction goes as the square root.
        let scale = (f64::from(pixel_budget()) / pixels as f64).sqrt() as f32;
        (((w as f32 * scale) as u32).max(1), ((h as f32 * scale) as u32).max(1))
    }



    /// How many pixels wide the next render of this should be, from the pane it last had and
    /// the zoom in force. Falls back to a sensible page when it has never been drawn.
    pub fn render_width(&self) -> u32 {
        if self.area_cols == 0 || self.area_rows == 0 {
            return FALLBACK_PAGE_WIDTH;
        }
        page_width_for(self.area_cols, self.area_rows, self.zoom, self.fit)
    }

    /// Steps the zoom by one notch, within bounds that keep a page readable and affordable.
    pub fn zoom_by(&mut self, steps: i32) -> bool {
        let next = (self.zoom * 1.25f32.powi(steps)).clamp(0.5, 4.0);
        let changed = (next - self.zoom).abs() > f32::EPSILON;
        self.zoom = next;
        changed
    }

    /// Whether re-making this would produce anything different. True for a document, whose
    /// pages are generated — from a file that may have been recompiled, or from a buffer being
    /// typed in. False for a picture: the file *is* the picture, so "refresh" would mean
    /// decoding the same bytes again, and the button is better spent on the run command, which
    /// for an image shows it in a terminal instead.
    pub fn refreshable(&self) -> bool {
        self.pages.is_some() || self.source.is_some()
    }

    /// The page a document is on, or `None` for a picture.
    pub fn page(&self) -> Option<usize> {
        self.pages.as_ref().map(|p| p.current)
    }
}

/// A rendered page or picture on its way back to the tab that asked for it, identified by path
/// because a tab's index can change while a thread is still working.
pub struct Decoded {
    pub path: PathBuf,
    /// Which page this answers for, so a reply that arrives after the reader has already paged
    /// on is dropped instead of yanking them back.
    pub page: Option<usize>,
    pub result: Result<image::DynamicImage, String>,
    /// The page count, when this render was also the one that established it.
    pub total: Option<usize>,
}

/// Beyond this, decoding is refused rather than attempted. A camera raw or a poster-sized scan
/// can be hundreds of megabytes decompressed, and the point of a preview is a glance.
const MAX_PIXELS: u64 = 80_000_000;

/// What a preview tab has been asked to produce.
pub enum Job {
    /// A picture, decoded and then scaled to the box the pane and the zoom ask for. A page is
    /// *rasterised* at its zoom; a picture arrives at whatever size it was saved at, so the same
    /// job is done here — without it the widget shrinks every picture to the pane and the zoom
    /// buttons do nothing at all. A zero box means "as it is": the pane has never been drawn, so
    /// there is nothing yet to scale against.
    Picture { path: PathBuf, box_px: (u32, u32), fit: Fit },
    /// One page of a document already on disk, rasterised `width_px` pixels wide.
    Page { path: PathBuf, page: usize, width_px: u32 },
    /// One page of a document made *from a buffer*: the markdown you are editing, turned into a
    /// real document so that pictures it refers to appear in the flow of the text — which a grid
    /// of cells cannot do, and which is the whole reason this goes the long way round.
    Markdown { path: PathBuf, text: String, page: usize, width_px: u32 },
}

impl Job {
    fn path(&self) -> &Path {
        match self {
            Job::Picture { path, .. } | Job::Page { path, .. } | Job::Markdown { path, .. } => path,
        }
    }

    fn page(&self) -> Option<usize> {
        match self {
            Job::Picture { .. } => None,
            Job::Page { page, .. } | Job::Markdown { page, .. } => Some(*page),
        }
    }
}

/// Starts producing what a preview tab should show, answering on `tx` when it is done. Returns
/// the state the tab holds meanwhile.
pub fn start_loading(job: Job, tx: Sender<Decoded>) -> State {
    std::thread::spawn(move || {
        let path = job.path().to_path_buf();
        let page = job.page();
        let (result, total) = match &job {
            Job::Picture { path, box_px, fit } => {
                (decode(path).map(|image| scale_picture(image, *box_px, *fit)), None)
            }
            Job::Page { path, page, width_px } => {
                // The count is asked for alongside the first page rather than in its own pass:
                // it needs the same tool, and a second subprocess for a number nobody is
                // waiting on would only slow the page down.
                (render_page(path, *page, *width_px), page_count(path))
            }
            Job::Markdown { path, text, page, width_px } => match markdown_to_pdf(path, text) {
                Ok(pdf) => {
                    let rendered = render_page(pdf.path(), *page, *width_px);
                    (rendered, page_count(pdf.path()))
                }
                Err(e) => (Err(e), None),
            },
        };
        // The receiver is gone when the tab was closed while this was still working, which is
        // ordinary rather than an error: nothing is waiting for the answer.
        let _ = tx.send(Decoded { path, page, result, total });
    });
    State::Loading
}

/// A file that deletes itself. Preview intermediates land in a shared temp directory, and one
/// left behind is litter on somebody\'s disk.
struct TempFile(PathBuf);

impl TempFile {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Where to find something that can turn a document into a PDF.
///
/// pandoc looks for `pdflatex` on the PATH and gives up if it is not there — and on macOS a TeX
/// distribution installs to /Library/TeX/texbin, which reaches the PATH through
/// /etc/paths.d and therefore only in shells started afterwards. An editor launched before that,
/// or from a launcher that never read those files, sees a machine with no LaTeX on it. The same
/// shape of problem as an interpreter installed outside PATH, and answered the same way: look in
/// the places it is actually installed.
///
/// The lighter engines come first where they exist: they need no TeX at all and start faster.
fn pdf_engine() -> Option<PathBuf> {
    const ENGINES: [&str; 5] = ["tectonic", "typst", "pdflatex", "xelatex", "lualatex"];
    const EXTRA_DIRS: [&str; 4] = [
        "/Library/TeX/texbin",
        "/usr/local/texlive/bin",
        "/opt/homebrew/bin",
        "/usr/local/bin",
    ];
    for engine in ENGINES {
        // On the PATH already: let pandoc have the bare name.
        if std::process::Command::new(engine)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
        {
            return Some(PathBuf::from(engine));
        }
        for dir in EXTRA_DIRS {
            let candidate = Path::new(dir).join(engine);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Turns the markdown *being edited* into a PDF. The text comes from the buffer rather than the
/// file so the preview can be ahead of the last save, but `source` still matters: it is where
/// pictures the document refers to are looked up from.
fn markdown_to_pdf(source: &Path, text: &str) -> Result<TempFile, String> {
    let stem = format!("cleecode-md-{}-{:?}", std::process::id(), std::thread::current().id());
    let dir = std::env::temp_dir();
    let input = TempFile(dir.join(format!("{stem}.md")));
    let output = TempFile(dir.join(format!("{stem}.pdf")));
    std::fs::write(input.path(), text).map_err(|e| e.to_string())?;

    // Relative image paths are relative to the *document*, not to the temporary copy of it, so
    // pandoc is told where the real one lives. Without this every picture would go missing —
    // which is the one thing this whole route exists to get right.
    let resources = source.parent().unwrap_or_else(|| Path::new("."));
    let mut command = std::process::Command::new("pandoc");
    command
        .arg(input.path())
        .arg("-o")
        .arg(output.path())
        .arg(format!("--resource-path={}", resources.display()));
    // Named explicitly rather than left to pandoc's own search, which only looks at the PATH.
    let Some(engine) = pdf_engine() else {
        return Err("no PDF engine found - install tectonic, typst or a TeX distribution".to_string());
    };
    command.arg(format!("--pdf-engine={}", engine.display()));
    let out = command
        .output()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "pandoc is not installed".to_string(),
            _ => e.to_string(),
        })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("pandoc: {}", stderr.trim().lines().last().unwrap_or("failed")));
    }
    Ok(output)
}

fn decode(path: &Path) -> Result<image::DynamicImage, String> {
    let reader = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    // Checked before decoding, not after: the point is not to allocate the thing at all.
    if let Ok((w, h)) = reader.into_dimensions() {
        let pixels = u64::from(w) * u64::from(h);
        if pixels > MAX_PIXELS {
            return Err(format!("{w}x{h} is too large to preview"));
        }
    }
    image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())
}

/// Sizes a picture for the pane it is going into, at the zoom and fit in force.
///
/// A photograph has a size of its own and no idea what a pane is, so "fit", "wide" and the zoom
/// buttons have to be answered here in pixels. `Resize::Fit` in the widget only ever shrinks to
/// the pane, so a picture left at its own size looks identical at every zoom — which is exactly
/// what it did before this existed.
///
/// A zero box means the pane has never been drawn and there is nothing to scale against, so the
/// picture is passed through untouched.
pub fn scale_picture(image: image::DynamicImage, box_px: (u32, u32), fit: Fit) -> image::DynamicImage {
    let Some((width, height)) = picture_size_in((image.width(), image.height()), box_px, fit) else {
        return image;
    };
    // Already that size: the worker scaled it on the way in, and resampling it a second time
    // would cost the same as the first for no change at all.
    if (image.width(), image.height()) == (width, height) {
        return image;
    }
    image.resize_exact(width, height, image::imageops::FilterType::Lanczos3)
}

/// The size a picture of `(w, h)` takes in a box, or `None` when there is nothing to work from.
fn picture_size_in((w, h): (u32, u32), (box_w, box_h): (u32, u32), fit: Fit) -> Option<(u32, u32)> {
    if box_w == 0 || box_h == 0 || w == 0 || h == 0 {
        return None;
    }
    Some(match fit {
        // The whole picture, as large as fits the box — enlarged as well as shrunk, since past
        // 100% the point of a zoom is to see the pixels closer.
        Fit::Page => {
            let scale = (f64::from(box_w) / f64::from(w)).min(f64::from(box_h) / f64::from(h));
            (((f64::from(w) * scale) as u32).max(1), ((f64::from(h) * scale) as u32).max(1))
        }
        // As wide as the box, however tall that makes it. What falls past the pane is scrolled to.
        Fit::Width => {
            let height = (u64::from(box_w) * u64::from(h) / u64::from(w)).min(u64::from(u32::MAX));
            (box_w, (height as u32).max(1))
        }
    })
}

/// Extensions shown a page at a time, by rasterising them first.
pub fn is_document(ext: &str) -> bool {
    ext == "pdf"
}

/// How wide a page is rasterised, when nothing better is known about the pane it is going into.
const FALLBACK_PAGE_WIDTH: u32 = 1600;

/// Never ask a rasteriser for less than this, or more.
const PAGE_WIDTH_RANGE: (u32, u32) = (600, 4000);

/// How many pixels a *rasterised page* may be.
///
/// A budget on work, not on transmission — a distinction worth being exact about, because
/// getting it wrong cost a picture 41ms for nothing. What reaches the terminal is decided by
/// `Resize::Fit`, which scales whatever it is given to the pane's own size: shrinking the source
/// first changes the quality and the effort, never the bytes.
///
/// So it is applied where the effort is ours to spend — choosing how large to rasterise a PDF
/// page, which is a subprocess, a decode and a resize — and *not* to pictures, which arrive at
/// whatever size they are and gain nothing from being reduced before the widget reduces them.
/// A page is never rasterised beyond this. Not a quality setting and not tunable: a page has to
/// be *exactly* as big as the pane it is drawn in, because the widget fits by shrinking and
/// never enlarges — so anything that trims it below the pane silently breaks fitting, which is
/// what a configurable version of this did.
const MAX_PREVIEW_PIXELS: u32 = 16_000_000;

fn pixel_budget() -> u32 {
    MAX_PREVIEW_PIXELS
}

/// The size of one character cell in pixels, as the terminal reported it. This is what turns a
/// pane measured in cells into a pane measured in pixels, which is the only unit a rasteriser
/// understands.
pub fn cell_size() -> Option<(u16, u16)> {
    picker().map(|p| {
        let size = p.font_size();
        (size.width, size.height)
    })
}

/// The shape of a page, tall side over wide side. A4 and US Letter are within 3% of each other,
/// and being a few per cent out only means the raster is a few per cent bigger than needed.
const PAGE_ASPECT: f32 = 1.414;

/// How many pixels wide to rasterise a page for a pane `cols` x `rows` cells.
///
/// Both dimensions matter, and using only the width was expensive: a portrait page rasterised
/// as wide as the pane comes out far taller than the pane is, and every one of those extra rows
/// is decoded, resampled and then thrown away. A page 1984 wide was producing 4.6 times the
/// pixels that reached the screen. Sizing it to *fit* the pane instead is both sharper than the
/// old fixed resolution and cheaper than either.
///
/// Fixed at 150dpi before, which on anything but a small window meant rendering a page smaller
/// than the space it had to fill and then stretching it — the graininess was never aliasing, it
/// was enlargement.
pub fn page_width_for(cols: u16, rows: u16, zoom: f32, fit: Fit) -> u32 {
    let (cw, ch) = cell_size().unwrap_or((8, 16));
    let pane_w = u32::from(cols) * u32::from(cw);
    let pane_h = u32::from(rows) * u32::from(ch).max(1);
    let wanted = match fit {
        // As wide as the pane, and as tall as the page turns out to be.
        Fit::Width => pane_w,
        // Narrow enough that the whole page is no taller than the pane.
        Fit::Page => pane_w.min((pane_h as f32 / PAGE_ASPECT) as u32).max(1),
    };
    let wanted = (wanted as f32 * zoom.max(0.1)) as u32;
    budgeted(wanted, pane_w, pane_h, fit).clamp(PAGE_WIDTH_RANGE.0, PAGE_WIDTH_RANGE.1)
}

/// Trims a page width, but only to stop something absurd.
///
/// This used to be a working constraint rather than a backstop, and it broke fitting: a band
/// narrower than the pane stays narrow, because `Resize::Fit` never enlarges — so "fit the
/// width" quietly produced a small page in the corner. The budget was there for a slowness that
/// turned out to be an unoptimised build, so it costs nothing to let a page be the size of the
/// pane it is going into, which is the size it has to be for fitting to mean anything.
fn budgeted(width: u32, pane_w: u32, pane_h: u32, fit: Fit) -> u32 {
    let budget = pixel_budget();
    let shown = match fit {
        Fit::Page => width.saturating_mul((width as f32 * PAGE_ASPECT) as u32),
        // A band as wide as the page and as tall as the pane is, in proportion.
        Fit::Width => {
            let band = (width as u64 * pane_h.max(1) as u64 / pane_w.max(1) as u64) as u32;
            width.saturating_mul(band.max(1))
        }
    };
    if shown <= budget {
        return width;
    }
    // Both sides shrink together, so the reduction goes as the square root.
    ((width as f32) * (budget as f32 / shown as f32).sqrt()) as u32
}

/// The pane, measured in the page's own pixels.
pub fn pane_pixels(cols: u16, rows: u16) -> (u32, u32) {
    let (cw, ch) = cell_size().unwrap_or((8, 16));
    (u32::from(cols) * u32::from(cw), u32::from(rows) * u32::from(ch))
}

/// The part of a rendered page the pane can show, cut at `(x, y)` in the page's own pixels.
///
/// Cutting the window here rather than handing the whole page to the widget is what makes zoom
/// mean anything at all. `Resize::Fit` shrinks whatever it is given until it fits, so a page
/// rasterised twice as large comes back exactly the same size on screen — zooming changed the
/// detail and nothing else. Cropping first is what turns "bigger" into "closer".
///
/// It also bounds the cost: the terminal is sent a pane's worth of pixels whether the page is
/// one screen across or five.
pub fn visible_window(
    page: &image::DynamicImage,
    cols: u16,
    rows: u16,
    scroll_x: u32,
    scroll_y: u32,
) -> image::DynamicImage {
    let (pane_w, pane_h) = pane_pixels(cols, rows);
    // Smaller than the pane in both directions: nothing to cut, and the widget will centre it.
    if page.height() <= pane_h && page.width() <= pane_w {
        return page.clone();
    }
    let w = pane_w.min(page.width()).max(1);
    let h = pane_h.min(page.height()).max(1);
    let x = scroll_x.min(page.width().saturating_sub(w));
    let y = scroll_y.min(page.height().saturating_sub(h));
    image::GenericImageView::view(page, x, y, w, h).to_image().into()
}

/// How far a page can be scrolled on each axis before its far edge is on screen.
pub fn max_scroll(page: &image::DynamicImage, cols: u16, rows: u16) -> (u32, u32) {
    let (pane_w, pane_h) = pane_pixels(cols, rows);
    (page.width().saturating_sub(pane_w), page.height().saturating_sub(pane_h))
}

/// A PDF is not decoded in-process. The Rust libraries that could are either AGPL (`mupdf`),
/// which this project's MIT licence cannot take, or want a prebuilt native blob shipped per
/// platform,
/// which the source-built releases have no way to carry. An external rasteriser is the honest
/// answer: it is replaceable, it is already installed alongside any TeX distribution, and when
/// it is missing the tab says so instead of the feature half-working.
///
/// `pdftoppm` first because it exists for exactly this and its arguments are stable; Ghostscript
/// second because it comes with everything.
fn rasterisers() -> [&'static str; 2] {
    ["pdftoppm", "gs"]
}

/// Renders one page to a temporary PNG and decodes it. One-based, as printed on the page.
fn render_page(path: &Path, page: usize, width_px: u32) -> Result<image::DynamicImage, String> {
    let out = std::env::temp_dir().join(format!(
        "cleecode-page-{}-{:?}-{page}.png",
        std::process::id(),
        std::thread::current().id()
    ));
    // Removed whatever happens, including on the error paths below: these land in a shared
    // temp directory and a preview left behind is litter on somebody's disk.
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _cleanup = Cleanup(out.clone());

    let mut last = String::from("no rasteriser found");
    for tool in rasterisers() {
        let status = match tool {
            // -singlefile makes it write exactly `out.png` rather than numbering the name.
            // Sized to the pane rather than to a fixed resolution, and told to antialias:
            // pdftoppm does by default, Ghostscript emphatically does not.
            "pdftoppm" => std::process::Command::new(tool)
                .args(["-png", "-scale-to-x", &width_px.to_string(), "-scale-to-y", "-1"])
                .args(["-aa", "yes", "-aaVector", "yes"])
                .args(["-f", &page.to_string(), "-l", &page.to_string()])
                .arg("-singlefile")
                .arg(path)
                // pdftoppm appends its own ".png", so it is handed the name without one.
                .arg(out.with_extension(""))
                .output(),
            // Ghostscript has no "this many pixels wide" flag, so the resolution is worked back
            // from one. Assuming A4 puts a US Letter page 3% out, which is invisible next to
            // getting the order of magnitude right — the alternative is another subprocess to
            // ask how big the page is.
            _ => std::process::Command::new(tool)
                .args(["-q", "-dNOPAUSE", "-dBATCH", "-dSAFER", "-sDEVICE=png16m"])
                .args(["-dTextAlphaBits=4", "-dGraphicsAlphaBits=4"])
                .arg(format!("-r{}", (width_px as f32 / 8.27).round().max(36.0)))
                .arg(format!("-dFirstPage={page}"))
                .arg(format!("-dLastPage={page}"))
                .arg(format!("-sOutputFile={}", out.display()))
                .arg(path)
                .output(),
        };
        match status {
            // A tool that is not installed at all: try the next one rather than reporting it.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => last = e.to_string(),
            Ok(output) if output.status.success() && out.exists() => return decode(&out),
            Ok(output) => {
                // Past the last page both tools succeed and write nothing, which is how the end
                // of a document announces itself without anyone having to know its length.
                let stderr = String::from_utf8_lossy(&output.stderr);
                last = if stderr.trim().is_empty() {
                    format!("{tool} produced no page {page}")
                } else {
                    format!("{tool}: {}", stderr.trim().lines().next().unwrap_or("failed"))
                };
            }
        }
    }
    Err(last)
}

/// How many pages a document has, or `None` when nothing available can say. Paging works either
/// way — the far end announces itself by failing to produce a page — so this is only for the
/// label, and asking two tools in turn costs nothing when the first one answers.
fn page_count(path: &Path) -> Option<usize> {
    pdfinfo_pages(path).or_else(|| ghostscript_pages(path))
}

/// poppler's own tool, whose output carries a plain `Pages:  12` line.
fn pdfinfo_pages(path: &Path) -> Option<usize> {
    let out = std::process::Command::new("pdfinfo").arg(path).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("Pages:"))
        .and_then(|n| n.trim().parse().ok())
}

/// Ghostscript has no flag for the count; this is the long-standing PostScript incantation for
/// it, which prints the number and nothing else.
fn ghostscript_pages(path: &Path) -> Option<usize> {
    let out = std::process::Command::new("gs")
        .args(["-q", "-dNODISPLAY", "-dNOSAFER", "-c"])
        .arg(format!("({}) (r) file runpdfbegin pdfpagecount = quit", path.display()))
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

impl State {
    /// Takes a rendered image and readies it for drawing, with `tx` the channel the resizing
    /// work goes down. Fails loudly rather than silently when the terminal was never asked what
    /// it can do — that would be a startup-order bug, and a blank tab would hide it.
    pub fn ready(image: image::DynamicImage, inverted: bool) -> State {
        // Inversion is applied to the pixels rather than asked of the terminal, which has no way
        // to be asked. A white page becomes a dark one with light text, which is what every PDF
        // reader means by a dark mode.
        let image = if inverted {
            let mut rgba = image.to_rgba8();
            image::imageops::invert(&mut rgba);
            image::DynamicImage::ImageRgba8(rgba)
        } else {
            image
        };
        match picker().cloned() {
            Some(picker) => State::Ready(Box::new(picker.new_resize_protocol(image))),
            None => State::Failed("the terminal was never asked what it can draw".to_string()),
        }
    }
}

/// Renders markdown to styled lines.
///
/// Deliberately not wrapped here: the lines are logical, and the widget wraps them to whatever
/// the pane is at the time. Wrapping at render time would mean re-rendering on every resize and
/// caching something that is only right at one width.
pub fn render_markdown(source: &str) -> Vec<ratatui::text::Line<'static>> {
    use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::{Line, Span};

    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut style = Style::default();
    // What is open right now, so text knows how to look and where to sit.
    let mut heading: Option<HeadingLevel> = None;
    let mut in_code_block = false;
    let mut quote_depth = 0usize;
    // One entry per open list; `Some(n)` counts an ordered one.
    let mut lists: Vec<Option<u64>> = Vec::new();

    /// Ends the line being built, with the indent and quote marks its context calls for.
    fn flush(lines: &mut Vec<Line<'static>>, spans: &mut Vec<Span<'static>>, prefix: &str) {
        if spans.is_empty() && prefix.is_empty() {
            return;
        }
        let mut out = Vec::new();
        if !prefix.is_empty() {
            out.push(Span::styled(prefix.to_string(), Style::default().fg(Color::DarkGray)));
        }
        out.append(spans);
        lines.push(Line::from(out));
    }

    let quote_prefix = |depth: usize| "\u{2502} ".repeat(depth);
    // Nested lists step in by two, which is enough to read as nesting without marching off the
    // right of a narrow pane.
    let list_indent = |depth: usize| " ".repeat(depth.saturating_sub(1) * 2);

    for event in Parser::new_ext(source, options) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                if !lines.is_empty() {
                    lines.push(Line::from(""));
                }
                heading = Some(level);
                style = Style::default().fg(match level {
                    HeadingLevel::H1 => Color::Cyan,
                    HeadingLevel::H2 => Color::LightCyan,
                    _ => Color::Blue,
                });
                if matches!(level, HeadingLevel::H1 | HeadingLevel::H2) {
                    style = style.add_modifier(Modifier::BOLD);
                }
                // The marks stay: they say which level this is without relying on colour, which
                // a monochrome terminal would not have.
                let hashes = "#".repeat(level as usize);
                spans.push(Span::styled(format!("{hashes} "), Style::default().fg(Color::DarkGray)));
            }
            Event::End(TagEnd::Heading(_)) => {
                flush(&mut lines, &mut spans, "");
                heading = None;
                style = Style::default();
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                in_code_block = true;
                style = Style::default().fg(Color::LightGreen);
                let label = match &kind {
                    CodeBlockKind::Fenced(lang) if !lang.is_empty() => format!("\u{250c} {lang}"),
                    _ => "\u{250c}".to_string(),
                };
                lines.push(Line::from(Span::styled(label, Style::default().fg(Color::DarkGray))));
            }
            Event::End(TagEnd::CodeBlock) => {
                flush(&mut lines, &mut spans, "\u{2502} ");
                lines.push(Line::from(Span::styled(
                    "\u{2514}".to_string(),
                    Style::default().fg(Color::DarkGray),
                )));
                in_code_block = false;
                style = Style::default();
            }
            Event::Start(Tag::BlockQuote(_)) => quote_depth += 1,
            Event::End(TagEnd::BlockQuote(_)) => quote_depth = quote_depth.saturating_sub(1),
            Event::Start(Tag::List(first)) => lists.push(first),
            Event::End(TagEnd::List(_)) => {
                lists.pop();
                if lists.is_empty() {
                    lines.push(Line::from(""));
                }
            }
            Event::Start(Tag::Item) => {
                let depth = lists.len();
                let marker = match lists.last_mut() {
                    Some(Some(n)) => {
                        let marker = format!("{n}. ");
                        *n += 1;
                        marker
                    }
                    _ => "\u{2022} ".to_string(),
                };
                spans.push(Span::styled(
                    format!("{}{marker}", list_indent(depth)),
                    Style::default().fg(Color::Yellow),
                ));
            }
            Event::End(TagEnd::Item) => flush(&mut lines, &mut spans, &quote_prefix(quote_depth)),
            Event::Start(Tag::Emphasis) => style = style.add_modifier(Modifier::ITALIC),
            Event::End(TagEnd::Emphasis) => style = style.remove_modifier(Modifier::ITALIC),
            Event::Start(Tag::Strong) => style = style.add_modifier(Modifier::BOLD),
            Event::End(TagEnd::Strong) => style = style.remove_modifier(Modifier::BOLD),
            Event::Start(Tag::Strikethrough) => style = style.add_modifier(Modifier::CROSSED_OUT),
            Event::End(TagEnd::Strikethrough) => style = style.remove_modifier(Modifier::CROSSED_OUT),
            Event::Start(Tag::Link { .. }) => {
                style = style.fg(Color::Blue).add_modifier(Modifier::UNDERLINED);
            }
            Event::End(TagEnd::Link) => style = Style::default(),
            Event::Text(text) => {
                if in_code_block {
                    // A fenced block keeps its own line breaks; everything else is reflowed by
                    // the widget, so only code needs splitting by hand.
                    let mut parts = text.split('\n').peekable();
                    while let Some(part) = parts.next() {
                        spans.push(Span::styled(part.to_string(), style));
                        if parts.peek().is_some() {
                            flush(&mut lines, &mut spans, "\u{2502} ");
                        }
                    }
                } else {
                    spans.push(Span::styled(text.to_string(), style));
                }
            }
            Event::Code(code) => {
                spans.push(Span::styled(
                    format!(" {code} "),
                    Style::default().fg(Color::LightGreen).bg(Color::Rgb(40, 40, 40)),
                ));
            }
            Event::SoftBreak => spans.push(Span::raw(" ")),
            Event::HardBreak => flush(&mut lines, &mut spans, &quote_prefix(quote_depth)),
            Event::Rule => {
                flush(&mut lines, &mut spans, "");
                lines.push(Line::from(Span::styled(
                    "\u{2500}".repeat(40),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            Event::TaskListMarker(done) => {
                let mark = if done { "[x] " } else { "[ ] " };
                spans.push(Span::styled(mark.to_string(), Style::default().fg(Color::Yellow)));
            }
            Event::End(TagEnd::Paragraph) => {
                flush(&mut lines, &mut spans, &quote_prefix(quote_depth));
                if lists.is_empty() {
                    lines.push(Line::from(""));
                }
            }
            // Tables, footnotes and inline HTML come through as their text: readable, if not
            // laid out. Better than dropping them, and honest about not being a browser.
            _ => {}
        }
        // A heading never spans lines, so nothing else has to remember to close it.
        let _ = heading;
    }
    flush(&mut lines, &mut spans, "");
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a tab is a view of, which decides what its bar may offer. A markdown preview keeps
    /// its source whichever rendering is up, so it must not read as a PDF when pandoc gave it
    /// pages, nor as a picture when it has none.
    #[test]
    fn a_preview_knows_what_it_is_a_view_of() {
        assert_eq!(Preview::picture().kind(), Kind::Picture);
        assert_eq!(Preview::document(1).kind(), Kind::Document);
        let markdown = Preview::rendered(PathBuf::from("notes.md"));
        assert_eq!(markdown.kind(), Kind::Markdown);
        let mut paged_markdown = Preview::rendered(PathBuf::from("notes.md"));
        paged_markdown.pages = Some(Pages { current: 1, total: None });
        assert_eq!(paged_markdown.kind(), Kind::Markdown, "a rendered document is still markdown");
    }

    /// The zoom buttons did nothing on a picture: the file was decoded at its own size whatever
    /// the zoom, and the widget shrank it to the pane either way. The zoom has to reach the
    /// pixels, which means the box a picture is scaled into has to grow with it.
    #[test]
    fn a_picture_is_scaled_to_the_zoom_it_is_looked_at() {
        use image::GenericImageView;
        let mut preview = Preview::picture();
        // Never drawn: nothing to scale against, and `scale_picture` leaves it alone.
        assert_eq!(preview.picture_box(), (0, 0));
        let untouched = image::DynamicImage::new_rgb8(640, 480);
        assert_eq!(scale_picture(untouched, (0, 0), Fit::Page).dimensions(), (640, 480));

        preview.area_cols = 100;
        preview.area_rows = 40;
        let (base_w, base_h) = preview.picture_box();
        assert!(base_w > 0 && base_h > 0);
        preview.zoom = 2.0;
        let (zoomed_w, zoomed_h) = preview.picture_box();
        assert!(zoomed_w > base_w && zoomed_h > base_h, "a zoomed picture is made larger");
    }

    /// How a picture lands in that box: the whole of it for "fit", the width of it for "wide".
    #[test]
    fn a_picture_fits_the_box_it_is_given() {
        use image::GenericImageView;
        let wide = image::DynamicImage::new_rgb8(1000, 200);
        // The whole picture inside the box, aspect kept: the width binds here.
        assert_eq!(scale_picture(wide.clone(), (500, 500), Fit::Page).dimensions(), (500, 100));
        // As wide as the box, however tall that makes it — the rest is scrolled to.
        assert_eq!(scale_picture(wide.clone(), (2000, 500), Fit::Width).dimensions(), (2000, 400));
        // Already the right size: handed back untouched rather than resampled a second time.
        let exact = scale_picture(wide, (500, 500), Fit::Page);
        assert_eq!(scale_picture(exact, (500, 500), Fit::Page).dimensions(), (500, 100));
    }

    /// Markdown's two renderings. Switching between them changes what the view *is* — pages
    /// against one long scroll — so what is on screen cannot be kept, and the next pass over the
    /// buffers has to make the other one.
    #[test]
    fn markdown_switches_between_the_document_and_the_text() {
        let mut preview = Preview::rendered(PathBuf::from("notes.md"));
        preview.shown_revision = 7;
        preview.set_text_only(true);
        assert!(preview.text_only && preview.text_view());
        assert!(preview.pages.is_none(), "styled text is one scroll, not a set of pages");
        assert_ne!(preview.shown_revision, 7, "the view has to be made again");

        preview.shown_revision = 9;
        preview.set_text_only(false);
        assert!(!preview.text_only);
        assert_ne!(preview.shown_revision, 9);
        // Whether it *can* be a document depends on the machine; that it is no longer pinned to
        // text does not.
        assert_eq!(preview.text_view(), !markdown_as_document());
    }

    /// The list decides which files stop being text, so it has to stay narrow. Being unreadable
    /// as text is not evidence of being an image — a .zip is binary too, and a preview tab that
    /// could only ever fail is worse than a read-only one that admits it has nothing.
    #[test]
    fn only_pictures_are_previewed() {
        for ext in ["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tiff", "tif"] {
            assert!(is_previewable(ext), "{ext} should be shown as a picture");
        }
        for ext in ["zip", "pdf", "rs", "md", "py", "svg", "", "exe", "tar"] {
            assert!(!is_previewable(ext), "{ext} must not be treated as a picture");
        }
        // A document is previewed too, but down a different road: it has to be rasterised a
        // page at a time, so it must not be mistaken for something the image crate can open.
        assert!(is_document("pdf"));
        for ext in ["png", "zip", "rs", ""] {
            assert!(!is_document(ext), "{ext} is not a paged document");
        }
    }

    /// A page that does not exist has to come back as an error, because that error *is* how the
    /// end of a document is found — both rasterisers exit 0 and write nothing past the last
    /// page, so anything reading the exit status alone would page on for ever.
    #[test]
    fn a_missing_page_is_an_error_not_a_blank() {
        let dir = std::env::temp_dir().join(format!("clee_pdf_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let not_a_pdf = dir.join("broken.pdf");
        std::fs::write(&not_a_pdf, b"%PDF-1.4 and then nothing valid at all").unwrap();

        // Whether a rasteriser is installed or not, this must be an error and must not hang or
        // panic: no tool at all, and a tool that refuses the file, are the same to the caller.
        assert!(render_page(&not_a_pdf, 1, 1200).is_err());
        assert!(render_page(&not_a_pdf, 99, 1200).is_err());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Sizes are checked from the header rather than after decoding, so a file that would not
    /// fit in memory is refused instead of being allocated first.
    #[test]
    fn an_unreadable_file_fails_rather_than_panicking() {
        let dir = std::env::temp_dir().join(format!("clee_prev_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // Not an image at all, despite the name: reported, not crashed on.
        let fake = dir.join("not-really.png");
        std::fs::write(&fake, b"this is plain text pretending to be a picture").unwrap();
        assert!(decode(&fake).is_err());

        // A file that is not there at all.
        assert!(decode(&dir.join("absent.png")).is_err());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
