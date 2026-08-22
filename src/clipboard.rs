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
        Clipboard { sys: Self::system_clipboard(), fallback: String::new() }
    }

    /// The system clipboard, or `None` when opening one would be a liability rather than a
    /// feature.
    ///
    /// Not opened at all over ssh, and that is not caution about a feature nobody uses — it is
    /// the same reasoning `set` already acts on eight lines down: over ssh the system clipboard
    /// is the *server's*, which nobody sitting in front of the terminal can paste from. The copy
    /// goes out through OSC 52 instead, and paste arrives as bracketed paste from the terminal,
    /// so neither direction ever asked this for anything.
    ///
    /// What it did do was hold a connection open. On Linux `arboard` talks X11 and keeps the
    /// connection for the lifetime of the handle; under `ssh -X` that is a *forwarded* display,
    /// and forwarding goes away — untrusted `-X` expires after twenty minutes by default, and a
    /// dropped link does it sooner. When it goes, libxcb's fatal-IO handler calls `exit()`: no
    /// unwinding, no panic to catch, and none of the teardown in `main` that puts the terminal
    /// back. What is left is a shell on top of CleeCode's last frame with mouse reporting still
    /// on, spraying `35;colonna;riga M` at every twitch of the pointer. Reported from a real
    /// session on 2026-08-22.
    ///
    /// Nobody has to copy anything for this to happen. The connection was opened at startup,
    /// for a clipboard that could never have been useful.
    fn system_clipboard() -> Option<SysClipboard> {
        if crate::dnd::running_over_ssh() {
            return None;
        }
        SysClipboard::new().ok()
    }

    pub fn set(&mut self, text: &str) {
        self.fallback = text.to_string();
        if let Some(cb) = self.sys.as_mut() {
            let _ = cb.set_text(text.to_string());
        }
        // Over ssh the system clipboard is the *server's*, which nobody can paste from — which
        // is also why `system_clipboard` never opened one. OSC 52 hands the text to the terminal
        // instead, and the terminal is on the machine sitting in front of you: the only route a
        // copy has out of a remote session.
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
