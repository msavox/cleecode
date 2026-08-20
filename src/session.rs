//! A live interpreter sitting at its own prompt in one of the terminals.
//!
//! Octave and Python are one feature here, not two. Everything the rest of the program wants —
//! which language a buffer is, whether an interpreter for it is already open, what to type at
//! that prompt to run a file, how to quote a path for it — differs by a line or two between them
//! and by nothing else. Built as two parallel implementations that might converge later, it
//! becomes two half-features that never do, so the seam goes in before the second language does.
//!
//! The Octave half of this was already in `dnd.rs`, hard-wired, for the Run button: handing a
//! `.m` file to an Octave that is already open rather than starting a second one. That is the
//! same question this asks, so it moved here and grew a second answer.

use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Language {
    Octave,
    Python,
}

impl Language {
    /// Which language a file is written in, or `None` for one we have no session for.
    pub fn of_path(path: &Path) -> Option<Language> {
        match path.extension()?.to_str()?.to_lowercase().as_str() {
            "m" => Some(Language::Octave),
            "py" | "pyw" => Some(Language::Python),
            _ => None,
        }
    }

    /// Program names that mean "an interpreter of this language is at its own prompt in here".
    ///
    /// `octave` is a launcher — on macOS it execs `octave-gui` even for a terminal session, and
    /// Windows ships `octave-cli.exe` — so every variant has to be recognised. Python's `python`
    /// and `python3` are both live, and `ipython` is here because it is a Python prompt even
    /// though what it wants said to it differs; that difference is [`Self::run_file`]'s problem.
    pub fn programs(self) -> &'static [&'static str] {
        match self {
            Language::Octave => &["octave", "octave-cli", "octave-gui", "octave-launch"],
            Language::Python => &["python", "python3", "ipython", "ipython3"],
        }
    }

    /// Whether a program name — a bare word from a run-command template, or a full path to the
    /// executable — is an interpreter for this language.
    pub fn is_interpreter(self, program: &str) -> bool {
        let base = program.rsplit(['/', '\\']).next().unwrap_or(program);
        let stem = base.strip_suffix(".exe").unwrap_or(base);
        if self.programs().contains(&stem) {
            return true;
        }
        // `python3.13` is a python, and so is `python3.13t`. A version on the end is not part of
        // the name anywhere the name is written down, so it comes off before asking again.
        let bare = stem.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.' || c == 't');
        !bare.is_empty() && self.programs().contains(&bare)
    }

    /// What to type at this language's prompt to run a file.
    ///
    /// A file, never the code itself. Pasting a multi-line block at a Python prompt is how the
    /// REPL ends up seeing an indented line with no header and answering `IndentationError`, and
    /// the same paste at an Octave prompt gets echoed line by line into the user's transcript.
    /// A temp file has neither problem, and it is one line at the prompt whatever it contains.
    pub fn run_file(self, path: &str) -> String {
        match self {
            Language::Octave => format!("run({})", self.quote(path)),
            // `exec` rather than `runpy` or an import: it runs in the prompt's own namespace, so
            // what the snippet defines is there afterwards — which is the entire point of
            // sending it to a live session instead of starting a new one.
            Language::Python => format!("exec(open({}).read())", self.quote(path)),
        }
    }

    /// A string literal holding `text`, for this language.
    pub fn quote(self, text: &str) -> String {
        match self {
            // Octave's single-quoted strings have exactly one special character, and it is
            // escaped by doubling.
            Language::Octave => format!("'{}'", text.replace('\'', "''")),
            // Python's are the usual thing, and a backslash in a Windows path makes this matter.
            Language::Python => {
                format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
            }
        }
    }

    /// The extension a scratch file for this language needs. Octave will not `run` a file that is
    /// not `.m`, and Python's tracebacks read better when the name looks like a module.
    pub fn scratch_extension(self) -> &'static str {
        match self {
            Language::Octave => "m",
            Language::Python => "py",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Language::Octave => "Octave",
            Language::Python => "Python",
        }
    }
}

/// A move around a figure.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Nav {
    In,
    Out,
    Left,
    Right,
    Up,
    Down,
    Reset,
}

impl Language {
    /// What to say at the prompt to move around figure `number`.
    ///
    /// The whole point is that this goes *back into the interpreter* rather than being done to
    /// the picture. Magnifying the pixels would leave the axis labels describing a range that is
    /// no longer on screen — the plot would say 0 to 100 while showing 25 to 75 — and no amount
    /// of sharpening fixes a number that is wrong. Re-drawing costs 37ms, measured, so the
    /// honest answer is also the affordable one.
    ///
    /// `view` is the current 3-D angle, which the figure's geometry sidecar already carries;
    /// rotating means naming the new angle, and there is no relative form to name it with.
    pub fn nav_command(self, nav: Nav, number: i64, is3d: bool, view: (f64, f64)) -> String {
        match self {
            Language::Octave => {
                let select = format!("figure({number}); ");
                let body = match (nav, is3d) {
                    // `zoom(factor)`, not `zoom on`: the mode wants a real window to click in,
                    // the factor form does not.
                    (Nav::In, _) => "zoom(2);".to_string(),
                    (Nav::Out, _) => "zoom(0.5);".to_string(),
                    (Nav::Reset, false) => "axis auto;".to_string(),
                    (Nav::Reset, true) => "view(-37.5, 30);".to_string(),
                    // A quarter of the span, which is far enough to be worth a keystroke and
                    // near enough that what you were looking at is still on screen.
                    (Nav::Left, false) => "xl = xlim(); xlim(xl - 0.25 * diff(xl));".to_string(),
                    (Nav::Right, false) => "xl = xlim(); xlim(xl + 0.25 * diff(xl));".to_string(),
                    (Nav::Up, false) => "yl = ylim(); ylim(yl + 0.25 * diff(yl));".to_string(),
                    (Nav::Down, false) => "yl = ylim(); ylim(yl - 0.25 * diff(yl));".to_string(),
                    (Nav::Left, true) => format!("view({}, {});", view.0 - 15.0, view.1),
                    (Nav::Right, true) => format!("view({}, {});", view.0 + 15.0, view.1),
                    (Nav::Up, true) => format!("view({}, {});", view.0, (view.1 + 15.0).min(90.0)),
                    (Nav::Down, true) => format!("view({}, {});", view.0, (view.1 - 15.0).max(-90.0)),
                };
                format!("{select}{body}")
            }
            Language::Python => {
                let select = format!(
                    "import matplotlib.pyplot as _plt; _f = _plt.figure({number}); _a = _f.axes[0]; "
                );
                let body = match (nav, is3d) {
                    (Nav::In, _) => "_a.set_xlim(*[c + (l - c) / 2 for c in [sum(_a.get_xlim()) / 2] for l in _a.get_xlim()]); _a.set_ylim(*[c + (l - c) / 2 for c in [sum(_a.get_ylim()) / 2] for l in _a.get_ylim()])".to_string(),
                    (Nav::Out, _) => "_a.set_xlim(*[c + (l - c) * 2 for c in [sum(_a.get_xlim()) / 2] for l in _a.get_xlim()]); _a.set_ylim(*[c + (l - c) * 2 for c in [sum(_a.get_ylim()) / 2] for l in _a.get_ylim()])".to_string(),
                    (Nav::Reset, false) => "_a.autoscale()".to_string(),
                    (Nav::Reset, true) => "_a.view_init(30, -60)".to_string(),
                    (Nav::Left, false) => "_a.set_xlim(*[l - (_a.get_xlim()[1] - _a.get_xlim()[0]) / 4 for l in _a.get_xlim()])".to_string(),
                    (Nav::Right, false) => "_a.set_xlim(*[l + (_a.get_xlim()[1] - _a.get_xlim()[0]) / 4 for l in _a.get_xlim()])".to_string(),
                    (Nav::Up, false) => "_a.set_ylim(*[l + (_a.get_ylim()[1] - _a.get_ylim()[0]) / 4 for l in _a.get_ylim()])".to_string(),
                    (Nav::Down, false) => "_a.set_ylim(*[l - (_a.get_ylim()[1] - _a.get_ylim()[0]) / 4 for l in _a.get_ylim()])".to_string(),
                    (Nav::Left, true) => format!("_a.view_init({}, {})", view.1, view.0 - 15.0),
                    (Nav::Right, true) => format!("_a.view_init({}, {})", view.1, view.0 + 15.0),
                    (Nav::Up, true) => format!("_a.view_init({}, {})", (view.1 + 15.0).min(90.0), view.0),
                    (Nav::Down, true) => format!("_a.view_init({}, {})", (view.1 - 15.0).max(-90.0), view.0),
                };
                // The names are underscore-prefixed for the same reason the startup file's are:
                // they land in the user's own namespace and would otherwise show up as variables
                // in their workspace panel.
                format!("{select}{body}; _f.canvas.draw_idle()")
            }
        }
    }
}

/// Which piece of the file was sent, so the status line can say which without the caller
/// spelling out two nearly identical sentences.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Piece {
    Selection,
    Cell,
}

/// Whether a line begins a `%%` cell.
///
/// One rule for both languages, which is what lets the editor's side of this be written once.
/// The marker itself is shared — `%%` — and only the comment character in front of it differs,
/// so `%% section` in Octave and `# %%` in Python are the same thing said twice. Both are what
/// the surrounding worlds already write, so neither community has to learn a CleeCode
/// convention to use the feature.
pub fn is_cell_marker(line: &str) -> bool {
    let text = line.trim_start();
    // Strip one comment character and any space after it, so `# %%`, `#%%` and a bare `%%` all
    // arrive at the same place.
    let text = match text.strip_prefix('#') {
        Some(rest) => rest.trim_start(),
        None => text,
    };
    text.starts_with("%%")
}

/// The `%%` cell containing `line`, as a half-open range of line numbers.
///
/// A file with no markers is one cell, which is the honest reading: "run this cell" in a script
/// nobody has divided means run the script. The marker line is part of the cell it opens, so its
/// title travels with it and shows up in the transcript — which is how you tell, afterwards,
/// which piece was run.
pub fn cell_at(lines: &[&str], line: usize) -> (usize, usize) {
    if lines.is_empty() {
        return (0, 0);
    }
    let line = line.min(lines.len() - 1);
    let start = (0..=line).rev().find(|&i| is_cell_marker(lines[i])).unwrap_or(0);
    let end = ((line + 1)..lines.len()).find(|&i| is_cell_marker(lines[i])).unwrap_or(lines.len());
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_placed_by_its_extension() {
        assert_eq!(Language::of_path(Path::new("plot.m")), Some(Language::Octave));
        assert_eq!(Language::of_path(Path::new("train.py")), Some(Language::Python));
        assert_eq!(Language::of_path(Path::new("main.rs")), None);
        assert_eq!(Language::of_path(Path::new("Makefile")), None);
    }

    #[test]
    fn an_interpreter_is_recognised_by_any_of_its_names() {
        // macOS execs octave-gui even for a terminal session; Windows ships octave-cli.exe.
        assert!(Language::Octave.is_interpreter("octave"));
        assert!(Language::Octave.is_interpreter("octave-cli"));
        assert!(Language::Octave.is_interpreter("octave-gui"));
        assert!(Language::Octave.is_interpreter("octave-cli.exe"));
        assert!(Language::Octave.is_interpreter("/opt/homebrew/bin/octave"));
        assert!(Language::Octave.is_interpreter(
            r"C:\Program Files\GNU Octave\Octave-10.1.0\mingw64\bin\octave-cli.exe"
        ));
        // A name that merely starts the same way is not a match.
        assert!(!Language::Octave.is_interpreter("octaveish"));
        assert!(Language::Python.is_interpreter("python3"));
        assert!(Language::Python.is_interpreter("/usr/bin/python"));
        assert!(Language::Python.is_interpreter("ipython"));
        // A version on the end is still the same program.
        assert!(Language::Python.is_interpreter("python3.13"));
        assert!(Language::Python.is_interpreter("python3.14t"), "free-threaded builds too");
        // And they do not answer for each other.
        assert!(!Language::Octave.is_interpreter("python3"));
        assert!(!Language::Python.is_interpreter("octave"));
        assert!(!Language::Python.is_interpreter("pythonista"));
    }

    #[test]
    fn each_language_is_told_to_run_a_file_in_its_own_words() {
        assert_eq!(Language::Octave.run_file("/tmp/cell.m"), "run('/tmp/cell.m')");
        assert_eq!(
            Language::Python.run_file("/tmp/cell.py"),
            "exec(open(\"/tmp/cell.py\").read())"
        );
    }

    /// A path with a quote in it is rare and a path with a backslash is not, and either one
    /// unescaped turns a command into a syntax error at somebody's prompt.
    #[test]
    fn a_path_is_quoted_for_the_prompt_it_is_going_to() {
        assert_eq!(Language::Octave.quote("/tmp/it's here/a.m"), "'/tmp/it''s here/a.m'");
        assert_eq!(Language::Python.quote(r"C:\tmp\a.py"), r#""C:\\tmp\\a.py""#);
        assert_eq!(Language::Python.quote(r#"say "hi""#), r#""say \"hi\"""#);
    }

    /// Verified against a live Octave before it was written down: zoom(2) takes [0 100] to
    /// [25 75], the pan forms move the window by a quarter of its span, and `axis auto` puts it
    /// back. What is checked here is that the right one is chosen, not that Octave works.
    #[test]
    fn moving_around_a_figure_is_said_in_the_language_of_the_session() {
        let octave = Language::Octave;
        assert_eq!(octave.nav_command(Nav::In, 1, false, (0.0, 90.0)), "figure(1); zoom(2);");
        assert_eq!(octave.nav_command(Nav::Out, 3, false, (0.0, 90.0)), "figure(3); zoom(0.5);");
        assert!(octave.nav_command(Nav::Right, 1, false, (0.0, 90.0)).contains("xlim(xl + 0.25"));
        assert!(octave.nav_command(Nav::Down, 1, false, (0.0, 90.0)).contains("ylim(yl - 0.25"));
        // Every figure is named before it is acted on, or the command lands on whichever one
        // the session last drew.
        for nav in [Nav::In, Nav::Out, Nav::Left, Nav::Right, Nav::Up, Nav::Down, Nav::Reset] {
            assert!(octave.nav_command(nav, 7, false, (0.0, 90.0)).starts_with("figure(7);"));
        }
    }

    /// On a surface the arrows turn it rather than sliding it, which is what they mean to
    /// anyone who has used the figure window: there is nothing off the edge to pan towards.
    #[test]
    fn arrows_rotate_a_surface_instead_of_panning_it() {
        let octave = Language::Octave;
        assert_eq!(octave.nav_command(Nav::Right, 2, true, (45.0, 30.0)), "figure(2); view(60, 30);");
        assert_eq!(octave.nav_command(Nav::Left, 2, true, (45.0, 30.0)), "figure(2); view(30, 30);");
        // Elevation stops at the poles rather than turning the surface inside out.
        assert!(octave.nav_command(Nav::Up, 2, true, (45.0, 85.0)).contains("view(45, 90)"));
        assert!(octave.nav_command(Nav::Down, 2, true, (45.0, -85.0)).contains("view(45, -90)"));
        assert_eq!(octave.nav_command(Nav::Reset, 2, true, (45.0, 30.0)), "figure(2); view(-37.5, 30);");
    }

    #[test]
    fn python_says_the_same_things_its_own_way() {
        let python = Language::Python;
        let zoom = python.nav_command(Nav::In, 1, false, (0.0, 0.0));
        assert!(zoom.contains("plt.figure(1)") && zoom.contains("set_xlim"));
        // Everything it leaves behind is underscore-prefixed, or it would appear in the user's
        // own workspace panel as a variable they never made.
        for name in ["_plt", "_f", "_a"] {
            assert!(zoom.contains(name), "{zoom}");
        }
        assert!(!zoom.contains(" plt") && !zoom.contains("= f "), "{zoom}");
        assert!(python.nav_command(Nav::Right, 1, true, (45.0, 30.0)).contains("view_init(30, 60)"));
    }

    #[test]
    fn a_cell_marker_is_the_same_mark_behind_either_comment_character() {
        assert!(is_cell_marker("%% load the data"));
        assert!(is_cell_marker("  %% indented"));
        assert!(is_cell_marker("# %% load the data"));
        assert!(is_cell_marker("#%% no space"));
        assert!(!is_cell_marker("% an ordinary Octave comment"));
        assert!(!is_cell_marker("# an ordinary Python comment"));
        assert!(!is_cell_marker("x = 1  %% not at the start"));
        assert!(!is_cell_marker(""));
    }

    #[test]
    fn a_cell_runs_from_its_marker_to_the_next() {
        let lines = vec![
            "%% first",      // 0
            "a = 1;",        // 1
            "b = 2;",        // 2
            "%% second",     // 3
            "c = 3;",        // 4
        ];
        assert_eq!(cell_at(&lines, 0), (0, 3), "on the marker itself");
        assert_eq!(cell_at(&lines, 2), (0, 3), "inside the first cell");
        assert_eq!(cell_at(&lines, 3), (3, 5), "on the second marker");
        assert_eq!(cell_at(&lines, 4), (3, 5), "inside the second cell");
    }

    #[test]
    fn lines_before_the_first_marker_are_their_own_cell() {
        let lines = vec!["setup = 1;", "%% work", "x = 2;"];
        assert_eq!(cell_at(&lines, 0), (0, 1));
        assert_eq!(cell_at(&lines, 2), (1, 3));
    }

    /// A script nobody has divided is one cell, so "run this cell" means run the script rather
    /// than doing nothing and leaving you to work out why.
    #[test]
    fn a_file_with_no_markers_is_one_cell() {
        let lines = vec!["a = 1;", "b = 2;", "c = 3;"];
        assert_eq!(cell_at(&lines, 1), (0, 3));
        assert_eq!(cell_at(&[], 0), (0, 0));
        // A cursor past the end — a buffer that shrank under it — clamps rather than panicking.
        assert_eq!(cell_at(&lines, 99), (0, 3));
    }
}
