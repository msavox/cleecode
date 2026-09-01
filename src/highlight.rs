use ratatui::style::{Color, Modifier, Style};
use ropey::Rope;
use std::borrow::Cow;
use std::path::Path;
use syntect::highlighting::{
    FontStyle, HighlightIterator, HighlightState, Highlighter as ThemeHighlighter, Theme, ThemeSet,
};
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};

/// How often a resumable state is kept, in lines.
///
/// One per line would be two heap allocations per line of the file, held for the lifetime of the
/// buffer, to save work that is cheap to redo: an edit resumes from the rung at or below it and
/// replays at most `STRIDE - 1` lines to reach the edit. Sixty-four lines is well under a
/// screenful of replay and a sixty-fourth of the memory.
const STRIDE: usize = 64;

pub struct Highlighter {
    syntax_set: SyntaxSet,
    theme: Theme,
}

/// A line's worth of resumable state: what the grammar was in the middle of, and what the theme
/// had made of it, as of the moment before that line is read.
///
/// Highlighting a file is a fold over its lines — line n's colours depend on every line above it,
/// which is why an editor that keeps no state has to re-read the file from the top after every
/// keystroke. Keeping the fold's accumulator is what lets it be resumed instead.
#[derive(Clone)]
struct LineState {
    parse: ParseState,
    theme: HighlightState,
}

/// How far one buffer's highlighting has got, and what it would need to carry on.
///
/// The spans themselves live beside this in the editor, and the pair is one cache in two pieces:
/// the first `valid` lines are done and the rest are not *stale*, they are simply not computed —
/// they are made when the view reaches them. An edit drops the watermark back to the edited line
/// instead of throwing the file away, so typing costs the lines after the cursor rather than all
/// of them.
#[derive(Default)]
pub struct LineCache {
    /// The language the cached spans were made with. A file renamed into another language has to
    /// start over, and this is what notices.
    syntax: Option<String>,
    /// `ladder[k]` is the state entering line `k * STRIDE`.
    ladder: Vec<LineState>,
    /// The state entering line `valid`, so carrying on from the watermark costs nothing.
    next: Option<LineState>,
    valid: usize,
}

impl LineCache {
    /// Forgets the lot: a different language, or a buffer replaced wholesale.
    pub fn clear(&mut self) {
        *self = LineCache::default();
    }

    /// Drops the watermark to take in an edit on `line`, and answers how many lines of spans
    /// survive it. Nothing above the edited line can be affected: the state a line is read in is
    /// settled before that line is looked at.
    pub fn invalidate_from(&mut self, line: usize) -> usize {
        if line >= self.valid {
            return self.valid;
        }
        let rung = line / STRIDE;
        self.valid = rung * STRIDE;
        self.ladder.truncate(rung + 1);
        self.next = self.ladder.last().cloned();
        self.valid
    }

    /// How many leading lines are highlighted.
    pub fn valid_lines(&self) -> usize {
        self.valid
    }
}

impl Highlighter {
    /// A highlighter for the default theme, for the tests that care about grammars rather than
    /// about colours. Everything that draws goes through `for_theme` instead.
    #[cfg(test)]
    pub fn new() -> Self {
        Self::for_theme(crate::theme::Theme::default())
    }

    /// A highlighter coloured by `theme`. Rebuilt rather than re-tinted when the theme changes:
    /// a syntect `Theme` is the thing the highlighter borrows from on every line, and swapping
    /// the field under it is the same work as building a new one.
    pub fn for_theme(theme: crate::theme::Theme) -> Self {
        Self::named(theme.syntax_theme())
    }

    fn named(wanted: &str) -> Self {
        // syntect's own defaults are the Sublime Text packages and nothing else, which in 2026
        // is a strange set of languages to have: no TypeScript, no TOML, no Kotlin, no Swift,
        // no Zig, no Dockerfile, no Vue. two-face ships bat's collection — those defaults plus
        // a hundred-odd grammars the world actually writes — as a precompiled dump, so this
        // costs a load from a byte slice rather than parsing grammars at startup.
        let syntax_set = two_face::syntax::extra_newlines();
        let theme_set = ThemeSet::load_defaults();
        let theme = theme_set
            .themes
            .get(wanted)
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

    /// Brings `spans` up to and including line `through`, carrying on from wherever the last call
    /// left off. `cache` and `spans` are the two halves of one cache: pass the same pair every
    /// time, for the same buffer.
    ///
    /// The buffer is read a line at a time straight from the rope. Copying it into one String
    /// first — which is what a whole-file highlight has to do — is a full copy of the file per
    /// keystroke, before a single line has been parsed.
    pub fn extend_to(
        &self,
        path: Option<&Path>,
        rope: &Rope,
        through: usize,
        cache: &mut LineCache,
        spans: &mut Vec<Vec<(Style, String)>>,
    ) {
        let first_line: Cow<str> = rope.line(0).into();
        let syntax = self.syntax_for(path, &first_line);
        // Either half without the other is not a cache at all, and neither is one made with a
        // grammar the file is no longer being read with.
        if spans.len() != cache.valid || cache.syntax.as_deref() != Some(syntax.name.as_str()) {
            cache.clear();
            spans.clear();
            cache.syntax = Some(syntax.name.clone());
        }
        let through = through.min(rope.len_lines().saturating_sub(1));
        if cache.valid > through {
            return;
        }
        // Built per call rather than kept: it borrows the theme, and a struct holding both would
        // have to borrow from itself. It is one walk over the theme's scopes, once a frame.
        let theme = ThemeHighlighter::new(&self.theme);
        let mut state = cache.next.clone().unwrap_or_else(|| LineState {
            parse: ParseState::new(syntax),
            theme: HighlightState::new(&theme, ScopeStack::new()),
        });
        while cache.valid <= through {
            if cache.valid.is_multiple_of(STRIDE) && cache.ladder.len() == cache.valid / STRIDE {
                cache.ladder.push(state.clone());
            }
            let line: Cow<str> = rope.line(cache.valid).into();
            spans.push(highlight_one(&mut state, &line, &self.syntax_set, &theme));
            cache.valid += 1;
        }
        cache.next = Some(state);
    }
}

/// One line, coloured in the state the line above it left behind.
///
/// The line terminator is trimmed off and empty runs dropped: the renderer draws lines, not the
/// breaks between them, and a span holding only a newline would take a cell that isn't there.
fn highlight_one(
    state: &mut LineState,
    line: &str,
    syntax_set: &SyntaxSet,
    theme: &ThemeHighlighter,
) -> Vec<(Style, String)> {
    let ranges = match state.parse.parse_line(line, syntax_set) {
        Ok(ops) => HighlightIterator::new(&mut state.theme, &ops, line, theme).collect::<Vec<_>>(),
        Err(_) => vec![(syntect::highlighting::Style::default(), line)],
    };
    let mut spans: Vec<(Style, String)> = Vec::new();
    for (sstyle, text_part) in ranges {
        let trimmed = text_part.trim_end_matches('\n');
        if trimmed.is_empty() {
            continue;
        }
        spans.push((convert_style(sstyle), trimmed.to_string()));
    }
    spans
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

    /// A file long enough to span several rungs of the ladder, with one line of the caller's
    /// choosing in the middle of it.
    fn sample(at: usize, line: &str) -> Rope {
        let text: Vec<String> = (0..130)
            .map(|i| {
                if i == at {
                    line.to_string()
                } else {
                    format!("fn f{i}() {{ let v = {i}; }}")
                }
            })
            .collect();
        Rope::from_str(&text.join("\n"))
    }

    fn from_scratch(hl: &Highlighter, rope: &Rope) -> Vec<Vec<(Style, String)>> {
        let mut cache = LineCache::default();
        let mut spans = Vec::new();
        hl.extend_to(Some(Path::new("sample.rs")), rope, usize::MAX, &mut cache, &mut spans);
        spans
    }

    /// The one that matters: resuming must be indistinguishable from starting over. The edit
    /// opens a block comment, so every line below it changes colour — a resumed pass that
    /// quietly kept what it had would be caught here rather than on screen.
    #[test]
    fn an_edited_line_re_highlights_to_the_same_spans_as_a_fresh_pass() {
        let hl = Highlighter::new();
        let path = Some(Path::new("sample.rs"));
        let before = sample(70, "fn f70() { let v = 70; }");
        let after = sample(70, "/* f70 is out of service");

        let mut cache = LineCache::default();
        let mut spans = Vec::new();
        hl.extend_to(path, &before, usize::MAX, &mut cache, &mut spans);
        assert_eq!(cache.valid_lines(), before.len_lines());

        spans.truncate(cache.invalidate_from(70));
        assert_eq!(cache.valid_lines(), 64, "resumed from the rung below the edit, not from the top");
        hl.extend_to(path, &after, usize::MAX, &mut cache, &mut spans);

        assert_eq!(spans, from_scratch(&hl, &after));
    }

    /// A keystroke pays for the screen it happens on, not for the file it happens in.
    #[test]
    fn lines_below_the_one_asked_for_are_left_for_later() {
        let hl = Highlighter::new();
        let path = Some(Path::new("sample.rs"));
        let rope = sample(0, "fn f0() {}");
        let mut cache = LineCache::default();
        let mut spans = Vec::new();

        hl.extend_to(path, &rope, 9, &mut cache, &mut spans);
        assert_eq!(spans.len(), 10);
        hl.extend_to(path, &rope, 19, &mut cache, &mut spans);
        assert_eq!(spans.len(), 20, "the rest arrives as the view reaches it");
        assert_eq!(spans[..10], from_scratch(&hl, &rope)[..10]);
    }

    /// Save As renames a buffer into another language; the spans it already has were made with
    /// the wrong grammar and none of them survive.
    #[test]
    fn a_buffer_read_as_another_language_starts_over() {
        let hl = Highlighter::new();
        let rope = Rope::from_str("# heading\nfn f() {}\n");
        let mut cache = LineCache::default();
        let mut spans = Vec::new();
        hl.extend_to(Some(Path::new("notes.md")), &rope, usize::MAX, &mut cache, &mut spans);
        let as_markdown = spans.clone();

        hl.extend_to(Some(Path::new("notes.rs")), &rope, usize::MAX, &mut cache, &mut spans);
        assert_ne!(spans, as_markdown);
        assert_eq!(spans, from_scratch(&hl, &rope));
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

