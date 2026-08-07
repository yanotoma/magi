# M3 — Pre-flight and capability tiers

**Target version:** `0.2.0-alpha.2`
**Depends on:** M2 (`Provider` trait, two protocol implementations, config, Settings)

## What this milestone is for

M2 made Magi answer. It also made Magi able to answer *badly* without saying so: any model can be selected, and nothing checks whether it can do what the rest of the app will ask of it. From M5, Magi's central feature is that the model can decide to look at your screen. A model that cannot see images will simply never do that, and the user has no way to tell that apart from a model that chose not to.

Pre-flight is what closes that gap. It is infrastructure rather than a convenience: the tier it assigns changes which prompt is sent, whether capture is offered, and what the tray reports.

## The constraint that shapes the design

Same as M2, and it is still the useful one: **no test may touch the network.** Pre-flight is entirely about talking to endpoints, so if that constraint is honoured the module has to be split into parts that touch the network and parts that decide things — with all the decisions in the second group.

That split is not busywork. Tier assignment is the part that can be wrong in a way nobody notices, because a wrong tier does not fail; it silently degrades. It has to be a pure function over probe results, tested across every combination.

## Decisions

### Probes go through the `Provider` trait, not around it

The probes need to send an image, a tool definition, and a JSON schema. None of those fit `TurnRequest`, and all three are formatted differently by the two protocols — an image is `image_url` in one and a `source` block in the other, a tool is `function` in one and `input_schema` in the other.

So the trait grows exactly one method:

```rust
async fn probe(&self, request: ProbeRequest) -> Result<ProbeReply, LlmError>;
```

`ProbeRequest` carries an optional image, an optional tool, and an optional schema. `ProbeReply` carries the text and any structurally parsed tool calls. Non-streaming, because a probe has no user watching it arrive.

The alternative — building probe payloads outside the provider — would put wire-format knowledge in a second place and guarantee the two drift. This keeps the rule M2 established: **each protocol's shape lives in its own implementation.**

Adding a method to the trait also means `FakeProvider` must answer probes, which is what makes the orchestration testable without a network.

### The probe image uses a real PNG encoder

The vision probe needs an image containing a known digit. Magi already hand-rolls PNG output in `tools/generate_tray_icon.py`, so hand-rolling it in Rust is clearly possible.

It is still the wrong call here. A subtly malformed PNG would be accepted by the HTTP layer and rejected or misread by the model, and the result would be a vision probe that fails for every model — reported to the user as "this model cannot see". That is the most expensive misdiagnosis this module can produce: it would disable the app's central feature across the board and look like a model problem rather than a Magi problem.

Verifying a hand-rolled encoder needs a decoder, which is circular. So: the `png` crate, pure Rust, no C dependency. The dependency buys certainty about the one input whose correctness the whole probe rests on.

The digit is drawn as seven-segment rectangles rather than with a font. No font dependency, deterministic output, and legible to a vision model at a small size.

### Probe results are cached outside `config.toml`

`config.toml` is a public contract surface (see `VERSIONING.md`) and it is the file the user owns and hand-edits. Probe results are neither: they are derived, disposable, and meaningless to edit by hand — the spec is explicit that tier is never written by hand.

Putting them in `config.toml` would add derived state to a documented schema, invite tampering with a value the app is supposed to determine, and mean a stale cache entry could not be cleared by deleting a file.

So they live in `capabilities.json` in the same directory, and deleting it is a supported way to force a re-probe. A parse failure there is not an error worth surfacing: it means "not probed yet".

### Tier assignment is a total function, not a chain of ifs

```rust
fn assign(capabilities: &Capabilities) -> Tier
```

Three tiers over three booleans is eight cases, and every one gets a test. The reason is the failure mode: a mis-assigned tier does not raise anything. It hands a vision-less model the tier-1 prompt, which tells it a capture tool exists, and the model then promises to look at a screen it cannot see.

### The prompt becomes tier-dependent

`commands.rs` currently holds `SYSTEM_PROMPT` as a constant. That moves to `llm/prompt.rs` and becomes a function of `(tier, user context, history)`, because the three tiers need genuinely different instructions:

- **Tier 1** needs to be told the capture tool exists and when to reach for it.
- **Tier 2** must **not** be told about tools at all. The harness captures ahead of it by heuristic, and mentioning tools to a model that malforms them only invites tool syntax leaking into prose.
- **Tier 3** needs to know it cannot see the screen, so it stops offering to look.

Tier 2's rule is the counter-intuitive one and the reason this is a function rather than one prompt with an appended sentence: the tier-2 prompt is not tier 1 minus a line, it is tier 1 with the tool concept removed entirely.

The `[prompt] context` rule from M2 survives unchanged: user text is appended, never substituted, in every tier.

## Order of work

Pure logic first, so the parts that can silently be wrong are settled before anything talks to a network.

1. `Capabilities`, `Tier`, `assign` — with the exhaustive table test
2. `llm/prompt.rs` — assembly per tier, tests per tier
3. `ProbeRequest`/`ProbeReply` on the trait, `FakeProvider` support
4. The probe image generator
5. The two protocol `probe` implementations, with request-shape and reply-parsing tests
6. The orchestrator that runs four probes into a `Capabilities`, tested against fakes
7. The cache
8. Commands, Settings capability matrix, *Re-test*
9. Tray tooltip

The degraded tray icon is listed last in `TASKS.md` and stays there. It is a design problem rather than a coding one — a cancel slash across three separated nodes does not read at 22pt — and it needs looking at in a real menu bar, not reasoning about in a plan.
