# Tasks

Complete breakdown of what is done and what is pending, across every milestone.

**Last updated:** 2026-08-06
**Current phase:** M2 — the text loop works end to end; manual verification pending
**Current version:** `0.1.0-alpha.1` (released — see [VERSIONING.md](VERSIONING.md))
**Overall:** 53 / 142 tasks done (37%)

Legend: `[x]` done · `[ ]` pending · `[~]` in progress · `[!]` blocked

---

## Status at a glance

| Milestone | Scope | Target version | Done | Total | Status |
|---|---|---|---:|---:|---|
| **M0** | Foundations | — | 15 | 15 | ✅ Complete |
| **M1** | Shell — tray, hotkey, windows | `0.1.0-alpha.1` | 15 | 16 | ✅ Shipped |
| **M2** | Config & providers | `0.2.0-alpha.1` | 23 | 26 | 🔨 In progress |
| **M3** | Pre-flight & capability tiers | `0.2.0-alpha.1` | 0 | 14 | ⬜ |
| **M4** | Audio & speech-to-text | `0.3.0-alpha.1` | 0 | 14 | ⬜ |
| **M5** | Screen capture & agentic vision | `0.4.0-alpha.1` | 0 | 13 | ⬜ |
| **M6** | Session machine & panel UX | `0.5.0-beta.1` | 0 | 16 | ⬜ |
| **M7** | Packaging & macOS release | `0.6.0-beta.1` | 0 | 14 | ⬜ |
| — | **v1 total** | `1.0.0` | **53** | **128** | |
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

## M1 — Shell ✅

The tray app exists, the hotkey works, the windows appear. No intelligence yet.

Manually verified on macOS: tray icon present, no Dock or app-switcher entry, `Alt+Space` toggles the panel once per press, the panel is transparent and undecorated, right-click opens the menu, and closing Settings does not quit the app.

One task is carried into M3/M4/M6 rather than done here — see the note on the tray icon below.

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
- [ ] Tray icon reflects session state (idle / listening / thinking) and capability tier — the mark, the generator, and the state-to-icon mapping exist and are tested; live updating waits for the states themselves, which arrive in M3/M4/M6
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

Ends with a working text loop: type in the panel, watch the answer stream back. No voice, no capture, no state machine.

The milestone was widened from pure infrastructure for one reason: with the original plan nothing worked until M6, and streaming into the UI is where the surprises live — split SSE frames, backpressure, cancellation. Finding those against a local model in M2 costs hours; finding them in M6 under four other subsystems costs days of working out which one is at fault.

- [x] `config.rs` — TOML schema with `serde`, loaded from the OS config dir
- [x] Config validation with actionable error messages, not just parse failures
- [x] Config versioning and a migration path (needed before the first public release, not after)
- [x] Write defaults on first run
- [x] Store and retrieve API keys via `keyring`, never in the TOML
- [ ] Optional OAuth sign-in for Anthropic as an alternative to a pasted key (`Authorization: Bearer` + `anthropic-beta: oauth-2025-04-20`, short-lived tokens, refresh handling). Still billed as API usage — it removes key-pasting friction, not cost
- [x] Provider registry — resolves a config to an implementation by protocol. Keyed by `kind`, not by vendor: Xiaomi serves both protocols on one host, so a vendor-keyed registry would have to pick one
- [x] Model discovery via `GET /v1/models`, so adding a provider does not mean typing model names by hand
- [x] Model picker in Settings — clicking a model chip activates it. A picker inside the panel waits for M6, when there is a reason to switch mid-conversation
- [x] Built-in presets for Ollama, LM Studio, OpenAI, Anthropic, OpenRouter, Xiaomi MiMo
- [x] Custom OpenAI-compatible endpoint as a first-class option (base URL + models + optional key) — a missing preset must never be a wall. Any endpoint speaking the OpenAI chat-completions shape works; pre-flight reports what it can actually do
- [x] `llm::provider` — the `Provider` trait; internal turn types are provider-neutral
- [x] `llm::openai` — OpenAI-compatible impl (Ollama, LM Studio, OpenAI, OpenRouter, MiMo)
- [x] `llm::anthropic` — Anthropic native impl (x-api-key + anthropic-version, top-level system, named SSE events)
- [x] Server-sent-event streaming with incremental token emission
- [x] Defensive SSE parsing — local backends split frames and omit terminators
- [x] `FakeProvider` for tests, replaying scripted turns (tool calls join it in M5)
- [x] SSE parsing as a pure function, every body replayed at hostile chunk sizes (one byte at a time as well as whole) so the split-frame path is exercised rather than assumed
- [x] Tauri commands `send_text_turn`, `cancel_turn`, config commands; events `magi://token`, `magi://turn-done`, `magi://error`
- [x] Cancellation — dismissing the panel, pressing Stop, or asking again aborts the in-flight task, which drops the receiver and stops the provider
- [x] Panel: text input and streaming answer, growing to a max height then scrolling
- [x] Inline errors naming the provider and the resolved URL — "connection refused" is useless without knowing what it tried to reach
- [x] Settings UI — provider list with add / edit / remove
- [x] Render answers as markdown — bold, lists, tables, code. Models emit markdown whether or not it is asked for, so plain text does not mean "no formatting", it means showing `**weather.com**` with the asterisks. Rendered with raw HTML disabled at the parser rather than sanitised afterwards, images disabled (a model-chosen image URL is a read-receipt beacon), and links rendered as non-navigable text next to their real destination — this panel is the app's own webview, so following a link would replace Magi's UI with no way back
- [ ] Settings UI — hotkey capture control
- [ ] `[prompt] context` in `config.toml` — free text appended to Magi's system prompt. **Additive, never a replacement**: Magi's own instructions carry the contract that makes agentic capture fire, and letting a user overwrite them breaks tier 1 silently

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
- [ ] `llm::prompt` — assemble messages from `(tier, config, history)` rather than from a constant. The prompt is tier-dependent: tier 1 needs instructions on when to call `capture_screen`; tier 2 must not be told about tools at all, since the harness captures ahead of it by heuristic and mentioning tools only invites malformed tool syntax in prose; tier 3 needs to know it cannot see the screen so it stops promising to look
- [ ] Unit tests for prompt assembly per tier — pure logic, no network
- [ ] Design the degraded tray icon. A cancel slash across three separated nodes does not read at 22pt — the mark is discontinuous, so the bar alternates between empty space and ring and every crossing forces a choice between eating the ring and breaking the bar. Needs a different idea, looked at in a real menu bar

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
- [ ] Prompt templates: pre-written user prompts bound to a trigger ("explain this error", "summarise this screen"). Distinct from the system prompt — these are user turns, not instructions, and they belong in the panel UI rather than in the prompt assembler

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

- [x] CI: `cargo test`, `cargo clippy -D warnings`, `cargo fmt --check`, `svelte-check`
- [ ] Consider extracting a `magi-core` crate with no Tauri dependency. Magi's interesting logic — config parsing and migration, tier assignment, prompt assembly, deictic detection, token-stream parsing — touches no platform API, but it currently lives in a crate that depends on `tauri`, so testing it drags in a whole platform. A core crate would let those tests run anywhere in seconds and would make the cross-platform claim structural rather than aspirational. Not worth it at M1's 300 lines of logic; revisit when `llm/`, `config/`, and `session/` land in M2
- [x] Split CI by what actually needs macOS. Everything currently runs on `macos-14`, but `cargo fmt`, `svelte-check`, and the task-count check are platform-independent — and macOS runners bill at a 10x minute multiplier and are markedly harder to get allocated (observed: two consecutive runs lost, one to a service outage and one to "job was not acquired by Runner of type hosted"). Move the platform-independent checks to `ubuntu-latest` and leave macOS for the build. Caveat: `cargo test` on Linux needs Tauri's webkit2gtk system dependencies, which is real work rather than a config line — but it would also prove the cross-platform claim is not vapour
- [ ] Keep all tests free of GPU, microphone, display, and network dependencies
- [ ] Set a Content-Security-Policy. `app.security.csp` is currently `null`, so Tauri injects none. The panel renders model output as HTML, and while the markdown renderer is configured so it cannot emit tags, that is one layer: a CSP is what makes an escape from that layer inert instead of exploitable. Needs care rather than a one-line change — Svelte's scoped styles and markdown-it's table alignment both use inline `style`, so `style-src` has to allow them, and the dev server's HMR needs its own origin allowed
- [ ] Windows packaging
- [ ] Linux packaging
- [ ] Issue and PR templates
- [ ] Community wake-word model contributions
