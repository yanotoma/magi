# Tasks

Complete breakdown of what is done and what is pending, across every milestone.

**Last updated:** 2026-08-06
**Current phase:** M1 code complete; manual verification pending before tagging `0.1.0-alpha.1`
**Current version:** `0.1.0-alpha.1` (unreleased — see [VERSIONING.md](VERSIONING.md))
**Overall:** 30 / 130 tasks done (23%)

Legend: `[x]` done · `[ ]` pending · `[~]` in progress · `[!]` blocked

---

## Status at a glance

| Milestone | Scope | Target version | Done | Total | Status |
|---|---|---|---:|---:|---|
| **M0** | Foundations | — | 15 | 15 | ✅ Complete |
| **M1** | Shell — tray, hotkey, windows | `0.1.0-alpha.1` | 15 | 16 | 🔨 Awaiting manual verification |
| **M2** | Config & providers | `0.2.0-alpha.1` | 0 | 17 | ⬜ |
| **M3** | Pre-flight & capability tiers | `0.2.0-alpha.1` | 0 | 11 | ⬜ |
| **M4** | Audio & speech-to-text | `0.3.0-alpha.1` | 0 | 14 | ⬜ |
| **M5** | Screen capture & agentic vision | `0.4.0-alpha.1` | 0 | 13 | ⬜ |
| **M6** | Session machine & panel UX | `0.5.0-beta.1` | 0 | 15 | ⬜ |
| **M7** | Packaging & macOS release | `0.6.0-beta.1` | 0 | 15 | ⬜ |
| — | **v1 total** | `1.0.0` | **30** | **116** | |
| **M8** | v2 — wake word & TTS | `1.1.0` | 0 | 9 | 🔮 Post-v1 |
| **M9** | v3 — computer use | `1.2.0` | 0 | 5 | 🔮 Post-v1 |

---

## M0 — Foundations ✅

- [x] Choose the MVP vertical slice (push-to-talk + screen → threaded text panel)
- [x] Evaluate and select the tech stack (full Rust / Tauri v2 over Python and Electron alternatives)
- [x] Decide the vision policy (agentic tool-based capture with three-tier degradation)
- [x] Decide platform priority (macOS first)
- [x] Verify Tauri v2 sidecar, global-shortcut, and tray APIs against current docs
- [x] Write the design specification
- [x] Write `docs/ARCHITECTURE.md`
- [x] Write `README.md`
- [x] Write `docs/TASKS.md` (this file)
- [x] Write `CONTRIBUTING.md`
- [x] Create project-local Claude skills (`rust`, `tauri-v2`, `svelte-5`)
- [x] Choose license (Apache-2.0, for the explicit patent grant)
- [x] Define the versioning policy and release train (`docs/VERSIONING.md`, `CHANGELOG.md`)
- [x] Correct the provider architecture: `Provider` trait with separate OpenAI-compatible and Anthropic-native implementations (they are different wire protocols, not one protocol with quirks)
- [x] Add project-local skills for LLM providers and macOS permissions

---

## M1 — Shell

The tray app exists, the hotkey works, the windows appear. No intelligence yet.

**Setup**
- [x] Install `rustup` and pin the toolchain in `rust-toolchain.toml`
- [x] Scaffold the Tauri v2 project (the `svelte-ts` template is SvelteKit + Svelte 5; kept, so each window loads its own route)
- [x] Configure `.gitignore` for Rust, Node, SvelteKit, and Tauri build artifacts
- [x] Set up `cargo fmt` and `cargo clippy -- -D warnings` in CI
- [x] Set the version to `0.1.0-alpha.1` in `package.json`, with `tauri.conf.json > version` pointing at `"../package.json"` so there is one source of truth
- [x] CI check asserting `Cargo.toml` and `package.json` versions agree

**Tray**
- [x] `tray.rs` — tray icon with menu (Open, Settings, Quit) using `TrayIconBuilder`
- [x] Left click opens the panel; right click opens the menu
- [ ] Tray icon reflects session state (idle / listening / thinking) and capability tier — the state-to-icon mapping is written and unit-tested; the icon assets and live updating are not, and the states it maps do not exist until M3/M4/M6
- [x] macOS: set activation policy to `Accessory` so no Dock icon appears

**Hotkey**
- [x] `hotkey.rs` — register the global shortcut via `tauri-plugin-global-shortcut`
- [x] Filter press vs release (the handler fires for both — a known footgun)
- [x] Detect and surface registration conflicts instead of failing silently
- [x] Add the required permissions to `capabilities/default.json`

**Windows**
- [x] `windows.rs` — panel window: transparent, undecorated, always-on-top, hidden by default (`skipTaskbar` is a no-op on macOS; the Dock icon is suppressed by the activation policy instead)
- [x] Settings window: normal decorated window; close hides rather than exits

---

## M2 — Config & providers

- [ ] `config.rs` — TOML schema with `serde`, loaded from the OS config dir
- [ ] Config validation with actionable error messages, not just parse failures
- [ ] Config versioning and a migration path (needed before the first public release, not after)
- [ ] Write defaults on first run
- [ ] Store and retrieve API keys via `keyring`, never in the TOML
- [ ] Optional OAuth sign-in for Anthropic as an alternative to a pasted key (`Authorization: Bearer` + `anthropic-beta: oauth-2025-04-20`, short-lived tokens, refresh handling). Still billed as API usage — it removes key-pasting friction, not cost
- [ ] Provider registry — id, kind, base URL, model, resolved tier
- [ ] Built-in presets for Ollama, LM Studio, OpenAI, Anthropic, OpenRouter
- [ ] Custom OpenAI-compatible endpoint as a first-class option (base URL + model + optional key) — a missing preset must never be a wall. Any endpoint speaking the OpenAI chat-completions shape works; pre-flight reports what it can actually do
- [ ] `llm::provider` — the `Provider` trait; internal turn types are provider-neutral
- [ ] `llm::openai` — OpenAI-compatible impl (Ollama, LM Studio, OpenAI, OpenRouter)
- [ ] `llm::anthropic` — Anthropic native impl (x-api-key, top-level system, input_schema, base64 image source)
- [ ] Server-sent-event streaming with incremental token emission
- [ ] Defensive SSE parsing — local backends split frames and omit terminators
- [ ] `FakeProvider` for tests, replaying scripted turns including tool calls
- [ ] Settings UI — provider list with add / edit / remove
- [ ] Settings UI — hotkey capture control

---

## M3 — Pre-flight & capability tiers

- [ ] `llm::preflight` module scaffolding
- [ ] Probe 1 — reachability, distinguishing bad URL / bad key / model not pulled
- [ ] Probe 2 — vision, using a generated image with a known digit
- [ ] Probe 3 — tool-calling, validating a well-formed call rather than non-empty output
- [ ] Probe 4 — structured output against a small JSON schema
- [ ] Tier assignment logic from probe results
- [ ] Cache results per provider + model
- [ ] Unit tests for tier assignment across every probe-result combination
- [ ] Settings UI — capability matrix per provider
- [ ] Settings UI — *Re-test* button with progress
- [ ] Surface the active tier in the tray tooltip

---

## M4 — Audio & speech-to-text

**Capture**
- [ ] `audio` module — enumerate input devices via `cpal`
- [ ] Open the default input and buffer PCM while recording
- [ ] Resample to 16 kHz mono (Whisper's required input format)
- [ ] Cap recording length and handle the buffer-full case
- [ ] `AudioSource` trait plus a fake that replays a fixture WAV
- [ ] Handle device disconnect mid-recording

**Transcription**
- [ ] `stt` module — `whisper-rs` integration
- [ ] Verify the cmake build works on Apple Silicon and Intel
- [ ] Enable Metal acceleration on macOS
- [ ] First-run model download with progress, resumable, checksum-verified
- [ ] Model selection in Settings (`base.en` default, `small`, `medium`)
- [ ] Run inference on `spawn_blocking` — it is CPU-bound and long
- [ ] `Transcriber` trait plus a fake
- [ ] Microphone permission request and denial handling

---

## M5 — Screen capture & agentic vision

- [ ] `capture` module — enumerate displays and windows via `xcap`
- [ ] Capture the active display as PNG bytes
- [ ] Capture a specific window
- [ ] Downscale before encoding — vision token cost scales with resolution
- [ ] `ScreenCapture` trait plus a fake returning a fixture image
- [ ] Screen Recording permission handling, including the restart-required path
- [ ] `llm::tools` — define the `capture_screen` tool schema
- [ ] Tool-call execution loop: `tool_use` → capture → resend with image
- [ ] Guard against capture loops (cap calls per turn)
- [ ] Tier 2 deictic heuristic ("here", "this", "this error", "this screen", …)
- [ ] Unit tests for deictic detection, including negative cases
- [ ] Emit `magi://captured` so the panel can show a capture indicator
- [ ] Capture audit log, visible in Settings

---

## M6 — Session machine & panel UX

**Rust**
- [ ] `session.rs` — the state machine (Idle → Listening → Transcribing → Thinking → Capturing → Streaming → Idle)
- [ ] Conversation thread held in memory; discarded on dismiss
- [ ] History assembly with a token budget and truncation strategy
- [ ] Tauri commands: `toggle_session`, `send_text_turn`, `dismiss_session`
- [ ] Emit `magi://state`, `magi://token`, `magi://error`
- [ ] Cancellation — dismissing mid-stream must actually abort the request
- [ ] Unit tests for every state transition, including error paths

**Svelte**
- [ ] `conversation.svelte.ts` — shared rune state
- [ ] Panel — thread view with per-turn roles
- [ ] Panel — token-by-token streaming render
- [ ] Panel — status indicator per state, including a distinct capture indicator
- [ ] Panel — text input for typed follow-ups
- [ ] Panel — Esc dismisses; click-outside behavior
- [ ] Panel — inline error surfaces per failure class
- [ ] Panel — markdown and code-block rendering with syntax highlighting

---

## M7 — Packaging & macOS release

- [ ] Configure `tauri.conf.json` bundle settings and app identifier
- [ ] App icon set, all required sizes
- [ ] `Info.plist` usage descriptions for Microphone and Screen Recording
- [ ] Enable `macOSPrivateApi` for window transparency (documented tradeoff: blocks App Store)
- [ ] Universal binary (aarch64 + x86_64)
- [ ] Code signing with a Developer ID certificate
- [ ] Notarization and stapling
- [ ] DMG with a drag-to-Applications layout
- [ ] Verify how each bundle format handles pre-release identifiers (`-alpha.1`); adopt a monotonic build number if they are dropped
- [ ] GitHub Actions release workflow triggered by `v*` tags, marking pre-release versions as GitHub pre-releases
- [ ] Keep `CHANGELOG.md` current — every user-visible change lands in `Unreleased` in the same PR
- [ ] Auto-update via `tauri-plugin-updater`
- [ ] First-run onboarding: permissions walkthrough, model choice, provider setup
- [ ] Measure and publish idle RAM and CPU (the headline claim needs a number behind it)

---

## M8 — v2: Ambient 🔮

- [ ] Wake word via `ort` + openWakeWord ONNX models
- [ ] Always-listening ring buffer with a passive-cost budget
- [ ] False-positive tuning and a sensitivity control
- [ ] Wake word feeds the same `session` state machine as the hotkey
- [ ] Bundle Piper as a Tauri sidecar
- [ ] Sentence-boundary chunking so TTS can start before the response completes
- [ ] Voice selection and speed control in Settings
- [ ] Barge-in — speaking interrupts playback
- [ ] Opt-in conversation persistence with a retention policy

---

## M9 — v3: Agency 🔮

- [ ] `enigo` behind an explicit per-session permission grant
- [ ] Accessibility permission handling
- [ ] Dry-run mode showing intended actions before execution
- [ ] Hard kill switch — global panic hotkey
- [ ] Coordinate-space handling across Retina and multi-display setups

---

## Skills roadmap

Project-local skills exist so future sessions don't re-derive version-specific traps. Written when the milestone that needs them starts — a skill written early goes stale before it is used.

| Skill | Status | Covers |
|---|---|---|
| `rust` | ✅ | Conventions, crate map, error handling, threading |
| `tauri-v2` | ✅ | v1/v2 API split, capabilities, tray, sidecars, overlay windows |
| `svelte-5` | ✅ | Runes vs Svelte 4, `.svelte.ts`, Tauri IPC import paths |
| `llm-providers` | ✅ | Provider protocol families, vision/tool/stream formats, pre-flight probes |
| `macos-permissions` | ✅ | TCC, Info.plist, signing, notarization, sidecar signing |
| `audio-stt` | M4 | cpal device handling, resampling, whisper-rs build and Metal |
| `screen-capture` | M5 | xcap, multi-display, downscaling for vision cost |
| `release` | M7 | Tag → universal build → sign → notarize → staple → GitHub release → updater |
| `wake-word` | M8 | openWakeWord ONNX via `ort`, false-positive tuning |
| `tts` | M8 | Piper sidecar, sentence chunking, barge-in |
| `computer-use` | M9 | enigo, coordinate spaces, Retina and multi-display |

A changelog skill was considered and rejected: the convention is three rules that already live in `VERSIONING.md` and `CLAUDE.md`, and a third copy would drift. It is a step inside the `release` skill, not a skill of its own.

## Rejected approaches

Recorded so they are not re-proposed.

| Idea | Why not |
|---|---|
| Use a Claude Pro/Max subscription instead of an API key | The subscription covers claude.ai and Claude Code, not third-party API clients. Reusing their stored credentials means reverse-engineering a private auth flow, violates the terms, and breaks without warning. |
| Drive Claude Code (`claude -p`) as a provider subprocess | It is an agentic coding tool, not a completions endpoint — wrong latency profile, wrong interface, and its tool-calling does not map onto `capture_screen`. Building on another client's auth is structurally fragile. |
| One OpenAI-compatible client covering every provider | Anthropic diverges on auth header, system prompt placement, tool schema, image encoding, and required fields. Five differences is a separate implementation, not a quirks flag. |

## Cross-cutting, ongoing

- [ ] CI: `cargo test`, `cargo clippy -D warnings`, `cargo fmt --check`, `svelte-check`
- [ ] Keep all tests free of GPU, microphone, display, and network dependencies
- [ ] Windows packaging
- [ ] Linux packaging
- [ ] Issue and PR templates
- [ ] Community wake-word model contributions
