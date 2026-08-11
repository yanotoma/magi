//! Show, hide, and toggle Magi's two windows.
//!
//! Both windows are declared in `tauri.conf.json` and start hidden. Nothing here
//! creates a window at runtime — declaring them up front means a missing window
//! is a configuration error caught at startup rather than a silent no-op later.
//!
//! A leaf. Windows are shown, hidden and focused here and nothing else is decided: what
//! opening or closing the panel *means* — the menu bar mark, the screenshot that expires —
//! lives in `session`, which `CLAUDE.md` names as the only module allowed to know about the
//! others. This file briefly did know, and the leak is worth naming: orchestration is easiest
//! to put wherever the window handle already is.

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

/// Hides the panel, and only that.
///
/// What *else* closing the panel entails — the screenshot that expires, the menu bar mark —
/// belongs to `session::toggle_panel`. This module is not allowed to know those things exist.
pub fn hide_panel(app: &AppHandle) -> Result<(), ShellError> {
    window(app, PANEL)?.hide()?;
    Ok(())
}

/// Whether the panel is on screen.
pub fn panel_is_visible(app: &AppHandle) -> Result<bool, ShellError> {
    Ok(window(app, PANEL)?.is_visible()?)
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
