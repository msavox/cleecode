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
    lines.push(format!("{head}{}{off}", layout.header()));
    lines.push(format!("{dim}{}{off}", "─".repeat((cols as usize).min(layout.width()))));
    for var in ordered(&snapshot.vars) {
        lines.push(layout.row(var));
    }
    lines.extend(history(snapshot, cols, rows, lines.len(), dim, off, head));
    lines
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
