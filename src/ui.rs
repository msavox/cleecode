use crate::app::{App, EditorPane, Focus};
use crate::i18n::{self, Key, Lang};
use crate::keymap::{self, Keymap};
use crate::menu::{self, ContextMenu, MenuBar};
use crate::terminal_panel::{TermSelection, TerminalWindow};
use crate::settings;
use crate::theme::Palette;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Wrap,
};
use ratatui::Frame;
use ratatui_image::StatefulImage;
use std::time::Duration;

pub struct Areas {
    pub menu_bar: Rect,
    pub sidebar: Option<Rect>,
    pub editor: Rect,
    pub terminals: Option<Vec<Rect>>,
    /// The agent drawer's column, when it is open **and pinned**. Always the rightmost thing in
    /// the window, and carved out of the main area before anything else is placed, so no other
    /// frame's rect ever overlaps it.
    pub drawer: Option<Rect>,
    /// The same rectangle when the drawer is open and set to autocollapse: painted over the
    /// frames rather than taken out of them, so it is *not* subtracted from anything above and
    /// every other rect here is exactly what it would be with no drawer at all.
    ///
    /// Two fields rather than one plus a flag because the difference is what the rest of the
    /// layout means: `drawer` is a region nobody else owns, `drawer_overlay` is a region
    /// somebody else also owns and the drawer is simply on top of. Code that must not be fooled
    /// by the overlay (the terminal seam's arithmetic, which is a percentage of what the drawer
    /// left) reads `drawer`; code about what is on screen reads [`drawer_rect`].
    pub drawer_overlay: Option<Rect>,
    /// The one column the ribbon rides: the right edge of the main area, carved out of it the
    /// way the drawer's own column is, so it is a region nobody else owns rather than a strip
    /// painted over somebody's last column of text.
    ///
    /// Carved whenever the drawer is not a pinned column — which includes *while an
    /// autocollapsing drawer is open over the top of it*. That is deliberate: the whole promise
    /// of autocollapse is that nothing underneath is resized, and a column handed back to the
    /// frames every time the drawer appeared would be a `SIGWINCH` on the way in and another on
    /// the way out. The overlay simply covers these cells while it is up, so the ribbon is not
    /// drawn and not clickable then — see [`drawer_ribbon_rect`], which is what the drawing and
    /// the mouse both ask.
    pub drawer_ribbon: Option<Rect>,
    pub status: Rect,
}

/// Where the drawer is on screen right now, in whichever mode it is in — the one question the
/// mouse asks, since a click lands on what it can see and not on what owns the cells.
pub fn drawer_rect(areas: &Areas) -> Option<Rect> {
    areas.drawer.or(areas.drawer_overlay)
}

/// The ribbon as something to see and to click, which is the column above *and* the drawer being
/// away. One function for the drawing and for the hit testing, so a handle that is not on screen
/// can never be the thing a click lands on.
pub fn drawer_ribbon_rect(areas: &Areas) -> Option<Rect> {
    drawer_rect(areas).is_none().then_some(areas.drawer_ribbon).flatten()
}

/// The other handle: the column an open drawer's left border rides, in whichever mode it is in.
///
/// The mirror of [`drawer_ribbon_rect`], and never on screen at the same time as it — the drawer
/// is either there or it is not, and the edge of the window carries whichever of the two applies.
/// It is not carved out of anything: the drawer's own border is already a column nobody else
/// owns, and the handle is painted on it.
pub fn drawer_close_ribbon_rect(areas: &Areas) -> Option<Rect> {
    drawer_rect(areas).filter(|r| r.width > 0).map(|r| Rect { width: 1, ..r })
}

pub struct LayoutParams {
    pub show_sidebar: bool,
    pub show_terminal: bool,
    pub show_menubar: bool,
    /// Whether a menu is open right now. The bar needs its row either way: hiding the bar is a
    /// preference about the idle screen, not a refusal to ever show a menu, and `Ctrl+Shift+B`
    /// is documented as reaching the menus while it is hidden.
    pub menu_active: bool,
    /// One relative weight per terminal window; adjacent weights shift when their seam is
    /// dragged. Its length is the window count.
    pub terminal_weights: Vec<u16>,
    pub sidebar_width: u16,
    pub terminal_pct: u16,
    pub terminal_on_right: bool,
    /// Whether the agent drawer has a column of its own right now.
    pub drawer_open: bool,
    /// Its share of the window, as a percentage. See `settings::drawer_pct`.
    pub drawer_pct: u16,
    /// Whether it is part of the layout (pinned) or painted over it (autocollapse). See
    /// `settings::drawer_pinned` for why the mode and the compositing are one setting.
    pub drawer_pinned: bool,
}

impl LayoutParams {
    pub fn from_app(app: &App) -> Self {
        LayoutParams {
            show_sidebar: app.settings.show_sidebar,
            show_terminal: app.settings.show_terminal,
            show_menubar: app.settings.show_menubar,
            menu_active: app.menu.active,
            terminal_weights: app.terminals.iter().map(|w| w.weight).collect(),
            sidebar_width: app.settings.sidebar_width,
            terminal_pct: app.settings.terminal_pct,
            terminal_on_right: app.settings.terminal_on_right,
            drawer_open: app.drawer.as_ref().is_some_and(|d| d.open),
            drawer_pct: app.settings.drawer_pct,
            drawer_pinned: app.settings.drawer_pinned,
        }
    }
}

/// Tiles the terminal region into one pane per window, sized by relative weight so a dragged seam
/// can give one window more room than its neighbours.
fn terminal_panes(area: Rect, weights: &[u16], direction: Direction) -> Vec<Rect> {
    let constraints: Vec<Constraint> = if weights.is_empty() {
        vec![Constraint::Fill(1)]
    } else {
        weights.iter().map(|&w| Constraint::Fill(w.max(1))).collect()
    };
    Layout::default().direction(direction).constraints(constraints).split(area).to_vec()
}

/// Splits the main area into (what is left of it, the drawer's rectangle).
///
/// One function for both modes, and therefore for drawing and for hit testing alike: the pinned
/// column and the autocollapsed overlay are the same rectangle, and the only thing the mode
/// decides is whether the first half of this pair is handed to everybody else or thrown away.
/// Two copies of this arithmetic would be two chances for a click to land a column off the
/// border it can see.
fn drawer_split(main: Rect, pct: u16) -> (Rect, Rect) {
    let h = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(100 - pct), Constraint::Percentage(pct)])
        .split(main);
    (h[0], h[1])
}

/// The ribbon's column, one cell wide, off the right of the main area — and what is left for
/// everybody else.
///
/// The narrowest main area that can spare it: below this the column would be a larger share of
/// the window than the frames it is advertising, and a window that small has other problems than
/// finding the drawer with a mouse. `None` then, and the ribbon simply is not there — the chord
/// and the View menu still are.
fn drawer_ribbon_split(main: Rect) -> (Rect, Option<Rect>) {
    const NARROWEST: u16 = 8;
    if main.width < NARROWEST || main.height == 0 {
        return (main, None);
    }
    let rest = Rect { width: main.width - 1, ..main };
    let ribbon = Rect { x: main.x + main.width - 1, width: 1, ..main };
    (rest, Some(ribbon))
}

/// The chevrons the two handles carry: out towards the drawer, back towards the work.
const RIBBON_OPEN_MARK: &str = "\u{2039}";
const RIBBON_CLOSE_MARK: &str = "\u{203A}";

/// The tallest a handle gets, and the share of the edge it may take on a short window.
///
/// Seven rows is a grip: long enough to aim at without looking, short enough to leave the edge
/// obviously an edge. A third of the height is the fallback that keeps it in proportion when
/// there is not enough window for that — the handle shrinks with the column rather than
/// swallowing it.
const RIBBON_PILL_MAX: u16 = 7;
const RIBBON_PILL_SHARE: u16 = 3;

/// How many bands the handle is extended with, three above the chevron's block and three below.
///
/// The colours are not here. They are `Palette::handle_stripes`, one set per theme, and the
/// reasoning is written on that field: six colours are a signature, and this file drawing the
/// same signature over nine editors would be the drawing code deciding what each of them looks
/// like. What this file owns is the *shape* — six bands, evenly, around the block — and every
/// theme fills it in with its own.
///
/// They are decoration and never a state: the pointer lights the chevron's block and leaves these
/// exactly as they are, because a thing that changes under the mouse is telling you it is a
/// control, and only the block is.
const RIBBON_BANDS: usize = 6;

/// How tall a band gets, tallest first. The list *is* the degradation: the first height whose
/// whole handle fits the column with a row of edge left over at each end is the one drawn, and a
/// column too short for even the thin banding gets the chevron's block on its own.
const RIBBON_STRIPE_HEIGHTS: [u16; 2] = [2, 1];

/// Where a handle's parts sit in the column it was given.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RibbonHandle {
    /// Everything, bands and block together, centred in the column.
    pub rect: Rect,
    /// The chevron's block, in the middle of it. Always drawn; always the click's own target.
    pub pill: Rect,
    /// How tall each band is, or `0` on a column with no room for them.
    pub stripe: u16,
}

/// The handle a ribbon draws on its column: a filled block with the chevron in it, banded above
/// and below in the six colours.
///
/// A filled block and not a run of ticks. Sparse marks down the whole edge were the first attempt
/// and they read as decoration — something the theme does to the border — rather than as a thing
/// to press. One solid grip, centred, is what every editor with a dockable side panel puts on
/// that edge, and it is legible at a glance from the far side of the screen. The bands came after
/// and are the part that makes it *this* editor's grip rather than any of theirs.
///
/// The *column* stays the click target whatever this returns; this is only what is drawn on it. A
/// control you can hit anywhere along the edge and see in one place is more forgiving than one
/// that is only where it is painted — and it is what lets the handle shrink on a short window
/// without the target shrinking with it.
pub fn drawer_ribbon_handle(rect: Rect) -> RibbonHandle {
    if rect.height == 0 {
        let empty = Rect { height: 0, ..rect };
        return RibbonHandle { rect: empty, pill: empty, stripe: 0 };
    }
    let tall = (rect.height / RIBBON_PILL_SHARE).clamp(1, RIBBON_PILL_MAX);
    let bands = RIBBON_BANDS as u16;
    // A row of clearance at each end, so the handle reads as sitting on the edge rather than as
    // the edge having been replaced by it.
    let stripe = RIBBON_STRIPE_HEIGHTS
        .into_iter()
        .find(|s| tall + s * bands + 2 <= rect.height)
        .unwrap_or(0);
    let total = tall + stripe * bands;
    let top = rect.y + (rect.height - total) / 2;
    RibbonHandle {
        rect: Rect { y: top, height: total, ..rect },
        pill: Rect { y: top + stripe * (bands / 2), height: tall, ..rect },
        stripe,
    }
}

/// A handle: the way into a drawer that is away, or the way out of one that is here.
///
/// The block is filled in the accent with the chevron in the colour that colour is meant to be
/// read against, and under the pointer it goes to `bright` — the strongest colour the theme has,
/// which is a step *up* from the accent on a dark theme and a step down into it on a light one,
/// and either way unmistakably a different thing than it was a moment ago. Both are palette roles
/// rather than colours, so the nine themes each get their own answer: `on_accent` is by
/// construction readable on `accent`, and it is equally readable on `bright`, because a theme's
/// brightest colour and its text-on-a-swatch colour sit at opposite ends of the same range.
///
/// The bands around it are the theme's own six — `Palette::handle_stripes`, quotations rather
/// than roles — and they do not answer the pointer at all. See [`RIBBON_BANDS`].
pub fn draw_drawer_ribbon(f: &mut Frame, pal: Palette, rect: Rect, engaged: bool, mark: &str) {
    let handle = drawer_ribbon_handle(rect);
    let pill = handle.pill;
    if pill.height == 0 || pill.width == 0 {
        return;
    }
    if handle.stripe > 0 {
        let half = (RIBBON_BANDS / 2) as u16;
        for (i, colour) in pal.handle_stripes.iter().enumerate() {
            let i = i as u16;
            // The order is one run top to bottom with the block in the middle of it, so the
            // second three start below the block rather than where the first three left off.
            let y = if i < half {
                handle.rect.y + i * handle.stripe
            } else {
                pill.y + pill.height + (i - half) * handle.stripe
            };
            let band = Rect { y, height: handle.stripe, ..rect };
            f.render_widget(Block::default().style(Style::default().bg(*colour)), band);
        }
    }
    let style =
        Style::default().bg(if engaged { pal.bright } else { pal.accent }).fg(pal.on_accent);
    let middle = pill.height / 2;
    let rows: Vec<Line> = (0..pill.height)
        .map(|i| Line::from(Span::styled(if i == middle { mark } else { " " }, style)))
        .collect();
    f.render_widget(Paragraph::new(rows).style(style), pill);
}

pub fn compute_layout(full: Rect, p: &LayoutParams) -> Areas {
    let menu_h = if p.show_menubar || p.menu_active { 1 } else { 0 };
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(menu_h), Constraint::Min(1), Constraint::Length(1)])
        .split(full);
    let menu_bar = outer[0];
    let main_area = outer[1];
    let status = outer[2];

    // The drawer comes off the right of the whole main area, before either arrangement below is
    // worked out — it is the rightmost column of the window in both, and everything else divides
    // what is left.
    //
    // Before, and not inside the branches, because the right-docked terminal panel is *also* a
    // column carved off the right. Two of them worked out independently would each take their
    // percentage of the same edge and land on top of each other; taken in this order the terminal
    // takes its share of what the drawer left, which is what "the drawer is part of the layout
    // and everything makes room for it" means in arithmetic. This is the pinned mode's whole
    // cost: opening the drawer resizes the frames, and every pty in them gets a SIGWINCH.
    //
    // In autocollapse the carve does not happen at all — that is the point of the mode. The
    // rectangle is the same one either way, so the drawer does not shift under the eye when the
    // setting changes; what changes is whether anything else was moved out of its way.
    //
    // And where the drawer is *not* a column of the layout, the ribbon is: one cell off the same
    // edge, so the way back in with a mouse is a region nobody else owns rather than a mark
    // painted over the last column of somebody's text. It is carved in both arrangements and in
    // both modes — including under an open autocollapsing drawer, where it is covered rather than
    // given back. Handing that column to the frames whenever the overlay appeared would resize
    // them twice for every visit, which is exactly the cost the mode exists to avoid.
    let (main_area, drawer, drawer_overlay, drawer_ribbon) = if p.drawer_open {
        let (rest, column) = drawer_split(main_area, p.drawer_pct);
        if p.drawer_pinned {
            (rest, Some(column), None, None)
        } else {
            let (rest, ribbon) = drawer_ribbon_split(main_area);
            (rest, None, Some(column), ribbon)
        }
    } else {
        let (rest, ribbon) = drawer_ribbon_split(main_area);
        (rest, None, None, ribbon)
    };

    if p.terminal_on_right {
        let (sidebar, rest) = if p.show_sidebar {
            let h = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(p.sidebar_width), Constraint::Min(1)])
                .split(main_area);
            (Some(h[0]), h[1])
        } else {
            (None, main_area)
        };
        let (editor, terminals) = if p.show_terminal {
            let h = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(100 - p.terminal_pct), Constraint::Percentage(p.terminal_pct)])
                .split(rest);
            (h[0], Some(terminal_panes(h[1], &p.terminal_weights, Direction::Vertical)))
        } else {
            (rest, None)
        };
        Areas { menu_bar, sidebar, editor, terminals, drawer, drawer_overlay, drawer_ribbon, status }
    } else {
        let (main_top, terminals) = if p.show_terminal {
            let v = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(100 - p.terminal_pct), Constraint::Percentage(p.terminal_pct)])
                .split(main_area);
            (v[0], Some(terminal_panes(v[1], &p.terminal_weights, Direction::Horizontal)))
        } else {
            (main_area, None)
        };

        let (sidebar, editor) = if p.show_sidebar {
            let h = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(p.sidebar_width), Constraint::Min(1)])
                .split(main_top);
            (Some(h[0]), h[1])
        } else {
            (None, main_top)
        };

        Areas { menu_bar, sidebar, editor, terminals, drawer, drawer_overlay, drawer_ribbon, status }
    }
}

pub fn inner_rect(r: Rect) -> Rect {
    Block::default().borders(Borders::ALL).inner(r)
}

/// Splits the editor column into (tab bar row, remaining content area).
/// Splits the whole editor region into one (unsplit) or two side-by-side panes. Index 0
/// is always the left/only pane, index 1 (when present) the right one.
pub fn editor_pane_rects(area: Rect, split: bool, left_pct: u16) -> Vec<Rect> {
    if !split {
        return vec![area];
    }
    // Under two columns there is no split to make — one side would have to be zero-wide — and
    // asking for it is not merely pointless but fatal: the clamp below would be handed a minimum
    // of 1 above a maximum of 0, and `clamp` panics on an inverted range. A window dragged narrow
    // with the split on reached exactly that, and it closed the editor. One pane is the answer.
    if area.width < 2 {
        return vec![area];
    }
    // The left pane gets `left_pct` of the width; the right takes the remainder, so no column is
    // lost to rounding. Both keep at least one column even at the clamp extremes.
    let mid = ((area.width as u32 * left_pct as u32) / 100) as u16;
    let mid = mid.clamp(1, area.width - 1);
    let left = Rect { width: mid, ..area };
    let right = Rect { x: area.x + mid, width: area.width - mid, ..area };
    vec![left, right]
}

pub fn split_editor_area(area: Rect) -> (Rect, Rect) {
    if area.height <= 1 {
        return (Rect { height: 0, ..area }, area);
    }
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);
    (v[0], v[1])
}

/// The same split with a row for the markdown formatting bar between the tabs and the text:
/// (tab bar, bar, content). The middle one is `None` when the bar is not up, and the other two
/// are then exactly what [`split_editor_area`] gives.
///
/// The three always partition the area between them — no row belongs to two of them and none to
/// nothing — because a click is placed by asking which of the three contains it.
pub fn split_editor_area_v2(area: Rect, with_toolbar: bool) -> (Rect, Option<Rect>, Rect) {
    let (tab_bar, rest) = split_editor_area(area);
    // Never the last row: a bar with no text under it is a bar over nothing.
    if !with_toolbar || rest.height <= 1 {
        return (tab_bar, None, rest);
    }
    let toolbar = Rect { height: 1, ..rest };
    let content = Rect { y: rest.y + 1, height: rest.height - 1, ..rest };
    (tab_bar, Some(toolbar), content)
}

/// A pane's three rectangles, with the formatting bar in them exactly when it is on screen.
///
/// Every call site goes through here rather than deciding for itself, so the renderer and the
/// mouse cannot come to different answers about which row a click landed on — the same rule the
/// tab strip's layout and the preview's buttons already follow.
pub fn pane_areas(app: &App, idx: usize, pane_rect: Rect) -> (Rect, Option<Rect>, Rect) {
    split_editor_area_v2(pane_rect, md_toolbar_visible(app, idx, pane_rect))
}

pub struct TabLayout {
    /// Whole-tab range: clicking anywhere in it (outside `close`) switches to it.
    pub full: (u16, u16),
    /// The columns the title text is drawn into, padding included. Narrower than the title
    /// wants when the strip cannot hold a whole tab, in which case the drawing ellipsizes into
    /// it — so this is what the renderer measures against rather than the title's own length.
    pub label: (u16, u16),
    /// What a click has to hit to close the tab. Wider than the glyph: a one-cell target is one
    /// the mouse misses, which is why the preview bar grew `NAV_MIN_WIDTH` after the same
    /// complaint. The padding either side of the × belongs to nothing else, so it is given away
    /// here — the glyph is still drawn in exactly one cell, and the tab is unchanged to look at.
    pub close: (u16, u16),
}

/// The arrow glyphs that stand in for tabs scrolled out of view, and the columns they take.
pub const SCROLL_LEFT_GLYPH: &str = "\u{2039}";
pub const SCROLL_RIGHT_GLYPH: &str = "\u{203a}";
const ARROW_W: u16 = 1;

/// Columns kept for the tab strip before the toolbar buttons give up their space. On a
/// narrow pane (a split view on a small terminal) seeing which files are open matters more
/// than the buttons, which stay reachable via `Ctrl+Shift+R` and the Run menu.
const MIN_TAB_STRIP: u16 = 12;

/// The visible slice of a pane's tab strip. Ranges are relative to the tab bar's left edge.
pub struct TabStrip {
    /// Index of the leftmost rendered tab, after scrolling to keep the active one in view.
    pub first: usize,
    /// One entry per rendered tab, left to right, starting at `first`.
    pub tabs: Vec<TabLayout>,
    /// The `‹` glyph, present when tabs are scrolled out of view to the left.
    pub left_arrow: Option<(u16, u16)>,
    /// The `›` glyph, present when tabs remain past the right edge.
    pub right_arrow: Option<(u16, u16)>,
}

impl TabStrip {
    /// The tab whose range contains `col`, as an index into all open editors.
    pub fn tab_at(&self, col: u16) -> Option<(usize, &TabLayout)> {
        self.tabs
            .iter()
            .enumerate()
            .find(|(_, t)| col >= t.full.0 && col < t.full.1)
            .map(|(i, t)| (self.first + i, t))
    }
}

/// Width each tab occupies: `" title* "` plus the `×` glyph and a trailing space.
/// The width of each tab in one pane's strip. Per pane, not per buffer: the two halves of a
/// split hold different files, so they have different strips.
pub fn tab_widths(app: &App, pane: EditorPane) -> Vec<u16> {
    let lang = app.settings.lang;
    app.pane_tabs(pane)
        .iter()
        .filter_map(|&i| app.editors.get(i))
        .map(|editor| {
            let dirty = if editor.dirty { "*" } else { "" };
            let prefix = format!(" {}{} ", editor.title(lang), dirty);
            prefix.chars().count() as u16 + 2 // + close glyph + trailing space
        })
        .collect()
}

/// How many tabs fit from `first` within `width`, plus whether each scroll arrow is needed.
/// The right arrow only claims a column once there is something hidden behind it, which can
/// itself push a tab out of view — hence the second pass.
fn fit_tabs(widths: &[u16], width: u16, first: usize) -> (usize, bool, bool) {
    let left = first > 0;
    let count_within = |avail: u16| {
        let mut used = 0u16;
        let mut count = 0usize;
        for w in &widths[first..] {
            if used + w > avail {
                break;
            }
            used += w;
            count += 1;
        }
        count
    };
    let avail = width.saturating_sub(if left { ARROW_W } else { 0 });
    let count = count_within(avail);
    if first + count >= widths.len() {
        return (count, left, false);
    }
    (count_within(avail.saturating_sub(ARROW_W)), left, true)
}

/// The smallest adjustment to `offset` that brings `active` into view: back to it when it sits
/// before the window, forward until it fits when it sits after.
///
/// Deliberately *not* applied on every render. Doing that made the strip snap back to the
/// active tab a frame after any manual scroll, so the `‹` arrow appeared to do nothing at all
/// — scrolling away from the active tab was undone instantly. Callers apply this when the
/// active tab changes instead, which leaves a deliberate scroll alone.
pub fn offset_revealing(widths: &[u16], width: u16, offset: usize, active: usize) -> usize {
    if widths.is_empty() {
        return 0;
    }
    let last = widths.len() - 1;
    let mut first = offset.min(last);
    if active <= first {
        return active.min(last);
    }
    loop {
        let (count, _, _) = fit_tabs(widths, width, first);
        // `count == 0` means no tab fits whole, and the strip then draws the one at `first`
        // clipped — so there is still exactly one tab on screen to walk toward.
        if active < first + count.max(1) || first >= last {
            return first;
        }
        first += 1;
    }
}

/// The narrowest a clipped tab can usefully be: one column of title, the × and its space.
const MIN_CLIPPED_TAB: u16 = 3;

/// One tab occupying `x..x + w`, where `w` is either the width the title wants or — for the
/// clipped case below — all the room there is.
fn tab_layout_at(x: u16, w: u16, clipped: bool) -> TabLayout {
    let close_start = x + w - 2; // the × sits before the trailing space
    // The trailing space is always part of the target. The space *before* the glyph is too, but
    // only when there is one: a clipped title ends in an ellipsis, and swallowing that column
    // would close the tab from a cell showing text.
    let close_from = if clipped { close_start } else { close_start.saturating_sub(1) };
    TabLayout { full: (x, x + w), label: (x, close_start), close: (close_from, x + w) }
}

/// Lays out the visible part of the tab strip, starting at `offset`.
pub fn tab_strip_layout(widths: &[u16], width: u16, offset: usize) -> TabStrip {
    let empty = TabStrip { first: 0, tabs: Vec::new(), left_arrow: None, right_arrow: None };
    if widths.is_empty() || width == 0 {
        return empty;
    }
    let first = offset.min(widths.len() - 1);
    let (count, left, right) = fit_tabs(widths, width, first);
    if count == 0 {
        return clipped_tab_strip(widths, width, first);
    }

    let mut x = if left { ARROW_W } else { 0 };
    let mut tabs = Vec::with_capacity(count);
    for w in &widths[first..first + count] {
        tabs.push(tab_layout_at(x, *w, false));
        x += w;
    }
    TabStrip {
        first,
        tabs,
        left_arrow: left.then_some((0, ARROW_W)),
        right_arrow: right.then_some((width - ARROW_W, width)),
    }
}

/// The strip when not even one whole tab fits — a long file name in a narrow split, which is
/// ordinary rather than exotic.
///
/// Returning nothing here is what made the whole bar disappear: no name, no dirty marker and no
/// ×, so the one open file had nothing on screen and no way to be closed with the mouse. A tab
/// cut short with an ellipsis says less than a whole one and everything more than an empty row.
///
/// The arrows still come first when there is anything on either side, because a strip you cannot
/// scroll out of is a strip stuck on whichever tab it opened on.
fn clipped_tab_strip(widths: &[u16], width: u16, first: usize) -> TabStrip {
    let empty = TabStrip { first: 0, tabs: Vec::new(), left_arrow: None, right_arrow: None };
    let left = first > 0;
    let mut avail = width.saturating_sub(if left { ARROW_W } else { 0 });
    // The right arrow only takes its column while what is left still makes a readable tab.
    let right = first + 1 < widths.len() && avail >= MIN_CLIPPED_TAB + ARROW_W;
    if right {
        avail -= ARROW_W;
    }
    if avail < MIN_CLIPPED_TAB {
        return empty;
    }
    let x = if left { ARROW_W } else { 0 };
    TabStrip {
        first,
        tabs: vec![tab_layout_at(x, avail, true)],
        left_arrow: left.then_some((0, ARROW_W)),
        right_arrow: right.then_some((width - ARROW_W, width)),
    }
}

/// The action button. On a preview tab it is a refresh — the file is already being shown, so
/// "run" would mean nothing there, and a button that means nothing is worse than one that means
/// something small.
fn run_button_label(app: &App, idx: usize) -> String {
    let key = if app.editors.get(idx).is_some_and(|e| e.preview.as_ref().is_some_and(|p| p.refreshable())) {
        Key::ToolbarRefresh
    } else {
        Key::ToolbarRun
    };
    let label = i18n::t(app.settings.lang, key);
    // Padded to whichever of the two words is longer, so switching to a preview tab cannot
    // shift the toolbar sideways under the pointer.
    let width = i18n::t(app.settings.lang, Key::ToolbarRun)
        .chars()
        .count()
        .max(i18n::t(app.settings.lang, Key::ToolbarRefresh).chars().count());
    format!(" \u{25b6} {label:<width$} ")
}

/// Short label for a venv in the toolbar. A nickname from `registered_venvs` wins, since it
/// is what the user chose to call it. Otherwise: an auto-discovered venv is already a bare
/// folder name, while a registered one is an absolute path whose full form would crowd out
/// the tab strip, so it shows just the venv folder — prefixed with its parent when the
/// folder name is a generic one that on its own wouldn't say which venv is active.
pub fn venv_display_name(venv: &str, registered: &[settings::RegisteredVenv]) -> String {
    if let Some(nickname) = registered.iter().find(|r| r.path() == venv).and_then(|r| r.nickname()) {
        return nickname.to_string();
    }
    // Cut by hand on both separators rather than through `Path`, which answers for the platform
    // it was compiled for: `is_absolute` is false on Windows for `/opt/venvs/ml-3.12` — no drive
    // letter — so a venv registered on a Mac and read on a PC showed as its whole path, and a
    // settings.toml does get copied between machines. `run_program_name` below cuts both too.
    let mut parts = venv.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).filter(|p| !p.is_empty());
    // No separator at all: an auto-discovered venv, whose folder name is already the label.
    let Some(name) = parts.next() else { return venv.to_string() };
    if matches!(name, ".venv" | "venv" | "env" | ".env") {
        if let Some(parent) = parts.next() {
            return format!("{parent}/{name}");
        }
    }
    name.to_string()
}

/// The program a run command starts with, as the toolbar should name it: the first word,
/// reduced to its file name when the template spells out a whole path.
///
/// A quoted program path — one with spaces in it, the usual shape on Windows — is split the way
/// a shell would, so it stays one token rather than two. An unquoted one is taken at its word
/// instead: `shell_words` is a POSIX splitter, where a backslash escapes the character after it,
/// and putting `C:\Octave\octave-cli.exe` through it produced `C:Octaveoctave-cli.exe` — a name
/// with no separator left in it to cut, so the whole mangled thing ended up on the button. Both
/// separators are cut on either platform: a settings.toml is copied between machines.
pub fn run_program_name(template: &str) -> String {
    let first_word = || template.split_whitespace().next().unwrap_or("").to_string();
    let program = if template.trim_start().starts_with(['"', '\'']) {
        shell_words::split(template)
            .ok()
            .and_then(|words| words.into_iter().next())
            .unwrap_or_else(first_word)
    } else {
        first_word()
    };
    let name = program.rsplit(['/', '\\']).next().unwrap_or(&program);
    name.strip_suffix(".exe").unwrap_or(name).to_string()
}

/// What the run-target button says about the buffer at `idx`: the answer to "what will Run use
/// on this file". For a python file that is which interpreter — the venv, the one choice that
/// isn't already written in the run command. For everything else it is the command's program,
/// or that there is no command yet.
pub fn run_target_text(app: &App, idx: usize) -> String {
    let lang = app.settings.lang;
    let ext = app.editor_ext(idx);
    if crate::app::is_python_ext(&ext) {
        // The selected venv is remembered globally, so opening a project that doesn't have it
        // left the button naming a venv that isn't there while runs quietly fell back to system
        // python. The label follows what would actually be used.
        return match crate::app::effective_venv(app.settings.active_venv.as_deref(), &app.available_venvs) {
            Some(name) => format!("venv: {}", venv_display_name(name, &app.settings.registered_venvs)),
            None => i18n::t(lang, Key::ToolbarVenvNone).to_string(),
        };
    }
    match app.run_command_for(&ext) {
        Some(template) => run_program_name(template),
        None => i18n::t(lang, Key::ToolbarRunNone).to_string(),
    }
}

/// Truncates to `width` columns, marking that it was cut.
fn fit(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    text.chars().take(width.saturating_sub(1)).chain(std::iter::once('\u{2026}')).collect()
}

/// Columns the run-target button takes, padding and drop-down arrow included.
///
/// Sized for the widest label any *open* file would put there, not for the file on screen: the
/// label now changes with the active tab, and letting the button's width follow it would resize
/// the tab strip — shuffling the tabs sideways — every time you switched tab.
pub fn run_target_button_width(app: &App) -> u16 {
    let widest =
        (0..app.editors.len()).map(|i| run_target_text(app, i).chars().count() as u16).max().unwrap_or(0);
    // A leading space, then " \u{25be} " after the text.
    (widest + 4).clamp(14, 26)
}

fn run_target_button_label(app: &App, idx: usize) -> String {
    let width = run_target_button_width(app) as usize - 4;
    format!(" {:<width$} \u{25be} ", fit(&run_target_text(app, idx), width))
}

/// Relative (start, end) ranges for the right-aligned toolbar buttons that fit within
/// `area_width`: the run-target selector (dropped first if there isn't room for both) and the
/// Run button. Their space is reserved up front, independent of how many tabs are open — the tab
/// strip scrolls instead of pushing the buttons off the bar — but they yield once fewer than
/// `MIN_TAB_STRIP` columns would be left for the tabs themselves.
///
/// Pane-independent, because the target button's width is too: only its text differs per pane.
pub fn toolbar_button_ranges(app: &App, area_width: u16) -> (Option<(u16, u16)>, Option<(u16, u16)>) {
    let run_w = run_button_label(app, 0).chars().count() as u16;
    let target_w = run_target_button_width(app);

    if target_w + run_w + MIN_TAB_STRIP <= area_width {
        let run_start = area_width - run_w;
        let target_start = run_start - target_w;
        (Some((target_start, target_start + target_w)), Some((run_start, run_start + run_w)))
    } else if run_w + MIN_TAB_STRIP <= area_width {
        let run_start = area_width - run_w;
        (None, Some((run_start, run_start + run_w)))
    } else {
        (None, None)
    }
}

/// Columns available to the tab strip once the toolbar buttons have taken their place.
pub fn tab_strip_width(app: &App, area_width: u16) -> u16 {
    let (target_range, run_range) = toolbar_button_ranges(app, area_width);
    match target_range.or(run_range) {
        Some((start, _)) => start,
        None => area_width,
    }
}

pub fn gutter_width(total_lines: usize, show_line_numbers: bool) -> u16 {
    if !show_line_numbers {
        return 0;
    }
    let digits = total_lines.max(1).to_string().len().max(3);
    (digits + 1) as u16
}

fn centered_rect(width: u16, height: u16, full: Rect) -> Rect {
    let width = width.min(full.width.max(1));
    let height = height.min(full.height.max(1));
    let x = full.x + (full.width.saturating_sub(width)) / 2;
    let y = full.y + (full.height.saturating_sub(height)) / 2;
    Rect { x, y, width, height }
}

const MENU_LOGO: &str = " 🐢 ";

/// How many columns a piece of the menu bar takes on screen, which is not how many `char`s it
/// is made of: the turtle in the logo is one character and two columns wide. Everything on this
/// row is placed by counting from one end or the other, so measuring in characters put the
/// right-hand end of the bar — and every click mapped onto it — a column out.
fn columns(text: &str) -> u16 {
    Span::raw(text).width() as u16
}

pub fn menu_title_ranges(menu: &MenuBar, lang: Lang) -> Vec<(u16, u16)> {
    let mut ranges = Vec::new();
    let mut x = columns(MENU_LOGO);
    for def in &menu.defs {
        let label = format!(" {} ", i18n::menu_title(lang, def.title_key));
        let w = columns(&label);
        ranges.push((x, x + w));
        x += w;
    }
    ranges
}

/// The same titles, cut to a bar `width` columns wide.
///
/// The unclamped ranges above are geometry — where the titles would go — and on a narrow window
/// the last of them run past the right edge. A title half on screen keeps the half that is
/// drawn; one entirely past the edge collapses to an empty range and so matches no column at
/// all. Both the drawing and the background button beside it work from this, so the bar cannot
/// claim room it never paints, and a click cannot land on a word that is not there.
pub fn menu_titles_within(menu: &MenuBar, lang: Lang, width: u16) -> Vec<(u16, u16)> {
    menu_title_ranges(menu, lang)
        .into_iter()
        .map(|(start, end)| (start.min(width), end.min(width)))
        .collect()
}

/// The menu bar's background button, as it is drawn: a half-filled circle while the background
/// is the terminal's, a full one while it is ours — the same way opacity is drawn everywhere
/// else. Three columns, because it is a switch and not a label; what it does is in the View menu
/// next to it, and in the manual.
///
/// Deliberately not the turtle, which sits at the other end of this row already meaning
/// something else, and which is two columns of emoji whose width not every terminal agrees on.
const BACKGROUND_BUTTON: [&str; 2] = [" ◐ ", " ● "];

/// The theme button, immediately left of the background one. Three columns like its
/// neighbour, so the two read as a pair rather than as one control and an ornament.
const THEME_BUTTON: &str = " ◩ ";

/// The badge naming the open workspace, right-aligned on the menu bar. Empty when none is open.
/// Built here rather than in the drawing code because the button beside it has to know how wide
/// it is, and a click has to land where the eye says it should.
fn workspace_badge(app: &App) -> String {
    app.active_workspace
        .as_deref()
        .map(|name| format!(" {} {} ", i18n::t(app.settings.lang, Key::WorkspaceBadge), name))
        .unwrap_or_default()
}

/// Where the background button sits on a bar `width` columns wide: hard against the workspace
/// badge, or against the right edge when there is no badge.
///
/// Empty when the bar is too narrow to hold it clear of the menu titles. A button drawn over a
/// title would be a button that cannot be seen and a title that cannot be clicked, and returning
/// nothing here removes both at once — the View menu entry still does the job.
pub fn menu_bar_button_range(app: &App, width: u16) -> std::ops::Range<u16> {
    let badge = columns(&workspace_badge(app));
    let titles = menu_titles_within(&app.menu, app.settings.lang, width)
        .last()
        .map(|(_, end)| *end)
        .unwrap_or(0);
    button_range(width, titles, badge)
}

/// Where the theme button sits: hard against the background button, and gone whenever that one
/// is. They give up their room in that order — the background button is the one worth keeping
/// longest, because it is the way back from a screen that cannot be read at all.
pub fn menu_bar_theme_range(app: &App, width: u16) -> std::ops::Range<u16> {
    let background = menu_bar_button_range(app, width);
    if background.is_empty() {
        return 0..0;
    }
    let titles = menu_titles_within(&app.menu, app.settings.lang, width)
        .last()
        .map(|(_, end)| *end)
        .unwrap_or(0);
    theme_range(background.start, titles)
}

/// The arithmetic of the above. `background_start` is where the button it leans on begins, which
/// is the only thing about that button this needs to know.
fn theme_range(background_start: u16, titles_end: u16) -> std::ops::Range<u16> {
    let button = columns(THEME_BUTTON);
    let end = background_start;
    let start = end.saturating_sub(button);
    if start < titles_end || end - start < button { start..start } else { start..end }
}

/// The arithmetic of the above, away from the app it reads those three numbers out of.
fn button_range(width: u16, titles_end: u16, badge: u16) -> std::ops::Range<u16> {
    let button = columns(BACKGROUND_BUTTON[0]);
    let end = width.saturating_sub(badge);
    let start = end.saturating_sub(button);
    if start < titles_end || end - start < button { start..start } else { start..end }
}

pub fn menu_dropdown_rect(menu: &MenuBar, lang: Lang, keymap: &Keymap, full: Rect) -> Rect {
    let ranges = menu_title_ranges(menu, lang);
    let (x, _) = ranges.get(menu.menu_index).copied().unwrap_or((0, 0));
    let items = &menu.defs[menu.menu_index].items;
    let label_width = items.iter().map(|i| i18n::t(lang, i.label_key).chars().count()).max().unwrap_or(0);
    // The right-hand column carries two kinds of thing: a shortcut, and — for an item that holds
    // a setting rather than doing something once — what that setting is right now. They share the
    // column because they are never both on the same row, and the width is the widest of either.
    let shortcut_width = items
        .iter()
        .map(|i| match i.shortcut {
            Some(sc) => keymap::shortcut_hint(lang, keymap, sc).chars().count(),
            None => menu::item_value_width(lang, i.action),
        })
        .max()
        .unwrap_or(0);
    let gap = if shortcut_width > 0 { 3 } else { 0 };
    let width = ((1 + label_width + gap + shortcut_width + 1) as u16).max(18);
    let separators = items.iter().filter(|i| i.new_group).count() as u16;
    let height = items.len() as u16 + separators + 2;
    // The menu it hangs from can be past the right edge of a narrow bar — the keys still reach
    // it, and a menu you can open but cannot read would be a poor answer to that. So the box is
    // pulled back onto the screen rather than anchored where its title would have been.
    let width = width.min(full.width.max(1));
    Rect {
        x: (full.x + x).min(full.right().saturating_sub(width)),
        y: 1,
        width,
        height: height.min(full.height.saturating_sub(1)),
    }
}

/// Where a context menu hangs: from its anchor, but pulled back so it never spills past the
/// right or bottom edge. Shared by the renderer and click handling so both agree on the rows.
pub fn context_menu_rect(menu: &ContextMenu, lang: Lang, keymap: &Keymap, full: Rect) -> Rect {
    let items = &menu.items;
    let label_width = items.iter().map(|i| i18n::t(lang, i.label_key).chars().count()).max().unwrap_or(0);
    let shortcut_width = items
        .iter()
        .filter_map(|i| i.shortcut)
        .map(|s| keymap::shortcut_hint(lang, keymap, s).chars().count())
        .max()
        .unwrap_or(0);
    let gap = if shortcut_width > 0 { 3 } else { 0 };
    let width = ((1 + label_width + gap + shortcut_width + 1) as u16).max(18).min(full.width.max(1));
    let separators = items.iter().filter(|i| i.new_group).count() as u16;
    let height = (items.len() as u16 + separators + 2).min(full.height.max(1));
    Rect {
        x: menu.anchor.0.min(full.width.saturating_sub(width)),
        y: menu.anchor.1.min(full.height.saturating_sub(height)),
        width,
        height,
    }
}

/// Where the run-target drop-down hangs: directly under the toolbar button of the pane it was
/// opened from. `None` when that button isn't on screen, in which case there is nothing to drop
/// down from. Shared by the renderer and by click handling, so both agree on the rows.
pub fn run_menu_rect(app: &App, editor_area: Rect, full: Rect) -> Option<Rect> {
    let menu = app.run_menu.as_ref()?;
    let panes = editor_pane_rects(editor_area, app.split_view, app.settings.split_pct);
    // A split closed while the menu was open leaves no right pane to hang from.
    let pane = panes.get(menu.pane.index()).or_else(|| panes.first()).copied()?;
    let (tab_bar, _, _) = pane_areas(app, app.pane_editor_index(menu.pane), pane);
    if tab_bar.height == 0 {
        return None;
    }
    let (target_range, _) = toolbar_button_ranges(app, tab_bar.width);
    let (start, _) = target_range?;

    let rows = app.run_menu_rows();
    let widest = rows
        .iter()
        .map(|r| r.label.chars().count() + r.detail.as_ref().map_or(0, |d| d.chars().count() + 3))
        .max()
        .unwrap_or(0);
    // Two for the border, two for the active marker.
    let width = ((widest + 4) as u16).clamp(20, full.width);
    let height = (rows.len() as u16 + 2).min(full.height.saturating_sub(tab_bar.y + 1));
    Some(Rect {
        // Prefer aligning with the button, but never hang off the right edge.
        x: (tab_bar.x + start).min(full.width.saturating_sub(width)),
        y: tab_bar.y + 1,
        width,
        height,
    })
}

/// The drawing the About box carries beside its text, in the two sizes it is kept in, held as a
/// bitmap rather than as characters: one letter per pixel, and two strings per screen row, since
/// a row is drawn as a half-block with one colour in front and another behind and so holds two
/// pixels stacked. That is what buys the drawing its resolution — the row grid is half as coarse
/// as the character grid — and it is why the letters name a colour rather than a glyph.
const ABOUT_ART_WIDE: &[&str] = &[
    "..............lll..............",
    ".............lllll.............",
    ".............lllll.............",
    ".......eel...lllll...lee.......",
    ".....ellll...rlllr...lllle.....",
    "...elllll..rrrsssrrr..llllle...",
    "..elllll.ersssssssssre.llllle..",
    ".llllle.rrssmmmmmmmssrr.elllll.",
    "ellle..ersssmmmmmmmsssre..ellle",
    ".ee....rsssssmmmmmsssssr....ee.",
    "......ermssssmmmmmssssmre......",
    "......rsmmmsssmmmsssmmmsr......",
    "......rsmmmmmsmmmsmmmmmsr......",
    "......rsmmmmmmmmmmmmmmmsr......",
    "......rsmmmmmmmmmmmmmmmsr......",
    "......rsmmmmmsmmmsmmmmmsr......",
    "......rsmmmsssmmmsssmmmsr......",
    "......ermssssmmmmmssssmre......",
    ".ee....rsssssmmmmmsssssr....ee.",
    "ellle..ersssmmmmmmmsssre..ellle",
    ".llllle.rrssmmmmmmmssrr.elllll.",
    "..elllll.ersssssssssre.llllle..",
    "...elllll..rrrsssrrr..llllle...",
    ".....ellll...errre...lllle.....",
    ".......eel....lll....lee.......",
    "..............lll..............",
    "..............ele..............",
    "...............................",
];

const ABOUT_ART_NARROW: &[&str] = &[
    "............eee............",
    "...........ellle...........",
    "...........lllll...........",
    ".......ee..lllll..ee.......",
    "....ellll..ellle..lllle....",
    "...lllll.errsssrre.lllll...",
    ".elllle.rsssssssssr.elllle.",
    "ellll..rssmmmmmmmssr..lllle",
    "llle..rssssmmmmmssssr..elll",
    ".....erssssmmmmmssssre.....",
    ".....rsmmsssmmmsssmmsr.....",
    ".....rsmmmmsmmmsmmmmsr.....",
    ".....rsmmmmmmmmmmmmmsr.....",
    ".....rsmmmmmmmmmmmmmsr.....",
    ".....rsmmmmsmmmsmmmmsr.....",
    ".....rsmmsssmmmsssmmsr.....",
    ".....erssssmmmmmssssre.....",
    "llle..rssssmmmmmssssr..elll",
    "ellll..rssmmmmmmmssr..lllle",
    ".elllle.rsssssssssr.elllle.",
    "...lllll.errsssrre.lllll...",
    "....ellll..errre..lllle....",
    ".......ee...lll...ee.......",
    "............lll............",
    ".............e.............",
    "...........................",
];

/// Columns given to the text beside the drawing. Wide enough for the two lines that cannot be
/// broken any narrower: the repository line, and the English close hint.
const ABOUT_TEXT_COLS: u16 = 36;

/// Columns of air between the drawing and the text.
const ABOUT_GUTTER: u16 = 3;

/// The drawing this terminal has room for, or `None` when it has room for neither and the box
/// falls back to the text on its own. The thresholds ask for a margin the modal does not use: a
/// box that reaches the edge of the screen reads as clipped even when every column of it is there.
fn about_art(full: Rect) -> Option<&'static [&'static str]> {
    if full.width >= 76 && full.height >= 18 {
        Some(ABOUT_ART_WIDE)
    } else if full.width >= 72 && full.height >= 17 {
        Some(ABOUT_ART_NARROW)
    } else {
        None
    }
}

fn about_art_width(art: &[&str]) -> u16 {
    art.iter().map(|row| row.chars().count() as u16).max().unwrap_or(0)
}

/// Screen rows the drawing takes: two pixel rows to each of them.
fn about_art_height(art: &[&str]) -> u16 {
    art.len() as u16 / 2
}

pub fn about_modal_rect(full: Rect) -> Rect {
    match about_art(full) {
        // The drawing, the gutter, the text and the two borders.
        Some(art) => centered_rect(
            about_art_width(art) + ABOUT_GUTTER + ABOUT_TEXT_COLS + 2,
            about_art_height(art) + 2,
            full,
        ),
        // Tall enough for the version, the wrapped tagline (three lines in Italian, the longer
        // of the two), the author and repository lines, and the close hint.
        None => centered_rect(60, 13, full),
    }
}

pub fn settings_modal_rect(app: &App, full: Rect) -> Rect {
    // Wide enough for the longest row it actually has to draw, rather than a number that was
    // right when it was written: the values are sentences in places ("the interpreter's own
    // windows") and the labels grow when a language server is mentioned, so a fixed 54 columns
    // put the value on top of the label in English and further over in Italian.
    let widest = app
        .settings
        .rows()
        .iter()
        .map(|r| r.label.chars().count() + r.value.chars().count())
        .max()
        .unwrap_or(0);
    // Two for the borders, two for the cursor's marker, and two between label and value.
    let width = (widest + 6).max(54) as u16;
    let height = settings::SETTINGS_COUNT as u16 + 2;
    centered_rect(width, height, full)
}

fn focused_border_style(pal: Palette, is_focused: bool, resizing: bool) -> Style {
    if is_focused {
        let color = if resizing { pal.resize_border } else { pal.accent };
        Style::default().fg(color)
    } else {
        Style::default().fg(pal.text_dim)
    }
}

const SPLASH_BANNER: &[&str] = &[
    r" ██████╗██╗     ███████╗███████╗ ██████╗ ██████╗ ██████╗ ███████╗",
    r"██╔════╝██║     ██╔════╝██╔════╝██╔════╝██╔═══██╗██╔══██╗██╔════╝",
    r"██║     ██║     █████╗  █████╗  ██║     ██║   ██║██║  ██║█████╗  ",
    r"██║     ██║     ██╔══╝  ██╔══╝  ██║     ██║   ██║██║  ██║██╔══╝  ",
    r"╚██████╗███████╗███████╗███████╗╚██████╗╚██████╔╝██████╔╝███████╗",
    r" ╚═════╝╚══════╝╚══════╝╚══════╝ ╚═════╝ ╚═════╝ ╚═════╝ ╚══════╝",
];

fn draw_splash(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    let lang = app.settings.lang;
    let mut lines: Vec<Line> = Vec::new();
    for row in SPLASH_BANNER {
        lines.push(Line::from(Span::styled(*row, Style::default().fg(pal.success))).alignment(ratatui::layout::Alignment::Center));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(i18n::t(lang, Key::SplashTagline)).alignment(ratatui::layout::Alignment::Center));
    lines.push(Line::from(""));
    lines.push(
        Line::from(format!("{} · v{}", i18n::t(lang, Key::SplashSubtitle), env!("CARGO_PKG_VERSION")))
            .alignment(ratatui::layout::Alignment::Center),
    );
    lines.push(
        Line::from(Span::styled("msavox 2026", Style::default().fg(pal.text_dim)))
            .alignment(ratatui::layout::Alignment::Center),
    );
    // Started with a workspace — `clee -w name`, or a resumed one — so say which, while the
    // splash is the only thing on screen and the shells behind it are still starting.
    if let Some(name) = app.active_workspace.as_deref() {
        lines.push(Line::from(""));
        lines.push(
            Line::from(vec![
                Span::styled(format!("{} ", i18n::t(lang, Key::WorkspaceBadge)), Style::default().fg(pal.text_dim)),
                Span::styled(name.to_string(), Style::default().fg(pal.success).add_modifier(Modifier::BOLD)),
            ])
            .alignment(ratatui::layout::Alignment::Center),
        );
    }
    lines.push(Line::from(""));
    lines.push(
        Line::from(Span::styled(i18n::t(lang, Key::SplashHint), Style::default().fg(pal.text_dim)))
            .alignment(ratatui::layout::Alignment::Center),
    );

    let content_height = lines.len() as u16;
    let top_pad = full.height.saturating_sub(content_height) / 2;
    let area = Rect {
        x: full.x,
        y: full.y + top_pad,
        width: full.width,
        height: content_height.min(full.height),
    };
    f.render_widget(Clear, full);
    f.render_widget(Paragraph::new(lines), area);
}

/// One frame, and then the background under it if the terminal's own is not to be trusted.
///
/// Split in two so that the painting cannot be skipped: the drawing below returns early for the
/// splash screen, and an unreadable splash is exactly as unreadable as an unreadable editor.
pub fn draw(f: &mut Frame, app: &mut App) {
    // A theme that brings its own surface has to paint it whether or not the user asked for an
    // opaque background: the setting is about a terminal whose colours you want to keep, and a
    // light theme has no colours of the terminal's left to keep.
    let pal = app.palette();
    let opaque = app.settings.opaque_background || app.theme.paints_its_own_background();
    draw_frame(f, app);
    // Last of all, once every widget has had its say about which cells it colours.
    if opaque {
        paint_background(f.buffer_mut(), pal);
    }
}

fn draw_frame(f: &mut Frame, app: &mut App) {
    let pal = app.palette();
    // Remembered for the key path, which opens pop-ups without being handed the layout.
    app.last_full = f.area();
    if app.show_splash {
        draw_splash(f, app, f.area());
        return;
    }

    let params = LayoutParams::from_app(app);
    let areas = compute_layout(f.area(), &params);

    if let Some(sidebar_area) = areas.sidebar {
        draw_file_tree(f, app, sidebar_area);
    }

    draw_editor(f, app, areas.editor);

    if let Some(term_areas) = areas.terminals.clone() {
        draw_terminals(f, app, &term_areas);
    }

    // A frame of the layout, so it is drawn with the frames and not over them: in pin mode the
    // drawer took its column out of the main area before anything else was placed, and nothing
    // it covers is anything else's.
    if let Some(drawer_area) = areas.drawer {
        draw_drawer(f, app, drawer_area);
    }

    // And the other mode: cells painted over frames that were never told anything happened, the
    // way the git panel goes over the editor. `Clear` first, because what is underneath is a
    // real editor and real panes still drawn at their full width — the whole saving is that
    // nothing under here was resized, so nothing under here sent a `SIGWINCH`.
    //
    // Above the panes and below every modal, which is the same place the completion popup and
    // the git panel sit: the drawer is a frame, and a box that takes the keyboard is entitled to
    // cover it.
    if let Some(overlay) = areas.drawer_overlay {
        f.render_widget(Clear, overlay);
        draw_drawer(f, app, overlay);
    }

    // The ribbon, on the column the drawer would be flush against — and only while the drawer is
    // away, which is what `drawer_ribbon_rect` asks. Drawn after both of the above rather than
    // before, so the one rule is visible in one place: what is on screen is the drawer or the way
    // back to it, never both.
    if let Some(ribbon) = drawer_ribbon_rect(&areas) {
        let engaged = app.drawer_ribbon_engaged(ribbon);
        draw_drawer_ribbon(f, pal, ribbon, engaged, RIBBON_OPEN_MARK);
    }

    // And its mirror: the handle on an open drawer's own left border, which is the way out for
    // the same hand. Drawn after the drawer in both modes, because it sits *on* that border —
    // the column it shares with the width seam, where a press that moves is a resize and a press
    // that does not is this. See `App::DragTarget::DrawerEdgePress`.
    if let Some(ribbon) = drawer_close_ribbon_rect(&areas) {
        let engaged = app.drawer_ribbon_engaged(ribbon);
        draw_drawer_ribbon(f, pal, ribbon, engaged, RIBBON_CLOSE_MARK);
    }

    draw_status(f, app, areas.status);
    draw_menu_bar(f, app, areas.menu_bar);

    // Above the panes, below every modal. The popup is not modal itself, so anything that does
    // take the keyboard is entitled to cover it — and covering it is also how it stops being
    // visible when a box opens over the buffer it belongs to.
    if let Some(popup) = app.completion.as_ref()
        && app.completion_live()
    {
        draw_completion(pal, f, popup, app.completion_anchor, f.area());
    }

    if app.show_settings {
        draw_settings_modal(f, app, f.area());
    }
    if app.menu.active {
        // While a menu is open, show the title bar even if it's normally hidden, so Ctrl+Shift+B
        // navigation (←/→ between menus) stays visible; drawn as a top-row overlay.
        if areas.menu_bar.height == 0 {
            let full = f.area();
            let bar = Rect { x: full.x, y: full.y, width: full.width, height: 1 };
            f.render_widget(Clear, bar);
            draw_menu_bar(f, app, bar);
        }
        draw_menu_dropdown(f, app, f.area());
    }
    if app.show_about {
        draw_about_modal(f, app, f.area());
    }
    if app.show_delete_confirm {
        draw_delete_confirm_modal(f, app, f.area());
    }
    if app.unsaved_prompt.is_some() {
        draw_unsaved_modal(f, app, f.area());
    }
    if app.show_rename {
        draw_rename_modal(f, app, f.area());
    }
    if app.symbol_rename.is_some() {
        draw_symbol_rename_modal(f, app, f.area());
    }
    if app.rename_preview.is_some() {
        draw_rename_preview(f, app, f.area());
    }
    if app.replace_sweep.is_some() {
        draw_replace_sweep(f, app, f.area());
    }
    if app.show_terminal_rename {
        draw_terminal_rename_modal(f, app, f.area());
    }
    if app.show_workspace_save {
        draw_workspace_save_modal(f, app, f.area());
    }
    if app.show_goto {
        draw_goto_modal(f, app, f.area());
    }
    if app.show_search {
        draw_search_modal(f, app, f.area());
    }
    if app.git_panel.is_some() {
        draw_git_panel(f, app, f.area());
    }
    if app.inspector.is_some() {
        draw_inspector(f, app, f.area());
    }
    if app.show_new_entry {
        draw_new_entry_modal(f, app, f.area());
    }
    if app.show_save_as {
        draw_save_as_modal(f, app, f.area());
    }
    // Drawn after the panes so it overlays the editor it hangs over.
    if app.run_menu.is_some() {
        draw_run_menu(f, app, areas.editor, f.area());
    }
    if app.theme_menu.is_some() {
        draw_theme_menu(f, app, f.area());
    }
    if app.venv_register.is_some() {
        draw_venv_register_modal(f, app, f.area());
    }
    if app.run_command_edit.is_some() {
        draw_run_command_modal(f, app, f.area());
    }
    if app.find.is_some() {
        draw_find_modal(f, app, f.area());
    }
    if app.picker.is_some() {
        draw_picker_modal(f, app, f.area());
    }
    // The manual covers the whole working area; nothing else is up while it reads.
    if app.manual.is_some() {
        draw_manual(f, app, f.area());
    }
    // Topmost: the context menu overlays whatever it was raised over.
    if app.context_menu.is_some() {
        draw_context_menu(f, app, f.area());
    }
}

/// Fills in every cell that would otherwise show the terminal through it.
///
/// Done as a pass over the finished frame rather than by painting a sheet underneath it, because
/// a sheet does not survive the frame: modals `Clear` the cells they cover, which resets them to
/// the terminal's background again, and each of those would become a translucent hole in the one
/// place — a dialog over a bright window — where the text most needs to be readable. Every cell
/// still saying "the terminal's own colour" at the end of the frame is one nothing else claimed.
///
/// Cells a picture is drawn over are left alone by the backend anyway (they are marked skipped),
/// so the graphics protocols are unaffected.
///
/// The same pass settles the text. Most of what the editor writes never states a foreground — it
/// is content, and content is whatever colour the terminal writes in. That is right until a theme
/// brings its own surface, at which point the terminal's idea of "text" can be the paper the
/// theme just painted. A cell still on `Reset` at the end of the frame is one nothing claimed,
/// here as much as for the background, so both are filled in the one walk.
fn paint_background(buffer: &mut ratatui::buffer::Buffer, pal: Palette) {
    for cell in buffer.content.iter_mut() {
        if cell.bg == Color::Reset {
            cell.bg = pal.background;
        }
        if cell.fg == Color::Reset && pal.text != Color::Reset {
            cell.fg = pal.text;
        }
    }
}

fn draw_menu_bar(f: &mut Frame, app: &App, area: Rect) {
    let pal = app.palette();
    // Hidden bar collapses to a zero-height row; nothing to paint (menus still reachable
    // via Ctrl+Shift+B, whose dropdown anchors to the top independently of this row).
    if area.height == 0 {
        return;
    }
    let lang = app.settings.lang;
    let mut spans = vec![Span::styled(MENU_LOGO, Style::default().bg(pal.bar))];
    let mut used = columns(MENU_LOGO);
    // Titles with no columns left to them are not drawn at all. The paragraph would have clipped
    // them anyway, but going through the same layout the button and the click use keeps `used`
    // honest about how much of the row was actually spent.
    let ranges = menu_titles_within(&app.menu, lang, area.width);
    for (i, def) in app.menu.defs.iter().enumerate() {
        if ranges.get(i).is_none_or(|(start, end)| start == end) {
            break;
        }
        let title = i18n::menu_title(lang, def.title_key);
        let label = format!(" {} ", title);
        used += columns(&label);
        let is_open = app.menu.active && app.menu.menu_index == i;
        let mut style = chrome(pal, if is_open {
            Style::default().fg(pal.on_accent).bg(pal.accent)
        } else {
            Style::default().fg(pal.on_bar).bg(pal.bar)
        });
        if i == 0 {
            style = style.add_modifier(Modifier::BOLD);
        }
        // The initial is underlined only while the bar is open, because that is the only time
        // pressing it does anything. Everywhere else an underlined initial means "Alt and this
        // letter", and CleeCode has no Alt+<letter> keys — macOS does not deliver them on
        // non-US layouts, so they were dropped. Advertising a key that does nothing is worse
        // than advertising none; showing it exactly when it works costs nothing, and the width
        // does not change either way, so the bar does not jump when a menu opens.
        let mut chars = title.chars();
        let mnemonic = chars.next().map(|c| c.to_string()).unwrap_or_default();
        let rest: String = chars.collect();
        let mut mnemonic_style =
            if app.menu.active { style.add_modifier(Modifier::UNDERLINED) } else { style };
        // Not on the open title: there the initial is already lit, and a second colour on top of
        // the highlight would be one distinction too many — and red on cyan is unreadable.
        if let Some(colour) = pal.accelerator.filter(|_| !is_open) {
            mnemonic_style = mnemonic_style.fg(colour);
        }
        spans.push(Span::styled(" ", style));
        spans.push(Span::styled(mnemonic, mnemonic_style));
        spans.push(Span::styled(format!("{} ", rest), style));
    }
    // The open workspace, right-aligned on the same row. Nothing on screen used to say which one
    // you were in — the name only appeared in the status line for a moment when it loaded, and
    // was gone by the time you wondered. Titles are drawn from the left and this from the right,
    // with the padding between them, so a long name eats blank space and never the menus.
    let workspace = workspace_badge(app);
    // The background button goes just inside it, and is the first thing to be given up when the
    // window is too narrow for all three.
    let button = menu_bar_button_range(app, area.width);
    let button_width = button.end - button.start;
    let themes = menu_bar_theme_range(app, area.width);
    let themes_width = themes.end - themes.start;
    let pad = area
        .width
        .saturating_sub(used)
        .saturating_sub(columns(&workspace))
        .saturating_sub(button_width)
        .saturating_sub(themes_width);
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad as usize), Style::default().bg(pal.bar)));
    }
    if themes_width > 0 {
        // Lit while its list is open, the same way an open menu title is lit.
        let style = if app.theme_menu.is_some() {
            Style::default().fg(pal.on_accent).bg(pal.accent)
        } else {
            Style::default().fg(pal.on_bar).bg(pal.bar)
        };
        spans.push(Span::styled(THEME_BUTTON, style));
    }
    if button_width > 0 {
        // What is actually on the screen, not what the setting says: with a theme that brings
        // its own surface the fill is on whatever the setting was left at, and a button showing
        // otherwise would be reporting a state the frame does not have.
        let on = app.settings.opaque_background || app.theme.paints_its_own_background();
        // Lit like the open menu title when it is on, so "something has been switched on here"
        // reads the same way everywhere on this row.
        let style = if on {
            Style::default().fg(pal.on_accent).bg(pal.accent)
        } else {
            Style::default().fg(pal.on_bar).bg(pal.bar)
        };
        spans.push(Span::styled(BACKGROUND_BUTTON[usize::from(on)], style));
    }
    if !workspace.is_empty() {
        spans.push(Span::styled(workspace, Style::default().fg(pal.on_accent).bg(pal.success)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_menu_dropdown(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    let lang = app.settings.lang;
    let rect = menu_dropdown_rect(&app.menu, lang, &app.keymap, full);
    let inner_width = rect.width.saturating_sub(2) as usize;
    // Separator rules are woven in between real items, so the row a given item
    // renders on drifts down by one for every group opened above it. Track the
    // selected item's display row so the highlight lands on the right line.
    let separator = ListItem::new(Line::from(Span::styled(
        "─".repeat(inner_width),
        Style::default().fg(pal.text_dim),
    )));
    let mut items: Vec<ListItem> = Vec::new();
    let mut selected_row = 0;
    let states = app.menu_states();
    for (idx, i) in app.menu.defs[app.menu.menu_index].items.iter().enumerate() {
        if i.new_group {
            items.push(separator.clone());
        }
        if idx == app.menu.item_index {
            selected_row = items.len();
        }
        let label = i18n::t(lang, i.label_key);
        // A shortcut if it has one, otherwise what it currently is — see `menu::item_value`.
        // Nothing has both: a row that carries a setting is a row you have to open the menu to
        // read, which is the whole reason its value is drawn here.
        let right = match i.shortcut {
            Some(sc) => Some(keymap::shortcut_hint(lang, &app.keymap, sc)),
            None => menu::item_value(lang, i.action, states).map(str::to_string),
        };
        let tail = match right {
            Some(sc) => {
                let content_width = inner_width.saturating_sub(2);
                let pad = content_width.saturating_sub(label.chars().count() + sc.chars().count()).max(1);
                format!("{}{}{} ", label, " ".repeat(pad), sc)
            }
            None => format!("{} ", label),
        };
        items.push(ListItem::new(accelerated_line(pal, &tail)));
    }
    let mut state = ListState::default();
    state.select(Some(selected_row));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent));
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    f.render_widget(Clear, rect);
    f.render_stateful_widget(list, rect, &mut state);
}

/// Adds the theme's chrome weight to a style. One place, so the bar, the tabs and the status
/// line cannot drift apart on the question of how heavy the frame is.
fn chrome(pal: Palette, style: Style) -> Style {
    if pal.bold_chrome { style.add_modifier(Modifier::BOLD) } else { style }
}

/// A drop-down row with its initial in the accelerator colour, for the themes that have one.
///
/// `tail` is the row without its leading space, because the space is not the initial and putting
/// it in the coloured span would paint a red block where the letter is not.
fn accelerated_line(pal: Palette, tail: &str) -> Line<'static> {
    let Some(colour) = pal.accelerator else {
        return Line::from(format!(" {tail}"));
    };
    let mut chars = tail.chars();
    let Some(initial) = chars.next() else { return Line::from(" ".to_string()) };
    Line::from(vec![
        Span::raw(" "),
        Span::styled(initial.to_string(), Style::default().fg(colour)),
        Span::raw(chars.collect::<String>()),
    ])
}

/// A caption over a group: dim and italic, so it reads as a label for the rows under it rather
/// than as a row that has been greyed out because it cannot be chosen.
fn menu_header(pal: Palette, label: &str, width: usize) -> ListItem<'static> {
    ListItem::new(Line::from(Span::styled(
        format!(" {label:<width$}"),
        Style::default().fg(pal.text_dim).add_modifier(Modifier::ITALIC),
    )))
}

fn draw_context_menu(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    let lang = app.settings.lang;
    let Some(menu) = app.context_menu.as_ref() else { return };
    let rect = context_menu_rect(menu, lang, &app.keymap, full);
    let inner_width = rect.width.saturating_sub(2) as usize;
    // Same separator-aware layout as the menu bar's drop-down: rules between groups shift the
    // selected item's row down, so track where the highlight should land.
    let separator = ListItem::new(Line::from(Span::styled(
        "─".repeat(inner_width),
        Style::default().fg(pal.text_dim),
    )));
    let mut items: Vec<ListItem> = Vec::new();
    let mut selected_row = 0;
    for (idx, i) in menu.items.iter().enumerate() {
        if i.new_group {
            items.push(separator.clone());
        }
        if idx == menu.selected {
            selected_row = items.len();
        }
        let label = i18n::t(lang, i.label_key);
        if i.header {
            items.push(menu_header(pal, label, inner_width.saturating_sub(1)));
            continue;
        }
        let line = match i.shortcut.map(|sc| keymap::shortcut_hint(lang, &app.keymap, sc)) {
            Some(sc) => {
                let content_width = inner_width.saturating_sub(2);
                let pad = content_width.saturating_sub(label.chars().count() + sc.chars().count()).max(1);
                format!(" {}{}{} ", label, " ".repeat(pad), sc)
            }
            None => format!(" {} ", label),
        };
        items.push(ListItem::new(line));
    }
    let mut state = ListState::default();
    state.select(Some(selected_row));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent));
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    f.render_widget(Clear, rect);
    f.render_stateful_widget(list, rect, &mut state);
}

fn draw_settings_modal(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    let rect = settings_modal_rect(app, full);
    f.render_widget(Clear, rect);
    let rows = app.settings.rows();
    let inner_width = rect.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let marker = if i == app.settings_selected { "> " } else { "  " };
            // The value against the right-hand edge rather than at a fixed column, so a label
            // longer than the column — "Language server (diagnostics, completion)" was — pushes
            // its value along instead of being run into by it.
            let pad = inner_width
                .saturating_sub(marker.chars().count() + r.label.chars().count() + r.value.chars().count())
                .max(1);
            ListItem::new(Line::from(format!("{marker}{}{}{}", r.label, " ".repeat(pad), r.value)))
        })
        .collect();
    let mut state = ListState::default();
    state.select(Some(app.settings_selected));
    let block = Block::default()
        .title(format!(" {} ", i18n::t(app.settings.lang, Key::SettingsTitle)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent));
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    f.render_stateful_widget(list, rect, &mut state);
}

/// Breaks `text` at spaces so that no line is wider than `width` columns. A word with no room of
/// its own is left to overflow rather than cut in half: the only word here long enough to manage
/// that is the repository address, which is easier to read hanging over the edge than in pieces.
fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match out.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => out.push(word.to_string()),
        }
    }
    out
}

/// Everything the About box says, wrapped to `width`. Wrapped here rather than left to `Wrap`
/// because the block is centred against the drawing beside it, and centring needs the line count.
fn about_text_lines(lang: Lang, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(
            format!("CleeCode v{}", env!("CARGO_PKG_VERSION")),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    lines.extend(wrap_words(i18n::t(lang, Key::AboutTagline), width).into_iter().map(Line::from));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        i18n::t(lang, Key::AboutAuthor),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        format!("{}  ·  MIT", i18n::t(lang, Key::AboutRepo)),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
    lines.extend(
        wrap_words(i18n::t(lang, Key::AboutCloseHint), width)
            .into_iter()
            .map(|row| Line::from(Span::styled(row, Style::default().fg(Color::DarkGray)))),
    );
    lines
}

/// The colours a pixel of the drawing can be, stated outright rather than taken from `Color::Red`
/// and `Color::Green`: those are palette entries every terminal theme is free to redefine, and the
/// themes that make their red a salmon turned the mark in the middle pink. The darkest green is
/// the one the outline is feathered with, which is what keeps the curves from reading as steps.
fn about_ink(pixel: char) -> Option<Color> {
    Some(match pixel {
        's' => Color::Rgb(94, 148, 82),
        'l' => Color::Rgb(74, 118, 66),
        'r' => Color::Rgb(58, 96, 52),
        'e' => Color::Rgb(42, 70, 38),
        'm' => Color::Rgb(196, 26, 34),
        _ => return None,
    })
}

/// Draws the bitmap into `area`, a row of it per screen row: the upper half-block takes the top
/// pixel as its foreground and the bottom one as its background, so both are drawn in full colour
/// in the one cell they share. A row with a pixel on one side only is drawn as the half that has
/// it, leaving whatever is behind the modal to show through the other half.
fn draw_about_art(f: &mut Frame, art: &[&str], area: Rect) {
    for row in 0..about_art_height(art).min(area.height) {
        let (top, bottom) = (art[row as usize * 2], art[row as usize * 2 + 1]);
        for (col, (over, under)) in top.chars().zip(bottom.chars()).enumerate() {
            let col = col as u16;
            if col >= area.width {
                break;
            }
            let Some(cell) = f.buffer_mut().cell_mut((area.x + col, area.y + row)) else {
                continue;
            };
            match (about_ink(over), about_ink(under)) {
                (None, None) => {}
                (Some(fg), None) => {
                    cell.set_symbol("\u{2580}").set_fg(fg);
                }
                (None, Some(fg)) => {
                    cell.set_symbol("\u{2584}").set_fg(fg);
                }
                (Some(fg), Some(bg)) => {
                    cell.set_symbol("\u{2580}").set_fg(fg).set_bg(bg);
                }
            }
        }
    }
}

fn draw_about_modal(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    let rect = about_modal_rect(full);
    f.render_widget(Clear, rect);
    let lang = app.settings.lang;
    let block = Block::default()
        .title(format!(" {} ", i18n::t(lang, Key::AboutTitle)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let Some(art) = about_art(full) else {
        let lines = about_text_lines(lang, inner.width as usize);
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        return;
    };

    let art_width = about_art_width(art);
    let text_width = inner.width.saturating_sub(art_width + ABOUT_GUTTER);
    let lines = about_text_lines(lang, text_width as usize);
    // The text is shorter than the drawing, so it sits against the middle of it rather than the
    // top. A translation long enough to fill the height simply starts at the top instead.
    let top = inner.height.saturating_sub(lines.len() as u16) / 2;

    f.render_widget(
        Paragraph::new(lines),
        Rect {
            x: inner.x + art_width + ABOUT_GUTTER,
            y: inner.y + top,
            width: text_width,
            height: inner.height.saturating_sub(top),
        },
    );
    draw_about_art(
        f,
        art,
        Rect { x: inner.x, y: inner.y, width: art_width, height: inner.height },
    );
}

pub fn delete_confirm_modal_rect(full: Rect) -> Rect {
    centered_rect(60, 5, full)
}

fn draw_delete_confirm_modal(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    let rect = delete_confirm_modal_rect(full);
    f.render_widget(Clear, rect);
    let name = app
        .delete_target
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let block = Block::default()
        .title(format!(" {} ", i18n::t(app.settings.lang, Key::ModalDelete)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.danger));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let text = i18n::msg_confirm_delete(app.settings.lang, &name);
    f.render_widget(Paragraph::new(Line::from(text)).wrap(Wrap { trim: false }), inner);
}

pub fn unsaved_modal_rect(full: Rect) -> Rect {
    centered_rect(64, 6, full)
}

fn draw_unsaved_modal(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    use crate::app::UnsavedPrompt;
    let rect = unsaved_modal_rect(full);
    f.render_widget(Clear, rect);
    let lang = app.settings.lang;
    let count = app.editors.iter().filter(|e| e.dirty).count();
    let detail = match app.unsaved_prompt {
        Some(UnsavedPrompt::CloseTab(idx)) => {
            app.editors.get(idx).map(|e| e.title(lang)).unwrap_or_default()
        }
        _ => i18n::msg_unsaved_count(lang, count),
    };
    let block = Block::default()
        .title(format!(" {} ", i18n::t(lang, Key::ModalUnsaved)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.warning));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let lines = vec![
        Line::from(i18n::msg_unsaved_question(lang, &detail)),
        Line::from(Span::styled(i18n::msg_unsaved_choices(lang), Style::default().fg(pal.text_muted))),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

pub fn rename_modal_rect(full: Rect) -> Rect {
    centered_rect(60, 6, full)
}

fn draw_rename_modal(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    let rect = rename_modal_rect(full);
    f.render_widget(Clear, rect);
    let old_name = app
        .rename_target
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let block = Block::default()
        .title(format!(" {} ", i18n::t(app.settings.lang, Key::ModalRename)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let prompt = i18n::msg_rename_prompt(app.settings.lang, &old_name);
    let lines = vec![Line::from(prompt), Line::from(Span::styled(app.rename_input.clone(), Style::default().fg(pal.warning)))];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    // Clamped to the box: a name longer than the modal is wide would otherwise park the caret
    // on whatever is drawn beside it, which reads as the cursor having escaped.
    let cursor_x = (inner.x + app.rename_input.chars().count() as u16)
        .min(inner.right().saturating_sub(1));
    f.set_cursor_position((cursor_x, inner.y + 1));
}

/// The box that asks what to call the name under the cursor instead.
///
/// The same shape as the file rename above, deliberately: one line of question, one of answer,
/// and the caret where the typing goes. What is being renamed is a different kind of thing, but
/// the gesture is the same one and there is nothing to be gained by its looking different.
fn draw_symbol_rename_modal(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    let lang = app.settings.lang;
    let Some(box_) = app.symbol_rename.as_ref() else { return };
    let rect = rename_modal_rect(full);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .title(format!(" {} ", i18n::t(lang, Key::ItemRenameSymbol)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let lines = vec![
        Line::from(i18n::msg_rename_symbol_prompt(lang, &box_.old_name)),
        Line::from(Span::styled(box_.typed.clone(), Style::default().fg(pal.warning))),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    // Clamped to the box, for the reason every other input here is: a name longer than the modal
    // is wide would park the caret on whatever is drawn beside it.
    let cursor_x =
        (inner.x + box_.typed.chars().count() as u16).min(inner.right().saturating_sub(1));
    f.set_cursor_position((cursor_x, inner.y + 1));
}

/// What a rename would change, before any of it is a buffer.
///
/// On the git panel's frame, and that is the argument for the whole design: this is a diff being
/// read before it is agreed to, which is what that panel is for, and a reader who has read one
/// has read this. The rows are diff-shaped so [`diff_span`] colours them without being told
/// anything new, and the footer spells its keys for the reason the git footer spells its own —
/// they are bare letters, safe only while the box owns the keyboard, and discoverable only if the
/// box says so.
fn draw_rename_preview(f: &mut Frame, app: &mut App, full: Rect) {
    let pal = app.palette();
    let lang = app.settings.lang;
    let Some(preview) = app.rename_preview.as_ref() else { return };
    let title = i18n::msg_rename_preview_title(
        lang,
        &preview.old_name,
        &preview.new_name,
        preview.edits,
        preview.files.len(),
    );
    let rows = draw_edit_preview(f, pal, lang, full, &title, &preview.rows, preview.scroll);
    // Told back to the preview because paging needs the height and the renderer is the only
    // thing that knows it — the same arrangement as the git panel's. Written after the draw
    // rather than before it, which changes nothing: it is read by the next key press.
    if let (Some(preview), Some(rows)) = (app.rename_preview.as_mut(), rows) {
        preview.body_rows = rows;
    }
}

/// The same box for the sweep across the project, because it is the same promise: what would
/// change, grouped by file, read before anything is written. The one difference is what the
/// title says — a query becoming a replacement rather than a name becoming a name.
fn draw_replace_sweep(f: &mut Frame, app: &mut App, full: Rect) {
    let pal = app.palette();
    let lang = app.settings.lang;
    let Some(sweep) = app.replace_sweep.as_ref() else { return };
    let title = i18n::msg_replace_preview_title(
        lang,
        &sweep.query,
        &sweep.replacement,
        sweep.edits,
        sweep.files.len(),
    );
    let rows = draw_edit_preview(f, pal, lang, full, &title, &sweep.rows, sweep.scroll);
    if let (Some(sweep), Some(rows)) = (app.replace_sweep.as_mut(), rows) {
        sweep.body_rows = rows;
    }
}

/// Draws one of the two previews of edits that have not happened yet, and answers how many rows
/// its body had room for.
///
/// One function for both because they are one thing on screen: the same frame, the same
/// diff-shaped rows, the same footer, the same keys. Two copies of this would be two boxes that
/// agree today and drift apart the first time one of them is adjusted.
///
/// `None` when the frame was too short to draw anything, which is also the signal that the row
/// count it returned would be a lie.
fn draw_edit_preview(
    f: &mut Frame,
    pal: Palette,
    lang: Lang,
    full: Rect,
    title: &str,
    rows: &[String],
    scroll: usize,
) -> Option<usize> {
    let rect = git_panel_rect(full);
    let block = Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent));
    let inner = block.inner(rect);
    if inner.height < 2 {
        return None;
    }
    // One row kept back for the keys, the way the git panel keeps one back for its own.
    let body = Rect { height: inner.height.saturating_sub(1), ..inner };
    let height = body.height as usize;

    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    let lines: Vec<Line> = rows
        .iter()
        .skip(scroll)
        .take(height)
        .map(|row| Line::from(diff_span(pal, row)))
        .collect();
    f.render_widget(Paragraph::new(lines), body);
    let keys = Rect { y: inner.bottom().saturating_sub(1), height: 1, ..inner };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            i18n::msg_preview_keys(lang),
            Style::default().fg(pal.text_dim),
        ))),
        keys,
    );
    Some(height)
}

/// Simple single-line input modal shared by Go-to-line and New file/folder.
fn draw_input_modal(pal: Palette, f: &mut Frame, full: Rect, title: &str, prompt: &str, input: &str) {
    let rect = centered_rect(60, 6, full);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let lines = vec![
        Line::from(prompt.to_string()),
        Line::from(Span::styled(input.to_string(), Style::default().fg(pal.warning))),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    // Clamped to the box for the same reason every other input here is: typing past the right
    // edge must stop the caret at the edge rather than walk it out of the frame.
    let cursor_x = (inner.x + input.chars().count() as u16).min(inner.right().saturating_sub(1));
    f.set_cursor_position((cursor_x, inner.y + 1));
}

fn draw_goto_modal(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    let lang = app.settings.lang;
    // What the number will mean depends on what is being looked at.
    let pages = app.editor().preview.as_ref().is_some_and(|p| p.pages.is_some());
    draw_input_modal(pal, 
        f,
        full,
        i18n::goto_title(lang, pages),
        i18n::msg_goto_prompt(lang, pages),
        &app.goto_input,
    );
}

/// Where the git panel sits: nearly the whole window, since a diff is wide and a line that wraps
/// is a line that has to be read twice. Shared with the click handling so "outside the panel"
/// means the same thing to both.
pub fn git_panel_rect(full: Rect) -> Rect {
    // Floors for the same reason the manual has them: on a window too small to hold the panel,
    // a box asked for zero rows is a box with nothing in it, and `centered_rect` clamps what it
    // is given to what there is.
    let width = full.width.saturating_sub(8).min(120).max(24);
    let height = full.height.saturating_sub(4).max(8);
    centered_rect(width, height, full)
}

/// The read-only git panel: a diff, a log and a branch list, one tab at a time.
///
/// A modal reader rather than a docked frame. The frames are the editor, the tree and the
/// shells — the three things you work *in* — and this is something you look at and dismiss, so
/// it costs no layout and takes nothing away from them while it is closed.
/// One tab of the git panel's header row: which tab it is, what it says, and the cells it takes.
pub struct GitTabSlot {
    pub tab: crate::app::GitTab,
    pub label: String,
    /// Offset from the left of the panel's inner area, in cells.
    pub x: u16,
    pub width: u16,
}

/// Where the git panel's three tabs sit on their row.
///
/// One function, used by the drawing and by the click. This is the same reason
/// `tab_strip_layout` exists: a hit-test that works the layout out its own way is a hit-test
/// that will one day disagree with what is on the screen, and nothing says so — the click just
/// lands on the wrong tab, or on none.
pub fn git_tab_slots(lang: i18n::Lang) -> Vec<GitTabSlot> {
    let mut x = 0u16;
    crate::app::GitTab::ALL
        .iter()
        .map(|&tab| {
            let label = i18n::msg_git_tab(lang, tab).to_string();
            // A space either side of the label, so the lit tab reads as a block rather than as
            // coloured text, and one more between tabs.
            let width = label.chars().count() as u16 + 2;
            let slot = GitTabSlot { tab, label, x, width };
            x += width + 1;
            slot
        })
        .collect()
}

/// The tab a click at `col` lands on, or `None` for the gaps between them.
pub fn git_tab_at(lang: i18n::Lang, header: Rect, col: u16) -> Option<crate::app::GitTab> {
    git_tab_slots(lang).into_iter().find_map(|slot| {
        let x = header.x + slot.x;
        (col >= x && col < x + slot.width && x + slot.width <= header.right()).then_some(slot.tab)
    })
}

/// One variable, a screenful at a time.
///
/// Read-only, and the numbers came from the session rather than from anything CleeCode worked
/// out: what is on screen is what the interpreter says it holds, at the moment it was asked.
fn draw_inspector(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    let Some(inspector) = app.inspector.as_ref() else { return };
    let lang = app.settings.lang;
    let rect = git_panel_rect(full);
    f.render_widget(Clear, rect);
    let slice = inspector.watch.slice.as_ref().filter(|s| s.name == inspector.name);
    let title = match slice {
        Some(s) if s.rows > 0 => format!(
            " {}  {}x{}  ({},{}) ",
            inspector.name, s.rows, s.cols, s.r0.max(1), s.c0.max(1)
        ),
        _ => format!(" {} ", inspector.name),
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    if inner.height < 2 {
        return;
    }

    let dim = Style::default().fg(pal.text_dim);
    let mut lines: Vec<Line> = Vec::new();
    match slice {
        None => lines.push(Line::from(Span::styled(i18n::msg_inspect_waiting(lang), dim))),
        Some(slice) if !slice.error.is_empty() => {
            lines.push(Line::from(Span::styled(slice.error.clone(), Style::default().fg(pal.danger))));
        }
        Some(slice) if slice.text => {
            for line in slice.lines() {
                lines.push(Line::from(Span::raw(line)));
            }
        }
        Some(slice) => {
            let grid = slice.grid();
            let width = inner.width as usize;
            // The row number takes the left edge, so a screenful of a big matrix still says
            // which part of it you are looking at.
            let gutter = format!("{}", slice.r0 + grid.len()).len().max(3) + 1;
            let cell = 11usize;
            let per_row = ((width.saturating_sub(gutter)) / cell).max(1);
            let mut header = format!("{:>w$}", "", w = gutter);
            for c in 0..per_row.min(slice.cols.saturating_sub(slice.c0.saturating_sub(1))) {
                header.push_str(&format!("{:>cell$}", slice.c0.max(1) + c, cell = cell));
            }
            lines.push(Line::from(Span::styled(header, dim)));
            for (r, row) in grid.iter().enumerate() {
                let mut text = format!("{:>w$}", slice.r0.max(1) + r, w = gutter);
                for value in row.iter().take(per_row) {
                    text.push_str(&format!("{:>cell$}", crate::wsview::cell_number(*value), cell = cell));
                }
                lines.push(Line::from(Span::raw(text)));
            }
        }
    }
    let body = Rect { height: inner.height.saturating_sub(1), ..inner };
    f.render_widget(Paragraph::new(lines), body);
    let hint = Rect { y: inner.y + inner.height - 1, height: 1, ..inner };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(i18n::msg_inspect_hint(lang), dim))),
        hint,
    );
}

/// How the git panel's interior is shared out: the rows the list gets, and whether git's last
/// word gets a row above the key line.
///
/// The bottom of the panel says what git said and which keys this tab has. Both come out of the
/// body's height rather than being drawn over it: a list that runs under its own footer is a
/// list whose last row is a lie. The notice was left out of that subtraction, so at exactly
/// three rows — tabs, one row of list, keys — it painted over the only row the list had.
///
/// Below four rows the notice is dropped rather than the list, because the list is what the
/// panel is for and the notice is repeated on the status line anyway.
fn git_body_layout(inner: Rect, has_notice: bool) -> (Rect, bool) {
    let show_notice = has_notice && inner.height >= 4;
    let body = Rect {
        y: inner.y + 1,
        height: inner.height.saturating_sub(2 + u16::from(show_notice)).max(1),
        ..inner
    };
    (body, show_notice)
}

fn draw_git_panel(f: &mut Frame, app: &mut App, full: Rect) {
    let pal = app.palette();
    use crate::app::{GitPrompt, GitTab};
    let lang = app.settings.lang;
    let rect = git_panel_rect(full);
    let Some(panel) = app.git_panel.as_ref() else { return };

    let block = Block::default()
        .title(format!(" {} ", i18n::msg_git_panel_title(lang)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent));
    let inner = block.inner(rect);
    if inner.height < 3 {
        return;
    }

    let (body, show_notice) = git_body_layout(inner, panel.notice.is_some());
    let rows = body.height as usize;
    // Told back to the panel before anything is drawn, because keeping a cursor on screen needs
    // the height and the renderer is the only thing that knows it. The same arrangement as the
    // completion popup's anchor, and for the same reason.
    if let Some(panel) = app.git_panel.as_mut() {
        panel.body_rows = rows;
    }
    let app: &App = app;
    let Some(panel) = app.git_panel.as_ref() else { return };

    f.render_widget(Clear, rect);
    f.render_widget(block, rect);

    // Tabs on their own row, the current one lit. Which tab you are on is the one thing that
    // must never be in doubt, since all four are lists of lines in the same box.
    //
    // Each drawn into the rectangle `git_tab_slots` gives it, rather than laid out again here:
    // the click asks the same function where the tabs are, and one function cannot disagree
    // with itself.
    let header = Rect { height: 1, ..inner };
    for slot in git_tab_slots(lang) {
        let x = header.x + slot.x;
        if x + slot.width > header.right() {
            break;
        }
        let style = if slot.tab == panel.tab {
            Style::default().fg(pal.on_accent).bg(pal.accent)
        } else {
            Style::default().fg(pal.text_dim)
        };
        let area = Rect { x, y: header.y, width: slot.width, height: 1 };
        f.render_widget(Paragraph::new(Span::styled(format!(" {} ", slot.label), style)), area);
    }

    draw_git_footer(pal, f, panel, lang, inner, show_notice);

    let Some(snap) = panel.snap.as_ref() else {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(i18n::msg_git_loading(lang), Style::default().fg(pal.text_dim)))),
            body,
        );
        return;
    };
    if let Some(error) = &snap.error {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(error.clone(), Style::default().fg(pal.danger)))),
            body,
        );
        return;
    }

    let width = body.width as usize;
    let lines: Vec<Line> = match panel.tab {
        GitTab::Status => {
            if snap.changes.is_empty() {
                vec![Line::from(Span::styled(
                    i18n::msg_git_clean(lang),
                    Style::default().fg(pal.text_dim),
                ))]
            } else {
                snap.changes
                    .iter()
                    .enumerate()
                    .skip(panel.scroll)
                    .take(rows)
                    .map(|(row, change)| status_line(pal, change, row == panel.selected, width))
                    .collect()
            }
        }
        GitTab::Diff => {
            if snap.diff.is_empty() {
                let of = snap.diff_of.as_ref().map(|p| p.display().to_string());
                vec![Line::from(Span::styled(
                    i18n::msg_git_no_changes(lang, of.as_deref()),
                    Style::default().fg(pal.text_dim),
                ))]
            } else {
                snap.diff.iter().skip(panel.scroll).take(rows).map(|l| Line::from(diff_span(pal, l))).collect()
            }
        }
        GitTab::Graph => {
            if panel.rows.is_empty() {
                vec![Line::from(Span::styled(
                    i18n::msg_git_no_commits(lang),
                    Style::default().fg(pal.text_dim),
                ))]
            } else {
                // One column for the art, the same on every row. A graph whose text starts in a
                // different place on each line is a graph nobody reads down.
                let art = crate::git_graph::width(&panel.rows);
                panel
                    .rows
                    .iter()
                    .enumerate()
                    .skip(panel.scroll)
                    .take(rows)
                    .map(|(row, r)| graph_line(pal, r, &snap.graph, art, row == panel.selected, width))
                    .collect()
            }
        }
        GitTab::Branches => snap
            .branches
            .iter()
            .enumerate()
            .skip(panel.scroll)
            .take(rows)
            .map(|(row, b)| branch_line(pal, b, row == panel.selected, width))
            .collect(),
        GitTab::Stashes => {
            if snap.stashes.is_empty() {
                vec![Line::from(Span::styled(
                    i18n::msg_git_no_stashes(lang),
                    Style::default().fg(pal.text_dim),
                ))]
            } else {
                snap.stashes
                    .iter()
                    .enumerate()
                    .skip(panel.scroll)
                    .take(rows)
                    .map(|(row, st)| stash_line(pal, st, row == panel.selected, width))
                    .collect()
            }
        }
    };
    f.render_widget(Paragraph::new(lines), body);

    // A command that stopped half-way is said over the top of whatever tab you are on, because
    // it is true of the repository rather than of the list: every action below behaves
    // differently until it is finished or put back.
    if let Some(unfinished) = snap.unfinished {
        let area = Rect { y: body.y, height: 1, ..body };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                i18n::msg_git_unfinished(lang, unfinished),
                Style::default().fg(pal.on_accent).bg(pal.warning),
            ))),
            area,
        );
    }

    if let Some(detail) = panel.detail.as_ref() {
        draw_git_detail(pal, f, detail, lang, rect);
    }

    if let Some(prompt) = panel.prompt.as_ref() {
        match prompt {
            GitPrompt::Text { kind, typed } => {
                draw_git_question(
                    f,
                    rect,
                    &i18n::msg_git_text_prompt(lang, kind, panel.staged_count()),
                    &format!("{typed}▏"),
                    pal.accent,
                );
            }
            GitPrompt::Confirm(confirm) => {
                // Red only where saying yes destroys something that is in no commit, no stash
                // and no reflog. Deleting a branch asks in the same shape and not in the same
                // colour, because red on every question is red on none of them.
                let colour = if confirm.destroys_work() { pal.danger } else { pal.warning };
                draw_git_question(f, rect, &i18n::msg_git_confirm_prompt(lang, confirm), "", colour);
            }
        }
    }
}

/// The colours the lanes are drawn in, and the reason the graph is readable at all.
///
/// Six, and none of them the cyan the panel highlights a row with: a lane in the colour of the
/// cursor is a lane that looks selected on every row. They repeat past six, which is honest —
/// two lanes in one colour is a graph with more branches than a screen has room to tell apart,
/// and the lines themselves still say which is which.
fn lane_colour(pal: Palette, lane: usize) -> Color {
    let lanes = [pal.success, pal.special, pal.warning, pal.info, pal.danger, pal.graph_extra];
    lanes[lane % lanes.len()]
}

/// One row of the graph: the drawing, then — if the row is a commit rather than the lines
/// between two — its hash, what points at it, and what it says.
fn graph_line(pal: Palette, 
    row: &crate::git_graph::Row,
    commits: &[crate::git::GraphCommit],
    art_width: usize,
    picked: bool,
    width: usize,
) -> Line<'static> {
    let art: String = row.art();
    let commit = row.commit.and_then(|at| commits.get(at));
    let Some(commit) = commit else {
        // A row of pure lines. Each character keeps its own lane's colour, which is what makes a
        // diagonal readable as *which* branch is joining.
        return Line::from(
            row.glyphs
                .iter()
                .map(|g| {
                    Span::styled(g.ch.to_string(), Style::default().fg(lane_colour(pal, g.lane)))
                })
                .collect::<Vec<_>>(),
        );
    };

    let refs = refs_text(&commit.refs);
    let tail = format!("  — {}, {}", commit.author, commit.when);
    if picked {
        // The whole row in one span, as everywhere else in this panel — the art included, so
        // the shape stays readable under the cursor even though its colours do not.
        let text = format!("{art:<art_width$} {:<9}{refs}{} {}", commit.hash, commit.subject, tail);
        let padded = format!("{text:<width$}");
        return Line::from(Span::styled(padded, Style::default().fg(pal.on_accent).bg(pal.accent)));
    }

    let mut spans: Vec<Span<'static>> = row
        .glyphs
        .iter()
        .map(|g| Span::styled(g.ch.to_string(), Style::default().fg(lane_colour(pal, g.lane))))
        .collect();
    spans.push(Span::raw(" ".repeat(art_width.saturating_sub(art.chars().count()) + 1)));
    spans.push(Span::styled(format!("{:<9}", commit.hash), Style::default().fg(pal.text_dim)));
    for name in &commit.refs {
        spans.push(Span::styled(format!("{} ", ref_label(name)), ref_style(pal, name.kind)));
    }
    spans.push(Span::raw(commit.subject.clone()));
    spans.push(Span::styled(tail, Style::default().fg(pal.text_dim)));
    Line::from(spans)
}

/// What a ref is drawn as. The kinds git's own log colours apart are coloured apart here for the
/// same reason: `main` and `origin/main` on different commits is the whole of "have I pushed
/// this", and in one colour it is two words that look alike.
fn ref_style(pal: Palette, kind: crate::git::RefKind) -> Style {
    use crate::git::RefKind::*;
    match kind {
        Head => Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        Local => Style::default().fg(pal.success),
        Remote => Style::default().fg(pal.danger),
        Tag => Style::default().fg(pal.warning),
    }
}

fn ref_label(name: &crate::git::RefName) -> String {
    match name.kind {
        // The branch that is checked out says so. Which commit you are standing on is the
        // question the graph is opened to answer, and a label identical to every other branch's
        // does not answer it.
        crate::git::RefKind::Head => format!("[{}]", name.text),
        crate::git::RefKind::Tag => format!("<{}>", name.text),
        _ => format!("({})", name.text),
    }
}

/// The refs as plain text, for the selected row where everything is one span.
fn refs_text(refs: &[crate::git::RefName]) -> String {
    if refs.is_empty() {
        return String::new();
    }
    refs.iter().map(ref_label).collect::<Vec<_>>().join(" ") + " "
}

fn stash_line(pal: Palette, stash: &crate::git::Stash, picked: bool, width: usize) -> Line<'static> {
    let text = format!("{}  {}", stash.name, stash.subject);
    if picked {
        let padded = format!("{text:<width$}");
        return Line::from(Span::styled(padded, Style::default().fg(pal.on_accent).bg(pal.accent)));
    }
    Line::from(vec![
        Span::styled(format!("{:<11}", stash.name), Style::default().fg(pal.warning)),
        Span::raw(stash.subject.clone()),
    ])
}

/// One commit read in full, over the graph it was picked from.
///
/// Its own box rather than a sixth tab: it is about the row the cursor is on rather than about
/// the repository, and a tab you can only reach from one row of one other tab is a tab that is
/// empty most of the time you look at it.
fn draw_git_detail(pal: Palette, f: &mut Frame, detail: &crate::app::GitDetail, lang: i18n::Lang, panel: Rect) {
    let rect = Rect {
        x: panel.x + 2,
        y: panel.y + 2,
        width: panel.width.saturating_sub(4),
        height: panel.height.saturating_sub(4),
    };
    if rect.height < 3 {
        return;
    }
    f.render_widget(Clear, rect);
    let title = format!(" {} — {} ", detail.hash, detail.subject);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.warning));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let lines: Vec<Line> = match detail.lines.as_ref() {
        None => vec![Line::from(Span::styled(
            i18n::msg_git_loading(lang),
            Style::default().fg(pal.text_dim),
        ))],
        Some(Err(complaint)) => {
            vec![Line::from(Span::styled(complaint.clone(), Style::default().fg(pal.danger)))]
        }
        Some(Ok(text)) => text
            .iter()
            .skip(detail.scroll)
            .take(inner.height as usize)
            .map(|l| Line::from(diff_span(pal, l)))
            .collect(),
    };
    f.render_widget(Paragraph::new(lines), inner);
}

/// One row of the status list: git's two letters, then the path.
///
/// The letters are coloured apart — the index's in green, the working tree's in red — because
/// the pair is the whole sentence: `MM` is a file that was added and then changed again, and a
/// single colour would leave that looking like one thing that happened once.
///
/// A picked row is one span across the full width instead of three, so the highlight covers the
/// row rather than stopping where the filename does.
fn status_line(pal: Palette, change: &crate::git::Change, picked: bool, width: usize) -> Line<'static> {
    let text = format!("{}{} {}", change.index, change.worktree, change.path.display());
    if picked {
        let padded = format!("{text:<width$}");
        return Line::from(Span::styled(padded, Style::default().fg(pal.on_accent).bg(pal.accent)));
    }
    let path = change.path.display().to_string();
    let untracked = change.untracked();
    // A file that is staged with nothing left over is exactly what the next commit will carry,
    // and saying so in the name is what makes the list answerable at a glance: `MM` and `M ` are
    // one letter apart and mean quite different things about what you are about to commit.
    let ready = change.staged() && !change.unstaged();
    Line::from(vec![
        Span::styled(
            change.index.to_string(),
            Style::default().fg(if untracked { pal.text_dim } else { pal.success }),
        ),
        Span::styled(
            format!("{} ", change.worktree),
            Style::default().fg(if untracked { pal.text_dim } else { pal.danger }),
        ),
        Span::styled(
            path,
            match (untracked, ready) {
                (true, _) => Style::default().fg(pal.text_dim),
                (_, true) => Style::default().fg(pal.success),
                _ => Style::default(),
            },
        ),
    ])
}

fn branch_line(pal: Palette, b: &crate::git::Branch, picked: bool, width: usize) -> Line<'static> {
    if picked {
        let mut text = format!("{} {}", if b.current { "●" } else { " " }, b.name);
        if let Some(upstream) = &b.upstream {
            text.push_str(&format!("  → {upstream}"));
        }
        if let Some(track) = &b.track {
            text.push_str(&format!(" {track}"));
        }
        let padded = format!("{text:<width$}");
        return Line::from(Span::styled(padded, Style::default().fg(pal.on_accent).bg(pal.accent)));
    }
    let mut spans = vec![
        Span::styled(if b.current { "● " } else { "  " }, Style::default().fg(pal.success)),
        Span::styled(
            b.name.clone(),
            if b.current {
                Style::default().fg(pal.success).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            },
        ),
    ];
    if let Some(upstream) = &b.upstream {
        spans.push(Span::styled(format!("  → {upstream}"), Style::default().fg(pal.text_dim)));
    }
    if let Some(track) = &b.track {
        spans.push(Span::styled(format!(" {track}"), Style::default().fg(pal.warning)));
    }
    Line::from(spans)
}

/// The last thing git said, and the keys this tab has.
///
/// The keys are drawn rather than left to the manual because they are single letters with no
/// modifier — which is only safe while the panel owns the keyboard, and only discoverable if the
/// panel says so. A key that stages a file on one tab and does nothing on the next has to say
/// which is which, or the way to find out is to press it.
fn draw_git_footer(pal: Palette, 
    f: &mut Frame,
    panel: &crate::app::GitPanel,
    lang: i18n::Lang,
    inner: Rect,
    show_notice: bool,
) {
    // Whether the notice gets a row is decided where the body's height is decided, and told to
    // this function rather than worked out again: two answers to that question is one row drawn
    // twice, which is what put git's last word over the last file in the list.
    if let Some((text, complaint)) = panel.notice.as_ref().filter(|_| show_notice) {
        let colour = if *complaint { pal.danger } else { pal.success };
        // The first line only: git's complaints run to paragraphs, and the terminal next door is
        // where the rest of one is read.
        let first = text.lines().next().unwrap_or_default().to_string();
        let area = Rect { y: inner.bottom().saturating_sub(2), height: 1, ..inner };
        f.render_widget(Paragraph::new(Line::from(Span::styled(first, Style::default().fg(colour)))), area);
    }
    let area = Rect { y: inner.bottom().saturating_sub(1), height: 1, ..inner };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            i18n::msg_git_keys(lang, panel.tab),
            Style::default().fg(pal.text_dim),
        ))),
        area,
    );
}

/// A question drawn over the panel: one line of question, one of answer.
fn draw_git_question(f: &mut Frame, panel: Rect, question: &str, typed: &str, colour: Color) {
    let width = panel.width.saturating_sub(6).max(20);
    let rect = centered_rect(width, 5, panel);
    f.render_widget(Clear, rect);
    let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(colour));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let lines = vec![
        Line::from(Span::styled(question.to_string(), Style::default().fg(colour))),
        Line::from(Span::raw(typed.to_string())),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

/// One line of a diff, coloured the way every diff has been coloured since diffs were coloured.
/// `+++`/`---` are file headers rather than added and removed lines, and are checked first — the
/// prefix test alone would paint a header green.
fn diff_span(pal: Palette, line: &str) -> Span<'_> {
    let style = if line.starts_with("+++") || line.starts_with("---") {
        Style::default().fg(pal.text_muted).add_modifier(Modifier::BOLD)
    } else if line.starts_with('+') {
        Style::default().fg(pal.success)
    } else if line.starts_with('-') {
        Style::default().fg(pal.danger)
    } else if line.starts_with("@@") {
        Style::default().fg(pal.accent)
    } else if line.starts_with("diff ") || line.starts_with("index ") {
        Style::default().fg(pal.text_dim)
    } else {
        Style::default()
    };
    Span::styled(line, style)
}

/// What to look for across the project, and what to put there instead. Its own box rather than
/// the shared input modal, because it carries the same two switches as the Find box and by the
/// same keys — a query typed here and a query typed there have to be the same kind of thing, and
/// the only way to know which way they are set is to be told.
///
/// Two fields, marked the way the terminal's name-and-command box marks its own, because they are
/// read the same way: whichever carries the caret is the one Tab last landed on. The second one
/// being empty is the whole difference between a search and a sweep, so its prompt says so rather
/// than leaving the reader to find out by pressing Enter.
fn draw_search_modal(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    use crate::app::SearchField;
    let lang = app.settings.lang;
    let rect = centered_rect(66, 9, full);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .title(format!(" {} ", i18n::t(lang, Key::ModalSearchProject)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let on_query = app.search_field == SearchField::Query;
    let marker = |active: bool| if active { "▶ " } else { "  " };
    let flags_on = app.search_case_sensitive || app.search_regex;
    let value = Style::default().fg(pal.warning);
    let label = Style::default().fg(pal.text_muted);
    let lines = vec![
        Line::from(Span::styled(
            format!("{}{}", marker(on_query), i18n::msg_search_prompt(lang)),
            label,
        )),
        Line::from(Span::styled(format!("  {}", app.search_input), value)),
        Line::from(Span::styled(
            format!("{}{}", marker(!on_query), i18n::msg_search_replace_prompt(lang)),
            label,
        )),
        Line::from(Span::styled(format!("  {}", app.search_replace), value)),
        Line::from(Span::styled(
            i18n::msg_find_flags(lang, app.search_case_sensitive, app.search_regex),
            Style::default().fg(if flags_on { pal.accent } else { pal.text_dim }),
        )),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    // The two spaces the value rows are indented by are part of the caret's arithmetic: it sits
    // after the last character typed, which is two columns in from the frame.
    let typed = match on_query { true => &app.search_input, false => &app.search_replace };
    let cursor_x =
        (inner.x + 2 + typed.chars().count() as u16).min(inner.right().saturating_sub(1));
    f.set_cursor_position((cursor_x, inner.y + if on_query { 1 } else { 3 }));
}

/// The terminal's name and its startup command, in one box: two prompts, two values, and a
/// caret on whichever field is being typed into.
fn draw_terminal_rename_modal(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    use crate::app::TerminalField;
    let lang = app.settings.lang;
    let rect = centered_rect(74, 7, full);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .title(format!(" {} ", i18n::t(lang, Key::ModalTerminalForm)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let on_name = app.terminal_rename_field == TerminalField::Name;
    let marker = |active: bool| if active { "▶ " } else { "  " };
    let value = Style::default().fg(pal.warning);
    let label = Style::default().fg(pal.text_muted);
    let lines = vec![
        Line::from(Span::styled(format!("{}{}", marker(on_name), i18n::msg_terminal_rename_prompt(lang)), label)),
        Line::from(Span::styled(format!("  {}", app.terminal_rename_input), value)),
        Line::from(Span::styled(format!("{}{}", marker(!on_name), i18n::msg_terminal_startup_prompt(lang)), label)),
        Line::from(Span::styled(format!("  {}", app.terminal_startup_input), value)),
        Line::from(Span::styled(i18n::msg_terminal_form_hint(lang), Style::default().fg(pal.text_dim))),
    ];
    f.render_widget(Paragraph::new(lines), inner);

    let (row, len) = if on_name {
        (1u16, app.terminal_rename_input.chars().count())
    } else {
        (3u16, app.terminal_startup_input.chars().count())
    };
    let cursor_x = (inner.x + 2 + len as u16).min(inner.right().saturating_sub(1));
    f.set_cursor_position((cursor_x, inner.y + row));
}

fn draw_workspace_save_modal(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    let prompt = i18n::msg_workspace_save_prompt(app.settings.lang);
    let lang = app.settings.lang;
    draw_input_modal(pal, f, full, i18n::t(lang, Key::ModalSaveWorkspace), &prompt, &app.workspace_save_input);
}

/// The manual's frame: bigger than the palette, but still a modal with the screen showing
/// around it. Shared by the renderer and by click handling.
pub fn manual_rect(full: Rect) -> Rect {
    let width = full.width.saturating_sub(4).min(100).max(24);
    let height = full.height.saturating_sub(2).min(40).max(8);
    centered_rect(width, height, full)
}

/// Columns given to the table of contents down the left side.
const MANUAL_LIST_WIDTH: u16 = 16;

pub fn manual_list_rect(rect: Rect) -> Rect {
    let inner = inner_rect(rect);
    Rect { width: MANUAL_LIST_WIDTH.min(inner.width), ..inner }
}

/// The reading pane: everything right of the section list, minus the hint line at the bottom.
fn manual_body_rect(rect: Rect) -> Rect {
    let inner = inner_rect(rect);
    let list = manual_list_rect(rect);
    // One column for the rule between the two, one for breathing room.
    let x = inner.x + list.width + 2;
    Rect {
        x,
        y: inner.y,
        width: inner.width.saturating_sub(list.width + 2),
        height: inner.height.saturating_sub(2),
    }
}

/// Rows of manual text on screen, so paging keys move by exactly one screenful.
pub fn manual_body_height(full: Rect) -> u16 {
    manual_body_rect(manual_rect(full)).height
}

/// Box-drawing characters, which in the manual only ever belong to a diagram.
fn is_box_rule(c: char) -> bool {
    matches!(
        c,
        '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '┼' | '─' | '│'
            | '║' | '╨' | '╧' | '╪' | '╫' | '═' | '╔' | '╗' | '╚' | '╝' | '╠' | '╣' | '╦' | '╩' | '╬'
    )
}

/// Whether a word is a key or a chord, and so worth picking out of the surrounding prose.
fn looks_like_key(word: &str) -> bool {
    let w = word.trim_matches(|c: char| matches!(c, ',' | '.' | ')' | '(' | ':' | ';' | '—'));
    if w.starts_with("Ctrl+") || w.starts_with("Alt+") || w.starts_with("Shift+") {
        return true;
    }
    if matches!(w, "Esc" | "Enter" | "Tab" | "Del" | "Backspace" | "Home" | "End" | "Space" | "PgUp" | "PgDn") {
        return true;
    }
    // F1..F12, but not a word that merely starts with F.
    w.len() >= 2 && w.starts_with('F') && w[1..].chars().all(|c| c.is_ascii_digit())
}

/// Colours one line of the manual. It is reference material people scan rather than read, so
/// the colour goes to the three things they scan *for* — the key names, the section headings and
/// the outline of the diagrams — and prose stays plain, which is what keeps the colour meaning
/// something. Styling happens here rather than in `manual.rs` so the text there stays plain
/// strings that are easy to edit and to check the width of.
///
/// Borrowed rather than `'static`, because a line whose chords the reader has remapped is built
/// for this frame and does not outlive it.
fn manual_line(pal: Palette, line: &str) -> Line<'_> {
    let rule = Style::default().fg(pal.text_dim);
    let key = Style::default().fg(pal.warning);
    let heading = Style::default().fg(pal.accent).add_modifier(Modifier::BOLD);
    let plain = Style::default();

    // A diagram: the rules recede so the labels drawn inside them come forward.
    if line.chars().any(is_box_rule) {
        let mut spans: Vec<Span> = Vec::new();
        let mut buf = String::new();
        let mut in_rule = false;
        for c in line.chars() {
            if is_box_rule(c) != in_rule && !buf.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut buf), if in_rule { rule } else { plain }));
            }
            in_rule = is_box_rule(c);
            buf.push(c);
        }
        if !buf.is_empty() {
            spans.push(Span::styled(buf, if in_rule { rule } else { plain }));
        }
        return Line::from(spans);
    }

    // A heading: flush left and introducing what follows.
    let trimmed = line.trim_end();
    if !line.starts_with(' ') && trimmed.ends_with(':') {
        return Line::from(Span::styled(line, heading));
    }

    // Everything else word by word, so a chord is picked out whether it sits in the indented
    // key column or in the middle of a sentence.
    let mut spans: Vec<Span> = Vec::new();
    let mut rest = line;
    while !rest.is_empty() {
        let gap: String = rest.chars().take_while(|c| *c == ' ').collect();
        if !gap.is_empty() {
            spans.push(Span::raw(&rest[..gap.len()]));
            rest = &rest[gap.len()..];
            continue;
        }
        let word_len = rest.find(' ').unwrap_or(rest.len());
        let (word, tail) = rest.split_at(word_len);
        spans.push(Span::styled(word, if looks_like_key(word) { key } else { plain }));
        rest = tail;
    }
    Line::from(spans)
}

fn draw_manual(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    let Some(state) = app.manual.as_ref() else { return };
    let lang = app.settings.lang;
    let sections = crate::manual::sections(lang, &app.keymap);
    let rect = manual_rect(full);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .title(format!(" {} · v{} ", i18n::t(lang, Key::ManualTitle), env!("CARGO_PKG_VERSION")))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    // Table of contents, numbered so the digit keys have something to point at.
    let list = manual_list_rect(rect);
    let toc: Vec<Line> = sections
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let style = if i == state.section {
                Style::default().fg(pal.on_accent).bg(pal.accent)
            } else {
                Style::default().fg(pal.text_muted)
            };
            let width = list.width as usize;
            let label = format!(" {} {}", i + 1, s.title);
            let label: String = label.chars().take(width).collect();
            let pad = width.saturating_sub(label.chars().count());
            Line::from(Span::styled(format!("{label}{}", " ".repeat(pad)), style))
        })
        .collect();
    f.render_widget(Paragraph::new(toc), list);

    // The rule between the contents and the text.
    let rule = Rect { x: inner.x + list.width + 1, y: inner.y, width: 1, height: inner.height };
    let rule_lines: Vec<Line> = (0..rule.height)
        .map(|_| Line::from(Span::styled("│", Style::default().fg(pal.text_dim))))
        .collect();
    f.render_widget(Paragraph::new(rule_lines), rule);

    let body_area = manual_body_rect(rect);
    let Some(section) = sections.get(state.section) else { return };
    let visible: Vec<Line> = section
        .body
        .iter()
        .skip(state.scroll)
        .take(body_area.height as usize)
        .map(|line| manual_line(pal, line.as_ref()))
        .collect();
    f.render_widget(Paragraph::new(visible), body_area);

    // Position within the section, then the key hints, on the two rows kept back above.
    let footer = Rect { x: body_area.x, y: body_area.y + body_area.height, width: body_area.width, height: 2 };
    let shown = (state.scroll + body_area.height as usize).min(section.body.len());
    let position = format!("{}/{}  ", shown, section.body.len().max(1));
    let footer_lines = vec![
        Line::from(Span::styled(
            format!("{} · {}", section.title, position),
            Style::default().fg(pal.text_dim),
        )),
        Line::from(Span::styled(i18n::t(lang, Key::ManualHint), Style::default().fg(pal.text_dim))),
    ];
    f.render_widget(Paragraph::new(footer_lines), footer);
}

fn draw_new_entry_modal(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    let lang = app.settings.lang;
    let title = i18n::t(lang, if app.new_entry_is_dir { Key::ModalNewFolder } else { Key::ModalNewFile });
    let prompt = i18n::msg_new_entry_prompt(lang, app.new_entry_is_dir);
    draw_input_modal(pal, f, full, title, prompt, &app.new_entry_input);
}

/// Where the theme list hangs: under its own button, and slid left if the list is wider than
/// the room between the button and the right edge — the same rule the menus follow, because a
/// drop-down that runs off the screen is a drop-down with rows nobody can read.
pub fn theme_menu_rect(app: &App, full: Rect) -> Option<Rect> {
    app.theme_menu?;
    let button = menu_bar_theme_range(app, full.width);
    if button.is_empty() {
        return None;
    }
    let choices = crate::theme::ThemeChoice::all();
    let widest = choices.iter().map(|c| columns(c.name())).max().unwrap_or(0);
    // Two for the borders, two for the marker that says which one is on.
    let width = (widest + 4).min(full.width);
    let height = choices.len() as u16 + 2;
    Some(Rect {
        x: button.start.min(full.width.saturating_sub(width)),
        y: full.y + 1,
        width,
        height: height.min(full.height.saturating_sub(1)),
    })
}

fn draw_theme_menu(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    let Some(selected) = app.theme_menu else { return };
    let Some(rect) = theme_menu_rect(app, full) else { return };
    f.render_widget(Clear, rect);
    let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(pal.accent));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let items: Vec<ListItem> = crate::theme::ThemeChoice::all()
        .into_iter()
        .map(|choice| {
            // The dot marks the choice in use, not the row the cursor is on: arrowing down a list
            // must not look like it has already changed anything. The choice and not the theme,
            // so "Auto" stays marked rather than the theme it resolved to — which is the honest
            // answer to "what did I pick", and the only one that survives a change of terminal.
            let marker = if choice == app.settings.theme { "\u{25cf} " } else { "  " };
            ListItem::new(Line::from(format!("{marker}{}", choice.name())))
        })
        .collect();
    let mut state = ListState::default();
    state.select(Some(selected));
    let list = List::new(items)
        .highlight_style(Style::default().fg(pal.on_accent).bg(pal.accent));
    f.render_stateful_widget(list, inner, &mut state);
}

fn draw_run_menu(f: &mut Frame, app: &App, editor_area: Rect, full: Rect) {
    let pal = app.palette();
    let Some(menu) = app.run_menu.as_ref() else { return };
    let Some(rect) = run_menu_rect(app, editor_area, full) else { return };
    f.render_widget(Clear, rect);
    let block = Block::default()
        // Named after the extension, so it is plain that what's chosen here applies to every
        // file of this kind rather than only to the one on screen.
        .title(format!(" {} \u{00b7} .{} ", i18n::t(app.settings.lang, Key::RunMenuTitle), menu.ext))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let items: Vec<ListItem> = app
        .run_menu_rows()
        .into_iter()
        .map(|row| {
            let marker = if row.active { "● " } else { "  " };
            let mut spans = vec![Span::raw(format!("{marker}{}", row.label))];
            if let Some(detail) = row.detail {
                spans.push(Span::styled(format!("  {detail}"), Style::default().fg(pal.text_dim)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items).highlight_style(Style::default().fg(pal.on_accent).bg(pal.accent));
    let mut state = ListState::default();
    state.select(Some(menu.selected));
    f.render_stateful_widget(list, inner, &mut state);
}

/// The run command for one extension, typed in full. Its own box rather than the shared
/// single-line one: a command line is long, and the placeholders are worth spelling out where
/// they are being typed instead of leaving them to the manual.
fn draw_run_command_modal(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    let Some((ext, scope)) = app.run_command_edit.as_ref() else { return };
    let lang = app.settings.lang;
    let rect = centered_rect(84, 7, full);
    f.render_widget(Clear, rect);
    // The file being written is named in the title. Two boxes that look alike but land in
    // different places is exactly the confusion worth spending a title on.
    let where_ = match scope {
        crate::app::RunScope::Global => "settings.toml".to_string(),
        crate::app::RunScope::Project => crate::settings::PROJECT_FILE.to_string(),
    };
    let block = Block::default()
        .title(format!(" {} .{ext} \u{2192} {where_} ", i18n::t(lang, Key::RunMenuTitle)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let dim = Style::default().fg(pal.text_dim);
    let lines = vec![
        Line::from(i18n::msg_run_command_prompt(lang, *scope)),
        Line::from(Span::styled(app.run_command_input.clone(), Style::default().fg(pal.warning))),
        Line::from(""),
        Line::from(Span::styled(i18n::msg_run_command_placeholders(lang), dim)),
    ];
    f.render_widget(Paragraph::new(lines), inner);
    // Clamped so a command longer than the box doesn't park the caret outside it.
    let cursor_x = (inner.x + app.run_command_input.chars().count() as u16)
        .min(inner.x + inner.width.saturating_sub(1));
    f.set_cursor_position((cursor_x, inner.y + 1));
}

fn draw_venv_register_modal(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    let lang = app.settings.lang;
    let (title, prompt) = match app.venv_register {
        Some(crate::app::VenvRegisterStep::Path) => {
            (i18n::t(lang, Key::ModalAddVenvPath), i18n::msg_venv_path_prompt(lang))
        }
        Some(crate::app::VenvRegisterStep::Nickname) => {
            (i18n::t(lang, Key::ModalAddVenvNickname), i18n::msg_venv_nickname_prompt(lang))
        }
        None => return,
    };
    draw_input_modal(pal, f, full, title, &prompt, &app.venv_register_input);
}

fn draw_save_as_modal(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    let prompt = i18n::msg_save_as_prompt(app.settings.lang);
    let lang = app.settings.lang;
    draw_input_modal(pal, f, full, i18n::t(lang, Key::ModalSaveAs), &prompt, &app.save_as_input);
}

/// Where the picker modal sits. Pulled out of the drawing so the mouse can land on exactly the
/// rows that were painted — a second copy of this arithmetic would drift the first time either
/// side changed.
pub fn picker_rect(full: Rect) -> Rect {
    let width = full.width.saturating_sub(8).clamp(20, 90);
    let height = full.height.saturating_sub(4).clamp(4, 20);
    centered_rect(width, height, full)
}

/// Which result a click landed on, as an index into `filtered` — the same thing `selected`
/// counts. `None` for the query line, the borders, or anywhere outside the modal.
pub fn picker_row_at(p: &crate::picker::Picker, full: Rect, col: u16, row: u16) -> Option<usize> {
    let inner = inner_rect(picker_rect(full));
    let inside = col >= inner.x
        && col < inner.x + inner.width
        && row >= inner.y
        && row < inner.y + inner.height;
    if !inside || row == inner.y {
        return None;
    }
    let list_rows = inner.height.saturating_sub(1) as usize;
    // The list scrolls to keep the selection visible, so the first row on screen is not
    // necessarily the first result — the same offset the drawing uses.
    let start = if p.selected >= list_rows {
        p.selected + 1 - list_rows
    } else {
        0
    };
    let index = start + (row - inner.y - 1) as usize;
    (index < p.filtered.len()).then_some(index)
}

/// Where the completion popup goes, given the cell the cursor is in.
///
/// Pure, so the awkward cases are settled in tests rather than by squinting at a terminal: the
/// list hangs under the cursor, flips above it when the bottom of the screen is nearer than the
/// list is tall, and slides left rather than spilling off the right edge. The text column is
/// lined up with the first letter of the word being completed, so the candidates read as a
/// continuation of what was typed rather than as a box that happens to be nearby.
pub fn completion_rect(anchor: (u16, u16), prefix_len: u16, width: u16, rows: u16, full: Rect) -> Rect {
    let height = rows + 2;
    let width = width.min(full.width.max(1));
    // The border and the two-cell selection marker sit between the box edge and the text.
    let x = anchor.0.saturating_sub(prefix_len + 3);
    let x = x.min(full.right().saturating_sub(width)).max(full.x);
    let below = anchor.1 + 1;
    let y = if below + height <= full.bottom() {
        below
    } else {
        // Not enough room under the cursor: hang the list above it instead, and if there is no
        // room there either, take the top of the screen rather than sliding off it.
        anchor.1.checked_sub(height).unwrap_or(full.y).max(full.y)
    };
    Rect { x, y, width, height }
}

/// Takes the popup and the cursor cell rather than the whole `App`, so a test can render one
/// into a buffer and read back what it drew — which is the only way to check a list of words
/// actually reaches the screen without a terminal to look at.
fn draw_completion(pal: Palette, f: &mut Frame, popup: &crate::complete::Popup, anchor: (u16, u16), full: Rect) {
    let rows: Vec<(&crate::complete::Candidate, bool)> = popup.visible().collect();
    if rows.is_empty() {
        return;
    }
    let prefix_len = popup.prefix.chars().count();
    let longest = rows.iter().map(|(c, _)| c.text.chars().count()).max().unwrap_or(0);
    // Two for the marker column, two for the border, two so a word is not flush against it.
    let width = (longest + 6).clamp(14, 44) as u16;
    let rect = completion_rect(anchor, prefix_len as u16, width, rows.len() as u16, full);

    f.render_widget(Clear, rect);
    let mut block =
        Block::default().borders(Borders::ALL).border_style(Style::default().fg(pal.text_dim));
    // Only when the list is taller than its window: otherwise the count says what is already
    // plainly on screen, and the border is the wrong place to say anything twice.
    if popup.len() > rows.len() {
        block = block
            .title(format!(" {}/{} ", popup.selected + 1, popup.len()))
            .title_style(Style::default().fg(pal.text_dim));
    }
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let text_width = inner.width.saturating_sub(2) as usize;
    let lines: Vec<Line> = rows
        .iter()
        .map(|(cand, selected)| {
            let base = if *selected {
                Style::default().fg(pal.on_accent).bg(pal.accent)
            } else if cand.source == crate::complete::Source::Keyword {
                // The same blue the highlighter gives a keyword, so the list says where the
                // candidate came from without spending a column on saying it.
                Style::default().fg(pal.info)
            } else if cand.source == crate::complete::Source::Session {
                // Green for something that exists in the interpreter right now, which is worth
                // telling apart: it means the name is real rather than merely written somewhere.
                Style::default().fg(pal.success)
            } else if cand.source == crate::complete::Source::Lsp {
                // Magenta for a name the language server offered. Same reason as the green one:
                // it says the word is known to something that understands the file, rather than
                // having been read off it — and after a dot, those are the only rows that mean
                // anything at all.
                Style::default().fg(pal.special)
            } else {
                Style::default().fg(pal.text_muted)
            };
            let mut label = cand.text.clone();
            if text_width == 0 {
                // No room even for a single ellipsis character: better to draw nothing than a
                // truncated label that is itself wider than the space it is meant to fit in.
                label.clear();
            } else if label.chars().count() > text_width {
                label = label.chars().take(text_width.saturating_sub(1)).collect::<String>() + "…";
            }
            // Bold the letters already typed — but only when they really are the opening of the
            // word. A fuzzy match has them scattered through it, and marking the first few there
            // would be pointing at the wrong letters.
            let lit = if label.to_lowercase().starts_with(&popup.prefix.to_lowercase()) {
                prefix_len.min(label.chars().count())
            } else {
                0
            };
            let head: String = label.chars().take(lit).collect();
            let tail: String = label.chars().skip(lit).collect();
            // Saturating: at very small popup widths (the modal squeezed against a window edge)
            // the marker and label alone can already fill or exceed `inner.width`, and an
            // unchecked subtraction there used to panic rather than simply draw no padding.
            let pad = (inner.width as usize).saturating_sub(2 + label.chars().count());
            let mut spans = vec![Span::styled(if *selected { "▶ " } else { "  " }, base)];
            if !head.is_empty() {
                spans.push(Span::styled(head, base.add_modifier(Modifier::BOLD)));
            }
            if !tail.is_empty() {
                spans.push(Span::styled(tail, base));
            }
            if pad > 0 {
                spans.push(Span::styled(" ".repeat(pad), base));
            }
            Line::from(spans)
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

/// How one row of a chooser is painted.
///
/// Almost every row is a row. The exception is the workspace list, where three of the entries
/// are not files at all: the default layout and the two session presets are built into
/// CleeCode, cannot be saved over and cannot be deleted, and until they were coloured there was
/// nothing on screen that said so — they sat in the list looking exactly like something you
/// had made and could throw away. Cyan is the colour the chooser already uses for the parts
/// that belong to the app rather than to you: its border, its prompt.
fn picker_row_style(pal: Palette, kind: crate::picker::PickerKind, label: &str, selected: bool) -> Style {
    if selected {
        return Style::default().fg(pal.on_accent).bg(pal.accent);
    }
    let built_in = matches!(kind, crate::picker::PickerKind::Workspaces)
        && crate::workspace::is_built_in(label);
    if built_in {
        Style::default().fg(pal.accent)
    } else {
        Style::default().fg(pal.text_muted)
    }
}

fn draw_picker_modal(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    let Some(p) = app.picker.as_ref() else { return };
    let rect = picker_rect(full);
    f.render_widget(Clear, rect);
    let title = format!(" {}  {} ", p.title, i18n::msg_picker_matches(app.settings.lang, p.filtered.len()));
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let list_rows = inner.height.saturating_sub(1) as usize;
    // Scroll so the selected row stays visible.
    let start = if p.selected >= list_rows { p.selected + 1 - list_rows } else { 0 };
    let width = inner.width as usize;

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("> ", Style::default().fg(pal.accent)),
        Span::styled(p.query.clone(), Style::default().fg(pal.bright)),
    ]));
    for (row, &item_idx) in p.filtered.iter().enumerate().skip(start).take(list_rows) {
        let item = &p.items[item_idx];
        let selected = row == p.selected;
        let row_style = picker_row_style(pal, p.kind, &item.label, selected);
        let sc = item.shortcut.as_deref().unwrap_or("");
        let sc_w = sc.chars().count();
        // Reserve room for the right-aligned shortcut, then fit/ellipsize the label.
        let prefix_w = 2;
        let sc_gap = if sc_w > 0 { sc_w + 2 } else { 0 };
        let label_budget = width.saturating_sub(prefix_w + sc_gap);
        let mut label = item.label.clone();
        if label.chars().count() > label_budget {
            label = label.chars().take(label_budget.saturating_sub(1)).collect::<String>() + "…";
        }
        let prefix = if selected { "▶ " } else { "  " };
        let used = prefix_w + label.chars().count() + sc_w;
        let pad = width.saturating_sub(used);
        let mut spans = vec![Span::styled(format!("{prefix}{label}"), row_style)];
        if pad > 0 {
            spans.push(Span::styled(" ".repeat(pad), row_style));
        }
        if sc_w > 0 {
            let sc_style = if selected {
                Style::default().fg(pal.on_accent).bg(pal.accent)
            } else {
                Style::default().fg(pal.text_dim)
            };
            spans.push(Span::styled(sc.to_string(), sc_style));
        }
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), inner);
    let cursor_x = (inner.x + 2 + p.query.chars().count() as u16)
        .min(inner.right().saturating_sub(1));
    f.set_cursor_position((cursor_x, inner.y));
}

fn draw_find_modal(f: &mut Frame, app: &App, full: Rect) {
    let pal = app.palette();
    let Some(fs) = app.find.as_ref() else { return };
    let lang = app.settings.lang;
    // Two rows more than the fields need: the flag line always, and the pattern error when there
    // is one. A modal that grew and shrank as you typed would move the fields under the cursor.
    let rect = centered_rect(72, 9, full);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .title(format!(" {} ", i18n::t(lang, Key::ModalFindReplace)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let count = if fs.query.is_empty() {
        String::new()
    } else if fs.matches.is_empty() {
        format!("  {}", i18n::t(lang, Key::FindNoMatches))
    } else {
        format!("  {}/{}", fs.current + 1, fs.matches.len())
    };
    let find_marker = if fs.focus_replace { "  " } else { "▶ " };
    let repl_marker = if fs.focus_replace { "▶ " } else { "  " };
    let label = Style::default().fg(pal.text_muted);
    let value = Style::default().fg(pal.warning);
    // A pattern that will not compile takes the place of the count: it is the answer to what
    // the query is doing, and "no matches" would be a lie about a search that never ran.
    let count = match &fs.error {
        Some(_) => String::new(),
        None => count,
    };
    let flags = Line::from(Span::styled(
        i18n::msg_find_flags(lang, fs.case_sensitive, fs.regex),
        Style::default().fg(if fs.case_sensitive || fs.regex { pal.accent } else { pal.text_dim }),
    ));
    // Both fields start their text in the same column, so the two rows read as one form rather
    // than as two sentences — which means the wider of the two labels sets the column, and the
    // caret below has to be placed from the same number rather than from a count of "Find:".
    let find_label = i18n::t(lang, Key::FindLabel);
    let replace_label = i18n::t(lang, Key::ReplaceLabel);
    let label_width = find_label.chars().count().max(replace_label.chars().count()) + 1;
    let mut lines = vec![
        Line::from(vec![
            Span::styled(format!("{find_marker}{find_label:<label_width$}"), label),
            Span::styled(fs.query.clone(), value),
            Span::styled(count, Style::default().fg(pal.text_dim)),
        ]),
        Line::from(vec![
            Span::styled(format!("{repl_marker}{replace_label:<label_width$}"), label),
            Span::styled(fs.replace.clone(), value),
        ]),
        flags,
    ];
    // What the current match turns into, and how many share its fate. Without this, Ctrl+A is a
    // key you press to find out what it does — and with a pattern, the difference between a
    // replacement that quotes a group back and one that writes a literal dollar is invisible
    // until after the file has been changed.
    if let Some(m) = fs.current_match() {
        let matched = app.editor().rope.slice(m.0..m.1).to_string();
        if let Some(preview) = fs.preview(&matched, inner.width.saturating_sub(24) as usize) {
            lines.push(Line::from(vec![
                Span::styled("  ", label),
                Span::styled(preview, Style::default().fg(pal.success)),
                Span::styled(
                    i18n::msg_replace_all_count(lang, fs.matches.len()),
                    Style::default().fg(pal.text_dim),
                ),
            ]));
        }
    }
    if let Some(detail) = &fs.error {
        lines.push(Line::from(Span::styled(
            i18n::msg_find_pattern_error(lang, detail),
            Style::default().fg(pal.danger),
        )));
    }
    lines.push(Line::from(Span::styled(i18n::msg_find_hint(lang), Style::default().fg(pal.text_dim))));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);

    // Cursor sits at the end of whichever field is focused.
    let (row, text_len) = if fs.focus_replace {
        (1u16, fs.replace.chars().count())
    } else {
        (0u16, fs.query.chars().count())
    };
    // Two columns for the ▶ marker, then whatever the labels were padded to above. Taken from
    // the same number that drew them: a hard-coded 11 was the English "▶ Find:    " and put the
    // caret inside the word in every other language.
    let cursor_x = (inner.x + 2 + label_width as u16 + text_len as u16)
        .min(inner.right().saturating_sub(1));
    f.set_cursor_position((cursor_x, inner.y + row));
}

fn git_status_color(pal: Palette, status: crate::git_status::FileStatus) -> Color {
    use crate::git_status::FileStatus;
    match status {
        FileStatus::Modified => pal.warning,
        FileStatus::Added => pal.success,
        FileStatus::Deleted => pal.danger,
        FileStatus::Renamed => pal.accent,
        FileStatus::Untracked => pal.text_muted,
    }
}

/// Icon + color for a file tree row based on its name. Directories are handled by the
/// caller (they use the expand/collapse chevron instead). Glyphs are Nerd Font Private
/// Use Area codepoints (same set as nvim-web-devicons) — they need a Nerd Font to render
/// as icons; CleeCode ships one and can install it via `--install-font` (see main.rs).
fn file_icon(name: &str) -> (&'static str, Color) {
    match name.to_lowercase().as_str() {
        ".gitignore" | ".gitattributes" | ".gitmodules" => return ("\u{e702}", Color::Rgb(245, 77, 39)),
        ".env" | ".env.local" | ".env.example" => return ("\u{f462}", Color::Rgb(250, 247, 67)),
        "dockerfile" | "docker-compose.yml" | "docker-compose.yaml" => {
            return ("\u{f0868}", Color::Rgb(69, 142, 230));
        }
        "makefile" => return ("\u{e779}", Color::Rgb(109, 128, 134)),
        _ => {}
    }
    let ext = std::path::Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "rs" => ("\u{e68b}", Color::Rgb(222, 165, 132)),
        "py" => ("\u{e606}", Color::Rgb(255, 188, 3)),
        "js" | "mjs" | "cjs" => ("\u{e60c}", Color::Rgb(203, 203, 65)),
        "ts" => ("\u{e628}", Color::Rgb(81, 154, 186)),
        "tsx" => ("\u{e7ba}", Color::Rgb(19, 84, 191)),
        "jsx" => ("\u{e625}", Color::Rgb(32, 194, 227)),
        "json" => ("\u{e60b}", Color::Rgb(203, 203, 65)),
        "yaml" | "yml" => ("\u{e8eb}", Color::Rgb(215, 0, 0)),
        "toml" => ("\u{e6b2}", Color::Rgb(156, 66, 33)),
        "md" => ("\u{f48a}", Color::Rgb(221, 221, 221)),
        "markdown" => ("\u{e609}", Color::Rgb(221, 221, 221)),
        "html" | "htm" => ("\u{e736}", Color::Rgb(228, 77, 38)),
        "css" => ("\u{e6b8}", Color::Rgb(102, 51, 153)),
        "scss" | "sass" => ("\u{e603}", Color::Rgb(245, 83, 133)),
        "sh" | "fish" => ("\u{e795}", Color::Rgb(77, 90, 94)),
        "bash" => ("\u{e760}", Color::Rgb(137, 224, 81)),
        "zsh" => ("\u{e795}", Color::Rgb(137, 224, 81)),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" => ("\u{e60d}", Color::Rgb(160, 116, 196)),
        "svg" => ("\u{f0721}", Color::Rgb(255, 177, 59)),
        "lock" => ("\u{e672}", Color::Rgb(187, 187, 187)),
        "sql" => ("\u{e706}", Color::Rgb(218, 216, 216)),
        "go" => ("\u{e627}", Color::Rgb(0, 173, 216)),
        "rb" => ("\u{e791}", Color::Rgb(112, 21, 22)),
        "php" => ("\u{e608}", Color::Rgb(160, 116, 196)),
        "c" => ("\u{e61e}", Color::Rgb(89, 158, 255)),
        "h" | "hpp" => ("\u{f0fd}", Color::Rgb(160, 116, 196)),
        "cpp" | "cc" => ("\u{e61d}", Color::Rgb(81, 154, 186)),
        "java" => ("\u{e738}", Color::Rgb(204, 62, 68)),
        "pdf" => ("\u{eaeb}", Color::Rgb(179, 11, 0)),
        _ => ("\u{f0f6}", Color::Rgb(109, 128, 134)),
    }
}

/// What a tree row shows for a file, and how many blanks follow it.
///
/// The layout is indent + icon + space + name + padding + the git dot, and the dot's column is
/// taken out first: a name allowed the whole row pushed the dot off the side, so the files whose
/// status is hardest to guess — the deeply nested ones, with the long names — were exactly the
/// ones that stopped saying they had changed. An ellipsis says the name was cut; a missing dot
/// said nothing at all.
fn tree_row_name(indent: &str, name: &str, inner_width: usize) -> (String, usize) {
    let indent_width = indent.chars().count();
    // Two columns for the icon and its space, one kept back for the dot.
    let budget = inner_width.saturating_sub(indent_width + 3);
    let name = fit(name, budget);
    let used = indent_width + 2 + name.chars().count();
    (name, inner_width.saturating_sub(used + 1))
}

fn draw_file_tree(f: &mut Frame, app: &mut App, area: Rect) {
    let pal = app.palette();
    let focused = app.focus == Focus::FileTree;
    let block = Block::default()
        .title(format!(" {} ", i18n::t(app.settings.lang, Key::PanelFile)))
        .borders(Borders::ALL)
        .border_style(focused_border_style(pal, focused, app.layout_resize_active()));

    let paths = app.file_tree.visible_paths();
    let inner_width = inner_rect(area).width as usize;
    let items: Vec<ListItem> = app
        .file_tree
        .visible
        .iter()
        .zip(paths.iter())
        .map(|(entry, path)| {
            if entry.is_up {
                return ListItem::new(Line::from("  .."));
            }
            let indent = "  ".repeat(entry.depth);
            let (icon, icon_color) = if entry.is_dir {
                (if entry.expanded { "\u{f07c}" } else { "\u{f07b}" }, pal.folder)
            } else {
                file_icon(&entry.name)
            };
            let dot = path.as_ref().and_then(|p| app.git_status.get(p));
            let (name, pad) = tree_row_name(&indent, &entry.name, inner_width);
            let mut spans = vec![
                Span::raw(indent),
                Span::styled(icon, Style::default().fg(icon_color)),
                Span::raw(format!(" {name}")),
            ];
            if pad > 0 {
                spans.push(Span::raw(" ".repeat(pad)));
            }
            spans.push(match dot {
                Some(status) => Span::styled("\u{25cf}", Style::default().fg(git_status_color(pal, *status))),
                None => Span::raw(" "),
            });
            ListItem::new(Line::from(spans))
        })
        .collect();

    let mut state = ListState::default();
    if !app.file_tree.visible.is_empty() {
        state.select(Some(app.file_tree.selected));
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    f.render_stateful_widget(list, area, &mut state);
}

fn clip_line_spans(spans: &[(Style, String)], skip: usize, take: usize) -> Vec<Span<'static>> {
    let mut result = Vec::new();
    let mut to_skip = skip;
    let mut remaining = take;
    for (style, text) in spans {
        if remaining == 0 {
            break;
        }
        let char_count = text.chars().count();
        if to_skip >= char_count {
            to_skip -= char_count;
            continue;
        }
        let start = to_skip;
        to_skip = 0;
        let available = char_count - start;
        let take_here = available.min(remaining);
        let substr: String = text.chars().skip(start).take(take_here).collect();
        remaining -= take_here;
        if !substr.is_empty() {
            result.push(Span::styled(substr, *style));
        }
    }
    result
}

fn apply_whitespace_marks(spans: Vec<(Style, String)>) -> Vec<(Style, String)> {
    spans
        .into_iter()
        .map(|(style, text)| {
            let marked: String = text
                .chars()
                .map(|c| match c {
                    ' ' => '·',
                    '\t' => '→',
                    other => other,
                })
                .collect();
            (style, marked)
        })
        .collect()
}

/// Overlays a background highlight on the [sel_from, sel_to) character range of a line's spans.
/// Restyles the characters in `[from, to)`, splitting spans wherever the range starts or ends
/// inside one.
///
/// One function for the selection and for a diagnostic's underline. Both are "these columns look
/// different", and the arithmetic — a range that begins mid-span, on a line whose spans came from
/// the highlighter and do not line up with anything — is the part worth having once rather than
/// twice.
fn restyle_range(
    spans: Vec<(Style, String)>,
    from: usize,
    to: usize,
    restyle: impl Fn(Style) -> Style,
) -> Vec<(Style, String)> {
    if from >= to {
        return spans;
    }
    let mut result = Vec::new();
    let mut pos = 0usize;
    for (style, text) in spans {
        let char_count = text.chars().count();
        let span_start = pos;
        let span_end = pos + char_count;
        pos = span_end;
        if span_end <= from || span_start >= to {
            result.push((style, text));
            continue;
        }
        let chars: Vec<char> = text.chars().collect();
        let local_from = from.saturating_sub(span_start).min(chars.len());
        let local_to = to.saturating_sub(span_start).min(chars.len());
        if local_from > 0 {
            result.push((style, chars[..local_from].iter().collect()));
        }
        if local_to > local_from {
            result.push((restyle(style), chars[local_from..local_to].iter().collect()));
        }
        if local_to < chars.len() {
            result.push((style, chars[local_to..].iter().collect()));
        }
    }
    result
}

fn highlight_selection(pal: Palette, spans: Vec<(Style, String)>, sel_from: usize, sel_to: usize) -> Vec<(Style, String)> {
    restyle_range(spans, sel_from, sel_to, |style| style.bg(pal.selection))
}

/// One cell of a line marked as the caret of a column selection that has no width.
///
/// The selection's own colour, because that is what it is: a selection one character wide, on
/// every line the block covers. The blank is there for the common case — a block standing at the
/// end of its lines, where the column the next keystroke writes at has no character under it yet
/// and `restyle_range` would have nothing to colour.
fn block_caret_mark(pal: Palette, spans: Vec<(Style, String)>, col: usize) -> Vec<(Style, String)> {
    let mut spans = spans;
    let width: usize = spans.iter().map(|(_, text)| text.chars().count()).sum();
    if col >= width {
        spans.push((Style::default(), " ".repeat(col + 1 - width)));
    }
    restyle_range(spans, col, col + 1, |style| style.bg(pal.selection))
}

pub fn severity_colour(pal: Palette, severity: crate::lsp::Severity) -> Color {
    match severity {
        crate::lsp::Severity::Error => pal.danger,
        crate::lsp::Severity::Warning => pal.warning,
        crate::lsp::Severity::Info => pal.accent,
        crate::lsp::Severity::Hint => pal.text_dim,
    }
}

/// Underlines what the language server marked on this line.
///
/// Colour and an underline, never a background: the selection owns the background, and a server
/// that painted over it would make you lose track of what you had selected while reading about
/// what is wrong with it. Applied before the selection for the same reason.
fn underline_marks(
    pal: Palette,
    spans: Vec<(Style, String)>,
    marks: &[&crate::lsp::Mark],
) -> Vec<(Style, String)> {
    let mut out = spans;
    // Worst first, so where two overlap the more serious colour is the one applied last and
    // survives on the shared columns.
    let mut ordered: Vec<&&crate::lsp::Mark> = marks.iter().collect();
    ordered.sort_by_key(|m| m.severity);
    for mark in ordered {
        let colour = severity_colour(pal, mark.severity);
        out = restyle_range(out, mark.start, mark.end, move |style| {
            style.fg(colour).add_modifier(Modifier::UNDERLINED)
        });
    }
    out
}

fn draw_tab_bar(f: &mut Frame, app: &App, area: Rect, active_position: usize, pane: EditorPane) {
    let pal = app.palette();
    let lang = app.settings.lang;
    let mut spans = Vec::new();
    let strip_width = tab_strip_width(app, area.width);
    let tabs = app.pane_tabs(pane);
    let strip = tab_strip_layout(&tab_widths(app, pane), strip_width, app.tab_offsets[pane.index()]);
    let arrow_style = Style::default().fg(pal.text_muted).bg(pal.tab_inactive);
    if strip.left_arrow.is_some() {
        spans.push(Span::styled(SCROLL_LEFT_GLYPH, arrow_style));
    }
    let mut used = strip.tabs.first().map(|t| t.full.0).unwrap_or(0);
    for (offset, (&editor_idx, layout)) in
        tabs[strip.first.min(tabs.len())..].iter().zip(&strip.tabs).enumerate()
    {
        let Some(editor) = app.editors.get(editor_idx) else { continue };
        let position = strip.first + offset;
        let dirty = if editor.dirty { "*" } else { "" };
        // Drawn into the columns the layout gave this tab rather than into as many as the name
        // wants: the two have to agree, or the × is clicked where it is not and the strip runs
        // off the end of the bar. `fit` cuts with an ellipsis; the padding is for the clipped
        // tab that is the whole strip, so the highlight still fills the row.
        let label_width = layout.label.1.saturating_sub(layout.label.0) as usize;
        let prefix = format!(
            "{:<label_width$}",
            fit(&format!(" {}{} ", editor.title(lang), dirty), label_width)
        );
        used = layout.full.1;
        let style = chrome(pal, if position == active_position {
            Style::default().fg(pal.on_accent).bg(pal.accent)
        } else {
            Style::default().fg(pal.text_muted).bg(pal.tab_inactive)
        });
        spans.push(Span::styled(prefix, style));
        spans.push(Span::styled("\u{2715}", style));
        spans.push(Span::styled(" ", style));
    }
    if let Some((start, _)) = strip.right_arrow {
        let pad = start.saturating_sub(used);
        if pad > 0 {
            spans.push(Span::raw(" ".repeat(pad as usize)));
        }
        spans.push(Span::styled(SCROLL_RIGHT_GLYPH, arrow_style));
        used = start + ARROW_W;
    }
    let (target_range, run_range) = toolbar_button_ranges(app, area.width);
    if let Some((start, _)) = target_range.or(run_range) {
        let pad = start.saturating_sub(used);
        if pad > 0 {
            spans.push(Span::raw(" ".repeat(pad as usize)));
        }
    }
    if target_range.is_some() {
        let label = run_target_button_label(app, app.pane_editor_index(pane));
        spans.push(Span::styled(label, Style::default().fg(pal.text_muted).bg(pal.tab_inactive)));
    }
    if run_range.is_some() {
        let label = run_button_label(app, app.pane_editor_index(pane));
        spans.push(Span::styled(label, Style::default().fg(pal.on_accent).bg(pal.success)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// One control on a preview's navigation bar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NavControl {
    PageBack,
    PageForward,
    GoToPage,
    ZoomOut,
    ZoomIn,
    FitPage,
    FitWidth,
    Invert,
    /// Markdown only: the rendered document, or the styled text.
    TextMode,
    /// The four that move a live figure. On a 2-D plot they slide the window, on a 3-D one they
    /// turn it — the same four, because that is what the keys do and a button that did something
    /// else would be a second thing to learn.
    ///
    /// Buttons rather than a line of text saying which keys to press. A picture with a bar under
    /// it is a thing people click, and the arrows are the controls somebody looking at a plot
    /// reaches for first.
    FigLeft,
    FigRight,
    FigUp,
    FigDown,
    /// Back to the view the figure was drawn with.
    FigReset,
    /// Out of the tab and into a file of its own.
    FigExport,
}

/// The bar's controls with the cells each occupies, left to right.
///
/// One function for the renderer and for hit testing, so a button cannot be drawn where it
/// cannot be clicked. Page controls only appear on something that has pages.
pub fn nav_bar_layout(app: &App, idx: usize, area: Rect) -> Vec<(NavControl, Rect)> {
    let Some(preview) = app.editors.get(idx).and_then(|e| e.preview.as_ref()) else {
        return Vec::new();
    };
    let Some(row) = nav_bar_rect(area) else { return Vec::new() };
    let kind = preview.kind();
    let mut controls = Vec::new();
    // A live figure gets its own controls, first, because they are what it is for. They go to
    // the session, which redraws with new limits — so the axis labels stay true, which is the
    // reason none of this magnifies the pixels the way the zoom beside it does.
    let figure = app.figure_nav_hint(idx).is_some();
    if figure {
        controls.extend([
            NavControl::FigLeft,
            NavControl::FigRight,
            NavControl::FigUp,
            NavControl::FigDown,
            NavControl::FigReset,
            NavControl::FigExport,
        ]);
    }
    // Styled text is scrolled, not paged, and has no pixels to zoom or invert: over it the bar
    // carries the one control that means anything there, the way back to the document.
    if !preview.text_view() {
        if preview.pages.is_some() {
            controls.extend([NavControl::PageBack, NavControl::PageForward, NavControl::GoToPage]);
        }
        controls.extend([
            NavControl::ZoomOut,
            NavControl::ZoomIn,
            NavControl::FitPage,
            NavControl::FitWidth,
        ]);
        controls.push(NavControl::Invert);
    }
    // Only where there is something to switch to: without pandoc the text view is not a choice,
    // it is the only rendering there is.
    if kind == crate::preview::Kind::Markdown && crate::preview::markdown_as_document() {
        controls.push(NavControl::TextMode);
    }

    let mut out = Vec::new();
    let mut x = row.x + 1;
    for control in controls {
        let width = nav_width(control, kind);
        if x + width > row.x + row.width {
            break;
        }
        out.push((control, Rect { x, y: row.y, width, height: 1 }));
        x += width + 1;
    }
    out
}

/// The same buttons, as the cells a click is allowed to land in.
///
/// Wider than what is drawn: the column of space each button is separated from the next by
/// belongs to one of them rather than to nothing, and the two ends of the run reach the edge of
/// the bar. On a row one cell tall, a gap that swallows a click is the difference between a
/// button that feels reliable and one that has to be aimed at — and the pointer is often a
/// trackpad. Nothing moves on screen; only the target grows.
pub fn nav_bar_hit_zones(app: &App, idx: usize, area: Rect) -> Vec<(NavControl, Rect)> {
    hit_zones_from(&nav_bar_layout(app, idx, area))
}

/// The arithmetic of the above, away from the app the buttons came from. Generic over what the
/// buttons are: the markdown formatting bar is the same one-row run of small targets with the
/// same gaps in it, and two copies of this would be two chances to get the ends wrong.
fn hit_zones_from<T: Copy>(drawn: &[(T, Rect)]) -> Vec<(T, Rect)> {
    let mut zones = Vec::with_capacity(drawn.len());
    for (i, (control, rect)) in drawn.iter().enumerate() {
        // Back to the previous button's right edge, forward to the next one's left — and at the
        // ends, out to the bar itself.
        let left = match i {
            0 => rect.x.saturating_sub(1),
            _ => drawn[i - 1].1.x + drawn[i - 1].1.width,
        };
        // The gap goes to the button *after* it, not to both: zones that touch exactly leave no
        // dead column and no column that two buttons could claim.
        let right = rect.x + rect.width;
        zones.push((*control, Rect { x: left, y: rect.y, width: right.saturating_sub(left).max(1), height: 1 }));
    }
    zones
}

/// The row a preview's navigation bar sits on: the last line inside the frame.
fn nav_bar_rect(area: Rect) -> Option<Rect> {
    let inner = inner_rect(area);
    (inner.height >= 2 && inner.width >= 8)
        .then(|| Rect { x: inner.x, y: inner.y + inner.height - 1, width: inner.width, height: 1 })
}

/// A button's name, and the key that does the same thing.
///
/// The two are drawn together, on the button. They used to be apart — buttons on the left, a
/// list of keys on the right — which put the words "go", "fit", "wide" and "dark" on the bar
/// twice, once as a label and once as a reminder of the label.
fn nav_label(control: NavControl, kind: crate::preview::Kind) -> (&'static str, &'static str) {
    match control {
        NavControl::PageBack => ("\u{25c2}", "\u{2190}"),
        NavControl::PageForward => ("\u{25b8}", "\u{2192}"),
        NavControl::GoToPage => ("go", "g"),
        NavControl::ZoomOut => ("\u{2212}", "-"),
        NavControl::ZoomIn => ("+", "+"),
        NavControl::FitPage => ("fit", "f"),
        NavControl::FitWidth => ("wide", "w"),
        // The same operation, named for what it does to the thing in front of you: a page has a
        // dark mode, a photograph has a negative. Calling both "dark" was how a picture came to
        // open inverted because a PDF had been read that way.
        NavControl::Invert if kind == crate::preview::Kind::Picture => ("invert", "i"),
        NavControl::Invert => ("dark", "d"),
        NavControl::TextMode => ("text", "t"),
        // Named by the arrow they answer to. The label is the key, which is the whole of the
        // convention on this bar.
        NavControl::FigLeft => ("\u{25c2}", "\u{2190}"),
        NavControl::FigRight => ("\u{25b8}", "\u{2192}"),
        NavControl::FigUp => ("\u{25b4}", "\u{2191}"),
        NavControl::FigDown => ("\u{25be}", "\u{2193}"),
        NavControl::FigReset => ("reset", "r"),
        NavControl::FigExport => ("save", "e"),
    }
}

/// The narrowest a button is allowed to be, however short its label.
///
/// `+` names itself with its own key and came to three cells — a target one row tall and three
/// columns wide, which is the size of a full stop on a big screen. Reported, in the same breath
/// as the buttons that did nothing, as "davvero piccoli e difficili da cliccare".
const NAV_MIN_WIDTH: u16 = 5;

/// How many cells a button takes: a space, the name, the key, a space. The zoom buttons name
/// themselves with their own key, so it is not written twice.
fn nav_width(control: NavControl, kind: crate::preview::Kind) -> u16 {
    let (name, key) = nav_label(control, kind);
    let natural = if name == key {
        name.chars().count() as u16 + 2
    } else {
        (name.chars().count() + key.chars().count()) as u16 + 3
    };
    natural.max(NAV_MIN_WIDTH)
}

/// The area a preview's picture gets, once the navigation bar has taken its row.
fn preview_image_rect(area: Rect) -> Rect {
    let inner = inner_rect(area);
    match nav_bar_rect(area) {
        Some(_) => Rect { height: inner.height.saturating_sub(1), ..inner },
        None => inner,
    }
}

/// The navigation bar: the controls, then what they are acting on — page, and zoom — pushed to
/// the right so the numbers stay in one place while the buttons stay in another.
fn draw_nav_bar(f: &mut Frame, app: &App, idx: usize, area: Rect) {
    let pal = app.palette();
    let Some(row) = nav_bar_rect(area) else { return };
    let Some(preview) = app.editors.get(idx).and_then(|e| e.preview.as_ref()) else { return };
    let lang = app.settings.lang;
    f.render_widget(
        Paragraph::new(" ".repeat(row.width as usize)).style(Style::default().bg(pal.surface_dim)),
        row,
    );

    // A figure's six buttons do not touch the picture: they ask the session that drew it to
    // redraw. With that session gone — a figure from Run, whose shell ends with the script —
    // they cannot do anything, and drawn like the others they invited a click that answered only
    // in the status line. Dimmed, they say so before the click.
    let live = app.figure_has_a_session(idx);
    for (control, rect) in nav_bar_layout(app, idx, area) {
        // The key that does the same thing is written under the label, so the bar teaches the
        // keyboard rather than competing with it.
        let style = Style::default().fg(pal.text_muted).bg(pal.surface);
        let style = match control {
            NavControl::FigLeft
            | NavControl::FigRight
            | NavControl::FigUp
            | NavControl::FigDown
            | NavControl::FigReset
            | NavControl::FigExport
                if !live =>
            {
                style.fg(pal.text_dim)
            }
            _ => style,
        };
        let style = match control {
            NavControl::FitWidth if preview.fit == crate::preview::Fit::Width => {
                style.fg(pal.on_accent).bg(pal.accent)
            }
            NavControl::FitPage if preview.fit == crate::preview::Fit::Page => {
                style.fg(pal.on_accent).bg(pal.accent)
            }
            NavControl::Invert if preview.inverted => style.fg(pal.on_accent).bg(pal.accent),
            NavControl::TextMode if preview.text_only => style.fg(pal.on_accent).bg(pal.accent),
            _ => style,
        };
        let (name, key) = nav_label(control, preview.kind());
        let dim = Style::default().fg(pal.text_dim).bg(style.bg.unwrap_or(Color::Reset));
        // Whatever the label came to, the button is at least `NAV_MIN_WIDTH` wide — so the
        // padding has to carry the button's own background, or a wider target would read as a
        // narrow button with a hole beside it.
        let natural = if name == key {
            name.chars().count() + 2
        } else {
            name.chars().count() + key.chars().count() + 3
        };
        let pad = (rect.width as usize).saturating_sub(natural);
        let (before, after) = (pad / 2, pad - pad / 2);
        let line = if name == key {
            Line::from(Span::styled(
                format!("{} {name} {}", " ".repeat(before), " ".repeat(after)),
                style,
            ))
        } else {
            Line::from(vec![
                Span::styled(format!("{} {name} ", " ".repeat(before)), style),
                Span::styled(format!("{key} {}", " ".repeat(after)), dim),
            ])
        };
        f.render_widget(Paragraph::new(line), rect);
    }

    // The state and the key hint, right-aligned — but never over the buttons, which are drawn
    // first and own their cells. What does not fit is dropped whole rather than truncated: half
    // a hint reads as a glitch, and the hint is the least important thing on the bar.
    let buttons_end = nav_bar_layout(app, idx, area)
        .last()
        .map(|(_, r)| r.x + r.width)
        .unwrap_or(row.x);
    let free = (row.x + row.width).saturating_sub(buttons_end + 1);

    let mut state = String::new();
    // What the arrows do on a live figure, which the buttons cannot say.
    //
    // They have always worked — arrows pan a 2-D plot and turn a 3-D one, `r` puts it back — and
    // the bar carried no sign of it, so the only way to find out was to press an arrow and see.
    // Somebody reading a bar that offers zoom, fit and invert reasonably concludes that panning
    // and rotating are the things it does not have. And unlike the buttons beside them these
    // keys do not act on the picture: they go to the session, which redraws with new limits, so
    // the numbers on the axes stay true.
    if let Some(hint) = app.figure_nav_hint(idx) {
        state.push_str(&hint);
    }
    // A GIF whose frames were too many to hold. What is on screen is the first of them, and
    // this is the only thing left saying so once the status line has moved on.
    if preview.animation_refused {
        state.push_str(&i18n::label_first_frame(lang));
    }
    if let Some(pages) = &preview.pages {
        state.push_str(&match pages.total {
            Some(total) => i18n::msg_page_of(lang, pages.current, total),
            None => i18n::msg_page(lang, pages.current),
        });
    }
    if (preview.zoom - 1.0).abs() > f32::EPSILON {
        state.push_str(&format!(" {}% ", (preview.zoom * 100.0).round() as i32));
    }
    // No list of keys here any more: each button carries its own, so a list would spell out
    // "fit", "wide" and "dark" a second time beside the buttons already saying them.
    let text = if state.chars().count() <= free as usize { state } else { String::new() };
    if text.is_empty() {
        return;
    }
    let width = text.chars().count() as u16;
    let rect = Rect { x: row.x + row.width - width, width, ..row };
    f.render_widget(Paragraph::new(Span::styled(text, Style::default().fg(pal.text_dim))), rect);
}

// ---- The markdown formatting bar ----------------------------------------------------------

/// One button on the markdown formatting bar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MdTool {
    Bold,
    Italic,
    Strike,
    Code,
    Heading,
    Bullet,
    Numbered,
    Task,
    Link,
    Quote,
    Fence,
}

/// The buttons, left to right, with the group each belongs to. The groups are what the spaces
/// in the run are for: four ways of marking a word, then headings, then the three kinds of list,
/// then the three that make a shape out of a block.
const MD_TOOLS: [(MdTool, u8); 11] = [
    (MdTool::Bold, 0),
    (MdTool::Italic, 0),
    (MdTool::Strike, 0),
    (MdTool::Code, 0),
    (MdTool::Heading, 1),
    (MdTool::Bullet, 2),
    (MdTool::Numbered, 2),
    (MdTool::Task, 2),
    (MdTool::Link, 3),
    (MdTool::Quote, 3),
    (MdTool::Fence, 3),
];

/// A button's name, and the markdown it writes.
///
/// The syntax is the point of the second half: the bar is there to be outgrown, and somebody who
/// has read `**` off the bold button a dozen times can type it and switch the bar off. ASCII
/// throughout — an emoji is one character of two columns on some terminals and one on others,
/// and every position on this row is arithmetic.
fn md_tool_label(tool: MdTool) -> (&'static str, &'static str) {
    match tool {
        MdTool::Bold => ("B", "**"),
        MdTool::Italic => ("I", "*"),
        MdTool::Strike => ("S", "~~"),
        MdTool::Code => ("C", "`"),
        MdTool::Heading => ("H", "#"),
        // Not `*`, which is what the bar's second button already means here, and not `-`, which
        // is the hint beside it.
        MdTool::Bullet => ("Li", "-"),
        // These three name themselves in their own syntax, so nothing is written twice.
        MdTool::Numbered => ("1.", ""),
        MdTool::Task => ("[ ]", ""),
        MdTool::Link => ("Ln", "[]()"),
        MdTool::Quote => (">", ""),
        MdTool::Fence => ("Cb", "```"),
    }
}

/// How many cells a button takes: a space, the name, the syntax, a space — and never fewer than
/// [`NAV_MIN_WIDTH`], for the reason recorded there.
fn md_tool_width(tool: MdTool) -> u16 {
    let (name, hint) = md_tool_label(tool);
    let natural = if hint.is_empty() {
        name.chars().count() as u16 + 2
    } else {
        (name.chars().count() + hint.chars().count()) as u16 + 3
    };
    natural.max(NAV_MIN_WIDTH)
}

/// The bar's buttons with the cells each occupies, left to right.
///
/// One function for the renderer and for hit testing, the same as the preview's bar: a button
/// drawn where it cannot be clicked is the complaint that gave that bar its minimum width. What
/// does not fit is dropped from the right rather than squeezed — a two-cell button is not a
/// button — so a narrow pane keeps the ones nearest to hand.
pub fn md_toolbar_layout(area: Rect) -> Vec<(MdTool, Rect)> {
    if area.height == 0 {
        return Vec::new();
    }
    let right = area.x.saturating_add(area.width);
    let mut out = Vec::new();
    let mut x = area.x.saturating_add(1);
    let mut group = MD_TOOLS[0].1;
    for (tool, tool_group) in MD_TOOLS {
        // An extra column where the groups meet, so the run reads as four sets of buttons.
        if tool_group != group {
            x = x.saturating_add(1);
            group = tool_group;
        }
        let width = md_tool_width(tool);
        if x.saturating_add(width) > right {
            break;
        }
        out.push((tool, Rect { x, y: area.y, width, height: 1 }));
        x = x.saturating_add(width).saturating_add(1);
    }
    out
}

/// The same buttons, as the cells a click is allowed to land in — the gaps included, each going
/// to the button after it. See [`hit_zones_from`].
pub fn md_toolbar_hit_zones(area: Rect) -> Vec<(MdTool, Rect)> {
    hit_zones_from(&md_toolbar_layout(area))
}

/// Whether buffer `idx` is a markdown file that can be typed into.
///
/// The one question both the bar and the eleven actions ask, so a button can never be offered
/// over a buffer the action behind it would refuse. A rendered view and a read-only buffer are
/// markdown you are looking at rather than markdown you are writing.
pub fn md_formattable(app: &App, idx: usize) -> bool {
    crate::preview::is_renderable(&app.editor_ext(idx))
        && app.editors.get(idx).is_some_and(|e| e.preview.is_none() && !e.read_only)
}

/// The shortest pane the bar will appear over: the tab strip, the bar, the content frame's two
/// border rows and two lines of the file under them.
///
/// Below that the bar would be taking the last of the room the text itself needs, and a pane
/// showing one line of a document is not a pane anybody is reading. The setting is untouched —
/// the bar comes back the moment the window does.
const MD_TOOLBAR_MIN_HEIGHT: u16 = 6;

/// Whether the formatting bar is on screen over a pane. The single source of truth: the
/// renderer, the viewport arithmetic and mouse handling all ask this, so none of them can be
/// drawing or clicking a row the others believe is text.
pub fn md_toolbar_visible(app: &App, idx: usize, area: Rect) -> bool {
    md_toolbar_shown(
        app.settings.show_md_toolbar,
        &app.editor_ext(idx),
        app.editors.get(idx).is_some_and(|e| e.preview.is_none() && !e.read_only),
        area,
    )
}

/// The rule of the above, away from the app the four answers are read out of.
fn md_toolbar_shown(wanted: bool, ext: &str, editable: bool, area: Rect) -> bool {
    wanted && editable && crate::preview::is_renderable(ext) && area.height >= MD_TOOLBAR_MIN_HEIGHT
}

/// The formatting bar, drawn the way the preview's navigation bar is: a label, and the syntax it
/// writes in a dimmer colour beside it.
fn draw_md_toolbar(pal: Palette, f: &mut Frame, area: Rect) {
    if area.height == 0 {
        return;
    }
    f.render_widget(
        Paragraph::new(" ".repeat(area.width as usize)).style(Style::default().bg(pal.surface_dim)),
        area,
    );
    for (tool, rect) in md_toolbar_layout(area) {
        let style = Style::default().fg(pal.text_muted).bg(pal.surface);
        let dim = Style::default().fg(pal.text_dim).bg(pal.surface);
        let (name, hint) = md_tool_label(tool);
        // The padding carries the button's own background, or a target wider than its label
        // would read as a narrow button with a hole beside it.
        let natural = if hint.is_empty() {
            name.chars().count() + 2
        } else {
            name.chars().count() + hint.chars().count() + 3
        };
        let pad = (rect.width as usize).saturating_sub(natural);
        let (before, after) = (pad / 2, pad - pad / 2);
        let line = if hint.is_empty() {
            Line::from(Span::styled(format!("{} {name} {}", " ".repeat(before), " ".repeat(after)), style))
        } else {
            Line::from(vec![
                Span::styled(format!("{} {name} ", " ".repeat(before)), style),
                Span::styled(format!("{hint} {}", " ".repeat(after)), dim),
            ])
        };
        f.render_widget(Paragraph::new(line), rect);
    }
}

/// The box a pane's scrollbars ride: its contents, less any row another control has claimed.
pub fn scrollbar_area(app: &App, idx: usize, area: Rect) -> Rect {
    if app.editors.get(idx).is_some_and(|e| e.preview.is_some()) {
        preview_image_rect(area)
    } else {
        inner_rect(area)
    }
}

/// A picture in a tab: drawn by CleeCode itself, straight down the stdout ratatui already
/// writes on, so it reaches the host terminal's graphics protocol where there is one and falls
/// back to coloured half-blocks where there is not.
///
/// While it decodes, and when it cannot be decoded at all, the frame says so in the middle
/// rather than sitting empty — an empty frame is exactly the silence this feature exists to
/// replace.
fn draw_preview_pane(
    f: &mut Frame,
    app: &mut App,
    idx: usize,
    pane: EditorPane,
    content_area: Rect,
    focused: bool,
) {
    let pal = app.palette();
    use crate::preview::State as Preview;
    let lang = app.settings.lang;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(focused_border_style(pal, focused, app.layout_resize_active()));
    f.render_widget(block, content_area);
    let inner = preview_image_rect(content_area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    draw_nav_bar(f, app, idx, content_area);
    // A zoomed page is larger than its pane, so it gets the same bars the text does — and they
    // are the one thing that shows there is more page off to the side.
    for axis in [Axis::Vertical, Axis::Horizontal] {
        let id = crate::app::ScrollbarId::Editor(pane, axis);
        let engaged = app.scrollbar_engaged(id, content_area, axis);
        if let Some((total, position, viewport)) = app.preview_scroll_view(idx, axis) {
            if engaged || app.editors[idx].scrolled_within(SCROLLBAR_LINGER) {
                draw_scrollbar(pal, f, scrollbar_area(app, idx, content_area), axis, total, position, viewport, engaged);
            }
        }
    }

    let centred = |f: &mut Frame, text: String, colour: Color| {
        let width = (text.chars().count() as u16).min(inner.width);
        let rect = Rect {
            x: inner.x + inner.width.saturating_sub(width) / 2,
            y: inner.y + inner.height / 2,
            width,
            height: 1,
        };
        f.render_widget(Paragraph::new(Span::styled(text, Style::default().fg(colour))), rect);
    };

    // Read before the buffer is borrowed mutably for its preview state.
    let top_line = app.editors[idx].top_line;
    let is_document = app.editors[idx].preview.as_ref().is_some_and(|p| p.pages.is_some());
    // The next render needs a number of pixels, and this is the only place that knows how wide
    // the pane actually is. Recorded every frame, so a resize is picked up by the render after.
    if let Some(preview) = app.editors[idx].preview.as_mut() {
        preview.area_cols = inner.width;
        preview.area_rows = inner.height;
    }
    match app.editors[idx].preview.as_mut().map(|p| &mut p.state) {
        Some(Preview::Loading) => centred(f, i18n::msg_preview_loading(lang), pal.text_dim),
        Some(Preview::Failed(reason)) => {
            let text = i18n::msg_preview_failed(lang, &reason.clone());
            centred(f, text, pal.danger);
        }
        Some(Preview::Ready(protocol)) => {
            // The filter is chosen per kind, because the two want opposite things.
            //
            // A document is text: `Nearest` makes every stroke either survive whole or vanish,
            // which is most of why pages looked gritty, so it is worth a better filter. A
            // photograph is not: on a 4K picture CatmullRom measured 41ms against Nearest's 7,
            // for a difference nobody looks for — and that cost lands between asking for the
            // picture and seeing it.
            let filter = if is_document {
                ratatui_image::FilterType::CatmullRom
            } else {
                ratatui_image::FilterType::Triangle
            };
            let widget = StatefulImage::default().resize(ratatui_image::Resize::Fit(Some(filter)));
            f.render_stateful_widget(widget, inner, protocol.as_mut());
        }
        Some(Preview::Rendered { lines, .. }) => {
            // Wrapped here rather than when the lines were made: they are logical lines, and
            // this is the only place that knows how wide the pane happens to be right now.
            //
            // Only the lines that can reach the pane are handed over, because handing over all
            // of them means copying every styled span of the whole document, every frame, to
            // draw a screenful. How many wrapped rows a logical line becomes is not known until
            // the paragraph lays it out, so the slice is deliberately generous: four rows of
            // wrapping per line is far more than prose does, and taking too many costs a copy
            // nobody sees while taking too few would clip the bottom of the page.
            let scroll = top_line.min(lines.len().saturating_sub(1));
            let reach = (inner.height as usize).saturating_mul(4).max(1);
            let visible: Vec<Line> = lines.iter().skip(scroll).take(reach).cloned().collect();
            f.render_widget(Paragraph::new(visible).wrap(Wrap { trim: false }), inner);
        }
        None => {}
    }
}

fn draw_editor(f: &mut Frame, app: &mut App, area: Rect) {
    let panes = editor_pane_rects(area, app.split_view, app.settings.split_pct);
    if panes.len() == 1 {
        let focused = app.focus == Focus::Editor;
        draw_editor_pane(f, app, panes[0], app.active_editor, focused, EditorPane::Left);
        return;
    }
    let left_focused = app.focus == Focus::Editor && app.editor_pane_focus == EditorPane::Left;
    let right_focused = app.focus == Focus::Editor && app.editor_pane_focus == EditorPane::Right;
    // Both panes carry both buttons: each describes the file in its own pane, so on the right
    // one they are not the duplicate the venv-only selector would have been.
    draw_editor_pane(f, app, panes[0], app.active_editor, left_focused, EditorPane::Left);
    draw_editor_pane(f, app, panes[1], app.active_editor_right, right_focused, EditorPane::Right);
}

/// An editor frame with nothing open in it.
///
/// The state you get by closing your last tab. It takes the whole frame, tab strip included:
/// a strip with no tabs on it is a bar of nothing, and the frame is already saying that.
fn draw_no_file_open(f: &mut Frame, app: &App, area: Rect, focused: bool) {
    let pal = app.palette();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(focused_border_style(pal, focused, app.layout_resize_active()));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let lines: Vec<Line> = i18n::msg_no_file_open(app.settings.lang)
        .iter()
        .map(|text| {
            Line::from(Span::styled(*text, Style::default().fg(pal.text_dim)))
                .alignment(ratatui::layout::Alignment::Center)
        })
        .collect();
    let height = lines.len() as u16;
    // A frame too short for the hint says nothing rather than drawing over its own border.
    if inner.height < height {
        return;
    }
    let y = inner.y + (inner.height - height) / 2;
    f.render_widget(Paragraph::new(lines), Rect { y, height, ..inner });
}

fn draw_editor_pane(f: &mut Frame, app: &mut App, area: Rect, idx: usize, focused: bool, pane: EditorPane) {
    let pal = app.palette();
    // Asked of the strip rather than of `idx`, which is a buffer number and stays 0 whether or
    // not there is a buffer 0 to be had.
    if app.pane_tabs(pane).is_empty() {
        draw_no_file_open(f, app, area, focused);
        return;
    }
    let (tab_bar_area, toolbar_area, content_area) = pane_areas(app, idx, area);
    if let Some(toolbar_area) = toolbar_area {
        draw_md_toolbar(pal, f, toolbar_area);
    }
    if tab_bar_area.height > 0 {
        // Only acts when the active tab changed; a manual scroll survives untouched.
        app.reveal_active_tab(pane, tab_bar_area.width);
        // The strip is addressed by position within this pane's own tabs; `idx` is the buffer.
        let position = app.pane_tab_position(pane);
        draw_tab_bar(f, app, tab_bar_area, position, pane);
    }

    // No title here: the open tab right above already shows the filename and dirty marker.
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(focused_border_style(pal, focused, app.layout_resize_active()));

    // A picture takes the whole frame and none of the text machinery: no gutter, no wrapping,
    // no syntax, and no scrollbars, because there is nothing to scroll through.
    if app.editors[idx].preview.is_some() {
        draw_preview_pane(f, app, idx, pane, content_area, focused);
        return;
    }

    let inner = block.inner(content_area);
    let total_lines = app.editors[idx].rope.len_lines();
    let gutter = gutter_width(total_lines, app.settings.show_line_numbers);
    let text_width = inner.width.saturating_sub(gutter) as usize;
    let viewport_height = inner.height as usize;
    if focused {
        app.editor_viewport = (viewport_height, text_width);
    }
    app.editors[idx].follow_cursor(viewport_height, if app.settings.word_wrap { 0 } else { text_width });

    // Taken once, and cloned: the renderer holds the editors mutably while it draws, and the
    // marks live on the app beside them.
    let marks: Vec<crate::lsp::Mark> = app.marks_for(app.editors[idx].path.as_deref()).to_vec();
    let breaks: Vec<usize> = app
        .breakpoints_in(app.editors[idx].path.as_deref())
        .map(|lines| lines.iter().copied().collect())
        .unwrap_or_default();
    // The line the session is stopped on, if it is stopped in this file.
    let stopped = app
        .stopped_at
        .as_ref()
        .filter(|(path, _)| Some(path.as_path()) == app.editors[idx].path.as_deref())
        .map(|(_, line)| line.saturating_sub(1));

    let top_line = app.editors[idx].top_line;
    let left_col = app.editors[idx].left_col;
    let cursor_line = app.editors[idx].cursor_line;
    let visible_rows = app.editors[idx].visible_rows_from(top_line, viewport_height);
    let cursor_row = visible_rows.iter().position(|&l| l == cursor_line).unwrap_or(0);

    // Only as far down as this frame can see, plus a screen for the scroll that follows.
    //
    // A line's colours depend on every line above it, so a buffer is coloured from the top down
    // and an edit invalidates what follows it — but only what follows it, and only as far as
    // anyone is looking. Colouring the whole file instead made a keystroke cost a full copy of
    // the buffer plus a parse of every line in it, so typing in a long file got slower the
    // longer the file was.
    //
    // A buffer in the declared large-file mode is not coloured at all, whatever the setting
    // says. Even bounded to the viewport the ladder colours from the top of the file down, so
    // the first scroll into the middle of a 50 MB file parses everything above it and keeps a
    // styled span vector per line — a copy of the file several times over, built while the
    // editor looks like it has hung. The mode says so on the status bar instead.
    let colour = app.settings.syntax_highlighting && !app.editors[idx].is_large();
    if colour {
        let through = visible_rows.last().copied().unwrap_or(0) + viewport_height;
        let highlighter = &app.highlighter;
        app.editors[idx].refresh_highlight(highlighter, through);
    } else {
        app.editors[idx].forget_highlight();
    }

    let mut lines: Vec<Line> = Vec::new();
    // Only filled in wrap mode, where it is the raw material for the caret's row.
    let mut wrapped_widths: Vec<usize> = Vec::new();
    for line_idx in visible_rows.iter().copied() {
        let mut spans: Vec<Span> = Vec::new();
        let on_line: Vec<&crate::lsp::Mark> = marks.iter().filter(|m| m.line == line_idx).collect();
        let worst = on_line.iter().map(|m| m.severity).max();
        if gutter > 0 {
            let is_current = line_idx == cursor_line;
            // The number carries the mark rather than a column of its own: a gutter one cell
            // wider would move every cursor position, every mouse mapping and every viewport
            // width that is worked out from it, which is a lot of arithmetic to disturb for a
            // dot. A red line number is not ambiguous.
            // A breakpoint takes the gutter before anything else does. It is the one mark in
            // there the user put on purpose, and a warning colouring over it would read as the
            // breakpoint having gone away.
            let at_break = breaks.contains(&(line_idx + 1));
            // A line somebody else wrote while you were looking at the file, from the last
            // reload. It comes after the diagnostic on purpose: a diagnostic is information —
            // something is wrong here — and an arrival is only evidence that something moved.
            // Where the two land on one line the reader needs the one that says to act.
            let arrived = app.editors[idx].line_arrived(line_idx);
            let num_style = if at_break {
                Style::default().fg(pal.on_accent).bg(pal.danger).add_modifier(Modifier::BOLD)
            } else if stopped == Some(line_idx) {
                Style::default().fg(pal.on_accent).bg(pal.warning).add_modifier(Modifier::BOLD)
            } else {
                match (worst, arrived, is_current) {
                (Some(severity), _, current) => {
                    let style = Style::default().fg(severity_colour(pal, severity));
                    if current { style.add_modifier(Modifier::BOLD) } else { style }
                }
                (None, true, current) => {
                    let style = Style::default().fg(pal.changed_line);
                    if current { style.add_modifier(Modifier::BOLD) } else { style }
                }
                (None, false, true) => Style::default().fg(pal.warning).add_modifier(Modifier::BOLD),
                (None, false, false) => Style::default().fg(pal.text_dim),
                }
            };
            let num_text = format!("{:>width$} ", line_idx + 1, width = (gutter as usize).saturating_sub(1));
            spans.push(Span::styled(num_text, num_style));
        }
        if app.editors[idx].folds.iter().any(|&(s, _)| s == line_idx) {
            spans.push(Span::styled("▸ ", Style::default().fg(pal.accent)));
        }

        let raw_spans: Vec<(Style, String)> = if colour {
            app.editors[idx].highlighted.get(line_idx).cloned().unwrap_or_default()
        } else {
            let mut text = app.editors[idx].rope.line(line_idx).to_string();
            if text.ends_with('\n') {
                text.pop();
            }
            vec![(Style::default(), text)]
        };
        let raw_spans = if app.settings.show_whitespace {
            apply_whitespace_marks(raw_spans)
        } else {
            raw_spans
        };
        let raw_spans = if on_line.is_empty() { raw_spans } else { underline_marks(pal, raw_spans, &on_line) };
        // The whole stopped line, marked: where the program *is* is worth more than a colour on
        // one word, and it is what you look for when you glance back at the editor.
        let raw_spans = match stopped == Some(line_idx) {
            true => restyle_range(raw_spans, 0, usize::MAX, |style| style.bg(pal.current_line)),
            false => raw_spans,
        };
        // The editor decides the shape — a run of text or a rectangle — so the highlight always
        // matches what a copy would take.
        let raw_spans = match app.editors[idx].selected_columns(line_idx) {
            Some((from, to)) => highlight_selection(pal, raw_spans, from, to),
            None => raw_spans,
        };
        // The column of carets a rectangle with no width leaves standing: what typing in a block
        // produces, and what says the next key will write on all of these lines at once. There is
        // one terminal cursor and it can only be in one place, so this is the only thing telling
        // the user their keystroke is about to happen eight times.
        let raw_spans = match app.editors[idx].block_caret(line_idx) {
            Some(col) => block_caret_mark(pal, raw_spans, col),
            None => raw_spans,
        };

        if app.settings.word_wrap {
            let mut width = 0usize;
            for (style, text) in raw_spans {
                width += text.chars().count();
                spans.push(Span::styled(text, style));
            }
            // Kept so the caret can be put on the row the text actually landed on: how many
            // rows a line takes is only knowable from how long it is, and that is known here.
            wrapped_widths.push(width);
        } else {
            spans.extend(clip_line_spans(&raw_spans, left_col, text_width.max(1)));
        }
        lines.push(Line::from(spans));
    }

    let mut paragraph = Paragraph::new(lines).block(block);
    if app.settings.word_wrap {
        paragraph = paragraph.wrap(Wrap { trim: false });
    }
    f.render_widget(paragraph, content_area);

    // After the paragraph, so the bars sit over the frame it drew rather than under it.
    app.editors[idx].observe_scroll();
    draw_editor_scrollbars(pal, f, app, idx, pane, content_area, viewport_height, text_width);

    if focused {
        let cursor_col = app.editors[idx].cursor_col;
        let cell = if app.settings.word_wrap {
            wrapped_cursor_offset(&wrapped_widths, cursor_row, cursor_col, text_width, viewport_height)
        } else {
            Some((cursor_col.saturating_sub(left_col) as u16, cursor_row as u16))
        };
        // Nothing is drawn when the caret's row falls off the bottom. Under-scrolling in wrap
        // mode is `follow_cursor`'s business and it counts logical lines, so this can happen;
        // a caret parked on the wrong line would say the edit is going somewhere it is not,
        // and no caret at least says "you cannot see where you are".
        if let Some((dx, dy)) = cell {
            let cursor_x = inner.x + gutter + dx;
            let cursor_y = inner.y + dy;
            f.set_cursor_position((cursor_x, cursor_y));
            // Where the completion popup hangs from. Recorded here because this is the only
            // place that knows which screen cell a buffer position ended up in — folds,
            // scrolling and the gutter all sit between the two.
            app.completion_anchor = (cursor_x, cursor_y);
        }
    }
}

/// Which cell of the viewport the caret is in once `Paragraph` has wrapped the lines above it,
/// as an offset from the text area's top-left corner. `None` when that cell is below the
/// viewport.
///
/// In wrap mode a line longer than the pane occupies several rows, so the caret's logical row is
/// not its screen row: every wrapped line above it pushes the caret one row further down, and
/// the caret's own column restarts from the left edge on each continuation row. Counting logical
/// rows instead — which is what the unwrapped path does, correctly — drew the caret above where
/// the text it belongs to had ended up.
///
/// Two approximations remain, both deliberate. Rows are counted as `ceil(width / text_width)`,
/// which is where a line of solid text breaks; `Wrap { trim: false }` breaks at word boundaries
/// and so can break earlier, putting the caret a row high inside a paragraph of long words. And
/// the gutter rides the first row of each line rather than every one of them, so with line
/// numbers on a continuation row is a few columns out. Both are far smaller errors than the
/// whole-line one they replace, and closing them means re-implementing the wrapper here.
fn wrapped_cursor_offset(
    line_widths: &[usize],
    cursor_row: usize,
    cursor_col: usize,
    text_width: usize,
    viewport_height: usize,
) -> Option<(u16, u16)> {
    let width = text_width.max(1);
    // `max(1)`: an empty line still occupies a row of its own.
    let above: usize = line_widths.iter().take(cursor_row).map(|w| w.div_ceil(width).max(1)).sum();
    let y = above + cursor_col / width;
    (y < viewport_height).then(|| ((cursor_col % width) as u16, y as u16))
}

/// The rows and text columns a pane's buffer gets, worked out from the pane's own rectangle.
///
/// Pure, and used by both the renderer and mouse handling, so the two agree on what a scrollbar
/// is describing without either having to remember what the other did — the same reason the tab
/// strip's layout is a function rather than a stored rect.
pub fn editor_viewport(app: &App, idx: usize, pane_rect: Rect) -> (Rect, usize, usize) {
    let (_, _, content_area) = pane_areas(app, idx, pane_rect);
    let inner = inner_rect(content_area);
    let gutter = gutter_width(app.editors[idx].rope.len_lines(), app.settings.show_line_numbers);
    (content_area, inner.height as usize, inner.width.saturating_sub(gutter) as usize)
}

/// What a pane's scrollbar on `axis` describes: the whole content, where the view sits in it,
/// and how much of it is on screen. `None` when it all fits, so there is no bar and a click on
/// that border belongs to whatever lies behind it.
pub fn editor_scroll_metrics(
    app: &App,
    idx: usize,
    axis: Axis,
    viewport_height: usize,
    text_width: usize,
) -> Option<(usize, usize, usize)> {
    let editor = &app.editors[idx];
    // The lines actually on screen, not the rows available to them: a collapsed fold hides
    // lines without freeing rows, so counting rows would claim more of the file is in view
    // than is.
    let visible = editor.visible_rows_from(editor.top_line, viewport_height);
    let (total, position, viewport) = match axis {
        Axis::Vertical => (editor.rope.len_lines(), editor.top_line, visible.len()),
        // Wrapped text has no sideways travel at all, so there is nothing a horizontal bar
        // could say about it.
        Axis::Horizontal if app.settings.word_wrap => return None,
        Axis::Horizontal => {
            // Measured across the lines on screen rather than the whole file: the width of a
            // buffer is one long scan, repeated every frame, and what the bar answers — "is
            // there more to the right of this?" — is about the text in front of you anyway.
            let widest = visible.into_iter().map(|l| editor.line_char_len(l)).max().unwrap_or(0);
            (widest, editor.left_col, text_width)
        }
    };
    (total > viewport).then_some((total, position, viewport))
}

/// The editor's scrollbars, drawn over the frame's own borders so the text keeps every column
/// and nothing reflows when they appear. They fade out once the view settles — a hint about
/// where you are while you move, not permanent furniture — but come back the moment the pointer
/// is on them, which is when they have to be aimable.
#[allow(clippy::too_many_arguments)]
fn draw_editor_scrollbars(
    pal: Palette,
    f: &mut Frame,
    app: &App,
    idx: usize,
    pane: EditorPane,
    area: Rect,
    viewport_height: usize,
    text_width: usize,
) {
    for axis in [Axis::Vertical, Axis::Horizontal] {
        let engaged = app.scrollbar_engaged(crate::app::ScrollbarId::Editor(pane, axis), area, axis);
        if !engaged && !app.editors[idx].scrolled_within(SCROLLBAR_LINGER) {
            continue;
        }
        let Some((total, position, viewport)) =
            editor_scroll_metrics(app, idx, axis, viewport_height, text_width)
        else {
            continue;
        };
        draw_scrollbar(pal, f, scrollbar_area(app, idx, area), axis, total, position, viewport, engaged);
    }
}

fn vt100_color(color: vt100::Color) -> Option<Color> {
    match color {
        vt100::Color::Default => None,
        vt100::Color::Idx(i) => Some(Color::Indexed(i)),
        vt100::Color::Rgb(r, g, b) => Some(Color::Rgb(r, g, b)),
    }
}

/// The cell holding a terminal panel's close button — the right-aligned `✕` on its top border,
/// one column in from the corner. Shared by the renderer and click handling so both agree on
/// where it is. `None` when the panel is too narrow to carry a title.
pub fn terminal_close_cell(area: Rect) -> Option<(u16, u16)> {
    if area.width < 3 {
        return None;
    }
    Some((area.x + area.width - 2, area.y))
}

/// The area a terminal window's active tab renders into — the whole pane interior. The tab strip
/// rides the top border (see `terminal_tab_strip_rect`), so it costs no interior row and the
/// content is simply the interior, keeping selection hit-testing aligned with what is drawn.
pub fn terminal_content_rect(area: Rect) -> Rect {
    inner_rect(area)
}

/// How long a scrollbar stays up after the last scroll before fading out again. Long enough to
/// still be there when a hand leaves the trackpad and goes for it: at 1.2s it was gone by the
/// time anyone tried to grab the thing they had just been watching.
const SCROLLBAR_LINGER: Duration = Duration::from_millis(2500);

/// Which way a scrollbar runs. The two differ only in which border they live on and which
/// coordinate they measure along, so everything else about them is shared.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    Vertical,
    Horizontal,
}

/// The strip a scrollbar occupies: the last column of the frame's *contents* for a vertical
/// bar, the last row for a horizontal one — inside the frame, not on its border.
///
/// The border was the obvious place, since a bar drawn there costs the content nothing. It was
/// also the wrong one: a border is already the seam you drag to resize the frame, and two
/// controls sharing a column means aiming at either one is a gamble. Inside, the bar overlays
/// the last column of text while it is showing, the way overlay scrollbars have always worked,
/// and the seam is left alone.
///
/// Takes the box the bars ride rather than the frame around it, because that box is not always
/// the whole interior: a preview keeps its last row for the navigation bar, and a horizontal
/// scrollbar drawn on the frame's own terms landed straight on top of it.
///
/// `None` when there is no room to spare.
pub fn scrollbar_strip(inner: Rect, axis: Axis) -> Option<Rect> {
    match axis {
        Axis::Vertical if inner.width >= 1 && inner.height >= 2 => Some(Rect {
            x: inner.x + inner.width - 1,
            y: inner.y,
            width: 1,
            height: inner.height,
        }),
        // One column short of the right edge, left to the vertical bar: the two would otherwise
        // both claim the inside corner, and a cell that belongs to two controls belongs to
        // neither.
        Axis::Horizontal if inner.width >= 2 && inner.height >= 1 => Some(Rect {
            x: inner.x,
            y: inner.y + inner.height - 1,
            width: inner.width - 1,
            height: 1,
        }),
        _ => None,
    }
}

/// How far from a scrollbar the pointer counts as being on it, for the purpose of showing it.
/// A bar is one cell wide and invisible until reached, so an exact hit meant aiming at something
/// that was not there yet — hard with a mouse and a matter of luck with a trackpad. Approaching
/// is enough to bring it up; the click still has to land on the bar.
const SCROLLBAR_REVEAL: u16 = 3;

/// The band the pointer has to be in for a scrollbar to show itself: the strip it would occupy,
/// grown inwards. Used for revealing only — `scrollbar_at`, which decides what a click hits,
/// keeps to the strip, so the cells beside a bar stay ordinary text.
pub fn scrollbar_reveal_zone(inner: Rect, axis: Axis) -> Option<Rect> {
    let strip = scrollbar_strip(inner, axis)?;
    Some(match axis {
        Axis::Vertical => {
            let width = SCROLLBAR_REVEAL.min(inner.width);
            Rect { x: strip.x + 1 - width, width, ..strip }
        }
        Axis::Horizontal => {
            let height = SCROLLBAR_REVEAL.min(inner.height);
            Rect { y: strip.y + 1 - height, height, ..strip }
        }
    })
}

/// A scrollbar's parts, as drawn. Built in one place so the arrows the renderer paints and the
/// cells a click lands on cannot drift apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ScrollbarLayout {
    /// The arrow at the low end — up, or left — which steps back by one line. Dropped, along
    /// with its twin, on a strip too short to give up two cells and still leave a track worth
    /// aiming at.
    pub back: Option<Rect>,
    pub forward: Option<Rect>,
    /// The groove between the arrows: the thumb rides it, and a click on it jumps there.
    pub track: Rect,
}

/// Arrows cost two of the strip's cells. Below this length the remaining track would be too
/// short to point at, so the whole strip stays track.
const SCROLLBAR_MIN_WITH_ARROWS: u16 = 5;

pub fn scrollbar_layout(strip: Rect, axis: Axis) -> ScrollbarLayout {
    let len = match axis {
        Axis::Vertical => strip.height,
        Axis::Horizontal => strip.width,
    };
    if len < SCROLLBAR_MIN_WITH_ARROWS {
        return ScrollbarLayout { back: None, forward: None, track: strip };
    }
    let (back, forward, track) = match axis {
        Axis::Vertical => (
            Rect { height: 1, ..strip },
            Rect { y: strip.y + len - 1, height: 1, ..strip },
            Rect { y: strip.y + 1, height: len - 2, ..strip },
        ),
        Axis::Horizontal => (
            Rect { width: 1, ..strip },
            Rect { x: strip.x + len - 1, width: 1, ..strip },
            Rect { x: strip.x + 1, width: len - 2, ..strip },
        ),
    };
    ScrollbarLayout { back: Some(back), forward: Some(forward), track }
}

/// Where a position within `total` puts the thumb along a track `len` cells long, and back
/// again. The two are each other's inverse at the ends, which is what makes dragging the thumb
/// to the bottom of the track land on the last line rather than near it.
///
/// The travel is `total - viewport`: the furthest the *top* of the view can go, not the length
/// of the content, or the last screenful would be unreachable.
pub fn scroll_position_from_track(offset: u16, len: u16, total: usize, viewport: usize) -> usize {
    let travel = total.saturating_sub(viewport);
    if len <= 1 {
        return 0;
    }
    // Rounded rather than truncated so the midpoint of the track is the middle of the file.
    let cells = (len - 1) as usize;
    (offset.min(len - 1) as usize * travel + cells / 2) / cells
}

/// Draws a scrollbar on `area`'s border: arrows at the ends, a groove, and the thumb. Nothing
/// is drawn when the content already fits, so a bar never appears with nowhere to go.
///
/// `lit` marks the bar as being pointed at or dragged, which is the moment its click targets
/// have to be legible rather than merely hinted at.
#[allow(clippy::too_many_arguments)]
fn draw_scrollbar(pal: Palette, 
    f: &mut Frame,
    box_: Rect,
    axis: Axis,
    total: usize,
    position: usize,
    viewport: usize,
    lit: bool,
) {
    if total <= viewport {
        return;
    }
    let Some(strip) = scrollbar_strip(box_, axis) else { return };
    let layout = scrollbar_layout(strip, axis);

    // The bar lies over the text, so idle it stays as small as it can be: the thumb alone, a
    // hint about where you are. Pointed at, it becomes a control and earns the groove and the
    // arrows that say where a click would land.
    if lit {
        let (back_glyph, forward_glyph) = match axis {
            Axis::Vertical => ("\u{25b4}", "\u{25be}"),
            Axis::Horizontal => ("\u{25c2}", "\u{25b8}"),
        };
        let arrow_style = Style::default().fg(pal.text_muted);
        if let Some(rect) = layout.back {
            f.render_widget(Paragraph::new(Span::styled(back_glyph, arrow_style)), rect);
        }
        if let Some(rect) = layout.forward {
            f.render_widget(Paragraph::new(Span::styled(forward_glyph, arrow_style)), rect);
        }
    }

    let orientation = match axis {
        Axis::Vertical => ScrollbarOrientation::VerticalRight,
        Axis::Horizontal => ScrollbarOrientation::HorizontalBottom,
    };
    let mut state = ScrollbarState::new(total).position(position).viewport_content_length(viewport);
    // A horizontal bar is one cell tall whatever it draws, and the default thumb is a full
    // block — which reads as a bar as thick as a line of text sitting under the content. An
    // eighth-block sits on the floor of its cell and looks like the thin rule it is meant to be.
    let (thumb_glyph, track_glyph) = match axis {
        Axis::Vertical => ("\u{2588}", "\u{2502}"),
        Axis::Horizontal => ("\u{2581}", "\u{2581}"),
    };
    let mut bar = Scrollbar::new(orientation)
        .thumb_symbol(thumb_glyph)
        // The arrows are drawn above, into cells this widget never sees, so hit testing and
        // painting read the same layout. The thumb always rides the track between them, so it
        // does not jump when the arrows appear.
        .begin_symbol(None)
        .end_symbol(None)
        .thumb_style(Style::default().fg(pal.accent));
    bar = if lit {
        bar.track_symbol(Some(track_glyph)).track_style(Style::default().fg(pal.text_dim))
    } else {
        // Nothing but the thumb: a groove painted over the text would blank a column of it for
        // no gain while nobody is aiming at the bar.
        bar.track_symbol(None)
    };
    f.render_stateful_widget(bar, layout.track, &mut state);
}

/// A terminal's scrollbar. Shown only when output has actually scrolled away, and then only
/// while the history is being moved through — except when the view is parked back in it, which
/// is the one moment the position is worth stating rather than hinting at.
fn draw_terminal_scrollbar(
    pal: Palette,
    f: &mut Frame,
    terminal: &crate::terminal_panel::TerminalPanel,
    area: Rect,
    engaged: bool,
) {
    let Some((total, position, viewport)) = terminal_scroll_metrics(terminal) else { return };
    // Parked back in the history the bar stays up whether or not it is being touched: that is
    // the one moment its position is worth stating rather than hinting at.
    if terminal.scrollback_offset() == 0 && !engaged && !terminal.scrolled_within(SCROLLBAR_LINGER) {
        return;
    }
    draw_scrollbar(pal, f, inner_rect(area), Axis::Vertical, total, position, viewport, engaged);
}

/// What a terminal's scrollbar describes: the held history followed by the live screen, with
/// the view's top sitting `offset` lines back from the end. `None` when nothing has scrolled
/// off yet, so there is no bar and a click on that border belongs to the seam behind it.
pub fn terminal_scroll_metrics(
    terminal: &crate::terminal_panel::TerminalPanel,
) -> Option<(usize, usize, usize)> {
    let held = terminal.scrollback_lines();
    if held == 0 {
        return None;
    }
    let rows = terminal.rows as usize;
    Some((held + rows, held - terminal.scrollback_offset(), rows))
}

/// The stretch of the top border a multi-tab window shows its tabs on: from just inside the left
/// corner, stopping short of the window close button on the right when one is present. Shared by
/// the renderer and click handling.
pub fn terminal_tab_strip_rect(area: Rect, window_close: bool) -> Rect {
    // One cell reserved on the right for the corner; two when the window also carries its own
    // close button, so the tabs never sit under it.
    let reserve = if window_close { 2 } else { 1 };
    Rect {
        x: area.x + 1,
        y: area.y,
        width: area.width.saturating_sub(1 + reserve),
        height: 1,
    }
}

/// One tab in a terminal window's strip: its whole x-range, and the column of its `✕` close
/// glyph (absent only when the strip is too narrow to fit it).
pub struct TermTab {
    pub full: (u16, u16),
    pub close: Option<u16>,
}

/// The display name of each tab in a window: its user-given name, or a default when it hasn't
/// been renamed. A single-tab window is named after its own position in the layout — every
/// window used to call its lone tab "Terminal 1", so two windows carried the same title —
/// while the tabs of a multi-tab window are numbered within it.
pub fn terminal_tab_labels(window: &TerminalWindow, window_index: usize, lang: Lang) -> Vec<String> {
    window
        .tabs
        .iter()
        .enumerate()
        .map(|(i, t)| {
            t.name.clone().unwrap_or_else(|| {
                let n = if window.tabs.len() == 1 { window_index } else { i };
                i18n::terminal_title(lang, n).trim().to_string()
            })
        })
        .collect()
}

/// The tabs laid out along a terminal window's strip, left to right. Each chip is ` {name} ✕ `
/// (a display name plus a close glyph). Shared by the renderer and click handling, and clipped to
/// the strip width. Empty when there's a single tab (no strip is shown).
pub fn terminal_tab_ranges(area: Rect, labels: &[String]) -> Vec<TermTab> {
    if labels.len() <= 1 {
        return Vec::new();
    }
    let mut tabs = Vec::new();
    let end = area.x + area.width;
    let mut x = area.x;
    for label in labels {
        if x >= end {
            break;
        }
        let lw = label.chars().count() as u16;
        let right = (x + lw + 4).min(end); // " name ✕ "
        let close_x = x + lw + 2; // the ✕ sits after "␠name␠"
        let close = (close_x < right).then_some(close_x);
        tabs.push(TermTab { full: (x, right), close });
        x = right;
    }
    tabs
}

/// Draws a terminal window's tab strip. The active tab is green — the terminal accent — so it
/// never reads as an editor tab (those go cyan). Each tab carries a `✕` to close it.
fn draw_terminal_tab_strip(pal: Palette, f: &mut Frame, area: Rect, labels: &[String], active: usize) {
    let tabs = terminal_tab_ranges(area, labels);
    let mut spans: Vec<Span> = Vec::new();
    for (i, tab) in tabs.iter().enumerate() {
        let budget = (tab.full.1 - tab.full.0) as usize;
        let chip: String = format!(" {} ✕ ", labels[i]).chars().take(budget).collect();
        let style = if i == active {
            Style::default().fg(pal.on_accent).bg(pal.success)
        } else {
            Style::default().fg(pal.text_muted).bg(pal.tab_inactive)
        };
        spans.push(Span::styled(chip, style));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Everything [`draw_single_terminal`] needs to know about the app around the window it is
/// drawing.
///
/// Gathered up front because the window itself is borrowed mutably for the whole call, and the
/// drawer's window is not in `app.terminals` at all — so the function cannot be handed an index
/// and go looking. Every field here was a question asked of `app` in the middle of the old body.
struct TerminalChrome {
    pal: Palette,
    lang: Lang,
    /// Whether the layout resize mode is on, which colours the focused border differently.
    resizing: bool,
    focused: bool,
    /// Whether to offer the window ✕ in the corner. False for the drawer: it is closed from the
    /// View menu, and a ✕ there would be a promise to kill the agent.
    closable: bool,
    /// Whether the pointer is on this pane's scrollbar, or dragging it. Resolved by the caller,
    /// which is the half that knows which `ScrollbarId` this pane answers to — `Terminal(i)` for
    /// the panel, `Drawer` for the drawer.
    engaged: bool,
    /// The number a nameless single tab is called by, which is the pane's place on screen.
    number: usize,
}

fn draw_terminals(f: &mut Frame, app: &mut App, term_areas: &[Rect]) {
    let active = app.active_terminal;
    let focus_terminal = app.focus == Focus::Terminal;
    let pal = app.palette();
    let lang = app.settings.lang;
    let resizing = app.layout_resize_active();
    let closable = app.terminals.len() > 1;
    for (i, area) in term_areas.iter().enumerate() {
        let id = crate::app::ScrollbarId::Terminal(i);
        // Asked before the window is borrowed mutably below: it is a question about the app as a
        // whole — where the pointer is, and what is being dragged.
        let engaged = app.scrollbar_engaged(id, *area, Axis::Vertical);
        let chrome = TerminalChrome {
            pal,
            lang,
            resizing,
            focused: focus_terminal && i == active,
            closable,
            engaged,
            number: i,
        };
        let Some(window) = app.terminals.get_mut(i) else { continue };
        draw_single_terminal(f, window, *area, chrome);
    }
}

fn draw_single_terminal(
    f: &mut Frame,
    window: &mut TerminalWindow,
    area: Rect,
    chrome: TerminalChrome,
) {
    let TerminalChrome { pal, lang, resizing, focused, closable, engaged, number } = chrome;
    let labels = terminal_tab_labels(window, number, lang);
    let active_tab = window.active;
    let tab_count = labels.len();
    let window_close = closable;

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(focused_border_style(pal, focused, resizing));
    // With a single tab the top border carries the (possibly renamed) terminal's name; with
    // several, the tabs ride the border instead (drawn below) and stand in for the title.
    if tab_count <= 1 {
        block = block.title(format!(" {} ", labels.first().map(String::as_str).unwrap_or("")));
    }
    // A close button in the top-right corner, but only when there's another terminal to fall back
    // to — the last one can't be closed, so offering the button would only mislead. Its cell is
    // `terminal_close_cell`, kept in step with the right-aligned title here.
    if window_close {
        block = block.title_top(
            Line::from(Span::styled("\u{2715}", Style::default().fg(pal.danger))).right_aligned(),
        );
    }
    // The tab strip rides the top border, so the content is the whole interior.
    let content = terminal_content_rect(area);

    // The border (and close button) first, then the tabs over the top border, then the contents.
    f.render_widget(block, area);
    if tab_count > 1 {
        let strip = terminal_tab_strip_rect(area, window_close);
        draw_terminal_tab_strip(pal, f, strip, &labels, active_tab);
    }

    let rows = content.height;
    let cols = content.width;
    let terminal = window.active_tab_mut();
    terminal.resize(rows, cols);

    // Keep the pane clean during shell startup: hide the banner/rc output until the shell
    // settles, so the user sees an empty pane (then a clean prompt) rather than a banner
    // that only gets cleared seconds later.
    if !terminal.is_ready() {
        if content.height > 0 && content.width > 0 {
            let hint = i18n::terminal_starting(lang);
            let hint_w = (hint.chars().count() as u16).min(content.width);
            let rect = Rect {
                x: content.x + content.width.saturating_sub(hint_w) / 2,
                y: content.y + content.height / 2,
                width: hint_w,
                height: 1,
            };
            f.render_widget(
                Paragraph::new(Span::styled(hint, Style::default().fg(pal.text_dim))),
                rect,
            );
        }
        return;
    }

    // Read before the parser is locked below: the lock is a plain mutex, so asking the panel
    // anything about its scrollback while holding it would deadlock the whole app.
    draw_terminal_scrollbar(pal, f, terminal, area, engaged);

    let selection = terminal.selection;
    let parser = crate::terminal_panel::lock_poisoned(&terminal.parser);
    let screen = parser.screen();

    let lines = terminal_lines(screen, selection);

    let cursor_pos = if focused && !screen.hide_cursor() {
        Some(screen.cursor_position())
    } else {
        None
    };

    // The border was already drawn; the terminal grid fills the content area below the strip.
    f.render_widget(Paragraph::new(lines), content);

    if let Some((cy, cx)) = cursor_pos {
        f.set_cursor_position((content.x + cx, content.y + cy));
    }
}

/// The agent drawer's column: the agent's pane when one is running, the launcher when the drawer
/// is open with nobody in it.
///
/// Drawn straight after the terminal panel and before every modal, because it is a frame of the
/// layout rather than something over the top of one — that is what pin mode means.
pub fn draw_drawer(f: &mut Frame, app: &mut App, area: Rect) {
    let pal = app.palette();
    let lang = app.settings.lang;
    let resizing = app.layout_resize_active();
    let focused = app.focus == Focus::Drawer;
    // Asked before the drawer is borrowed mutably, for the same reason `draw_terminals` asks it
    // there: it is a question about the pointer, which belongs to the app.
    let engaged = app.scrollbar_engaged(crate::app::ScrollbarId::Drawer, area, Axis::Vertical);
    let Some(drawer) = app.drawer.as_mut() else { return };
    match drawer.window.as_mut() {
        Some(window) => draw_single_terminal(
            f,
            window,
            area,
            TerminalChrome {
                pal,
                lang,
                resizing,
                focused,
                // The ✕ in the corner, in the cell every other pane keeps it in. It reads as the
                // terminal panel's close button and does something quieter: it takes the column
                // away and leaves the pty running, which is the View menu's own path and the
                // only thing closing the drawer has ever meant. `App::click_drawer` claims that
                // cell before anything in the frame can, so the resemblance never becomes a
                // route into `close_terminal`.
                closable: true,
                engaged,
                // Never read: the drawer's one tab is always named after its agent.
                number: 0,
            },
        ),
        None => draw_drawer_launcher(f, pal, lang, area, drawer.selected, focused, resizing),
    }
}

/// Where each agent's entry sits in the launcher, and whether there is room for the marks.
///
/// One function, used by the drawing and by the mouse alike, because the two must not be able to
/// disagree: a click that starts the agent above the one under the pointer is precisely the bug a
/// second copy of this arithmetic produces. The rects returned are one per agent, in
/// [`Agent::all`] order, each covering that agent's whole block.
///
/// **Three rungs, and they are given up in that order.** A mark in its selection frame with two
/// blank rows before the next is the whole thing — the marks carry their names in bricks, so
/// there is no caption under them to lose; the blank rows go first, one at a time, because
/// spacing is the cheapest thing on screen to lose and the marks are what the panel is for; the
/// marks go last, leaving four captions — the rung where the names come back as text, because
/// there is nowhere else for them to be. An empty list means the column is too short even for
/// those, in which case nothing is drawn and nothing can be clicked.
pub fn drawer_launcher_rows(inner: Rect) -> (bool, Vec<Rect>) {
    let count = crate::session::Agent::all().len() as u16;
    // Four columns beyond the widest mark: the selection frame's border and breathing room,
    // two a side.
    let room = crate::drawer::widest_art() + 4 <= inner.width;
    // A mark inside its selection frame: border, the mark's rows, border.
    let tall = crate::drawer::ART_ROWS as u16 + 2;
    let fits = |per: u16, gap: u16| count * per + count.saturating_sub(1) * gap <= inner.height;
    let big = room && (fits(tall, 2) || fits(tall, 1) || fits(tall, 0));
    let per = if big { tall } else { 1 };
    let gap = if fits(per, 2) { 2 } else { u16::from(fits(per, 1)) };
    if !fits(per, gap) {
        return (big, Vec::new());
    }
    let total = count * per + count.saturating_sub(1) * gap;
    let top = inner.y + (inner.height - total) / 2;
    let rows = (0..count)
        .map(|i| Rect { x: inner.x, y: top + i * (per + gap), width: inner.width, height: per })
        .collect();
    (big, rows)
}

/// One row of a drawn mark, as spans.
///
/// Runs of one style are merged rather than emitted a span per cell: a mark is a couple of dozen
/// cells wide and four of them are drawn every frame, and ratatui allocates per span.
///
/// `installed` is the only thing the panel is allowed to say about a mark. The colours themselves
/// are the owners' and are fixed — the same rule the file tree's icons run under — so a name that
/// is not here is drawn *dim* rather than recoloured: still recognisably that program's mark,
/// visibly not something you can start. The phrase under its banner carries the rest of the
/// answer, in words.
fn art_spans(cells: &[crate::drawer::ArtCell], installed: bool) -> Vec<Span<'static>> {
    let paint = |ink: Option<crate::drawer::Ink>| ink.map(|(r, g, b)| Color::Rgb(r, g, b));
    let style = |cell: &crate::drawer::ArtCell| {
        let mut style = Style::default();
        if let Some(fg) = paint(cell.fg) {
            style = style.fg(fg);
        }
        if let Some(bg) = paint(cell.bg) {
            style = style.bg(bg);
        }
        if !installed {
            style = style.add_modifier(Modifier::DIM);
        }
        style
    };
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut current: Option<Style> = None;
    for cell in cells {
        let want = style(cell);
        if current != Some(want) {
            if let Some(had) = current.take() {
                spans.push(Span::styled(std::mem::take(&mut run), had));
            }
            current = Some(want);
        }
        run.push(cell.ch);
    }
    if let Some(had) = current {
        spans.push(Span::styled(run, had));
    }
    spans
}

/// The empty state: the four agents, written large, one of them highlighted.
///
/// The ROADMAP's answer to "which agent does the key summon" — the empty state *is* the
/// selector, so the question never has to be settled in a setting. Every one of the four is
/// shown whether or not it is installed, because the empty drawer is also where you find out
/// what CleeCode knows how to run; the ones that are not here are drawn dim and said to be
/// missing rather than quietly left out.
fn draw_drawer_launcher(
    f: &mut Frame,
    pal: Palette,
    lang: Lang,
    area: Rect,
    selected: usize,
    focused: bool,
    resizing: bool,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(focused_border_style(pal, focused, resizing))
        .title(format!(" {} ", i18n::t(lang, Key::DrawerTitle)))
        // The same ✕, in the same cell, as the drawer wears with an agent in it: the empty
        // drawer is as dismissable as the full one, and a control that came and went with the
        // contents would be one to hunt for. `terminal_close_cell` is where it is.
        .title_top(
            Line::from(Span::styled("\u{2715}", Style::default().fg(pal.danger))).right_aligned(),
        );
    let inner = block.inner(area);
    f.render_widget(block, area);

    let (big, rows) = drawer_launcher_rows(inner);
    // The block — frame, mark, name in bricks — is centred in the column, and every entry is
    // indented the same amount so the four keep one left edge. Only with the marks: the bare
    // captions are a list, and a list reads from the margin.
    let frame_width = (crate::drawer::widest_art() + 4).min(inner.width);
    let frame_x = inner.x + inner.width.saturating_sub(frame_width) / 2;
    for (i, agent) in crate::session::Agent::all().into_iter().enumerate() {
        let Some(rect) = rows.get(i).copied() else { continue };
        let name = agent.workspace_name();
        // Asked of the drawer's own probe and not of `Agent::on_path`: the answer decides whether
        // a click starts the agent or offers to install it, so it has to be able to change while
        // this panel is up. See `drawer::installed`.
        let installed = crate::drawer::installed(agent);
        if big {
            // The chosen one wears a frame, which is the whole answer to "what am I about to
            // start": the marks carry their names, so the highlight's only job is to be seen,
            // and a ring around a banner is seen from further away than a marker beside it.
            if i == selected {
                let frame = Rect { x: frame_x, width: frame_width, ..rect };
                f.render_widget(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(pal.accent)),
                    frame,
                );
            }
            let lines: Vec<Line> = crate::drawer::art(agent)
                .into_iter()
                .map(|cells| Line::from(art_spans(&cells, installed)))
                .collect();
            let mark = Rect {
                x: frame_x + 2,
                y: rect.y + 1,
                width: frame_width.saturating_sub(4),
                height: crate::drawer::ART_ROWS as u16,
            };
            f.render_widget(Paragraph::new(lines), mark);
            // No caption: the name is in the mark. What is *not* in the mark is the honest
            // phrase, so an agent that is not here says so under its banner — on the frame's
            // bottom row, where it reads as the frame's own label when the missing one is also
            // the chosen one.
            if !installed {
                let below = Rect { x: frame_x, y: rect.y + rect.height - 1, width: frame_width, height: 1 };
                f.render_widget(
                    Paragraph::new(
                        Line::from(Span::styled(
                            format!(" {} ", i18n::t(lang, Key::DrawerNotInstalled)),
                            Style::default().fg(pal.text_dim),
                        ))
                        .alignment(ratatui::layout::Alignment::Center),
                    )
                    .style(Style::reset()),
                    below,
                );
            }
            continue;
        }
        // The caption rung: no room for the marks, so the names carry everything — the ▸ for
        // the choice, the dim colour and the phrase for the one that is not here.
        let colour = if !installed {
            pal.text_dim
        } else if i == selected {
            pal.accent
        } else {
            pal.text
        };
        let mut style = Style::default().fg(colour);
        if i == selected {
            style = style.add_modifier(Modifier::BOLD);
        }
        let head = if i == selected { "▸ " } else { "  " };
        let marker = Style::default().fg(pal.accent);
        let mut caption = vec![Span::styled(head, marker), Span::styled(name, style)];
        if !installed {
            caption.push(Span::styled(
                format!("  {}", i18n::t(lang, Key::DrawerNotInstalled)),
                Style::default().fg(pal.text_dim),
            ));
        }
        f.render_widget(Paragraph::new(Line::from(caption)), rect);
    }

    // The two keys, on the bottom row, and only where there is a row to spare below the list.
    let hint = i18n::t(lang, Key::DrawerHint);
    let last = rows.last().map(|r| r.y + r.height).unwrap_or(inner.y);
    if !rows.is_empty() && last + 1 < inner.y + inner.height {
        let row = Rect { y: inner.y + inner.height - 1, height: 1, ..inner };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(hint, Style::default().fg(pal.text_dim)))
                .alignment(ratatui::layout::Alignment::Center)),
            row,
        );
    }
}

/// A parser's screen as rows of styled text, ready to be handed to ratatui.
///
/// Split out from the drawing so it can be checked against a real parser: what it gets wrong is
/// invisible in the geometry and only shows up in the characters that come out.
///
/// The wide-character rule is the one that was missing. vt100 stores a two-column glyph in two
/// cells — the glyph in the first, a marked continuation in the second — and that second cell
/// reports no contents, which used to make it a space. So a CJK glyph or an emoji was drawn as
/// two columns of glyph plus one of padding, and everything to its right on that row sat one
/// column further along than the program had put it: a line of Japanese drifted steadily off the
/// end, and any box drawing next to one came apart. ratatui measures the glyph as the two columns
/// it is, so dropping the continuation is all that is needed for the row to add up again.
fn terminal_lines(screen: &vt100::Screen, selection: Option<TermSelection>) -> Vec<Line<'static>> {
    let (screen_rows, screen_cols) = screen.size();
    let mut lines: Vec<Line> = Vec::new();
    for row in 0..screen_rows {
        let mut spans: Vec<Span> = Vec::new();
        let mut current = String::new();
        let mut current_style = Style::default();
        let mut have_style = false;

        for col in 0..screen_cols {
            let cell = screen.cell(row, col);
            if cell.is_some_and(vt100::Cell::is_wide_continuation) {
                continue;
            }
            let (contents, style) = match cell {
                Some(c) if c.has_contents() => {
                    let mut style = Style::default();
                    if let Some(fg) = vt100_color(c.fgcolor()) {
                        style = style.fg(fg);
                    }
                    if let Some(bg) = vt100_color(c.bgcolor()) {
                        style = style.bg(bg);
                    }
                    if c.bold() {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    if c.italic() {
                        style = style.add_modifier(Modifier::ITALIC);
                    }
                    if c.underline() {
                        style = style.add_modifier(Modifier::UNDERLINED);
                    }
                    if c.inverse() {
                        style = style.add_modifier(Modifier::REVERSED);
                    }
                    (c.contents().to_string(), style)
                }
                _ => (" ".to_string(), Style::default()),
            };
            // Selected cells get the editor's selection colours rather than a reverse-video
            // flip, which would be invisible on text that is already inverse (a prompt, a
            // highlighted match). Differing styles also break the span run here, so the
            // highlight lands on exactly the selected cells.
            let style = match selection {
                Some(selection) if selection.contains(row, col) => {
                    Style::default().fg(Color::Black).bg(Color::LightBlue)
                }
                _ => style,
            };

            if have_style && style == current_style {
                current.push_str(&contents);
            } else {
                if !current.is_empty() {
                    spans.push(Span::styled(current.clone(), current_style));
                }
                current = contents;
                current_style = style;
                have_style = true;
            }
        }
        if !current.is_empty() {
            spans.push(Span::styled(current, current_style));
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// A colour as a style. One line, and it exists so the diagnostic and the hover go through the
/// same drawing instead of two nearly identical branches.
fn colour_of(colour: Color) -> Style {
    Style::default().fg(colour)
}

/// The worst diagnostic on the line the cursor is on, if any.
///
/// The worst rather than the first: two servers' worth of hints can sit on a line that also has
/// the error you are actually looking for, and the error is the one the row is for.
fn diagnostic_under_cursor(app: &App) -> Option<(String, crate::lsp::Severity)> {
    let editor = app.editor();
    let line = editor.cursor_line;
    app.marks_for(editor.path.as_deref())
        .iter()
        .filter(|m| m.line == line)
        .max_by_key(|m| m.severity)
        .map(|m| (m.message.clone(), m.severity))
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let pal = app.palette();
    let msg = if app.resize_mode {
        i18n::t(app.settings.lang, Key::ResizeModeHint).to_string()
    } else {
        app.status_message.clone()
    };
    let style = if app.resize_mode {
        Style::default().fg(pal.on_accent).bg(pal.warning)
    } else {
        Style::default().fg(pal.text_muted)
    };
    let paragraph = Paragraph::new(Line::from(Span::raw(msg.clone()))).style(style);
    f.render_widget(paragraph, area);

    // What the server says about the line the cursor is on, right-aligned so it shares the row
    // with the status message rather than replacing it. Neither wins: a diagnostic is about
    // where you are and stays as long as you are there, while "Saved" is about what you just
    // did — hiding either behind the other would be answering a question nobody asked.
    // The room left after the status message, minus a two-cell gap so the two never touch. Below
    // about a word's worth there is nothing useful to say, and a truncated diagnostic that says
    // "cann…" is worse than the squiggle already on screen.
    let room = (area.width as usize).saturating_sub(msg.chars().count() + 2);
    // A diagnostic wins that space over what the thing under the cursor *is*, and it is not
    // close. An error on this line is news and the type of a name is not — while there is
    // something wrong with it, the type is very likely the reason. So the hover fills the same
    // spot only when the line is clean, and in a colour that says it is not a complaint.
    let said = diagnostic_under_cursor(app)
        .map(|(text, severity)| (text, severity_colour(pal, severity)))
        .or_else(|| app.what_it_is().map(|text| (text.to_string(), pal.text_dim)))
        .filter(|_| !app.resize_mode && room >= 8)
        .map(|(text, colour)| match text.chars().count() > room {
            true => (text.chars().take(room - 1).collect::<String>() + "…", colour),
            false => (text, colour),
        });

    // Where the caret is, hard against the right edge. Numbers only: every editor puts them
    // there, they need no translating, and the pair is read at a glance rather than parsed —
    // which is why it earns its four or five columns even on a narrow window.
    //
    // Counted from one, the way the file's own gutter counts and the way Go-to-line asks; the
    // buffer counts from zero and nothing outside it should have to know that.
    //
    // Last in the queue, though: it goes only into what the message and the diagnostic have left,
    // because a position can be worked out by looking at the gutter and an error cannot be worked
    // out at all.
    let spent = msg.chars().count() + 2 + said.as_ref().map_or(0, |(t, _)| t.chars().count() + 2);
    let position = (!app.editors.is_empty() && !app.resize_mode)
        .then(|| {
            let editor = app.editor();
            format!("{}:{}", editor.cursor_line + 1, editor.cursor_col + 1)
        })
        .filter(|p| spent + p.chars().count() <= area.width as usize);
    let taken = position.as_ref().map_or(0, |p| p.chars().count() + 2) as u16;

    // What the buffer on disk would be saved as: always "UTF-8" — a buffer that could not be
    // decoded as UTF-8 opens read-only rather than being edited (`Editor::open`), so a writable
    // buffer's encoding is never in question, and naming it honestly costs nothing extra here.
    // Naming it *wrong* would cost a charset-detection dependency this editor doesn't have.
    // Then the line ending, which the Edit menu's "Convert line endings" flips.
    //
    // Shown beside the position it is a fact about the same way, but it is the least important
    // of the four things on this line: nobody opens a file to check its line endings, they
    // notice this only when it's the answer to something odd. So it is the first to go on a
    // narrow window — checked against `spent + taken`, i.e. after the message, the diagnostic
    // and the position have already claimed their room.
    //
    // A buffer in the declared large-file mode adds one word to the same chip. The sentence
    // that announced the mode on open is a status message, and the next thing you do takes the
    // status line back — after which nothing on screen would say why this file has no colours.
    // A word that stays is what makes the mode declared rather than merely mentioned.
    //
    // Appended in a second step, and only if it also fits, so the encoding and the line ending
    // are not dragged off a narrow bar by a word that is worth less than they are.
    let fits = |width: usize| spent + taken as usize + width + 2 <= area.width as usize;
    let chip = (!app.resize_mode)
        .then(|| app.editor())
        .filter(|ed| ed.path.is_some() && ed.preview.is_none() && !ed.read_only)
        .map(|ed| {
            let base = match ed.line_ending {
                crate::editor::LineEnding::Crlf => "UTF-8 CRLF".to_string(),
                crate::editor::LineEnding::Lf => "UTF-8 LF".to_string(),
            };
            ed.is_large()
                .then(|| format!("{base} · {}", i18n::t(app.settings.lang, Key::StatusLargeFile)))
                .filter(|wide| fits(wide.chars().count()))
                .unwrap_or(base)
        })
        .filter(|c| fits(c.chars().count()));
    let chip_taken = chip.as_ref().map_or(0, |c| c.chars().count() + 2) as u16;

    if let Some(text) = position {
        let width = text.chars().count() as u16;
        let spot = Rect { x: area.right() - width, y: area.y, width, height: 1 };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(text, colour_of(pal.text_dim)))),
            spot,
        );
    }
    if let Some(text) = chip {
        let width = text.chars().count() as u16;
        let spot = Rect { x: area.right() - width - taken, y: area.y, width, height: 1 };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(text, colour_of(pal.text_dim)))),
            spot,
        );
    }
    if let Some((text, colour)) = said {
        let width = text.chars().count() as u16;
        let spot = Rect { x: area.right() - width - taken - chip_taken, y: area.y, width, height: 1 };
        f.render_widget(Paragraph::new(Line::from(Span::styled(text, colour_of(colour)))), spot);
    }

    // The easter egg walks along the status line, over whatever message is there. It is the one
    // place on screen already given over to things that come and go, so nothing is lost behind
    // it, and you can carry on working while it crosses.
    if let Some(col) = app.turtle_at(area.width) {
        let spot = Rect { x: area.x + col, y: area.y, width: 2.min(area.width - col), height: 1 };
        f.render_widget(Paragraph::new(Line::from(Span::raw("🐢"))), spot);
    }
}

#[cfg(test)]
mod tests {

    /// The bar is one row tall, so every column of it has to belong to a button: the single
    /// space drawn between two of them used to belong to neither, and a click that landed there
    /// did nothing at all — which, on a row of small targets, is most of what "the buttons do
    /// not work" feels like.
    #[test]
    fn the_click_zones_leave_no_dead_column_between_buttons() {
        // Built from the geometry alone: what matters is that consecutive zones touch, whatever
        // the buttons happen to be.
        let drawn = [
            (NavControl::ZoomOut, Rect { x: 5, y: 9, width: 5, height: 1 }),
            (NavControl::ZoomIn, Rect { x: 11, y: 9, width: 5, height: 1 }),
            (NavControl::FitPage, Rect { x: 17, y: 9, width: 6, height: 1 }),
        ];
        let zones = hit_zones_from(&drawn);
        assert_eq!(zones.len(), drawn.len());
        for pair in zones.windows(2) {
            assert_eq!(
                pair[0].1.x + pair[0].1.width,
                pair[1].1.x,
                "a column between two buttons belongs to neither"
            );
        }
        // Each zone still contains the button it is for, and the first reaches back over the
        // space in front of it.
        for (i, (control, zone)) in zones.iter().enumerate() {
            assert_eq!(*control, drawn[i].0);
            assert!(zone.x <= drawn[i].1.x && zone.x + zone.width >= drawn[i].1.x + drawn[i].1.width);
        }
        assert_eq!(zones[0].1.x, drawn[0].1.x - 1);
    }

    /// No button is ever three cells wide, whatever its label says.
    #[test]
    fn even_the_shortest_button_is_worth_aiming_at() {
        for kind in [crate::preview::Kind::Picture, crate::preview::Kind::Document] {
            for control in [NavControl::ZoomIn, NavControl::ZoomOut, NavControl::FigLeft] {
                assert!(nav_width(control, kind) >= NAV_MIN_WIDTH, "{:?}", nav_width(control, kind));
            }
        }
    }
    use super::*;

    /// Five tabs of 10 columns each.
    const W: [u16; 5] = [10, 10, 10, 10, 10];

    /// An 80×24 screen, the size every terminal still agrees on.
    const SCREEN: Rect = Rect { x: 0, y: 0, width: 80, height: 24 };

    /// The three built-in workspaces cannot be saved over and cannot be deleted — the delete
    /// list is files, and they are not files. What was missing was any way to see that before
    /// trying: in the list they looked like something you had made yourself. They are coloured
    /// now, and only there — the same name in the command palette or a file list is an ordinary
    /// row and must stay one.
    #[test]
    fn the_built_in_workspaces_are_marked_as_the_apps_own() {
        use crate::picker::PickerKind;
        let pal = crate::theme::Theme::CleeCode.palette();
        let colour = |kind, name| picker_row_style(pal, kind, name, false).fg;
        let mine = colour(PickerKind::Workspaces, "my layout");
        assert_ne!(colour(PickerKind::Workspaces, "octave"), mine);
        assert_ne!(colour(PickerKind::Workspaces, "pylab"), mine);
        assert_ne!(colour(PickerKind::Workspaces, crate::workspace::DEFAULT_NAME), mine);
        // A file of the user's own that happens to be named like one is impossible — `save_in`
        // refuses the name — so a row named "octave" in that list is always the built-in.
        assert_eq!(colour(PickerKind::Files, "octave"), mine, "only the workspace list marks them");
        // Selected is selected, whatever it is: one highlight, not two competing ones.
        assert_eq!(
            picker_row_style(pal, PickerKind::Workspaces, "octave", true),
            picker_row_style(pal, PickerKind::Workspaces, "my layout", true)
        );
    }

    #[test]
    fn the_completion_list_hangs_under_the_cursor_and_lines_up_with_the_word() {
        // Cursor at column 20 having typed four letters: the word starts at 16, and the text
        // column of the box — past the border and the two-cell marker — must land there.
        let rect = completion_rect((20, 5), 4, 20, 6, SCREEN);
        assert_eq!(rect.x + 3, 16);
        assert_eq!(rect.y, 6, "the row under the cursor");
        assert_eq!(rect.height, 8, "six rows plus the border");
    }

    /// The tabs used to be reachable only from the keyboard, because the drawing knew where they
    /// were and nothing else did. This is the check that the two now agree: the click is asked
    /// where each tab is, and the answer is compared against what was actually painted.
    #[test]
    fn a_click_lands_on_the_git_tab_that_was_drawn_there() {
        use crate::app::GitTab;
        let lang = i18n::Lang::En;
        let header = Rect { x: 5, y: 2, width: 60, height: 1 };
        for slot in git_tab_slots(lang) {
            let left = header.x + slot.x;
            let right = left + slot.width - 1;
            assert_eq!(git_tab_at(lang, header, left), Some(slot.tab), "left edge of {:?}", slot.tab);
            assert_eq!(git_tab_at(lang, header, right), Some(slot.tab), "right edge of {:?}", slot.tab);
        }
        // The single space between two tabs belongs to neither.
        let first = &git_tab_slots(lang)[0];
        assert_eq!(git_tab_at(lang, header, header.x + first.width), None);
        // Nothing to the left of the first tab, and nothing past the last.
        let last = git_tab_slots(lang).pop().unwrap();
        assert_eq!(git_tab_at(lang, header, header.x + last.x + last.width), None);
        assert_eq!(GitTab::ALL.len(), git_tab_slots(lang).len());
    }

    #[test]
    fn a_git_tab_clipped_by_a_narrow_panel_cannot_be_clicked() {
        // Drawing stops at the panel edge, so the hit-test has to stop there too — otherwise the
        // click works on a tab nobody can see.
        let lang = i18n::Lang::En;
        let narrow = Rect { x: 0, y: 0, width: 12, height: 1 };
        let last = git_tab_slots(lang).pop().unwrap();
        assert!(last.x + last.width > narrow.width, "the fixture must actually clip a tab");
        assert_eq!(git_tab_at(lang, narrow, last.x), None);
    }

    /// Rendered into a buffer and read back, because everything above this only checks where the
    /// box goes — not that the words ever reach the screen.
    /// The regression: a two-column glyph took three columns on screen, so everything to its
    /// right on the row sat one place further along than the program had put it — a line of
    /// Japanese drifted off the end, and a box drawn beside one came apart.
    ///
    /// Driven through the real parser, because the bug is in what vt100 stores: a wide glyph
    /// occupies two cells, and the second reports no contents, which is indistinguishable from a
    /// blank unless `is_wide_continuation` is asked.
    #[test]
    fn a_wide_glyph_takes_the_two_columns_it_has_and_not_three() {
        let mut parser = vt100::Parser::new(2, 10, 0);
        parser.process("日本ab".as_bytes());

        let lines = terminal_lines(parser.screen(), None);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with("日本ab"), "the row reads wrong: {text:?}");
        // Four glyphs and the four blank columns left over, not six and four: the continuations
        // are gone rather than standing in as spaces.
        assert_eq!(text.chars().count(), 4 + 4, "the row is {text:?}");
        // And the row is exactly as wide as the screen it came from, which is the thing that was
        // actually broken — ratatui measures the wide glyph as the two columns it occupies.
        assert_eq!(lines[0].width(), 10);

        // A row of nothing but wide glyphs is the same claim at the limit.
        let mut full = vt100::Parser::new(2, 10, 0);
        full.process("日本語テキ".as_bytes());
        assert_eq!(terminal_lines(full.screen(), None)[0].width(), 10);
    }

    /// The About box is drawn at whatever size the terminal has room for, so the sizes have to
    /// agree with the thresholds that pick them: one column too many and the box is clipped by
    /// `centered_rect`, which takes the close hint off the bottom of it.
    /// The two buttons at the right-hand end share a row with the menu titles and with each
    /// other. Overlapping either way is the same bug twice: a button drawn over a title is a
    /// button nobody can see and a title nobody can click.
    /// The initial is split into a span of its own only for the themes that colour it. For the
    /// rest the row stays one span, because three spans that are all the same colour is three
    /// times the work to draw the same line.
    #[test]
    fn only_a_theme_with_an_accelerator_splits_the_initial_off() {
        let plain = accelerated_line(crate::theme::Theme::CleeCode.palette(), "File ");
        assert_eq!(plain.spans.len(), 1, "the default theme should draw one span");
        assert_eq!(plain.to_string(), " File ");

        let turbo = accelerated_line(crate::theme::Theme::Turbo.palette(), "File ");
        assert_eq!(turbo.spans.len(), 3, "the initial should have a span of its own");
        assert_eq!(turbo.spans[1].content, "F");
        assert_eq!(turbo.spans[1].style.fg, crate::theme::Theme::Turbo.palette().accelerator);
        // The row still reads the same: colouring a letter must not move it.
        assert_eq!(turbo.to_string(), " File ");
    }

    #[test]
    fn the_two_bar_buttons_never_overlap_or_land_on_a_title() {
        for width in 0..=200u16 {
            for titles in 0..=width {
                for badge in [0u16, 12] {
                    let background = button_range(width, titles, badge);
                    let themes = theme_range(background.start, titles);
                    if themes.is_empty() {
                        continue;
                    }
                    assert!(
                        themes.end <= background.start,
                        "the buttons overlap at {width}x{titles}: {themes:?} into {background:?}"
                    );
                    assert!(
                        themes.start >= titles,
                        "the theme button sits on a title at {width}x{titles}: {themes:?}"
                    );
                    assert!(!background.is_empty(), "a theme button with no background button");
                }
            }
        }
    }

    /// The background button is the one worth keeping longest: it is the way back from a screen
    /// that cannot be read, and the themes are reachable from the list either way. So when the
    /// bar runs out of room the theme button goes first.
    #[test]
    fn the_theme_button_gives_up_its_room_first() {
        // A bar with exactly one button's worth of space to the right of the titles.
        let width = 40;
        let titles = width - columns(BACKGROUND_BUTTON[0]);
        let background = button_range(width, titles, 0);
        assert!(!background.is_empty(), "the background button should still fit");
        assert!(theme_range(background.start, titles).is_empty(), "the theme button should not");
    }

    #[test]
    fn the_about_box_fits_the_terminal_it_is_drawn_in() {
        for width in 20..=200u16 {
            for height in 6..=60u16 {
                let full = Rect { x: 0, y: 0, width, height };
                let rect = about_modal_rect(full);
                if let Some(art) = about_art(full) {
                    assert!(
                        rect.width <= width && rect.height <= height,
                        "{width}x{height} was offered a drawing it cannot hold: {rect:?}"
                    );
                    assert_eq!(
                        rect.height,
                        about_art_height(art) + 2,
                        "the drawing is cut short"
                    );
                }
            }
        }
    }

    /// Both languages have to fit the column beside the drawing: the text is wrapped to it by
    /// hand, and a line too long for it is not wrapped again by the paragraph, it is cut off.
    #[test]
    fn the_about_text_fits_the_column_beside_the_drawing() {
        for art in [ABOUT_ART_WIDE, ABOUT_ART_NARROW] {
            for lang in [Lang::En, Lang::It] {
                let lines = about_text_lines(lang, ABOUT_TEXT_COLS as usize);
                for line in &lines {
                    assert!(
                        line.width() <= ABOUT_TEXT_COLS as usize,
                        "{lang:?} overflows the column: {:?}",
                        line.to_string()
                    );
                }
                assert!(
                    lines.len() <= about_art_height(art) as usize,
                    "{lang:?} is taller than the drawing it sits beside"
                );
            }
        }
    }

    /// Every row of the bitmap is padded to the same width, because the column the text starts at
    /// is measured from the widest row and a short row would leave the text hanging off it. The
    /// row count has to be even as well: they are read two at a time, and an odd one left over
    /// would be read past the end of the array.
    #[test]
    fn the_about_drawing_is_a_rectangle() {
        for art in [ABOUT_ART_WIDE, ABOUT_ART_NARROW] {
            let width = about_art_width(art);
            assert_eq!(art.len() % 2, 0, "an odd number of pixel rows: {}", art.len());
            for row in art {
                assert_eq!(row.chars().count() as u16, width, "ragged row: {row:?}");
                assert!(
                    row.chars().all(|p| p == '.' || about_ink(p).is_some()),
                    "a pixel with no colour behind it: {row:?}"
                );
            }
        }
    }

    #[test]
    fn the_completion_list_draws_the_words_it_was_given() {
        let pal = crate::theme::Theme::CleeCode.palette();
        use crate::complete::{Candidate, Popup, Source};
        let cands = vec![
            Candidate { text: "config_path".into(), source: Source::Buffer, distance: 0, freq: 1 },
            Candidate { text: "const".into(), source: Source::Keyword, distance: 9, freq: 1 },
        ];
        let popup = Popup::open(0, 0, "con".into(), cands).unwrap();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 12)).unwrap();
        terminal.draw(|f| draw_completion(pal, f, &popup, (10, 2), f.area())).unwrap();

        let buffer = terminal.backend().buffer();
        let screen: Vec<String> = (0..12)
            .map(|y| (0..40).map(|x| buffer[(x, y)].symbol()).collect::<String>())
            .collect();
        let text = screen.join("\n");
        assert!(text.contains("config_path"), "the buffer word is missing:\n{text}");
        assert!(text.contains("const"), "the keyword is missing:\n{text}");
        // The best match is picked, and the marker says so.
        assert!(text.contains("▶ config_path"), "nothing is marked as selected:\n{text}");
        // Under the cursor's row, not over it.
        assert!(screen[0].trim().is_empty() && screen[1].trim().is_empty());
        assert!(!screen[3].trim().is_empty());
    }

    #[test]
    fn the_completion_list_flips_above_the_cursor_near_the_bottom() {
        // Row 20 of 24, with an eight-row box: there is no room below, and plenty above.
        let rect = completion_rect((20, 20), 4, 20, 6, SCREEN);
        assert_eq!(rect.y, 12);
        assert_eq!(rect.y + rect.height, 20, "it stops at the cursor's own row");
    }

    #[test]
    fn the_completion_list_stays_on_screen_at_either_edge() {
        // Far right: slid left until it fits, rather than spilling off.
        let right = completion_rect((78, 5), 2, 30, 4, SCREEN);
        assert!(right.right() <= SCREEN.right(), "{right:?} runs off the right edge");
        // Far left: the box would start before column 0, so it starts at 0.
        let left = completion_rect((1, 5), 1, 20, 4, SCREEN);
        assert_eq!(left.x, 0);
        // A cursor on the top row with no room either way: the top of the screen, not off it.
        let squeezed = completion_rect((10, 0), 2, 20, 20, Rect { x: 0, y: 0, width: 80, height: 8 });
        assert_eq!(squeezed.y, 0);
    }

    /// A split editor in a window dragged very narrow used to panic — `clamp(1, 0)` — and close
    /// CleeCode. Too narrow to split now yields the single pane the callers already handle.
    #[test]
    fn a_split_too_narrow_to_make_falls_back_to_one_pane() {
        for width in [0, 1] {
            let area = Rect::new(0, 0, width, 20);
            let panes = editor_pane_rects(area, true, 50);
            assert_eq!(panes.len(), 1, "width {width} cannot hold two panes");
            assert_eq!(panes[0], area);
        }
        // From two columns up the split is real, and neither side is ever zero-wide — including
        // at the percentage extremes, where the rounding lands hardest.
        for width in [2, 3, 80] {
            for pct in [0, 1, 50, 99, 100] {
                let panes = editor_pane_rects(Rect::new(0, 0, width, 20), true, pct);
                assert_eq!(panes.len(), 2, "width {width} pct {pct}");
                assert!(panes[0].width >= 1 && panes[1].width >= 1, "width {width} pct {pct}");
                assert_eq!(panes[0].width + panes[1].width, width, "no column lost to rounding");
            }
        }
    }

    /// The turtle in the logo is one `char` and two columns. Everything on the menu bar is
    /// placed by counting from one end or the other, so measuring it in characters put the
    /// right-hand end of the bar a column left of where it was drawn — and the click ranges
    /// with it, which is why this is measured and not counted.
    #[test]
    fn the_menu_bar_is_measured_in_columns_not_characters() {
        assert_eq!(columns(MENU_LOGO), 4);
        assert!(MENU_LOGO.chars().count() < columns(MENU_LOGO) as usize, "the logo is wide");
        // The button's two faces have to be the same width, or it would shift the badge beside
        // it every time it was pressed.
        assert_eq!(columns(BACKGROUND_BUTTON[0]), columns(BACKGROUND_BUTTON[1]));
        assert_eq!(columns(BACKGROUND_BUTTON[0]), 3);
    }

    /// The button has to be where the click looks for it, which is the same arithmetic run
    /// twice — once to draw it and once to hit-test it. Both go through `button_range`, so this
    /// is where the two agree.
    #[test]
    fn the_background_button_sits_beside_the_workspace_badge() {
        // No badge: hard against the right edge, three columns wide.
        assert_eq!(button_range(80, 40, 0), 77..80);
        // With one: just inside it, never under it.
        assert_eq!(button_range(80, 40, 12), 65..68);

        // Too narrow to clear the menu titles: no button rather than one drawn over a title,
        // which would take a menu away to make room for a switch.
        assert!(button_range(44, 40, 12).is_empty());
        assert!(button_range(3, 40, 0).is_empty());
        // Nothing at all to draw on, and no arithmetic that wraps round.
        assert!(button_range(0, 0, 0).is_empty());
        assert!(button_range(2, 0, 0).is_empty());
    }

    /// The point of doing this to the finished frame: whatever a widget left showing the
    /// terminal through it gets filled in, and whatever a widget coloured is left alone.
    #[test]
    fn the_background_is_painted_only_where_nothing_else_claimed_it() {
        let pal = crate::theme::Theme::CleeCode.palette();
        let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 3, 1));
        buffer[(0, 0)].set_bg(Color::Reset); // as a `Clear`ed modal leaves it
        buffer[(1, 0)].set_bg(Color::Cyan); // a selected menu title
        buffer[(2, 0)].set_bg(Color::Rgb(30, 30, 30)); // the status line
        paint_background(&mut buffer, pal);
        assert_eq!(buffer[(0, 0)].bg, pal.background);
        assert_eq!(buffer[(1, 0)].bg, Color::Cyan);
        assert_eq!(buffer[(2, 0)].bg, Color::Rgb(30, 30, 30));
        // And the colour it fills with must be one a translucent terminal cannot see through:
        // asking for the default background again would paint nothing at all.
        assert!(matches!(pal.background, Color::Rgb(..)));
    }

    #[test]
    fn terminal_panes_split_by_weight() {
        let area = Rect { x: 0, y: 0, width: 100, height: 10 };
        // Equal weights: an even split.
        let even = terminal_panes(area, &[1000, 1000], Direction::Horizontal);
        assert_eq!(even[0].width, 50);
        assert_eq!(even[1].width, 50);
        // 3:1 weights give the first pane three-quarters, contiguously.
        let uneven = terminal_panes(area, &[1500, 500], Direction::Horizontal);
        assert_eq!(uneven[0].width, 75);
        assert_eq!(uneven[1].width, 25);
        assert_eq!(uneven[1].x, uneven[0].x + uneven[0].width);
    }

    /// Hiding the menu bar is a preference about the idle screen, not a promise never to show a
    /// Workspaces, the palette and quick open were all keyboard-only: the list was drawn but no
    /// click reached it. The mapping has to agree with the drawing exactly, including the offset
    /// the list scrolls by once the selection is past the bottom.
    #[test]
    fn a_click_lands_on_the_picker_row_it_looks_like() {
        use crate::picker::{PickAction, PickItem, Picker, PickerKind};
        let items: Vec<PickItem> = (0..40)
            .map(|i| PickItem {
                label: format!("item {i}"),
                shortcut: None,
                action: PickAction::OpenFile("x".into()),
            })
            .collect();
        let mut p = Picker::new("Test", PickerKind::Workspaces, items);
        let full = Rect::new(0, 0, 120, 40);
        let inner = inner_rect(picker_rect(full));

        // The first row under the query line is the first result.
        assert_eq!(picker_row_at(&p, full, inner.x + 2, inner.y + 1), Some(0));
        assert_eq!(picker_row_at(&p, full, inner.x + 2, inner.y + 3), Some(2));
        // The query line itself is not a result, and neither is anything off the modal.
        assert_eq!(picker_row_at(&p, full, inner.x + 2, inner.y), None);
        assert_eq!(picker_row_at(&p, full, 0, 0), None);

        // Once the selection has pushed the list down, the top row is no longer result 0 — the
        // mouse has to follow the same scroll the drawing applies.
        let list_rows = inner.height.saturating_sub(1) as usize;
        p.selected = list_rows + 4;
        let start = p.selected + 1 - list_rows;
        assert_eq!(
            picker_row_at(&p, full, inner.x + 2, inner.y + 1),
            Some(start)
        );
        assert_eq!(
            picker_row_at(&p, full, inner.x + 2, inner.y + list_rows as u16),
            Some(p.selected)
        );

        // A short list has no rows past its end to click on.
        let few = Picker::new(
            "Test",
            PickerKind::Workspaces,
            vec![PickItem {
                label: "only".into(),
                shortcut: None,
                action: PickAction::OpenFile("x".into()),
            }],
        );
        assert_eq!(picker_row_at(&few, full, inner.x + 2, inner.y + 1), Some(0));
        assert_eq!(picker_row_at(&few, full, inner.x + 2, inner.y + 2), None);
    }

    /// The drawer's column comes off the right of the window before anything else is placed, in
    /// both arrangements — which is the one thing that keeps it from fighting the right-docked
    /// terminal panel for the same edge.
    #[test]
    fn the_drawer_takes_its_column_off_the_right_and_the_rest_makes_room() {
        let full = Rect::new(0, 0, 200, 40);
        let params = |drawer_open, terminal_on_right, drawer_pinned| LayoutParams {
            show_sidebar: true,
            show_terminal: true,
            show_menubar: true,
            menu_active: false,
            terminal_weights: vec![crate::terminal_panel::TERMINAL_WEIGHT_DEFAULT],
            sidebar_width: 30,
            terminal_pct: 35,
            terminal_on_right,
            drawer_open,
            drawer_pct: 40,
            drawer_pinned,
        };

        for on_right in [false, true] {
            let closed = compute_layout(full, &params(false, on_right, true));
            let open = compute_layout(full, &params(true, on_right, true));
            let drawer = open.drawer.expect("an open drawer has a column");
            assert!(closed.drawer.is_none(), "and a closed one has none");

            // Rightmost, full height, and the size the percentage asked for.
            assert_eq!(drawer.x + drawer.width, full.width, "flush with the window's right edge");
            // The whole main area: the menu bar's row and the status line, and nothing else.
            assert_eq!(drawer.y, 1);
            assert_eq!(drawer.height, full.height - 2, "the full height of the main area");
            assert_eq!(drawer.width, full.width * 40 / 100);

            // Everything else fits in what is left. This is the assertion that fails if the
            // drawer is carved inside the orientation branches instead of before them: the
            // right-docked terminal would take its 35% of the whole width and overlap.
            assert!(open.editor.x + open.editor.width <= drawer.x, "the editor stops at the seam");
            for rect in open.terminals.iter().flatten() {
                assert!(rect.x + rect.width <= drawer.x, "so does every terminal window");
            }
            assert!(open.editor.width < closed.editor.width, "the editor gave up the room");
            assert!(open.drawer_overlay.is_none(), "a pinned drawer is never an overlay");
        }
    }

    /// The other mode, in both arrangements: the same rectangle, and nothing else moved.
    ///
    /// The point of autocollapse is what it does *not* do. Every rect here has to come out
    /// identical to the closed-drawer layout, because a rect that changed is a pane that was
    /// resized and a pane that was resized is a `SIGWINCH` to the pty inside it — which is the
    /// whole cost the mode exists to avoid. And it has to be the same rectangle the pinned mode
    /// carves, or the drawer would jump sideways when the setting is changed.
    #[test]
    fn an_autocollapsing_drawer_is_painted_over_a_layout_that_never_heard_about_it() {
        let full = Rect::new(0, 0, 200, 40);
        let params = |drawer_open, terminal_on_right, drawer_pinned| LayoutParams {
            show_sidebar: true,
            show_terminal: true,
            show_menubar: true,
            menu_active: false,
            terminal_weights: vec![crate::terminal_panel::TERMINAL_WEIGHT_DEFAULT],
            sidebar_width: 30,
            terminal_pct: 35,
            terminal_on_right,
            drawer_open,
            drawer_pct: 40,
            drawer_pinned,
        };

        for on_right in [false, true] {
            let closed = compute_layout(full, &params(false, on_right, false));
            let over = compute_layout(full, &params(true, on_right, false));
            let pinned = compute_layout(full, &params(true, on_right, true));

            let overlay = over.drawer_overlay.expect("an open unpinned drawer is an overlay");
            assert!(over.drawer.is_none(), "and it is not a column of the layout");
            assert_eq!(
                Some(overlay),
                pinned.drawer,
                "the same rectangle in both modes: the drawer does not move, what is under it does"
            );
            assert_eq!(overlay.x + overlay.width, full.width, "flush with the window's right edge");
            assert_eq!(overlay.y, 1);
            assert_eq!(overlay.height, full.height - 2, "the full height of the main area");

            // Nothing made room. This is the assertion the mode is for.
            assert_eq!(over.editor, closed.editor, "the editor was not resized");
            assert_eq!(over.sidebar, closed.sidebar);
            assert_eq!(over.terminals, closed.terminals, "and neither was any pty's pane");
            assert!(
                over.editor.x + over.editor.width > overlay.x,
                "the frames run on underneath it, which is what makes this a repaint and not a reflow"
            );
        }
    }

    /// The ribbon is the drawer's absence made clickable: one carved column on the same edge,
    /// in both arrangements, and never on screen at the same time as the drawer itself.
    ///
    /// The assertion that matters most is the last one. Under an autocollapsing drawer the column
    /// stays carved — it is covered, not handed back — because the frames underneath must come
    /// out of `compute_layout` identical whether the drawer is up or away, and a column returned
    /// to them on the way in is a `SIGWINCH` to every pty in the window.
    #[test]
    fn the_ribbon_is_one_carved_column_on_the_right_whenever_the_drawer_is_away() {
        let full = Rect::new(0, 0, 200, 40);
        let params = |drawer_open, terminal_on_right, drawer_pinned| LayoutParams {
            show_sidebar: true,
            show_terminal: true,
            show_menubar: true,
            menu_active: false,
            terminal_weights: vec![crate::terminal_panel::TERMINAL_WEIGHT_DEFAULT],
            sidebar_width: 30,
            terminal_pct: 35,
            terminal_on_right,
            drawer_open,
            drawer_pct: 40,
            drawer_pinned,
        };

        for on_right in [false, true] {
            for pinned in [false, true] {
                let closed = compute_layout(full, &params(false, on_right, pinned));
                let ribbon = closed.drawer_ribbon.expect("a closed drawer leaves a ribbon");

                // The same edge the drawer takes, one cell of it, the full height of the main
                // area — so it is there to be reached wherever the pointer is up or down the
                // window.
                assert_eq!(ribbon.width, 1, "one honest column, and only one");
                assert_eq!(ribbon.x + ribbon.width, full.width, "flush with the right edge");
                assert_eq!(ribbon.y, 1, "below the menu bar");
                assert_eq!(ribbon.height, full.height - 2, "and above the status line");
                assert_eq!(drawer_ribbon_rect(&closed), Some(ribbon), "and it is on screen");

                // Carved, not painted: nothing else reaches the column, which is why a click on
                // it cannot be a click on the editor's scrollbar riding the same edge.
                assert!(closed.editor.x + closed.editor.width <= ribbon.x, "the editor stops short");
                for rect in closed.terminals.iter().flatten() {
                    assert!(rect.x + rect.width <= ribbon.x, "so does every terminal window");
                }
            }

            // Pinned and open: the drawer has the edge, and there is no ribbon at all.
            let pinned_open = compute_layout(full, &params(true, on_right, true));
            assert!(pinned_open.drawer_ribbon.is_none(), "the drawer occupies the edge itself");
            assert!(drawer_ribbon_rect(&pinned_open).is_none());

            // Autocollapsed and open: the column is still carved, and still not on screen.
            let closed = compute_layout(full, &params(false, on_right, false));
            let over = compute_layout(full, &params(true, on_right, false));
            assert_eq!(
                over.drawer_ribbon, closed.drawer_ribbon,
                "the column stays carved under the overlay, so nothing underneath is resized"
            );
            assert!(
                drawer_ribbon_rect(&over).is_none(),
                "covered by the drawer it summons: the two are never both on screen"
            );
        }

        // A window with no room to spare keeps its columns for the frames. The chord and the
        // View menu are still the way in there.
        let cramped = compute_layout(Rect::new(0, 0, 6, 10), &params(false, false, true));
        assert!(cramped.drawer_ribbon.is_none());
    }

    /// The handle fits its column, centred, at every height a window can be — and the banding is
    /// what gives way first when there is not enough of it. The block is the last thing standing:
    /// it is the control, and the bands are what is around it.
    #[test]
    fn the_ribbons_handle_is_a_banded_block_in_the_middle_of_the_column_it_was_given() {
        let bands = RIBBON_BANDS as u16;
        for height in [1u16, 2, 3, 5, 9, 12, 20, 28, 40, 200] {
            let rect = Rect::new(9, 1, 1, height);
            let handle = drawer_ribbon_handle(rect);
            let pill = handle.pill;

            assert!(pill.height >= 1, "a column with a row in it carries a handle");
            assert!(pill.height <= RIBBON_PILL_MAX, "and never a longer grip than a grip");
            assert_eq!(pill.x, rect.x, "on the column, and only on it");
            assert_eq!(pill.width, rect.width);

            // Everything is inside the column, and the whole of it is what is centred — the
            // block alone would sit off-centre once the bands are unequal for want of a row.
            assert!(handle.rect.y >= rect.y, "the handle starts inside its column");
            assert!(
                handle.rect.y + handle.rect.height <= rect.y + rect.height,
                "and ends inside it"
            );
            assert_eq!(
                handle.rect.height,
                pill.height + handle.stripe * bands,
                "the handle is the block and its bands and nothing else"
            );
            let above = handle.rect.y - rect.y;
            let below = rect.y + rect.height - (handle.rect.y + handle.rect.height);
            assert!(above.abs_diff(below) <= 1, "the handle sits in the middle of the column");

            // The block sits between the two runs of three, which is what puts it in the middle
            // of the colours rather than at one end of them.
            assert_eq!(
                pill.y,
                handle.rect.y + handle.stripe * (bands / 2),
                "three bands above the block"
            );
            assert_eq!(
                pill.y + pill.height + handle.stripe * (bands / 2),
                handle.rect.y + handle.rect.height,
                "and three below it"
            );

            // The degradation, in order: full bands, thin bands, none — and the clearance at
            // each end is what a banded handle is asked to leave.
            assert!(
                RIBBON_STRIPE_HEIGHTS.contains(&handle.stripe) || handle.stripe == 0,
                "a band is one of the heights offered, or there are no bands"
            );
            if handle.stripe > 0 {
                assert!(
                    handle.rect.height + 2 <= rect.height,
                    "a banded handle leaves a row of edge at each end"
                );
                let taller = RIBBON_STRIPE_HEIGHTS
                    .iter()
                    .find(|s| **s > handle.stripe)
                    .map(|s| pill.height + s * bands + 2);
                assert!(
                    taller.is_none_or(|wanted| wanted > rect.height),
                    "and it is the tallest banding that would fit"
                );
            } else {
                assert!(
                    pill.height + RIBBON_STRIPE_HEIGHTS[RIBBON_STRIPE_HEIGHTS.len() - 1] * bands + 2
                        > rect.height,
                    "the bands are dropped only when even the thin ones do not fit"
                );
            }
        }

        // The two ends of the discipline, said plainly: a tall column gets the whole mark, a
        // short one keeps the grip and loses the colours.
        assert_eq!(drawer_ribbon_handle(Rect::new(0, 1, 1, 28)).stripe, 2);
        assert_eq!(drawer_ribbon_handle(Rect::new(0, 1, 1, 15)).stripe, 1);
        assert_eq!(drawer_ribbon_handle(Rect::new(0, 1, 1, 9)).stripe, 0);
        assert_eq!(drawer_ribbon_handle(Rect::new(0, 0, 1, 0)).pill.height, 0);
    }

    /// The other handle, on the open drawer's own left border — in both modes, since in both the
    /// drawer has a left border and it is the same column either way. And never at the same time
    /// as the one on the window's edge: the two are the drawer being away and the drawer being
    /// here, which is one question with one answer.
    #[test]
    fn an_open_drawer_carries_the_closing_handle_on_its_own_edge() {
        let full = Rect::new(0, 0, 200, 40);
        let params = |drawer_open, terminal_on_right, drawer_pinned| LayoutParams {
            show_sidebar: true,
            show_terminal: true,
            show_menubar: true,
            menu_active: false,
            terminal_weights: vec![crate::terminal_panel::TERMINAL_WEIGHT_DEFAULT],
            sidebar_width: 30,
            terminal_pct: 35,
            terminal_on_right,
            drawer_open,
            drawer_pct: 40,
            drawer_pinned,
        };

        for on_right in [false, true] {
            for pinned in [false, true] {
                let open = compute_layout(full, &params(true, on_right, pinned));
                let drawer = drawer_rect(&open).expect("an open drawer is somewhere");
                let handle = drawer_close_ribbon_rect(&open).expect("and carries its handle");

                assert_eq!(handle.x, drawer.x, "on the border, which is the width seam's column");
                assert_eq!(handle.width, 1, "one column, the same as the one on the far edge");
                assert_eq!(handle.y, drawer.y);
                assert_eq!(handle.height, drawer.height, "the whole edge is the click target");
                assert!(
                    drawer_ribbon_rect(&open).is_none(),
                    "and the way in is not offered while you are already in"
                );

                let closed = compute_layout(full, &params(false, on_right, pinned));
                assert!(
                    drawer_close_ribbon_rect(&closed).is_none(),
                    "nothing to close when there is no drawer on screen"
                );
                assert!(drawer_ribbon_rect(&closed).is_some(), "that edge carries the way in");
            }
        }
    }

    /// The launcher's rows are laid out once and read by both the drawing and the mouse. What
    /// matters is that they never overlap and never leave the frame — a click resolved against a
    /// row drawn somewhere else starts the wrong agent.
    #[test]
    fn the_launcher_rows_stay_inside_the_frame_and_apart_from_each_other() {
        let count = crate::session::Agent::all().len();
        let needed = crate::drawer::widest_art() + 4;
        for width in [12u16, 24, 34, needed - 1, needed, needed + 1, 70] {
            for height in [3u16, 7, 12, 20, 26, 27, 28, 31, 34, 40] {
                let inner = Rect::new(1, 1, width, height);
                let (big, rows) = drawer_launcher_rows(inner);
                if rows.is_empty() {
                    continue;
                }
                assert_eq!(rows.len(), count, "every agent gets a row or none of them does");
                assert!(!big || width >= needed, "the marks need room for the widest of them");
                assert!(
                    !big || rows[0].height == crate::drawer::ART_ROWS as u16 + 2,
                    "a big row is the mark in its selection frame"
                );
                for pair in rows.windows(2) {
                    assert!(
                        pair[0].y + pair[0].height <= pair[1].y,
                        "{width}x{height}: rows must not overlap"
                    );
                }
                let last = rows[count - 1];
                assert!(
                    last.y + last.height <= inner.y + inner.height,
                    "{width}x{height}: the list stays inside the frame"
                );
                assert!(rows[0].y >= inner.y);
            }
        }
    }

    /// The ladder, rung by rung: the blank row between entries is given up before the marks are,
    /// and the marks before the captions. Written as a sweep down the heights, because what has
    /// to hold is that each rung is reached in turn and none is skipped — a column one row short
    /// of the marks falling all the way to bare captions would throw away the panel's whole point
    /// for the sake of a blank line.
    #[test]
    fn the_launcher_gives_up_the_spacing_before_it_gives_up_the_marks() {
        let count = crate::session::Agent::all().len() as u16;
        let tall = crate::drawer::ART_ROWS as u16 + 2;
        let wide = Rect::new(0, 0, crate::drawer::widest_art() + 4, 0);
        let rung = |height: u16| {
            let (big, rows) = drawer_launcher_rows(Rect { height, ..wide });
            if rows.is_empty() {
                return "nothing";
            }
            let spaced = rows.windows(2).all(|p| p[1].y > p[0].y + p[0].height);
            match (big, spaced) {
                (true, true) => "marks, spaced",
                (true, false) => "marks",
                (false, true) => "captions, spaced",
                (false, false) => "captions",
            }
        };
        assert_eq!(rung(count * tall + (count - 1) * 2), "marks, spaced");
        assert_eq!(rung(count * tall + count - 1), "marks, spaced", "one blank row still spaces");
        assert_eq!(rung(count * tall), "marks", "the blank rows go first");
        assert_eq!(rung(count * tall - 1), "captions, spaced", "then the marks");
        assert_eq!(rung(count), "captions");
        assert_eq!(rung(count - 1), "nothing", "and below that it says nothing rather than half");
        // Too narrow for the widest mark in its frame is the caption rung whatever the height:
        // four marks in a column are as wide as the widest, and half a mark is not a mark.
        let narrow = Rect::new(0, 0, crate::drawer::widest_art() + 3, 40);
        assert!(!drawer_launcher_rows(narrow).0);
    }

    /// menu again — and `Ctrl+Shift+B` is documented as reaching the menus while it is hidden.
    /// Without a row to draw in, opening one produced nothing at all.
    #[test]
    fn an_open_menu_gets_its_row_even_when_the_bar_is_hidden() {
        let full = Rect::new(0, 0, 120, 40);
        let params = |show_menubar, menu_active| LayoutParams {
            show_sidebar: true,
            show_terminal: true,
            show_menubar,
            menu_active,
            terminal_weights: vec![crate::terminal_panel::TERMINAL_WEIGHT_DEFAULT],
            sidebar_width: 30,
            terminal_pct: 35,
            terminal_on_right: false,
            drawer_open: false,
            drawer_pct: crate::settings::DRAWER_PCT_DEFAULT,
            drawer_pinned: true,
        };

        assert_eq!(compute_layout(full, &params(false, false)).menu_bar.height, 0, "hidden and idle");
        assert_eq!(compute_layout(full, &params(true, false)).menu_bar.height, 1, "shown");
        assert_eq!(compute_layout(full, &params(false, true)).menu_bar.height, 1, "hidden but open");

        // The row has to come out of the frames below, not off the bottom of the window.
        let opened = compute_layout(full, &params(false, true));
        let closed = compute_layout(full, &params(false, false));
        assert_eq!(opened.status.y, closed.status.y, "the status line does not move");
        assert_eq!(opened.editor.y, closed.editor.y + 1, "the frames below start one row lower");
    }

    #[test]
    fn context_menu_stays_on_screen() {
        use crate::menu::{ContextMenu, ContextTarget};
        let full = Rect { x: 0, y: 0, width: 80, height: 24 };
        // Anchored comfortably inside: the menu opens exactly there.
        let m = ContextMenu::new(ContextTarget::Editor, (10, 5), false);
        let rect = context_menu_rect(&m, Lang::En, &Keymap::default(), full);
        assert_eq!((rect.x, rect.y), (10, 5));
        assert!(rect.x + rect.width <= full.width && rect.y + rect.height <= full.height);

        // Anchored in the far bottom-right: pulled back so it never spills off either edge.
        let m2 = ContextMenu::new(ContextTarget::Editor, (79, 23), false);
        let rect2 = context_menu_rect(&m2, Lang::En, &Keymap::default(), full);
        assert_eq!(rect2.x + rect2.width, full.width);
        assert_eq!(rect2.y + rect2.height, full.height);
    }

    #[test]
    fn terminal_tabs_only_appear_with_more_than_one() {
        let area = Rect { x: 5, y: 2, width: 40, height: 10 };
        // One tab: no strip. The tabs ride the top border, so the content is always the full
        // interior regardless of tab count.
        assert!(terminal_tab_ranges(area, &["Terminal 1".to_string()]).is_empty());
        assert_eq!(terminal_content_rect(area), inner_rect(area));

        // The strip rides the top border row, starting just inside the left corner. With a window
        // close button present, it stops two cells short of the right edge.
        let strip = terminal_tab_strip_rect(area, true);
        assert_eq!((strip.x, strip.y), (6, 2));
        assert_eq!(strip.x + strip.width, area.x + area.width - 2);

        // Three tabs: ` Terminal N ✕ ` chips (name = 10 cells, chip = 14) laid left to right, each
        // with a close glyph after the name.
        let labels: Vec<String> = (1..=3).map(|n| format!("Terminal {n}")).collect();
        let tabs = terminal_tab_ranges(strip, &labels);
        assert_eq!(tabs.len(), 3);
        assert_eq!(tabs[0].full, (6, 20));
        assert_eq!(tabs[0].close, Some(18));
        assert_eq!(tabs[1].full, (20, 34));
        assert_eq!(tabs[1].close, Some(32));
    }

    #[test]
    fn terminal_close_cell_sits_top_right_inside_the_corner() {
        let area = Rect { x: 10, y: 4, width: 20, height: 8 };
        // One column in from the top-right corner (x+width-1), on the top border row.
        assert_eq!(terminal_close_cell(area), Some((28, 4)));
        // Too narrow to carry a title: no button.
        assert_eq!(terminal_close_cell(Rect { x: 0, y: 0, width: 2, height: 5 }), None);
    }

    #[test]
    fn tab_strip_shows_every_tab_when_they_fit() {
        let strip = tab_strip_layout(&W, 50, 0);
        assert_eq!(strip.first, 0);
        assert_eq!(strip.tabs.len(), 5);
        assert_eq!(strip.tabs[0].full, (0, 10));
        assert_eq!(strip.tabs[4].full, (40, 50));
        // The × is drawn one cell before the tab's trailing space, and the label stops there.
        assert_eq!(strip.tabs[0].label, (0, 8));
        // The click target is the glyph plus the padding either side of it: three cells, so the
        // close is aimed at rather than hunted for.
        assert_eq!(strip.tabs[0].close, (7, 10));
        assert_eq!(strip.tabs[4].close, (47, 50));
        assert!(strip.left_arrow.is_none() && strip.right_arrow.is_none());
    }

    /// The close target may never reach into the tab next door: one cell to the left of it is
    /// still the tab it belongs to, and one cell to the right is the neighbour's own title.
    #[test]
    fn the_close_target_stays_inside_its_own_tab() {
        let strip = tab_strip_layout(&W, 50, 0);
        for (i, tab) in strip.tabs.iter().enumerate() {
            assert!(tab.close.0 >= tab.full.0 && tab.close.1 <= tab.full.1, "tab {i}");
            assert!(tab.close.0 >= tab.label.1 - 1, "tab {i} eats its own title");
        }
        // And the hit-test maps every one of those cells to the tab that drew them.
        assert_eq!(strip.tab_at(7).map(|(i, _)| i), Some(0));
        assert_eq!(strip.tab_at(9).map(|(i, _)| i), Some(0));
        assert_eq!(strip.tab_at(10).map(|(i, _)| i), Some(1));
    }

    #[test]
    fn tab_strip_marks_tabs_hidden_to_the_right() {
        // 25 columns: two whole tabs, and the › arrow claims a column of the leftover.
        let strip = tab_strip_layout(&W, 25, 0);
        assert_eq!(strip.tabs.len(), 2);
        assert_eq!(strip.left_arrow, None);
        assert_eq!(strip.right_arrow, Some((24, 25)));
    }

    #[test]
    fn revealing_scrolls_forward_to_the_active_tab() {
        // Tab 4 cannot be seen from offset 0 in 25 columns, so the window advances to it.
        let offset = offset_revealing(&W, 25, 0, 4);
        let strip = tab_strip_layout(&W, 25, offset);
        let rendered = strip.first..strip.first + strip.tabs.len();
        assert!(rendered.contains(&4), "active tab must be rendered, got {rendered:?}");
        // Scrolled off the left, so that arrow shows and tabs start after it.
        assert_eq!(strip.left_arrow, Some((0, 1)));
        assert_eq!(strip.right_arrow, None);
        assert_eq!(strip.tabs[0].full.0, 1);
    }

    #[test]
    fn revealing_scrolls_back_when_the_active_tab_is_behind_the_window() {
        assert_eq!(offset_revealing(&W, 25, 3, 0), 0);
        // Already visible: nothing moves.
        assert_eq!(offset_revealing(&W, 25, 2, 2), 2);
        assert_eq!(offset_revealing(&[], 25, 0, 0), 0);
    }

    /// The regression behind "the ‹ arrow does nothing": revealing used to run on every render,
    /// so scrolling away from the active tab was undone a frame later. Layout must now leave a
    /// deliberate offset exactly where it is, in both directions.
    #[test]
    fn layout_does_not_snap_back_to_the_active_tab() {
        // Active tab is 4, user scrolled the window left to 1.
        let strip = tab_strip_layout(&W, 25, 1);
        assert_eq!(strip.first, 1, "a manual scroll must survive rendering");
        assert!(!(strip.first..strip.first + strip.tabs.len()).contains(&4));
        // And one step further left still moves.
        assert_eq!(tab_strip_layout(&W, 25, 0).first, 0);
    }

    #[test]
    fn tab_at_maps_columns_to_absolute_indices() {
        let strip = tab_strip_layout(&W, 25, 2);
        assert_eq!(strip.first, 2);
        // Column 1 is the first rendered tab, which is tab 2 overall, not tab 0.
        assert_eq!(strip.tab_at(1).map(|(i, _)| i), Some(2));
        assert_eq!(strip.tab_at(11).map(|(i, _)| i), Some(3));
        // The arrow column belongs to no tab.
        assert_eq!(strip.tab_at(0).map(|(i, _)| i), None);
    }

    /// A tab wider than the whole strip used to take the bar down with it: the layout answered
    /// "nothing fits" and the row went blank — no file name, no dirty marker, no ×, and no way
    /// to close the file with the mouse. One tab cut short is the answer; an empty bar is not.
    #[test]
    fn a_tab_wider_than_the_strip_is_clipped_rather_than_dropped() {
        let strip = tab_strip_layout(&W, 7, 0);
        assert_eq!(strip.tabs.len(), 1);
        assert_eq!(strip.first, 0);
        // The whole strip, bar the › that says the other four are still there.
        assert_eq!(strip.right_arrow, Some((6, 7)));
        assert_eq!(strip.tabs[0].full, (0, 6));
        assert_eq!(strip.tabs[0].label, (0, 4));
        // No padding to give away on the left: that cell is the ellipsis, and it is text.
        assert_eq!(strip.tabs[0].close, (4, 6));
        assert_eq!(strip.tab_at(0).map(|(i, _)| i), Some(0));

        // Scrolled to the last tab there is nothing to the right, so it keeps that column.
        let last = tab_strip_layout(&W, 7, 4);
        assert_eq!(last.first, 4);
        assert_eq!(last.right_arrow, None);
        assert_eq!(last.left_arrow, Some((0, 1)));
        assert_eq!(last.tabs[0].full, (1, 7));
    }

    #[test]
    fn tab_strip_degrades_without_panicking_when_too_narrow() {
        assert!(tab_strip_layout(&W, 0, 0).tabs.is_empty());
        // Under three columns there is no room for a letter, the × and its space, so nothing is
        // drawn rather than a close button with no tab attached to it.
        assert!(tab_strip_layout(&W, 2, 0).tabs.is_empty());
        assert!(tab_strip_layout(&[], 50, 0).tabs.is_empty());
    }

    /// In wrap mode the caret used to be put on its *logical* row, so every long line above it
    /// shifted the text down and left the caret behind — typing appeared to happen on somebody
    /// else's line.
    #[test]
    fn the_caret_follows_the_text_down_the_rows_a_wrapped_line_takes() {
        // Ten columns of text. The first line takes three rows, the second one, the third two.
        let widths = [25usize, 4, 11];
        // On the first line, second row, fifth column.
        assert_eq!(wrapped_cursor_offset(&widths, 0, 14, 10, 24), Some((4, 1)));
        // The short line under it starts on row 3, not on row 1.
        assert_eq!(wrapped_cursor_offset(&widths, 1, 2, 10, 24), Some((2, 3)));
        // And the one after that on row 4 — the short line still takes a row of its own.
        assert_eq!(wrapped_cursor_offset(&widths, 2, 0, 10, 24), Some((0, 4)));
        // Past the wrap on that line: second row of it.
        assert_eq!(wrapped_cursor_offset(&widths, 2, 10, 10, 24), Some((0, 5)));
        // An empty line above still occupies a row.
        assert_eq!(wrapped_cursor_offset(&[0, 0], 2, 0, 10, 24), Some((0, 2)));
        // Nothing above: unwrapped behaviour, which is the same answer.
        assert_eq!(wrapped_cursor_offset(&[7], 0, 3, 10, 24), Some((3, 0)));
    }

    /// Scrolling in wrap mode counts logical lines, so the caret can genuinely be below the
    /// viewport. Drawn at the nearest row it would claim an edit is landing somewhere it is not.
    #[test]
    fn a_caret_wrapped_off_the_bottom_is_not_drawn_at_all() {
        let widths = [30usize, 30, 30];
        assert_eq!(wrapped_cursor_offset(&widths, 2, 0, 10, 24), Some((0, 6)));
        assert_eq!(wrapped_cursor_offset(&widths, 2, 0, 10, 6), None);
        // A zero-width pane must answer rather than divide by it.
        assert_eq!(wrapped_cursor_offset(&widths, 0, 0, 0, 24), Some((0, 0)));
    }

    /// The bar is drawn from the left, so on a narrow window the last titles run past the edge.
    /// A title half on screen is clickable as far as it is drawn; one entirely off it is not
    /// clickable at all, and must therefore claim no columns.
    #[test]
    fn menu_titles_are_cut_to_the_bar_they_are_drawn_on() {
        let menu = MenuBar::new();
        let full = menu_title_ranges(&menu, Lang::En);
        let last = full.last().copied().unwrap();
        assert!(last.1 > 20, "the bar has to overflow 20 columns for this to be a test");

        let cut = menu_titles_within(&menu, Lang::En, 20);
        assert_eq!(cut.len(), full.len(), "every menu keeps its place in the list");
        for ((start, end), (was_start, was_end)) in cut.iter().zip(&full) {
            assert!(*end <= 20, "a title claims column {end} of a 20-column bar");
            assert!(start <= end);
            // Untouched while it fits.
            if *was_end <= 20 {
                assert_eq!((*start, *end), (*was_start, *was_end));
            }
        }
        // The one that straddles the edge keeps the half that is painted.
        let straddling = full.iter().position(|(s, e)| *s < 20 && *e > 20);
        if let Some(i) = straddling {
            assert_eq!(cut[i], (full[i].0, 20));
            assert!(cut[i].0 < cut[i].1, "a visible title has to stay hittable");
        }
        // And everything past it collapses to nothing, so no column maps to it.
        assert!(cut.iter().filter(|(s, e)| s == e).count() > 0);
        assert!(!cut.iter().any(|(s, e)| s == e && *s < 20));
    }

    /// Ctrl+Shift+B still reaches a menu whose title is off the right edge, and the box it drops
    /// has to be readable when it gets there.
    #[test]
    fn a_dropdown_stays_on_screen_even_when_its_title_is_not() {
        let mut menu = MenuBar::new();
        menu.menu_index = menu.defs.len() - 1;
        for width in [10u16, 20, 40, 80] {
            let full = Rect { x: 0, y: 0, width, height: 24 };
            let rect = menu_dropdown_rect(&menu, Lang::En, &Keymap::default(), full);
            assert!(rect.right() <= full.right(), "{width}: {rect:?} runs off the edge");
            assert!(rect.width > 0 && rect.x >= full.x, "{width}: {rect:?}");
        }
    }

    /// The notice takes a row from the body or it lands on the body's last line. At three rows
    /// there is nothing to take, so it is the notice that goes.
    #[test]
    fn the_git_notice_never_paints_over_the_list() {
        for height in 3..12u16 {
            let inner = Rect { x: 2, y: 5, width: 40, height };
            let (body, notice) = git_body_layout(inner, true);
            let notice_row = inner.bottom() - 2;
            let keys_row = inner.bottom() - 1;
            assert!(body.y >= inner.y + 1, "the tab row keeps its own line");
            assert!(body.bottom() <= keys_row, "height {height}: the list runs under the keys");
            if notice {
                assert!(body.bottom() <= notice_row, "height {height}: the notice covers a row of list");
            } else {
                assert_eq!(height, 3, "the notice is only given up at the floor");
            }
            // Without one, the body simply gets the row back.
            let (wider, shown) = git_body_layout(inner, false);
            assert!(!shown);
            assert!(wider.height >= body.height);
        }
    }

    /// A name long enough to fill the sidebar used to push the git dot off the edge, so the rows
    /// most in need of a status were the ones that had none.
    #[test]
    fn a_long_tree_name_is_cut_so_the_git_dot_keeps_its_column() {
        // Room for the dot and a blank before it: nothing is cut.
        let (name, pad) = tree_row_name("  ", "main.rs", 20);
        assert_eq!(name, "main.rs");
        assert_eq!(2 + 2 + name.chars().count() + pad + 1, 20);

        // Too long: cut with an ellipsis, and the dot's column survives.
        let (name, pad) = tree_row_name("      ", "a_very_long_module_name.rs", 20);
        assert!(name.ends_with('…'), "{name:?} does not say it was cut");
        assert_eq!(6 + 2 + name.chars().count() + pad + 1, 20, "the row still adds up");
        assert_eq!(pad, 0);

        // A sidebar dragged to nothing must answer rather than subtract past zero.
        let _ = tree_row_name("            ", "deep.rs", 4);
    }

    #[test]
    fn venv_label_shortens_absolute_paths() {
        // Auto-discovered venvs are relative names and stay as they are.
        assert_eq!(venv_display_name(".venv", &[]), ".venv");
        // A registered venv shows its own folder, not the whole path.
        assert_eq!(venv_display_name("/opt/venvs/ml-3.12", &[]), "ml-3.12");
        // A generic folder name is qualified by its parent, so two registered ".venv"
        // directories remain distinguishable in the toolbar.
        assert_eq!(venv_display_name("/work/project-a/.venv", &[]), "project-a/.venv");
        assert_eq!(venv_display_name("/work/project-a/.venv/", &[]), "project-a/.venv");
        // The other platform's separator is cut too: a settings.toml gets copied between
        // machines, and `is_absolute` used to answer "no" to a POSIX path on Windows and leave
        // the whole thing in the toolbar.
        assert_eq!(venv_display_name(r"C:\venvs\ml-3.12", &[]), "ml-3.12");
        assert_eq!(venv_display_name(r"C:\work\project-a\.venv", &[]), "project-a/.venv");
    }

    #[test]
    fn venv_nickname_wins_over_the_derived_name() {
        let registered = vec![
            settings::RegisteredVenv::Named { name: "ml".to_string(), path: "/opt/venvs/ml-3.12".to_string() },
            settings::RegisteredVenv::Path("/opt/venvs/plain".to_string()),
        ];
        assert_eq!(venv_display_name("/opt/venvs/ml-3.12", &registered), "ml");
        // Registered without a nickname, and venvs that aren't registered at all, keep the
        // folder-derived label.
        assert_eq!(venv_display_name("/opt/venvs/plain", &registered), "plain");
        assert_eq!(venv_display_name("/opt/venvs/other", &registered), "other");
    }

    #[test]
    fn the_toolbar_names_a_command_by_its_program() {
        assert_eq!(run_program_name("pdflatex -interaction=nonstopmode {file}"), "pdflatex");
        assert_eq!(run_program_name("octave --persist {file}"), "octave");
        // A command spelled out as a path is still named by the program, not by the path — the
        // button has one short slot, and the full line is in the drop-down underneath.
        assert_eq!(run_program_name("/opt/homebrew/bin/octave-cli {file}"), "octave-cli");
        // A Windows path has to be quoted to survive the space in "Program Files", so the
        // program is one token and both separators lead to the same short name.
        assert_eq!(
            run_program_name(r#""C:\Program Files\Octave\octave-cli.exe" {file}"#),
            "octave-cli"
        );
        // Unquoted, because it has no spaces to protect: the backslashes have to survive being
        // read, or there is no separator left to cut the name at and the whole path lands on the
        // button.
        assert_eq!(run_program_name(r"C:\Octave\bin\octave-cli.exe {file}"), "octave-cli");
        assert_eq!(run_program_name(""), "");
    }

    /// The bar is drawn from this layout and clicked through it, so every button it paints has
    /// to be one that can be hit — same cells, no overlaps, nothing a single column wide.
    #[test]
    fn every_nav_button_drawn_is_a_button_that_can_be_clicked() {
        use crate::preview::Kind;
        let mut x = 5u16;
        let mut last_end = 0u16;
        let controls = [
            (NavControl::PageBack, Kind::Document),
            (NavControl::GoToPage, Kind::Document),
            (NavControl::Invert, Kind::Document),
            // The same button under its other name, which is the longer of the two.
            (NavControl::Invert, Kind::Picture),
            (NavControl::TextMode, Kind::Markdown),
        ];
        for (control, kind) in controls {
            let width = nav_width(control, kind);
            assert!(width >= 3, "a one-cell target is not clickable");
            // Name and key are drawn together on the button, so neither is repeated elsewhere.
            let (name, key) = nav_label(control, kind);
            assert!(!name.is_empty() && !key.is_empty());
            assert!(x > last_end, "buttons must not overlap");
            last_end = x + width;
            x += width + 1;
        }
    }

    /// The same rule for the formatting bar: what is drawn is what can be hit, and no two
    /// buttons claim a column. A row of one-cell targets is what "the buttons do not work"
    /// feels like on a trackpad, which is the complaint `NAV_MIN_WIDTH` was written for.
    #[test]
    fn every_formatting_button_drawn_is_one_that_can_be_clicked() {
        // Wide enough for the whole run, so nothing is dropped from the right.
        let row = Rect { x: 4, y: 7, width: 120, height: 1 };
        let drawn = md_toolbar_layout(row);
        assert_eq!(drawn.len(), MD_TOOLS.len(), "the whole bar should fit in 120 columns");
        let zones = md_toolbar_hit_zones(row);
        assert_eq!(zones.len(), drawn.len());
        for (i, (tool, rect)) in drawn.iter().enumerate() {
            assert!(rect.width >= 3, "{tool:?} is {} cells wide", rect.width);
            assert!(rect.x >= row.x && rect.x + rect.width <= row.x + row.width, "{tool:?} runs off the bar");
            let (name, _) = md_tool_label(*tool);
            assert!(!name.is_empty(), "{tool:?} has no label");
            // The zone contains the button it belongs to, and touches its neighbours exactly.
            let zone = zones[i].1;
            assert_eq!(zones[i].0, *tool);
            assert!(zone.x <= rect.x && zone.x + zone.width >= rect.x + rect.width, "{tool:?}");
            if i > 0 {
                let before = zones[i - 1].1;
                assert_eq!(before.x + before.width, zone.x, "{tool:?}: a column belongs to neither");
            }
        }
    }

    /// A pane too narrow for the whole run drops buttons from the right rather than squeezing
    /// them, and never draws one that runs off the edge.
    #[test]
    fn a_narrow_pane_drops_formatting_buttons_instead_of_shrinking_them() {
        let mut seen = 0;
        for width in 0..120u16 {
            let row = Rect { x: 0, y: 0, width, height: 1 };
            let drawn = md_toolbar_layout(row);
            for (tool, rect) in &drawn {
                assert!(rect.x + rect.width <= width, "width {width}: {tool:?} runs off the bar");
                assert_eq!(rect.width, md_tool_width(*tool), "width {width}: {tool:?} was squeezed");
            }
            assert!(drawn.len() >= seen, "width {width}: a wider bar lost a button");
            seen = drawn.len();
        }
    }

    /// The three rectangles have to partition the pane: a row belonging to two of them is drawn
    /// over twice, and one belonging to none swallows every click that lands on it. And with the
    /// bar off, the split has to be exactly the one every call site had before it existed.
    #[test]
    fn the_editor_split_partitions_the_pane_with_or_without_the_bar() {
        for height in 0..12u16 {
            let area = Rect { x: 3, y: 2, width: 40, height };
            let (tab_bar, none, content) = split_editor_area_v2(area, false);
            assert!(none.is_none(), "height {height}: a bar appeared unasked");
            assert_eq!((tab_bar, content), split_editor_area(area), "height {height}");

            let (tab_bar, toolbar, content) = split_editor_area_v2(area, true);
            let rows = tab_bar.height + toolbar.map_or(0, |t| t.height) + content.height;
            assert_eq!(rows, area.height, "height {height}: the rows do not add up");
            if let Some(toolbar) = toolbar {
                assert_eq!(toolbar.y, tab_bar.y + tab_bar.height, "height {height}");
                assert_eq!(content.y, toolbar.y + toolbar.height, "height {height}");
                assert!(content.height >= 1, "height {height}: a bar with no text under it");
                assert_eq!((toolbar.x, toolbar.width), (area.x, area.width), "height {height}");
            }
        }
    }

    /// Four ways for the bar not to be there, and the setting is only one of them. Getting any
    /// of the other three wrong draws a row of markdown buttons over a file they cannot act on
    /// — or, on a short pane, over the last line of the file itself.
    #[test]
    fn the_formatting_bar_stays_away_from_everything_it_is_not_for() {
        let tall = Rect { x: 0, y: 0, width: 80, height: 24 };
        assert!(md_toolbar_shown(true, "md", true, tall), "a markdown buffer should have it");
        assert!(!md_toolbar_shown(false, "md", true, tall), "switched off in the View menu");
        assert!(!md_toolbar_shown(true, "rs", true, tall), "these actions write markdown");
        assert!(!md_toolbar_shown(true, "", true, tall), "a buffer never saved has no syntax to write");
        assert!(!md_toolbar_shown(true, "md", false, tall), "a read-only buffer refuses every edit");
        for height in 0..MD_TOOLBAR_MIN_HEIGHT {
            let short = Rect { height, ..tall };
            assert!(!md_toolbar_shown(true, "md", true, short), "height {height} has no row to spare");
        }
        assert!(md_toolbar_shown(true, "md", true, Rect { height: MD_TOOLBAR_MIN_HEIGHT, ..tall }));
    }

    /// A picture's button says "invert" and a document's says "dark", because they are not the
    /// same act: one makes a negative of a photograph, the other turns a white page dark. They
    /// shared a name once, and a picture opened inverted because a PDF had been read that way.
    #[test]
    fn a_picture_is_inverted_and_a_document_is_darkened() {
        use crate::preview::Kind;
        assert_eq!(nav_label(NavControl::Invert, Kind::Picture), ("invert", "i"));
        assert_eq!(nav_label(NavControl::Invert, Kind::Document), ("dark", "d"));
        assert_eq!(nav_label(NavControl::Invert, Kind::Markdown), ("dark", "d"));
    }

    /// The state text and the buttons share one row, and the state is drawn second — so if it
    /// were allowed to start too far left it would paint over a button, which is how the
    /// "wide" button once ended up reading "page".
    #[test]
    fn the_state_text_never_reaches_the_buttons() {
        // A row 60 wide with buttons ending at 30 leaves 29 cells for state and hint.
        let row_end = 60u16;
        let buttons_end = 30u16;
        let free = row_end.saturating_sub(buttons_end + 1);
        assert_eq!(free, 29);

        // Anything that fits is right-aligned *within* what is free, never over a button.
        for text_len in [1u16, 10, 29] {
            let x = row_end - text_len;
            assert!(x > buttons_end, "text of {text_len} would overlap a button ending at {buttons_end}");
        }
        // And one cell too long no longer fits, so it is dropped rather than overlapping.
        assert!(30 > free);
    }

    /// The bar takes a row from the picture, and the two must not both claim it.
    #[test]
    fn the_bar_and_the_picture_divide_the_frame() {
        let area = Rect { x: 2, y: 3, width: 40, height: 12 };
        let inner = inner_rect(area);
        let bar = nav_bar_rect(area).expect("a normal frame has room");
        let image = preview_image_rect(area);
        assert_eq!(bar.y, inner.y + inner.height - 1);
        assert_eq!(image.height, inner.height - 1);
        assert_eq!(image.y + image.height, bar.y, "the picture stops where the bar starts");

        // A frame with no room inside keeps neither, rather than drawing one over the other.
        assert_eq!(nav_bar_rect(Rect { x: 0, y: 0, width: 40, height: 2 }), None);
        assert_eq!(nav_bar_rect(Rect { x: 0, y: 0, width: 4, height: 12 }), None);
    }

    #[test]
    fn scrollbars_sit_inside_the_frame_leaving_the_border_to_the_seam() {
        // A frame at (10,5) 40x12 has its contents at (11,6) 38x10.
        let area = Rect { x: 10, y: 5, width: 40, height: 12 };
        let inner = inner_rect(area);

        let bar = scrollbar_strip(inner_rect(area), Axis::Vertical).expect("a normal frame has room");
        // Last column of the *contents*, one in from the border — the border is the resize seam
        // and stays entirely its own.
        assert_eq!(bar.x, 48);
        assert_eq!(bar.x, inner.x + inner.width - 1);
        assert_ne!(bar.x, area.x + area.width - 1, "the border belongs to the seam");
        assert_eq!(bar.width, 1);
        // The full height of the contents: inside the frame there are no corners to dodge.
        assert_eq!(bar.y, 6);
        assert_eq!(bar.height, 10);

        let bar = scrollbar_strip(inner_rect(area), Axis::Horizontal).expect("a normal frame has room");
        assert_eq!(bar.y, 15);
        assert_eq!(bar.y, inner.y + inner.height - 1);
        assert_eq!(bar.height, 1);
        assert_eq!(bar.x, 11);
        // One short of the right, which is the vertical bar's column: no cell answers to both.
        assert_eq!(bar.width, 37);
        let vertical = scrollbar_strip(inner_rect(area), Axis::Vertical).unwrap();
        assert_eq!(bar.x + bar.width, vertical.x);

        // Frames too small to have an interior get no bar rather than one drawn on the border.
        let vert = |w, h| scrollbar_strip(inner_rect(Rect { x: 0, y: 0, width: w, height: h }), Axis::Vertical);
        assert_eq!(vert(2, 12), None, "no interior width");
        assert_eq!(vert(40, 3), None, "one row of contents is not a scrollbar");
        assert_eq!(vert(0, 0), None);
        let horiz = |w, h| scrollbar_strip(inner_rect(Rect { x: 0, y: 0, width: w, height: h }), Axis::Horizontal);
        assert_eq!(horiz(3, 12), None, "one column of contents, all of it the vertical bar's");
        assert_eq!(horiz(40, 2), None);
    }

    /// A one-cell bar that is invisible until the pointer is exactly on it has to be aimed at
    /// blind. The band that brings it up is wider than the bar; what a click hits is not.
    #[test]
    fn a_scrollbar_shows_itself_before_the_pointer_is_on_it() {
        let inner = inner_rect(Rect { x: 10, y: 5, width: 40, height: 12 });
        let strip = scrollbar_strip(inner, Axis::Vertical).unwrap();
        let zone = scrollbar_reveal_zone(inner, Axis::Vertical).unwrap();

        // Wider than the bar, ending on it, and never outside the contents.
        assert!(zone.width > strip.width);
        assert_eq!(zone.x + zone.width, strip.x + strip.width);
        assert!(zone.x >= inner.x);
        assert_eq!((zone.y, zone.height), (strip.y, strip.height));

        let horiz_strip = scrollbar_strip(inner, Axis::Horizontal).unwrap();
        let horiz_zone = scrollbar_reveal_zone(inner, Axis::Horizontal).unwrap();
        assert!(horiz_zone.height > horiz_strip.height);
        assert_eq!(horiz_zone.y + horiz_zone.height, horiz_strip.y + horiz_strip.height);
        assert!(horiz_zone.y >= inner.y);

        // A frame narrower than the band keeps the band inside it rather than off the left edge.
        let narrow = inner_rect(Rect { x: 0, y: 0, width: 4, height: 12 });
        let zone = scrollbar_reveal_zone(narrow, Axis::Vertical).unwrap();
        assert_eq!((zone.x, zone.width), (narrow.x, narrow.width));

        // No bar, no band: the two agree about frames with nothing to spare.
        let cramped = inner_rect(Rect { x: 0, y: 0, width: 2, height: 12 });
        assert_eq!(scrollbar_reveal_zone(cramped, Axis::Vertical), None);
    }

    /// The arrows are painted by us and hit-tested by the app off this one layout, so the cells
    /// it hands back have to tile the strip exactly — a gap would be a dead cell, an overlap a
    /// click that did two things.
    #[test]
    fn arrows_take_the_ends_and_leave_the_rest_as_track() {
        let strip = Rect { x: 7, y: 3, width: 1, height: 10 };
        let layout = scrollbar_layout(strip, Axis::Vertical);
        assert_eq!(layout.back.expect("room for arrows").y, 3);
        assert_eq!(layout.forward.expect("room for arrows").y, 12);
        assert_eq!(layout.track.y, 4);
        assert_eq!(layout.track.height, 8);

        let strip = Rect { x: 2, y: 9, width: 12, height: 1 };
        let layout = scrollbar_layout(strip, Axis::Horizontal);
        assert_eq!(layout.back.expect("room for arrows").x, 2);
        assert_eq!(layout.forward.expect("room for arrows").x, 13);
        assert_eq!(layout.track.x, 3);
        assert_eq!(layout.track.width, 10);

        // Too short to give up two cells and still leave a track worth aiming at: no arrows,
        // and the whole strip stays track rather than becoming two arrows and nothing between.
        let tiny = Rect { x: 0, y: 0, width: 1, height: 4 };
        let layout = scrollbar_layout(tiny, Axis::Vertical);
        assert_eq!(layout.back, None);
        assert_eq!(layout.forward, None);
        assert_eq!(layout.track, tiny);
    }

    /// Dragging the thumb to either end of the track has to land exactly on the first and last
    /// line, not near them — landing one short of the end is the classic scrollbar bug.
    #[test]
    fn the_track_maps_onto_the_whole_travel_of_the_view() {
        // 100 lines, 20 on screen: the top of the view can travel 0..=80.
        let pos = |offset| scroll_position_from_track(offset, 11, 100, 20);
        assert_eq!(pos(0), 0);
        assert_eq!(pos(10), 80, "the far end of the track is the last screenful");
        assert_eq!(pos(5), 40, "and the middle is the middle");
        // Past the end is clamped rather than running off.
        assert_eq!(pos(50), 80);

        // Nothing to scroll, and a track with nowhere to move, both stay at the top instead of
        // dividing by zero.
        assert_eq!(scroll_position_from_track(3, 11, 20, 20), 0);
        assert_eq!(scroll_position_from_track(0, 1, 100, 20), 0);
        assert_eq!(scroll_position_from_track(0, 0, 100, 20), 0);
    }

    #[test]
    fn an_overlong_label_is_cut_rather_than_widening_the_button() {
        assert_eq!(fit("go run", 10), "go run");
        assert_eq!(fit("exactly-10", 10), "exactly-10");
        assert_eq!(fit("venv: some/very/long/name", 10), "venv: som\u{2026}");
    }
}


