use crate::app::{App, Focus};
use crate::i18n::{self, Key, Lang};
use crate::menu::MenuBar;
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
    pub terminal_count: usize,
    pub sidebar_width: u16,
    pub terminal_pct: u16,
    pub terminal_on_right: bool,
}

impl LayoutParams {
    pub fn from_app(app: &App) -> Self {
        LayoutParams {
            show_sidebar: app.settings.show_sidebar,
            show_terminal: app.settings.show_terminal,
            terminal_count: app.terminals.len(),
            sidebar_width: app.settings.sidebar_width,
            terminal_pct: app.settings.terminal_pct,
            terminal_on_right: app.settings.terminal_on_right,
        }
    }
}

fn terminal_panes(area: Rect, count: usize, direction: Direction) -> Vec<Rect> {
    let n = count.max(1);
    let constraints: Vec<Constraint> = (0..n).map(|_| Constraint::Percentage((100 / n) as u16)).collect();
    Layout::default().direction(direction).constraints(constraints).split(area).to_vec()
}

pub fn compute_layout(full: Rect, p: &LayoutParams) -> Areas {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)])
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
            (h[0], Some(terminal_panes(h[1], p.terminal_count, Direction::Vertical)))
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
            (v[0], Some(terminal_panes(v[1], p.terminal_count, Direction::Horizontal)))
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

pub fn tab_ranges(app: &App) -> Vec<(u16, u16)> {
    let lang = app.settings.lang;
    let mut ranges = Vec::new();
    let mut x = 0u16;
    for editor in &app.editors {
        let dirty = if editor.dirty { "*" } else { "" };
        let label = format!(" {}{} ", editor.title(lang), dirty);
        let w = label.chars().count() as u16;
        ranges.push((x, x + w));
        x += w;
    }
    ranges
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
    let height = items.len() as u16 + 2;
    Rect {
        x: x.min(full.width.saturating_sub(width)),
        y: 1,
        width: width.min(full.width),
        height: height.min(full.height.saturating_sub(1)),
    }
}

pub fn about_modal_rect(full: Rect) -> Rect {
    centered_rect(60, 9, full)
}

pub fn settings_modal_rect(full: Rect) -> Rect {
    let width = 54u16;
    let height = settings::SETTINGS_COUNT as u16 + 2;
    centered_rect(width, height, full)
}

fn focused_border_style(is_focused: bool) -> Style {
    if is_focused {
        Style::default().fg(Color::Cyan)
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
        draw_menu_dropdown(f, app, f.area());
    }
    if app.show_about {
        draw_about_modal(f, app, f.area());
    }
    if app.show_delete_confirm {
        draw_delete_confirm_modal(f, app, f.area());
    }
}

fn draw_menu_bar(f: &mut Frame, app: &App, area: Rect) {
    let lang = app.settings.lang;
    let mut spans = vec![Span::styled(MENU_LOGO, Style::default().bg(Color::Black))];
    let mut used = MENU_LOGO.chars().count() as u16;
    for (i, def) in app.menu.defs.iter().enumerate() {
        let label = format!(" {} ", i18n::t(lang, def.title_key));
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
        spans.push(Span::styled(label, style));
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
    let items: Vec<ListItem> = app.menu.defs[app.menu.menu_index]
        .items
        .iter()
        .map(|i| {
            let label = i18n::t(lang, i.label_key);
            let line = match i.shortcut {
                Some(sc) => {
                    let content_width = inner_width.saturating_sub(2);
                    let pad = content_width.saturating_sub(label.chars().count() + sc.chars().count()).max(1);
                    format!(" {}{}{} ", label, " ".repeat(pad), sc)
                }
                None => format!(" {} ", label),
            };
            ListItem::new(line)
        })
        .collect();
    let mut state = ListState::default();
    state.select(Some(app.menu.item_index));
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

fn draw_file_tree(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::FileTree;
    let block = Block::default()
        .title(format!(" {} ", i18n::t(app.settings.lang, Key::PanelFile)))
        .borders(Borders::ALL)
        .border_style(focused_border_style(focused));

    let items: Vec<ListItem> = app
        .file_tree
        .visible
        .iter()
        .map(|entry| {
            if entry.is_up {
                return ListItem::new(Line::from("  .."));
            }
            let indent = "  ".repeat(entry.depth);
            let icon = if entry.is_dir {
                if entry.expanded { "▾ " } else { "▸ " }
            } else {
                "  "
            };
            let label = format!("{indent}{icon}{}", entry.name);
            ListItem::new(Line::from(label))
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

fn draw_tab_bar(f: &mut Frame, app: &App, area: Rect) {
    let lang = app.settings.lang;
    let mut spans = Vec::new();
    for (i, editor) in app.editors.iter().enumerate() {
        let dirty = if editor.dirty { "*" } else { "" };
        let label = format!(" {}{} ", editor.title(lang), dirty);
        let style = if i == app.active_editor {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(Color::Gray).bg(Color::DarkGray)
        };
        spans.push(Span::styled(label, style));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_editor(f: &mut Frame, app: &mut App, area: Rect) {
    let (tab_bar_area, content_area) = split_editor_area(area);
    if tab_bar_area.height > 0 {
        draw_tab_bar(f, app, tab_bar_area);
    }

    let idx = app.active_editor;
    let focused = app.focus == Focus::Editor;
    let dirty_marker = if app.editors[idx].dirty { " *" } else { "" };
    let title = format!(" {}{} ", app.editors[idx].title(app.settings.lang), dirty_marker);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(focused_border_style(focused));

    let inner = block.inner(content_area);
    let total_lines = app.editors[idx].rope.len_lines();
    let gutter = gutter_width(total_lines, app.settings.show_line_numbers);
    let text_width = inner.width.saturating_sub(gutter) as usize;
    let viewport_height = inner.height as usize;
    app.editor_viewport = (viewport_height, text_width);
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

fn draw_terminals(f: &mut Frame, app: &mut App, term_areas: &[Rect]) {
    let active = app.active_terminal;
    let focus_terminal = app.focus == Focus::Terminal;
    for (i, area) in term_areas.iter().enumerate() {
        let is_focused_pane = focus_terminal && i == active;
        draw_single_terminal(f, app, *area, i, is_focused_pane);
    }
}

fn draw_single_terminal(f: &mut Frame, app: &mut App, area: Rect, index: usize, focused: bool) {
    let title = i18n::terminal_title(app.settings.lang, index);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(focused_border_style(focused));
    let inner = block.inner(area);

    let rows = inner.height;
    let cols = inner.width;

    let Some(terminal) = app.terminals.get_mut(index) else { return };
    terminal.resize(rows, cols);

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

    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, area);

    if let Some((cy, cx)) = cursor_pos {
        f.set_cursor_position((inner.x + cx, inner.y + cy));
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
