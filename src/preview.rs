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
    /// Ready to draw. The protocol holds the picture and, once drawn, the version of it resized
    /// to the pane — which is why the renderer needs it by `&mut`.
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
    /// Set once making a document out of this has failed. The tab then stays on the styled-text
    /// rendering rather than spending half a second failing the same way after every pause —
    /// the reason it failed will not have changed by itself. \u{25b6} Refresh clears it and tries again.
    pub document_failed: bool,
    /// `None` for a picture, which is one image with nothing to page through. `Some` for a PDF,
    /// even before the page count is known: paging has to work while that is still being asked
    /// for, and on a file whose count cannot be established at all.
    pub pages: Option<Pages>,
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
        Preview { state: State::Loading, pages: None, source: None, settled: None, shown_revision: 0, document_failed: false }
    }

    pub fn document(page: usize) -> Self {
        Preview {
            state: State::Loading,
            pages: Some(Pages { current: page, total: None }),
            source: None,
            settled: None,
            shown_revision: 0,
            document_failed: false,
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
        }
    }

    /// Whether the buffer has moved since what is on screen was made.
    pub fn stale(&self, revision: u64) -> bool {
        self.shown_revision != revision
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
    /// A picture, decoded as it is.
    Picture(PathBuf),
    /// One page of a document already on disk.
    Page { path: PathBuf, page: usize },
    /// One page of a document made *from a buffer*: the markdown you are editing, turned into a
    /// real document so that pictures it refers to appear in the flow of the text — which a grid
    /// of cells cannot do, and which is the whole reason this goes the long way round.
    Markdown { path: PathBuf, text: String, page: usize },
}

impl Job {
    fn path(&self) -> &Path {
        match self {
            Job::Picture(path) | Job::Page { path, .. } | Job::Markdown { path, .. } => path,
        }
    }

    fn page(&self) -> Option<usize> {
        match self {
            Job::Picture(_) => None,
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
            Job::Picture(path) => (decode(path), None),
            Job::Page { path, page } => {
                // The count is asked for alongside the first page rather than in its own pass:
                // it needs the same tool, and a second subprocess for a number nobody is
                // waiting on would only slow the page down.
                (render_page(path, *page), page_count(path))
            }
            Job::Markdown { path, text, page } => match markdown_to_pdf(path, text) {
                Ok(pdf) => {
                    let rendered = render_page(pdf.path(), *page);
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

/// Extensions shown a page at a time, by rasterising them first.
pub fn is_document(ext: &str) -> bool {
    ext == "pdf"
}

/// What a page is rendered at. Generous next to a pane's own pixel count — a wide pane is maybe
/// 1000 pixels across — so the picture is downscaled to fit rather than stretched, which is the
/// difference between readable body text and a blur.
const PAGE_DPI: u32 = 150;

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
fn render_page(path: &Path, page: usize) -> Result<image::DynamicImage, String> {
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
            "pdftoppm" => std::process::Command::new(tool)
                .args(["-png", "-r", &PAGE_DPI.to_string()])
                .args(["-f", &page.to_string(), "-l", &page.to_string()])
                .arg("-singlefile")
                .arg(path)
                // pdftoppm appends its own ".png", so it is handed the name without one.
                .arg(out.with_extension(""))
                .output(),
            _ => std::process::Command::new(tool)
                .args(["-q", "-dNOPAUSE", "-dBATCH", "-dSAFER", "-sDEVICE=png16m"])
                .arg(format!("-r{PAGE_DPI}"))
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
    /// Takes a rendered image and readies it for drawing. Fails loudly rather than silently when
    /// the terminal was never asked what it can do — that would be a startup-order bug, and a
    /// blank tab would hide it.
    pub fn ready(image: image::DynamicImage) -> State {
        match picker() {
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
        assert!(render_page(&not_a_pdf, 1).is_err());
        assert!(render_page(&not_a_pdf, 99).is_err());

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
