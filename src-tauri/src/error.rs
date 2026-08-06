//! Errors the shell can produce.
//!
//! These are an enum rather than a string because each variant maps to a
//! different user-facing surface: the caller needs to know whether to show a
//! permissions prompt, a settings warning, or a bug-report link.

/// A failure in the desktop shell — tray, windows, or global shortcuts.
#[derive(Debug, thiserror::Error)]
pub enum ShellError {
    #[error("failed to build the tray icon: {0}")]
    Tray(String),

    #[error("window '{0}' was not found")]
    WindowNotFound(String),

    #[error("shortcut '{shortcut}' could not be registered: {reason}")]
    ShortcutRegistration { shortcut: String, reason: String },

    #[error(transparent)]
    Tauri(#[from] tauri::Error),
}
