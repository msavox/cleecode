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
