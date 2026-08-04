use crate::app::{App, EditorPane, Focus};
use crate::i18n::{self, Key, Lang};
use crate::menu::{ContextMenu, MenuBar};
use crate::settings;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

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
    let menu_h = if p.show_menubar { 1 } else { 0 };
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
    // The left pane gets `left_pct` of the width; the right takes the remainder, so no column is
    // lost to rounding. Both keep at least one column even at the clamp extremes.
    let mid = ((area.width as u32 * left_pct as u32) / 100) as u16;
    let mid = mid.clamp(1, area.width.saturating_sub(1));
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
/// than the buttons, which stay reachable via `F10` and the Run menu.
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
pub fn tab_widths(app: &App) -> Vec<u16> {
    let lang = app.settings.lang;
    app.editors
        .iter()
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

fn run_button_label(lang: Lang) -> String {
    format!(" \u{25b6} {} ", i18n::t(lang, Key::ToolbarRun))
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
    let path = std::path::Path::new(venv);
    if !path.is_absolute() {
        return venv.to_string();
    }
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return venv.to_string();
    };
    if matches!(name.as_str(), ".venv" | "venv" | "env" | ".env") {
        if let Some(parent) = path.parent().and_then(|p| p.file_name()) {
            return format!("{}/{}", parent.to_string_lossy(), name);
        }
    }
    name
}

fn venv_button_label(app: &App) -> String {
    // The selected venv is remembered globally, so opening a project that doesn't have it left
    // the button naming a venv that isn't there while runs quietly fell back to system python.
    // The label follows what would actually be used.
    match crate::app::effective_venv(app.settings.active_venv.as_deref(), &app.available_venvs) {
        Some(name) => format!(" venv: {} \u{25be} ", venv_display_name(name, &app.settings.registered_venvs)),
        None => format!(" {} \u{25be} ", i18n::t(app.settings.lang, Key::ToolbarVenvNone)),
    }
}

/// Relative (start, end) ranges for the right-aligned toolbar buttons that fit within
/// `area_width`: the venv selector (dropped first if there isn't room for both) and the Run
/// button. Their space is reserved up front, independent of how many tabs are open — the tab
/// strip scrolls instead of pushing the buttons off the bar — but they yield once fewer than
/// `MIN_TAB_STRIP` columns would be left for the tabs themselves.
pub fn toolbar_button_ranges(
    app: &App,
    area_width: u16,
    with_venv: bool,
) -> (Option<(u16, u16)>, Option<(u16, u16)>) {
    let run_w = run_button_label(app.settings.lang).chars().count() as u16;
    let venv_w = venv_button_label(app).chars().count() as u16;

    if with_venv && venv_w + run_w + MIN_TAB_STRIP <= area_width {
        let run_start = area_width - run_w;
        let venv_start = run_start - venv_w;
        (Some((venv_start, venv_start + venv_w)), Some((run_start, run_start + run_w)))
    } else if run_w + MIN_TAB_STRIP <= area_width {
        let run_start = area_width - run_w;
        (None, Some((run_start, run_start + run_w)))
    } else {
        (None, None)
    }
}

/// Columns available to the tab strip once the toolbar buttons have taken their place.
pub fn tab_strip_width(app: &App, area_width: u16, with_venv: bool) -> u16 {
    let (venv_range, run_range) = toolbar_button_ranges(app, area_width, with_venv);
    match venv_range.or(run_range) {
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

/// Where the venv drop-down hangs: directly under its toolbar button, in the left/only editor
/// pane's tab bar. `None` when that button isn't on screen, in which case there is nothing to
/// drop down from. Shared by the renderer and by click handling, so both agree on the rows.
pub fn venv_dropdown_rect(app: &App, editor_area: Rect, full: Rect) -> Option<Rect> {
    let pane = editor_pane_rects(editor_area, app.split_view, app.settings.split_pct).first().copied()?;
    let (tab_bar, _) = split_editor_area(pane);
    if tab_bar.height == 0 {
        return None;
    }
    // The venv selector only ever renders in the left/only pane, hence with_venv = true.
    let (venv_range, _) = toolbar_button_ranges(app, tab_bar.width, true);
    let (start, _) = venv_range?;

    let rows = app.venv_dropdown_rows();
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

/// The colour a frame's border takes while a resize is under way (F8 mode, or a border drag).
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
        // While a menu is open, show the title bar even if it's normally hidden, so F9
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
    if app.venv_dropdown.is_some() {
        draw_venv_dropdown(f, app, areas.editor, f.area());
    }
    if app.venv_register.is_some() {
        draw_venv_register_modal(f, app, f.area());
    }
    if app.find.is_some() {
        draw_find_modal(f, app, f.area());
    }
    if app.picker.is_some() {
        draw_picker_modal(f, app, f.area());
    }
    // Topmost: the context menu overlays whatever it was raised over.
    if app.context_menu.is_some() {
        draw_context_menu(f, app, f.area());
    }
}

fn draw_menu_bar(f: &mut Frame, app: &App, area: Rect) {
    // Hidden bar collapses to a zero-height row; nothing to paint (menus still reachable
    // via F9 / Alt+<letter>, whose dropdown anchors to the top independently of this row).
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
        let mut chars = title.chars();
        let mnemonic = chars.next().map(|c| c.to_string()).unwrap_or_default();
        let rest: String = chars.collect();
        spans.push(Span::styled(" ", style));
        spans.push(Span::styled(mnemonic, style.add_modifier(Modifier::UNDERLINED)));
        spans.push(Span::styled(format!("{} ", rest), style));
    }
    let pad = area.width.saturating_sub(used);
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad as usize), Style::default().bg(Color::Black)));
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
    draw_input_modal(f, full, "Go to line", i18n::msg_goto_prompt(lang), &app.goto_input);
}

fn draw_new_entry_modal(f: &mut Frame, app: &App, full: Rect) {
    let lang = app.settings.lang;
    let title = if app.new_entry_is_dir { "New folder" } else { "New file" };
    let prompt = i18n::msg_new_entry_prompt(lang, app.new_entry_is_dir);
    draw_input_modal(f, full, title, prompt, &app.new_entry_input);
}

fn draw_venv_dropdown(f: &mut Frame, app: &App, editor_area: Rect, full: Rect) {
    let Some(selected) = app.venv_dropdown else { return };
    let Some(rect) = venv_dropdown_rect(app, editor_area, full) else { return };
    f.render_widget(Clear, rect);
    let block = Block::default()
        .title(format!(" {} ", i18n::t(app.settings.lang, Key::VenvPickerTitle)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let items: Vec<ListItem> = app
        .venv_dropdown_rows()
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
    state.select(Some(selected));
    f.render_stateful_widget(list, inner, &mut state);
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

fn draw_picker_modal(f: &mut Frame, app: &App, full: Rect) {
    let Some(p) = app.picker.as_ref() else { return };
    let width = full.width.saturating_sub(8).min(90).max(20);
    let height = full.height.saturating_sub(4).min(20).max(4);
    let rect = centered_rect(width, height, full);
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

fn draw_tab_bar(f: &mut Frame, app: &App, area: Rect, active_idx: usize, with_venv: bool, pane: EditorPane) {
    let lang = app.settings.lang;
    let mut spans = Vec::new();
    let strip_width = tab_strip_width(app, area.width, with_venv);
    let strip = tab_strip_layout(&tab_widths(app), strip_width, app.tab_offsets[pane.index()]);
    let arrow_style = Style::default().fg(Color::Gray).bg(Color::DarkGray);
    if strip.left_arrow.is_some() {
        spans.push(Span::styled(SCROLL_LEFT_GLYPH, arrow_style));
    }
    let mut used = strip.tabs.first().map(|t| t.full.0).unwrap_or(0);
    for (offset, editor) in app.editors[strip.first..].iter().take(strip.tabs.len()).enumerate() {
        let i = strip.first + offset;
        let dirty = if editor.dirty { "*" } else { "" };
        let prefix = format!(" {}{} ", editor.title(lang), dirty);
        used += prefix.chars().count() as u16 + 2; // + close glyph + trailing space
        let style = if i == active_idx {
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
    let (venv_range, run_range) = toolbar_button_ranges(app, area.width, with_venv);
    if let Some((start, _)) = venv_range.or(run_range) {
        let pad = start.saturating_sub(used);
        if pad > 0 {
            spans.push(Span::raw(" ".repeat(pad as usize)));
        }
    }
    if venv_range.is_some() {
        let label = venv_button_label(app);
        spans.push(Span::styled(label, Style::default().fg(Color::Gray).bg(Color::DarkGray)));
    }
    if run_range.is_some() {
        let label = run_button_label(lang);
        spans.push(Span::styled(label, Style::default().fg(Color::Black).bg(Color::Green)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_editor(f: &mut Frame, app: &mut App, area: Rect) {
    let panes = editor_pane_rects(area, app.split_view, app.settings.split_pct);
    if panes.len() == 1 {
        let focused = app.focus == Focus::Editor;
        draw_editor_pane(f, app, panes[0], app.active_editor, focused, true, EditorPane::Left);
        return;
    }
    let left_focused = app.focus == Focus::Editor && app.editor_pane_focus == EditorPane::Left;
    let right_focused = app.focus == Focus::Editor && app.editor_pane_focus == EditorPane::Right;
    // Both panes get a Run button so either focused file can be run; the (global) venv
    // selector stays on the left pane only to avoid a redundant, space-hungry duplicate.
    draw_editor_pane(f, app, panes[0], app.active_editor, left_focused, true, EditorPane::Left);
    draw_editor_pane(f, app, panes[1], app.active_editor_right, right_focused, false, EditorPane::Right);
}

fn draw_editor_pane(
    f: &mut Frame,
    app: &mut App,
    area: Rect,
    idx: usize,
    focused: bool,
    with_venv: bool,
    pane: EditorPane,
) {
    let (tab_bar_area, content_area) = split_editor_area(area);
    if tab_bar_area.height > 0 {
        // Only acts when the active tab changed; a manual scroll survives untouched.
        app.reveal_active_tab(pane, tab_bar_area.width, with_venv);
        draw_tab_bar(f, app, tab_bar_area, idx, with_venv, pane);
    }

    // No title here: the open tab right above already shows the filename and dirty marker.
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(focused_border_style(focused, app.layout_resize_active()));

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
    let sel_range = app.editors[idx].selection_range();
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
        let raw_spans = if let Some(((sl, sc), (el, ec))) = sel_range {
            if line_idx >= sl && line_idx <= el {
                let line_len = app.editors[idx].line_char_len(line_idx);
                let from = if line_idx == sl { sc } else { 0 };
                let to = if line_idx == el { ec } else { line_len };
                highlight_selection(raw_spans, from, to)
            } else {
                raw_spans
            }
        } else {
            raw_spans
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

    if focused {
        let cursor_x = inner.x + gutter + app.editors[idx].cursor_col.saturating_sub(left_col) as u16;
        let cursor_y = inner.y + cursor_row as u16;
        f.set_cursor_position((cursor_x, cursor_y));
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

/// The display name of each tab in a window — the same "Terminal N" wording as a window's own
/// title, so the tabs read clearly rather than as bare numbers.
pub fn terminal_tab_labels(lang: Lang, count: usize) -> Vec<String> {
    (0..count).map(|n| i18n::terminal_title(lang, n).trim().to_string()).collect()
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
fn draw_terminal_tab_strip(f: &mut Frame, area: Rect, lang: Lang, count: usize, active: usize) {
    let labels = terminal_tab_labels(lang, count);
    let tabs = terminal_tab_ranges(area, &labels);
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
    let (tab_count, active_tab) = {
        let Some(window) = app.terminals.get(index) else { return };
        (window.tabs.len(), window.active)
    };
    let window_close = app.terminals.len() > 1;

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(focused_border_style(focused, app.layout_resize_active()));
    // With a single tab the top border just carries the terminal's name; with several, the tabs
    // ride the border instead (drawn below) and stand in for the title.
    if tab_count <= 1 {
        block = block.title(i18n::terminal_title(app.settings.lang, index));
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
        draw_terminal_tab_strip(f, strip, app.settings.lang, tab_count, active_tab);
    }

    let rows = content.height;
    let cols = content.width;
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

    let selection = terminal.selection;
    let parser = terminal.parser.lock().unwrap();
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Five tabs of 10 columns each.
    const W: [u16; 5] = [10, 10, 10, 10, 10];

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
        assert!(terminal_tab_ranges(area, &terminal_tab_labels(Lang::En, 1)).is_empty());
        assert_eq!(terminal_content_rect(area), inner_rect(area));

        // The strip rides the top border row, starting just inside the left corner. With a window
        // close button present, it stops two cells short of the right edge.
        let strip = terminal_tab_strip_rect(area, true);
        assert_eq!((strip.x, strip.y), (6, 2));
        assert_eq!(strip.x + strip.width, area.x + area.width - 2);

        // Three tabs: ` Terminal N ✕ ` chips (name = 10 cells, chip = 14) laid left to right, each
        // with a close glyph after the name.
        let labels = terminal_tab_labels(Lang::En, 3);
        assert_eq!(labels[0], "Terminal 1");
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
}
