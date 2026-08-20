mod app;
mod app_install;
mod clipboard;
mod complete;
mod dnd;
mod editor;
mod file_tree;
mod find;
mod font_install;
mod git;
mod git_status;
mod highlight;
mod i18n;
mod manual;
mod menu;
mod picker;
mod preview;
mod search;
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
clee — CleeCode, a terminal IDE: an editor, a file tree and real terminals in one window.

USAGE:
    clee [FILE|DIRECTORY]   a directory becomes the project root; a file opens in the
                            current one. With no argument, the last project and its open
                            files come back.
    clee -w NAME            open a saved workspace
    clee -e FILE            open just that file, with everything else hidden

OPTIONS:
    -e, --edit FILE       Editor only: no sidebar, no terminals, no menu bar. Your saved
                          layout and session are left alone, so a quick edit does not
                          become the state you come back to.
    -w, --workspace NAME  Open a workspace: its root, files, frame sizes and terminals,
                          each shell running the command it was given. With no NAME,
                          lists the ones you have.
    --install-font        Install the bundled Nerd Font, so the file tree icons render.
    --install-app         macOS: put a CleeCode launcher in /Applications, so it can live
                          in the Dock and be the app that opens a file or a folder.
    --resume              Start in the project last worked in, wherever this was run from.
                          What the Dock launcher uses; a bare `clee` still uses the
                          directory you are standing in.
    -h, --help            Print this help.
    -V, --version         Print the version.

KEYS TO START WITH:
    Ctrl+P                every action in the app, fuzzy-searched — nothing to memorise
    Ctrl+Shift+M          the built-in manual, in English or Italian
    Ctrl+Alt+arrows       go to the frame in that direction
    Ctrl+Shift+B          open the menu bar

    A focused terminal keeps every other Ctrl chord for the shell running in it.

More in the manual (Ctrl+Shift+M) and in man clee.
";

/// A terminal whose writes go through a large buffer. See where it is built for why.
type BufferedTerminal =
    ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::BufWriter<std::io::Stdout>>>;

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
    if args.iter().any(|a| a == "--install-app") {
        app_install::install();
        return Ok(());
    }
    let resume = args.iter().any(|a| a == "--resume");
    let mut open_workspace: Option<String> = None;
    // `-edit` is accepted alongside the usual spellings: it is what the request asked for, and
    // refusing a flag over a missing dash helps nobody.
    let mut edit_file: Option<std::path::PathBuf> = None;
    if let Some(i) = args
        .iter()
        .position(|a| a == "-e" || a == "--edit" || a == "-edit")
    {
        match args.get(i + 1) {
            Some(path) => edit_file = Some(std::path::PathBuf::from(path)),
            None => {
                eprintln!("clee -e needs a file to edit");
                return Ok(());
            }
        }
    }
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
    // Not `ratatui::init()`, which writes to a bare `Stdout` — and Rust's `Stdout` is line
    // buffered, so a payload with no newlines in it leaves in 8 KB pieces. That is exactly
    // right for a TUI's usual traffic of a few changed cells, and exactly wrong for a picture:
    // one image is megabytes of escape sequence, which became thousands of small writes onto a
    // pty and seconds of waiting to see it. A big buffer sends it in a handful of them instead.
    const FRAME_BUFFER: usize = 4 * 1024 * 1024;

    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(stdout(), crossterm::terminal::EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(std::io::BufWriter::with_capacity(
        FRAME_BUFFER,
        stdout(),
    ));
    let mut terminal = ratatui::Terminal::new(backend)?;
    crossterm::execute!(stdout(), EnableMouseCapture, EnableBracketedPaste)?;
    // Ask for disambiguated key reporting where the terminal offers it. Without it Ctrl+Tab
    // arrives as a plain Tab — the two are the same byte, 0x09, in the encoding terminals have
    // used since VT100 — so cycling frames from the keyboard would be impossible to tell from
    // indenting. Ghostty, kitty, WezTerm and foot support this; Terminal.app does not, which is
    // why Alt+1/2/3 reach the frames directly and work everywhere.
    // Asked here, alongside the other capability query and for the same reason: it writes to
    // stdout and waits for the terminal's answer, which cannot happen once frames are being
    // drawn. It has its own timeout and falls back to half-blocks, so a silent terminal costs a
    // coarser picture rather than a stall.
    preview::detect_terminal();
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
    let result = shielded(|| run(&mut terminal, open_workspace, edit_file, resume));
    let _ = write!(stdout(), "\x1b[23;2t");
    // Popped before anything else is undone: leaving the flags pushed would hand the shell back
    // a terminal that reports keys in a mode it never asked for.
    if enhanced {
        let _ = crossterm::execute!(stdout(), crossterm::event::PopKeyboardEnhancementFlags);
    }
    let _ = crossterm::execute!(stdout(), DisableBracketedPaste, DisableMouseCapture);
    let _ = crossterm::execute!(stdout(), crossterm::terminal::LeaveAlternateScreen);
    let _ = crossterm::terminal::disable_raw_mode();
    match result {
        Ok(r) => r,
        Err(text) => {
            eprintln!("clee closed after an internal error: {text}");
            Ok(())
        }
    }
}

fn run(
    terminal: &mut BufferedTerminal,
    open_workspace: Option<String>,
    edit_file: Option<std::path::PathBuf>,
    resume: bool,
) -> Result<()> {
    let size = terminal.size()?;
    let cwd = std::env::current_dir()?;
    // Only a real path counts. Without the filter `clee --resume` would take its own flag
    // for a file name and try to open it — the other flags never get this far, but this one
    // is passed through to here.
    let arg = std::env::args()
        .nth(1)
        .filter(|a| !a.starts_with('-'))
        .map(std::path::PathBuf::from);
    let arg_is_dir = arg.as_ref().map(|p| p.is_dir()).unwrap_or(false);

    // Resume the last workspace (project folder + open tabs) when launched with no
    // explicit argument; an argument always takes precedence and starts fresh. A directory
    // argument becomes the project root; a file argument is opened within the current dir.
    let saved = Settings::load();
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
        // A bare `clee` never reopens a named workspace. Opening one is a decision — `-w`, or
        // picking it from the menu — and having yesterday's `claude` shell start itself because
        // it happened to be the last one used is a surprise, not a convenience. What does carry
        // over is the layout, which lives in the settings and is restored either way, so you get
        // the default workspace with at most the shape you left the window in.
        None => None,
    };
    let root = match &arg {
        _ if edit_file.is_some() => edit_file
            .as_ref()
            .and_then(|f| f.parent().map(|p| p.to_path_buf()))
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| cwd.clone()),
        _ if resumed.is_some() && named.is_some() => {
            resumed.as_ref().map(|w| w.root.clone()).filter(|p| p.is_dir()).unwrap_or_else(|| cwd.clone())
        }
        Some(p) if p.is_dir() => p.clone(),
        Some(_) => cwd.clone(),
        // Started from the Dock, where there is no directory you were standing in: the last
        // project is the only sensible answer, and it makes the restore below match.
        None if resume => {
            saved.last_root.clone().filter(|p| p.is_dir()).unwrap_or_else(|| cwd.clone())
        }
        // Where you are. A shell command that ignores the directory it was typed in is a
        // surprise every time it happens in a folder that is not the one you left — and the
        // session you *did* leave is still restored, below, whenever you come back to it.
        None => cwd.clone(),
    };

    let mut app = App::new(root, size.height, size.width)?;

    // Launched with an explicit file/folder: skip the splash and go straight to work. A named
    // workspace keeps it, since the splash is where its name is announced.
    if arg.is_some() && named.is_none() {
        app.show_splash = false;
    }

    // Minimal mode: the editor on one file and nothing else. Frames are hidden rather than
    // removed — the rest of the app assumes a terminal exists, and one that is never drawn costs
    // nothing to keep. The splash goes too: this is meant to feel like `micro`, and a title card
    // is not what you want when you opened a file to change one line.
    if let Some(path) = &edit_file {
        app.settings.show_sidebar = false;
        app.settings.show_terminal = false;
        app.settings.show_menubar = false;
        app.show_splash = false;
        app.focus = app::Focus::Editor;
        app.open_file_in_tab(path.clone());
    }

    // A name given on the command line settles what to open, so the argument and resume paths
    // below are skipped entirely — `clee -w work` should not also try to open "-w" as a file.
    let opened_by_name = edit_file.is_some()
        || match named {
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
                // The files come back only where they belong. Reopening the last session's
                // buffers in a different project would put somebody else's files in front of
                // you, named after a folder you are not in.
                // Compared after resolving symlinks, since the remembered path was written
                // that way and the one you typed may not be.
                let real = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.into());
                let same_project =
                    saved.last_root.as_deref().is_some_and(|last| real(last) == real(&app.root));
                for path in saved.last_open_files.iter().filter(|_| same_project) {
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
            app.poll_turtle();
            app.poll_background_messages();
            app.poll_previews();
            app.refresh_rendered_previews();
            app.poll_terminal_exits();
            app.poll_git_status();
            app.poll_search();
            app.poll_git_panel();
        }));
        if let Err(text) = polled {
            app.status_message = i18n::msg_internal_error(app.settings.lang, &text);
        }

        if app.should_quit {
            break;
        }
    }

    // Minimal mode leaves no trace: not the hidden frames, not the file, not the project. It is
    // a one-off edit, and coming back to a CleeCode with everything switched off — because of a
    // `clee -e` last Tuesday — would be the opposite of what it is for.
    if edit_file.is_some() {
        return Ok(());
    }

    // Canonicalize before saving: a path opened via a relative CLI argument would
    // otherwise be stored as typed, and re-resolve against whatever the *next* launch's
    // cwd happens to be rather than the file it actually pointed to.
    let canonical = |p: std::path::PathBuf| std::fs::canonicalize(&p).unwrap_or(p);
    app.settings.last_root = Some(canonical(app.root.clone()));
    app.settings.last_open_files = app.editors.iter().filter_map(|e| e.path.clone()).map(canonical).collect();
    app.settings.last_active_file = app.editors.get(app.active_editor).and_then(|e| e.path.clone()).map(canonical);
    // The workspace in use is written back as it stands, so a terminal renamed or a seam
    // nudged during the session is still there next time it is opened. The built-in is the
    // exception and is never written: it is the layout you go back to, so it has to stay put.
    if let Some(name) = app.active_workspace.clone() {
        if !workspace::is_default(&name) {
            let _ = workspace::save(&app.capture_workspace(name));
        }
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
