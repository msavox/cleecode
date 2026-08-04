mod app;
mod clipboard;
mod dnd;
mod editor;
mod file_tree;
mod find;
mod font_install;
mod git_status;
mod highlight;
mod i18n;
mod manual;
mod menu;
mod picker;
mod settings;
mod terminal_panel;
mod ui;
mod workspace;

use anyhow::Result;
use app::App;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
};
use ratatui::layout::Rect;
use settings::Settings;
use std::io::{stdout, Write};
use std::panic::AssertUnwindSafe;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Where the panic hook leaves what it caught, for the loop to pick up and show. The hook
/// itself can't print: stderr goes straight onto the alternate screen, scribbling over the UI.
static LAST_PANIC: Mutex<Option<String>> = Mutex::new(None);

/// Runs one step of the loop without letting it take the process down. A panic anywhere in
/// here — a terminal's parser, a layout arithmetic slip, a widget — used to close CleeCode
/// outright, killing every shell running inside it: an editing session and a long-running
/// `claude` in a pane, gone, with no way to get them back. Whatever went wrong, the blast
/// radius has to stay smaller than the whole application.
///
/// Returns the panic's description when it caught one, so the caller can decide what to
/// sacrifice (usually the terminal being drawn) and tell the user.
fn shielded<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    match std::panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => Ok(value),
        Err(_) => Err(LAST_PANIC
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .unwrap_or_else(|| "unknown error".to_string())),
    }
}

/// How many panics one session will write down. A bug that fires on every frame would otherwise
/// append to the log thirty times a second for as long as the editor is left open; past the first
/// few the lines are all the same anyway, and the point is to diagnose, not to fill a disk.
const PANIC_LOG_LIMIT: usize = 40;

/// Silences the default handler (which would print over the UI) and records what happened,
/// both for the status line and in a log file, since the on-screen message is one line and
/// disappears with the next action.
fn install_panic_hook() {
    let logged = std::sync::atomic::AtomicUsize::new(0);
    std::panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".to_string());
        let where_ = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let text = if where_.is_empty() { payload } else { format!("{payload} ({where_})") };

        let n = logged.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if n < PANIC_LOG_LIMIT {
            if let Some(dir) = settings::config_dir() {
                let _ = std::fs::create_dir_all(&dir);
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("panic.log"))
                {
                    let _ = writeln!(f, "clee {}: {}", env!("CARGO_PKG_VERSION"), text);
                    if n + 1 == PANIC_LOG_LIMIT {
                        let _ = writeln!(f, "clee {}: (further panics this session not logged)", env!("CARGO_PKG_VERSION"));
                    }
                }
            }
        }
        // Recorded regardless of the log cap: the status line should keep telling the truth even
        // once the file has stopped growing.
        *LAST_PANIC.lock().unwrap_or_else(|e| e.into_inner()) = Some(text);
    }));
}

/// One-line usage text, printed for `--help` and by `--version`'s sibling flags. Kept
/// hand-rolled (rather than pulling in an argument parser) because the whole surface is one
/// optional path plus two flags.
const USAGE: &str = "\
clee — CleeCode, a terminal IDE

USAGE:
    clee [FILE|DIRECTORY]
    clee -w NAME

    A directory becomes the project root; a file is opened in the current directory.
    With no argument, the last project folder and its open files are restored.

OPTIONS:
    -w, --workspace NAME
                      Open a saved workspace: its project root, files, frame sizes and
                      terminals, shells and startup commands included. `clee -w` with no
                      name lists the ones you have.
    --install-font    Install the bundled CleeCodeMono Nerd Font for the current user
    -h, --help        Print this help
    -V, --version     Print the version
";

fn main() -> Result<()> {
    // Flags are checked before the terminal is taken over, so their output stays visible.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return Ok(());
    }
    if args.iter().any(|a| a == "-V" || a == "--version") {
        println!("clee {} (cleecode)", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|a| a == "--install-font") {
        font_install::install();
        return Ok(());
    }
    let mut open_workspace: Option<String> = None;
    // `clee -w` with no name is a question, not a mistake: list what there is and stop, while
    // the output can still be seen.
    if let Some(i) = args.iter().position(|a| a == "-w" || a == "--workspace") {
        match args.get(i + 1) {
            Some(name) => open_workspace = Some(name.clone()),
            None => {
                let saved = workspace::list();
                if saved.is_empty() {
                    println!("No saved workspaces of your own yet — save one from the Workspace menu.");
                    println!("    {:<24} (built in)", workspace::DEFAULT_NAME);
                } else {
                    println!("Saved workspaces:");
                    for ws in saved {
                        println!("    {:<24} {}", ws.name, ws.root.display());
                    }
                    println!("    {:<24} (built in)", workspace::DEFAULT_NAME);
                }
                return Ok(());
            }
        }
    }
    install_panic_hook();
    let mut terminal = ratatui::init();
    crossterm::execute!(stdout(), EnableMouseCapture, EnableBracketedPaste)?;
    // Ask for disambiguated key reporting where the terminal offers it. Without it Ctrl+Tab
    // arrives as a plain Tab — the two are the same byte, 0x09, in the encoding terminals have
    // used since VT100 — so cycling frames from the keyboard would be impossible to tell from
    // indenting. Ghostty, kitty, WezTerm and foot support this; Terminal.app does not, which is
    // why Alt+1/2/3 reach the frames directly and work everywhere.
    let enhanced = matches!(crossterm::terminal::supports_keyboard_enhancement(), Ok(true));
    if enhanced {
        let _ = crossterm::execute!(
            stdout(),
            crossterm::event::PushKeyboardEnhancementFlags(
                crossterm::event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            )
        );
    }
    // Push (save) the terminal's current title, then set our own; tmux/the shell would
    // otherwise leave their own title showing in the window chrome for the whole session.
    let _ = write!(stdout(), "\x1b[22;2t\x1b]0;CleeCode\x07");
    let _ = stdout().flush();
    // Even a panic the loop couldn't shield must not skip the teardown below: leaving the
    // terminal in raw mode on the alternate screen hands the user an unusable shell.
    let result = shielded(|| run(&mut terminal, open_workspace));
    let _ = write!(stdout(), "\x1b[23;2t");
    // Popped before anything else is undone: leaving the flags pushed would hand the shell back
    // a terminal that reports keys in a mode it never asked for.
    if enhanced {
        let _ = crossterm::execute!(stdout(), crossterm::event::PopKeyboardEnhancementFlags);
    }
    let _ = crossterm::execute!(stdout(), DisableBracketedPaste, DisableMouseCapture);
    ratatui::restore();
    match result {
        Ok(r) => r,
        Err(text) => {
            eprintln!("clee closed after an internal error: {text}");
            Ok(())
        }
    }
}

fn run(terminal: &mut ratatui::DefaultTerminal, open_workspace: Option<String>) -> Result<()> {
    let size = terminal.size()?;
    let cwd = std::env::current_dir()?;
    let arg = std::env::args().nth(1).map(std::path::PathBuf::from);
    let arg_is_dir = arg.as_ref().map(|p| p.is_dir()).unwrap_or(false);

    // Resume the last workspace (project folder + open tabs) when launched with no
    // explicit argument; an argument always takes precedence and starts fresh. A directory
    // argument becomes the project root; a file argument is opened within the current dir.
    let saved = Settings::load();
    // A saved workspace remembers more than the plain resume does — terminal names, startup
    // commands, frame sizes — so it wins when there is one. Its root is picked up here, before
    // the first shells are spawned, so they start in the right directory.
    // `-w name` is an explicit request, so it beats both the resume and a path argument. Falling
    // back to the resume when the name is unknown would silently open the wrong thing, so a bad
    // name simply opens nothing and says so once the UI is up.
    let named = open_workspace.as_deref().map(|n| {
        // The built-in one is not a file, so it is answered here rather than looked up. Its root
        // is settled below like any other startup: the argument, or the current directory.
        let found = if workspace::is_default(n) { None } else { workspace::load(n) };
        (n.to_string(), found)
    });
    let resumed = match &named {
        Some((_, found)) => found.clone(),
        None => arg.is_none().then(|| saved.last_workspace.as_deref().and_then(workspace::load)).flatten(),
    };
    let root = match &arg {
        _ if resumed.is_some() && named.is_some() => {
            resumed.as_ref().map(|w| w.root.clone()).filter(|p| p.is_dir()).unwrap_or_else(|| cwd.clone())
        }
        Some(p) if p.is_dir() => p.clone(),
        Some(_) => cwd.clone(),
        None => resumed
            .as_ref()
            .map(|w| w.root.clone())
            .filter(|p| p.is_dir())
            .or_else(|| saved.last_root.clone().filter(|p| p.is_dir()))
            .unwrap_or_else(|| cwd.clone()),
    };

    let mut app = App::new(root, size.height, size.width)?;

    // Launched with an explicit file/folder: skip the splash and go straight to work. A named
    // workspace keeps it, since the splash is where its name is announced.
    if arg.is_some() && named.is_none() {
        app.show_splash = false;
    }

    // A name given on the command line settles what to open, so the argument and resume paths
    // below are skipped entirely — `clee -w work` should not also try to open "-w" as a file.
    let opened_by_name = match named {
        Some((name, found)) => {
            match found {
                Some(ws) => app.apply_workspace(ws),
                None if workspace::is_default(&name) => {
                    let root = app.root.clone();
                    app.apply_workspace(workspace::default_workspace(root));
                }
                None => app.status_message = i18n::msg_workspace_unknown(app.settings.lang, &name),
            }
            true
        }
        None => false,
    };

    if !opened_by_name {
        match arg {
        Some(path) if !arg_is_dir => app.open_file_in_tab(path),
        Some(_) => {} // directory: already the root, nothing to open
        None => match resumed {
            Some(ws) => app.apply_workspace(ws),
            None => {
                for path in &saved.last_open_files {
                    if path.exists() {
                        app.open_file_in_tab(path.clone());
                    }
                }
                if let Some(active_path) = &saved.last_active_file {
                    if let Some(idx) =
                        app.editors.iter().position(|e| e.path.as_deref() == Some(active_path.as_path()))
                    {
                        app.active_editor = idx;
                    }
                }
            }
        },
        }
    }
    let mut last_external_check = Instant::now();
    // Consecutive frames that failed to draw. One is a hiccup worth reporting and carrying on
    // from; a run of them means the screen can't be painted at all, and something has to give.
    let mut failed_draws = 0u8;

    loop {
        match shielded(|| terminal.draw(|f| ui::draw(f, &mut app))) {
            Ok(drawn) => {
                drawn?;
                failed_draws = 0;
            }
            Err(text) => {
                failed_draws += 1;
                // Drawing is dominated by the terminal panes, so a pane is the likely culprit,
                // and a broken terminal should cost you that terminal and nothing else. Each
                // step here gives up strictly less than closing the editor did: the pane, then
                // the panel it lives in (`close_terminal` refuses to remove the last window, and
                // the editor is still worth having without it), and only then the session.
                if failed_draws >= 3 {
                    failed_draws = 0;
                    if app.terminals.len() > 1 {
                        let idx = app.active_terminal.min(app.terminals.len() - 1);
                        app.close_terminal(idx);
                    } else if app.settings.show_terminal {
                        app.settings.show_terminal = false;
                    } else {
                        // Nothing terminal-shaped left to blame. Breaking out still runs the
                        // teardown below, so the session is saved and the screen restored —
                        // which is exactly what dying on a panic never did.
                        app.status_message = format!("Internal error, closing: {text}");
                        break;
                    }
                }
                app.status_message = i18n::msg_internal_error(app.settings.lang, &text);
            }
        }

        if event::poll(Duration::from_millis(33))? {
            let event = event::read()?;
            let size = terminal.size()?;
            // Handling an event is where user input meets every subsystem, so it is the most
            // likely place to hit a bug. Shielded, that costs a status line instead of the
            // session — the shells in the panes keep running, untouched.
            let outcome = shielded(AssertUnwindSafe(|| match event {
                Event::Key(key) => {
                    if key.kind == event::KeyEventKind::Press {
                        app.handle_key(key);
                    }
                }
                Event::Mouse(mouse) => {
                    let full = Rect::new(0, 0, size.width, size.height);
                    let params = ui::LayoutParams::from_app(&app);
                    let areas = ui::compute_layout(full, &params);
                    app.handle_mouse(mouse, &areas, full);
                }
                Event::Paste(text) => app.handle_paste(text),
                Event::Resize(_, _) => {}
                _ => {}
            }));
            if let Err(text) = outcome {
                app.status_message = i18n::msg_internal_error(app.settings.lang, &text);
            }
        }

        let polled = shielded(AssertUnwindSafe(|| {
            if last_external_check.elapsed() >= Duration::from_millis(700) {
                app.poll_external_changes();
                last_external_check = Instant::now();
            }
            app.poll_splash();
            app.poll_background_messages();
            app.poll_terminal_exits();
            app.poll_git_status();
        }));
        if let Err(text) = polled {
            app.status_message = i18n::msg_internal_error(app.settings.lang, &text);
        }

        if app.should_quit {
            break;
        }
    }

    // Canonicalize before saving: a path opened via a relative CLI argument would
    // otherwise be stored as typed, and re-resolve against whatever the *next* launch's
    // cwd happens to be rather than the file it actually pointed to.
    let canonical = |p: std::path::PathBuf| std::fs::canonicalize(&p).unwrap_or(p);
    app.settings.last_root = Some(canonical(app.root.clone()));
    app.settings.last_open_files = app.editors.iter().filter_map(|e| e.path.clone()).map(canonical).collect();
    app.settings.last_active_file = app.editors.get(app.active_editor).and_then(|e| e.path.clone()).map(canonical);
    // The workspace in use is written back as it stands, so a terminal renamed or a seam
    // nudged during the session is still there next time it is opened.
    if let Some(name) = app.active_workspace.clone() {
        let _ = workspace::save(&app.capture_workspace(name));
    }
    app.settings.last_workspace = app.active_workspace.clone();
    app.settings.save();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property the whole safety net rests on: a panic in one step is reported back rather
    /// than unwinding out of the process. Without it, any bug anywhere took the editor and every
    /// shell running in it down at once.
    ///
    /// The default hook prints the caught panic to stderr while this runs — that is the test
    /// working, not failing.
    #[test]
    fn a_panic_is_contained_and_the_caller_carries_on() {
        assert_eq!(shielded(|| 2 + 2), Ok(4));

        let caught = shielded(|| panic!("terminal fell over"));
        assert!(caught.is_err(), "a panic must come back as an error, not end the process");

        // The step after a caught panic still runs: this is what "the session survives" means.
        assert_eq!(shielded(|| "still here"), Ok("still here"));
    }

    /// `shielded` reads what the hook recorded; with no hook installed there is nothing to read,
    /// and it must still describe the failure rather than panicking on an empty slot.
    #[test]
    fn a_panic_with_nothing_recorded_still_describes_itself() {
        *LAST_PANIC.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let caught: Result<u8, String> = shielded(|| panic!("boom"));
        let Err(text) = caught else { panic!("must be caught") };
        assert!(!text.is_empty());
    }
}
