use ratatui::style::{Color, Modifier, Style};
use std::path::Path;
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};

pub struct Highlighter {
    syntax_set: SyntaxSet,
    theme: Theme,
}

impl Highlighter {
    pub fn new() -> Self {
        let syntax_set = SyntaxSet::load_defaults_newlines();
        let theme_set = ThemeSet::load_defaults();
        let theme = theme_set
            .themes
            .get("base16-ocean.dark")
            .or_else(|| theme_set.themes.values().next())
            .expect("syntect ships at least one default theme")
            .clone();
        Highlighter { syntax_set, theme }
    }

    fn syntax_for(&self, path: Option<&Path>, text: &str) -> &SyntaxReference {
        let by_ext = path
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .and_then(|ext| self.syntax_set.find_syntax_by_extension(ext));
        let by_name = path
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .and_then(|name| self.syntax_set.find_syntax_by_extension(name));
        by_ext
            .or(by_name)
            .or_else(|| {
                text.lines()
                    .next()
                    .and_then(|first| self.syntax_set.find_syntax_by_first_line(first))
            })
            .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text())
    }

    /// Highlight the whole buffer, returning one Vec of (style, text) runs per line.
    pub fn highlight(&self, path: Option<&Path>, text: &str) -> Vec<Vec<(Style, String)>> {
        let syntax = self.syntax_for(path, text);
        let mut h = HighlightLines::new(syntax, &self.theme);
        let mut out = Vec::new();

        for line in text.split_inclusive('\n') {
            let ranges = h
                .highlight_line(line, &self.syntax_set)
                .unwrap_or_else(|_| vec![(syntect::highlighting::Style::default(), line)]);
            let mut spans: Vec<(Style, String)> = Vec::new();
            for (sstyle, text_part) in ranges {
                let trimmed = text_part.trim_end_matches('\n');
                if trimmed.is_empty() {
                    continue;
                }
                spans.push((convert_style(sstyle), trimmed.to_string()));
            }
            out.push(spans);
        }
        if text.is_empty() {
            out.push(Vec::new());
        }
        out
    }
}

fn convert_style(s: syntect::highlighting::Style) -> Style {
    let mut style = Style::default().fg(Color::Rgb(s.foreground.r, s.foreground.g, s.foreground.b));
    if s.font_style.contains(FontStyle::BOLD) {
        style = style.add_modifier(Modifier::BOLD);
    }
    if s.font_style.contains(FontStyle::ITALIC) {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if s.font_style.contains(FontStyle::UNDERLINE) {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    style
}
