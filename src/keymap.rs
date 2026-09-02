//! Where the application layer's chords come from.
//!
//! CleeCode's own keys live on `Ctrl+Shift+<letter>` and `Ctrl+Shift+<arrow>`, and the reasons
//! for that choice — no function keys, no `Alt+<letter>`, no `Ctrl+<arrow>` — are written out
//! beside the dispatch in `app.rs`. They are sound *for an Italian layout on macOS*, which is
//! the machine this is developed on, and arbitrary for everybody else: a layout where `Ctrl` is
//! somewhere else, a terminal that eats one of these, a keyboard with no arrow cluster.
//!
//! So the table is not the law any more, it is the default. A `[keys]` table in settings.toml
//! moves one chord at a time:
//!
//! ```toml
//! [keys]
//! find-in-project = "Ctrl+Alt+F"
//! ```
//!
//! and everything not named there keeps the chord it always had. This is deliberately not a
//! keymap system in the vim sense — there are no modes, no sequences, and no way to bind a key
//! to something that is not already an action. It is the ability to move a chord that does not
//! exist on your keyboard.
//!
//! Two rules that follow from being user input rather than source:
//!
//! * nothing here panics. A misspelt action, a chord nobody can type, two actions on the same
//!   chord — each is a warning on the status line and a default left in place, never an error
//!   that stops the editor from starting.
//! * where two actions end up sharing a chord, the one declared first in [`Action`] wins, and
//!   the other simply never fires. [`Keymap::action_for`] walks the actions in that order and
//!   answers with the first match, so the rule is the loop rather than a special case.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::i18n::{self, Lang};

/// A chord the user can move. One variant per key the application layer owns.
///
/// Declaration order is load-bearing twice over: it decides who wins a collision, and it is the
/// order the commented block written into settings.toml lists them in.
///
/// What is *not* here is as deliberate as what is. The letters typed inside a modal or the git
/// panel are that box's alphabet, not chords — remapping `y` in a yes/no prompt would be
/// remapping the word "yes". `Ctrl+Tab` is the one key a focused terminal gives back, and
/// `Alt+1/2/3` the layout presets; both are answers to a constraint rather than preferences.
/// And nothing inside a terminal is ours to move at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Manual,
    Settings,
    RunFile,
    RunSelection,
    ToggleBreakpoint,
    InspectVariable,
    NewTerminalWindow,
    NewTerminalTab,
    CloseTerminalTab,
    ToggleFold,
    ResizeMode,
    MenuBar,
    ContextMenu,
    GoToDefinition,
    JumpBack,
    GitPanel,
    FindInProject,
    NextTab,
    PrevTab,
    NextTerminal,
    PrevTerminal,
    RenameTerminal,
    SaveWorkspace,
    SaveAll,
}

/// Every action, its name in settings.toml, and the chord CleeCode ships with.
///
/// One table rather than three matches, because the three would drift: an action added to the
/// enum and forgotten here is caught by `every_action_has_a_default`, and there is nowhere else
/// for it to hide. Built once and kept, because it is walked on every key press.
///
/// The names are kebab-case and say what the action does, not which key it happens to be on:
/// `find-in-project`, not `ctrl-shift-h`. A name that quoted the default would be wrong the
/// moment somebody used this feature.
fn table() -> &'static [(Action, &'static str, Chord)] {
    static TABLE: std::sync::OnceLock<Vec<(Action, &'static str, Chord)>> =
        std::sync::OnceLock::new();
    TABLE.get_or_init(build_table)
}

fn build_table() -> Vec<(Action, &'static str, Chord)> {
    let cs = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
    let chord = |code| Chord { mods: cs, code };
    let letter = |c| chord(KeyCode::Char(c));
    vec![
        (Action::Manual, "manual", letter('m')),
        (Action::Settings, "settings", letter('o')),
        (Action::RunFile, "run-file", letter('r')),
        (Action::RunSelection, "run-selection", letter('x')),
        (Action::ToggleBreakpoint, "toggle-breakpoint", letter('p')),
        (Action::InspectVariable, "inspect-variable", letter('i')),
        (Action::NewTerminalWindow, "new-terminal-window", letter('n')),
        (Action::NewTerminalTab, "new-terminal-tab", letter('t')),
        (Action::CloseTerminalTab, "close-terminal-tab", letter('k')),
        (Action::ToggleFold, "toggle-fold", letter('f')),
        (Action::ResizeMode, "resize-mode", letter('u')),
        (Action::MenuBar, "menu-bar", letter('b')),
        (Action::ContextMenu, "context-menu", letter('g')),
        (Action::GoToDefinition, "go-to-definition", letter('j')),
        (Action::JumpBack, "jump-back", letter('l')),
        (Action::GitPanel, "git-panel", letter('d')),
        (Action::FindInProject, "find-in-project", letter('h')),
        (Action::NextTab, "next-tab", chord(KeyCode::Right)),
        (Action::PrevTab, "prev-tab", chord(KeyCode::Left)),
        (Action::NextTerminal, "next-terminal", chord(KeyCode::Down)),
        (Action::PrevTerminal, "prev-terminal", chord(KeyCode::Up)),
        (Action::RenameTerminal, "rename-terminal", letter('e')),
        (Action::SaveWorkspace, "save-workspace", letter('w')),
        (Action::SaveAll, "save-all", letter('s')),
    ]
}

impl Action {
    /// Every action, in declaration order.
    pub fn all() -> impl Iterator<Item = Action> {
        table().iter().map(|(action, _, _)| *action)
    }

    /// The name this action answers to in settings.toml.
    pub fn name(self) -> &'static str {
        table().get(self.index()).map(|(_, name, _)| *name).unwrap_or("")
    }

    pub fn from_name(name: &str) -> Option<Action> {
        table().iter().find(|(_, n, _)| *n == name).map(|(action, _, _)| *action)
    }

    /// The chord CleeCode ships with, whatever settings.toml says.
    pub fn default_chord(self) -> Chord {
        table()
            .get(self.index())
            .map(|(_, _, chord)| *chord)
            .unwrap_or(Chord { mods: KeyModifiers::NONE, code: KeyCode::Null })
    }

    /// Where this action sits in the table, which is where its chord sits in the keymap.
    fn index(self) -> usize {
        table().iter().position(|(action, _, _)| *action == self).unwrap_or(0)
    }
}

/// The modifiers a chord can carry. `SUPER` is masked out rather than compared: the Command key
/// reaches a terminal application only by accident, and a chord that stopped working because
/// something else was also held down would be a chord that works on one machine.
const RELEVANT: KeyModifiers =
    KeyModifiers::CONTROL.union(KeyModifiers::ALT).union(KeyModifiers::SHIFT);

/// One key press: modifiers, and the key they are held with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Chord {
    pub mods: KeyModifiers,
    pub code: KeyCode,
}

impl Chord {
    /// Reads a chord out of settings.toml: modifiers first, the key last, joined by `+`.
    ///
    /// Case does not matter anywhere — `Ctrl+Shift+M` and `ctrl+shift+m` are the same chord, and
    /// [`Chord::display`] round-trips through this. Both the words and the arrows are accepted
    /// for the four directions, because the manual writes `←` and a keyboard says `left`.
    ///
    /// The project's own rules — no function keys, no `Alt+<letter>` — are rules for *our*
    /// defaults, not for this. Somebody whose layout makes `F5` the comfortable key is exactly
    /// who this exists for.
    pub fn parse(text: &str) -> Result<Chord, String> {
        let mut mods = KeyModifiers::NONE;
        let mut key: Option<KeyCode> = None;
        for part in text.split('+') {
            let part = part.trim();
            if part.is_empty() {
                return Err(format!("\"{text}\" has an empty piece between two +"));
            }
            if key.is_some() {
                return Err(format!("\"{text}\" names more than one key"));
            }
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => mods |= KeyModifiers::CONTROL,
                "shift" => mods |= KeyModifiers::SHIFT,
                "alt" | "option" | "meta" => mods |= KeyModifiers::ALT,
                _ => key = Some(parse_key(part)?),
            }
        }
        match key {
            Some(code) => Ok(Chord { mods, code }),
            None => Err(format!("\"{text}\" names modifiers but no key")),
        }
    }

    /// The chord as the manual and the menus write it: `Ctrl+Shift+M`, `Ctrl+Alt+←`.
    ///
    /// Modifier order is Ctrl, Alt, Shift, which is the order every string already written in
    /// this source uses. Getting it wrong would be invisible in a test and obvious in a menu
    /// sitting next to a hard-coded neighbour.
    pub fn display(&self) -> String {
        let mut out = String::new();
        if self.mods.contains(KeyModifiers::CONTROL) {
            out.push_str("Ctrl+");
        }
        if self.mods.contains(KeyModifiers::ALT) {
            out.push_str("Alt+");
        }
        if self.mods.contains(KeyModifiers::SHIFT) {
            out.push_str("Shift+");
        }
        out.push_str(&key_name(self.code));
        out
    }

    /// Whether a key press is this chord.
    ///
    /// Letters are compared without case, because that is how they arrive: a terminal sends
    /// `Ctrl+Shift+M` as `M` with both modifiers on some emulators and as `m` on others, and the
    /// dispatch this replaces spelled out both spellings in every single arm for that reason.
    pub fn matches(&self, key: KeyEvent) -> bool {
        if (key.modifiers & RELEVANT) != (self.mods & RELEVANT) {
            return false;
        }
        match (self.code, key.code) {
            (KeyCode::Char(a), KeyCode::Char(b)) => a.eq_ignore_ascii_case(&b),
            (a, b) => a == b,
        }
    }
}

fn parse_key(part: &str) -> Result<KeyCode, String> {
    let lower = part.to_lowercase();
    let named = match lower.as_str() {
        "left" | "←" => Some(KeyCode::Left),
        "right" | "→" => Some(KeyCode::Right),
        "up" | "↑" => Some(KeyCode::Up),
        "down" | "↓" => Some(KeyCode::Down),
        "enter" | "return" => Some(KeyCode::Enter),
        "tab" => Some(KeyCode::Tab),
        "esc" | "escape" => Some(KeyCode::Esc),
        "space" => Some(KeyCode::Char(' ')),
        _ => None,
    };
    if let Some(code) = named {
        return Ok(code);
    }
    // F1..F12. Only the twelve a keyboard has: `F40` parses as a number and is not a key.
    if let Some(digits) = lower.strip_prefix('f') {
        if let Ok(n) = digits.parse::<u8>() {
            return if (1..=12).contains(&n) {
                Ok(KeyCode::F(n))
            } else {
                Err(format!("\"{part}\" is not one of F1 to F12"))
            };
        }
    }
    let mut chars = lower.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphanumeric() => Ok(KeyCode::Char(c)),
        _ => Err(format!("\"{part}\" is not a key name")),
    }
}

/// How a key is written for a reader. The arrows are the glyphs the manual draws, not the words.
fn key_name(code: KeyCode) -> String {
    match code {
        KeyCode::Left => "←".to_string(),
        KeyCode::Right => "→".to_string(),
        KeyCode::Up => "↑".to_string(),
        KeyCode::Down => "↓".to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) => c.to_ascii_uppercase().to_string(),
        KeyCode::F(n) => format!("F{n}"),
        other => format!("{other:?}"),
    }
}

/// The chords in force: the defaults with whatever settings.toml moved.
pub struct Keymap {
    /// One chord per action, indexed by `Action::index`.
    chords: Vec<Chord>,
    /// For each action whose chord moved, how it used to be written and how it is written now,
    /// longest spelling first. This is what lets the manual and the menus stop lying without
    /// either of them having to know what a chord is: they hand over a line of text and get
    /// back the same line with the moved chords rewritten. Empty in the overwhelmingly common
    /// case of nobody having remapped anything, which makes [`Keymap::relabel`] free.
    relabels: Vec<(String, String)>,
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap { chords: table().iter().map(|(_, _, chord)| *chord).collect(), relabels: Vec::new() }
    }
}

impl Keymap {
    /// Builds the keymap from a `[keys]` table, together with everything worth telling the user
    /// about it. The warnings are sentences for the status line, not errors: a settings file
    /// with a typo in it still starts an editor, with the chord that typo was meant to move
    /// left where it was.
    pub fn build(keys: &BTreeMap<String, String>, lang: Lang) -> (Keymap, Vec<String>) {
        let mut map = Keymap::default();
        let mut warnings = Vec::new();
        for (name, spelling) in keys {
            let Some(action) = Action::from_name(name) else {
                warnings.push(i18n::msg_keys_unknown_action(lang, name));
                continue;
            };
            match Chord::parse(spelling) {
                Ok(chord) => map.chords[action.index()] = chord,
                Err(_) => warnings.push(i18n::msg_keys_bad_chord(lang, name, spelling)),
            }
        }
        // Two actions on one chord is not fatal and not silent. The second one never fires —
        // `action_for` answers with the first match in declaration order — so the user is told
        // which of the two they have just switched off, rather than discovering it by pressing.
        let actions: Vec<Action> = Action::all().collect();
        for (later, action) in actions.iter().enumerate() {
            let chord = map.chords[later];
            if let Some(earlier) = actions[..later].iter().position(|a| map.chords[a.index()] == chord) {
                warnings.push(i18n::msg_keys_conflict(
                    lang,
                    &chord.display(),
                    actions[earlier].name(),
                    action.name(),
                ));
            }
        }
        map.relabels = map.build_relabels();
        (map, warnings)
    }

    fn build_relabels(&self) -> Vec<(String, String)> {
        let mut pairs: Vec<(String, String)> = table()
            .iter()
            .enumerate()
            .filter(|(index, (_, _, default))| self.chords[*index] != *default)
            .map(|(index, (_, _, default))| (default.display(), self.chords[index].display()))
            .collect();
        // Longest first, so a chord whose spelling starts with another's is matched whole.
        pairs.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        pairs
    }

    /// The chord this action is on right now.
    pub fn chord(&self, action: Action) -> Chord {
        self.chords.get(action.index()).copied().unwrap_or_else(|| action.default_chord())
    }

    /// Which action a key press is, if any. Declaration order decides a tie.
    pub fn action_for(&self, key: KeyEvent) -> Option<Action> {
        table()
            .iter()
            .enumerate()
            .find(|(index, _)| self.chords.get(*index).is_some_and(|chord| chord.matches(key)))
            .map(|(_, (action, _, _))| *action)
    }

    /// Whether a key press is one particular action's chord. For the overlays that close on the
    /// chord that opened them, where the question is about one action rather than all of them.
    pub fn matches(&self, action: Action, key: KeyEvent) -> bool {
        self.chord(action).matches(key)
    }

    /// The same text with every moved chord written the way it is actually bound.
    ///
    /// This is how the manual and the menus tell the truth without being rewritten. They keep
    /// their hand-written prose — which says *why* a key is what it is, and no table can — and
    /// the spellings inside it are corrected on the way to the screen. Nothing is copied while
    /// nothing has been remapped, which is nearly always.
    pub fn relabel(&self, text: &'static str) -> Cow<'static, str> {
        if self.relabels.is_empty() {
            return Cow::Borrowed(text);
        }
        let mut out: Option<String> = None;
        let mut at = 0;
        while at < text.len() {
            let rest = &text[at..];
            match self.relabels.iter().find(|(old, _)| rest.starts_with(old.as_str())) {
                Some((old, new)) => {
                    out.get_or_insert_with(|| text[..at].to_string()).push_str(new);
                    at += old.len();
                }
                None => {
                    let Some(c) = rest.chars().next() else { break };
                    if let Some(buffer) = out.as_mut() {
                        buffer.push(c);
                    }
                    at += c.len_utf8();
                }
            }
        }
        match out {
            Some(text) => Cow::Owned(text),
            None => Cow::Borrowed(text),
        }
    }
}

/// The hint a menu shows for `shortcut`: the chord as it is actually bound, in the words on the
/// reader's keyboard. One funnel, so a remapped chord cannot reach a menu unrewritten.
pub fn shortcut_hint(lang: Lang, keymap: &Keymap, shortcut: &'static str) -> String {
    let bound = keymap.relabel(shortcut);
    i18n::shortcut_label(lang, &bound).to_string()
}

/// The `[keys]` block the Keybindings menu entry writes into settings.toml when there is none:
/// the section header, and every action commented out on the chord it ships with. The defaults
/// are the right thing to write here precisely because this is only ever written where there is
/// no `[keys]` table, which is to say where nothing has been moved yet.
///
/// Generated from the table rather than written by hand, so an action added next year appears
/// here by itself. The entries are comments because none of them is a change: uncomment the one
/// line you want to move, edit the chord, save.
pub fn commented_section(lang: Lang) -> String {
    let mut out = String::new();
    for line in i18n::keys_section_header(lang) {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("[keys]\n");
    // Names padded to a common width so the chords line up in a column, which is how a list of
    // twenty-four of them becomes something you can scan for the one you want.
    let width = table().iter().map(|(_, name, _)| name.len()).max().unwrap_or(0);
    for (_, name, chord) in table() {
        let chord = chord.display();
        out.push_str(&format!("# {name:width$} = \"{chord}\"\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nothing may be unreachable, and nothing may sit on top of something else. Both halves of
    /// this were true by inspection when the table was written and neither stays true by itself.
    #[test]
    fn every_action_has_a_default_and_no_two_share_one() {
        let actions: Vec<Action> = Action::all().collect();
        assert_eq!(actions.len(), table().len());
        for action in &actions {
            assert!(!action.name().is_empty(), "{action:?} has no name in settings.toml");
            assert_eq!(
                Action::from_name(action.name()),
                Some(*action),
                "{action:?} does not answer to its own name"
            );
        }
        let mut seen: Vec<String> = Vec::new();
        for action in &actions {
            let chord = action.default_chord().display();
            assert!(!seen.contains(&chord), "{chord} is the default for two actions, one of them {action:?}");
            seen.push(chord);
        }
    }

    /// The defaults are exactly what CleeCode has always shipped, and this is the file that says
    /// so. Every one of them is `Ctrl+Shift` and a letter or an arrow — no function key, no
    /// `Alt+<letter>`, no bare `Ctrl+<arrow>` — for the reasons written beside the dispatch.
    #[test]
    fn the_defaults_stay_inside_the_projects_own_rules() {
        for action in Action::all() {
            let chord = action.default_chord();
            assert_eq!(
                chord.mods,
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                "{action:?} is not on the Ctrl+Shift layer"
            );
            assert!(
                matches!(
                    chord.code,
                    KeyCode::Char(c) if c.is_ascii_lowercase()
                ) || matches!(
                    chord.code,
                    KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down
                ),
                "{action:?} is on a key this project does not put defaults on"
            );
        }
    }

    #[test]
    fn a_chord_survives_being_written_down_and_read_back() {
        for text in [
            "Ctrl+Shift+M",
            "Ctrl+Alt+←",
            "Alt+Shift+↓",
            "F5",
            "Ctrl+F12",
            "Shift+Enter",
            "Ctrl+Space",
            "Ctrl+Shift+7",
            "Esc",
            "Tab",
        ] {
            let chord = Chord::parse(text).unwrap_or_else(|e| panic!("{text} should parse: {e}"));
            assert_eq!(chord.display(), text, "{text} is not written back the way it was read");
            assert_eq!(Chord::parse(&chord.display()), Ok(chord));
        }
        // Case and spelling are the reader's business, not ours.
        assert_eq!(Chord::parse("ctrl+shift+m"), Chord::parse("Ctrl+Shift+M"));
        assert_eq!(Chord::parse("control+left"), Chord::parse("Ctrl+←"));
        assert_eq!(Chord::parse("option+f"), Chord::parse("Alt+F"));
    }

    #[test]
    fn nonsense_is_refused_rather_than_guessed_at() {
        for text in ["", "ctrl+", "+m", "ctrl+shft+m", "ctrl+m+n", "ctrl+F40", "ctrl+shift"] {
            assert!(Chord::parse(text).is_err(), "{text:?} should not parse");
        }
    }

    fn keys(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn an_override_wins_and_leaves_everything_else_alone() {
        let (map, warnings) = Keymap::build(&keys(&[("find-in-project", "Ctrl+Alt+F")]), Lang::En);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(map.chord(Action::FindInProject).display(), "Ctrl+Alt+F");
        // Untouched actions keep the chord they shipped with.
        assert_eq!(map.chord(Action::Manual), Action::Manual.default_chord());
        let pressed = KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL | KeyModifiers::ALT);
        assert_eq!(map.action_for(pressed), Some(Action::FindInProject));
        // And the chord it left behind stops doing anything.
        let old = KeyEvent::new(KeyCode::Char('H'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        assert_eq!(map.action_for(old), None);
    }

    #[test]
    fn a_name_nobody_recognises_is_a_warning_not_a_failure() {
        let (map, warnings) = Keymap::build(&keys(&[("find_in_project", "Ctrl+Alt+F")]), Lang::En);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("find_in_project"), "{}", warnings[0]);
        assert_eq!(map.chord(Action::FindInProject), Action::FindInProject.default_chord());

        let (map, warnings) = Keymap::build(&keys(&[("find-in-project", "Ctrl+Shft+F")]), Lang::En);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Ctrl+Shft+F"), "{}", warnings[0]);
        assert_eq!(map.chord(Action::FindInProject), Action::FindInProject.default_chord());
    }

    /// Two actions on one chord: a warning, and the one declared first is the one that fires.
    #[test]
    fn a_collision_is_reported_and_the_earlier_action_keeps_the_chord() {
        let (map, warnings) = Keymap::build(&keys(&[("save-all", "Ctrl+Shift+M")]), Lang::En);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Ctrl+Shift+M"), "{}", warnings[0]);
        let pressed = KeyEvent::new(KeyCode::Char('m'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        assert_eq!(map.action_for(pressed), Some(Action::Manual), "the manual is declared first");
    }

    /// The whole point of `relabel`: prose written years ago says the chord in force today.
    #[test]
    fn moved_chords_are_rewritten_in_text_and_untouched_ones_are_not() {
        let (map, _) = Keymap::build(&keys(&[("manual", "F1"), ("git-panel", "Ctrl+Alt+G")]), Lang::En);
        let line = "Ctrl+Shift+M opens this manual, Ctrl+Shift+D the git panel, Ctrl+Shift+R runs.";
        assert_eq!(
            map.relabel(line),
            "F1 opens this manual, Ctrl+Alt+G the git panel, Ctrl+Shift+R runs."
        );
        // Nothing remapped, nothing copied.
        let plain = Keymap::default();
        assert!(matches!(plain.relabel(line), Cow::Borrowed(_)));
    }

    /// Two chords swapped must not chase each other round: a single pass over the text, not one
    /// replacement after another.
    #[test]
    fn swapping_two_chords_does_not_undo_itself() {
        let (map, _) =
            Keymap::build(&keys(&[("manual", "Ctrl+Shift+D"), ("git-panel", "Ctrl+Shift+M")]), Lang::En);
        assert_eq!(map.relabel("Ctrl+Shift+M and Ctrl+Shift+D"), "Ctrl+Shift+D and Ctrl+Shift+M");
    }

    /// The block the menu entry seeds: one commented line per action, generated rather than
    /// written, and legal TOML once the reader uncomments any of it.
    #[test]
    fn the_seeded_block_offers_every_action_and_parses() {
        let block = commented_section(Lang::En);
        for action in Action::all() {
            assert!(
                block.contains(&format!("# {}", action.name())),
                "{} is missing from the seeded block",
                action.name()
            );
            assert!(block.contains(&action.default_chord().display()));
        }
        assert!(block.contains("[keys]"));
        // As written it is an empty table, and uncommenting a line has to keep it valid.
        let parsed: toml::Table = toml::from_str(&block).expect("the seeded block is valid TOML");
        assert!(parsed.get("keys").is_some());
        let live = block.replace("# manual", "manual");
        let parsed: toml::Table = toml::from_str(&live).expect("an uncommented line is valid TOML");
        assert_eq!(parsed["keys"]["manual"].as_str(), Some("Ctrl+Shift+M"));
        // And it reaches `Settings` as the table the keymap is built from, rather than as a
        // field a partial file happens not to mention.
        let settings: crate::settings::Settings = toml::from_str(&live).expect("Settings reads it");
        assert_eq!(settings.keys.get("manual").map(String::as_str), Some("Ctrl+Shift+M"));
    }
}
