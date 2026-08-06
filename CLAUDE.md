# Magi — instructions for AI assistants

## What this project is

A background desktop AI agent: system tray, global hotkey, local speech-to-text, agentic screen capture, model-agnostic. Named after the supercomputer from *Neon Genesis Evangelion*.

Read [`docs/superpowers/specs/2026-08-06-magi-design.md`](docs/superpowers/specs/2026-08-06-magi-design.md) before making design decisions. Read [`docs/TASKS.md`](docs/TASKS.md) to know what is actually built.

## Language

**All code, comments, documentation, commit messages, and issues are in English.** The maintainer converses in Spanish; project artifacts are English-only. This is not negotiable — it is an open-source project aimed at a global contributor base.

## Skills

Use the project-local skills in `.claude/skills/`:

| Skill | When |
|---|---|
| `tauri-v2` | Any Tauri code — tray, hotkeys, windows, sidecars, capabilities |
| `svelte-5` | Any Svelte component |
| `rust` | Any Rust code |

They exist because Tauri v1/v2 and Svelte 4/5 have incompatible APIs, and the older versions dominate training data. Do not rely on recalled knowledge for these three.

Check documentation with the context7 MCP server rather than recalled knowledge for any library.

## Hard rules

- **No `unwrap()` or `expect()`** outside tests and setup code. A panic in a background tray app is invisible: the hotkey silently stops working with no window to crash and no terminal to print to.
- **Never block the Tauri main thread.** Whisper inference and screen capture go to `spawn_blocking`.
- **No test may require** a GPU, microphone, display, or network. Every hardware- or network-touching module sits behind a trait with a fake.
- **API keys never touch `config.toml`.** They live in the OS keychain via `keyring`, so users can safely paste configs into bug reports.
- **Adding a plugin means adding its capability permissions in the same commit.** A registered Tauri v2 plugin missing from a capability file fails at runtime, not compile time.
- **Arrow functions** in all JavaScript and TypeScript.
- **No Claude attribution in commit messages.**

## Architecture invariant

`session.rs` is the only module that knows about the others. `audio`, `stt`, `capture`, and `llm` are leaves that know nothing about the application, each behind a trait. Preserve this — it is what makes the no-hardware-in-CI constraint achievable.

## Keep docs in sync

`docs/TASKS.md` is the source of truth for status. Tick tasks in the same PR that implements them, and update the counts in the summary table.
