//! The IPC surface: what the frontend is allowed to ask for.
//!
//! Everything here is a thin adapter. The rules live in [`crate::config`]; these
//! functions translate between it and the frontend, and turn errors into strings
//! the UI can show. Keeping logic out of this layer is what lets it be tested
//! without a running app.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::config::secrets::SecretStore;
use crate::config::{ActiveModel, AppearanceConfig, Config, ProviderConfig, Theme};
use crate::llm::discovery;

/// Everything a command needs, assembled once at startup.
pub struct AppState {
    /// One client, reused. Building a fresh one per call would discard the
    /// connection pool and the TLS session cache on every request.
    pub http: reqwest::Client,
    pub config: Mutex<Config>,
    pub config_dir: PathBuf,
    pub secrets: Box<dyn SecretStore>,
}

/// A command failure, as a message the panel can render.
///
/// Commands return `Result<T, String>` rather than a typed error because the
/// frontend can only act on prose anyway. The typed errors live one layer down,
/// where callers can still match on them.
type CommandResult<T> = Result<T, String>;

fn to_message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// A provider as the settings screen needs to see it.
///
/// Deliberately not `ProviderConfig`: the UI must know *whether* a key is stored
/// without ever receiving it. Sending the key so the form could pre-fill it
/// would put the secret in the webview, in devtools, and in any screenshot of
/// the settings window.
#[derive(Debug, Serialize)]
pub struct ProviderView {
    pub id: String,
    pub kind: String,
    pub base_url: String,
    pub models: Vec<String>,
    pub requires_key: bool,
    pub has_key: bool,
    /// Enough to tell two keys apart, never enough to use one. `None` when no
    /// key is stored.
    pub key_hint: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ConfigView {
    pub providers: Vec<ProviderView>,
    pub active: Option<ActiveModel>,
    pub hotkey: String,
    pub appearance: AppearanceConfig,
    pub config_path: String,
}

#[derive(Debug, Deserialize)]
pub struct SaveProviderRequest {
    pub provider: ProviderConfig,
    /// `None` leaves any stored key untouched; `Some("")` clears it.
    pub api_key: Option<String>,
}

/// The stored key for a provider, if there is one.
///
/// A keychain error is treated as "no key" rather than surfaced: Settings must
/// still render so the user can fix whatever is wrong, and a locked keychain
/// should not blank the whole screen.
fn stored_key(state: &State<'_, AppState>, provider_id: &str) -> Option<String> {
    state
        .secrets
        .get(provider_id)
        .ok()
        .flatten()
        .filter(|k| !k.is_empty())
}

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> CommandResult<ConfigView> {
    let config = state.config.lock().map_err(to_message)?;

    let providers = config
        .providers
        .iter()
        .map(|p| ProviderView {
            id: p.id.clone(),
            kind: match p.kind {
                crate::config::ProviderKind::OpenaiCompatible => "openai-compatible".into(),
                crate::config::ProviderKind::Anthropic => "anthropic".into(),
            },
            base_url: p.base_url.clone(),
            models: p.models.clone(),
            requires_key: p.requires_key,
            has_key: stored_key(&state, &p.id).is_some(),
            key_hint: stored_key(&state, &p.id)
                .as_deref()
                .map(crate::config::secrets::fingerprint),
        })
        .collect();

    Ok(ConfigView {
        providers,
        active: config.active.clone(),
        hotkey: config.hotkey.toggle.clone(),
        appearance: config.appearance.clone(),
        // Shown in Settings so the file is discoverable. A config nobody can
        // find is a config nobody can paste into a bug report.
        config_path: Config::path_in(&state.config_dir).display().to_string(),
    })
}

/// Adds a provider, or replaces one with the same id.
#[tauri::command]
pub fn save_provider(
    state: State<'_, AppState>,
    request: SaveProviderRequest,
) -> CommandResult<ConfigView> {
    {
        let mut config = state.config.lock().map_err(to_message)?;

        match config
            .providers
            .iter_mut()
            .find(|p| p.id == request.provider.id)
        {
            Some(existing) => *existing = request.provider.clone(),
            None => config.providers.push(request.provider.clone()),
        }

        // Validate and persist before touching the keychain, so a rejected
        // config does not leave a secret behind for a provider that was never
        // saved.
        config.save(&state.config_dir).map_err(to_message)?;
    }

    if let Some(key) = request.api_key {
        if key.is_empty() {
            state
                .secrets
                .delete(&request.provider.id)
                .map_err(to_message)?;
        } else {
            state
                .secrets
                .set(&request.provider.id, &key)
                .map_err(to_message)?;
        }
    }

    get_config(state)
}

#[tauri::command]
pub fn remove_provider(state: State<'_, AppState>, id: String) -> CommandResult<ConfigView> {
    {
        let mut config = state.config.lock().map_err(to_message)?;
        config.providers.retain(|p| p.id != id);

        // Removing the provider a selection points at would leave the config
        // invalid, so the selection goes with it.
        if config.active.as_ref().is_some_and(|a| a.provider == id) {
            config.active = None;
        }

        config.save(&state.config_dir).map_err(to_message)?;
    }

    // Orphaned secrets are worth cleaning up: a key nobody can see is a key
    // nobody remembers to revoke.
    state.secrets.delete(&id).map_err(to_message)?;

    get_config(state)
}

#[tauri::command]
pub fn set_active_model(
    state: State<'_, AppState>,
    provider: String,
    model: String,
) -> CommandResult<ConfigView> {
    {
        let mut config = state.config.lock().map_err(to_message)?;
        let previous = config.active.take();

        config.active = Some(ActiveModel {
            provider: provider.clone(),
            model: model.clone(),
        });

        // Save validates. On rejection the previous selection is restored, so a
        // failed change does not leave the app with no model at all.
        if let Err(error) = config.save(&state.config_dir) {
            config.active = previous;
            return Err(to_message(error));
        }
    }

    get_config(state)
}

/// Asks a provider which models it serves.
///
/// Takes the provider from the form rather than from the saved config, so the
/// user can discover models for an endpoint they have typed but not yet saved —
/// which is the order people actually work in.
#[tauri::command]
pub async fn discover_models(
    state: State<'_, AppState>,
    provider: ProviderConfig,
    api_key: Option<String>,
) -> CommandResult<Vec<String>> {
    // A key typed into the form wins; otherwise fall back to the stored one, so
    // rediscovering an existing provider does not mean retyping its key.
    let key = match api_key.filter(|k| !k.is_empty()) {
        Some(typed) => Some(typed),
        None => state.secrets.get(&provider.id).map_err(to_message)?,
    };

    discovery::discover_models(&state.http, &provider, key.as_deref()).await
}

/// Applies a theme to every window and remembers it.
#[tauri::command]
pub fn set_theme(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    theme: Theme,
) -> CommandResult<ConfigView> {
    {
        let mut config = state.config.lock().map_err(to_message)?;
        config.appearance.theme = theme;
        config.save(&state.config_dir).map_err(to_message)?;
    }

    apply_theme(&app, theme);
    get_config(state)
}

/// Pushes the theme down to the webviews.
///
/// `None` means "follow the system", which is what makes System a real option
/// rather than a synonym for whichever mode happened to be active at startup.
pub fn apply_theme(app: &tauri::AppHandle, theme: Theme) {
    use tauri::Manager;

    let requested = match theme {
        Theme::System => None,
        Theme::Light => Some(tauri::Theme::Light),
        Theme::Dark => Some(tauri::Theme::Dark),
    };

    for label in [crate::windows::PANEL, crate::windows::SETTINGS] {
        if let Some(window) = app.get_webview_window(label) {
            if let Err(error) = window.set_theme(requested) {
                tracing::warn!(%error, label, "could not apply the theme to this window");
            }
        }
    }
}
