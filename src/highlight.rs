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
        // syntect's own defaults are the Sublime Text packages and nothing else, which in 2026
        // is a strange set of languages to have: no TypeScript, no TOML, no Kotlin, no Swift,
        // no Zig, no Dockerfile, no Vue. two-face ships bat's collection — those defaults plus
        // a hundred-odd grammars the world actually writes — as a precompiled dump, so this
        // costs a load from a byte slice rather than parsing grammars at startup.
        let syntax_set = two_face::syntax::extra_newlines();
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
        let name = path.and_then(|p| p.file_name()).and_then(|n| n.to_str());
        let ext = path.and_then(|p| p.extension()).and_then(|e| e.to_str());
        // Whole-file-name first, because it is the more specific claim: `CMakeLists.txt` is a
        // CMake file, and matching its `.txt` before its name would call it plain prose.
        // Grammars register those names in the same list as their extensions.
        let by_name = name.and_then(|name| self.syntax_set.find_syntax_by_extension(name));
        let by_ext = ext.and_then(|ext| self.syntax_set.find_syntax_by_extension(ext));
        // A grammar lists the extensions it had when it was written, so a language that later
        // grew a spelling (`.cjs`, `.jsonc`) or a dialect close enough to read as its parent
        // (`.astro` as HTML) matches nothing until it is pointed at a grammar that fits.
        // Tried after both real lookups, never in place of them.
        let alias = |token: &str| aliased_syntax(&token.to_ascii_lowercase());
        let by_alias = ext
            .and_then(alias)
            .or_else(|| name.and_then(alias))
            .and_then(|ext| self.syntax_set.find_syntax_by_extension(ext));
        by_name
            .or(by_ext)
            .or(by_alias)
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

/// An extension or file name the grammars don't know, mapped onto one they do.
///
/// Only where the answer is honest: a dialect of the language it maps to (`.cjs` is
/// JavaScript), a superset of its file format (`.jsonc` is JSON with comments), or a template
/// language whose surroundings are the thing being written (`.astro`, `.hbs` are HTML with
/// expressions in it — the expressions come out unstyled, everything around them doesn't).
/// A language that merely looks similar is left plain: guessing wrong colours a keyword that
/// isn't one, which reads worse than no colour at all.
fn aliased_syntax(token: &str) -> Option<&'static str> {
    Some(match token {
        "mjs" | "cjs" | "jsx" => "js",
        "mts" | "cts" => "ts",
        "jsonc" | "json5" => "json",
        "mdx" => "md",
        "containerfile" => "dockerfile",
        "editorconfig" | "service" | "npmrc" | "nvmrc" => "ini",
        "plist" | "xaml" | "resx" | "csproj" | "vcxproj" | "props" | "targets" => "xml",
        // Starlark: Python's grammar, Bazel's job. The `.bzl`/`BUILD` spellings are known
        // already, `.star` is the one that isn't.
        "star" => "py",
        "s" => "asm",
        "hlsl" => "glsl",
        "astro" | "ejs" | "njk" | "hbs" | "handlebars" | "mustache" | "liquid" | "tpl"
        | "cshtml" | "razor" | "vm" => "html",
        "just" | "justfile" => "make",
        // Ruby by convention rather than by extension.
        "gemfile" | "rakefile" | "vagrantfile" | "brewfile" | "podfile" | "guardfile"
        | "capfile" | "fastfile" | "appfile" => "rb",
        _ => return None,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn syntax_name(file: &str, text: &str) -> String {
        Highlighter::new().syntax_for(Some(Path::new(file)), text).name.clone()
    }

    /// The set the editor is expected to know. TypeScript in particular was plain text for as
    /// long as the grammars were syntect's own defaults, which stop at the Sublime packages.
    #[test]
    fn the_languages_people_open_are_highlighted() {
        for (file, expected) in [
            ("api.ts", "TypeScript"),
            ("App.tsx", "TypeScriptReact"),
            ("main.rs", "Rust"),
            ("Cargo.toml", "TOML"),
            ("app.vue", "Vue Component"),
            ("Counter.svelte", "Svelte"),
            ("Main.kt", "Kotlin"),
            ("View.swift", "Swift"),
            ("main.zig", "Zig"),
            ("main.dart", "Dart"),
            ("schema.graphql", "GraphQL"),
            ("main.tf", "Terraform"),
            ("flake.nix", "Nix"),
            ("Dockerfile", "Dockerfile"),
            ("query.sql", "SQL"),
            ("go.mod", "Gomod"),
        ] {
            assert_eq!(syntax_name(file, ""), expected, "{file}");
        }
    }

    /// Spellings and dialects with no grammar of their own, read as the language they are.
    #[test]
    fn near_relatives_borrow_a_grammar() {
        assert_eq!(syntax_name("build.cjs", ""), "JavaScript");
        assert_eq!(syntax_name("tsconfig.jsonc", ""), "JSON");
        assert_eq!(syntax_name("page.astro", ""), "HTML");
        assert_eq!(syntax_name("Gemfile", ""), "Ruby");
    }

    /// A file name is a more specific claim than the extension on the end of it, so it is asked
    /// first: `.txt` would otherwise make this plain prose.
    #[test]
    fn a_known_file_name_beats_its_extension() {
        assert_eq!(syntax_name("CMakeLists.txt", ""), "CMake");
    }

    /// No extension, no known name: the shebang is the only thing left saying what it is.
    #[test]
    fn a_shebang_names_a_script_with_no_extension() {
        assert_eq!(syntax_name("deploy", "#!/usr/bin/env python3\nprint(1)\n"), "Python");
    }

    /// Nothing recognisable stays unstyled rather than being guessed at.
    #[test]
    fn an_unknown_file_falls_back_to_plain_text() {
        assert_eq!(syntax_name("dump.qqq", "zzz\n"), "Plain Text");
    }
}
