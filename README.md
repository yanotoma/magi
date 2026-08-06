<div align="center">

# Magi

**A background AI agent for your desktop. It sees your screen, hears you, and stays out of the way.**

Named after the three-way supercomputer from *Neon Genesis Evangelion*.

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Status](https://img.shields.io/badge/status-pre--alpha-orange.svg)](docs/TASKS.md)
[![Platform](https://img.shields.io/badge/platform-macOS-lightgrey.svg)](#platform-support)

</div>

---

> **`0.1.0-alpha.1` — unreleased.** The design is settled and documented; implementation has not started, and there is nothing to download yet. See [docs/TASKS.md](docs/TASKS.md) for exactly what is done and what is pending, and [docs/VERSIONING.md](docs/VERSIONING.md) for the release train.

## What it does

Magi lives in your system tray. Press a hotkey, ask a question out loud, and get an answer that accounts for whatever is on your screen.

```
tap Alt+Space  →  speak  →  tap again  →  answer
```

It runs on your machine. Speech recognition is local. Nothing is sent anywhere unless you configure a remote model, and the screen is read only when the agent decides it needs to look.

## What makes it different

**Agentic vision.** Most screen-aware assistants attach a screenshot to every message. That is expensive — a screenshot costs over a thousand vision tokens, and because chat history is resent on every request, cost grows *quadratically* with conversation length.

Magi exposes screen capture as a **tool the model chooses to call**. Ask "what is a mutex?" and nothing is captured. Ask "what's wrong here?" and the model reaches for the screen itself. You pay for vision only on the turns that need it, and you get an honest log of exactly when the agent looked.

**It adapts to your model instead of demanding one.** A pre-flight check probes each configured model for vision, tool-calling, and structured output, then assigns a capability tier:

| Tier | Model can | Magi does |
|:---:|---|---|
| 1 | Vision + reliable tool-calling | Agentic capture — the model decides when to look |
| 2 | Vision only | Local heuristic detects "here" / "this error" and captures ahead of time |
| 3 | Neither | Capture disabled, clearly indicated in the UI |

You never configure this. Magi figures it out and tells you what it found.

## Design commitments

1. **Privacy by construction** — local by default, capture is deliberate and logged
2. **Negligible passive cost** — it runs all day; idle RAM and CPU are the metrics that matter
3. **Model-agnostic** — Ollama, LM Studio, OpenAI, Anthropic, OpenRouter are equal citizens
4. **Installable by a non-developer** — one download, one drag, one permission prompt

**Non-goals:** a chat app, a coding assistant, a general automation platform, anything requiring an account.

## Stack

Rust core in a Tauri v2 shell — roughly a 15 MB binary against Electron's ~150 MB baseline, which is the difference that matters for a process that never quits.

| | |
|---|---|
| Shell, tray, overlay | Tauri v2 |
| Frontend | Svelte 5 + TypeScript |
| Screen capture | `xcap` |
| Audio | `cpal` |
| Speech-to-text | `whisper-rs` (whisper.cpp) |
| LLM transport | `reqwest` + `tokio` |
| Secrets | OS keychain via `keyring` |

Wake word (openWakeWord via `ort`), text-to-speech (Piper), and computer use (`enigo`) are designed in and scheduled for v2/v3.

<details>
<summary><b>Why Rust and not Python?</b></summary>

<br>

The obvious alternative was a Tauri shell driving a Python engine, since the ML ecosystem is nominally Python-first. Examined closely, it is not: `faster-whisper` wraps CTranslate2 (C++), openWakeWord is ONNX on onnxruntime (C++), Piper is a C++ binary. In every case Python contributes glue, not computation.

Paying for that glue costs a 250–400 MB installer and ~200 MB resident while idle — exactly the metric that matters most for a process that runs all day. Rust reaches the same native runtimes with a ~15 MB shell.

Pure Python was rejected additionally because the GIL puts an always-listening audio thread in direct contention with the UI thread. That is structural, not tunable.

</details>

## Platform support

| Platform | Status |
|---|---|
| macOS (Apple Silicon + Intel) | v1 target |
| Windows | Designed for, not yet packaged |
| Linux | Designed for, not yet packaged |

Every crate chosen is cross-platform. macOS ships first because it is the hardest — three separate TCC permissions plus notarization — and a permission model that survives macOS ports downward easily.

## Documentation

| | |
|---|---|
| [Design specification](docs/superpowers/specs/2026-08-06-magi-design.md) | The full design and the reasoning behind each decision |
| [Architecture](docs/ARCHITECTURE.md) | Module map, contracts, data flow |
| [Tasks](docs/TASKS.md) | Complete done/pending breakdown across all milestones |
| [Versioning](docs/VERSIONING.md) | What the version number promises, and the release train |
| [Contributing](CONTRIBUTING.md) | How to get set up and what a good PR looks like |

## Contributing

Early contributors have unusual leverage here — the design is settled but no code is written, so architectural feedback is still cheap to act on. If you disagree with something in the design spec, an issue arguing the case is genuinely more valuable right now than a patch.

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[Apache-2.0](LICENSE)
