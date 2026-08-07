# Changelog

All notable changes to Magi are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). See [`docs/VERSIONING.md`](docs/VERSIONING.md) for what the version number promises.

## [Unreleased]

Magi answers for the first time.

### Added
- Ask a question in the panel and watch the answer stream back, token by token
- Answers render as markdown — bold, lists, tables, and code blocks
- Providers are configurable from Settings: add, edit, and remove endpoints, with presets for Ollama, LM Studio, OpenAI, Anthropic, OpenRouter, and Xiaomi MiMo, plus any custom OpenAI-compatible URL
- Available models are discovered from the endpoint instead of typed in by hand
- Click a model to make it the active one
- Anthropic's own API is supported alongside the OpenAI-compatible shape
- API keys are stored in the OS keychain, never in `config.toml`, so a config file is safe to paste into a bug report. Settings shows a fingerprint such as `sk-p…4f2a`; the key itself is never sent to the interface
- `config.toml` in the OS config directory, with defaults written on first run, validation that says what to fix, and a schema version so future releases can migrate it
- Settings has a sidebar — Hotkeys, Models, General
- Light, Dark, and System themes
- The model's reasoning can be shown, off by default, under General
- `Escape`, Stop, or asking again cancels an answer in flight
- The global shortcut is configurable: click the shortcut in Settings → Hotkeys and press a new combination. One another application already owns is refused, and the old shortcut keeps working
- Standing context under Settings → General, sent with every question — where you are, what you work on, which units you think in. It adds to Magi's instructions and cannot replace them

### Fixed
- Cancelling an answer with `Escape` left the panel stuck: reopening it showed only a Stop button that did nothing, and typing was impossible
- The panel's corners showed a lighter patch where the translucent background did not follow the window's rounded shape
- A shortcut set in `config.toml` was ignored at launch: the default was registered instead, so a changed hotkey worked until Magi was quit and then reverted

### Known limitations
- No voice and no screen capture yet — this release is text only
- Nothing verifies that the chosen model can actually do what is asked of it; that is the next milestone
- Links in an answer are shown with their destination but are not clickable
- Combinations the operating system reserves for itself cannot be bound, and it does not report which application holds one

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
