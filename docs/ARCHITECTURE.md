# Architecture

This document describes how Magi is put together. For *why* each decision was made, see the [design specification](superpowers/specs/2026-08-06-magi-design.md).

## Overview

```
┌──────────────────────── Tauri v2 shell (Rust) ────────────────────────┐
│                                                                        │
│  tray.rs        icon, menu, capability-tier indicator                  │
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

## Directory layout

```
magi/
├── src-tauri/
│   ├── src/
│   │   ├── main.rs          # thin: builder, plugins, tray, shortcut registration
│   │   ├── tray.rs
│   │   ├── hotkey.rs
│   │   ├── windows.rs
│   │   ├── config.rs        # load / save / validate; secrets via keyring
│   │   ├── session.rs       # the conversation state machine
│   │   ├── audio/           # cpal input, resampling, buffering
│   │   ├── stt/             # whisper-rs wrapper, model management
│   │   ├── capture/         # display & window capture (xcap)
│   │   └── llm/
│   │       ├── provider.rs  # the Provider trait + registry
│   │       ├── openai.rs    # OpenAI-compatible impl
│   │       ├── anthropic.rs # Anthropic native impl
│   │       ├── preflight.rs # capability probing → tier assignment
│   │       └── tools.rs     # tool definitions, incl. capture_screen
│   ├── capabilities/        # Tauri v2 permission files
│   └── tauri.conf.json
├── src/                     # Svelte 5 frontend
│   ├── lib/
│   │   ├── conversation.svelte.ts
│   │   └── components/
│   └── routes/
│       ├── panel/
│       └── settings/
├── docs/
└── .claude/skills/          # project-local skills: rust, tauri-v2, svelte-5
```

## Module contracts

Every module below the state machine is a **leaf**: it knows nothing about the application. Each is defined by a trait so it can be faked in tests and swapped later.

| Module | Responsibility | Trait |
|---|---|---|
| `audio` | Open default input, buffer 16 kHz mono PCM while recording | `AudioSource` |
| `stt` | PCM in, text out | `Transcriber` |
| `capture` | Capture a display or window as PNG bytes | `ScreenCapture` |
| `llm::provider` | One turn in; streamed tokens and optional tool calls out | `Provider` |
| `llm::preflight` | Endpoint config in, capability tier out | — |
| `session` | Owns the state machine and the thread | — |
| `config` | Load, validate, persist | — |

`session` is the only module that knows about the others.

This is not architectural decoration — it is what makes the CI constraint achievable. **No test may require a GPU, microphone, display, or network**, because CI has none of them. The trait boundaries are what let the interesting logic be tested without any of those.

## The conversation state machine

```
        ┌──────────────────────── Esc / error ───────────────────────┐
        ↓                                                            │
     ┌──────┐  hotkey   ┌───────────┐  hotkey   ┌──────────────┐     │
     │ Idle │ ────────→ │ Listening │ ────────→ │ Transcribing │     │
     └──────┘           └───────────┘           └──────┬───────┘     │
        ↑                                              ↓             │
        │            ┌───────────┐              ┌──────────┐         │
        └─────────── │ Streaming │ ←─────────── │ Thinking │ ────────┘
                     └───────────┘              └────┬─────┘
                                                     │ tool_use
                                                     ↓
                                              ┌─────────────┐
                                              │  Capturing  │
                                              └──────┬──────┘
                                                     │ resend with image
                                                     └──→ Thinking
```

`Capturing` is a real state, not an implementation detail — the panel shows a distinct indicator while it is active. Users should always be able to see the moment the agent looks at their screen.

## Data flow for one turn

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

### Threading rule

Whisper inference and screen capture are CPU-bound and always run on `spawn_blocking`. HTTP runs on the async runtime.

**Never block the Tauri main thread.** Blocking it freezes the tray and the hotkey, which in a background app is indistinguishable from a crash — the user has no window to notice is unresponsive.

## IPC surface

Rust → frontend, via `emit`:

| Event | Payload | Meaning |
|---|---|---|
| `magi://state` | `SessionState` | State machine transition |
| `magi://token` | `String` | One chunk of a streaming response |
| `magi://captured` | `{ target, at }` | The agent just read the screen |
| `magi://error` | `MagiError` | Classified, user-presentable failure |

Frontend → Rust, via `invoke`:

| Command | Purpose |
|---|---|
| `toggle_session` | Same effect as the hotkey |
| `send_text_turn` | Typed follow-up |
| `dismiss_session` | End the thread, clear history |
| `get_config` / `set_config` | Settings |
| `run_preflight` | Probe a provider, return its capability matrix |
| `permission_status` | Live TCC state per permission |

## Configuration

Human-editable TOML in the OS config directory. On macOS: `~/Library/Application Support/dev.magi.app/config.toml`.

**API keys are never written to this file.** They live in the OS keychain via `keyring`. This is deliberate: an open-source project's users paste their configs into bug reports, and a config file that is always safe to paste eliminates that entire class of credential leak.

## Error handling

Two layers:

- **Module boundaries** use `thiserror` enums so callers can match on *which* failure occurred. A capture that failed because permission was denied needs a different response than one that failed because no display was found.
- **Top level** uses `anyhow` for context chaining.

`unwrap()` and `expect()` are banned outside tests and setup code. A panic in a background tray app is invisible: the hotkey silently stops working, with no window to crash and no terminal to print to. It is the worst failure mode Magi has.

## Capability tiers

Tier assignment comes out of pre-flight and drives real runtime behavior:

| Tier | Model capability | Vision behavior |
|---|---|---|
| 1 | Vision + reliable tool-calling | Agentic capture |
| 2 | Vision, unreliable tool-calling | Local deictic heuristic triggers capture pre-emptively |
| 3 | No vision | Capture disabled, surfaced in tray and Settings |

The Tier 2 heuristic is a deliberately dumb keyword matcher over the transcript, not a model call. It is allowed to over-trigger: a spurious capture costs tokens, a missed one costs a wrong answer.
