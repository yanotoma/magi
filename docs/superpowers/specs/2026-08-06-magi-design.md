# Magi — Design Specification

**Date:** 2026-08-06
**Status:** Draft, pending maintainer review
**Scope:** v1 vertical slice. Later milestones appear only as extension points.

---

## 1. What Magi is

Magi is a background desktop agent that answers questions about what you are doing, using what it can see on your screen and what you say out loud. It lives in the system tray, wakes on a global hotkey, and gets out of the way.

Named after the three-way supercomputer from *Neon Genesis Evangelion*.

**Design commitments, in priority order:**

1. **Privacy by construction.** Nothing leaves the machine unless the user configured a remote provider. The agent captures the screen only when it decides it needs to, and every capture is logged.
2. **Negligible passive cost.** Idle RAM and CPU are the metrics that matter for something that runs all day.
3. **Model-agnostic.** Local (Ollama, LM Studio) and remote (OpenAI, Anthropic, OpenRouter) are equal citizens. Magi adapts to what a model can do rather than demanding a specific one.
4. **Installable by a non-developer.** One download, one drag, one permission prompt.

**Non-goals:** a chat app, a coding assistant, a general automation platform, anything requiring an account.

---

## 2. Technology stack

Full Rust core with a Tauri v2 shell.

| Layer | Choice |
|---|---|
| Desktop shell, tray, overlay | Tauri v2 (`tray-icon`, `global-shortcut`, `shell` plugins) |
| Frontend | Svelte 5 + TypeScript + Vite |
| Screen capture | `xcap` |
| Audio capture | `cpal` |
| Speech-to-text | `whisper-rs` (whisper.cpp bindings) |
| LLM transport | `reqwest` + `tokio`; a `Provider` trait with two implementations |
| Secrets | `keyring` (OS keychain) |
| Config | `serde` + `toml` |
| Logging | `tracing` |
| Wake word (v2) | openWakeWord ONNX models via `ort` |
| Text-to-speech (v2) | Piper, bundled as a Tauri sidecar |
| Mouse/keyboard (v3) | `enigo` |

### Why not Python

The obvious alternative was a Tauri shell driving a Python engine, since the ML ecosystem is nominally Python-first. Examined closely, it is not: `faster-whisper` wraps CTranslate2 (C++), openWakeWord is ONNX running on onnxruntime (C++), Piper is a C++ binary. Python contributes glue, not computation.

Paying for that glue costs a 250–400 MB installer and roughly 200 MB of resident memory while idle — precisely the metric that matters most for a process that runs all day. Rust reaches the same native runtimes with a ~15 MB shell and tens of megabytes resident.

Pure Python (PySide6) was rejected additionally because the GIL puts the always-listening audio thread in direct contention with the UI thread; that is structural, not tunable. Electron was rejected for a ~150 MB baseline.

**Accepted cost:** the maintainer is new to Rust. Mitigated by keeping the Rust surface thin and conventional, and by the project-local skills in `.claude/skills/`.

---

## 3. Platform scope

**v1 ships macOS only.** The architecture is cross-platform from day one — every chosen crate supports all three targets — but CI, packaging, and permission handling are macOS-only until the slice is proven.

macOS is deliberately first because it is the hardest: it requires three separate TCC permissions (Microphone, Screen Recording, Accessibility) plus notarization. A permission model that survives macOS ports downward easily.

---

## 4. The v1 interaction

```
tap Alt+Space
   ↓
panel appears, mic opens              ← toggle, not hold
   ↓
user speaks; tap again to stop
   ↓
whisper.cpp transcribes locally
   ↓
LLM turn — may call capture_screen()
   ↓
response streams into the panel thread
   ↓
user types or speaks a follow-up, or dismisses with Esc
```

The panel is a transparent, undecorated, always-on-top window holding a conversation thread. It supports voice and typed follow-ups. Dismissing it ends the thread.

**Explicitly out of v1:** wake word, TTS, computer use, conversation history across sessions.

---

## 5. Agentic vision

This is the central design decision and the one that most distinguishes Magi.

`capture_screen` is **not a fixed pipeline stage**. It is a tool exposed to the model. The model decides when it needs to look.

```
"what is a mutex?"        → no tool call → no capture, no vision tokens
"what's happening here?"  → tool call    → capture → answer
```

### Why this over capturing every turn

A 1512×982 screenshot costs roughly 1,100 vision tokens on Claude and 1,400 on GPT-4o. Because conversation history is resent on every request, attaching one image per turn makes cost grow **quadratically** with thread length. On local models the same problem appears as prefill latency, several hundred milliseconds per image per turn.

Agentic capture inverts the tradeoff: an extra round-trip (~300–800 ms) is paid **only on turns that actually need vision**, instead of paying image cost on turns that do not.

It also produces an honest privacy story. The screen is read at specific, logged, model-initiated moments — not continuously.

### Capability tiers

Not every model can do this. Tool-calling reliability varies enormously, and small local models frequently ignore or malform tool syntax. The pre-flight check assigns each configured model a tier, and Magi degrades accordingly:

| Tier | Model capability | Vision behavior |
|---|---|---|
| **1** | Vision + reliable tool-calling | Agentic capture. The intended experience. |
| **2** | Vision, unreliable tool-calling | Local deictic heuristic ("here", "this", "this error", "this screen") triggers capture before the request is sent. |
| **3** | No vision | Capture disabled. Tray icon and Settings both indicate the limitation. |

Tier assignment is automatic. The user never configures it by hand.

The Tier 2 heuristic is a deliberately dumb keyword-and-pattern matcher over the transcript, not a model call. It is allowed to over-trigger — a spurious capture costs tokens, a missed one costs a wrong answer.

---

## 6. Pre-flight check

Because tier assignment drives real behavior, pre-flight is infrastructure, not a convenience feature. It runs when a model is first configured and whenever the user clicks *Re-test*.

Four probes against the configured endpoint:

1. **Reachability** — `GET /v1/models`, or a minimal completion for endpoints that lack it. Distinguishes wrong URL, wrong key, and model-not-pulled.
2. **Vision** — send a tiny generated image (a solid-color square with a known digit) and ask what it shows. Verifies genuine image handling rather than a silent accept-and-ignore.
3. **Tool-calling** — offer one trivial tool and a prompt that unambiguously requires it. Verifies a well-formed call, not just non-empty output.
4. **Structured output** — request a small JSON schema and validate the response parses.

Results are cached per provider+model and shown as an explicit capability matrix in Settings, so a user who wonders why screen reading is off has a direct answer.

The probes deliberately use throwaway inputs. Pre-flight never sends real screen contents.

---

## 7. Architecture

```
┌──────────────────────── Tauri v2 shell (Rust) ────────────────────────┐
│                                                                        │
│  tray.rs        icon, menu, tier indicator                             │
│  hotkey.rs      global shortcut, press/release filtering               │
│  windows.rs     panel (transparent overlay) + settings window          │
│                                                                        │
│  ┌── session.rs ── conversation state machine ──────────────────────┐  │
│  │   Idle → Listening → Transcribing → Thinking → Streaming → Idle  │  │
│  └────────┬──────────┬──────────┬───────────┬──────────────────────┘  │
│           │          │          │           │                          │
│      audio/     stt/        capture/      llm/                         │
│      cpal       whisper-rs  xcap          provider · openai            │
│                                           anthropic · preflight        │
│                                           tools                        │
│                                                                        │
│  config.rs      TOML in OS config dir · secrets in OS keychain         │
└────────────────────────────────┬───────────────────────────────────────┘
                                 │  invoke() / emit()
┌────────────────────────────────┴───────────────────────────────────────┐
│                    Svelte 5 frontend (TypeScript)                       │
│   panel: thread view, streaming tokens, input box, capture indicator    │
│   settings: providers, hotkey, capability matrix, permissions status    │
└─────────────────────────────────────────────────────────────────────────┘
```

### Module contracts

Each module is defined by a trait so it can be faked in tests and swapped later.

| Module | Responsibility | Trait |
|---|---|---|
| `audio` | Open the default input, buffer 16 kHz mono PCM while recording | `AudioSource` |
| `stt` | PCM in, text out | `Transcriber` |
| `capture` | Capture a display or window as PNG bytes | `ScreenCapture` |
| `llm::provider` | One turn in, streamed tokens plus optional tool calls out | `Provider` |
| `llm::preflight` | Endpoint config in, capability tier out | — |
| `session` | Owns the state machine and the thread; the only stateful module | — |
| `config` | Load, validate, persist. Secrets via keychain, never in the TOML | — |

`session` is the only module that knows about the others. Everything else is a leaf with no knowledge of the application. This is what keeps each piece testable without a display, a microphone, or a network.

### Data flow for one turn

```
hotkey press
  → session: Idle → Listening
  → audio.start()                              [spawn_blocking]
hotkey press again
  → audio.stop() → PCM buffer
  → session: Listening → Transcribing
  → stt.transcribe(pcm)                        [spawn_blocking, CPU-bound]
  → session: Transcribing → Thinking
  → llm.turn(history + user_text, tools=[capture_screen])
       ├─ tool_use(capture_screen) → capture.grab() → resend with image
       └─ text → emit "magi://token" per chunk  [async]
  → session: Streaming → Idle
```

Whisper inference and screen capture are CPU-bound and always run on `spawn_blocking`. Blocking the Tauri main thread freezes the tray and the hotkey, which in a background app looks identical to a crash.

---

## 8. Configuration and secrets

Config is human-editable TOML in the OS config directory (`~/Library/Application Support/dev.magi.app/config.toml` on macOS).

```toml
schema_version = 1

[hotkey]
toggle = "Alt+Space"

[prompt]
# Appended to Magi's own system prompt, never replacing it. Magi's instructions
# carry the contract that makes agentic capture fire, so a value that could
# overwrite them would disable tier 1 with no way for the user to connect the
# symptom to this field.
context = "I work in Kitchener, Ontario, mostly in Rust."

[appearance]
theme = "system"        # system | light | dark
show_thinking = false

[capture]
target = "active-display"   # active-display | active-window | display:N
redact_on_capture = false   # v2

[active]
provider = "local"
model = "qwen2.5-vl:7b"

[[provider]]
id = "local"
kind = "openai-compatible"
base_url = "http://localhost:11434/v1"
# A list, not one name: an endpoint routinely serves many. Discovered from
# GET /v1/models rather than typed by hand.
models = ["qwen2.5-vl:7b", "llama3.2"]
requires_key = false
# tier is derived by preflight, never written by hand

[[provider]]
id = "claude"
# NOT openai-compatible. Anthropic's API differs in auth header, system prompt
# placement, tool schema, image encoding, and required fields — it is a separate
# wire protocol, and `kind` is what selects the implementation.
kind = "anthropic"
base_url = "https://api.anthropic.com/v1"
models = ["claude-opus-5"]
requires_key = true
# api key lives in the OS keychain under service "dev.magi.app", account "claude"
```

Adding a section with a default is additive, so it does not bump `schema_version` — see [VERSIONING.md](../../VERSIONING.md) for what does.

**API keys never touch the config file.** They live in the OS keychain via the `keyring` crate. A config file that is safe to paste into a GitHub issue removes an entire class of accidental credential leaks — a real risk for an open-source project whose users will be pasting configs into bug reports.

---

## 9. Permissions (macOS)

Three separate TCC permissions, requested lazily at first genuine use rather than all at once on launch:

| Permission | Needed for | Requested when |
|---|---|---|
| Microphone | Voice input | First hotkey activation |
| Screen Recording | `capture_screen` | First tool-initiated capture |
| Accessibility | Reliable global hotkeys; computer use in v3 | First launch (hotkey is core) |

Screen Recording cannot be requested programmatically in a way that survives denial — macOS requires an app restart after granting. Settings shows a live status row per permission with a *Open System Settings* button, and the panel degrades with an explicit message rather than failing silently.

---

## 10. Error handling

Layered: `thiserror` enums at module boundaries so callers can react to *which* failure occurred, `anyhow` at the top for context.

`unwrap()` and `expect()` are banned outside tests and setup code. **A panic in a background tray app is invisible** — the hotkey simply stops working, with nothing on screen and nothing in a terminal the user is looking at. This is Magi's worst failure mode, so it is a hard rule rather than a preference.

User-visible failures are classified and each has a defined surface:

| Failure | Surface |
|---|---|
| Provider unreachable | Panel inline error + tray icon state, with the resolved URL shown |
| Model lacks vision but capture was needed | Panel note explaining the tier, linking to Settings |
| Permission denied | Panel banner + Settings row + button to open System Settings |
| STT model missing | First-run download flow with progress, resumable |
| Hotkey registration conflict | Settings warning naming the conflict; app still usable via tray |

---

## 11. Testing

- **Unit tests** cover the logic that is actually subtle: config parsing and migration, tier assignment from probe results, deictic detection, prompt and history assembly, token-stream parsing.
- **Fakes over mocks.** Each trait gets a hand-written fake. `FakeProvider` replays scripted turns including tool calls, which is how the agentic-vision path is tested without a model.
- **No test may require** a GPU, microphone, display, or network. CI has none of them. This constraint is what forced the trait boundaries, and it is worth defending.
- **Manual test matrix** for the parts that cannot be automated: permission grant and denial paths, hotkey conflicts, multi-display capture, sleep/wake, and provider swap mid-thread.

---

## 12. Open assumptions

Recorded explicitly so they can be challenged in review rather than discovered later:

1. **License: Apache-2.0** — chosen over MIT for its explicit patent grant, which matters for a project that may accept corporate contributions.
2. **No conversation persistence in v1.** Dismissing the panel discards the thread. Privacy-preserving default; persistence becomes opt-in in v2.
3. **Default hotkey `Alt+Space`**, configurable. Low conflict rate on macOS.
4. **Default capture target is the active display**, not all displays. Multi-display users get one image, not three.
5. **Whisper model is downloaded on first run**, not bundled, to keep the installer small. `base.en` by default, with `small` and `medium` selectable.

---

## 13. Milestones after v1

Extension points exist in the architecture; none are built.

- **v2 — Ambient:** wake word (`ort` + openWakeWord) feeding the same `session` state machine; Piper TTS as a sidecar consuming the same token stream; opt-in conversation persistence.
- **v3 — Agency:** `enigo` behind an explicit per-session permission grant, a dry-run mode that shows intended actions before executing, and a hard kill switch.
- **Ongoing:** Windows and Linux packaging, community wake-word models, provider presets.
