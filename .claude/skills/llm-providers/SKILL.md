---
name: llm-providers
description: Use when writing or reviewing Magi's LLM integration — the provider trait, request/response mapping, vision payloads, tool-calling, streaming, or the pre-flight capability probes. "OpenAI-compatible" is a family, not a standard, and Anthropic is outside it; this skill pins the actual wire formats.
---

# LLM providers in Magi

Magi is model-agnostic. That is a product commitment, and it is paid for here.

## The core mistake to avoid

**"OpenAI-compatible" is a loose family, not a specification. Anthropic is not in it.**

Treating Anthropic as "just another OpenAI-compatible endpoint" is the single most likely design error in this module. It diverges on auth, system prompts, tool schemas, vision payloads, and required fields — five things, not one, which is why it needs its own implementation rather than a flag.

| Concern | OpenAI-compatible | Anthropic native |
|---|---|---|
| Auth header | `Authorization: Bearer <key>` | `x-api-key: <key>` **plus** `anthropic-version: 2023-06-01` |
| System prompt | a message with `role: "system"` | top-level `system` parameter |
| Tool definition | `{"type": "function", "function": {"name", "description", "parameters"}}` | `{"name", "description", "input_schema"}` |
| Tool result | a message with `role: "tool"` | a `tool_result` block inside a **user** message |
| Image | `{"type": "image_url", "image_url": {"url": "data:image/png;base64,..."}}` | `{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "..."}}` |
| `max_tokens` | optional | **required** — omitting it is an error |
| Stream format | SSE, `choices[].delta.content` | SSE, typed events (`content_block_delta` → `delta.text`) |

## Architecture consequence

This is why `llm` is organized as a **trait with implementations**, not one client with per-provider branches:

```rust
#[async_trait]
pub trait Provider: Send + Sync {
    async fn turn(&self, req: TurnRequest) -> Result<TurnStream, LlmError>;
    async fn probe(&self) -> Result<Capabilities, LlmError>;
}
```

Two implementations: `OpenAiCompatible` (Ollama, LM Studio, OpenRouter, OpenAI, vLLM, llama.cpp server) and `AnthropicNative`. Magi's internal `TurnRequest` / `TurnResponse` types are provider-neutral; each implementation owns its own mapping to and from the wire.

Resist adding a third implementation for each new vendor. Anything that speaks the OpenAI shape belongs in `OpenAiCompatible` with, at most, a small quirks struct.

## Model IDs are runtime data, not constants

**Never hardcode a model list in Magi.** Model IDs change faster than releases, and hardcoding them would break the model-agnostic promise. The runtime source of truth is `GET /v1/models` (OpenAI-compatible) or `GET /v1/models` on Anthropic, plus whatever the user typed.

Current Anthropic IDs, for docs and defaults only — **no date suffixes**:

| Model | ID |
|---|---|
| Claude Opus 5 | `claude-opus-5` |
| Claude Sonnet 5 | `claude-sonnet-5` |
| Claude Haiku 4.5 | `claude-haiku-4-5` |

If a model ID looks unfamiliar, that means it was released after the model's training cutoff — it is not necessarily wrong. Prefer probing over asserting.

## Vision payloads

Downscale before encoding. Vision token cost scales with resolution, and it is the dominant cost in Magi's agentic-capture design.

**OpenAI-compatible:**
```json
{"role": "user", "content": [
  {"type": "image_url", "image_url": {"url": "data:image/png;base64,iVBOR..."}},
  {"type": "text", "text": "what's happening here?"}
]}
```

**Anthropic:**
```json
{"role": "user", "content": [
  {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "iVBOR..."}},
  {"type": "text", "text": "what's happening here?"}
]}
```

Note the Anthropic base64 string must contain no newlines. Put the image block before the text block — both providers handle either order for images, but image-first is the documented convention and matters for documents.

## Tool calling — the agentic capture path

This is Magi's critical path. `capture_screen` is a tool, so a bug here disables the product's headline feature.

**Anthropic loop:**
1. Send `tools: [{name, description, input_schema}]`.
2. Response arrives with `stop_reason: "tool_use"` and a `tool_use` block carrying `id`, `name`, `input`.
3. Append the assistant's **entire** `content` array to history — dropping the `tool_use` block breaks the turn.
4. Append a **user** message containing `{"type": "tool_result", "tool_use_id": <the id>, "content": ...}`.
5. Repeat until `stop_reason` is `end_turn`.

**OpenAI-compatible loop:** the response carries `choices[0].message.tool_calls`; results go back as messages with `role: "tool"` and `tool_call_id`.

Two rules that hold on both sides:

- **Always parse tool input as JSON.** Never string-match the serialized form — escaping differs across providers and model versions.
- **Return one result per tool call.** A missing result for an emitted call is rejected.

Anthropic supports `"strict": true` on a tool definition (with `additionalProperties: false` and `required` set), which guarantees the input validates. Use it for `capture_screen` — it is a cheap correctness win on the path that matters most.

## Streaming

Both are SSE, and both need incremental token emission into `magi://token`.

- **Anthropic** events: `message_start`, `content_block_start`, `content_block_delta` (read `delta.text`), `content_block_stop`, `message_delta` (carries `stop_reason` and usage), `message_stop`.
- **OpenAI-compatible**: `data:` lines carrying `choices[0].delta.content`, terminated by `data: [DONE]`.

Local backends are the ones that break here. Ollama and LM Studio have both shipped versions with subtly different SSE framing — missing `[DONE]`, non-standard keep-alives, chunked JSON split across frames. **Parse defensively and never assume a frame is a complete JSON object.**

## What pre-flight actually probes

Tier assignment drives real behavior (see the design spec), so the probes must test the capability, not the claim.

| Probe | Method | Failure it must distinguish |
|---|---|---|
| Reachability | `GET /v1/models`, or a 1-token completion where absent | bad URL vs bad key vs model not pulled |
| Vision | generate a solid-color image containing a known digit, ask what it shows | genuine vision vs silently ignoring the image |
| Tool calling | one trivial tool plus a prompt that cannot be answered without it | a well-formed call vs prose that mentions the tool |
| Structured output | small JSON schema, validate the response parses | real schema support vs best-effort JSON |

Probe 3 is the one that earns its keep. **Small local models frequently ignore tool definitions or malform the call syntax**, and a model that returns non-empty text has not passed — check for a structurally valid call.

Pre-flight uses throwaway inputs only. It never sends real screen contents.

### Budget the probes for thinking, not for the answer

**A tight `max_tokens` on a probe reports the most capable models as the least capable.** Reasoning tokens are generated and billed whether or not the limit accommodates them, so a small limit does not avoid that cost — it truncates the answer the cost was spent producing. A model that thinks for three hundred tokens about a picture of a `7` returns empty `content`, and every verdict reads empty as failure.

Magi hit this with a 256-token probe budget and a reasoning model that advertises vision in its own documentation. The limit that looked frugal was the expensive one.

Two rules follow:

- Size the probe budget for thinking plus a short answer, not for the answer alone (`PROBE_MAX_TOKENS`).
- Carry the finish reason into the reply. `finish_reason: "length"` in the OpenAI family and `stop_reason: "max_tokens"` in Anthropic's both mean the answer was cut off, and a truncated failure must be distinguishable from a wrong one — they look identical in the verdict, but one is your bug and the other is a model limitation.

## Local backend quirks worth remembering

| Backend | Quirk |
|---|---|
| Ollama | Base URL is `http://localhost:11434/v1`. Tool support varies sharply by model, not by server version. A model that is not pulled yields a 404 that reads like a bad endpoint |
| LM Studio | Base URL is `http://localhost:1234/v1`. Server must be started explicitly; a connection refused is the normal state, not an error worth alarming about |
| OpenRouter | OpenAI-shaped, but capabilities vary per underlying model. Pre-flight per model, never per provider |
| Xiaomi MiMo | Serves both protocols on one host: `/v1` is OpenAI-shaped, `/anthropic` is Anthropic-shaped. Capabilities differ **per model on the same endpoint** — `mimo-v2.5` (Omni) does images, function calls and structured output; `mimo-v2.5-pro` is text-only and answers an image payload with a 404 whose message is about the model not existing. Thinking is on by default and turned off with `"thinking": {"type": "disabled"}`; the docs use `max_completion_tokens` rather than `max_tokens` |

## Checklist before committing provider code

- [ ] No hardcoded model list
- [ ] Anthropic and OpenAI-compatible are separate `Provider` implementations, not branches
- [ ] Tool inputs parsed as JSON, never string-matched
- [ ] Every tool call gets exactly one result
- [ ] `max_tokens` always set (required by Anthropic, harmless elsewhere)
- [ ] API key read from the keychain, never from config or logs
- [ ] SSE parsing tolerates split frames and missing terminators
- [ ] A fake implementation of the trait exists and the tests use it
