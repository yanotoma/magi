//! Magi's composition root.
//!
//! `main.rs` only calls [`run`]. Everything the application is made of is wired
//! together here, so there is one place to read to understand what happens at
//! startup.

pub mod error;
pub mod hotkey;
pub mod tray;
pub mod windows;

use tauri::WindowEvent;

/// Builds and runs the Tauri application.
///
/// The `expect` at the end is permitted by the project's no-panic rule: it is in
/// the startup path, where a failure means the app genuinely cannot run and there
/// is no user-facing surface left to degrade into.
pub fn run() {
    init_tracing();

    tauri::Builder::default()
        .plugin(hotkey::plugin())
        .setup(|app| {
            // Magi is a background agent: no Dock icon, no app-switcher entry.
            // `skipTaskbar` in the window config does not do this on macOS — it
            // is a no-op there — so the activation policy is the only lever.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            tray::init(app)?;

            // A shortcut conflict must not prevent startup. The tray icon is
            // still a working way in, and killing launch over a hotkey clash
            // would leave the user with no entry point at all.
            if let Err(error) = hotkey::register_default(app.handle()) {
                tracing::warn!(%error, "continuing without a global shortcut");
            }

            Ok(())
        })
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
