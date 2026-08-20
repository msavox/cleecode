//! The workspace viewer: `clee --watch-workspace <dir>`.
//!
//! It runs in one of CleeCode's own terminal windows, which is what makes it a *window* and not
//! a panel — it stays on screen beside the prompt instead of being something you open and
//! dismiss. That was the point: you watch values change as you run cells, rather than asking.
//!
//! CleeCode itself rather than a script, so there is nothing to install and the table is drawn
//! by us. It reads the snapshot the interpreter writes — see `wsnap.rs` — and redraws when it
//! changes.
//!
//! No raw mode and no alternate screen. Ctrl+C ends it the way Ctrl+C ends anything, the pane
//! can be resized without the viewer having to care, and nothing has hijacked the terminal if it
//! dies. It is a program printing a table, and that is the whole of it.

use crate::wsnap::{ordered, Snapshot, Watch};
use std::io::Write;
use std::path::Path;

/// How often the directory is looked at. The interpreter publishes once per command, so this is
/// about how soon a change is noticed, not how often anything is read: at four times a second a
/// value appears as fast as a person can look up from the prompt.
const TICK: std::time::Duration = std::time::Duration::from_millis(250);

pub fn watch(dir: &Path) -> std::io::Result<()> {
    let mut current: Option<Watch> = None;
    let mut shown: Option<(std::path::PathBuf, u64)> = None;
    let mut last_size = (0u16, 0u16);
    loop {
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        // A resized pane has to redraw even when nothing new arrived, or the table stays cut to
        // the width it had when it was written.
        let resized = (cols, rows) != last_size;
        last_size = (cols, rows);

        // Whichever session last ran something. Following it rather than being wired to one
        // pane is what lets a single window serve two prompts.
        if let Some(path) = crate::wsnap::newest_in(dir)
            && current.as_ref().map(|w| &w.path) != Some(&path)
        {
            current = Some(Watch::new(path));
        }
        let changed = current.as_mut().map(|w| w.poll()).unwrap_or(false);

        let state = current.as_ref().and_then(|w| w.snapshot.as_ref()).map(|s| (
            current.as_ref().map(|w| w.path.clone()).unwrap_or_default(),
            s.seq,
        ));
        if changed || resized || state != shown {
            shown = state;
            let snapshot = current.as_ref().and_then(|w| w.snapshot.as_ref());
            let mut out = std::io::stdout();
            // Home and clear-to-end rather than a full clear: the pane does not flash, and what
            // was there is overwritten row by row as the new table is written over it.
            write!(out, "\x1b[H\x1b[J")?;
            for line in render(snapshot, cols, rows) {
                writeln!(out, "{line}\r")?;
            }
            out.flush()?;
        }
        std::thread::sleep(TICK);
    }
}

/// The table, as coloured lines. Pure, so what it decides at a given width is testable without
/// a terminal to look at.
pub fn render(snapshot: Option<&Snapshot>, cols: u16, rows: u16) -> Vec<String> {
    // A pane with no width can show nothing, and everything below assumes there is room for at
    // least one character — the text clipper answers "…" when asked for zero, which is one column
    // more than there is.
    if cols == 0 || rows == 0 {
        return Vec::new();
    }
    let dim = "\x1b[90m";
    let off = "\x1b[0m";
    let head = "\x1b[1m";
    let Some(snapshot) = snapshot else {
        return vec![
            format!("{dim}Waiting for a session…{off}"),
            String::new(),
            format!("{dim}Start an interpreter in one of the terminals and this fills in.{off}"),
            format!("{dim}Nothing is typed at your prompt to ask it.{off}"),
        ];
    };
    // A file written by a newer CleeCode is said to be one rather than half-read into a table
    // that looks right and is not. Nothing about this format is guessable from a version we do
    // not know, so guessing is the one thing not to do.
    if snapshot.v != 0 && snapshot.v != 1 {
        return vec![
            format!("{dim}This session is writing version {} of the workspace format.{off}", snapshot.v),
            format!("{dim}This CleeCode reads version 1. Update it, or the two will disagree{off}"),
            format!("{dim}quietly rather than loudly.{off}"),
        ];
    }
    if snapshot.vars.is_empty() {
        return vec![
            title(snapshot, cols, dim, off),
            String::new(),
            format!("{dim}The workspace is empty.{off}"),
        ];
    }

    let layout = Layout::for_width(cols, &snapshot.vars);
    let mut lines = vec![title(snapshot, cols, dim, off), String::new()];
    lines.extend(stack(snapshot, cols, off));
    lines.push(format!("{head}{}{off}", layout.header()));
    lines.push(format!("{dim}{}{off}", "─".repeat((cols as usize).min(layout.width()))));
    for var in ordered(&snapshot.vars) {
        lines.push(layout.row(var));
    }
    lines.extend(history(snapshot, cols, rows, lines.len(), dim, off, head));
    let lines = fit_to_pane(lines, cols, rows, dim, off);
    lines.iter().map(|line| cut_to(line, cols)).collect()
}

/// Never hand back more lines than the pane has rows, and say what was left out.
///
/// The window writes what it is given, line by line, so a table longer than the pane used to
/// scroll it: the title and the header went off the top, and the next tick's cursor-home wrote
/// the new table over a buffer that had already moved. A session with more variables than the
/// pane is tall is completely ordinary, so this was not an edge case.
///
/// Cutting silently would be its own bug — a panel that shows nine of your twelve variables and
/// looks complete is worse than one that shows eight and says so. The last line says how many
/// are not shown, which is also the sentence that tells you to make the pane taller.
fn fit_to_pane(mut lines: Vec<String>, cols: u16, rows: u16, dim: &str, off: &str) -> Vec<String> {
    let rows = rows as usize;
    if lines.len() <= rows {
        return lines;
    }
    if rows == 0 {
        return Vec::new();
    }
    let hidden = lines.len() - rows + 1;        // +1: the note takes a row of its own
    lines.truncate(rows);
    if let Some(last) = lines.last_mut() {
        // The advice is worth a line only if the line has room for it. Cut mid-word — "make
        // this pa" — it reads like the bug it is there to prevent, so the count goes alone.
        let full = format!("… {hidden} more — make this pane taller");
        let note = if full.chars().count() <= cols as usize {
            full
        } else {
            format!("… {hidden} more")
        };
        *last = format!("{dim}{note}{off}");
    }
    lines
}

/// Cut a coloured line to the pane's width, counting only what is visible.
///
/// The table already drops whole columns rather than squeezing them, but at the narrow end it
/// still writes a header wider than the pane — measured at one column: eleven characters. A line
/// too wide does not simply get cut off, it *wraps*, and one wrapped row pushes everything below
/// it down and out. So this is the guarantee, made once for every line rather than trusted to
/// each piece that builds one.
///
/// Escape sequences pass through without counting, and the reset is put back on the end: cutting
/// a line in the middle of "\x1b[90m" would leave the rest of the pane painted by an escape that
/// never finished.
fn cut_to(line: &str, cols: u16) -> String {
    let mut out = String::new();
    let mut seen = 0usize;
    let mut chars = line.chars().peekable();
    let mut coloured = false;
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            out.push(c);
            for c in chars.by_ref() {
                out.push(c);
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
            coloured = true;
            continue;
        }
        if seen == cols as usize {
            break;
        }
        out.push(c);
        seen += 1;
    }
    if coloured && !out.ends_with("\x1b[0m") {
        out.push_str("\x1b[0m");
    }
    out
}

/// Where the session is stopped, and how it got there.
///
/// Above the variables rather than below, because while stopped the variables *are* the frame's
/// — the two are one thing, and reading them in the wrong order would mean reading the frame's
/// locals as though they were the workspace.
fn stack(snapshot: &Snapshot, cols: u16, off: &str) -> Vec<String> {
    if !snapshot.debug.stopped {
        return Vec::new();
    }
    let mark = "\x1b[33m";      // the same yellow the editor marks the stopped line with
    let mut out = vec![format!(
        "{mark}stopped in {} at line {}{off}",
        snapshot.debug.name, snapshot.debug.line
    )];
    // The frames under it, oldest last, so the shape reads the way a backtrace does.
    for frame in snapshot.debug.stack.iter().skip(1) {
        out.push(format!(
            "\x1b[90m  called from {} at line {}{off}",
            clip(&frame.name, cols as usize / 2),
            frame.line
        ));
    }
    out.push(String::new());
    out
}

/// The last few commands, under the variables, in whatever rows are left.
///
/// Free to produce — the Octave hook already reads the history to decide whether anything
/// happened — and it goes here rather than in a window of its own because it answers the
/// question the variables raise: *what did I do to get this*. Dropped entirely when the pane is
/// short, since a table of variables cut in half to make room for it would be a poor trade.
fn history(
    snapshot: &Snapshot,
    cols: u16,
    rows: u16,
    used: usize,
    dim: &str,
    off: &str,
    head: &str,
) -> Vec<String> {
    let room = (rows as usize).saturating_sub(used + 3);
    if snapshot.history.is_empty() || room < 2 {
        return Vec::new();
    }
    let mut out = vec![String::new(), format!("{head}Recent{off}")];
    for command in snapshot.history.iter().rev().take(room).collect::<Vec<_>>().into_iter().rev() {
        out.push(format!("{dim}{}{off}", clip(command, cols as usize)));
    }
    out
}

fn title(snapshot: &Snapshot, cols: u16, dim: &str, off: &str) -> String {
    let language = match snapshot.lang.as_str() {
        "" => "workspace",
        other => other,
    };
    let count = snapshot.vars.len();
    // The directory's last part, so two prompts in two projects are tellable apart without the
    // title eating the row. Dropped entirely when there is no room for it.
    let where_ = Path::new(&snapshot.cwd)
        .file_name()
        .map(|n| format!(" · {}", n.to_string_lossy()))
        .unwrap_or_default();
    let text = format!(
        "{language}{where_} · {count} variable{} · {}",
        if count == 1 { "" } else { "s" },
        human_bytes(snapshot.bytes())
    );
    let short = format!("{language} · {count} · {}", human_bytes(snapshot.bytes()));
    let text = if text.chars().count() <= cols as usize { text } else { short };
    format!("{dim}{}{off}", clip(&text, cols as usize))
}

/// Which columns fit, and how wide each is.
///
/// The pane holding this is usually narrow — it is a window beside the editor, not the whole
/// screen — so columns are dropped rather than squeezed: three readable ones beat six unreadable
/// ones. Name and shape always survive, because a variable you cannot identify and cannot size
/// is not worth a row.
struct Layout {
    name: usize,
    class: usize,
    shape: usize,
    stats: bool,
    preview: usize,
}

impl Layout {
    fn for_width(cols: u16, vars: &[crate::wsnap::Var]) -> Layout {
        let cols = cols.max(12) as usize;
        let widest = |f: fn(&crate::wsnap::Var) -> usize, least: usize, most: usize| {
            vars.iter().map(f).max().unwrap_or(least).clamp(least, most)
        };
        let mut name = widest(|v| v.name.chars().count(), 4, 18);
        let mut shape = widest(|v| v.shape().chars().count(), 4, 12);
        // The two that always survive still have to fit between them, or a long name pushes the
        // shape off the end and the terminal wraps one variable onto two lines — which is the
        // one thing that stops a table being a table.
        while name + shape + 2 > cols && name > 3 {
            if name >= shape {
                name -= 1;
            } else {
                shape -= 1;
            }
        }
        let shape = shape.min(cols.saturating_sub(name + 2));

        // Then class, then the three statistics, then whatever is left goes to the preview —
        // each only if the one before it fitted.
        let mut used = name + 1 + shape + 1;
        let class = widest(|v| v.class.chars().count(), 5, 14);
        let class = if used + class < cols { used += class + 1; class } else { 0 };
        let stats = used + 3 * 11 <= cols;
        if stats {
            used += 3 * 11;
        }
        let preview = cols.saturating_sub(used);
        Layout { name, class, shape, stats, preview: if preview >= 6 { preview } else { 0 } }
    }

    fn width(&self) -> usize {
        self.name + 1 + self.shape + 1
            + if self.class > 0 { self.class + 1 } else { 0 }
            + if self.stats { 33 } else { 0 }
            + self.preview
    }

    fn header(&self) -> String {
        let mut out = format!("{:<w$} {:<s$} ", "Name", "Size", w = self.name, s = self.shape);
        if self.class > 0 {
            out.push_str(&format!("{:<c$} ", "Class", c = self.class));
        }
        if self.stats {
            out.push_str(&format!("{:>10} {:>10} {:>10} ", "Min", "Max", "Mean"));
        }
        if self.preview > 0 {
            out.push_str("Value");
        }
        out
    }

    fn row(&self, var: &crate::wsnap::Var) -> String {
        let name = clip(&var.name, self.name);
        let mut out = format!("\x1b[36m{name:<w$}\x1b[0m {:<s$} ", clip(&var.shape(), self.shape),
                              w = self.name, s = self.shape);
        if self.class > 0 {
            out.push_str(&format!("\x1b[90m{:<c$}\x1b[0m ", clip(&var.class, self.class), c = self.class));
        }
        if self.stats {
            out.push_str(&format!("{:>10} {:>10} {:>10} ", num(var.min), num(var.max), num(var.mean)));
        }
        if self.preview > 0 {
            let mut preview = clip(&var.preview, self.preview);
            // A count of NaNs is worth more than the tail of a preview: it is the thing that
            // silently turns a mean into nothing.
            if var.nans > 0 {
                preview = clip(&format!("{} NaN, {preview}", var.nans), self.preview);
            }
            out.push_str(&format!("\x1b[90m{preview}\x1b[0m"));
        }
        out
    }
}

/// One number in a grid: whole numbers plain, the rest to four places, and a NaN said as NaN
/// rather than shown as a blank that reads like a missing cell.
pub fn cell_number(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Inf".to_string() } else { "-Inf".to_string() };
    }
    if value == value.trunc() && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    format!("{value:.4}")
}

/// A number as a person reads it, or a dash where the interpreter said there is none — a char
/// array has no minimum, and neither has an array too large to have been scanned.
fn num(value: Option<f64>) -> String {
    match value {
        None => "-".to_string(),
        Some(v) if v == v.trunc() && v.abs() < 1e15 => format!("{}", v as i64),
        Some(v) => format!("{v:.4}"),
    }
}

fn human_bytes(bytes: i64) -> String {
    const UNITS: [&str; 4] = ["B", "kB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{value:.1} {}", UNITS[unit]) }
}

fn clip(text: &str, width: usize) -> String {
    let flat: String = text.chars().map(|c| if c.is_whitespace() { ' ' } else { c }).collect();
    if flat.chars().count() <= width {
        return flat;
    }
    flat.chars().take(width.saturating_sub(1)).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    /// Every size a terminal can be, including the ones it should not.
    ///
    /// This is where CleeCode has actually crashed before — a terminal opened at zero height, a
    /// split editor in a very narrow window — and the workspace view is the newest thing drawing
    /// into a pane whose size it does not choose. A user dragging a seam passes through every
    /// width on the way, one frame each.
    #[test]
    fn no_size_a_pane_can_be_brings_it_down() {
        let snapshot = sample();
        for cols in 0u16..=200 {
            for rows in [0u16, 1, 2, 3, 4, 5, 9, 40, 200] {
                let lines = render(Some(&snapshot), cols, rows);
                assert!(lines.len() <= rows as usize, "{cols}x{rows} drew more rows than it has");
                for line in plain(&lines) {
                    assert!(
                        line.chars().count() <= cols as usize,
                        "{cols}x{rows} drew a line of {} columns: {line:?}",
                        line.chars().count()
                    );
                }
            }
        }
        // And with nothing to show, which is what the first second of every session looks like.
        for cols in 0u16..=200 {
            let _ = render(None, cols, 24);
        }
    }

    /// The snapshot is written by somebody else's interpreter, so it is input, not data.
    ///
    /// A half-written file is the ordinary case rather than the hostile one: the writer renames
    /// its temporary file into place, but an older prototype did not, and a user can point
    /// CLEECODE_OCTAVE_WS at anything. None of it may do worse than show no workspace.
    #[test]
    fn a_snapshot_that_is_not_one_is_refused_rather_than_believed() {
        let whole = r#"{"v":1,"seq":12,"lang":"octave","cwd":"/proj","vars":[
            {"name":"A","class":"double","size":[6,6],"bytes":288,"min":1,"max":36,"mean":18.5},
            {"name":"testo","class":"char","size":[1,10],"bytes":10}],
            "history":["A = magic(6)","testo = 'ciao'"],
            "debug":{"stopped":true,"name":"calcola","file":"/proj/calcola.m","line":3,
                     "stack":[{"name":"calcola","line":3}]}}"#;
        assert!(Snapshot::parse(whole).is_some(), "the whole thing has to parse first");
        for cut in 0..whole.len() {
            if let Some(s) = Snapshot::parse(&whole[..cut]) {
                let _ = render(Some(&s), 80, 24);          // truncated but parsed: still drawable
            }
        }
        for junk in ["", " ", "{", "[]", "null", "{\"v\":1}", "\u{0}\u{1}\u{2}",
                     "{\"vars\":[{\"name\":\"a\",\"size\":[-1,-1]}]}",
                     "{\"vars\":[{\"name\":\"a\",\"size\":[99999999999,2]}]}"] {
            if let Some(s) = Snapshot::parse(junk) {
                let _ = render(Some(&s), 80, 24);
            }
        }
    }

    use super::*;
    use crate::wsnap::Var;

    fn var(name: &str, class: &str, size: &[i64], bytes: i64) -> Var {
        Var {
            name: name.to_string(),
            class: class.to_string(),
            size: size.to_vec(),
            bytes: Some(bytes),
            min: Some(1.0),
            max: Some(999.0),
            mean: Some(42.5),
            preview: "[1 2 3]".to_string(),
            ..Var::default()
        }
    }

    fn sample() -> Snapshot {
        Snapshot {
            v: 1,
            seq: 3,
            lang: "octave".to_string(),
            vars: vec![var("gamma", "double", &[1, 10], 80), var("alpha", "char", &[1, 4], 4)],
            ..Snapshot::default()
        }
    }

    fn plain(lines: &[String]) -> Vec<String> {
        // Strips the colour, so a test is about what is written rather than how it is painted.
        let escape = regexless_strip;
        lines.iter().map(|l| escape(l)).collect()
    }

    fn regexless_strip(text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for c in chars.by_ref() {
                    if c == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn with_no_session_it_says_so_rather_than_showing_an_empty_table() {
        let lines = plain(&render(None, 80, 24));
        assert!(lines[0].contains("Waiting"), "{lines:?}");
        // And says the thing that is least obvious about it: nothing is typed at the prompt.
        assert!(lines.iter().any(|l| l.contains("Nothing is typed")), "{lines:?}");
    }

    #[test]
    fn an_empty_workspace_is_not_the_same_as_no_session() {
        let snap = Snapshot { lang: "octave".to_string(), ..Snapshot::default() };
        let lines = plain(&render(Some(&snap), 80, 24));
        assert!(lines[0].contains("octave"), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("empty")), "{lines:?}");
    }

    #[test]
    fn the_table_lists_the_variables_by_name() {
        let lines = plain(&render(Some(&sample()), 100, 24));
        assert!(lines[0].contains("octave") && lines[0].contains("2 variables"), "{lines:?}");
        let rows: Vec<&String> = lines.iter().filter(|l| l.starts_with("alpha") || l.starts_with("gamma")).collect();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].starts_with("alpha"), "sorted by name: {rows:?}");
        assert!(rows[0].contains("1x4") && rows[0].contains("char"));
    }

    /// The pane holding this is a window beside the editor, not the whole screen, so columns are
    /// dropped rather than squeezed — three readable ones beat six unreadable ones.
    #[test]
    fn narrow_panes_drop_columns_instead_of_squeezing_them() {
        let wide = plain(&render(Some(&sample()), 120, 24));
        assert!(wide[2].contains("Min") && wide[2].contains("Value"), "{:?}", wide[2]);

        let middling = plain(&render(Some(&sample()), 46, 24));
        assert!(middling[2].contains("Class"), "{:?}", middling[2]);
        assert!(!middling[2].contains("Min"), "the statistics go first: {:?}", middling[2]);

        let narrow = plain(&render(Some(&sample()), 16, 24));
        assert!(narrow[2].contains("Name") && narrow[2].contains("Size"), "{:?}", narrow[2]);
        assert!(!narrow[2].contains("Class"), "{:?}", narrow[2]);

        // Whatever survives at a given width still survives at every wider one: columns come
        // back in the order they went, never in a different one.
        let header = |cols| plain(&render(Some(&sample()), cols, 24))[2].clone();
        for (narrower, wider) in [(16u16, 46u16), (46, 60), (60, 120)] {
            for column in ["Name", "Size", "Class", "Min", "Value"] {
                if header(narrower).contains(column) {
                    assert!(header(wider).contains(column), "{column} vanished at {wider}");
                }
            }
        }
    }

    /// No row may be wider than the pane, or the terminal wraps it and one variable becomes two
    /// lines — which is exactly what makes a table stop being readable.
    #[test]
    fn no_row_is_wider_than_the_pane() {
        let mut snap = sample();
        snap.vars.push(var("a_really_long_variable_name_here", "containers.Map", &[1000, 2000], 16_000_000));
        snap.vars[2].preview = "a very long preview that would run off the end of any pane".to_string();
        for cols in [20u16, 30, 46, 60, 100, 200] {
            for line in plain(&render(Some(&snap), cols, 24)) {
                assert!(
                    line.chars().count() <= cols as usize,
                    "{} chars at {cols} columns: {line:?}",
                    line.chars().count()
                );
            }
        }
    }

    /// Free to produce and worth showing: the variables say what you have, and this says what
    /// you did to get it.
    /// While stopped the variables are the frame's own, so where we are stopped has to be read
    /// before them — otherwise a frame's locals read as though they were the workspace.
    #[test]
    fn being_stopped_is_said_above_the_variables() {
        let mut snap = sample();
        snap.debug = crate::wsnap::Debug {
            stopped: true,
            name: "calcola".into(),
            file: "/proj/calcola.m".into(),
            line: 3,
            stack: vec![
                crate::wsnap::Frame { name: "calcola".into(), line: 3 },
                crate::wsnap::Frame { name: "principale".into(), line: 12 },
            ],
        };
        let lines = plain(&render(Some(&snap), 90, 24));
        let stopped = lines.iter().position(|l| l.contains("stopped in calcola at line 3")).unwrap();
        let table = lines.iter().position(|l| l.starts_with("Name")).unwrap();
        assert!(stopped < table, "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("called from principale at line 12")), "{lines:?}");
    }

    #[test]
    fn a_session_that_is_running_says_nothing_about_frames() {
        let lines = plain(&render(Some(&sample()), 90, 24));
        assert!(!lines.iter().any(|l| l.contains("stopped in")), "{lines:?}");
    }

    #[test]
    fn recent_commands_go_under_the_variables_when_there_is_room() {
        let mut snap = sample();
        snap.history = vec!["a = 1;".into(), "b = magic(4);".into()];
        let tall = plain(&render(Some(&snap), 80, 24));
        assert!(tall.iter().any(|l| l == "Recent"), "{tall:?}");
        // Newest last, the way a transcript reads.
        let first = tall.iter().position(|l| l.contains("a = 1;")).unwrap();
        let then = tall.iter().position(|l| l.contains("magic")).unwrap();
        assert!(first < then, "{tall:?}");

        // A short pane keeps the variables whole rather than halving them to make room.
        let short = plain(&render(Some(&snap), 80, 8));
        assert!(!short.iter().any(|l| l == "Recent"), "{short:?}");
        assert!(short.iter().any(|l| l.starts_with("alpha")), "the table survives: {short:?}");
    }

    #[test]
    fn a_session_with_no_history_shows_no_heading_for_it() {
        let tall = plain(&render(Some(&sample()), 80, 24));
        assert!(!tall.iter().any(|l| l == "Recent"), "{tall:?}");
    }

    #[test]
    fn a_statistic_the_interpreter_did_not_give_shows_as_a_dash() {
        assert_eq!(num(None), "-");
        assert_eq!(num(Some(7.0)), "7");
        assert_eq!(num(Some(0.5)), "0.5000");
    }

    /// A NaN count is worth more than the tail of a preview: it is the thing that silently turns
    /// a mean into nothing.
    #[test]
    fn nans_are_reported_where_they_can_be_seen() {
        let mut snap = sample();
        snap.vars[0].nans = 3;
        let lines = plain(&render(Some(&snap), 120, 24));
        assert!(lines.iter().any(|l| l.starts_with("gamma") && l.contains("3 NaN")), "{lines:?}");
    }

    #[test]
    fn sizes_read_the_way_people_write_them() {
        assert_eq!(human_bytes(80), "80 B");
        assert_eq!(human_bytes(2048), "2.0 kB");
        assert_eq!(human_bytes(16_000_000), "15.3 MB");
    }
}
