# Contributing to Magi

Magi is pre-alpha. The desktop shell exists — tray, global hotkey, two windows — and there is no intelligence behind it yet.

That makes this an unusually good moment to contribute. **Architectural feedback is cheaper to act on now than it will ever be again.** If you disagree with something in the [design spec](docs/superpowers/specs/2026-08-06-magi-design.md), an issue arguing the case is more valuable right now than a patch.

## Before you start

Read, in order:

1. [`README.md`](README.md) — what Magi is and what it deliberately is not
2. [`docs/superpowers/specs/2026-08-06-magi-design.md`](docs/superpowers/specs/2026-08-06-magi-design.md) — the design and the reasoning behind it
3. [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — module map and contracts
4. [`docs/TASKS.md`](docs/TASKS.md) — what is actually open

Pick something from `TASKS.md` and comment on the tracking issue before you start, so two people don't build the same thing.

## Setup

**Prerequisites**

| | |
|---|---|
| Rust | Installed via [`rustup`](https://rustup.rs). The version is pinned in `rust-toolchain.toml` and applied automatically — you do not need to switch it manually. **On macOS, check the PATH line landed in a file zsh reads** — see below. |
| Node | 20+ |
| cmake | Required to build whisper.cpp. `brew install cmake` |
| macOS | Xcode Command Line Tools |

```bash
git clone https://github.com/yanotoma/magi.git
cd magi
npm install
npm run tauri dev
```

**If that fails with `failed to run 'cargo metadata'` / `No such file or directory`,**
`cargo` is installed but not on your `PATH`. rustup appends its PATH line to the
shell profiles it detects, and on macOS it may write only `~/.profile` — which
**zsh does not read**, and zsh is the default shell. The binary is fine; the
wiring is not. Fix it once:

```bash
grep -q 'cargo/env' ~/.zshrc || echo '. "$HOME/.cargo/env"' >> ~/.zshrc
source ~/.cargo/env   # for the shell you already have open
```

## Working with an AI assistant

This repo ships project-local skills in `.claude/skills/` for Rust, Tauri v2, Svelte 5, LLM providers, and macOS permissions. They exist because these areas share one failure mode: **the wrong version or the wrong assumption is better represented in training data than the right one.**

- Tauri v1's `SystemTray` and v2's `TrayIconBuilder` are unrelated APIs
- Svelte 4's `$:` reactivity and Svelte 5 runes are unrelated APIs
- "OpenAI-compatible" reads like a standard, but Anthropic's API is outside it
- macOS permissions fail with no error and no log entry

The skills pin the correct version and annotate the traps. If you use Claude Code, they load automatically. If you use another assistant, read them yourself — they are plain Markdown and the traps are real.

## Code standards

**Rust**
- `cargo fmt` and `cargo clippy -- -D warnings` are enforced in CI. Run both before pushing.
- No `unwrap()` or `expect()` outside tests and setup code. A panic in a background tray app is invisible — the hotkey silently stops working, with no window to crash and no terminal to print to. This is a hard rule, not a preference.
- `thiserror` enums at module boundaries, `anyhow` at the top level.
- Never block the Tauri main thread. CPU-bound work goes to `spawn_blocking`.
- Clarity over cleverness. The maintainer is new to Rust; a reviewer should not need a detour into the reference to read your PR.

**Svelte**
- Svelte 5 runes only. No `export let`, no `$:`, no stores.
- Runes require the `.svelte.ts` extension in shared modules.
- Arrow functions.
- TypeScript on every component, no `any`.

**Dependencies**

Adding one is a design decision, not a detail. Justify it in the PR description. Magi's install size and passive memory footprint are product features, not implementation details.

## Testing

**No test may require a GPU, microphone, display, or network.** CI has none of them.

This is why every hardware- or network-touching module sits behind a trait with a hand-written fake. `FakeProvider` replays scripted turns including tool calls — that is how the agentic-vision path gets tested without a model.

If you find yourself wanting to relax this constraint, the design is probably wrong somewhere. Open an issue about the design instead.

## Pull requests

- One concern per PR.
- Reference the task from `docs/TASKS.md` and tick it in the same PR.
- Explain *why*, not just *what*. The diff shows what changed.
- Adding a Tauri plugin? Add its capability permissions in the same commit. A registered plugin missing from a capability file fails at runtime, not at compile time.

## Reporting bugs

Include your OS version, how you installed Magi, which provider and model you configured, and the capability tier shown in Settings. Tier is frequently the answer.

Your `config.toml` is safe to paste — API keys are stored in the OS keychain and never written to it. That is deliberate, precisely so bug reports are safe.

## License

Contributions are licensed under [Apache-2.0](LICENSE).
