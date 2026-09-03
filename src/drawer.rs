//! The agent drawer: a column on the right of the window that holds one coding agent.
//!
//! It is a terminal window like any other inside — a pty, a vt100 parser, the same drawing code —
//! and everything interesting about it is where it *lives*.

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
        if let Some(agent) = self.agent {
            self.selected = agent.index();
        }
    }
}

/// A three-row block alphabet, three cells to a letter.
///
/// Hand-drawn here rather than pulled from a figlet font because it needs to spell exactly four
/// words and nothing else, and a font file for thirteen letters is a dependency for the sake of
/// having one. `SPLASH_BANNER` in `ui.rs` is the same idea at a larger size.
///
/// **Wordmarks, not logos.** The four names are drawn in CleeCode's own lettering, the same way
/// the themes name the editors they are quoting: naming a program in order to launch it is
/// nominative use, and redrawing somebody's mark is a different thing entirely. The rule is in
/// docs/ROADMAP.md, under the drawer.
const LETTERS: [(char, [&str; 3]); 13] = [
    ('a', ["▄▀▄", "█▀█", "▀ ▀"]),
    ('c', ["▄▀▀", "█  ", "▀▄▄"]),
    ('d', ["  █", "▄▀█", "▀▄▀"]),
    ('e', ["▄▀▄", "█▀▀", "▀▄▀"]),
    ('g', ["▄▀▄", "█ █", "▀▄█"]),
    ('i', [" ▄ ", " █ ", " ▀ "]),
    ('l', [" █ ", " █ ", " ▀ "]),
    ('m', ["▄▄▄", "█▀█", "▀ ▀"]),
    ('n', ["▄▀▄", "█ █", "▀ ▀"]),
    ('o', ["▄▀▄", "█ █", "▀▄▀"]),
    ('p', ["▄▀▄", "█▀▀", "▀  "]),
    ('u', ["▄ ▄", "█ █", "▀▄▀"]),
    ('x', ["▄ ▄", " █ ", "▀ ▀"]),
];

/// How tall a wordmark is.
pub const WORDMARK_ROWS: usize = 3;

/// How wide `name` comes out, in cells: three per letter and one between them.
pub fn wordmark_width(name: &str) -> u16 {
    let letters = name.chars().count() as u16;
    (letters * 4).saturating_sub(1)
}

/// `name` in the alphabet above, as [`WORDMARK_ROWS`] rows.
///
/// `None` for a name this alphabet cannot spell, which is how the launcher stays honest about a
/// fifth agent added to [`Agent::all`] without a letter being added here: it falls back to the
/// plain name rather than drawing a word with a hole in it.
pub fn wordmark(name: &str) -> Option<[String; WORDMARK_ROWS]> {
    let mut rows = [String::new(), String::new(), String::new()];
    for (i, ch) in name.chars().enumerate() {
        let (_, glyph) = LETTERS.iter().find(|(letter, _)| *letter == ch)?;
        for (row, part) in rows.iter_mut().zip(glyph) {
            if i > 0 {
                row.push(' ');
            }
            row.push_str(part);
        }
    }
    Some(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The alphabet has to spell every name the launcher offers. A fifth agent — the enum is
    /// built to take one — arriving without its letters would otherwise be found on screen.
    #[test]
    fn every_agent_name_can_be_spelled() {
        for agent in Agent::all() {
            let name = agent.workspace_name();
            let drawn = wordmark(name).unwrap_or_else(|| panic!("no wordmark for {name}"));
            for row in &drawn {
                assert_eq!(
                    row.chars().count() as u16,
                    wordmark_width(name),
                    "{name}: every row is the width the layout was told to expect"
                );
            }
        }
    }

    /// A name outside the alphabet is refused rather than drawn with gaps in it.
    #[test]
    fn an_unspellable_name_has_no_wordmark() {
        assert!(wordmark("zephyr").is_none());
        assert!(wordmark("CLAUDE").is_none(), "the alphabet is lower case, as the names are");
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
