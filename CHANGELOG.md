# Changelog

All notable changes to Magi are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). See [`docs/VERSIONING.md`](docs/VERSIONING.md) for what the version number promises.

## [Unreleased]

## [0.3.0-alpha.1] - 2026-08-08

### Added
- **Speak any language.** Magi detects it by default — Spanish, Portuguese, Japanese, about ninety-nine of them — and transcribes in the language you spoke rather than translating. Settings → Voice can pin one if you always use the same
- **Language shortlist.** If auto-detection gets a short utterance wrong — two seconds of Spanish came back as French at p=0.18 — you can tell Magi which languages you actually speak. Settings → Voice shows a checkbox list: leave it empty to detect from all ninety-nine, tick one to pin it, or tick a few to restrict detection to your choices
- **Talk to Magi.** Hold `Alt+Shift+Space`, say something, let go. It transcribes on your Mac and puts the words in the panel's input — you still decide whether to send them. Configurable, and separate from the panel shortcut so neither has to be timed
- **Local speech-to-text.** Settings → Voice picks a model — Base, Small or Medium — downloads it with a progress bar, and shows whether your microphone is available. Transcription runs entirely on your Mac; no audio is sent anywhere
- The microphone permission is shown as a live status row rather than discovered when a recording fails, with a button that opens the right System Settings pane
- Downloaded models can be deleted again

### Changed
- `voice.language` in `config.toml` is now `voice.languages` (a list of ISO 639-1 codes). Existing configs are migrated automatically on first launch — no edits needed
- macOS 11 (Big Sur) is now the minimum. The speech engine needs a newer system library than Tauri's default of macOS 10.13 allowed, and 11 is the first release that runs on Apple Silicon — which is what Magi is built and tested for

## [0.2.0-alpha.2] - 2026-08-07

Magi now checks what a model can do before relying on it.

### Added
- **Capability testing.** Settings → Models shows a matrix per provider: can Magi reach it, can the model read an image, does it call tools properly, does it return valid JSON. Press *Test* on any model to find out; press *Re-test* after changing anything
- Each model gets a capability label — *Agentic capture*, *Assisted capture*, *Text only* or *Unreachable* — with a plain-language explanation of what that means and what to do about it
- The tray tooltip names the active model and its capability, so a model that cannot see your screen says so without opening anything
- Answers are now tailored to what the model can do. A model that cannot see the screen is told so, and stops offering to look
- Results are remembered in `capabilities.json` next to `config.toml`, so testing happens once rather than on every launch. Deleting that file forces a re-test, and saving a provider clears its results — capabilities belong to the endpoint as much as to the model

### Changed
- Untested models are shown as untested rather than as unsupported. Nobody has asked them yet, which is not the same as a refusal
- Adding a provider is behind a button, so an empty form no longer occupies the Models screen
- Fetching a provider's models now proposes them instead of selecting them all. Pick the ones you want, with a search box for endpoints that serve hundreds
- Provider cards fold away, and show their model count when folded
- The Settings window opens larger, and model names no longer wrap
- Adding or editing a provider now fills the whole screen instead of sharing it with the provider list
- Consistent styling across the Settings screens: sections are separated by rules rather than some panes using bordered cards and others nothing
- The provider list is just names now. The endpoint and the API key fingerprint live in the edit form, where they can be changed; a provider missing a required key still says so
- Icon buttons for edit, remove and fold, and a `+` on Add a provider

## [0.2.0-alpha.1] - 2026-08-07

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

[Unreleased]: https://github.com/yanotoma/magi/compare/v0.3.0-alpha.1...HEAD
[0.3.0-alpha.1]: https://github.com/yanotoma/magi/compare/v0.2.0-alpha.2...v0.3.0-alpha.1
[0.2.0-alpha.2]: https://github.com/yanotoma/magi/compare/v0.2.0-alpha.1...v0.2.0-alpha.2
[0.2.0-alpha.1]: https://github.com/yanotoma/magi/compare/v0.1.0-alpha.1...v0.2.0-alpha.1
[0.1.0-alpha.1]: https://github.com/yanotoma/magi/releases/tag/v0.1.0-alpha.1
