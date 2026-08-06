---
name: rust
description: Use when writing or reviewing any Rust code in Magi — async orchestration, error handling, the crate layout, or the audio/vision/LLM modules. Encodes this project's Rust conventions and the specific crates chosen for each capability.
---

# Rust in Magi

Magi's core is Rust. The maintainer is new to Rust, so **clarity beats cleverness** in every tradeoff. No lifetime gymnastics, no trait-level metaprogramming, no macro DSLs unless a reviewer can read them without a detour.

## Toolchain

Pinned in `rust-toolchain.toml` at the repo root. `rustup` honors it automatically — contributors never need to run a version-switch command. Never bypass the pin.

## Crate map — one crate per capability

| Capability | Crate | Notes |
|---|---|---|
| Screen capture | `xcap` | Cross-platform, captures screens **and** individual windows |
| Audio capture | `cpal` | Low-level; expect to handle sample rate conversion to 16 kHz mono for Whisper yourself |
| STT | `whisper-rs` | Bindings to whisper.cpp. Needs cmake at build time. Metal on macOS |
| Wake word (v2) | `ort` | ONNX Runtime bindings, runs openWakeWord models |
| Mouse/keyboard (v3) | `enigo` | Needs macOS Accessibility permission |
| HTTP / LLM | `reqwest` + `tokio` | A `Provider` trait with two impls. "OpenAI-compatible" is a family, not a standard — Anthropic is outside it. See the `llm-providers` skill |
| Errors | `thiserror` (libraries) + `anyhow` (top level) | See below |
| Logging | `tracing` + `tracing-subscriber` | Structured; never `println!` in committed code |
| Config | `serde` + `toml` | Config lives in the OS config dir, human-editable |

Adding a new dependency is a design decision, not a detail. Justify it in the PR.

## Error handling

Two layers, and the distinction is not cosmetic:

- **Module boundaries** use `thiserror` enums, so callers can match on the failure and react. A failed screen capture because permission was denied must be distinguishable from a failed capture because no display was found — the first shows a permissions prompt, the second is a bug report.
- **Top level** (Tauri commands, `main`) uses `anyhow::Result` for context chaining.

```rust
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("screen recording permission not granted")]
    PermissionDenied,
    #[error("no display found at index {0}")]
    NoSuchDisplay(usize),
    #[error(transparent)]
    Backend(#[from] xcap::XCapError),
}
```

**Never `unwrap()` or `expect()` outside of tests and `main`'s setup.** A panic in a background tray app is invisible to the user — the app just stops responding to the hotkey with no error anywhere. This is the worst failure mode Magi has, so it is a hard rule.

## Async

`tokio` via Tauri's runtime. Do not spawn a second runtime.

The rule that matters: **never block the Tauri main thread.** Whisper inference, screen capture, and HTTP all move to `tokio::task::spawn_blocking` (for CPU-bound) or `tauri::async_runtime::spawn` (for async I/O). Whisper inference is CPU-bound and long — it always goes to `spawn_blocking`.

Long operations stream progress back to the UI with `app.emit("magi://...", payload)` rather than returning one big result at the end.

## Module layout

```
src-tauri/src/
├── main.rs          # three-line entry point: calls magi_lib::run()
├── lib.rs           # composition root: builder, plugins, tray, shortcut registration
├── config.rs        # load/save/validate user config
├── capture/         # screen + window capture (xcap)
├── audio/           # cpal input, resampling, VAD
├── stt/             # whisper-rs wrapper
├── llm/
│   ├── provider.rs  # the Provider trait + registry
│   ├── openai.rs    # OpenAI-compatible impl
│   ├── anthropic.rs # Anthropic native impl
│   ├── preflight.rs # capability probing -> tier assignment
│   └── tools.rs     # tool definitions incl. capture_screen
└── session.rs       # conversation state machine
```

If `lib.rs` grows past ~200 lines, something belongs in a module.

**Why the `main.rs` / `lib.rs` split.** The Tauri template ships it for mobile
support, which Magi will never target — but it is kept for a different reason:
integration tests under `src-tauri/tests/` cannot import from a binary-only
crate. Read `lib.rs` to understand what the app does at startup; `main.rs` has
nothing in it.

## Testing

- Pure logic (config parsing, tier assignment, prompt assembly, deictic detection) gets unit tests. This is most of the interesting code.
- Anything touching hardware or the network sits behind a trait so tests use a fake. `Provider`, `ScreenCapture`, and `Transcriber` are traits for exactly this reason.
- No test may require a GPU, a microphone, a display, or a network connection. CI has none of them.

## Style

- `cargo fmt` and `cargo clippy -- -D warnings` are enforced in CI. Run both before pushing.
- Prefer `?` over `match` on `Result` when there is nothing to add.
- Public items in library modules get doc comments explaining *why*, not *what*.
