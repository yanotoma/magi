//! Show, hide, and toggle Magi's two windows.
//!
//! Both windows are declared in `tauri.conf.json` and start hidden. Nothing here
//! creates a window at runtime — declaring them up front means a missing window
//! is a configuration error caught at startup rather than a silent no-op later.

use crate::error::ShellError;
use tauri::{AppHandle, Manager, WebviewWindow};

/// The transparent overlay the user talks to.
pub const PANEL: &str = "panel";

/// The ordinary settings window.
pub const SETTINGS: &str = "settings";

fn window(app: &AppHandle, label: &str) -> Result<WebviewWindow, ShellError> {
    app.get_webview_window(label)
        .ok_or_else(|| ShellError::WindowNotFound(label.to_string()))
}

pub fn show_panel(app: &AppHandle) -> Result<(), ShellError> {
    let panel = window(app, PANEL)?;
    panel.show()?;
    panel.set_focus()?;
    Ok(())
}

pub fn hide_panel(app: &AppHandle) -> Result<(), ShellError> {
    window(app, PANEL)?.hide()?;
    Ok(())
}

/// Flips the panel's visibility. Both the tray icon and the global hotkey route
/// here, so there is one definition of what "toggle" means.
pub fn toggle_panel(app: &AppHandle) -> Result<(), ShellError> {
    let panel = window(app, PANEL)?;
    if panel.is_visible()? {
        panel.hide()?;
    } else {
        panel.show()?;
        panel.set_focus()?;
    }
    Ok(())
}

pub fn show_settings(app: &AppHandle) -> Result<(), ShellError> {
    let settings = window(app, SETTINGS)?;
    settings.show()?;
    // Closing the settings window hides it; it can also be minimised while
    // hidden, so both have to be undone to bring it back.
    settings.unminimize()?;
    settings.set_focus()?;
    Ok(())
}
