//! The agent drawer: a column on the right of the window that holds one coding agent.
//!
//! It is a terminal window like any other inside — a pty, a vt100 parser, the same drawing code —
//! and everything interesting about it is where it *lives*.

use crate::i18n::Lang;
use crate::session::Agent;
use crate::terminal_panel::TerminalWindow;

/// The drawer and whatever is currently in it.
///
/// **Deliberately not a member of `App::terminals`.** `App::rebuild_terminals` drains and
/// replaces that vector wholesale on every workspace switch — reusing the shells it finds there,
/// spawning the rest — and the drawer's whole promise is the opposite one: the conversation
/// survives. An agent you have been talking to for an hour must not become a spare shell handed
/// out to the next workspace's `octave` tab. Living outside the vector is what makes that
/// structural rather than a rule someone has to remember, and it is why the polling loops in
/// `app.rs` name the drawer explicitly: nothing that iterates `terminals` reaches it by accident,
/// in either direction.
///
/// The width is not here either. It is `settings.drawer_pct`, beside `terminal_pct` and
/// `split_pct`, because it is a layout scalar the seam drag writes to and the workspace file
/// records — and a second copy on this struct would be a second thing to keep in step.
pub struct Drawer {
    /// The agent's pane, or `None` while the launcher is showing. It is a whole `TerminalWindow`
    /// rather than a bare panel so the drawing code needs no special case; it holds exactly one
    /// tab, because the drawer is one agent's home and a tab strip in it would be an invitation
    /// to make it a second terminal panel.
    pub window: Option<TerminalWindow>,
    /// Which agent is running, or was. Kept across the agent exiting so the launcher can put the
    /// highlight back where it was.
    pub agent: Option<Agent>,
    /// The highlighted row of the launcher.
    pub selected: usize,
    /// Whether the column is carved out of the layout right now.
    ///
    /// Closing sets this to `false` and touches nothing else: the pty goes on running, exactly
    /// as the terminal panel's does under `Ctrl+J`. That is the difference between dismissing an
    /// agent and killing it, and it is the only reason hiding the drawer is a cheap thing to do.
    pub open: bool,
}

impl Drawer {
    /// A drawer showing the launcher, with `agent` highlighted — the last one used, where there
    /// is one to remember.
    pub fn with_launcher(agent: Option<Agent>) -> Self {
        forget_installed();
        Drawer {
            window: None,
            agent,
            selected: agent.map(Agent::index).unwrap_or(0),
            open: true,
        }
    }

    /// Whether the launcher is what is drawn in it right now.
    pub fn showing_launcher(&self) -> bool {
        self.window.is_none()
    }

    /// The agent under the highlight.
    pub fn highlighted(&self) -> Agent {
        Agent::all()[self.selected.min(Agent::all().len() - 1)]
    }

    /// Moves the highlight, wrapping — four names are a ring, and stopping at the end of a list
    /// this short is a rule the user pays for without being told about it. The same rule the
    /// menus and the pickers already use.
    pub fn move_selection(&mut self, delta: isize) {
        let len = Agent::all().len() as isize;
        let at = (self.selected as isize + delta % len + len) % len;
        self.selected = at as usize;
    }

    /// Puts the launcher back, dropping whatever was in the pane.
    ///
    /// The honest outcome of an agent exiting: the conversation is over, and what is on offer is
    /// the same choice as before. Nothing is respawned — a shell appearing where an agent was is
    /// the one thing this panel must never do, because it looks exactly like the agent still
    /// being there.
    pub fn back_to_launcher(&mut self) {
        self.window = None;
        // Coming back to the list is one of the two moments the launcher's answer to "is this one
        // installed?" can have gone stale under it — the other is the drawer opening. See
        // [`forget_installed`].
        forget_installed();
        if let Some(agent) = self.agent {
            self.selected = agent.index();
        }
    }
}

/// Whether an open drawer is still on screen now that the keyboard is wherever it is.
///
/// **In a TUI the signal is the focus, not the mouse passing over.** There is no hover here worth
/// the name — the pointer may not exist at all, and a panel that withdrew when a pointer left it
/// would be a panel that never withdrew for half the people using it. What there always is, and
/// what always means "I have gone back to what I was doing", is the keyboard moving to another
/// frame: an arrow out of the drawer, `Ctrl+Tab`, `Esc`, a click landing anywhere else.
///
/// Pinned means pinned, so the answer is yes whatever the focus is doing. Autocollapse means the
/// drawer is on screen exactly while it has the keyboard — and the way back is the way in, which
/// is `Ctrl+Shift+A` or the View menu. Nothing is killed either way: what closes is the column,
/// and the pty behind it goes on running, so the collapse costs the conversation nothing.
///
/// A function rather than a method because there is no `App` to build in a test in this repo, and
/// this rule is the whole of the mode.
pub fn stays_open(pinned: bool, has_the_keyboard: bool) -> bool {
    pinned || has_the_keyboard
}

// ---- the marks --------------------------------------------------------------------------------

/// **Evocations, drawn in cells, of the marks the four CLIs are known by.**
///
/// They are here to identify the third-party program the row launches — the reason a file browser
/// puts a `.py` beside a Python file — and nothing more: nominative use, the same rule the file
/// tree's icons and the themes' editor names already run under. Each is a few dozen half-block
/// cells at a size no logo survives being copied at; none is anybody's artwork lifted, none is
/// used as CleeCode's own mark, and the brand colours are fixed values rather than palette roles
/// precisely because they belong to somebody else and must not drift with a theme. House rule
/// discussed 2026-09-03; the older rule this replaces (block-letter wordmarks, "not logos") is in
/// docs/ROADMAP.md under the drawer and is superseded by this note.
///
/// **Why pixels rather than characters.** A half-block cell is two square pixels stacked, so a
/// grid written here as text is drawn at twice the vertical resolution of the terminal's own
/// cells — which is the whole reason a ten-row creature fits in five rows of a panel. The letters
/// in the grids are inks, not glyphs; [`ink`] says which colour each one is, and `.` is the
/// drawer showing through.
///
/// Anthropic's burst beside Clawd, the small orange critter Claude Code is mascotted by. Two
/// elements, because the mark and the mascot are how that CLI is recognised and either alone
/// reads as half of it.
const CLAUDE_ART: &[&str] = &[
    ".....A.....  ...CCCCCC...",
    "A...AAA...A  ..CCCCCCCC..",
    "AA..AAA..AA  .CCCCCCCCCC.",
    ".AAAAAAAAA.  CCCCCCCCCCCC",
    "..AAAAAAA..  CC..CCCC..CC",
    "..AAAAAAA..  CC..CCCC..CC",
    ".AAAAAAAAA.  CCCCC..CCCCC",
    "AA..AAA..AA  CCCCCCCCCCCC",
    "A...AAA...A  .CCC....CCC.",
    ".....A.....  ..CC....CC..",
];

/// opencode's squared terminal block: a dark tile with a white rim, a prompt chevron and a block
/// cursor inside it. The rim is what makes a black square visible on a dark drawer, and the
/// black/white contrast is theirs.
const OPENCODE_ART: &[&str] = &[
    ".WWWWWWWWWWW.",
    "WKKKKKKKKKKKW",
    "WKWWKKKWWWKKW",
    "WKKWWKKWWWKKW",
    "WKKKWWKWWWKKW",
    "WKKWWKKWWWKKW",
    "WKWWKKKWWWKKW",
    "WKKKKKKKKKKKW",
    "WKKKKKKKKKKKW",
    ".WWWWWWWWWWW.",
];

/// OpenAI's hexagonal knot, as far as thirteen columns allow: two nested hexagons joined by three
/// strands — left, right and below — which is what is left of the weave at this size.
const CODEX_ART: &[&str] = &[
    "....XXXXX....",
    "..XX.....XX..",
    ".X..XXXXX..X.",
    "X..X.....X..X",
    "XXXX.....X..X",
    "X..X.....XXXX",
    "X..X.....X..X",
    ".X..XXXXX..X.",
    "..XX..X..XX..",
    "....XXXXX....",
];

/// Gemini's four-pointed sparkle, with the needle points its own mark has. Every pixel is the
/// same ink letter: the colour comes from how far down the row is — see [`gemini_gradient`].
const GEMINI_ART: &[&str] = &[
    ".....G.....",
    ".....G.....",
    "....GGG....",
    "...GGGGG...",
    "GGGGGGGGGGG",
    "GGGGGGGGGGG",
    "...GGGGG...",
    "....GGG....",
    ".....G.....",
    ".....G.....",
];

/// A colour, as it is written down by the people who own it. Not a [`crate::theme::Palette`]
/// role: these four are brand colours and are fixed for the same reason the file tree's icons
/// are, so `drawer.rs` says them in plain numbers and `ui.rs` turns them into whatever the
/// drawing library calls a colour.
pub type Ink = (u8, u8, u8);

/// Anthropic's coral.
const ANTHROPIC_CORAL: Ink = (0xD9, 0x77, 0x57);
/// Clawd, a shade up from it so the critter and the burst read as two things.
const CLAWD_ORANGE: Ink = (0xF0, 0x80, 0x5A);
/// OpenAI's knot, white on a dark panel.
const OPENAI_WHITE: Ink = (0xE8, 0xE8, 0xE8);
/// opencode's two, which are only ever each other's opposite.
const OPENCODE_WHITE: Ink = (0xFA, 0xFA, 0xFA);
const OPENCODE_BLACK: Ink = (0x14, 0x14, 0x14);
/// Gemini's sparkle runs blue at the top into violet at the bottom.
const GEMINI_BLUE: Ink = (0x47, 0x96, 0xE3);
const GEMINI_VIOLET: Ink = (0x91, 0x68, 0xC0);

/// How many pixel rows a mark is written in. Twice [`ART_ROWS`], because a half-block cell holds
/// two of them.
const ART_PIXEL_ROWS: usize = 10;

/// How many cells tall a mark is drawn.
pub const ART_ROWS: usize = ART_PIXEL_ROWS / 2;

/// The gradient down gemini's sparkle: blue at the top pixel row, violet at the bottom, mixed
/// evenly in between. Rows past the art are clamped rather than wrapped, so a caller that asks
/// for a row that is not there gets the end of the gradient and never a panic.
fn gemini_gradient(row: usize) -> Ink {
    let last = (ART_PIXEL_ROWS - 1) as u32;
    let at = (row as u32).min(last);
    let mix = |from: u8, to: u8| {
        let (from, to) = (from as u32, to as u32);
        // Integer arithmetic, rounded to nearest, so the ten steps are the same ten on every
        // machine and the test below can name them.
        ((from * (last - at) + to * at + last / 2) / last) as u8
    };
    (
        mix(GEMINI_BLUE.0, GEMINI_VIOLET.0),
        mix(GEMINI_BLUE.1, GEMINI_VIOLET.1),
        mix(GEMINI_BLUE.2, GEMINI_VIOLET.2),
    )
}

/// The grid an agent's mark is written in. A `match` rather than a lookup, so a fifth agent added
/// to [`Agent::all`] does not compile until it has been drawn — the block alphabet this replaced
/// could only find that out at run time, and answered by drawing nothing.
fn pixels(agent: Agent) -> &'static [&'static str] {
    match agent {
        Agent::Claude => CLAUDE_ART,
        Agent::OpenCode => OPENCODE_ART,
        Agent::Codex => CODEX_ART,
        Agent::Gemini => GEMINI_ART,
    }
}

/// What colour one pixel is: its letter says which ink, and the pixel row says how far down a
/// gradient it is — which only gemini's mark uses. `None` is the drawer showing through.
fn ink(mark: char, row: usize) -> Option<Ink> {
    match mark {
        'A' => Some(ANTHROPIC_CORAL),
        'C' => Some(CLAWD_ORANGE),
        'X' => Some(OPENAI_WHITE),
        'W' => Some(OPENCODE_WHITE),
        'K' => Some(OPENCODE_BLACK),
        'G' => Some(gemini_gradient(row)),
        _ => None,
    }
}

/// One cell of a drawn mark: the glyph, the ink of its upper half, and the ink behind it — which
/// is the *lower* half, and is only ever set where the two halves are different colours.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ArtCell {
    pub ch: char,
    pub fg: Option<Ink>,
    pub bg: Option<Ink>,
}

/// How wide `agent`'s mark comes out, in cells.
pub fn art_width(agent: Agent) -> u16 {
    pixels(agent).iter().map(|row| row.chars().count()).max().unwrap_or(0) as u16
}

/// The widest of the four, which is the width the launcher has to have to show any of them: the
/// marks are drawn in one column, so the room they need is the room the largest needs.
pub fn widest_art() -> u16 {
    Agent::all().into_iter().map(art_width).max().unwrap_or(0)
}

/// `agent`'s mark as [`ART_ROWS`] rows of cells.
///
/// The encoding is the whole trick: two stacked pixels become one cell. Both lit and the same
/// colour is a full block; both lit and *different* colours is an upper half-block with the lower
/// colour behind it, which is what lets gemini's gradient change inside a single row of the
/// terminal; one lit is the half-block on that side; neither is a space.
pub fn art(agent: Agent) -> Vec<Vec<ArtCell>> {
    let grid = pixels(agent);
    let width = art_width(agent) as usize;
    (0..ART_ROWS)
        .map(|row| {
            let at = |pixel_row: usize, x: usize| {
                grid.get(pixel_row)
                    .and_then(|line| line.chars().nth(x))
                    .and_then(|mark| ink(mark, pixel_row))
            };
            (0..width)
                .map(|x| match (at(row * 2, x), at(row * 2 + 1, x)) {
                    (None, None) => ArtCell { ch: ' ', fg: None, bg: None },
                    (Some(up), None) => ArtCell { ch: '▀', fg: Some(up), bg: None },
                    (None, Some(down)) => ArtCell { ch: '▄', fg: Some(down), bg: None },
                    (Some(up), Some(down)) if up == down => {
                        ArtCell { ch: '█', fg: Some(up), bg: None }
                    }
                    (Some(up), Some(down)) => ArtCell { ch: '▀', fg: Some(up), bg: Some(down) },
                })
                .collect()
        })
        .collect()
}

// ---- installing the one that is not here --------------------------------------------------

/// What to type to install `agent`, as that project's own documentation gives it.
///
/// Checked against each project's install page on 2026-09-03 and taken from it rather than from
/// memory, because these move: Claude Code's npm package is no longer the recommended path (the
/// docs lead with the install script), and opencode's script is still the one they put first.
/// A command that has drifted installs nothing and says so at a prompt, which is the failure this
/// list is arranged to make rare and survivable.
///
/// It is *typed*, never run — see the caller in `app.rs`. So these are one line each, with
/// nothing conditional in them: a person has to be able to read the line before pressing Enter,
/// and a line nobody can read is a line nobody should be asked to approve.
#[cfg(not(windows))]
pub fn install_command(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "curl -fsSL https://claude.ai/install.sh | bash",
        Agent::OpenCode => "curl -fsSL https://opencode.ai/install | bash",
        Agent::Codex => "npm install -g @openai/codex",
        Agent::Gemini => "npm install -g @google/gemini-cli",
    }
}

/// The same, where the shell is a Windows one and a `curl … | bash` is not a sentence. Every one
/// of these is documented by the project too; the two that are npm are the same line either way.
#[cfg(windows)]
pub fn install_command(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "winget install Anthropic.ClaudeCode",
        Agent::OpenCode => "npm install -g opencode-ai",
        Agent::Codex => "npm install -g @openai/codex",
        Agent::Gemini => "npm install -g @google/gemini-cli",
    }
}

/// How long an answer to "is this one installed?" is trusted for.
///
/// The question costs a walk of every directory on the PATH and the launcher asks it on every
/// frame, so it cannot be asked every time; the answer changes exactly when somebody installs an
/// agent, which is now something this panel *offers to do*, so it cannot be asked once either.
/// A couple of seconds is under the time it takes to read the status line and look back at the
/// drawer, and is a walk of ten directories twice a second at worst — only while the launcher is
/// the thing on screen.
const PATH_MEMO: std::time::Duration = std::time::Duration::from_secs(2);

/// The remembered answer for all four, and when it was found out.
static FOUND: std::sync::Mutex<Option<(std::time::Instant, [bool; 4])>> =
    std::sync::Mutex::new(None);

/// Whether `agent` is installed on this machine, asked freshly enough to be worth acting on.
///
/// **Deliberately not [`Agent::on_path`].** That one is remembered for the life of the process,
/// which was right while the answer only decided how a name was *drawn*. It is wrong now: the
/// launcher offers to install the missing one, so the very next thing that happens after a `false`
/// may be that it becomes true, and a panel that goes on saying "not installed" about a program
/// the user has just installed at its invitation is the panel calling the user a liar.
pub fn installed(agent: Agent) -> bool {
    let mut memo = FOUND.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let found = match (*memo).filter(|(asked, _)| asked.elapsed() < PATH_MEMO) {
        Some((_, found)) => found,
        None => {
            let found = probe();
            *memo = Some((std::time::Instant::now(), found));
            found
        }
    };
    found[agent.index()]
}

/// Throws the remembered answer away, so the next ask is a real one.
///
/// Called where the answer has most likely just changed — the drawer opening, the launcher coming
/// back, an install command going to a shell — rather than relied on alone: the memo above expires
/// by itself, and this is only the shortcut past waiting for it.
pub fn forget_installed() {
    *FOUND.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

/// The walk itself. [`crate::tools::tool`] and not `which`, for the reason written in `tools.rs`:
/// started from the Dock this process has launchd's PATH, which has none of the places npm and
/// Homebrew put things.
///
/// [`Agent::on_path`] comes first and is the fast path rather than a second opinion. It is the
/// same question answered once for the life of the process, which is only wrong in one direction:
/// an agent that was here at startup does not stop being here while a panel is open, and if it
/// somehow did, the shell would say `command not found` — the same thing it says today. What that
/// once-for-all answer *cannot* do is notice an agent arriving, which is exactly what this offer
/// makes likely, so the walk below is what the missing ones get.
fn probe() -> [bool; 4] {
    let mut found = [false; 4];
    for agent in Agent::all() {
        found[agent.index()] = agent.on_path()
            || agent.programs().iter().any(|name| crate::tools::tool(name).is_some());
    }
    found
}

/// The status line after an install command has been typed at a shell prompt.
///
/// It says all three things the moment needs: which program is missing, that the line is *at* a
/// prompt rather than run, and whose keypress the last one is. `Ctrl+Shift+A`'s discipline, said
/// out loud — the editor never presses Enter on somebody's behalf, least of all on a line that
/// downloads and runs a script.
///
/// Written here rather than in `i18n.rs` because it arrived with the art beside it and the two
/// were built in one sitting; it belongs in `i18n.rs` with the rest and should be moved there the
/// next time that file is opened.
pub fn msg_install_typed(lang: Lang, agent: &str, command: &str) -> String {
    match lang {
        Lang::En => format!("{agent} is not installed — `{command}` is at a shell prompt, unsent. Enter is yours."),
        Lang::It => format!("{agent} non è installato — `{command}` è al prompt di una shell, non inviato. L'Invio è tuo."),
    }
}

/// The same moment when there was nowhere to put the line. Rare — a shell is opened when none is
/// free — and it still has to say the command, or the offer was worth nothing.
pub fn msg_install_no_shell(lang: Lang, agent: &str, command: &str) -> String {
    match lang {
        Lang::En => format!("{agent} is not installed — no shell to type into. Install it with: {command}"),
        Lang::It => format!("{agent} non è installato — nessuna shell in cui scrivere. Installalo con: {command}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every mark is a rectangle of the height the layout was told to expect, drawn in inks this
    /// file knows. A grid one character short on one row would draw a mark with a bite out of it
    /// and nothing anywhere would say so.
    #[test]
    fn every_agent_has_a_mark_of_the_size_the_layout_expects() {
        for agent in Agent::all() {
            let name = agent.workspace_name();
            let grid = pixels(agent);
            assert_eq!(grid.len(), ART_PIXEL_ROWS, "{name}: a mark is ten pixel rows");
            let width = art_width(agent);
            assert!(width > 0, "{name}: a mark with no width is not a mark");
            for (i, row) in grid.iter().enumerate() {
                assert_eq!(
                    row.chars().count() as u16,
                    width,
                    "{name}: pixel row {i} is not the width of the grid"
                );
                for mark in row.chars() {
                    assert!(
                        mark == '.' || mark == ' ' || ink(mark, i).is_some(),
                        "{name}: {mark:?} on row {i} is an ink nothing defines"
                    );
                }
            }
            let drawn = art(agent);
            assert_eq!(drawn.len(), ART_ROWS, "{name}: five cell rows out of ten pixel rows");
            for row in &drawn {
                assert_eq!(row.len(), width as usize, "{name}: every cell row is the full width");
            }
            assert!(
                drawn.iter().flatten().any(|cell| cell.fg.is_some()),
                "{name}: a mark that is all blank cells is a blank"
            );
        }
    }

    /// The half-block encoding, at the four cases it has: nothing, one half, both halves the same
    /// colour, both halves different. The last one is the only reason `bg` exists, and it is what
    /// carries gemini's gradient through a single row of the terminal.
    #[test]
    fn two_pixels_become_one_cell() {
        let codex = art(Agent::Codex);
        // The knot's top pixel row is empty at the left edge and its second is too, so the corner
        // cell is a space with no ink at all.
        assert_eq!(codex[0][0], ArtCell { ch: ' ', fg: None, bg: None });
        // Its middle rows have the outer hexagon's side lit in both halves, one ink: a full block.
        assert_eq!(codex[2][0], ArtCell { ch: '█', fg: Some(OPENAI_WHITE), bg: None });
        // The sparkle's top point is lit in the upper half of its first cell row and in the lower
        // half too — but the gradient has moved between them, so the cell carries both.
        let gemini = art(Agent::Gemini);
        let point = gemini[0][5];
        assert_eq!(point.ch, '▀');
        assert_eq!(point.fg, Some(gemini_gradient(0)));
        assert_eq!(point.bg, Some(gemini_gradient(1)));
        assert_ne!(point.fg, point.bg, "the whole point of the second colour");
        // A half on its own is the half-block on that side, and nothing behind it.
        let claude = art(Agent::Claude);
        assert_eq!(claude[0][4], ArtCell { ch: '▄', fg: Some(ANTHROPIC_CORAL), bg: None });
        assert_eq!(claude[0][5], ArtCell { ch: '█', fg: Some(ANTHROPIC_CORAL), bg: None });
    }

    /// The gradient reaches both ends and only moves downwards. A "gradient" that arrived at the
    /// bottom still blue would be four rows of flat colour nobody would notice was wrong.
    #[test]
    fn the_sparkle_runs_blue_into_violet() {
        assert_eq!(gemini_gradient(0), GEMINI_BLUE);
        assert_eq!(gemini_gradient(ART_PIXEL_ROWS - 1), GEMINI_VIOLET);
        assert_eq!(gemini_gradient(999), GEMINI_VIOLET, "past the end is the end, not a panic");
        for row in 1..ART_PIXEL_ROWS {
            let (before, now) = (gemini_gradient(row - 1), gemini_gradient(row));
            assert!(now.0 >= before.0 && now.1 <= before.1 && now.2 <= before.2, "row {row}");
        }
        // And the two ends of the drawn art are visibly apart, which is what a reader sees.
        let art = art(Agent::Gemini);
        let top = art[0][5].fg.unwrap();
        let bottom = art[ART_ROWS - 1][5].bg.or(art[ART_ROWS - 1][5].fg).unwrap();
        assert_ne!(top, bottom);
    }

    /// The marks are drawn in the colours their owners use, and those are values rather than
    /// palette roles — the same rule the file tree's icons run under. A mark that changed colour
    /// with the theme would stop being the mark.
    #[test]
    fn the_marks_are_drawn_in_their_own_colours() {
        let coral = |agent| art(agent).into_iter().flatten().any(|c| c.fg == Some(ANTHROPIC_CORAL));
        assert!(coral(Agent::Claude), "the burst is Anthropic's coral");
        assert!(!coral(Agent::Codex), "and nobody else's mark is");
        let clawd = art(Agent::Claude).into_iter().flatten().any(|c| c.fg == Some(CLAWD_ORANGE));
        assert!(clawd, "the critter beside it is its own orange, so the two read apart");
        let both: Vec<_> = art(Agent::OpenCode)
            .into_iter()
            .flatten()
            .filter_map(|cell| cell.fg)
            .collect();
        assert!(both.contains(&OPENCODE_WHITE) && both.contains(&OPENCODE_BLACK));
    }

    /// Every agent has a command to install it, it is one line, and it names the program it
    /// installs. A blank one would type nothing at a prompt and say it had done something.
    #[test]
    fn every_agent_has_an_install_command() {
        for agent in Agent::all() {
            let command = install_command(agent);
            assert!(!command.trim().is_empty(), "{:?}", agent);
            assert!(!command.contains('\n'), "one line, so it can be read before it is approved");
            assert_eq!(command.trim(), command);
        }
        // Distinct: four rows offering the same line would be one of them being installed four
        // times over.
        let all: std::collections::BTreeSet<_> =
            Agent::all().into_iter().map(install_command).collect();
        assert_eq!(all.len(), Agent::all().len());
    }

    /// The two sentences the offer can end in say the command out loud, in both languages: a
    /// status line that says "not installed" and nothing else leaves the user where they started.
    #[test]
    fn the_install_offer_says_what_it_did_and_what_it_typed() {
        let command = install_command(Agent::Gemini);
        for lang in [Lang::En, Lang::It] {
            let typed = msg_install_typed(lang, "gemini", command);
            assert!(typed.contains(command) && typed.contains("gemini"));
            let none = msg_install_no_shell(lang, "gemini", command);
            assert!(none.contains(command) && none.contains("gemini"));
        }
    }

    /// The launcher's own PATH question answers, and forgetting makes the next ask a real one.
    /// It cannot assert *what* is installed on the machine running the suite — that is the point
    /// of the function — so it asserts it is stable while remembered and survives being forgotten.
    #[test]
    fn the_installed_answer_can_be_forgotten() {
        let first: Vec<bool> = Agent::all().into_iter().map(installed).collect();
        let again: Vec<bool> = Agent::all().into_iter().map(installed).collect();
        assert_eq!(first, again, "the memo answers the same way until it expires");
        forget_installed();
        let after: Vec<bool> = Agent::all().into_iter().map(installed).collect();
        assert_eq!(first, after, "and asking again finds the same machine");
    }

    /// The whole of autocollapse, in the only place it can be tested without an `App`: pinned
    /// ignores the focus, autocollapse is on screen exactly while it holds the keyboard.
    #[test]
    fn autocollapse_is_on_screen_only_while_it_has_the_keyboard() {
        assert!(stays_open(true, true));
        assert!(stays_open(true, false), "pinned means pinned: looking away is not dismissing");
        assert!(stays_open(false, true));
        assert!(!stays_open(false, false), "the focus leaving is what puts it away");
    }

    #[test]
    fn the_highlight_wraps_both_ways() {
        let mut drawer = Drawer::with_launcher(None);
        assert_eq!(drawer.highlighted(), Agent::all()[0]);
        drawer.move_selection(-1);
        assert_eq!(drawer.highlighted(), Agent::all()[3], "up from the first is the last");
        drawer.move_selection(1);
        assert_eq!(drawer.highlighted(), Agent::all()[0], "and down again is the first");
        for _ in 0..4 {
            drawer.move_selection(1);
        }
        assert_eq!(drawer.highlighted(), Agent::all()[0], "a full turn is where it started");
    }

    /// The last agent used is where the highlight starts, which is the whole of "remembers".
    #[test]
    fn the_launcher_opens_on_the_last_agent_used() {
        let drawer = Drawer::with_launcher(Some(Agent::Codex));
        assert_eq!(drawer.highlighted(), Agent::Codex);
        assert!(drawer.showing_launcher());
    }

    /// An agent that has exited leaves the launcher, not a shell — and leaves it pointing at the
    /// agent that just ended, which is the one most likely to be wanted again.
    #[test]
    fn an_exited_agent_returns_the_drawer_to_the_launcher() {
        let mut drawer = Drawer::with_launcher(Some(Agent::Gemini));
        drawer.agent = Some(Agent::Gemini);
        drawer.back_to_launcher();
        assert!(drawer.showing_launcher());
        assert_eq!(drawer.highlighted(), Agent::Gemini);
    }
}
