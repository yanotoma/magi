//! Magi's composition root.
//!
//! `main.rs` only calls [`run`]. Everything the application is made of is wired
//! together here, so there is one place to read to understand what the app does
//! at startup.

/// Builds and runs the Tauri application.
///
/// The `expect` here is permitted by the project's no-panic rule: it is in the
/// startup path, where a failure means the app genuinely cannot run and there is
/// no user-facing surface left to degrade into.
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("fatal: Tauri failed to start");
}
