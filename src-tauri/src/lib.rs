//! Magi's composition root.
//!
//! `main.rs` only calls [`run`]. Everything the application is made of is wired
//! together here, so there is one place to read to understand what happens at
//! startup.

pub mod audio;
pub mod capture;
pub mod commands;
pub mod config;
pub mod error;
pub mod hotkey;
pub mod llm;
pub mod logging;
pub mod permissions;
pub mod session;
pub mod stt;
pub mod tray;
pub mod voice;
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
    // Bound, not dropped. The guard flushes the file writer, and letting it fall out of
    // scope here would stop the log file receiving anything — silently, with no error
    // anywhere, which is precisely the failure `logging` exists to end. It lives until
    // `run` returns, which is when the app exits.
    let _log_guard = logging::init();

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
                // The file name, not the path. The directory is Magi's own and never in
                // doubt, while the path to it runs through the user's home and carries
                // their account name into a file they may attach to a bug report.
                tracing::error!(
                    %error,
                    file = %Config::path_in(&config_dir)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                    "config could not be loaded; starting with defaults and leaving the file alone"
                );
                Config::default()
            });

            let theme = config.appearance.theme;
            let shortcut = config.hotkey.toggle.clone();
            let active = config.active.clone();
            let push_to_talk = config.hotkey.push_to_talk.clone();
            let config_voice_model = config.voice.model;
            // `None` means detect. The config stores "auto" as a string because it also
            // has to survive being hand-edited; the transcriber wants the absence.
            let config_voice_languages = config.voice.languages.clone();

            // Probe results, if any exist. Infallible: a missing or unreadable file
            // means nothing has been probed yet, which is an ordinary first-run
            // state rather than a failure to report.
            let capabilities = crate::llm::cache::CapabilityCache::load(&config_dir);

            // Read before the cache is moved into the state, so the tray shows the
            // right thing from the first hover rather than only after something
            // changes.
            let startup_tier = active
                .as_ref()
                .and_then(|a| capabilities.tier(&a.provider, &a.model));

            // Speech models live beside the config but not in it: a 141 MB binary blob
            // has no business next to a file the user is encouraged to open in an
            // editor and paste into bug reports.
            let models_dir = app.path().app_data_dir()?.join("models");

            app.manage(AppState {
                microphone: Box::new(crate::audio::Microphone::new()),
                transcriber: Mutex::new(std::sync::Arc::new(
                    crate::stt::WhisperTranscriber::new(
                        config_voice_model,
                        &models_dir,
                        config_voice_languages,
                    ),
                )),
                http: reqwest::Client::new(),
                http_blocking: reqwest::blocking::Client::new(),
                models_dir,
                downloading: Mutex::new(None),
                config: Mutex::new(config),
                config_dir,
                secrets: std::sync::Arc::new(KeyringStore),
                key_hints: Mutex::new(std::collections::HashMap::new()),
                capabilities: Mutex::new(capabilities),
                // ScreenCaptureKit on macOS; the fake everywhere else, so a Linux build
                // links and runs rather than needing a second `AppState` shape. Nothing
                // reaches this yet — the tool-call loop that would is M5's remaining work.
                #[cfg(target_os = "macos")]
                screen: std::sync::Arc::new(crate::capture::ScreenCaptureKit::new()),
                #[cfg(not(target_os = "macos"))]
                screen: std::sync::Arc::new(crate::capture::FakeCapture::headless()),
                session: std::sync::Arc::new(crate::session::Session::new()),
                last_capture: Mutex::new(None),
                panel_hidden_at: Mutex::new(None),
                capture_log: std::sync::Arc::new(crate::capture::CaptureLog::new()),
                in_flight: Mutex::new(None),
            });

            commands::apply_theme(app.handle(), theme);

            tray::init(app)?;

            tray::refresh_tooltip(
                app.handle(),
                active.as_ref().map(|a| a.model.as_str()),
                startup_tier,
            );

            // The configured shortcut, not the default one. Registering the
            // default here would ignore the field the user just set in Settings,
            // so their hotkey would work until they quit and then revert on every
            // launch — a bug that looks like the setting not saving at all.
            //
            // A shortcut conflict must not prevent startup. The tray icon is
            // still a working way in, and killing launch over a hotkey clash
            // would leave the user with no entry point at all.
            hotkey::register_all(app.handle(), &shortcut, &push_to_talk);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::get_appearance,
            commands::save_provider,
            commands::remove_provider,
            commands::set_active_model,
            commands::discover_models,
            commands::run_preflight,
            commands::set_theme,
            commands::set_show_thinking,
            commands::set_prompt_context,
            commands::set_hotkey,
            commands::get_voice,
            commands::set_speech_model,
            commands::set_voice_languages,
            commands::download_speech_model,
            commands::remove_speech_model,
            commands::get_capture,
            commands::clear_capture_log,
            commands::request_screen_recording,
            commands::test_capture,
            commands::open_permission_settings,
            commands::get_logs,
            commands::open_log_folder,
            commands::send_text_turn,
            commands::cancel_turn,
            commands::clear_session,
            commands::prompt_templates,
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
