# Changelog

All notable changes to Magi are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). See [`docs/VERSIONING.md`](docs/VERSIONING.md) for what the version number promises.

## [Unreleased]

## [0.1.0-alpha.1] - 2026-08-06

First build. The shell exists; there is nothing intelligent behind it yet.

### Added
- System tray icon with Open, Settings, and Quit
- Global hotkey (`Alt+Space`) toggling a transparent overlay panel
- Settings window (read-only placeholder)
- Runs as a macOS background agent: no Dock icon, no app-switcher entry
- Closing a window hides it instead of quitting
- Design specification, architecture documentation, and a full task breakdown
- Project-local AI assistant skills for Rust, Tauri v2, Svelte 5, LLM providers, and macOS permissions
- Versioning policy and release train
- CI on macOS: `cargo fmt`, `clippy -D warnings`, `cargo test`, `svelte-check`, and a release build

### Known limitations
- macOS only
- No audio, no screen capture, no model integration
- The hotkey is not configurable
- The tray icon is Tauri's default and does not yet change with state
- Requires Accessibility permission for the global shortcut
- Not signed or notarized — there is no downloadable build

[Unreleased]: https://github.com/yanotoma/magi/compare/v0.1.0-alpha.1...HEAD
[0.1.0-alpha.1]: https://github.com/yanotoma/magi/releases/tag/v0.1.0-alpha.1
