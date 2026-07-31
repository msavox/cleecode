use std::io::Write;
use std::process::{Command, Stdio};

/// Wraps the system clipboard (pbcopy/pbpaste on macOS) with an in-memory
/// fallback so copy/cut/paste keep working even where those tools aren't available.
pub struct Clipboard {
    fallback: String,
}

impl Clipboard {
    pub fn new() -> Self {
        Clipboard { fallback: String::new() }
    }

    pub fn set(&mut self, text: &str) {
        self.fallback = text.to_string();
        let _ = set_system_clipboard(text);
    }

    pub fn get(&self) -> String {
        get_system_clipboard().unwrap_or_else(|| self.fallback.clone())
    }
}

#[cfg(target_os = "macos")]
fn set_system_clipboard(text: &str) -> Option<()> {
    let mut child = Command::new("pbcopy").stdin(Stdio::piped()).spawn().ok()?;
    child.stdin.take()?.write_all(text.as_bytes()).ok()?;
    child.wait().ok()?;
    Some(())
}

#[cfg(target_os = "macos")]
fn get_system_clipboard() -> Option<String> {
    let output = Command::new("pbpaste").output().ok()?;
    if output.status.success() {
        String::from_utf8(output.stdout).ok()
    } else {
        None
    }
}

#[cfg(not(target_os = "macos"))]
fn set_system_clipboard(_text: &str) -> Option<()> {
    None
}

#[cfg(not(target_os = "macos"))]
fn get_system_clipboard() -> Option<String> {
    None
}
