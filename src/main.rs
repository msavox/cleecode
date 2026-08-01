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
mod menu;
mod picker;
mod settings;
mod terminal_panel;
mod ui;

use anyhow::Result;
use app::App;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
};
use ratatui::layout::Rect;
use settings::Settings;
use std::io::{stdout, Write};
use std::time::{Duration, Instant};

fn main() -> Result<()> {
    if std::env::args().any(|a| a == "--install-font") {
        font_install::install();
        return Ok(());
    }
    let mut terminal = ratatui::init();
    crossterm::execute!(stdout(), EnableMouseCapture, EnableBracketedPaste)?;
    // Push (save) the terminal's current title, then set our own; tmux/the shell would
    // otherwise leave their own title showing in the window chrome for the whole session.
    let _ = write!(stdout(), "\x1b[22;2t\x1b]0;CleeCode\x07");
    let _ = stdout().flush();
    let result = run(&mut terminal);
    let _ = write!(stdout(), "\x1b[23;2t");
    let _ = crossterm::execute!(stdout(), DisableBracketedPaste, DisableMouseCapture);
    ratatui::restore();
    result
}

fn run(terminal: &mut ratatui::DefaultTerminal) -> Result<()> {
    let size = terminal.size()?;
    let cwd = std::env::current_dir()?;
    let arg = std::env::args().nth(1).map(std::path::PathBuf::from);
    let arg_is_dir = arg.as_ref().map(|p| p.is_dir()).unwrap_or(false);

    // Resume the last workspace (project folder + open tabs) when launched with no
    // explicit argument; an argument always takes precedence and starts fresh. A directory
    // argument becomes the project root; a file argument is opened within the current dir.
    let saved = Settings::load();
    let root = match &arg {
        Some(p) if p.is_dir() => p.clone(),
        Some(_) => cwd.clone(),
        None => saved.last_root.clone().filter(|p| p.is_dir()).unwrap_or_else(|| cwd.clone()),
    };

    let mut app = App::new(root, size.height, size.width)?;

    // Launched with an explicit file/folder: skip the splash and go straight to work.
    if arg.is_some() {
        app.show_splash = false;
    }

    match arg {
        Some(path) if !arg_is_dir => app.open_file_in_tab(path),
        Some(_) => {} // directory: already the root, nothing to open
        None => {
            for path in &saved.last_open_files {
                if path.exists() {
                    app.open_file_in_tab(path.clone());
                }
            }
            if let Some(active_path) = &saved.last_active_file {
                if let Some(idx) = app.editors.iter().position(|e| e.path.as_deref() == Some(active_path.as_path())) {
                    app.active_editor = idx;
                }
            }
        }
    }
    let mut last_external_check = Instant::now();

    loop {
        terminal.draw(|f| ui::draw(f, &mut app))?;

        if event::poll(Duration::from_millis(33))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind == event::KeyEventKind::Press {
                        app.handle_key(key);
                    }
                }
                Event::Mouse(mouse) => {
                    let size = terminal.size()?;
                    let full = Rect::new(0, 0, size.width, size.height);
                    let params = ui::LayoutParams::from_app(&app);
                    let areas = ui::compute_layout(full, &params);
                    app.handle_mouse(mouse, &areas, full);
                }
                Event::Paste(text) => app.handle_paste(text),
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        if last_external_check.elapsed() >= Duration::from_millis(700) {
            app.poll_external_changes();
            last_external_check = Instant::now();
        }
        app.poll_splash();
        app.poll_background_messages();
        app.poll_terminal_exits();
        app.poll_git_status();

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
    app.settings.save();
    Ok(())
}
