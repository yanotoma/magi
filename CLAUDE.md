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
| `rust` | Any Rust code |
| `tauri-v2` | Any Tauri code — tray, hotkeys, windows, sidecars, capabilities |
| `svelte-5` | Any Svelte component |
| `llm-providers` | Any provider, request-mapping, streaming, or pre-flight code |
| `macos-permissions` | TCC permissions, Info.plist, signing, notarization |
| `audio-stt` | Any `cpal`, resampling, `whisper-rs`, or model-download code |

They exist because these areas share one failure mode: **the wrong version or the wrong assumption is better represented in training data than the right one.** Tauri v1/v2 and Svelte 4/5 have incompatible APIs; "OpenAI-compatible" reads like a standard but Anthropic is outside it; macOS permissions fail with no error and no log. Do not rely on recalled knowledge in any of these five areas.

`docs/TASKS.md` carries the roadmap for skills not yet written — each is authored when its milestone starts, so it does not go stale before use.

Check documentation with the context7 MCP server rather than recalled knowledge for any library.

## Hard rules

- **No `unwrap()` or `expect()`** outside tests and setup code. A panic in a background tray app is invisible: the hotkey silently stops working with no window to crash and no terminal to print to.
- **Never block the Tauri main thread.** Whisper inference, screen capture, and **every keychain call** go to `spawn_blocking`. The keychain is the one that gets missed, because `store.get(id)` reads like a cheap getter and is actually a synchronous round trip to `securityd` that stops until the user answers an access dialog. A blocked main thread cannot draw the dialog it is waiting for, so the app deadlocks outright — spinning cursor, dead tray icon, hotkey does nothing, and no error anywhere. Note that a synchronous `#[tauri::command] fn` runs **on the main thread**: anything reaching the keychain must be `async` and go through `commands::with_secrets`.
- **No test may require** a GPU, microphone, display, or network. Every hardware- or network-touching module sits behind a trait with a fake.
- **API keys never touch `config.toml`.** They live in the OS keychain via `keyring`, so users can safely paste configs into bug reports.
- **Adding a plugin means adding its capability permissions in the same commit.** A registered Tauri v2 plugin missing from a capability file fails at runtime, not compile time.
- **Arrow functions** in all JavaScript and TypeScript.
- **Styling goes through the tokens in `src/app.css`.** No ad-hoc `color-mix`, radius literal, or one-off opacity in a component. There is no CSS methodology here and none is wanted — Svelte scopes classes per component, so BEM solves a problem the compiler already solved, and a utility framework would replace the CSS system colours (`Canvas`, `CanvasText`, `AccentColor`) that make Light/Dark/System follow the OS with no JavaScript. What the tokens prevent is the drift that actually happened: two settings panes reached fourteen different greys and ten border radii because every rule invented its own. The panel window is the documented exception — it composites over an arbitrary desktop, so its contrast cannot derive from `Canvas`.
- **No Claude attribution in commit messages.**
- **`cargo build` is not the build that ships.** `npm run tauri build` sets `MACOSX_DEPLOYMENT_TARGET` from `bundle.macOS.minimumSystemVersion`, and a C++ dependency can compile under the host SDK and fail under an older deployment target — whisper.cpp uses `std::filesystem`, which libc++ marks unavailable before macOS 10.15, so Tauri's 10.13 default broke the release build while `cargo build` stayed green. `verify.sh` deliberately does not run it (a release build of whisper.cpp takes minutes), so **open the PR early and let CI compile the real thing** rather than discovering it at release time.
- **Run `tools/verify.sh` before committing, and never summarise a test run by hand.** Piping `cargo test` through `grep '^test result' | awk '{sum += $4}'` counts the passing tests of a *failing* target, because `test result: FAILED. 205 passed;` starts with the same words and keeps the count in the same column. A broken suite reads as a slightly lower total. Five failing tests were committed and pushed on the strength of that number. The script pipes nothing and lets exit statuses propagate.

## Architecture invariant

`session.rs` is the only module that knows about the others. `audio`, `stt`, `capture`, and `llm` are leaves that know nothing about the application, each behind a trait. Preserve this — it is what makes the no-hardware-in-CI constraint achievable.

## Versioning

Semver, starting at `0.1.0-alpha.1`. Read [`docs/VERSIONING.md`](docs/VERSIONING.md) before bumping anything.

The public contract is exactly four surfaces: the `config.toml` schema, the Tauri `invoke` commands, the `magi://*` events, and documented user-facing behavior. A change is breaking only if it requires the user to act. Internal module structure and crate choices are not part of the contract.

`package.json` is the single source of truth for the version; `tauri.conf.json` points at it.

## Keep docs in sync

`docs/TASKS.md` is the source of truth for status. Tick tasks in the same PR that implements them, and update the counts in the summary table.

`CHANGELOG.md` gets every user-visible change in the same PR that makes it. Reconstructing a changelog from git history afterwards produces a list of commits, not a list of changes users care about.
