# Magi M2 (Config & Providers) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Configure a model and hold a text conversation with it. Type into the panel, watch the answer stream back.

**Architecture:** Config in TOML with a schema version and a migration hook; API keys in the OS keychain behind a trait; a `Provider` trait with two implementations, because "OpenAI-compatible" is a family and Anthropic is outside it. A text turn goes panel → Tauri command → provider → SSE → `magi://token` → panel.

**Tech Stack:** `serde` + `toml`, `keyring`, `reqwest` + `tokio`, `thiserror`, Svelte 5.

**Target version:** `0.2.0-alpha.1`

**Out of scope:** voice, screen capture, the full session state machine, capability tiers (M3). A turn here is a single request with no tools.

## Global Constraints

Copied from `CLAUDE.md` and the design spec. Every task's requirements include these.

- **All code, comments, documentation, and commit messages in English.**
- **No `unwrap()`/`expect()`** outside tests and startup. A panic in a background tray app is invisible.
- **No test may require a GPU, microphone, display, network, or OS keychain.** CI has none of them.
- **Never block the Tauri main thread.**
- `thiserror` at module boundaries, `anyhow` at the top.
- **Arrow functions** in all TS. **Svelte 5 runes only**; runes in shared modules need `.svelte.ts`.
- **API keys never touch `config.toml`.**
- **Every plugin gets its capability permissions in the same commit.**
- Read `.claude/skills/llm-providers/SKILL.md` before touching any provider code. Read `.claude/skills/rust/SKILL.md` and `svelte-5` for their areas.
- Each commit compiles and its tests pass. No deliberately broken intermediate states.

## The constraint that shapes everything

**No test may touch the network or the keychain.** That is not a testing preference; it decides the module boundaries.

It means HTTP cannot be called from provider logic directly — request building and response parsing must be pure functions over bytes, with the transport injected. It means `keyring` sits behind a trait with an in-memory fake. Get this wrong and the tests quietly become integration tests that fail on a runner with no network.

Concretely: **SSE parsing is a pure function from a byte chunk to events.** It is the single most defect-prone piece in this milestone and the one place fixtures pay for themselves.

## File Structure

```
src-tauri/src/
├── lib.rs                    # + config load, provider registry, new commands
├── config/
│   ├── mod.rs                # Config, load/save, defaults
│   ├── migrate.rs            # schema_version -> current
│   └── secrets.rs            # SecretStore trait + keyring impl + fake
├── llm/
│   ├── mod.rs
│   ├── provider.rs           # Provider trait, TurnRequest, StreamEvent, LlmError
│   ├── openai.rs             # OpenAI-compatible: request mapping + SSE parsing
│   ├── anthropic.rs          # Anthropic native: different wire protocol
│   ├── sse.rs                # transport-agnostic SSE frame reader
│   └── fake.rs               # FakeProvider for tests and for the UI before wiring
├── commands.rs               # Tauri commands: config, providers, send_text_turn
└── tests/fixtures/           # recorded SSE bodies, one per provider

src/
├── lib/
│   ├── conversation.svelte.ts  # turns + streaming buffer
│   └── ipc.ts                  # typed invoke/listen wrappers
└── routes/
    ├── panel/+page.svelte      # input + streamed answer
    └── settings/+page.svelte   # provider list
```

---

## Task 1: Config schema, defaults, and migration

**Files:** create `src-tauri/src/config/mod.rs`, `src-tauri/src/config/migrate.rs`; modify `lib.rs`

**Produces:** `Config`, `Config::load(dir)`, `Config::save(dir)`, `ProviderConfig`, `ConfigError`.

- [ ] **Step 1: Write failing tests**

In `config/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_usable_without_a_file() {
        let config = Config::default();
        assert_eq!(config.hotkey.toggle, "Alt+Space");
        assert!(config.providers.is_empty());
    }

    #[test]
    fn round_trips_through_toml() {
        let original = Config::default();
        let parsed: Config = toml::from_str(&toml::to_string_pretty(&original).unwrap()).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn rejects_a_provider_with_no_base_url() {
        let err = toml::from_str::<Config>(
            r#"
            [[provider]]
            id = "local"
            kind = "openai-compatible"
            model = "qwen2.5"
            "#,
        );
        assert!(err.is_err(), "base_url is required");
    }

    #[test]
    fn an_api_key_in_the_file_is_a_hard_error() {
        // Keys belong in the keychain. Silently ignoring one would leave the
        // user believing a secret is configured when nothing reads it, and
        // would leak it into any bug report that pastes the config.
        let result = toml::from_str::<Config>(
            r#"
            [[provider]]
            id = "openai"
            kind = "openai-compatible"
            base_url = "https://api.openai.com/v1"
            model = "gpt-4o"
            api_key = "sk-oops"
            "#,
        );
        assert!(result.is_err(), "api_key must be rejected, not ignored");
    }

    #[test]
    fn unknown_schema_version_is_reported_not_guessed() {
        let result = Config::from_toml("schema_version = 999\n");
        assert!(matches!(result, Err(ConfigError::UnsupportedSchema(999))));
    }
}
```

- [ ] **Step 2: Run them, confirm they fail**

`cargo test config` — expect compile failure, nothing is defined.

- [ ] **Step 3: Implement**

Key decisions to encode:

- `#[serde(deny_unknown_fields)]` on `ProviderConfig`. This is what turns a stray `api_key` into an error instead of a silent no-op, and it catches typos like `base_ur` that would otherwise fall back to a default.
- `schema_version: u32` as the first field, defaulting to the current version. Migration exists from the first release, not after the first breaking change — retrofitting migration onto configs already in the wild is the expensive version of this problem.
- `Config::load` takes a directory rather than reading `dirs` itself, so tests use a temp dir and never touch the real config.

- [ ] **Step 4: Tests pass. Commit.**

---

## Task 2: Secrets behind a trait

**Files:** create `src-tauri/src/config/secrets.rs`

**Produces:** `SecretStore` trait, `KeyringStore`, `InMemoryStore`.

- [ ] **Step 1: Write failing tests against the fake**

```rust
#[test]
fn stores_and_reads_back_a_key() { /* InMemoryStore */ }

#[test]
fn a_missing_key_is_none_not_an_error() {
    // A provider with no key configured is the normal case for Ollama.
    // Returning Err here would make the happy path look like a failure.
}

#[test]
fn deleting_a_provider_removes_its_key() { /* no orphaned secrets */ }
```

- [ ] **Step 2: Confirm they fail, then implement**

```rust
pub trait SecretStore: Send + Sync {
    fn get(&self, provider_id: &str) -> Result<Option<String>, SecretError>;
    fn set(&self, provider_id: &str, secret: &str) -> Result<(), SecretError>;
    fn delete(&self, provider_id: &str) -> Result<(), SecretError>;
}
```

`KeyringStore` uses service `dev.magi.app`, account = provider id. **No test constructs a `KeyringStore`** — CI has no keychain, and a test that touches the real one would also pollute the developer's own.

- [ ] **Step 3: Commit**

---

## Task 3: The Provider trait and turn types

**Files:** create `src-tauri/src/llm/mod.rs`, `provider.rs`, `fake.rs`

**Produces:** `Provider`, `TurnRequest`, `Message`, `StreamEvent`, `LlmError`, `FakeProvider`.

- [ ] **Step 1: Define the types**

```rust
pub enum StreamEvent {
    Token(String),
    Done { stop_reason: StopReason },
    Error(LlmError),
}

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    async fn turn(&self, request: TurnRequest) -> Result<BoxStream<StreamEvent>, LlmError>;
}
```

`TurnRequest` is provider-neutral: messages, model, max_tokens, an optional system prompt. Each implementation maps it to its own wire format. Nothing here mentions `x-api-key`, `input_schema`, or `image_url` — the moment a provider-specific name appears in these types, the abstraction has failed.

- [ ] **Step 2: `FakeProvider` replays a scripted event list**

It exists for tests *and* for the panel UI, which can be built and demoed against it before either real provider works.

- [ ] **Step 3: Tests, commit**

---

## Task 4: SSE parsing as a pure function

**Files:** create `src-tauri/src/llm/sse.rs`, `src-tauri/tests/fixtures/`

This is the highest-risk code in the milestone. It gets its own task and its own fixtures.

- [ ] **Step 1: Write the failing tests first**

```rust
#[test]
fn parses_a_well_formed_frame() {}

#[test]
fn handles_a_frame_split_across_chunks() {
    // Local backends split frames mid-JSON. A parser that assumes one chunk is
    // one frame works against OpenAI and fails against Ollama.
}

#[test]
fn handles_multiple_frames_in_one_chunk() {}

#[test]
fn ignores_comment_and_keepalive_lines() {}

#[test]
fn terminates_on_done_sentinel() {}

#[test]
fn a_stream_that_ends_without_done_is_not_an_error() {
    // Several local servers just close the connection. Treating that as a
    // failure would surface an error after a perfectly good answer.
}

#[test]
fn malformed_json_in_one_frame_does_not_kill_the_stream() {}
```

- [ ] **Step 2: Implement a buffering reader**

`SseParser::push(&mut self, chunk: &[u8]) -> Vec<SseFrame>` — holds a partial line across calls. Pure: no I/O, no async, no provider knowledge.

- [ ] **Step 3: Record fixtures**

Real SSE bodies from Ollama and Anthropic, saved as files, replayed byte-by-byte **in deliberately awkward chunk sizes** (1 byte, 3 bytes, whole body) so the split-frame path is exercised rather than assumed.

- [ ] **Step 4: Commit**

---

## Task 5: OpenAI-compatible provider

**Files:** create `src-tauri/src/llm/openai.rs`

- [ ] **Step 1: Tests for request mapping (pure)**

Assert the JSON body: `messages` with a `system` role message, `stream: true`, model, `max_tokens`.

- [ ] **Step 2: Tests for response mapping (pure, from fixtures)**

Feed a recorded body through the parser and assert the token sequence.

- [ ] **Step 3: Implement, with the HTTP client injected**

The struct owns a `reqwest::Client`, but request building and response parsing are free functions the tests call directly. No test constructs the HTTP path.

- [ ] **Step 4: Commit**

---

## Task 6: Anthropic provider

**Files:** create `src-tauri/src/llm/anthropic.rs`

Same shape as Task 5, different wire format. Re-read the `llm-providers` skill first; the five divergences are listed there.

- [ ] Tests assert `x-api-key` + `anthropic-version` headers, top-level `system`, and `max_tokens` present
- [ ] Tests parse `content_block_delta` → `delta.text`, and `message_delta` for the stop reason
- [ ] Commit

---

## Task 7: Provider registry, presets, custom endpoint

**Files:** modify `config/mod.rs`, create registry in `llm/mod.rs`

- [ ] Registry resolves a `ProviderConfig` to a boxed `Provider` by `kind`
- [ ] Built-in presets: Ollama, LM Studio, OpenAI, Anthropic, OpenRouter — base URL and a sensible default model, nothing more
- [ ] **A custom endpoint is a first-class option, not a fallback.** A missing preset must never be a wall; presets are a convenience over base URL + model + optional key
- [ ] Tests: resolution per kind, unknown kind is a clear error
- [ ] Commit

---

## Task 8: Tauri commands and events

**Files:** create `src-tauri/src/commands.rs`; modify `lib.rs`, `capabilities/default.json`

- [ ] `get_config` / `set_config`
- [ ] `list_providers`
- [ ] `send_text_turn(text) -> ()`, streaming via events
- [ ] Emit `magi://token`, `magi://turn-done`, `magi://error`
- [ ] The turn runs on the async runtime, never the main thread
- [ ] Cancellation: dismissing the panel aborts an in-flight request rather than leaving it to finish into a dropped receiver
- [ ] Commit

---

## Task 9: Panel — input and streaming answer

**Files:** modify `src/routes/panel/+page.svelte`; create `src/lib/conversation.svelte.ts`, `src/lib/ipc.ts`

- [ ] `conversation.svelte.ts` holds turns and the in-flight buffer (`.svelte.ts` is required for runes)
- [ ] Textarea; Enter sends, Shift+Enter newlines
- [ ] Tokens append as they arrive
- [ ] Errors render inline, naming the provider and the resolved URL — "connection refused" is useless without knowing what it tried to reach
- [ ] `Escape` still dismisses and now also cancels
- [ ] Panel grows to fit the answer, within a max height, then scrolls
- [ ] Commit

---

## Task 10: Settings — providers

**Files:** modify `src/routes/settings/+page.svelte`

- [ ] List configured providers; add, edit, remove
- [ ] Preset picker plus a custom option exposing base URL, model, and key
- [ ] The key field writes to the keychain, never to the config
- [ ] Commit

---

## Task 11: Verify and release

- [ ] Full suite: `cargo fmt --check`, `clippy -D warnings`, `cargo test`, `npm run check`, release build
- [ ] **Manual, against Ollama:** configure, ask a question, watch tokens stream
- [ ] **Manual, against a custom OpenAI-compatible endpoint** (the maintainer's Xiaomi MiMo key) — this is the case presets do not cover and the one most likely to expose a wrong assumption
- [ ] **Manual, against Anthropic** if a key is available
- [ ] Manual: wrong URL, wrong key, server down — each produces a distinct, actionable message
- [ ] CHANGELOG, TASKS, tag `v0.2.0-alpha.1`

---

## Self-review

**Risks, in order.** SSE parsing against real local backends is the likeliest source of defects, which is why it is a standalone task with fixtures replayed at hostile chunk sizes. Cancellation is the likeliest thing to be forgotten, so it appears in both Task 8 and Task 9. Testability is the likeliest thing to erode, so the pure-function boundary is stated in the constraints rather than left implied.

**Deliberately excluded.** No tool calling — that is M5, with `capture_screen`. No conversation persistence. No capability tiers; every provider is treated as plain text until M3 probes it.
