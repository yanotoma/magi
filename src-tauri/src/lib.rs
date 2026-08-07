//! Magi's composition root.
//!
//! `main.rs` only calls [`run`]. Everything the application is made of is wired
//! together here, so there is one place to read to understand what happens at
//! startup.

pub mod commands;
pub mod config;
pub mod error;
pub mod hotkey;
pub mod llm;
pub mod tray;
pub mod windows;

use std::sync::Mutex;

use tauri::{Manager, WindowEvent};

use crate::commands::AppState;
use crate::config::secrets::KeyringStore;
use crate::config::Config;

/// Builds and runs the Tauri application.
///
/// The `expect` at the end is permitted by the project's no-panic rule: it is in
/// the startup path, where a failure means the app genuinely cannot run and there
/// is no user-facing surface left to degrade into.
pub fn run() {
    init_tracing();

    tauri::Builder::default()
        .plugin(hotkey::plugin())
        // Remember where the user dragged the panel to.
        //
        // POSITION only, deliberately. The plugin can also restore VISIBLE, and
        // doing so here would show the panel on launch — turning a background
        // agent into an app that greets you with a window every login. SIZE and
        // MAXIMIZED are equally wrong for a fixed-size overlay.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(tauri_plugin_window_state::StateFlags::POSITION)
                .build(),
        )
        .setup(|app| {
            // Magi is a background agent: no Dock icon, no app-switcher entry.
            // `skipTaskbar` in the window config does not do this on macOS — it
            // is a no-op there — so the activation policy is the only lever.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let config_dir = app.path().app_config_dir()?;

            // A config that fails to parse must not stop the app from starting.
            // The user would have no window in which to be told why, and no way
            // to reach Settings to fix it. Fall back to defaults, say so loudly
            // in the log, and leave the broken file untouched so it can be
            // repaired rather than silently overwritten.
            let config = Config::load(&config_dir).unwrap_or_else(|error| {
                tracing::error!(
                    %error,
                    path = %Config::path_in(&config_dir).display(),
                    "config could not be loaded; starting with defaults and leaving the file alone"
                );
                Config::default()
            });

            let theme = config.appearance.theme;
            let shortcut = config.hotkey.toggle.clone();

            app.manage(AppState {
                http: reqwest::Client::new(),
                config: Mutex::new(config),
                config_dir,
                secrets: Box::new(KeyringStore),
                key_hints: Mutex::new(std::collections::HashMap::new()),
                in_flight: Mutex::new(None),
            });

            commands::apply_theme(app.handle(), theme);

            tray::init(app)?;

            // The configured shortcut, not the default one. Registering the
            // default here would ignore the field the user just set in Settings,
            // so their hotkey would work until they quit and then revert on every
            // launch — a bug that looks like the setting not saving at all.
            //
            // A shortcut conflict must not prevent startup. The tray icon is
            // still a working way in, and killing launch over a hotkey clash
            // would leave the user with no entry point at all.
            if let Err(error) = hotkey::register(app.handle(), &shortcut) {
                tracing::warn!(%error, shortcut, "continuing without a global shortcut");
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::save_provider,
            commands::remove_provider,
            commands::set_active_model,
            commands::discover_models,
            commands::set_theme,
            commands::set_show_thinking,
            commands::set_prompt_context,
            commands::set_hotkey,
            commands::send_text_turn,
            commands::cancel_turn,
        ])
        .on_window_event(|window, event| {
            // Closing a window must never quit a tray app. Hide instead, and let
            // the tray's Quit item be the only way out.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Err(error) = window.hide() {
                    tracing::error!(%error, label = window.label(), "failed to hide window on close");
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("fatal: Tauri failed to start");
}

/// Structured logging, off unless asked for.
///
/// `RUST_LOG=magi=debug` turns it on. A background app has no terminal the user
/// is watching, so logging is for bug reports rather than for the running user.
fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("magi=info,warn"));

    tracing_subscriber::fmt().with_env_filter(filter).init();
}
