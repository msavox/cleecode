//! The colours the interface is drawn in, in one place.
//!
//! Every colour outside this file used to be written where it was used, which worked for exactly
//! one terminal: a very dark one. The palette is a struct rather than a configuration file on
//! purpose — first the colours are separated from the drawing, and only then, if it is asked for,
//! is the separation exposed. A field here is a *role*, not a colour: `warning` is what a modified
//! file and an unsaved buffer share, and a theme is free to make that orange, brown or yellow.
//!
//! Two colours deliberately stay out. The file-type icons carry the colours of the things they
//! stand for — Rust's orange, Python's blue — and a theme that repainted them would be renaming
//! them. The drawing on the About box is likewise its own.

use ratatui::style::Color;
use serde::{Deserialize, Serialize};

/// One theme's worth of colours.
///
/// `Copy` because it is: a couple of dozen `Color`s, each a small enum. That is what lets it be
/// passed to the drawing helpers by value-sized reference without anyone having to think about
/// lifetimes, the same way the language is passed to everything that writes a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// The surface the editor paints when it will not trust the terminal's own background. Also
    /// what the opaque-background pass fills every unclaimed cell with.
    pub background: Color,
    /// Ordinary text — the colour a cell nothing else has claimed ends up with. `Reset` means
    /// "whatever the terminal draws text in", which is right for a theme that means to sit on the
    /// terminal's own colours and wrong for one that brings its own surface.
    pub text: Color,
    /// Labels, inactive tabs, the prompt beside a field: present, but not the thing being read.
    pub text_muted: Color,
    /// Hints, counts, separators, the disabled half of a toggle. One step further back than muted.
    pub text_dim: Color,
    /// The colour of "this one, right now": a focused border, the selected row, an active toggle.
    pub accent: Color,
    /// Text on top of `accent`, `success` or `warning` used as a background.
    pub on_accent: Color,
    /// The menu bar's own background, and the padding either side of it.
    pub bar: Color,
    /// Text on the menu bar that is not lit.
    pub on_bar: Color,
    /// The small raised surfaces: the buttons on the picture toolbar and the markdown bar.
    pub surface: Color,
    /// The row those buttons sit on, a shade behind them.
    pub surface_dim: Color,
    /// The background of a tab that is not the one you are looking at.
    pub tab_inactive: Color,
    /// Added, tracked, on the current branch, went through.
    pub success: Color,
    /// Modified, unsaved, stashed, worth reading before continuing.
    pub warning: Color,
    /// Deleted, failed, refused, about to destroy work.
    pub danger: Color,
    /// A third and fourth category with no ranking between them: the sources a completion came
    /// from, the lanes of the commit graph.
    pub info: Color,
    pub special: Color,
    /// The graph's sixth lane, which needs a colour the other five have not taken.
    pub graph_extra: Color,
    /// What you have typed into a field, as against the label beside it.
    pub input: Color,
    /// The brightest text the theme has, for the one line that has to win: the picker's query.
    pub bright: Color,
    /// The background of selected text in the editor.
    pub selection: Color,
    /// The background of a line the editor is pointing at — a search hit, the line being run.
    pub current_line: Color,
    /// The number of a line that arrived from outside while the file was open: an agent's write,
    /// a formatter, a branch switched under the buffer.
    ///
    /// Green, the way a diff draws the lines that were added. The gutter has no other green —
    /// every mark it can already draw is red (a breakpoint, an error), yellow (the stopped line,
    /// a warning, the cursor's own line) or the accent — so this reads as its own thing without
    /// anybody having to be told which of two similar colours they are looking at.
    pub changed_line: Color,
    /// Folders in the tree. A colour, not an icon colour: it belongs to the tree, not to a type.
    pub folder: Color,
    /// The colour the initial of a menu entry is drawn in, for a theme that wants one — the DOS
    /// IDEs marked the letter you could press, and the mark is half of what that look *is*.
    /// `None` leaves the initial the colour of the rest of the word, which is what the themes
    /// that are not quoting anything want.
    pub accelerator: Option<Color>,
    /// Whether the chrome — the menu bar, the tabs, the status line — is drawn bold.
    ///
    /// Worth being clear about what this does and does not buy. On a terminal drawing the sixteen
    /// named colours, bold is how the bright half of the palette is reached, and that is where
    /// the idea comes from. These themes state their colours in RGB, where bold changes the
    /// weight of the glyph and not its colour — so this is not a way to get a brighter red. What
    /// it is good for is the other half of the DOS look: a text mode drew a heavy bitmap font,
    /// and a chrome drawn bold sits closer to that than the same colours drawn thin.
    pub bold_chrome: bool,
    /// The border of a pane being dragged. Deliberately outside the rest of the palette — it says
    /// "you are holding this", and a theme that made it blend in would be answering the wrong
    /// question.
    pub resize_border: Color,
}

impl Palette {
    /// Whether this palette's text can be read on a background that is not its own.
    ///
    /// `Reset` means the text is drawn in whatever colour the terminal writes in, which cannot
    /// clash with the terminal's own background by construction. Otherwise it comes down to how
    /// light the text is: dark text needs the light surface it was picked for, and picking it is
    /// what committed the theme to painting.
    pub fn needs_its_own_background(self) -> bool {
        match self.text {
            // Named colours are the terminal's own, and follow it wherever it goes.
            Color::Rgb(r, g, b) => {
                // Rec. 601 luma, integer arithmetic so this stays usable in a const context
                // later if it needs to be. Half-bright is the line: below it the text is being
                // drawn for paper.
                (299 * r as u32 + 587 * g as u32 + 114 * b as u32) / 1000 < 128
            }
            _ => false,
        }
    }
}

/// The themes that ship with CleeCode.
///
/// `CleeCode` is what the editor has always looked like, given a name so it can be one choice
/// among several rather than the absence of a choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    #[default]
    #[serde(rename = "cleecode")]
    CleeCode,
    #[serde(rename = "cleecode-light")]
    CleeCodeLight,
    Turbo,
    SolarizedDark,
    SolarizedLight,
    Eighties,
    Mocha,
    OceanLight,
    #[serde(rename = "github")]
    GitHub,
}

impl Theme {
    /// Dark ones first, then light, each group in the order they were added. The picker shows
    /// them in this order and nothing sorts it, so this list *is* the menu.
    pub const ALL: [Theme; 9] = [
        Theme::CleeCode,
        Theme::Turbo,
        Theme::SolarizedDark,
        Theme::Eighties,
        Theme::Mocha,
        Theme::CleeCodeLight,
        Theme::SolarizedLight,
        Theme::OceanLight,
        Theme::GitHub,
    ];

    /// The name shown in the picker. Not translated: a theme's name is a name, and "Turbo" reads
    /// the same in every language the editor speaks.
    pub fn name(self) -> &'static str {
        match self {
            Theme::CleeCode => "CleeCode",
            Theme::CleeCodeLight => "CleeCode Light",
            Theme::Turbo => "Turbo",
            Theme::SolarizedDark => "Solarized Dark",
            Theme::SolarizedLight => "Solarized Light",
            Theme::Eighties => "Eighties",
            Theme::Mocha => "Mocha",
            Theme::OceanLight => "Ocean Light",
            Theme::GitHub => "GitHub",
        }
    }

    /// Whether the theme has to paint its own surface rather than leaving the choice to the user.
    ///
    /// The answer is read off the palette rather than listed here, because the thing that decides
    /// it is the text colour and nothing else. A theme with light text is making one assumption
    /// about the terminal behind it — that it is dark — and that is the same assumption the user
    /// made by choosing a dark theme, so it can be left translucent for anyone who likes seeing
    /// their wallpaper through the editor. A theme with dark text cannot: unpainted, its text
    /// lands on whatever the terminal's background is, which for most people is black on black.
    pub fn paints_its_own_background(self) -> bool {
        self.palette().needs_its_own_background()
    }

    /// The syntect theme the highlighter is built with. The four named here are compiled into the
    /// binary already, by syntect's own defaults — choosing one costs nothing.
    pub fn syntax_theme(self) -> &'static str {
        match self {
            Theme::CleeCode => "base16-ocean.dark",
            Theme::CleeCodeLight => "base16-ocean.light",
            // Blue on blue would be unreadable; the eighties palette is the warmest dark one
            // syntect ships, which is as close as a stock theme gets to the DOS look.
            Theme::Turbo => "base16-eighties.dark",
            Theme::SolarizedDark => "Solarized (dark)",
            Theme::SolarizedLight => "Solarized (light)",
            Theme::Eighties => "base16-eighties.dark",
            Theme::Mocha => "base16-mocha.dark",
            Theme::OceanLight => "base16-ocean.light",
            Theme::GitHub => "InspiredGitHub",
        }
    }

    pub fn palette(self) -> Palette {
        match self {
            Theme::CleeCode => CLEECODE,
            Theme::CleeCodeLight => CLEECODE_LIGHT,
            Theme::Turbo => TURBO,
            Theme::SolarizedDark => SOLARIZED_DARK,
            Theme::SolarizedLight => SOLARIZED_LIGHT,
            Theme::Eighties => EIGHTIES,
            Theme::Mocha => MOCHA,
            Theme::OceanLight => OCEAN_LIGHT,
            Theme::GitHub => GITHUB,
        }
    }
}

/// What the editor has always looked like. Every field is the colour that used to be written at
/// the point of use, so the default theme draws the same frame it drew before the palette existed
/// — which is the only way a change this wide can be checked by reading it.
const CLEECODE: Palette = Palette {
    background: Color::Rgb(24, 24, 24),
    text: Color::Reset,
    text_muted: Color::Gray,
    text_dim: Color::DarkGray,
    accent: Color::Cyan,
    on_accent: Color::Black,
    bar: Color::Black,
    on_bar: Color::Gray,
    surface: Color::Rgb(45, 45, 45),
    surface_dim: Color::Rgb(30, 30, 30),
    tab_inactive: Color::DarkGray,
    success: Color::Green,
    warning: Color::Yellow,
    danger: Color::Red,
    info: Color::Blue,
    special: Color::Magenta,
    graph_extra: Color::LightGreen,
    input: Color::Yellow,
    bright: Color::White,
    selection: Color::Rgb(60, 90, 130),
    current_line: Color::Rgb(70, 60, 0),
    // The one bright green among the named colours, and the theme is built out of those.
    changed_line: Color::LightGreen,
    folder: Color::Rgb(120, 170, 255),
    accelerator: None,
    bold_chrome: false,
    resize_border: Color::Rgb(255, 140, 0),
};

/// The same editor on a white desk. The accents keep their meanings and lose their brightness:
/// a cyan that reads as "selected" on near-black is a haze on white, so the roles that carry
/// meaning are darkened until they hold their own against the paper.
const CLEECODE_LIGHT: Palette = Palette {
    background: Color::Rgb(250, 250, 250),
    text: Color::Rgb(32, 34, 38),
    text_muted: Color::Rgb(88, 94, 102),
    text_dim: Color::Rgb(134, 140, 148),
    accent: Color::Rgb(0, 92, 153),
    on_accent: Color::Rgb(255, 255, 255),
    bar: Color::Rgb(222, 224, 228),
    on_bar: Color::Rgb(48, 52, 58),
    surface: Color::Rgb(232, 234, 238),
    surface_dim: Color::Rgb(214, 217, 222),
    tab_inactive: Color::Rgb(206, 210, 216),
    success: Color::Rgb(22, 116, 58),
    warning: Color::Rgb(148, 94, 0),
    danger: Color::Rgb(176, 32, 40),
    info: Color::Rgb(38, 82, 176),
    special: Color::Rgb(132, 44, 148),
    graph_extra: Color::Rgb(60, 132, 76),
    input: Color::Rgb(122, 76, 0),
    bright: Color::Rgb(16, 18, 20),
    selection: Color::Rgb(198, 216, 240),
    current_line: Color::Rgb(248, 240, 198),
    // Darker than the dark theme's green by the same rule as everything else here: on paper a
    // bright one is a haze, and the gutter is small text.
    changed_line: Color::Rgb(20, 132, 70),
    folder: Color::Rgb(38, 96, 168),
    accelerator: None,
    bold_chrome: false,
    resize_border: Color::Rgb(200, 96, 0),
};

/// The blue screen, for anyone who learned to program on one.
///
/// The palette is the EGA sixteen, which is the whole point: those machines had no others, and a
/// close-but-modern blue reads as "a blue theme" rather than as the thing it is quoting. The
/// chrome inverts — a light grey bar with dark text over a blue field — which is why the palette
/// needs `bar` and `on_bar` at all, and why this theme is worth having as the second one written:
/// it is the case that proves the roles are roles and not a second name for the dark theme.
const TURBO: Palette = Palette {
    background: Color::Rgb(0, 0, 168),
    text: Color::Rgb(170, 170, 170),
    text_muted: Color::Rgb(85, 255, 255),
    text_dim: Color::Rgb(85, 85, 255),
    accent: Color::Rgb(0, 168, 168),
    on_accent: Color::Rgb(0, 0, 0),
    bar: Color::Rgb(170, 170, 170),
    on_bar: Color::Rgb(0, 0, 0),
    surface: Color::Rgb(170, 170, 170),
    surface_dim: Color::Rgb(85, 85, 85),
    tab_inactive: Color::Rgb(85, 85, 85),
    success: Color::Rgb(85, 255, 85),
    warning: Color::Rgb(255, 255, 85),
    danger: Color::Rgb(255, 85, 85),
    info: Color::Rgb(85, 85, 255),
    special: Color::Rgb(255, 85, 255),
    graph_extra: Color::Rgb(0, 170, 0),
    input: Color::Rgb(255, 255, 85),
    bright: Color::Rgb(255, 255, 255),
    selection: Color::Rgb(0, 168, 168),
    current_line: Color::Rgb(0, 0, 85),
    // EGA bright green. There are sixteen colours and no others, which is the theme.
    changed_line: Color::Rgb(85, 255, 85),
    folder: Color::Rgb(255, 255, 85),
    accelerator: Some(Color::Rgb(170, 0, 0)),
    bold_chrome: true,
    resize_border: Color::Rgb(255, 85, 85),
};

/// Ethan Schoonover's Solarized, dark. The surfaces and the accents are the published palette —
/// base03 under everything, base02 for the chrome, and the eight accent hues — which is also what
/// syntect's own copy of the theme highlights code with, so the frame and the code agree.
const SOLARIZED_DARK: Palette = Palette {
    background: Color::Rgb(0, 43, 54),
    text: Color::Rgb(131, 148, 150),
    text_muted: Color::Rgb(101, 123, 131),
    text_dim: Color::Rgb(88, 110, 117),
    accent: Color::Rgb(38, 139, 210),
    on_accent: Color::Rgb(0, 43, 54),
    bar: Color::Rgb(7, 54, 66),
    on_bar: Color::Rgb(147, 161, 161),
    surface: Color::Rgb(7, 54, 66),
    surface_dim: Color::Rgb(0, 43, 54),
    tab_inactive: Color::Rgb(44, 76, 85),
    success: Color::Rgb(133, 153, 0),
    warning: Color::Rgb(181, 137, 0),
    danger: Color::Rgb(220, 50, 47),
    info: Color::Rgb(42, 161, 152),
    special: Color::Rgb(108, 113, 196),
    graph_extra: Color::Rgb(211, 54, 130),
    input: Color::Rgb(181, 137, 0),
    bright: Color::Rgb(253, 246, 227),
    selection: Color::Rgb(44, 76, 85),
    current_line: Color::Rgb(7, 54, 66),
    // Solarized's own green, which is the only green the scheme has and the same on both grounds.
    changed_line: Color::Rgb(133, 153, 0),
    folder: Color::Rgb(38, 139, 210),
    accelerator: None,
    bold_chrome: false,
    resize_border: Color::Rgb(203, 75, 22),
};

/// The same palette on paper: Solarized is one scheme with two backgrounds, and the accents do
/// not move between them. Only the greys are read from the other end of the ramp.
const SOLARIZED_LIGHT: Palette = Palette {
    background: Color::Rgb(253, 246, 227),
    text: Color::Rgb(101, 123, 131),
    text_muted: Color::Rgb(88, 110, 117),
    text_dim: Color::Rgb(147, 161, 161),
    accent: Color::Rgb(38, 139, 210),
    on_accent: Color::Rgb(253, 246, 227),
    bar: Color::Rgb(238, 232, 213),
    on_bar: Color::Rgb(88, 110, 117),
    surface: Color::Rgb(238, 232, 213),
    surface_dim: Color::Rgb(226, 220, 201),
    tab_inactive: Color::Rgb(226, 220, 201),
    success: Color::Rgb(133, 153, 0),
    warning: Color::Rgb(181, 137, 0),
    danger: Color::Rgb(220, 50, 47),
    info: Color::Rgb(42, 161, 152),
    special: Color::Rgb(108, 113, 196),
    graph_extra: Color::Rgb(211, 54, 130),
    input: Color::Rgb(181, 137, 0),
    bright: Color::Rgb(0, 43, 54),
    selection: Color::Rgb(238, 232, 213),
    current_line: Color::Rgb(245, 239, 220),
    changed_line: Color::Rgb(133, 153, 0),
    folder: Color::Rgb(38, 139, 210),
    accelerator: None,
    bold_chrome: false,
    resize_border: Color::Rgb(203, 75, 22),
};

/// base16 Eighties, by Chris Kempson. The accents are the ones its own syntax theme colours
/// keywords, strings and numbers with, which is how the chrome ends up matching the code without
/// anybody choosing twice.
const EIGHTIES: Palette = Palette {
    background: Color::Rgb(45, 45, 45),
    text: Color::Rgb(211, 208, 200),
    text_muted: Color::Rgb(168, 166, 158),
    text_dim: Color::Rgb(116, 115, 105),
    accent: Color::Rgb(102, 153, 204),
    on_accent: Color::Rgb(45, 45, 45),
    bar: Color::Rgb(57, 57, 57),
    on_bar: Color::Rgb(211, 208, 200),
    surface: Color::Rgb(57, 57, 57),
    surface_dim: Color::Rgb(45, 45, 45),
    tab_inactive: Color::Rgb(81, 81, 81),
    success: Color::Rgb(153, 204, 153),
    warning: Color::Rgb(255, 204, 102),
    danger: Color::Rgb(242, 119, 122),
    info: Color::Rgb(102, 204, 204),
    special: Color::Rgb(204, 153, 204),
    graph_extra: Color::Rgb(249, 145, 87),
    input: Color::Rgb(255, 204, 102),
    bright: Color::Rgb(242, 240, 236),
    selection: Color::Rgb(81, 81, 81),
    current_line: Color::Rgb(57, 57, 57),
    // base16 Eighties' own green, the one its syntax theme colours strings with.
    changed_line: Color::Rgb(153, 204, 153),
    folder: Color::Rgb(102, 153, 204),
    accelerator: None,
    bold_chrome: false,
    resize_border: Color::Rgb(249, 145, 87),
};

/// base16 Mocha, the warm one: browns where the others have greys, which is the whole of its
/// character and the reason its greens and blues are muted rather than clean.
const MOCHA: Palette = Palette {
    background: Color::Rgb(59, 50, 40),
    text: Color::Rgb(208, 200, 198),
    text_muted: Color::Rgb(165, 155, 150),
    text_dim: Color::Rgb(126, 112, 90),
    accent: Color::Rgb(138, 179, 181),
    on_accent: Color::Rgb(59, 50, 40),
    bar: Color::Rgb(75, 64, 52),
    on_bar: Color::Rgb(208, 200, 198),
    surface: Color::Rgb(75, 64, 52),
    surface_dim: Color::Rgb(59, 50, 40),
    tab_inactive: Color::Rgb(100, 82, 64),
    success: Color::Rgb(190, 181, 91),
    warning: Color::Rgb(244, 188, 135),
    danger: Color::Rgb(203, 96, 119),
    info: Color::Rgb(123, 189, 164),
    special: Color::Rgb(168, 155, 185),
    graph_extra: Color::Rgb(210, 139, 113),
    input: Color::Rgb(244, 188, 135),
    bright: Color::Rgb(245, 238, 235),
    selection: Color::Rgb(100, 82, 64),
    current_line: Color::Rgb(75, 64, 52),
    // Warm and muted like the rest of it: base16 Mocha has no clean green and inventing one
    // would put the only cold colour in the theme in its smallest text.
    changed_line: Color::Rgb(190, 181, 91),
    folder: Color::Rgb(138, 179, 181),
    accelerator: None,
    bold_chrome: false,
    resize_border: Color::Rgb(210, 139, 113),
};

/// base16 Ocean on paper. The scheme's accents were chosen against a near-black and are washed
/// out on white, so the six that carry meaning are the same hues taken darker — the alternative
/// was a status bar whose warnings and errors are the same pale wash from across the room.
const OCEAN_LIGHT: Palette = Palette {
    background: Color::Rgb(239, 241, 245),
    text: Color::Rgb(79, 91, 102),
    text_muted: Color::Rgb(99, 110, 124),
    text_dim: Color::Rgb(145, 152, 166),
    accent: Color::Rgb(76, 105, 140),
    on_accent: Color::Rgb(239, 241, 245),
    bar: Color::Rgb(223, 225, 232),
    on_bar: Color::Rgb(63, 72, 82),
    surface: Color::Rgb(229, 231, 238),
    surface_dim: Color::Rgb(214, 217, 225),
    tab_inactive: Color::Rgb(214, 217, 225),
    success: Color::Rgb(106, 140, 80),
    warning: Color::Rgb(154, 96, 62),
    danger: Color::Rgb(165, 60, 70),
    info: Color::Rgb(86, 122, 121),
    special: Color::Rgb(130, 92, 124),
    graph_extra: Color::Rgb(148, 106, 74),
    input: Color::Rgb(154, 96, 62),
    bright: Color::Rgb(30, 36, 44),
    selection: Color::Rgb(208, 214, 228),
    current_line: Color::Rgb(232, 234, 222),
    changed_line: Color::Rgb(86, 124, 62),
    folder: Color::Rgb(76, 105, 140),
    accelerator: None,
    bold_chrome: false,
    resize_border: Color::Rgb(180, 96, 40),
};

/// White paper, the lightest theme in the set. The accents are the ones its syntax theme uses for
/// keywords, numbers and functions; the roles that carry meaning rather than syntax — added,
/// modified, deleted — are picked to sit on white, because a syntax theme has no opinion on what
/// a deleted file should look like.
const GITHUB: Palette = Palette {
    background: Color::Rgb(255, 255, 255),
    text: Color::Rgb(50, 50, 50),
    text_muted: Color::Rgb(106, 115, 125),
    text_dim: Color::Rgb(150, 152, 150),
    accent: Color::Rgb(0, 92, 197),
    on_accent: Color::Rgb(255, 255, 255),
    bar: Color::Rgb(240, 241, 243),
    on_bar: Color::Rgb(50, 50, 50),
    surface: Color::Rgb(246, 248, 250),
    surface_dim: Color::Rgb(232, 234, 237),
    tab_inactive: Color::Rgb(232, 234, 237),
    success: Color::Rgb(34, 134, 58),
    warning: Color::Rgb(176, 112, 0),
    danger: Color::Rgb(181, 42, 29),
    info: Color::Rgb(0, 134, 179),
    special: Color::Rgb(121, 93, 163),
    graph_extra: Color::Rgb(167, 29, 93),
    input: Color::Rgb(176, 112, 0),
    bright: Color::Rgb(20, 22, 24),
    selection: Color::Rgb(248, 238, 199),
    current_line: Color::Rgb(245, 245, 245),
    // The green GitHub itself draws an added line in.
    changed_line: Color::Rgb(34, 134, 58),
    folder: Color::Rgb(0, 92, 197),
    accelerator: None,
    bold_chrome: false,
    resize_border: Color::Rgb(203, 36, 49),
};

#[cfg(test)]
mod tests {
    use super::*;

    /// The default theme has to be the colours that were written at the point of use, or the
    /// palette is not a refactoring but a redecoration nobody asked for.
    #[test]
    fn the_default_theme_is_the_editor_as_it_was() {
        let p = Theme::CleeCode.palette();
        assert_eq!(p.accent, Color::Cyan);
        assert_eq!(p.text_dim, Color::DarkGray);
        assert_eq!(p.text_muted, Color::Gray);
        assert_eq!(p.on_accent, Color::Black);
        assert_eq!(p.background, Color::Rgb(24, 24, 24));
        assert_eq!(p.text, Color::Reset, "the dark theme leaves text to the terminal");
    }

    /// Only the default leaves its text to the terminal. Every theme added since states one,
    /// because a theme that brings a background and not a foreground is half a theme.
    #[test]
    fn only_the_default_leaves_its_text_to_the_terminal() {
        for theme in Theme::ALL {
            if theme == Theme::CleeCode {
                continue;
            }
            assert_ne!(
                theme.palette().text,
                Color::Reset,
                "{} brings colours but leaves text to the terminal",
                theme.name()
            );
        }
    }

    /// Every theme names a syntect theme that syntect actually has. A typo here would fall back
    /// to whatever `themes.values().next()` happens to be, which is silent and wrong.
    #[test]
    fn every_theme_names_a_syntax_theme_that_exists() {
        let set = syntect::highlighting::ThemeSet::load_defaults();
        for theme in Theme::ALL {
            assert!(
                set.themes.contains_key(theme.syntax_theme()),
                "{} asks for a syntax theme syntect does not ship: {}",
                theme.name(),
                theme.syntax_theme()
            );
        }
    }

    /// The light themes have to paint; the dark ones are left to the user's setting. Left
    /// translucent, dark text lands on whatever the terminal's background is — black, for most
    /// people — which is the exact unreadability the themes exist to fix. Light text has no such
    /// problem: a dark terminal is what its user already chose.
    #[test]
    fn the_light_themes_paint_and_the_dark_ones_are_left_to_the_user() {
        for theme in [Theme::CleeCodeLight, Theme::SolarizedLight, Theme::OceanLight, Theme::GitHub]
        {
            assert!(theme.paints_its_own_background(), "{} must paint", theme.name());
        }
        for theme in
            [Theme::CleeCode, Theme::Turbo, Theme::SolarizedDark, Theme::Eighties, Theme::Mocha]
        {
            assert!(
                !theme.paints_its_own_background(),
                "{} has light text and can be left translucent",
                theme.name()
            );
        }
    }

    /// An initial drawn in the colour of the bar it sits on is an initial nobody can see, and one
    /// drawn in the colour of the rest of the word is a field that need not have existed.
    #[test]
    fn an_accelerator_stands_apart_from_what_is_around_it() {
        for theme in Theme::ALL {
            let p = theme.palette();
            let Some(colour) = p.accelerator else { continue };
            assert_ne!(colour, p.bar, "{}: the initial is the bar it sits on", theme.name());
            assert_ne!(colour, p.on_bar, "{}: the initial is the rest of the word", theme.name());
        }
    }

    /// The key each theme is written into `settings.toml` under. Spelled out rather than left to
    /// whatever the derive produces, because these are in users' config files: renaming a variant
    /// must not silently reset somebody's theme to the default, and the only way to notice is to
    /// have written the keys down.
    #[test]
    fn the_settings_keys_are_what_they_have_always_been() {
        let expected = [
            (Theme::CleeCode, "cleecode"),
            (Theme::Turbo, "turbo"),
            (Theme::SolarizedDark, "solarized-dark"),
            (Theme::Eighties, "eighties"),
            (Theme::Mocha, "mocha"),
            (Theme::CleeCodeLight, "cleecode-light"),
            (Theme::SolarizedLight, "solarized-light"),
            (Theme::OceanLight, "ocean-light"),
            (Theme::GitHub, "github"),
        ];
        for (theme, key) in expected {
            let written = toml::to_string(&Wrapper { theme }).unwrap();
            assert_eq!(written.trim(), format!("theme = \"{key}\""), "{}", theme.name());
            let read: Wrapper = toml::from_str(&written).unwrap();
            assert_eq!(read.theme, theme, "{} did not survive the round trip", theme.name());
        }
        assert_eq!(expected.len(), Theme::ALL.len(), "a theme with no key written down");
    }

    #[derive(Serialize, Deserialize)]
    struct Wrapper {
        theme: Theme,
    }

    /// The gutter draws five things in the same column, and two of them the same colour would be
    /// a breakpoint that reads as an external edit — or worse, an error that reads as one.
    #[test]
    fn a_changed_line_cannot_be_mistaken_for_the_other_marks() {
        for theme in Theme::ALL {
            let p = theme.palette();
            for (name, other) in
                [("danger", p.danger), ("warning", p.warning), ("dim", p.text_dim), ("accent", p.accent)]
            {
                assert_ne!(
                    p.changed_line,
                    other,
                    "{}: a changed line is drawn in the same colour as {name}",
                    theme.name()
                );
            }
        }
    }

    /// Names are what the picker shows and what `settings.toml` round-trips; two themes sharing
    /// one would make the picker ambiguous and the setting unreadable.
    #[test]
    fn theme_names_are_distinct() {
        let mut names: Vec<&str> = Theme::ALL.iter().map(|t| t.name()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "two themes share a name");
    }
}
