//! The agent drawer: a column on the right of the window that holds the coding agents.
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
    /// The agents' panes, or `None` while nothing has been started. It is a whole `TerminalWindow`
    /// rather than a bare panel so the drawing code needs no special case, and it may hold several
    /// tabs — one agent each.
    ///
    /// **Every tab holds an agent, never a shell.** That is the rule that replaces the old
    /// one-tab rule, and it is the one worth keeping: the strip is not an invitation to make this
    /// a second terminal panel, because the only door a tab can come through is the launcher.
    /// There is no "new shell here" in this column and there is no path that leaves a bare prompt
    /// in a pane shaped like an agent — see `launch_drawer_agent`, which is the sole caller of
    /// `add_tab` on this window.
    pub window: Option<TerminalWindow>,
    /// Which agent was launched last. Kept across an agent exiting so the launcher can put the
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
    /// Whether the launcher is up *over* agents that are already running — someone asking for
    /// another tab.
    ///
    /// A flag rather than a fifth thing the window could contain, because a tab in this column is
    /// only ever born from a chosen agent: while the choice is being made there is nothing to put
    /// in a tab yet, so a half-made tab would have to be drawn as something, and the only honest
    /// something is a shell prompt — the one thing this panel must never show. So the launcher
    /// takes the whole pane instead, exactly as it does on the first launch, and the tabs behind
    /// it go on running untouched. Cancel (`Esc`) clears the flag and hands the column straight
    /// back to the agent that had it; choosing one clears it too, in `launch_drawer_agent`.
    pub choosing: bool,
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
            // Nothing is running, so there is nothing to be choosing *over*: an empty window is
            // already the launcher, and the flag would be a second way of saying so.
            choosing: false,
        }
    }

    /// Whether the launcher is what is drawn in it right now.
    ///
    /// Two ways in, and they are the same screen: nothing has been started yet, or agents are
    /// running and one more has been asked for. The second is why this is not simply
    /// `window.is_none()` any more — see [`Drawer::choosing`].
    pub fn showing_launcher(&self) -> bool {
        self.window.is_none() || self.choosing
    }

    /// Puts the launcher up over whatever is running, to choose the agent for another tab.
    pub fn start_choosing(&mut self) {
        // The same staleness the drawer opening has: the list is about to say which of the four
        // are installed, and the user may have installed one since it last asked.
        forget_installed();
        self.choosing = true;
        if let Some(agent) = self.agent {
            self.selected = agent.index();
        }
    }

    /// Takes it back down again, leaving the running tabs exactly as they were.
    ///
    /// The cancel half of [`Drawer::start_choosing`], and it says nothing about the focus: the
    /// keyboard was in this column before the launcher went up and has no reason to leave it
    /// because a choice was not made. `app.rs` is where that distinction is spent.
    pub fn stop_choosing(&mut self) {
        self.choosing = false;
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
    /// The honest outcome of the last agent exiting: the conversation is over, and what is on
    /// offer is the same choice as before. Nothing is respawned — a shell appearing where an agent
    /// was is the one thing this panel must never do, because it looks exactly like the agent
    /// still being there.
    ///
    /// Called only when the drawer has *emptied*: with tabs left, the one that ended is removed
    /// and the rest go on. `choosing` is cleared with the window, because the flag only ever meant
    /// "the launcher is up over something" and there is nothing left for it to be over.
    pub fn back_to_launcher(&mut self) {
        self.window = None;
        self.choosing = false;
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
/// Clawd, the small coral critter Claude Code greets you as: the wide body, the two square black
/// eyes, the stubby arms out the sides and the four legs it stands on. Drawn from the welcome
/// screen's own proportions — the critter alone, because the critter alone is how that CLI says
/// hello.
const CLAUDE_ART: &[&str] = &[
    "..AAAAAAAAAAA..",
    "..AAAAAAAAAAA..",
    // The eyes sit on pixel rows 2 and 3 on purpose: that pair is exactly one cell, so each eye
    // is a crisp full block rather than two half-block smudges astride a cell boundary.
    "..AAEEAAAEEAA..",
    "AAAAEEAAAEEAAAA",
    "AAAAAAAAAAAAAAA",
    "..AAAAAAAAAAA..",
    "..AAAAAAAAAAA..",
    "..AA.AA.AA.AA..",
    "..AA.AA.AA.AA..",
    "..AA.AA.AA.AA..",
];

/// Codex's cloud: the lobed blob with the white prompt inside it — a chevron and the underscore
/// of a cursor. Every cloud pixel is the same ink letter: the colour comes from how far down the
/// row is — see [`codex_gradient`] — which is the lavender-into-blue wash the real icon wears.
const CODEX_ART: &[&str] = &[
    "...DDDDDDD...",
    "..DDDDDDDDD..",
    ".DDDDDDDDDDD.",
    "DDDPPDDDDDDDD",
    "DDDDPPDDDDDDD",
    "DDDDDPPDDDDDD",
    "DDDDPPDDDDDDD",
    ".DDPPDDPPPPD.",
    "..DDDDDPPPP..",
    "...DDDDDDD...",
];

/// Gemini CLI's tile: the rounded dark square, its border running blue on the left into pink on
/// the right, and the bold blue chevron in the middle with the violet caught on its point.
const GEMINI_ART: &[&str] = &[
    ".BBBBVVVVRRR.",
    "BNNNNNNNNNNNR",
    "BNNNCCNNNNNNR",
    "BNNNNCCNNNNNR",
    "BNNNNNVVNNNNR",
    "BNNNNNVVNNNNR",
    "BNNNNCCNNNNNR",
    "BNNNCCNNNNNNR",
    "BNNNNNNNNNNNR",
    ".BBBBBBVVRRR.",
];

/// A colour, as it is written down by the people who own it. Not a [`crate::theme::Palette`]
/// role: these four are brand colours and are fixed for the same reason the file tree's icons
/// are, so `drawer.rs` says them in plain numbers and `ui.rs` turns them into whatever the
/// drawing library calls a colour.
pub type Ink = (u8, u8, u8);

/// Clawd's coral, which is Anthropic's coral, sampled from the welcome screen itself.
const CLAWD_CORAL: Ink = (0xD9, 0x77, 0x57);
/// And the black of its eyes.
const CLAWD_EYE: Ink = (0x00, 0x00, 0x00);
/// opencode's two tones, off their own splash: "open" in the grey, "code" in the white.
const OPENCODE_GREY: Ink = (0xB4, 0xB2, 0xB2);
const OPENCODE_WHITE: Ink = (0xEF, 0xED, 0xED);
/// Codex's cloud runs lavender at the top into blue at the bottom; the prompt inside is white.
const CODEX_LAVENDER: Ink = (0xA9, 0xA6, 0xFF);
const CODEX_BLUE: Ink = (0x3E, 0x49, 0xFF);
const CODEX_PROMPT: Ink = (0xFF, 0xFF, 0xFF);
/// Gemini CLI's tile: the border's blue, the violet it passes through, the pink it arrives at,
/// the near-black it holds, and the chevron's own blue.
const GEMINI_BLUE: Ink = (0x1B, 0x80, 0xFD);
const GEMINI_VIOLET: Ink = (0x7F, 0x89, 0xEF);
const GEMINI_PINK: Ink = (0xD7, 0x61, 0x8E);
const GEMINI_NIGHT: Ink = (0x1E, 0x1E, 0x2E);
const GEMINI_CHEVRON: Ink = (0x0C, 0x8A, 0xFC);

/// How many pixel rows a mark is written in. Twice [`ART_ROWS`], because a half-block cell holds
/// two of them.
const ART_PIXEL_ROWS: usize = 10;

/// How many cells tall a mark is drawn.
pub const ART_ROWS: usize = ART_PIXEL_ROWS / 2;

/// The gradient down Codex's cloud: lavender at the top pixel row, blue at the bottom, mixed
/// evenly in between. Rows past the art are clamped rather than wrapped, so a caller that asks
/// for a row that is not there gets the end of the gradient and never a panic.
fn codex_gradient(row: usize) -> Ink {
    let last = (ART_PIXEL_ROWS - 1) as u32;
    let at = (row as u32).min(last);
    let mix = |from: u8, to: u8| {
        let (from, to) = (from as u32, to as u32);
        // Integer arithmetic, rounded to nearest, so the ten steps are the same ten on every
        // machine and the test below can name them.
        ((from * (last - at) + to * at + last / 2) / last) as u8
    };
    (
        mix(CODEX_LAVENDER.0, CODEX_BLUE.0),
        mix(CODEX_LAVENDER.1, CODEX_BLUE.1),
        mix(CODEX_LAVENDER.2, CODEX_BLUE.2),
    )
}

/// The mascot half of an agent's mark. `None` is opencode, whose name *is* its mark — their
/// splash is the word and nothing but the word — so its row is the wordmark alone. A `match`
/// rather than a lookup, so a fifth agent added to [`Agent::all`] does not compile until someone
/// has decided what its row shows — the block alphabet this replaced could only find that out at
/// run time, and answered by drawing nothing.
fn mascot(agent: Agent) -> Option<&'static [&'static str]> {
    match agent {
        Agent::Claude => Some(CLAUDE_ART),
        Agent::OpenCode => None,
        Agent::Codex => Some(CODEX_ART),
        Agent::Gemini => Some(GEMINI_ART),
    }
}

/// The brick alphabet the names are written in, beside each mascot: six pixel rows to a letter —
/// three cells — `#` where the ink goes. Thirteen letters because the four names need thirteen,
/// and no more for the reason the old three-row alphabet had no more: a font file for thirteen
/// letters is a dependency for the sake of having one.
const NAME_FONT: [(char, [&str; 6]); 13] = [
    ('a', [".#.", "#.#", "###", "#.#", "#.#", "#.#"]),
    ('c', ["###", "#..", "#..", "#..", "#..", "###"]),
    ('d', ["##.", "#.#", "#.#", "#.#", "#.#", "##."]),
    ('e', ["###", "#..", "##.", "#..", "#..", "###"]),
    ('g', ["###", "#..", "#..", "#.#", "#.#", "###"]),
    ('i', ["#", "#", "#", "#", "#", "#"]),
    ('l', ["#..", "#..", "#..", "#..", "#..", "###"]),
    ('m', ["#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#"]),
    ('n', ["#..#", "##.#", "##.#", "#.##", "#.##", "#..#"]),
    ('o', ["###", "#.#", "#.#", "#.#", "#.#", "###"]),
    ('p', ["###", "#.#", "###", "#..", "#..", "#.."]),
    ('u', ["#.#", "#.#", "#.#", "#.#", "#.#", "###"]),
    ('x', ["#.#", "#.#", ".#.", ".#.", "#.#", "#.#"]),
];

/// The pixel row the name's letters start on. Rows 2..8 is six rows on an even boundary, which
/// is three whole cells: letters astride a cell boundary come out as half-block smudges, and a
/// name has to stay text-crisp beside a mascot that is allowed to be a picture.
const NAME_TOP: usize = 2;

/// The ink one letter of `agent`'s name is written in. One each, except opencode: "open" in
/// their grey and "code" in their white is the split that makes the word their mark.
fn name_ink(agent: Agent, letter: usize) -> char {
    match agent {
        Agent::Claude => 'A',
        Agent::OpenCode => {
            if letter < 4 {
                'G'
            } else {
                'W'
            }
        }
        Agent::Codex => 'P',
        Agent::Gemini => 'C',
    }
}

/// The whole mark as rows of ink letters: mascot, a two-column gap, and the name in bricks —
/// or the name alone, where the name is the mark. A letter the font does not have is skipped
/// rather than drawn as a hole; the test below is what keeps that an impossibility rather than
/// a behaviour.
fn pixel_grid(agent: Agent) -> Vec<String> {
    let mut rows: Vec<String> = match mascot(agent) {
        Some(grid) => grid.iter().map(|row| format!("{row}..")).collect(),
        None => vec![String::new(); ART_PIXEL_ROWS],
    };
    for (i, letter) in agent.workspace_name().chars().enumerate() {
        let Some((_, glyph)) = NAME_FONT.iter().find(|(l, _)| *l == letter) else { continue };
        let ink = name_ink(agent, i);
        let width = glyph[0].chars().count();
        for (at, row) in rows.iter_mut().enumerate() {
            // The gap between letters; the gap after the mascot is the mascot's own two columns.
            if i > 0 {
                row.push('.');
            }
            match at.checked_sub(NAME_TOP).and_then(|line| glyph.get(line)) {
                Some(line) => row.extend(line.chars().map(|p| if p == '#' { ink } else { '.' })),
                None => row.extend(std::iter::repeat_n('.', width)),
            }
        }
    }
    rows
}

/// What colour one pixel is: its letter says which ink, and the pixel row says how far down a
/// gradient it is — which only Codex's cloud uses. `None` is the drawer showing through.
fn ink(mark: char, row: usize) -> Option<Ink> {
    match mark {
        'A' => Some(CLAWD_CORAL),
        'E' => Some(CLAWD_EYE),
        'G' => Some(OPENCODE_GREY),
        'W' => Some(OPENCODE_WHITE),
        'D' => Some(codex_gradient(row)),
        'P' => Some(CODEX_PROMPT),
        'B' => Some(GEMINI_BLUE),
        'V' => Some(GEMINI_VIOLET),
        'R' => Some(GEMINI_PINK),
        'N' => Some(GEMINI_NIGHT),
        'C' => Some(GEMINI_CHEVRON),
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
    pixel_grid(agent).iter().map(|row| row.chars().count()).max().unwrap_or(0) as u16
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
/// colour behind it, which is what lets the cloud's gradient change inside a single row of the
/// terminal; one lit is the half-block on that side; neither is a space.
pub fn art(agent: Agent) -> Vec<Vec<ArtCell>> {
    let grid = pixel_grid(agent);
    let width = grid.iter().map(|row| row.chars().count()).max().unwrap_or(0);
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
            // The font spells every name the launcher offers. `pixel_grid` skips a letter it
            // does not have rather than panicking, which is exactly why a fifth agent arriving
            // without its letters has to be caught here and not on screen.
            for letter in name.chars() {
                assert!(
                    NAME_FONT.iter().any(|(l, _)| *l == letter),
                    "{name}: no bricks for {letter:?}"
                );
            }
            let grid = pixel_grid(agent);
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
    /// carries the cloud's gradient through a single row of the terminal.
    #[test]
    fn two_pixels_become_one_cell() {
        let codex = art(Agent::Codex);
        // The cloud's top pixel row is empty at the left edge and its second is too, so the
        // corner cell is a space with no ink at all.
        assert_eq!(codex[0][0], ArtCell { ch: ' ', fg: None, bg: None });
        // The cloud's crown is lit in the upper half of its first cell row and in the lower half
        // too — but the gradient has moved between them, so the cell carries both.
        let crown = codex[0][5];
        assert_eq!(crown.ch, '▀');
        assert_eq!(crown.fg, Some(codex_gradient(0)));
        assert_eq!(crown.bg, Some(codex_gradient(1)));
        assert_ne!(crown.fg, crown.bg, "the whole point of the second colour");
        // The tile's left border is lit in both halves of its middle rows, one ink: a full block.
        let gemini = art(Agent::Gemini);
        assert_eq!(gemini[1][0], ArtCell { ch: '█', fg: Some(GEMINI_BLUE), bg: None });
        // A half on its own is the half-block on that side, and nothing behind it: the border's
        // rounded corner leaves the tile's top-left cell lit only below.
        assert_eq!(gemini[0][0], ArtCell { ch: '▄', fg: Some(GEMINI_BLUE), bg: None });
        // And two same-ink halves anywhere else are one block: Clawd's shoulder.
        let claude = art(Agent::Claude);
        assert_eq!(claude[0][2], ArtCell { ch: '█', fg: Some(CLAWD_CORAL), bg: None });
    }

    /// The gradient reaches both ends and only moves downwards. A "gradient" that arrived at the
    /// bottom still lavender would be four rows of flat colour nobody would notice was wrong.
    #[test]
    fn the_cloud_runs_lavender_into_blue() {
        assert_eq!(codex_gradient(0), CODEX_LAVENDER);
        assert_eq!(codex_gradient(ART_PIXEL_ROWS - 1), CODEX_BLUE);
        assert_eq!(codex_gradient(999), CODEX_BLUE, "past the end is the end, not a panic");
        for row in 1..ART_PIXEL_ROWS {
            let (before, now) = (codex_gradient(row - 1), codex_gradient(row));
            assert!(now.0 <= before.0 && now.1 <= before.1 && now.2 >= before.2, "row {row}");
        }
        // And the two ends of the drawn art are visibly apart, which is what a reader sees.
        let art = art(Agent::Codex);
        let top = art[0][6].fg.unwrap();
        let bottom = art[ART_ROWS - 1][6].bg.or(art[ART_ROWS - 1][6].fg).unwrap();
        assert_ne!(top, bottom);
    }

    /// The marks are drawn in the colours their owners use, and those are values rather than
    /// palette roles — the same rule the file tree's icons run under. A mark that changed colour
    /// with the theme would stop being the mark.
    #[test]
    fn the_marks_are_drawn_in_their_own_colours() {
        let coral = |agent| art(agent).into_iter().flatten().any(|c| c.fg == Some(CLAWD_CORAL));
        assert!(coral(Agent::Claude), "Clawd is Anthropic's coral");
        assert!(!coral(Agent::Codex), "and nobody else's mark is");
        let inks = |agent| {
            art(agent).into_iter().flatten().flat_map(|c| [c.fg, c.bg]).flatten().collect::<Vec<_>>()
        };
        assert!(inks(Agent::Claude).contains(&CLAWD_EYE), "the eyes are what make it Clawd");
        let word = inks(Agent::OpenCode);
        assert!(
            word.contains(&OPENCODE_GREY) && word.contains(&OPENCODE_WHITE),
            "open in the grey, code in the white: the split is the signature"
        );
        let tile = inks(Agent::Gemini);
        assert!(
            tile.contains(&GEMINI_BLUE) && tile.contains(&GEMINI_PINK),
            "the border runs blue into pink, so both ends have to be there"
        );
        assert!(inks(Agent::Codex).contains(&CODEX_PROMPT), "the prompt is what the cloud says");
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

    /// A drawer with something in its pane, without needing a pty to say so.
    ///
    /// `window` is only ever read here through `is_none`, so the whole of "an agent is running"
    /// for these tests is a window that exists — and a `TerminalWindow` with no tabs is one, while
    /// costing no shell on a build runner. What the tabs *are* is the app's business and is tested
    /// there; what is the state machine's business is which screen the column shows.
    fn with_an_agent_running(agent: Agent) -> Drawer {
        let mut drawer = Drawer::with_launcher(Some(agent));
        drawer.agent = Some(agent);
        drawer.window = Some(TerminalWindow {
            tabs: Vec::new(),
            active: 0,
            weight: crate::terminal_panel::TERMINAL_WEIGHT_DEFAULT,
        });
        drawer
    }

    /// The whole of which screen the column shows, at the four states it has. The launcher is one
    /// screen reached two ways — nothing started yet, or another tab asked for — and the flag is
    /// what makes the second possible without a half-made tab existing anywhere.
    #[test]
    fn the_launcher_shows_when_nothing_is_running_and_when_another_tab_is_asked_for() {
        let fresh = Drawer::with_launcher(None);
        assert!(fresh.showing_launcher(), "a drawer with nothing in it is the launcher");
        assert!(!fresh.choosing, "and it is not *choosing over* anything: there is nothing there");

        let mut running = with_an_agent_running(Agent::Claude);
        assert!(!running.showing_launcher(), "an agent in the pane is what is drawn");

        running.start_choosing();
        assert!(running.showing_launcher(), "asking for another tab puts the list over it");
        assert_eq!(running.highlighted(), Agent::Claude, "on the last one used, as ever");

        // Esc: the choice is cancelled and the column goes straight back to the agent that had
        // it. Nothing about the running tabs was ever touched.
        running.stop_choosing();
        assert!(!running.showing_launcher(), "cancelling returns the agent, it does not empty it");
        assert!(running.window.is_some());
    }

    /// The drawer emptying clears the flag with the window. `choosing` means "the list is up over
    /// something"; with nothing left to be over, a flag still set would be a second, invisible
    /// reason the launcher was showing — and `stop_choosing` would then hand the column back to a
    /// pane that is gone.
    #[test]
    fn emptying_the_drawer_clears_the_choice_it_was_in_the_middle_of() {
        let mut drawer = with_an_agent_running(Agent::Codex);
        drawer.start_choosing();
        drawer.back_to_launcher();
        assert!(!drawer.choosing);
        assert!(drawer.showing_launcher(), "and the launcher is showing for the plain reason");
        drawer.stop_choosing();
        assert!(drawer.showing_launcher(), "with nothing to go back to, cancelling changes nothing");
        assert_eq!(drawer.highlighted(), Agent::Codex);
    }
}
