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
//!
//! `handle_stripes` is the case that went the other way, and it is worth knowing why. Six fixed
//! colours would have been a third exception; six colours *per theme* are a role after all — the
//! role being "the six this theme would sign its name with" — so they came in here, where a theme
//! can answer for them.

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
    /// The six bands the agent drawer's handle is extended with, top to bottom: three above the
    /// chevron's block and three below it.
    ///
    /// **A field, and not a constant in `ui.rs`, because a theme asked for one.** That is the
    /// ROADMAP's Turbo test applied to a decoration rather than to a role: the palette exists so
    /// that a theme which wants something of its own gets to say so, and "un tema con un campo
    /// opaco è quel meccanismo con un altro colore dentro" — a mark hard-coded in the drawing
    /// code would be the same six colours painted over nine different editors. Six colours are a
    /// *statement*, and every theme in here has one to make.
    ///
    /// So each theme declares its own, and they are quotations rather than choices: the default
    /// pair speaks the 1977 Apple rainbow, Turbo the six bright colours EGA numbered 9 through
    /// 14, Solarized its published accent run, the base16 themes the accent row their own syntax
    /// files declare, and the two remaining light themes their scales taken down onto paper. The
    /// hues are ordered as each scheme orders them, which is why the arc does not run the same
    /// way in all of them — running them all green-to-blue would have been the drawing code
    /// choosing again, one level up.
    ///
    /// Fixed colours inside each set, for the reason `ui::file_icon`'s are fixed: a quotation a
    /// theme may repaint is not quoting anything. The set is chosen per theme; the colours in it
    /// are not negotiated with anybody.
    pub handle_stripes: [Color; 6],
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

/// What `settings.toml` holds: a theme by name, or the standing instruction to ask the terminal.
///
/// The distinction is worth a type. A theme is what the editor is drawn in *right now*; a choice
/// is what the user asked for, and "ask the terminal" is an answer that outlives any one session
/// — the terminal can be a different colour tomorrow, and the setting should still be right.
/// So the choice is what is saved and what the pickers show as selected, and the theme it
/// resolves to is what everything else reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeChoice {
    /// Follow the terminal: light if its background is light, dark otherwise.
    Auto,
    /// This one, whatever the terminal is doing.
    Fixed(Theme),
}

/// The key `Auto` is written into `settings.toml` under. No theme may take this name, which the
/// tests below check rather than trust.
const AUTO_KEY: &str = "auto";

impl Default for ThemeChoice {
    /// The dark theme, not `Auto`. Somebody who has been using CleeCode has no `theme` key in
    /// their settings file, and a default of `Auto` would repaint their editor on the strength
    /// of a terminal colour they never mentioned. A new default is a new setting's business.
    fn default() -> Self {
        ThemeChoice::Fixed(Theme::default())
    }
}

impl ThemeChoice {
    /// The theme to draw in, given what the terminal said its background was — `None` when it
    /// was not asked or did not answer.
    ///
    /// Pure on purpose: the querying is somebody else's problem (see `preview::detect_background`),
    /// and everything interesting about the decision can then be read and tested here.
    pub fn resolve(self, background: Option<(u8, u8, u8)>) -> Theme {
        match self {
            ThemeChoice::Fixed(theme) => theme,
            // A terminal that says nothing is assumed dark. It is what terminals mostly are, it
            // is what CleeCode has always drawn for, and light text on an unknown background is
            // the failure that costs least: dark text on a dark terminal is unreadable, light
            // text on a light one is merely faint.
            ThemeChoice::Auto => match background {
                Some(rgb) if luminance(rgb) > 0.5 => Theme::CleeCodeLight,
                _ => Theme::CleeCode,
            },
        }
    }

    /// Auto first, then the themes in the order they are listed in: this list is the drop-down.
    /// Auto leads because it is the choice that needs no knowledge of the set below it.
    pub fn all() -> Vec<ThemeChoice> {
        std::iter::once(ThemeChoice::Auto).chain(Theme::ALL.map(ThemeChoice::Fixed)).collect()
    }

    /// What the picker shows. Untranslated for the same reason a theme's name is: "Auto" is the
    /// word in every language the editor speaks, and the row beside it says "CleeCode".
    pub fn name(self) -> &'static str {
        match self {
            ThemeChoice::Auto => "Auto",
            ThemeChoice::Fixed(theme) => theme.name(),
        }
    }
}

/// How light a colour is, 0.0 for black and 1.0 for white.
///
/// Rec. 601 luma, the same weights `Palette::needs_its_own_background` decides its own version of
/// this question with — one rule for "is this light", used in both places, rather than two that
/// could disagree about a grey.
pub fn luminance((r, g, b): (u8, u8, u8)) -> f32 {
    (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32) / 255.0
}

/// Written as the theme's own key, so a file that says `theme = "turbo"` today keeps saying it.
impl Serialize for ThemeChoice {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            ThemeChoice::Auto => serializer.serialize_str(AUTO_KEY),
            ThemeChoice::Fixed(theme) => theme.serialize(serializer),
        }
    }
}

/// One extra name on top of the theme keys. The theme half is handed to `Theme`'s own derived
/// implementation rather than spelled out again here: the keys live in one place, and a name
/// this does not know is still the error it always was.
impl<'de> Deserialize<'de> for ThemeChoice {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::IntoDeserializer;
        let key = String::deserialize(deserializer)?;
        if key == AUTO_KEY {
            return Ok(ThemeChoice::Auto);
        }
        Theme::deserialize(IntoDeserializer::<D::Error>::into_deserializer(key.as_str()))
            .map(ThemeChoice::Fixed)
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
    // The 1977 Apple rainbow, exactly. The default theme is the one CleeCode gets to have a
    // sense of humour in, and this is the joke: a handle you pull a computer's assistant out of,
    // wearing the six stripes off the machine that put one in a home. Green through blue, the way
    // that mark runs.
    handle_stripes: [
        Color::Rgb(0x61, 0xBB, 0x46),
        Color::Rgb(0xFD, 0xB8, 0x27),
        Color::Rgb(0xF5, 0x82, 0x1F),
        Color::Rgb(0xE0, 0x3A, 0x3E),
        Color::Rgb(0x96, 0x3D, 0x97),
        Color::Rgb(0x00, 0x9D, 0xDC),
    ],
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
    // The same six on paper, deepened by the rule the rest of this palette follows: the 1977
    // colours were printed on a beige case, not on white, and left as they are the yellow and the
    // orange are a haze at this size.
    handle_stripes: [
        Color::Rgb(62, 142, 44),
        Color::Rgb(192, 138, 0),
        Color::Rgb(194, 94, 10),
        Color::Rgb(178, 36, 42),
        Color::Rgb(122, 46, 124),
        Color::Rgb(0, 117, 168),
    ],
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
    // The six bright colours of the sixteen, in the order the hardware numbered them: 9 light
    // blue through 14 yellow. Dark grey and white are the two the mark cannot have, being the
    // chrome. There are sixteen colours and no others, which is the theme.
    handle_stripes: [
        Color::Rgb(85, 85, 255),
        Color::Rgb(85, 255, 85),
        Color::Rgb(85, 255, 255),
        Color::Rgb(255, 85, 85),
        Color::Rgb(255, 85, 255),
        Color::Rgb(255, 255, 85),
    ],
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
    // Solarized's accent run as Ethan Schoonover publishes it — yellow, orange, red, magenta,
    // violet, blue — which is already six colours in a stated order, so there is nothing here to
    // choose. Every one of them is a colour this palette uses somewhere else.
    handle_stripes: [
        Color::Rgb(181, 137, 0),
        Color::Rgb(203, 75, 22),
        Color::Rgb(220, 50, 47),
        Color::Rgb(211, 54, 130),
        Color::Rgb(108, 113, 196),
        Color::Rgb(38, 139, 210),
    ],
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
    // Identical to the dark theme's, and that is the point of Solarized: one scheme, two
    // grounds, and the accents do not move between them. Deepening these for paper would be
    // correcting the scheme rather than quoting it.
    handle_stripes: [
        Color::Rgb(181, 137, 0),
        Color::Rgb(203, 75, 22),
        Color::Rgb(220, 50, 47),
        Color::Rgb(211, 54, 130),
        Color::Rgb(108, 113, 196),
        Color::Rgb(38, 139, 210),
    ],
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
    // base16's accent row, base08 through base0D, in the order base16 numbers it: red, orange,
    // yellow, green, cyan, blue. The scheme ships six accents and this is them.
    handle_stripes: [
        Color::Rgb(242, 119, 122),
        Color::Rgb(249, 145, 87),
        Color::Rgb(255, 204, 102),
        Color::Rgb(153, 204, 153),
        Color::Rgb(102, 204, 204),
        Color::Rgb(102, 153, 204),
    ],
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
    // base08 through base0D again, this time Mocha's — warm and muted the whole way across,
    // which is why its rainbow reads as one colour family lit from different sides.
    handle_stripes: [
        Color::Rgb(203, 96, 119),
        Color::Rgb(210, 139, 113),
        Color::Rgb(244, 188, 135),
        Color::Rgb(190, 181, 91),
        Color::Rgb(123, 189, 164),
        Color::Rgb(138, 179, 181),
    ],
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
    // base16 Ocean's accent row in the same order, taken down onto paper by the rule the rest of
    // this palette is: the scheme's own accents were chosen against a near-black, and six pale
    // washes side by side would be one pale wash.
    handle_stripes: [
        Color::Rgb(165, 60, 70),
        Color::Rgb(176, 92, 56),
        Color::Rgb(150, 118, 30),
        Color::Rgb(106, 140, 80),
        Color::Rgb(86, 122, 121),
        Color::Rgb(76, 105, 140),
    ],
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
    // Primer's own scale in Primer's own order: red, orange, green, teal, blue, purple. Every
    // one is a colour this palette already uses for something, which is what keeps the mark part
    // of the theme rather than a sticker on it.
    handle_stripes: [
        Color::Rgb(181, 42, 29),
        Color::Rgb(176, 112, 0),
        Color::Rgb(34, 134, 58),
        Color::Rgb(0, 134, 179),
        Color::Rgb(0, 92, 197),
        Color::Rgb(121, 93, 163),
    ],
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

    /// Every theme signs the drawer's handle with six colours of its own, and they have to be
    /// six colours rather than a gradient with a rounding error in it.
    ///
    /// Two bands the same is the failure this is for: they are drawn touching, so a pair that
    /// matches is one band of twice the height and the mark quietly has five stripes. Adjacent is
    /// what has to be checked — a set may reuse a hue at the far end on purpose — and the
    /// separation is a floor rather than an equality, because two colours a couple of points
    /// apart are the same colour to anyone looking at a two-row band from across a desk.
    #[test]
    fn every_theme_signs_the_handle_with_six_colours_of_its_own() {
        // The channel distance below which two RGB colours read as one band. Generous on purpose:
        // this is a decoration a metre away, not a diff.
        const APART: i32 = 24;
        let channels = |c: Color| match c {
            Color::Rgb(r, g, b) => (r as i32, g as i32, b as i32),
            other => panic!("a band has to be a colour, not a name the terminal decides: {other:?}"),
        };
        for theme in Theme::ALL {
            let stripes = theme.palette().handle_stripes;
            for (i, pair) in stripes.windows(2).enumerate() {
                let (a, b) = (channels(pair[0]), channels(pair[1]));
                let apart = (a.0 - b.0).abs() + (a.1 - b.1).abs() + (a.2 - b.2).abs();
                assert!(
                    apart >= APART,
                    "{}: bands {} and {} are the same band ({:?} vs {:?})",
                    theme.name(),
                    i,
                    i + 1,
                    pair[0],
                    pair[1]
                );
            }
        }
    }

    /// The default theme's six are the 1977 Apple rainbow, exactly. Written down because it is a
    /// quotation: a colour drifting by a point here is not a tweak, it is a misquote — and it is
    /// the set the drawer's driver reads off the screen.
    #[test]
    fn the_default_theme_wears_the_rainbow() {
        assert_eq!(
            Theme::CleeCode.palette().handle_stripes,
            [
                Color::Rgb(0x61, 0xBB, 0x46),
                Color::Rgb(0xFD, 0xB8, 0x27),
                Color::Rgb(0xF5, 0x82, 0x1F),
                Color::Rgb(0xE0, 0x3A, 0x3E),
                Color::Rgb(0x96, 0x3D, 0x97),
                Color::Rgb(0x00, 0x9D, 0xDC),
            ]
        );
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

    #[derive(Serialize, Deserialize)]
    struct ChoiceWrapper {
        theme: ThemeChoice,
    }

    /// The settings file gained a value, not a spelling: every theme key still reads and writes
    /// as itself, and `auto` is the one name added beside them.
    #[test]
    fn a_choice_is_written_as_a_theme_key_or_as_auto() {
        for theme in Theme::ALL {
            let choice = ThemeChoice::Fixed(theme);
            let written = toml::to_string(&ChoiceWrapper { theme: choice }).unwrap();
            let plain = toml::to_string(&Wrapper { theme }).unwrap();
            assert_eq!(written, plain, "{} is written differently as a choice", theme.name());
            let read: ChoiceWrapper = toml::from_str(&written).unwrap();
            assert_eq!(read.theme, choice, "{} did not survive the round trip", theme.name());
        }
        let auto = toml::to_string(&ChoiceWrapper { theme: ThemeChoice::Auto }).unwrap();
        assert_eq!(auto.trim(), "theme = \"auto\"");
        let read: ChoiceWrapper = toml::from_str(&auto).unwrap();
        assert_eq!(read.theme, ThemeChoice::Auto);
    }

    /// A name that is neither a theme nor `auto` is refused rather than quietly defaulted: the
    /// settings loader has its own answer for a broken file, and swallowing the error here would
    /// take that decision away from it.
    #[test]
    fn an_unknown_name_is_not_a_choice() {
        assert!(toml::from_str::<ChoiceWrapper>("theme = \"dracula\"").is_err());
        assert!(toml::from_str::<ChoiceWrapper>("theme = \"Auto\"").is_err());
    }

    /// No theme may be called `auto`, or the word would mean two things in the same file and the
    /// one written down here would win silently.
    #[test]
    fn no_theme_answers_to_auto() {
        for theme in Theme::ALL {
            let written = toml::to_string(&Wrapper { theme }).unwrap();
            assert_ne!(written.trim(), format!("theme = \"{AUTO_KEY}\""), "{}", theme.name());
        }
    }

    /// The default is the editor as it was. A user with no `theme` key must not be repainted by
    /// a terminal colour they never mentioned.
    #[test]
    fn the_default_choice_is_the_dark_theme_and_not_auto() {
        assert_eq!(ThemeChoice::default(), ThemeChoice::Fixed(Theme::CleeCode));
        assert_eq!(ThemeChoice::default().resolve(Some((255, 255, 255))), Theme::CleeCode);
    }

    /// What `auto` decides, and what it does when there is nothing to decide from.
    #[test]
    fn auto_follows_the_terminal_and_falls_back_to_dark() {
        let light = |rgb| ThemeChoice::Auto.resolve(Some(rgb));
        assert_eq!(light((255, 255, 255)), Theme::CleeCodeLight, "white paper");
        assert_eq!(light((253, 246, 227)), Theme::CleeCodeLight, "solarized light's own surface");
        assert_eq!(light((0, 0, 0)), Theme::CleeCode, "black");
        assert_eq!(light((24, 24, 24)), Theme::CleeCode, "the default theme's own surface");
        assert_eq!(light((0, 43, 54)), Theme::CleeCode, "solarized dark's own surface");
        // A terminal that was never asked, or that said nothing: dark, which is what CleeCode
        // has always drawn for.
        assert_eq!(ThemeChoice::Auto.resolve(None), Theme::CleeCode);
    }

    /// Green weighs more than blue, and the answer stays inside the range the threshold is
    /// stated in — a luminance above one would make "> 0.5" true for colours that are not light.
    #[test]
    fn luminance_runs_from_black_to_white() {
        assert_eq!(luminance((0, 0, 0)), 0.0);
        assert!((luminance((255, 255, 255)) - 1.0).abs() < 1e-6);
        assert!(luminance((0, 255, 0)) > luminance((0, 0, 255)));
        for rgb in [(255, 0, 0), (0, 255, 0), (0, 0, 255), (128, 128, 128)] {
            let l = luminance(rgb);
            assert!((0.0..=1.0).contains(&l), "{rgb:?} is outside the range");
        }
    }

    /// The two themes `auto` picks between have to be one of each, or the setting is a switch
    /// with the same thing on both ends.
    #[test]
    fn auto_picks_between_a_light_theme_and_a_dark_one() {
        assert!(ThemeChoice::Auto.resolve(Some((255, 255, 255))).paints_its_own_background());
        assert!(!ThemeChoice::Auto.resolve(None).paints_its_own_background());
    }

    /// Auto leads the list and every theme follows it, in the order the theme list is in: this
    /// is what the drop-down draws, so a theme missing here is a theme nobody can choose.
    #[test]
    fn the_choice_list_is_auto_and_then_the_themes() {
        let all = ThemeChoice::all();
        assert_eq!(all.len(), Theme::ALL.len() + 1);
        assert_eq!(all[0], ThemeChoice::Auto);
        for (i, theme) in Theme::ALL.iter().enumerate() {
            assert_eq!(all[i + 1], ThemeChoice::Fixed(*theme));
        }
        assert_eq!(all[0].name(), "Auto");
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
