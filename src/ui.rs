use crate::app::{App, EditorPane, Focus};
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
/// Splits the whole editor region into one (unsplit) or two side-by-side panes. Index 0
/// is always the left/only pane, index 1 (when present) the right one.
pub fn editor_pane_rects(area: Rect, split: bool) -> Vec<Rect> {
    if !split {
        return vec![area];
    }
    let mid = area.width / 2;
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

pub fn tab_layouts(app: &App) -> Vec<TabLayout> {
    let lang = app.settings.lang;
    let mut out = Vec::new();
    let mut x = 0u16;
    for editor in &app.editors {
        let dirty = if editor.dirty { "*" } else { "" };
        let prefix = format!(" {}{} ", editor.title(lang), dirty);
        let prefix_w = prefix.chars().count() as u16;
        let close_start = x + prefix_w;
        let close_end = close_start + 1;
        let tab_end = close_end + 1; // trailing space after the × glyph
        out.push(TabLayout { full: (x, tab_end), close: (close_start, close_end) });
        x = tab_end;
    }
    out
}

pub fn tab_ranges(app: &App) -> Vec<(u16, u16)> {
    tab_layouts(app).into_iter().map(|t| t.full).collect()
}

fn run_button_label(lang: Lang) -> String {
    format!(" \u{25b6} {} ", i18n::t(lang, Key::ToolbarRun))
}

fn venv_button_label(app: &App) -> String {
    match &app.settings.active_venv {
        Some(name) => format!(" venv: {name} \u{25be} "),
        None => format!(" {} \u{25be} ", i18n::t(app.settings.lang, Key::ToolbarVenvNone)),
    }
}

/// Relative (start, end) ranges for the right-aligned toolbar buttons that fit within
/// `area_width` without overlapping the open tabs: the venv selector (dropped first if
/// there isn't room for both) and the Run button.
pub fn toolbar_button_ranges(app: &App, area_width: u16) -> (Option<(u16, u16)>, Option<(u16, u16)>) {
    let used: u16 = tab_ranges(app).last().map(|(_, e)| *e).unwrap_or(0);
    let run_w = run_button_label(app.settings.lang).chars().count() as u16;
    let venv_w = venv_button_label(app).chars().count() as u16;

    if used + venv_w + run_w <= area_width {
        let run_start = area_width - run_w;
        let venv_start = run_start - venv_w;
        (Some((venv_start, venv_start + venv_w)), Some((run_start, run_start + run_w)))
    } else if used + run_w <= area_width {
        let run_start = area_width - run_w;
        (None, Some((run_start, run_start + run_w)))
    } else {
        (None, None)
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
    if app.show_rename {
        draw_rename_modal(f, app, f.area());
    }
}

fn draw_menu_bar(f: &mut Frame, app: &App, area: Rect) {
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
        .border_style(focused_border_style(focused));

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

fn draw_tab_bar(f: &mut Frame, app: &App, area: Rect, active_idx: usize, show_toolbar: bool) {
    let lang = app.settings.lang;
    let mut spans = Vec::new();
    let mut used = 0u16;
    for (i, editor) in app.editors.iter().enumerate() {
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
    let (venv_range, run_range) = if show_toolbar { toolbar_button_ranges(app, area.width) } else { (None, None) };
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
    let panes = editor_pane_rects(area, app.split_view);
    if panes.len() == 1 {
        let focused = app.focus == Focus::Editor;
        draw_editor_pane(f, app, panes[0], app.active_editor, focused, true);
        return;
    }
    let left_focused = app.focus == Focus::Editor && app.editor_pane_focus == EditorPane::Left;
    let right_focused = app.focus == Focus::Editor && app.editor_pane_focus == EditorPane::Right;
    draw_editor_pane(f, app, panes[0], app.active_editor, left_focused, true);
    draw_editor_pane(f, app, panes[1], app.active_editor_right, right_focused, false);
}

fn draw_editor_pane(f: &mut Frame, app: &mut App, area: Rect, idx: usize, focused: bool, show_toolbar: bool) {
    let (tab_bar_area, content_area) = split_editor_area(area);
    if tab_bar_area.height > 0 {
        draw_tab_bar(f, app, tab_bar_area, idx, show_toolbar);
    }

    // No title here: the open tab right above already shows the filename and dirty marker.
    let block = Block::default().borders(Borders::ALL).border_style(focused_border_style(focused));

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
