//! Global shortcut registration.
//!
//! A global shortcut is a system-wide claim, so this module validates before
//! asking the OS for one and treats a refusal as recoverable rather than fatal.

use crate::error::ShellError;
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// Low conflict rate on macOS, and reachable with one hand.
pub const DEFAULT_SHORTCUT: &str = "Alt+Space";

const MODIFIERS: &[&str] = &[
    "Alt",
    "Option",
    "Control",
    "Ctrl",
    "Command",
    "Cmd",
    "Super",
    "Meta",
    "Shift",
    "CmdOrCtrl",
    "CommandOrControl",
];

const NAMED_KEYS: &[&str] = &[
    "Space",
    "Enter",
    "Return",
    "Tab",
    "Escape",
    "Esc",
    "Backspace",
    "Delete",
    "Up",
    "Down",
    "Left",
    "Right",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "Comma",
    "Period",
    "Slash",
    "Backslash",
    "Semicolon",
    "Quote",
    "BracketLeft",
    "BracketRight",
    "Minus",
    "Equal",
    "Backquote",
];

/// Why a shortcut string was rejected.
///
/// Separate from [`ShellError`] on purpose: these are user-correctable input
/// errors that Settings can render next to the field, whereas a registration
/// failure is environmental and needs a different explanation.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HotkeyError {
    #[error("shortcut is empty")]
    Empty,

    #[error("shortcut must include at least one modifier")]
    NoModifier,

    #[error("shortcut must include a key, not only modifiers")]
    NoKey,

    #[error("'{0}' is not a recognized modifier or key")]
    UnknownToken(String),
}

fn is_valid_key(token: &str) -> bool {
    if NAMED_KEYS.iter().any(|k| k.eq_ignore_ascii_case(token)) {
        return true;
    }

    // Any single alphanumeric character.
    if token.len() == 1 && token.chars().all(|c| c.is_ascii_alphanumeric()) {
        return true;
    }

    // F1 through F24.
    if let Some(number) = token
        .strip_prefix('F')
        .or_else(|| token.strip_prefix('f'))
        .and_then(|n| n.parse::<u8>().ok())
    {
        return (1..=24).contains(&number);
    }

    false
}

/// Validates a shortcut string before handing it to the OS.
///
/// This exists because registering a bare key globally is destructive: a
/// shortcut of `Space` swallows the spacebar in every application on the
/// machine, and the user's only recourse is to quit Magi — which they may not
/// manage, because typing is broken. Rejecting it here costs nothing.
pub fn validate_shortcut(input: &str) -> Result<(), HotkeyError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(HotkeyError::Empty);
    }

    let mut modifiers = 0usize;
    let mut keys = 0usize;

    for token in trimmed.split('+').map(str::trim) {
        if MODIFIERS.iter().any(|m| m.eq_ignore_ascii_case(token)) {
            modifiers += 1;
        } else if is_valid_key(token) {
            keys += 1;
        } else {
            return Err(HotkeyError::UnknownToken(token.to_string()));
        }
    }

    if keys == 0 {
        return Err(HotkeyError::NoKey);
    }
    if modifiers == 0 {
        return Err(HotkeyError::NoModifier);
    }

    Ok(())
}

/// Builds the global-shortcut plugin with Magi's handler attached.
pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _shortcut, event| {
            // The handler fires on BOTH press and release. Without this filter
            // the panel toggles twice per keypress, which looks like the hotkey
            // doing nothing at all.
            if event.state() != ShortcutState::Pressed {
                return;
            }

            if let Err(error) = crate::windows::toggle_panel(app) {
                tracing::error!(%error, "failed to toggle panel from hotkey");
            }
        })
        .build()
}

/// Swaps one registered shortcut for another, keeping the old one on failure.
///
/// The order is the whole point of this function.
///
/// Validation runs first, while `previous` is still registered, so a typo cannot
/// cost the user their working hotkey. Then, if the OS refuses the new
/// combination — most often because another application already owns it — the old
/// one is put back. Unregistering first and registering second would leave a
/// window in which a rejected shortcut means no hotkey at all, and for a
/// background app whose main entry point *is* the hotkey, that is close to
/// uninstalling it: the tray is the only way back, and the user has no reason to
/// expect the failure left them worse off than before they tried.
///
/// Restoring can itself fail. That is reported rather than swallowed, because the
/// user needs to know the hotkey is gone entirely — the one state where the tray
/// icon is the only way in.
pub fn rebind(app: &AppHandle, previous: &str, next: &str) -> Result<(), ShellError> {
    validate_shortcut(next).map_err(|e| ShellError::ShortcutRegistration {
        shortcut: next.to_string(),
        reason: e.to_string(),
    })?;

    if previous == next {
        return Ok(());
    }

    // A previous shortcut that was never successfully registered — startup found
    // it already taken — cannot be unregistered. That is not an error worth
    // failing the rebind for; the goal is for `next` to work.
    if let Err(error) = app.global_shortcut().unregister(previous) {
        tracing::debug!(%error, previous, "previous shortcut was not registered");
    }

    match register(app, next) {
        Ok(()) => Ok(()),
        Err(error) => {
            match register(app, previous) {
                Ok(()) => tracing::info!(previous, "restored the previous shortcut"),
                Err(restore) => tracing::error!(
                    %restore,
                    previous,
                    "could not restore the previous shortcut; no global hotkey is registered"
                ),
            }
            Err(error)
        }
    }
}

/// Validates and registers a shortcut.
pub fn register(app: &AppHandle, shortcut: &str) -> Result<(), ShellError> {
    validate_shortcut(shortcut).map_err(|e| ShellError::ShortcutRegistration {
        shortcut: shortcut.to_string(),
        reason: e.to_string(),
    })?;

    app.global_shortcut()
        .register(shortcut)
        .map_err(|e| ShellError::ShortcutRegistration {
            shortcut: shortcut.to_string(),
            // The OS does not say which application already holds a combination,
            // so this reports what is actually known rather than guessing.
            reason: format!(
                "{e} — another application may already own this combination, \
                 or Accessibility permission has not been granted"
            ),
        })?;

    tracing::info!(shortcut, "global shortcut registered");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_default_shortcut() {
        assert!(validate_shortcut(DEFAULT_SHORTCUT).is_ok());
    }

    #[test]
    fn accepts_multiple_modifiers() {
        assert!(validate_shortcut("CmdOrCtrl+Shift+M").is_ok());
    }

    #[test]
    fn accepts_function_keys() {
        assert!(validate_shortcut("Alt+F5").is_ok());
        assert_eq!(
            validate_shortcut("Alt+F25"),
            Err(HotkeyError::UnknownToken("F25".into()))
        );
    }

    #[test]
    fn rejects_an_empty_shortcut() {
        assert_eq!(validate_shortcut(""), Err(HotkeyError::Empty));
        assert_eq!(validate_shortcut("   "), Err(HotkeyError::Empty));
    }

    #[test]
    fn rejects_a_bare_key_with_no_modifier() {
        // Registering a bare key globally swallows it system-wide: "Space" would
        // stop the spacebar working in every application on the machine.
        assert_eq!(validate_shortcut("Space"), Err(HotkeyError::NoModifier));
        assert_eq!(validate_shortcut("A"), Err(HotkeyError::NoModifier));
    }

    #[test]
    fn rejects_modifiers_with_no_key() {
        assert_eq!(validate_shortcut("Alt+Shift"), Err(HotkeyError::NoKey));
    }

    #[test]
    fn rejects_an_unknown_token() {
        assert_eq!(
            validate_shortcut("Alt+Blorp"),
            Err(HotkeyError::UnknownToken("Blorp".into()))
        );
    }

    #[test]
    fn rejects_a_trailing_separator() {
        assert_eq!(
            validate_shortcut("Alt+"),
            Err(HotkeyError::UnknownToken(String::new()))
        );
    }
}
