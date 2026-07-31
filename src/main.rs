mod app;
mod clipboard;
mod dnd;
mod editor;
mod file_tree;
mod font_install;
mod git_status;
mod highlight;
mod i18n;
mod menu;
mod settings;
mod terminal_panel;
mod ui;

use anyhow::Result;
use app::App;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
};
use ratatui::layout::Rect;
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
    let root = std::env::current_dir()?;
    let file_arg = std::env::args().nth(1);
    let mut app = App::new(root, size.height, size.width)?;
    if let Some(path) = file_arg {
        app.open_file_in_tab(std::path::PathBuf::from(path));
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

    app.settings.save();
    Ok(())
}
