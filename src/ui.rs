use crate::app::{App, EditorPane, Focus};
use crate::i18n::{self, Key, Lang};
use crate::menu::{ContextMenu, MenuBar};
use crate::terminal_panel::TerminalWindow;
use crate::settings;
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
    pub status: Rect,
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

pub fn compute_layout(full: Rect, p: &LayoutParams) -> Areas {
    let menu_h = if p.show_menubar || p.menu_active { 1 } else { 0 };
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(menu_h), Constraint::Min(1), Constraint::Length(1)])
        .split(full);
    let menu_bar = outer[0];
    let main_area = outer[1];
    let status = outer[2];

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
        Areas { menu_bar, sidebar, editor, terminals, status }
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

        Areas { menu_bar, sidebar, editor, terminals, status }
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

pub struct TabLayout {
    /// Whole-tab range: clicking anywhere in it (outside `close`) switches to it.
    pub full: (u16, u16),
    /// The "×" close glyph's own range within the tab.
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
        // `count == 0` means not even one tab fits: there is nothing to scroll toward.
        if count == 0 || active < first + count || first >= last {
            return first;
        }
        first += 1;
    }
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
        return empty;
    }

    let mut x = if left { ARROW_W } else { 0 };
    let mut tabs = Vec::with_capacity(count);
    for w in &widths[first..first + count] {
        let close_start = x + w - 2; // the × sits before the trailing space
        tabs.push(TabLayout { full: (x, x + w), close: (close_start, close_start + 1) });
        x += w;
    }
    TabStrip {
        first,
        tabs,
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
/// Split the way a shell would, so a program path quoted because it has spaces in it — the
/// usual shape on Windows — is one token rather than two. Both separators are cut on either
/// platform: a settings.toml is copied between machines.
pub fn run_program_name(template: &str) -> String {
    let program = shell_words::split(template)
        .ok()
        .and_then(|words| words.into_iter().next())
        .unwrap_or_else(|| template.split_whitespace().next().unwrap_or("").to_string());
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

pub fn menu_title_ranges(menu: &MenuBar, lang: Lang) -> Vec<(u16, u16)> {
    let mut ranges = Vec::new();
    let mut x = MENU_LOGO.chars().count() as u16;
    for def in &menu.defs {
        let label = format!(" {} ", i18n::t(lang, def.title_key));
        let w = label.chars().count() as u16;
        ranges.push((x, x + w));
        x += w;
    }
    ranges
}

pub fn menu_dropdown_rect(menu: &MenuBar, lang: Lang, full: Rect) -> Rect {
    let ranges = menu_title_ranges(menu, lang);
    let (x, _) = ranges.get(menu.menu_index).copied().unwrap_or((0, 0));
    let items = &menu.defs[menu.menu_index].items;
    let label_width = items.iter().map(|i| i18n::t(lang, i.label_key).chars().count()).max().unwrap_or(0);
    let shortcut_width = items.iter().filter_map(|i| i.shortcut).map(|s| s.chars().count()).max().unwrap_or(0);
    let gap = if shortcut_width > 0 { 3 } else { 0 };
    let width = ((1 + label_width + gap + shortcut_width + 1) as u16).max(18);
    let separators = items.iter().filter(|i| i.new_group).count() as u16;
    let height = items.len() as u16 + separators + 2;
    Rect {
        x: x.min(full.width.saturating_sub(width)),
        y: 1,
        width: width.min(full.width),
        height: height.min(full.height.saturating_sub(1)),
    }
}

/// Where a context menu hangs: from its anchor, but pulled back so it never spills past the
/// right or bottom edge. Shared by the renderer and click handling so both agree on the rows.
pub fn context_menu_rect(menu: &ContextMenu, lang: Lang, full: Rect) -> Rect {
    let items = &menu.items;
    let label_width = items.iter().map(|i| i18n::t(lang, i.label_key).chars().count()).max().unwrap_or(0);
    let shortcut_width = items.iter().filter_map(|i| i.shortcut).map(|s| s.chars().count()).max().unwrap_or(0);
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
    let (tab_bar, _) = split_editor_area(pane);
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

pub fn about_modal_rect(full: Rect) -> Rect {
    // Tall enough for the version, the wrapped tagline (three lines in Italian, the longer
    // of the two), the author and repository lines, and the close hint.
    centered_rect(60, 13, full)
}

pub fn settings_modal_rect(full: Rect) -> Rect {
    let width = 54u16;
    let height = settings::SETTINGS_COUNT as u16 + 2;
    centered_rect(width, height, full)
}

/// The colour a frame's border takes while a resize is under way (Ctrl+Shift+U, or a border drag).
/// Orange, to stand clearly apart from the cyan of ordinary focus.
const RESIZE_BORDER_COLOR: Color = Color::Rgb(255, 140, 0);

fn focused_border_style(is_focused: bool, resizing: bool) -> Style {
    if is_focused {
        let color = if resizing { RESIZE_BORDER_COLOR } else { Color::Cyan };
        Style::default().fg(color)
    } else {
        Style::default().fg(Color::DarkGray)
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
    let lang = app.settings.lang;
    let mut lines: Vec<Line> = Vec::new();
    for row in SPLASH_BANNER {
        lines.push(Line::from(Span::styled(*row, Style::default().fg(Color::Green))).alignment(ratatui::layout::Alignment::Center));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(i18n::t(lang, Key::SplashTagline)).alignment(ratatui::layout::Alignment::Center));
    lines.push(Line::from(""));
    lines.push(
        Line::from(format!("{} · v{}", i18n::t(lang, Key::SplashSubtitle), env!("CARGO_PKG_VERSION")))
            .alignment(ratatui::layout::Alignment::Center),
    );
    lines.push(
        Line::from(Span::styled("msavox 2026", Style::default().fg(Color::DarkGray)))
            .alignment(ratatui::layout::Alignment::Center),
    );
    // Started with a workspace — `clee -w name`, or a resumed one — so say which, while the
    // splash is the only thing on screen and the shells behind it are still starting.
    if let Some(name) = app.active_workspace.as_deref() {
        lines.push(Line::from(""));
        lines.push(
            Line::from(vec![
                Span::styled(format!("{} ", i18n::t(lang, Key::WorkspaceBadge)), Style::default().fg(Color::DarkGray)),
                Span::styled(name.to_string(), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            ])
            .alignment(ratatui::layout::Alignment::Center),
        );
    }
    lines.push(Line::from(""));
    lines.push(
        Line::from(Span::styled(i18n::t(lang, Key::SplashHint), Style::default().fg(Color::DarkGray)))
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

pub fn draw(f: &mut Frame, app: &mut App) {
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

    draw_status(f, app, areas.status);
    draw_menu_bar(f, app, areas.menu_bar);

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
    if app.show_terminal_rename {
        draw_terminal_rename_modal(f, app, f.area());
    }
    if app.show_workspace_save {
        draw_workspace_save_modal(f, app, f.area());
    }
    if app.show_goto {
        draw_goto_modal(f, app, f.area());
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

fn draw_menu_bar(f: &mut Frame, app: &App, area: Rect) {
    // Hidden bar collapses to a zero-height row; nothing to paint (menus still reachable
    // via Ctrl+Shift+B, whose dropdown anchors to the top independently of this row).
    if area.height == 0 {
        return;
    }
    let lang = app.settings.lang;
    let mut spans = vec![Span::styled(MENU_LOGO, Style::default().bg(Color::Black))];
    let mut used = MENU_LOGO.chars().count() as u16;
    for (i, def) in app.menu.defs.iter().enumerate() {
        let title = i18n::t(lang, def.title_key);
        let label = format!(" {} ", title);
        used += label.chars().count() as u16;
        let is_open = app.menu.active && app.menu.menu_index == i;
        let mut style = if is_open {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(Color::Gray).bg(Color::Black)
        };
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
        let mnemonic_style = if app.menu.active { style.add_modifier(Modifier::UNDERLINED) } else { style };
        spans.push(Span::styled(" ", style));
        spans.push(Span::styled(mnemonic, mnemonic_style));
        spans.push(Span::styled(format!("{} ", rest), style));
    }
    // The open workspace, right-aligned on the same row. Nothing on screen used to say which one
    // you were in — the name only appeared in the status line for a moment when it loaded, and
    // was gone by the time you wondered. Titles are drawn from the left and this from the right,
    // with the padding between them, so a long name eats blank space and never the menus.
    let workspace = app
        .active_workspace
        .as_deref()
        .map(|name| format!(" {} {} ", i18n::t(lang, Key::WorkspaceBadge), name))
        .unwrap_or_default();
    let pad = area.width.saturating_sub(used).saturating_sub(workspace.chars().count() as u16);
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad as usize), Style::default().bg(Color::Black)));
    }
    if !workspace.is_empty() {
        spans.push(Span::styled(workspace, Style::default().fg(Color::Black).bg(Color::Green)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_menu_dropdown(f: &mut Frame, app: &App, full: Rect) {
    let lang = app.settings.lang;
    let rect = menu_dropdown_rect(&app.menu, lang, full);
    let inner_width = rect.width.saturating_sub(2) as usize;
    // Separator rules are woven in between real items, so the row a given item
    // renders on drifts down by one for every group opened above it. Track the
    // selected item's display row so the highlight lands on the right line.
    let separator = ListItem::new(Line::from(Span::styled(
        "─".repeat(inner_width),
        Style::default().fg(Color::DarkGray),
    )));
    let mut items: Vec<ListItem> = Vec::new();
    let mut selected_row = 0;
    for (idx, i) in app.menu.defs[app.menu.menu_index].items.iter().enumerate() {
        if i.new_group {
            items.push(separator.clone());
        }
        if idx == app.menu.item_index {
            selected_row = items.len();
        }
        let label = i18n::t(lang, i.label_key);
        let line = match i.shortcut {
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
        .border_style(Style::default().fg(Color::Cyan));
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    f.render_widget(Clear, rect);
    f.render_stateful_widget(list, rect, &mut state);
}

fn draw_context_menu(f: &mut Frame, app: &App, full: Rect) {
    let lang = app.settings.lang;
    let Some(menu) = app.context_menu.as_ref() else { return };
    let rect = context_menu_rect(menu, lang, full);
    let inner_width = rect.width.saturating_sub(2) as usize;
    // Same separator-aware layout as the menu bar's drop-down: rules between groups shift the
    // selected item's row down, so track where the highlight should land.
    let separator = ListItem::new(Line::from(Span::styled(
        "─".repeat(inner_width),
        Style::default().fg(Color::DarkGray),
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
        let line = match i.shortcut {
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
        .border_style(Style::default().fg(Color::Cyan));
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    f.render_widget(Clear, rect);
    f.render_stateful_widget(list, rect, &mut state);
}

fn draw_settings_modal(f: &mut Frame, app: &App, full: Rect) {
    let rect = settings_modal_rect(full);
    f.render_widget(Clear, rect);
    let rows = app.settings.rows();
    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let marker = if i == app.settings_selected { "> " } else { "  " };
            ListItem::new(Line::from(format!("{marker}{:<34}{}", r.label, r.value)))
        })
        .collect();
    let mut state = ListState::default();
    state.select(Some(app.settings_selected));
    let block = Block::default()
        .title(format!(" {} ", i18n::t(app.settings.lang, Key::SettingsTitle)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    f.render_stateful_widget(list, rect, &mut state);
}

fn draw_about_modal(f: &mut Frame, app: &App, full: Rect) {
    let rect = about_modal_rect(full);
    f.render_widget(Clear, rect);
    let lang = app.settings.lang;
    let block = Block::default()
        .title(format!(" {} ", i18n::t(lang, Key::AboutTitle)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let lines = vec![
        Line::from(Span::styled(
            format!("CleeCode v{}", env!("CARGO_PKG_VERSION")),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(i18n::t(lang, Key::AboutTagline)),
        Line::from(""),
        Line::from(Span::styled(
            i18n::t(lang, Key::AboutAuthor),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!("{}  ·  MIT", i18n::t(lang, Key::AboutRepo)),
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            i18n::t(lang, Key::AboutCloseHint),
            Style::default().fg(Color::DarkGray),
        )),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

pub fn delete_confirm_modal_rect(full: Rect) -> Rect {
    centered_rect(60, 5, full)
}

fn draw_delete_confirm_modal(f: &mut Frame, app: &App, full: Rect) {
    let rect = delete_confirm_modal_rect(full);
    f.render_widget(Clear, rect);
    let name = app
        .delete_target
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let block = Block::default()
        .title(" Delete? ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let text = i18n::msg_confirm_delete(app.settings.lang, &name);
    f.render_widget(Paragraph::new(Line::from(text)).wrap(Wrap { trim: false }), inner);
}

pub fn unsaved_modal_rect(full: Rect) -> Rect {
    centered_rect(64, 6, full)
}

fn draw_unsaved_modal(f: &mut Frame, app: &App, full: Rect) {
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
        .title(" Unsaved changes ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let lines = vec![
        Line::from(i18n::msg_unsaved_question(lang, &detail)),
        Line::from(Span::styled(i18n::msg_unsaved_choices(lang), Style::default().fg(Color::Gray))),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

pub fn rename_modal_rect(full: Rect) -> Rect {
    centered_rect(60, 6, full)
}

fn draw_rename_modal(f: &mut Frame, app: &App, full: Rect) {
    let rect = rename_modal_rect(full);
    f.render_widget(Clear, rect);
    let old_name = app
        .rename_target
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let block = Block::default()
        .title(" Rename ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let prompt = i18n::msg_rename_prompt(app.settings.lang, &old_name);
    let lines = vec![Line::from(prompt), Line::from(Span::styled(app.rename_input.clone(), Style::default().fg(Color::Yellow)))];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    let cursor_x = inner.x + app.rename_input.chars().count() as u16;
    let cursor_y = inner.y + 1;
    f.set_cursor_position((cursor_x, cursor_y));
}

/// Simple single-line input modal shared by Go-to-line and New file/folder.
fn draw_input_modal(f: &mut Frame, full: Rect, title: &str, prompt: &str, input: &str) {
    let rect = centered_rect(60, 6, full);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let lines = vec![
        Line::from(prompt.to_string()),
        Line::from(Span::styled(input.to_string(), Style::default().fg(Color::Yellow))),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    f.set_cursor_position((inner.x + input.chars().count() as u16, inner.y + 1));
}

fn draw_goto_modal(f: &mut Frame, app: &App, full: Rect) {
    let lang = app.settings.lang;
    // What the number will mean depends on what is being looked at.
    let pages = app.editor().preview.as_ref().is_some_and(|p| p.pages.is_some());
    draw_input_modal(
        f,
        full,
        i18n::goto_title(lang, pages),
        i18n::msg_goto_prompt(lang, pages),
        &app.goto_input,
    );
}

/// The terminal's name and its startup command, in one box: two prompts, two values, and a
/// caret on whichever field is being typed into.
fn draw_terminal_rename_modal(f: &mut Frame, app: &App, full: Rect) {
    use crate::app::TerminalField;
    let lang = app.settings.lang;
    let rect = centered_rect(74, 7, full);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .title(" Terminal name & startup command ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let on_name = app.terminal_rename_field == TerminalField::Name;
    let marker = |active: bool| if active { "▶ " } else { "  " };
    let value = Style::default().fg(Color::Yellow);
    let label = Style::default().fg(Color::Gray);
    let lines = vec![
        Line::from(Span::styled(format!("{}{}", marker(on_name), i18n::msg_terminal_rename_prompt(lang)), label)),
        Line::from(Span::styled(format!("  {}", app.terminal_rename_input), value)),
        Line::from(Span::styled(format!("{}{}", marker(!on_name), i18n::msg_terminal_startup_prompt(lang)), label)),
        Line::from(Span::styled(format!("  {}", app.terminal_startup_input), value)),
        Line::from(Span::styled(i18n::msg_terminal_form_hint(lang), Style::default().fg(Color::DarkGray))),
    ];
    f.render_widget(Paragraph::new(lines), inner);

    let (row, len) = if on_name {
        (1u16, app.terminal_rename_input.chars().count())
    } else {
        (3u16, app.terminal_startup_input.chars().count())
    };
    f.set_cursor_position((inner.x + 2 + len as u16, inner.y + row));
}

fn draw_workspace_save_modal(f: &mut Frame, app: &App, full: Rect) {
    let prompt = i18n::msg_workspace_save_prompt(app.settings.lang);
    draw_input_modal(f, full, "Save workspace", &prompt, &app.workspace_save_input);
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
fn manual_line(line: &'static str) -> Line<'static> {
    let rule = Style::default().fg(Color::DarkGray);
    let key = Style::default().fg(Color::Yellow);
    let heading = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
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
    let Some(state) = app.manual.as_ref() else { return };
    let lang = app.settings.lang;
    let sections = crate::manual::sections(lang);
    let rect = manual_rect(full);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .title(format!(" {} · v{} ", i18n::t(lang, Key::ManualTitle), env!("CARGO_PKG_VERSION")))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    // Table of contents, numbered so the digit keys have something to point at.
    let list = manual_list_rect(rect);
    let toc: Vec<Line> = sections
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let style = if i == state.section {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::Gray)
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
        .map(|_| Line::from(Span::styled("│", Style::default().fg(Color::DarkGray))))
        .collect();
    f.render_widget(Paragraph::new(rule_lines), rule);

    let body_area = manual_body_rect(rect);
    let Some(section) = sections.get(state.section) else { return };
    let visible: Vec<Line> = section
        .body
        .iter()
        .skip(state.scroll)
        .take(body_area.height as usize)
        .map(|line| manual_line(line))
        .collect();
    f.render_widget(Paragraph::new(visible), body_area);

    // Position within the section, then the key hints, on the two rows kept back above.
    let footer = Rect { x: body_area.x, y: body_area.y + body_area.height, width: body_area.width, height: 2 };
    let shown = (state.scroll + body_area.height as usize).min(section.body.len());
    let position = format!("{}/{}  ", shown, section.body.len().max(1));
    let footer_lines = vec![
        Line::from(Span::styled(
            format!("{} · {}", section.title, position),
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(i18n::t(lang, Key::ManualHint), Style::default().fg(Color::DarkGray))),
    ];
    f.render_widget(Paragraph::new(footer_lines), footer);
}

fn draw_new_entry_modal(f: &mut Frame, app: &App, full: Rect) {
    let lang = app.settings.lang;
    let title = if app.new_entry_is_dir { "New folder" } else { "New file" };
    let prompt = i18n::msg_new_entry_prompt(lang, app.new_entry_is_dir);
    draw_input_modal(f, full, title, prompt, &app.new_entry_input);
}

fn draw_run_menu(f: &mut Frame, app: &App, editor_area: Rect, full: Rect) {
    let Some(menu) = app.run_menu.as_ref() else { return };
    let Some(rect) = run_menu_rect(app, editor_area, full) else { return };
    f.render_widget(Clear, rect);
    let block = Block::default()
        // Named after the extension, so it is plain that what's chosen here applies to every
        // file of this kind rather than only to the one on screen.
        .title(format!(" {} \u{00b7} .{} ", i18n::t(app.settings.lang, Key::RunMenuTitle), menu.ext))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let items: Vec<ListItem> = app
        .run_menu_rows()
        .into_iter()
        .map(|row| {
            let marker = if row.active { "● " } else { "  " };
            let mut spans = vec![Span::raw(format!("{marker}{}", row.label))];
            if let Some(detail) = row.detail {
                spans.push(Span::styled(format!("  {detail}"), Style::default().fg(Color::DarkGray)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items).highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan));
    let mut state = ListState::default();
    state.select(Some(menu.selected));
    f.render_stateful_widget(list, inner, &mut state);
}

/// The run command for one extension, typed in full. Its own box rather than the shared
/// single-line one: a command line is long, and the placeholders are worth spelling out where
/// they are being typed instead of leaving them to the manual.
fn draw_run_command_modal(f: &mut Frame, app: &App, full: Rect) {
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
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let dim = Style::default().fg(Color::DarkGray);
    let lines = vec![
        Line::from(i18n::msg_run_command_prompt(lang, *scope)),
        Line::from(Span::styled(app.run_command_input.clone(), Style::default().fg(Color::Yellow))),
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
    let lang = app.settings.lang;
    let (title, prompt) = match app.venv_register {
        Some(crate::app::VenvRegisterStep::Path) => ("Add venv (1/2)", i18n::msg_venv_path_prompt(lang)),
        Some(crate::app::VenvRegisterStep::Nickname) => {
            ("Add venv (2/2)", i18n::msg_venv_nickname_prompt(lang))
        }
        None => return,
    };
    draw_input_modal(f, full, title, &prompt, &app.venv_register_input);
}

fn draw_save_as_modal(f: &mut Frame, app: &App, full: Rect) {
    let prompt = i18n::msg_save_as_prompt(app.settings.lang);
    draw_input_modal(f, full, "Save as", &prompt, &app.save_as_input);
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

fn draw_picker_modal(f: &mut Frame, app: &App, full: Rect) {
    let Some(p) = app.picker.as_ref() else { return };
    let rect = picker_rect(full);
    f.render_widget(Clear, rect);
    let title = format!(" {}  ({} matches) ", p.title, p.filtered.len());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let list_rows = inner.height.saturating_sub(1) as usize;
    // Scroll so the selected row stays visible.
    let start = if p.selected >= list_rows { p.selected + 1 - list_rows } else { 0 };
    let width = inner.width as usize;

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("> ", Style::default().fg(Color::Cyan)),
        Span::styled(p.query.clone(), Style::default().fg(Color::White)),
    ]));
    for (row, &item_idx) in p.filtered.iter().enumerate().skip(start).take(list_rows) {
        let item = &p.items[item_idx];
        let selected = row == p.selected;
        let row_style = if selected {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(Color::Gray)
        };
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
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            spans.push(Span::styled(sc.to_string(), sc_style));
        }
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), inner);
    f.set_cursor_position((inner.x + 2 + p.query.chars().count() as u16, inner.y));
}

fn draw_find_modal(f: &mut Frame, app: &App, full: Rect) {
    let Some(fs) = app.find.as_ref() else { return };
    let lang = app.settings.lang;
    let rect = centered_rect(72, 7, full);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .title(" Find / Replace ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let count = if fs.query.is_empty() {
        String::new()
    } else if fs.matches.is_empty() {
        "  (no matches)".to_string()
    } else {
        format!("  {}/{}", fs.current + 1, fs.matches.len())
    };
    let find_marker = if fs.focus_replace { "  " } else { "▶ " };
    let repl_marker = if fs.focus_replace { "▶ " } else { "  " };
    let label = Style::default().fg(Color::Gray);
    let value = Style::default().fg(Color::Yellow);
    let lines = vec![
        Line::from(vec![
            Span::styled(format!("{find_marker}Find:    "), label),
            Span::styled(fs.query.clone(), value),
            Span::styled(count, Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(vec![
            Span::styled(format!("{repl_marker}Replace: "), label),
            Span::styled(fs.replace.clone(), value),
        ]),
        Line::from(Span::styled(i18n::msg_find_hint(lang), Style::default().fg(Color::DarkGray))),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);

    // Cursor sits at the end of whichever field is focused.
    let (row, text_len) = if fs.focus_replace {
        (1u16, fs.replace.chars().count())
    } else {
        (0u16, fs.query.chars().count())
    };
    // "▶ Find:    " / "▶ Replace: " prefixes are both 11 columns wide.
    f.set_cursor_position((inner.x + 11 + text_len as u16, inner.y + row));
}

fn git_status_color(status: crate::git_status::FileStatus) -> Color {
    use crate::git_status::FileStatus;
    match status {
        FileStatus::Modified => Color::Yellow,
        FileStatus::Added => Color::Green,
        FileStatus::Deleted => Color::Red,
        FileStatus::Renamed => Color::Cyan,
        FileStatus::Untracked => Color::Gray,
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

fn draw_file_tree(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::FileTree;
    let block = Block::default()
        .title(format!(" {} ", i18n::t(app.settings.lang, Key::PanelFile)))
        .borders(Borders::ALL)
        .border_style(focused_border_style(focused, app.layout_resize_active()));

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
                (if entry.expanded { "\u{f07c}" } else { "\u{f07b}" }, Color::Rgb(120, 170, 255))
            } else {
                file_icon(&entry.name)
            };
            // indent + icon (1 col) + space (1 col) + name, then right-pad up to the dot.
            let left_len = indent.chars().count() + 2 + entry.name.chars().count();
            let dot = path.as_ref().and_then(|p| app.git_status.get(p));
            let pad = inner_width.saturating_sub(left_len + 1);
            let mut spans = vec![
                Span::raw(indent),
                Span::styled(icon, Style::default().fg(icon_color)),
                Span::raw(format!(" {}", entry.name)),
            ];
            if pad > 0 {
                spans.push(Span::raw(" ".repeat(pad)));
            }
            spans.push(match dot {
                Some(status) => Span::styled("\u{25cf}", Style::default().fg(git_status_color(*status))),
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
fn highlight_selection(spans: Vec<(Style, String)>, sel_from: usize, sel_to: usize) -> Vec<(Style, String)> {
    if sel_from >= sel_to {
        return spans;
    }
    let mut result = Vec::new();
    let mut pos = 0usize;
    for (style, text) in spans {
        let char_count = text.chars().count();
        let span_start = pos;
        let span_end = pos + char_count;
        pos = span_end;
        if span_end <= sel_from || span_start >= sel_to {
            result.push((style, text));
            continue;
        }
        let chars: Vec<char> = text.chars().collect();
        let local_from = sel_from.saturating_sub(span_start).min(chars.len());
        let local_to = sel_to.saturating_sub(span_start).min(chars.len());
        if local_from > 0 {
            result.push((style, chars[..local_from].iter().collect()));
        }
        if local_to > local_from {
            let sel_style = style.bg(Color::Rgb(60, 90, 130));
            result.push((sel_style, chars[local_from..local_to].iter().collect()));
        }
        if local_to < chars.len() {
            result.push((style, chars[local_to..].iter().collect()));
        }
    }
    result
}

fn draw_tab_bar(f: &mut Frame, app: &App, area: Rect, active_position: usize, pane: EditorPane) {
    let lang = app.settings.lang;
    let mut spans = Vec::new();
    let strip_width = tab_strip_width(app, area.width);
    let tabs = app.pane_tabs(pane);
    let strip = tab_strip_layout(&tab_widths(app, pane), strip_width, app.tab_offsets[pane.index()]);
    let arrow_style = Style::default().fg(Color::Gray).bg(Color::DarkGray);
    if strip.left_arrow.is_some() {
        spans.push(Span::styled(SCROLL_LEFT_GLYPH, arrow_style));
    }
    let mut used = strip.tabs.first().map(|t| t.full.0).unwrap_or(0);
    for (offset, &editor_idx) in tabs[strip.first.min(tabs.len())..]
        .iter()
        .take(strip.tabs.len())
        .enumerate()
    {
        let Some(editor) = app.editors.get(editor_idx) else { continue };
        let position = strip.first + offset;
        let dirty = if editor.dirty { "*" } else { "" };
        let prefix = format!(" {}{} ", editor.title(lang), dirty);
        used += prefix.chars().count() as u16 + 2; // + close glyph + trailing space
        let style = if position == active_position {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(Color::Gray).bg(Color::DarkGray)
        };
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
        spans.push(Span::styled(label, Style::default().fg(Color::Gray).bg(Color::DarkGray)));
    }
    if run_range.is_some() {
        let label = run_button_label(app, app.pane_editor_index(pane));
        spans.push(Span::styled(label, Style::default().fg(Color::Black).bg(Color::Green)));
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
    }
}

/// How many cells a button takes: a space, the name, the key, a space. The zoom buttons name
/// themselves with their own key, so it is not written twice.
fn nav_width(control: NavControl, kind: crate::preview::Kind) -> u16 {
    let (name, key) = nav_label(control, kind);
    if name == key {
        name.chars().count() as u16 + 2
    } else {
        (name.chars().count() + key.chars().count()) as u16 + 3
    }
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
    let Some(row) = nav_bar_rect(area) else { return };
    let Some(preview) = app.editors.get(idx).and_then(|e| e.preview.as_ref()) else { return };
    let lang = app.settings.lang;
    f.render_widget(
        Paragraph::new(" ".repeat(row.width as usize)).style(Style::default().bg(Color::Rgb(30, 30, 30))),
        row,
    );

    for (control, rect) in nav_bar_layout(app, idx, area) {
        // The key that does the same thing is written under the label, so the bar teaches the
        // keyboard rather than competing with it.
        let style = Style::default().fg(Color::Gray).bg(Color::Rgb(45, 45, 45));
        let style = match control {
            NavControl::FitWidth if preview.fit == crate::preview::Fit::Width => {
                style.fg(Color::Black).bg(Color::Cyan)
            }
            NavControl::FitPage if preview.fit == crate::preview::Fit::Page => {
                style.fg(Color::Black).bg(Color::Cyan)
            }
            NavControl::Invert if preview.inverted => style.fg(Color::Black).bg(Color::Cyan),
            NavControl::TextMode if preview.text_only => style.fg(Color::Black).bg(Color::Cyan),
            _ => style,
        };
        let (name, key) = nav_label(control, preview.kind());
        let dim = Style::default().fg(Color::DarkGray).bg(style.bg.unwrap_or(Color::Reset));
        let line = if name == key {
            Line::from(Span::styled(format!(" {name} "), style))
        } else {
            Line::from(vec![
                Span::styled(format!(" {name} "), style),
                Span::styled(format!("{key} "), dim),
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
    f.render_widget(Paragraph::new(Span::styled(text, Style::default().fg(Color::DarkGray))), rect);
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
    use crate::preview::State as Preview;
    let lang = app.settings.lang;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(focused_border_style(focused, app.layout_resize_active()));
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
                draw_scrollbar(f, scrollbar_area(app, idx, content_area), axis, total, position, viewport, engaged);
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
        Some(Preview::Loading) => centred(f, i18n::msg_preview_loading(lang), Color::DarkGray),
        Some(Preview::Failed(reason)) => {
            let text = i18n::msg_preview_failed(lang, &reason.clone());
            centred(f, text, Color::Red);
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
            let scroll = top_line.min(lines.len().saturating_sub(1)) as u16;
            let text = ratatui::text::Text::from(lines.clone());
            f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }).scroll((scroll, 0)), inner);
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

fn draw_editor_pane(f: &mut Frame, app: &mut App, area: Rect, idx: usize, focused: bool, pane: EditorPane) {
    let (tab_bar_area, content_area) = split_editor_area(area);
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
        .border_style(focused_border_style(focused, app.layout_resize_active()));

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
    app.editors[idx].adjust_scroll(viewport_height, if app.settings.word_wrap { 0 } else { text_width });

    if app.editors[idx].syntax_dirty {
        if app.settings.syntax_highlighting {
            let text = app.editors[idx].rope.to_string();
            let path = app.editors[idx].path.clone();
            app.editors[idx].highlighted = app.highlighter.highlight(path.as_deref(), &text);
        } else {
            app.editors[idx].highlighted.clear();
        }
        app.editors[idx].syntax_dirty = false;
    }

    let top_line = app.editors[idx].top_line;
    let left_col = app.editors[idx].left_col;
    let cursor_line = app.editors[idx].cursor_line;
    let visible_rows = app.editors[idx].visible_rows_from(top_line, viewport_height);
    let cursor_row = visible_rows.iter().position(|&l| l == cursor_line).unwrap_or(0);
    let mut lines: Vec<Line> = Vec::new();
    for line_idx in visible_rows.iter().copied() {
        let mut spans: Vec<Span> = Vec::new();
        if gutter > 0 {
            let is_current = line_idx == cursor_line;
            let num_style = if is_current {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let num_text = format!("{:>width$} ", line_idx + 1, width = (gutter as usize).saturating_sub(1));
            spans.push(Span::styled(num_text, num_style));
        }
        if app.editors[idx].folds.iter().any(|&(s, _)| s == line_idx) {
            spans.push(Span::styled("▸ ", Style::default().fg(Color::Cyan)));
        }

        let raw_spans: Vec<(Style, String)> = if app.settings.syntax_highlighting {
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
        // The editor decides the shape — a run of text or a rectangle — so the highlight always
        // matches what a copy would take.
        let raw_spans = match app.editors[idx].selected_columns(line_idx) {
            Some((from, to)) => highlight_selection(raw_spans, from, to),
            None => raw_spans,
        };

        if app.settings.word_wrap {
            for (style, text) in raw_spans {
                spans.push(Span::styled(text, style));
            }
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
    draw_editor_scrollbars(f, app, idx, pane, content_area, viewport_height, text_width);

    if focused {
        let cursor_x = inner.x + gutter + app.editors[idx].cursor_col.saturating_sub(left_col) as u16;
        let cursor_y = inner.y + cursor_row as u16;
        f.set_cursor_position((cursor_x, cursor_y));
    }
}

/// The rows and text columns a pane's buffer gets, worked out from the pane's own rectangle.
///
/// Pure, and used by both the renderer and mouse handling, so the two agree on what a scrollbar
/// is describing without either having to remember what the other did — the same reason the tab
/// strip's layout is a function rather than a stored rect.
pub fn editor_viewport(app: &App, idx: usize, pane_rect: Rect) -> (Rect, usize, usize) {
    let (_, content_area) = split_editor_area(pane_rect);
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
fn draw_editor_scrollbars(
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
        draw_scrollbar(f, scrollbar_area(app, idx, area), axis, total, position, viewport, engaged);
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

/// How long a scrollbar stays up after the last scroll before fading out again.
const SCROLLBAR_LINGER: Duration = Duration::from_millis(1200);

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
fn draw_scrollbar(
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
        let arrow_style = Style::default().fg(Color::Gray);
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
        .thumb_style(Style::default().fg(Color::Cyan));
    bar = if lit {
        bar.track_symbol(Some(track_glyph)).track_style(Style::default().fg(Color::DarkGray))
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
    draw_scrollbar(f, inner_rect(area), Axis::Vertical, total, position, viewport, engaged);
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
fn draw_terminal_tab_strip(f: &mut Frame, area: Rect, labels: &[String], active: usize) {
    let tabs = terminal_tab_ranges(area, labels);
    let mut spans: Vec<Span> = Vec::new();
    for (i, tab) in tabs.iter().enumerate() {
        let budget = (tab.full.1 - tab.full.0) as usize;
        let chip: String = format!(" {} ✕ ", labels[i]).chars().take(budget).collect();
        let style = if i == active {
            Style::default().fg(Color::Black).bg(Color::Green)
        } else {
            Style::default().fg(Color::Gray).bg(Color::DarkGray)
        };
        spans.push(Span::styled(chip, style));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_terminals(f: &mut Frame, app: &mut App, term_areas: &[Rect]) {
    let active = app.active_terminal;
    let focus_terminal = app.focus == Focus::Terminal;
    for (i, area) in term_areas.iter().enumerate() {
        let is_focused_pane = focus_terminal && i == active;
        draw_single_terminal(f, app, *area, i, is_focused_pane);
    }
}

fn draw_single_terminal(f: &mut Frame, app: &mut App, area: Rect, index: usize, focused: bool) {
    let (labels, active_tab) = {
        let Some(window) = app.terminals.get(index) else { return };
        (terminal_tab_labels(window, index, app.settings.lang), window.active)
    };
    let tab_count = labels.len();
    let window_close = app.terminals.len() > 1;

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(focused_border_style(focused, app.layout_resize_active()));
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
            Line::from(Span::styled("\u{2715}", Style::default().fg(Color::Red))).right_aligned(),
        );
    }
    // The tab strip rides the top border, so the content is the whole interior.
    let content = terminal_content_rect(area);

    // The border (and close button) first, then the tabs over the top border, then the contents.
    f.render_widget(block, area);
    if tab_count > 1 {
        let strip = terminal_tab_strip_rect(area, window_close);
        draw_terminal_tab_strip(f, strip, &labels, active_tab);
    }

    let rows = content.height;
    let cols = content.width;
    // Read before the pane is borrowed mutably below, since it is a question about the app as a
    // whole — where the pointer is, and what is being dragged.
    let engaged =
        app.scrollbar_engaged(crate::app::ScrollbarId::Terminal(index), area, Axis::Vertical);
    let Some(terminal) = app.terminals.get_mut(index).map(|w| w.active_tab_mut()) else { return };
    terminal.resize(rows, cols);

    // Keep the pane clean during shell startup: hide the banner/rc output until the shell
    // settles, so the user sees an empty pane (then a clean prompt) rather than a banner
    // that only gets cleared seconds later.
    if !terminal.is_ready() {
        if content.height > 0 && content.width > 0 {
            let hint = i18n::terminal_starting(app.settings.lang);
            let hint_w = (hint.chars().count() as u16).min(content.width);
            let rect = Rect {
                x: content.x + content.width.saturating_sub(hint_w) / 2,
                y: content.y + content.height / 2,
                width: hint_w,
                height: 1,
            };
            f.render_widget(
                Paragraph::new(Span::styled(hint, Style::default().fg(Color::DarkGray))),
                rect,
            );
        }
        return;
    }

    // Read before the parser is locked below: the lock is a plain mutex, so asking the panel
    // anything about its scrollback while holding it would deadlock the whole app.
    draw_terminal_scrollbar(f, terminal, area, engaged);

    let selection = terminal.selection;
    let parser = crate::terminal_panel::lock_poisoned(&terminal.parser);
    let screen = parser.screen();
    let (screen_rows, screen_cols) = screen.size();

    let mut lines: Vec<Line> = Vec::new();
    for row in 0..screen_rows {
        let mut spans: Vec<Span> = Vec::new();
        let mut current = String::new();
        let mut current_style = Style::default();
        let mut have_style = false;

        for col in 0..screen_cols {
            let cell = screen.cell(row, col);
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

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let msg = if app.resize_mode {
        i18n::t(app.settings.lang, Key::ResizeModeHint).to_string()
    } else {
        app.status_message.clone()
    };
    let style = if app.resize_mode {
        Style::default().fg(Color::Black).bg(Color::Yellow)
    } else {
        Style::default().fg(Color::Gray)
    };
    let paragraph = Paragraph::new(Line::from(Span::raw(msg))).style(style);
    f.render_widget(paragraph, area);

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
    use super::*;

    /// Five tabs of 10 columns each.
    const W: [u16; 5] = [10, 10, 10, 10, 10];

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
        let m = ContextMenu::new(ContextTarget::Editor, (10, 5));
        let rect = context_menu_rect(&m, Lang::En, full);
        assert_eq!((rect.x, rect.y), (10, 5));
        assert!(rect.x + rect.width <= full.width && rect.y + rect.height <= full.height);

        // Anchored in the far bottom-right: pulled back so it never spills off either edge.
        let m2 = ContextMenu::new(ContextTarget::Editor, (79, 23));
        let rect2 = context_menu_rect(&m2, Lang::En, full);
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
        // The × sits before the tab's trailing space.
        assert_eq!(strip.tabs[0].close, (8, 9));
        assert!(strip.left_arrow.is_none() && strip.right_arrow.is_none());
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

    #[test]
    fn tab_strip_degrades_without_panicking_when_too_narrow() {
        assert!(tab_strip_layout(&W, 0, 0).tabs.is_empty());
        // Narrower than a single tab: nothing is rendered rather than a half-drawn tab.
        assert!(tab_strip_layout(&W, 4, 0).tabs.is_empty());
        assert!(tab_strip_layout(&[], 50, 0).tabs.is_empty());
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
