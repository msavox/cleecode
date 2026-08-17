use arboard::Clipboard as SysClipboard;

/// Wraps the system clipboard (via `arboard`, which talks to the native clipboard on
/// macOS, Windows, X11 and Wayland) with an in-memory fallback so copy/cut/paste keep
/// working even where no system clipboard is reachable (e.g. a headless Linux box).
pub struct Clipboard {
    /// Held for the lifetime of the app: on X11 `arboard` serves clipboard requests from a
    /// background thread owned by this handle, so dropping it between operations would lose
    /// ownership of the selection. None if no clipboard backend could be initialised.
    sys: Option<SysClipboard>,
    fallback: String,
}

impl Clipboard {
    pub fn new() -> Self {
        Clipboard { sys: SysClipboard::new().ok(), fallback: String::new() }
    }

    pub fn set(&mut self, text: &str) {
        self.fallback = text.to_string();
        if let Some(cb) = self.sys.as_mut() {
            let _ = cb.set_text(text.to_string());
        }
        // Over ssh the system clipboard is the *server's*, which nobody can paste from. OSC 52
        // hands the text to the terminal instead, and the terminal is on the machine sitting in
        // front of you — the only route a copy has out of a remote session.
        if crate::dnd::running_over_ssh() {
            set_via_terminal(text);
        }
    }

    pub fn get(&mut self) -> String {
        if let Some(cb) = self.sys.as_mut() {
            if let Ok(text) = cb.get_text() {
                return text;
            }
        }
        self.fallback.clone()
    }
}

/// Asks the terminal to put `text` on its own clipboard (OSC 52).
///
/// Written straight to stdout, which ratatui also owns: safe because the sequence moves no
/// cursor and paints no cell, so a frame drawn around it is unaffected. Terminals cap how much
/// they will accept, so an oversized copy is dropped rather than sent as a truncated half.
fn set_via_terminal(text: &str) {
    use base64::Engine;
    use std::io::Write;
    const LIMIT: usize = 64 * 1024;
    if text.len() > LIMIT {
        return;
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b]52;c;{encoded}\x07");
    let _ = out.flush();
}
