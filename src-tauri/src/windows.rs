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
        // Hidden, not ended. The thread survives being closed and only **Clear** discards it
        // — a reversal of the design doc, and the maintainer's call. Escape and clicking away
        // are easy to trigger by accident, and a conversation lost to a mistaken keypress is
        // a cost paid every time, against a privacy benefit that matters only when somebody
        // else is at the machine.
        panel.hide()?;
        // The thread stays; the screenshot does not stay forever. Five minutes closed and the
        // image is released — long enough to look something up and come back, short enough
        // that a picture of the screen is not still in memory after lunch.
        crate::commands::expire_capture_later(app);
        crate::session::refresh_shell(app);
        return Ok(());
    }

    panel.show()?;
    panel.set_focus()?;
    crate::commands::cancel_capture_expiry(app);

    // The menu bar has a mark for the panel being open and had no way to learn about it:
    // until now only a session event made the tray recompute, so opening the panel left the
    // icon showing whatever it happened to be showing.
    crate::session::refresh_shell(app);
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
