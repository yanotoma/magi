//! The menu-bar icon: Magi's only permanently visible surface.

use crate::error::ShellError;
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

/// Every state the tray icon can represent.
///
/// Only `Idle` and `PanelOpen` are reachable in M1. The rest are declared now so
/// that M3 (capability tiers), M4 (audio), and M6 (the session state machine)
/// extend this enum instead of each inventing a parallel one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellState {
    Idle,
    PanelOpen,
    Listening,
    Thinking,
    /// The configured model cannot see the screen (capability tier 3).
    Degraded,
}

/// The idle mark, embedded at compile time so the tray cannot fail to find it.
///
/// A macOS template image: black with an alpha channel and no colour, so the
/// system inverts it for light and dark menu bars and highlights it on click.
/// Regenerate with `python3 tools/generate_tray_icon.py`.
const ICON_IDLE: &[u8] = include_bytes!("../icons/tray/tray-idle.png");

/// Maps a state to its icon resource name.
///
/// Pure, so it is testable without a display — which matters because CI has
/// none. Asserting against `TrayIconBuilder` instead would pass whether or not
/// an icon ever appeared on screen.
///
/// Only `tray-idle` has an asset today. `tray-listening` and `tray-thinking`
/// are generated and waiting for M4 and M6; `tray-degraded` has no design yet
/// — see the note in `tools/generate_tray_icon.py`.
pub fn tray_icon_name(state: ShellState) -> &'static str {
    match state {
        ShellState::Idle | ShellState::PanelOpen => "tray-idle",
        ShellState::Listening => "tray-listening",
        ShellState::Thinking => "tray-thinking",
        ShellState::Degraded => "tray-degraded",
    }
}

/// Derives the shell state from panel visibility.
///
/// Trivial today, but it is the seam where M4's listening state and M6's
/// thinking state attach. Naming it now means those milestones change one
/// function instead of threading a new condition through the tray code.
pub fn state_for_panel(panel_visible: bool) -> ShellState {
    if panel_visible {
        ShellState::PanelOpen
    } else {
        ShellState::Idle
    }
}

/// Builds the tray icon and its menu. Called once from the setup hook.
pub fn init(app: &tauri::App) -> Result<(), ShellError> {
    let open = MenuItemBuilder::with_id("open", "Open Magi").build(app)?;
    let settings = MenuItemBuilder::with_id("settings", "Settings…").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit Magi").build(app)?;

    let menu = MenuBuilder::new(app)
        .items(&[&open, &settings])
        .separator()
        .items(&[&quit])
        .build()?;

    TrayIconBuilder::with_id("main")
        .icon(
            tauri::image::Image::from_bytes(ICON_IDLE)
                .map_err(|e| ShellError::Tray(format!("tray icon failed to decode: {e}")))?,
        )
        // Tell macOS this is a template image so it inverts the mark for light
        // and dark menu bars. Without it the icon is drawn as-is and goes
        // invisible against a matching background.
        .icon_as_template(true)
        .menu(&menu)
        // Left click toggles the panel, so the menu belongs on right click only.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => log_if_err(
                "toggle panel from tray menu",
                crate::windows::toggle_panel(app),
            ),
            "settings" => log_if_err("open settings", crate::windows::show_settings(app)),
            "quit" => app.exit(0),
            other => tracing::warn!(id = other, "unhandled tray menu event"),
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                log_if_err(
                    "toggle panel from tray click",
                    crate::windows::toggle_panel(tray.app_handle()),
                );
            }
        })
        .build(app)
        .map_err(|e| ShellError::Tray(e.to_string()))?;

    Ok(())
}

/// Tray callbacks cannot return a `Result`, and a panic here would take down a
/// background app with nothing on screen to explain why. Log and keep running.
fn log_if_err(action: &str, result: Result<(), ShellError>) {
    if let Err(error) = result {
        tracing::error!(%error, action, "tray action failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_and_panel_open_share_the_base_icon() {
        assert_eq!(tray_icon_name(ShellState::Idle), "tray-idle");
        assert_eq!(tray_icon_name(ShellState::PanelOpen), "tray-idle");
    }

    #[test]
    fn active_states_have_distinct_icons() {
        assert_eq!(tray_icon_name(ShellState::Listening), "tray-listening");
        assert_eq!(tray_icon_name(ShellState::Thinking), "tray-thinking");
    }

    #[test]
    fn degraded_is_visually_distinct_from_idle() {
        assert_ne!(
            tray_icon_name(ShellState::Degraded),
            tray_icon_name(ShellState::Idle),
            "a user whose model cannot see the screen must be able to tell at a glance"
        );
    }

    #[test]
    fn panel_visibility_maps_to_a_shell_state() {
        assert_eq!(state_for_panel(true), ShellState::PanelOpen);
        assert_eq!(state_for_panel(false), ShellState::Idle);
    }
}
