# Tasks

Complete breakdown of what is done and what is pending, across every milestone.

**Last updated:** 2026-08-12
**Current phase:** M6 — session machine & panel UX, targeting `0.5.0-beta.1`
**Current version:** `0.4.0-alpha.1` (released — see [VERSIONING.md](VERSIONING.md))
**Overall:** 139 / 163 tasks done (85%)

Legend: `[x]` done · `[ ]` pending · `[~]` in progress · `[!]` blocked

---

## Status at a glance

| Milestone | Scope | Target version | Done | Total | Status |
|---|---|---|---:|---:|---|
| **M0** | Foundations | — | 15 | 15 | ✅ Complete |
| **M1** | Shell — tray, hotkey, windows | `0.1.0-alpha.1` | 15 | 16 | ✅ Shipped |
| **M2** | Config & providers | `0.2.0-alpha.1` | 28 | 28 | ✅ Shipped |
| **M3** | Pre-flight & capability tiers | `0.2.0-alpha.2` | 13 | 14 | ✅ Shipped |
| **M4** | Audio & speech-to-text | `0.3.0-alpha.1` | 27 | 27 | ✅ Shipped |
| **M5** | Screen capture & agentic vision | `0.4.0-alpha.1` | 15 | 16 | ✅ Shipped |
| **M6** | Session machine & panel UX | `0.5.0-beta.1` | 18 | 19 | 🔨 In progress |
| **M7** | Packaging & macOS release | `0.6.0-beta.1` | 8 | 14 | 🔨 In progress |
| — | **v1 total** | `1.0.0` | **139** | **149** | |
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
- [x] CI check asserting `Cargo.toml` and `package.json` versions agree (`src-tauri/tests/version_sync.rs`, run by `cargo test`). Also asserts `tauri.conf.json` still delegates its version, and that the released version has a matching `CHANGELOG.md` heading — so a bump cannot land without its changelog entry, nor an entry without a bump

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

## M2 — Config & providers ✅

Ends with a working text loop: type in the panel, watch the answer stream back. No voice, no capture, no state machine.

The milestone was widened from pure infrastructure for one reason: with the original plan nothing worked until M6, and streaming into the UI is where the surprises live — split SSE frames, backpressure, cancellation. Finding those against a local model in M2 costs hours; finding them in M6 under four other subsystems costs days of working out which one is at fault.

Manually verified on macOS against Xiaomi MiMo: a provider configured from Settings, models discovered from the endpoint, answers streaming into the panel and rendering as markdown, cancellation via `Escape` and Stop, the global shortcut rebound and surviving a restart, and `[prompt] context` still in effect after Clear and after quitting.

Three bugs the milestone only surfaced when run, all worth keeping in mind:
- Cancelling aborted the task, and an aborted task cannot emit its own completion — so the panel waited forever for a message from something that no longer existed. The side that initiates a cancellation already knows the turn is over and must resolve its own state.
- The keychain was read from a synchronous command, which Tauri runs on the main thread. It deadlocked at launch against an access dialog only the main thread could have drawn. See the hard rule in `CLAUDE.md` and the keychain section of the `macos-permissions` skill.
- A settings field that saved only on `blur` never saved at all, because this window hides rather than closes and a focused field is not reliably blurred when its window disappears.

- [x] `config.rs` — TOML schema with `serde`, loaded from the OS config dir
- [x] Config validation with actionable error messages, not just parse failures
- [x] Config versioning and a migration path (needed before the first public release, not after)
- [x] Write defaults on first run
- [x] Store and retrieve API keys via `keyring`, never in the TOML
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
- [x] Settings UI — provider list with add / edit / remove, behind an *Add a provider* button rather than a form permanently occupying the page
- [x] Model picker with search, so discovery proposes rather than decides. Fetching from an endpoint no longer selects everything it returns: OpenRouter and AI Studio answer with hundreds, and taking all of them would fill the provider card and the capability matrix with models nobody asked for. The user chooses; search exists because scrolling three hundred rows is not a way to find anything
- [x] Render answers as markdown — bold, lists, tables, code. Models emit markdown whether or not it is asked for, so plain text does not mean "no formatting", it means showing `**weather.com**` with the asterisks. Rendered with raw HTML disabled at the parser rather than sanitised afterwards, images disabled (a model-chosen image URL is a read-receipt beacon), and links rendered as non-navigable text next to their real destination — this panel is the app's own webview, so following a link would replace Magi's UI with no way back
- [x] Settings UI — hotkey capture control. Records from `event.code`, not `event.key`: `key` reports what the layout *produces*, so Alt+A is "å" on macOS and a stored binding would depend on the layout at the moment it was recorded. Validation runs before the old shortcut is released and the old one is restored if the OS refuses the new one, so a failed attempt never leaves a background app with no way in
- [x] `[prompt] context` in `config.toml` — free text appended to Magi's system prompt. **Additive, never a replacement**: Magi's own instructions carry the contract that makes agentic capture fire, and letting a user overwrite them breaks tier 1 silently. Enforced by a single `system_prompt()` with no branch that omits Magi's half, and a test asserting no input — hostile ones included — can produce a prompt not led by it
- [x] Register the shortcut the config actually names at startup, not the default. Registering the default meant a hotkey set in Settings worked until quit and then reverted, which reads as the setting failing to save
- [x] Validate `[hotkey] toggle` on load, not only when Settings writes it. The file is meant to be hand-edited, and a hand-written `toggle = "Space"` would be registered as typed — swallowing the spacebar in every application on the machine

---

## M3 — Pre-flight & capability tiers ✅

Manually verified on macOS against Xiaomi MiMo: `mimo-v2.5` probes to *Agentic capture* (reads the test image, calls the tool, declines the JSON schema) while `mimo-v2.5-pro` on the same endpoint and key probes to *Text only*. Per-model tiers on one provider is exactly why results are keyed by model.

One task is carried rather than done — the degraded tray icon, below. It is a design problem that needs looking at in a real menu bar, not a coding one.

Two bugs the milestone only surfaced when run, both worth remembering:
- The vision probe reported a sighted model as blind, because the generated seven-segment digit had unfilled corners and rendered as two detached strokes. The model read it correctly and answered `1`. Ten tests passed; what found it was rendering the PNG and looking at it. Legibility is not directly testable — connectivity is, and a flood fill now asserts every digit is one connected shape.
- Probes were given 256 tokens on the reasoning that they only need a word. Thinking tokens are billed whether or not the limit fits them, so a tight limit truncates the answer rather than avoiding its cost, and an empty reply reads as a failed capability.

- [x] `llm::preflight` module scaffolding — verdict functions pure and separate from the async orchestration, so every way a model can almost-pass is a unit test
- [x] Probe 1 — reachability, distinguishing bad URL / bad key / model not pulled. A trivial completion rather than `GET /v1/models`: a provider can list a model it cannot serve, and the Anthropic-shaped endpoints have no listing route
- [x] Probe 2 — vision, using a generated seven-segment `7`. Fails a confident description with no digit in it, which is what an endpoint that accepted the payload and ignored it produces — and fails a denial that happens to guess right, since a lucky guess must not promote a blind model
- [x] Probe 3 — tool-calling, validating a well-formed call rather than non-empty output. Also rejects a call with an empty argument object and a call to a tool never offered: structurally valid is not the same as usable
- [x] Probe 4 — structured output against a small JSON schema. Accepts a fenced code block, rejects prose around the JSON and `"celsius": "21"` — a schema half-followed is not schema support
- [x] Tier assignment logic from probe results. Total function, no fallback branch. `Unreachable` is its own tier: a text-only model works and an unreachable one does not, and the fixes are unrelated
- [x] Cache results per provider + model in `capabilities.json`, **not** `config.toml` — that file is a contract surface the user hand-edits, and probe results are derived, disposable, and meaningless to write by hand. Cleared whenever a provider is saved, since capabilities belong to the endpoint as much as the model
- [x] Unit tests for tier assignment across every probe-result combination — all sixteen, as a table, so adding a capability forces the list to be revisited
- [x] Settings UI — capability matrix per provider. Three states per cell, not two: an untested model shows a dash, because "untested" and "failed" are different claims and only one is Magi's to make. Every cell explains what was actually sent and what the outcome means — a column headed "JSON" makes a cross look arbitrary, when the measured fact is narrower and more useful: a schema was sent and the reply did not match it
- [x] Settings UI — *Test* / *Re-test* per model, one at a time. Concurrent probes against a metered API can trip a rate limit, which would be recorded as a capability the model lacks
- [x] Surface the active tier in the tray tooltip — the only passive reminder a user gets that their model cannot see the screen
- [x] `llm::prompt` — assemble messages from `(tier, config, history)` rather than from a constant. The prompt is tier-dependent: tier 1 needs instructions on when to call `capture_screen`; tier 2 must not be told about tools at all, since the harness captures ahead of it by heuristic and mentioning tools only invites malformed tool syntax in prose; tier 3 needs to know it cannot see the screen so it stops promising to look
- [x] Unit tests for prompt assembly per tier — pure logic, no network. Includes an attempt to displace Magi's instructions from all four tiers with hostile context values
- [ ] Design the degraded tray icon. A cancel slash across three separated nodes does not read at 22pt — the mark is discontinuous, so the bar alternates between empty space and ring and every crossing forces a choice between eating the ring and breaking the bar. Needs a different idea, looked at in a real menu bar

---

## M4 — Audio & speech-to-text

**Capture**
- [x] `audio` module — enumerate input devices via `cpal`. `device.description()?.name()`, not the `name()` that older examples use
- [x] Open the default input and buffer PCM while recording. The rate and format are pinned from `supported_input_configs` rather than taken from `default_input_config()`, whose selection order changed in 0.18 — and `stream.play()` is called, without which the callback never fires and the recording is silence with no error
- [x] Resample to 16 kHz mono (Whisper's required input format), low-passed before interpolating. Without the filter, 48→16 kHz folds everything above 8 kHz down into the audible range; a test asserts a 15 kHz tone comes out attenuated rather than relocated to 1 kHz. `rubato` was rejected as far more than one fixed speech conversion needs — see the plan
- [x] Cap recording length and handle the buffer-full case. Reaching the cap **stops and transcribes** rather than discarding: the user said something, and the last thing to do with it is throw it away because they said too much
- [x] `AudioSource` trait plus a fake. The trait promises **16 kHz mono `f32`**, not "whatever the device gave us" — so the fake and the real implementation return the same thing and a fixture exercises the same code path a microphone would, rather than a parallel one
- [x] Handle device disconnect mid-recording. Flagged from the error callback and read on stop, so unplugging a microphone mid-sentence returns the sentence. `ErrorKind::Xrun` is deliberately not treated as a disconnection: it is a dropout on a stream that is still alive

**Push-to-talk**

These were missing from the list, not from the plan. M4's checkboxes enumerated the components and forgot the wiring, so the milestone reached 19/19 with `Microphone` and `WhisperTranscriber` never constructed anywhere in the app — the capability existed and nothing called it. `VERSIONING.md` promises `0.3.0-alpha.1` is "speak, get a local transcript", and a checklist is not the contract.

Deliberately not the session state machine, which is M6's. This is the smallest wiring that makes the promise true.

- [x] A **second** hotkey for voice (`Alt+Shift+Space`), configurable, rather than overloading the toggle. Distinguishing a tap from a hold on one key means the panel toggle fires on release and behind a timer, which would degrade the one interaction that already works. Two identical shortcuts are refused at load: the OS gives the combination to whichever registered first and the other silently never fires
- [x] Hold to record, release to transcribe, with the transcript **appended** to whatever is already in the input. Someone who typed half a question and spoke the rest meant both
- [x] Transcription on `spawn_blocking`. On the main thread it would freeze the tray and the hotkey; on an async worker it would hold a runtime thread for its whole duration
- [x] Panel indicator with two distinct states. Recording ends when you let go and transcription ends when it ends, so being told which you are waiting for is the difference between patience and wondering whether the key registered. The dot is red only while audio is actually going in

**Transcription**
- [x] `stt` module — `whisper-rs` integration. Written against the 0.16 source, whose segment API is an iterator of `WhisperSegment` rather than the `full_get_segment_text(i)` that documentation still shows
- [x] Verify the cmake build works on Apple Silicon (whisper.cpp compiles in about two minutes; `.cargo/config.toml` links `libc++` and Accelerate). Intel is untested — no machine to try it on, and the same flags are configured for `x86_64-apple-darwin`
- [x] Enable Metal acceleration on macOS, target-gated so the Linux CI job compiles whisper.cpp without it. Feature-gating the whole crate to keep `cargo test` free of cmake was rejected: a build that compiles the real transcriber only when someone remembers a flag is one where it is usually not compiled
- [x] First-run model download with progress, resumable, checksum-verified. The resume path was verified against the real endpoint by seeding a 20 MB partial file and confirming the completed download passed its checksum — that is the one failure a happy-path test cannot catch, since resuming from a wrong offset yields a file of exactly the right length and the wrong contents
- [x] Verify against the checksum from HuggingFace's API, **never the ETag** on the download URL. Both are 64 hex characters and they are different values, so the wrong one fails every download on every machine and reads as a corrupt network
- [x] Trust the server's `content-length` rather than a hardcoded size. Two of the size constants were initially wrong by ~12 kB, which would have made a *completed* download look longer than the model and be discarded on every attempt, forever
- [x] Model selection in Settings → Voice, with a real progress bar and byte counts. Selecting a model that is not downloaded is allowed — refusing until the file exists would mean picking Small and then separately asking for it, when picking it *is* the request
- [x] Microphone permission shown as a live status row, with a button to the right System Settings pane — offered only when it would help, since `restricted` opens a toggle the user cannot move
- [x] Delete a downloaded model. `medium.en` is 1.4 GB, and an app that can put that on your disk and not take it off again quietly costs you space forever
- [x] `Transcriber::transcribe` is synchronous so the `spawn_blocking` obligation is explicit at the call site; inference uses half the cores rather than all of them, since Magi transcribes while the user is still working
- [x] `Transcriber` trait plus a fake. Synchronous on purpose — inference is CPU-bound for seconds, so making it `async` would suggest it yields when it would in fact occupy a runtime thread throughout. Rejects Whisper's known silence artefacts ("Thank you.", "[BLANK_AUDIO]"), matched on the whole string so a real question containing a polite phrase survives
- [x] Microphone permission request and denial handling. `NSMicrophoneUsageDescription` in a `src-tauri/Info.plist` that Tauri merges — without it the process is **terminated** on first microphone access, with no exception and nothing on screen for a tray app. The state is read without prompting via `AVCaptureDevice.authorizationStatusForMediaType`, so Settings shows what is true rather than finding out when a recording fails
- [x] Distinguish *not yet asked* from *denied* from *restricted*. The untouched state is the intended path, not a failure, and must not read as one; a Mac managed by a configuration profile cannot be fixed in System Settings, so pointing there would send the user somewhere useless

**Language shortlist**
- [x] Settings → Voice shows a checkbox list of languages in place of a single dropdown — leave it empty to detect from all ~99, tick one to pin it, or tick several to restrict detection to your shortlist. Unrestricted detection on a short utterance can misidentify two seconds of Spanish as French; constraining the candidates removes the miss
- [x] English-only models (`.en`) say so in Settings and note the language shortlist is not active — they cannot transcribe anything else, so appearing to honour the setting would be a promise they cannot keep
- [x] The old `voice.language` key in `config.toml` is read automatically on first launch and converted to the new `voice.languages` list, so configs written before this change need no edits
- [x] `config.toml` validation catches unrecognised language codes, lists longer than eight entries, and duplicates, reporting each as a distinct error

---

## M5 — Screen capture & agentic vision

- [x] `capture` module — `capture/screen.rs` wraps `objc2-screen-capture-kit` (ScreenCaptureKit). `xcap` was rejected: every current Rust capture crate uses `CGWindowListCreateImage`, which is marked `obsoleted=15.0` and, measured, returns a picture of an empty desktop when Screen Recording permission is absent rather than reporting an error. ScreenCaptureKit fails with a real error (`SCStreamErrorUserDeclined = -3801`). The trade-off: the one-shot screenshot call (`SCScreenshotManager`) requires macOS 14, raising Magi's minimum from 11
- [x] Capture the active display as PNG bytes — "active" means the display containing the frontmost window, not the primary display. On a single monitor the distinction does not exist; on a three-monitor desk it is the difference between capturing what the user is looking at and capturing whatever the primary display happens to show
- [x] Capture a specific window — the default `capture_screen` target. A 1440×900 window fits the standard-tier token budget at ~1.09×; the same ultrawide desktop shrinks to 0.46× (same token cost, ~2.4× the pixels per character). The default is the sharpest per token: the full screen spends most of its budget on wallpaper. The tool result carries the window list as text beside the image — a model was observed reading blurry pixels to recover application names that `windows()` returns exactly as strings
- [x] Downscale before encoding — vision token cost scales with resolution
- [x] `ScreenCapture` trait plus a fake returning a fixture image — in `capture/source.rs`
- [x] Screen Recording permission handling, including the restart-required path. `CGPreflightScreenCaptureAccess` checks access without prompting — the only way to query the permission without triggering the system dialog, at the cost of two reachable states instead of three: never-asked and denied are indistinguishable because it returns a bare bool. Both are handled the same way: an explanation and a System Settings deep link in the new Screen pane. The restart-required aspect is not a separate code path — it is covered by the existing `Permission::Denied` explanation text, which already notes that quitting and reopening Magi is required
- [x] `llm::tools` — define the `capture_screen` tool schema. `tools.rs` carries the full spec, a `Reason` type distinguishing a model-initiated capture (the model states its own reason) from a deictic one (the user's deictic phrase is passed through), and `CaptureBudget` — consumed by the loop guard below
- [x] Tool-call execution loop — `commands.rs` streams the assistant turn, accumulates tool calls via `llm/toolstream.rs` (which reassembles streamed fragments; no individual fragment is valid JSON in either provider family), captures the requested target, replays the assistant turn whole, answers each call with the image and window list as text, then repeats. Bounded by `CaptureBudget` (3 per turn, saturating)
- [x] Guard against capture loops (cap calls per turn). `CaptureBudget` is a saturating `u8` capped at 3: saturation means a loop that exhausts the budget cannot wrap the counter back to zero and hand itself a fresh one
- [x] Tier 2 deictic heuristic — `asks_about_the_screen(text)` detects phrases in English and Spanish that imply the user wants the screen seen ("here", "this screen", "this error", "acá", "esta pantalla", …) and returns the longest match and its language so the capture audit log can say why. Matching is over tokens, not substrings, so "this" does not fire inside "thistle". Two words excluded with tests: English "that" (a conjunction far more often than a demonstrative) and Spanish "está" ("is") — without the exclusion, an accent-stripping normaliser folds "está" into "esta" ("this") and "¿está funcionando?" becomes a request for a screenshot. Spanish was not in the design doc's examples, which were written before M4 shipped speech in eleven languages — an English-only list means a Spanish speaker on a Tier 2 model silently never gets a capture.
- [x] Unit tests for deictic detection, including negative cases
- [x] Wire the Tier 2 path — `heuristic_capture` runs the deictic matcher over the user's text before the request and attaches a screenshot to the question itself, recorded with `Reason::PhraseMatched` so the log names the phrase that caused it. The matched phrase also chooses the target at no extra cost: one naming the screen captures the display, anything else captures the window in front, which is both the commoner case and the sharper picture. A capture that fails is not reported to the user — they asked a question, not for a screenshot, and a guess that did not work should not become an error about a feature they never invoked
- [x] Emit `magi://captured` so the panel can show a capture indicator — the panel shows "Read <what>" while the turn runs. `Reason::UserAsked` is used for the Settings test button
- [x] Capture audit log, visible in Settings — a new **Screen** pane (between Voice and Hotkeys) showing the Screen Recording permission row, the list of captures, and a Clear button. The log is in-memory only, never written to disk, and that is deliberate: a persisted list of which windows were open and when is a record of someone's working day, and `config.toml` is meant to be safe to paste into a bug report; a sibling file of window titles would undo that. Each entry carries what was captured, why (either the model's stated reason or the user's own deictic phrase), the time, the pixel dimensions, and the visual-token cost. Bounded at 300 entries, oldest dropped. The empty state says plainly "Magi has not read your screen" rather than rendering an empty table — the absence of captures is a fact worth stating
- [ ] Extend the deictic heuristic to the remaining nine Settings languages — pt, fr, de, it, nl, ja, zh, ko, ru. A user who selects any of these gets a Tier 2 model that silently never captures, because "no match" is indistinguishable from "nothing to look at". Adding a language is adding rows to a table, but each row must be verified by a speaker of that language and cross-checked against all existing rows for collisions: a word that means "the" in one language and "this" in another would fire on every sentence
- [x] Correct the vision token figure in `docs/superpowers/specs/2026-08-06-magi-design.md` section 5 — it states a 1512×982 screenshot costs "roughly 1,100 vision tokens on Claude", but Anthropic's current formula (`ceil(w/28) * ceil(h/28)`, standard-tier cap 1568) yields 1944 for that size. The conclusion is unaffected — images dominate cost — but a stale number in a design doc reads as authoritative and will mislead the next reader

---

## M6 — Session machine & panel UX

**Rust**
- [x] `session.rs` — the state machine (Idle → Listening → Transcribing → Thinking → Capturing → Streaming → Idle)
- [x] Conversation thread held in memory; discarded on dismiss. The discard was missing until M6 began: `reset` existed, was imported by the panel, and had no caller, so dismissing hid a thread that returned on reopening — against a design-doc promise made twice, and a privacy one at that
- [x] History assembly with a token budget and truncation strategy. Drops whole *exchanges* oldest-first, never a message: both APIs reject a tool call with no matching result, so splitting one would break the request rather than shorten the conversation. It stops rather than skips, since a thread missing a turn from its middle reads as the user having changed the subject and back. The newest exchange always survives — an over-budget request at least produces the provider's own error, while sending nothing answers a question nobody asked. Text is over-estimated on purpose; images are exact, read from the PNG header Magi itself wrote
- [x] Make the history budget per-provider. `context_tokens` on a provider in `config.toml`, settable in Settings, and absent by default — Magi still does not guess, and a user who sets nothing gets the constant that was there before, which is asserted rather than described. What the task under-specified is that **a context window is not a budget**: the window holds the system prompt, the tool schemas, the history *and* the reply, so `llm::budget` returns both numbers from one subtraction. Two things fell out of knowing the window that were invisible without it. `max_tokens` was a flat 4096, which on an 8k model reserves half the window for a reply of a few hundred tokens and charges the conversation for it — a reply now takes at most a quarter of a small window. And the tool-calling loop appended capture results to a request whose budget had already been spent, so the one path that grows a request mid-turn was the one path that never measured it; it re-fits now, which is safe because `fit` keeps the newest exchange whole and a call therefore never parts from its result
- [x] The panel toggle lives in `session` — renamed from this task's `toggle_session`, which described moving the state machine out of `Idle`. That came from the original interaction model where one hotkey opened the panel *and* the microphone; M2 split them. Opening a window is not an activity, so the session state is deliberately untouched and the panel being open is reported through `ShellState::PanelOpen`. What the task was really about is layering: `windows.rs` had grown calls into `commands` and `session`, because orchestration is easiest to put wherever the window handle already is, and `CLAUDE.md` makes `session.rs` the only module allowed to know about the others. It is a leaf again
- [x] Emit `magi://state` from the session machine. `magi://token` and `magi://error` already exist — as do nine more events added since this was written, which is itself the argument for one state event rather than a growing vocabulary the panel has to infer from
- [x] Cancellation — dismissing mid-stream actually aborts the request. `dismiss()` calls `stop()`, which is `cancelTurn()` plus `cancelStream()`; the backend aborts the task, which drops the receiver so the provider stops on its next send. Both halves are needed — an aborted task cannot emit a completion, so nothing would clear the streaming state and the panel would sit showing Stop
- [x] Unit tests for every state transition, including error paths. Exhaustive over states × events, so a new variant cannot be added without them noticing, and the ways a busy state may reach `Idle` are listed explicitly — anything else arriving there is a bug, and anything added to the list is a decision

**Svelte**
- [x] `conversation.svelte.ts` — shared rune state. The `.svelte.ts` extension is required: runes in a plain `.ts` file are a compile error
- [x] Panel — thread view with per-turn roles
- [x] Panel — token-by-token streaming render
- [x] Panel — status indicator per state, including a distinct capture indicator: a spinner while the request is away, a pulse while recording and transcribing, "Read <what>" while a screenshot is in play, and an inline alert on failure
- [x] Tray — drive the icon from the session state. `ShellState` already has five variants
      and `tray_icon_name` maps them, but neither has a caller outside tests: the icon is
      assigned once in `init` and never changes, so the tray always shows idle. Nothing about
      the tray is done however much of it exists. The state machine above is what it should
      follow
- [ ] Tray — art for `Degraded`, the one state with none. Deferred with a real reason
      recorded in `tools/generate_tray_icon.py`: a cancel slash across three separated nodes
      reads as nothing at 22pt, because the mark is discontinuous and every crossing forces a
      choice between eating the ring and breaking the bar. It needs a different idea, looked
      at in a real menu bar. `PanelOpen` is already settled — it shares `tray-idle`, since a
      panel that is open is visible on screen and needs no second announcement
- [x] Panel — text input for typed follow-ups
- [x] Panel — click-outside dismisses, via window focus loss, and **only when nothing is in flight**. Dismissing unconditionally means glancing at another window while Magi is thinking throws the answer away — hostile in a way the user cannot undo, since the thread is gone and the tokens are spent. Escape still closes it at any time for anyone who means it. Guarded against its own hide, which loses focus and would otherwise re-enter the handler
- [x] Panel — inline error surfaces per failure class, and notices separately from errors — a recording that hit the two-minute cap is worth saying and is not a failure
- [x] Code-block syntax highlighting, via highlight.js with twelve grammars registered individually rather than its common bundle. It is the one thing in `markdown.ts` allowed to emit HTML, and the guarantee that replaces "cannot emit HTML" is narrower and verified: highlight.js escapes the code it is given, checked by passing `<script>alert(1)</script>` through it and confirming the angle brackets come back escaped with no live tag. Colours are mapped from its classes to a small palette in the panel's own stylesheet rather than importing one of its themes — a theme is forty hex values, which is the drift the tokens exist to prevent, and it is fixed while the panel is always a dark translucent surface
- [x] Prompt templates: four pre-written user turns, shown as chips on an empty thread only — they are a way to start, not a toolbar, and under a conversation they would sit between the answer and the follow-up. **Filtered by the backend**, which knows the active model's tier: offering "summarise my screen" to a model that cannot see is the same broken promise as telling one it can look, except the user learns to distrust the buttons rather than the model. One template needs nothing, so the row is never empty — an empty row reads as broken rather than as unsupported. Clicking fills the box and focuses it rather than sending, which keeps one rule across the whole panel: nothing is asked without the user pressing Enter

---

## M7 — Packaging & macOS release

Four of these were already true when the milestone opened, done incidentally by M1–M5 rather
than skipped — the identifier since M1, the icons when the mark was drawn, the microphone
usage description with M4, transparency with the panel. They are ticked with the evidence that
settled each one, because "0/14" invited redoing work that was done.

One task turned out to rest on a wrong premise and is corrected below rather than performed.

- [x] Configure `tauri.conf.json` bundle settings and app identifier. `identifier` has been `dev.magi.app` since M1 — it is also the TCC subject and the keychain service, so it could not have waited. Filled in here: `category`, `copyright`, `shortDescription` and `longDescription`, which are what a DMG shows in Finder's Get Info and were blank
- [x] App icon set, all required sizes. `icons/icon.icns` carries the complete macOS set — verified by extracting it with `iconutil -c iconset`, which yields all ten variants (16, 32, 128, 256 and 512, each with its `@2x`). The loose PNGs beside it cover only 32/128/256/512, so an audit that counts files concludes 16×16 is missing; Tauri uses the supplied `.icns` and never rebuilds it from those, so it is not
- [x] `Info.plist` usage descriptions for Microphone and Screen Recording. **The Screen Recording half of this task cannot be done, because the key does not exist.** `NSMicrophoneUsageDescription` is present and is Magi's own sentence. Screen Recording has no `NS*UsageDescription` counterpart — macOS supplies its own string for that prompt, which is why `.claude/skills/macos-permissions/SKILL.md` says so explicitly and why the in-app explanation in Settings → Screen is the only place Magi gets to state a reason. An automated audit of this repo proposed adding `NSScreenCaptureUsageDescription` as required-and-missing; it is neither, M5 shipped and was verified end to end without it, and adding it would have been a plausible-looking no-op that future readers would have trusted
- [x] Enable `macOSPrivateApi` for window transparency (documented tradeoff: blocks App Store). Both halves are in place and both are needed: `app.macOSPrivateApi: true` in `tauri.conf.json` and the `macos-private-api` feature on the `tauri` dependency in `Cargo.toml`, where the comment records the trade
- [x] Universal binary (aarch64 + x86_64). Magi had been shipping arm64-only — `codesign -dvv` on the bundle reported `Mach-O thin (arm64)`, so an Intel Mac would have downloaded a DMG it cannot run with nothing in the build saying so. CI builds `universal-apple-darwin` and then **asserts with `lipo -archs` that both architectures are present**, because passing the target is an instruction and not a result. The targets are added in the job rather than in `rust-toolchain.toml`, so the Linux jobs do not fetch two Apple targets they will never link
- [ ] Code signing with a Developer ID certificate. **Blocked on credentials, not on work.** Needs `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD` and `APPLE_SIGNING_IDENTITY` as repository secrets
- [ ] Notarization and stapling. Blocked on the same, plus `APPLE_ID`, `APPLE_PASSWORD` (an app-specific password, not the account one) and `APPLE_TEAM_ID`. Note from the skill: a notarization failure returns a log *URL* and the actual reason is inside that JSON rather than in the CLI output, so the workflow has to fetch and print it. And **verifying on the build machine proves nothing** — Gatekeeper caches provenance for locally built apps, so the downloaded DMG has to be opened on a machine that has never seen it
- [x] DMG with a drag-to-Applications layout. Window size and both icon positions pinned in `bundle.macOS.dmg` rather than inherited, so the window a user sees is described in this repo instead of in a dependency that may change its defaults. Configured, not yet looked at — the layout is two coordinate pairs and whether it *reads* right is a thing to open, which waits for a signed build worth opening
- [x] Verify how each bundle format handles pre-release identifiers (`-alpha.1`); adopt a monotonic build number if they are dropped. **They are not dropped, so no build number is needed** — the task's own condition is unmet. Read from the built bundle's `Contents/Info.plist`: `CFBundleShortVersionString` and `CFBundleVersion` both carry `0.4.0-alpha.1` verbatim. Worth knowing that this is *out of spec and works anyway*: Apple documents both keys as period-separated integers, and `-alpha.1` is neither. It survives because nothing on the direct-distribution path parses them strictly — Finder displays the string and Gatekeeper does not care. The two places it would start to matter are the App Store, which Magi has already given up for window transparency, and `tauri-plugin-updater` below, which compares versions as semver and therefore handles pre-release identifiers *better* than a monotonic integer would
- [ ] GitHub Actions release workflow triggered by `v*` tags, marking pre-release versions as GitHub pre-releases. Writable today, but it would be a workflow whose one interesting step — the signed, notarized bundle — cannot run, and a release workflow that has never produced a release is not evidence of anything. Waits on the certificate so that the first tag exercises it for real
- [x] Keep `CHANGELOG.md` current — every user-visible change lands in `Unreleased` in the same PR. Enforced rather than remembered: `the_released_version_has_a_changelog_entry` in `src-tauri/tests/version_sync.rs` asserts the newest `## [x.y.z]` heading matches `package.json`, so a version bump cannot land without its section, nor a section without a bump
- [ ] Auto-update via `tauri-plugin-updater`
- [ ] First-run onboarding: permissions walkthrough, model choice, provider setup
- [ ] Measure and publish idle RAM and CPU (the headline claim needs a number behind it). `tools/measure_idle.sh` is the harness; the number is what is still missing. Deliberately split that way, because a figure written down once cannot be checked later — the next person reading a regression cannot tell it from a first measurement taken on a busy laptop. The script refuses two things rather than reporting them: a **debug build**, whose memory is not the shipped app's, and a **loaded machine**, since idle is the whole quantity being measured and sampling during a build is exactly when it is tempting to. Run it on a settled machine against a release bundle, then put the median in `README.md` naming the Mac and the macOS version it came from

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
| `audio-stt` | ✅ | cpal 0.18 behavioural breaks, the 16 kHz contract, the realtime callback, whisper-rs build and Metal, and the model download whose obvious checksum is the wrong value |
| `screen-capture` | ✅ | The capture API obsoleted in macOS 15, the permission that degrades silently instead of failing, logical points vs pixels, and the vision-token budget that is not a pixel budget |
| `release` | M7 | Tag → universal build → sign → notarize → staple → GitHub release → updater |
| `wake-word` | M8 | openWakeWord ONNX via `ort`, false-positive tuning |
| `tts` | M8 | Piper sidecar, sentence chunking, barge-in |
| `computer-use` | M9 | enigo, coordinate spaces, Retina and multi-display |

A changelog skill was considered and rejected: the convention is three rules that already live in `VERSIONING.md` and `CLAUDE.md`, and a third copy would drift. It is a step inside the `release` skill, not a skill of its own.

## Rejected approaches

Recorded so they are not re-proposed.

| Idea | Why not |
|---|---|
| Use a Claude Pro/Max subscription instead of an API key | The subscription covers claude.ai and Claude Code, not third-party API clients. Reusing their stored credentials means reverse-engineering a private auth flow, violates the terms, and breaks without warning. **Confirmed since:** Anthropic began blocking third-party Max OAuth on 2026-01-09 and its legal-compliance page now states plainly that consumer OAuth tokens in any other product "constitutes a violation of the Consumer Terms of Service". opencode removed the feature in February 2026 citing legal requests. Declining this in M0 turned out to be the only path that did not need undoing. |
| OAuth sign-in for Anthropic as an alternative to a pasted key | Was an M2 task; removed as **not implementable**, not merely unwise. Anthropic runs no OAuth program for third-party applications and offers no way to register a client id, so the only route is to reuse Claude Code's hard-coded one — which is the rejected approach above wearing a different name. The three supported methods are API keys, Workload Identity Federation (for cloud workloads federating an existing IdP identity, which a desktop app has none of), and App Attest. App Attest is genuinely for macOS apps, but its tokens bill **the developer's** workspace — for an open-source tool where each user brings their own account, that would mean the maintainer paying for every user's usage. Pasting an API key stays the only honest option. Revisit only if Anthropic publishes third-party client registration. |
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

---

## Deferred: code-switching within one recording

Also outside the summary table, for the reason given below.

**Open question, not a known defect.** Whisper decides the language once, on the first
window, and that decision governs the whole recording — `whisper_full` runs its detect
before the main loop, and `detect_among` likewise inspects offset 0 only. So a shortlist
answers "which of my languages is this recording in", not "which language is this
sentence in". Someone speaking Spanish and English in the same breath gets one governing
language either way.

What is unknown is how much that costs in practice, and there is reason to think the
answer is "little": the language token conditions the decoder rather than binding it —
a multilingual model pinned to `en` and handed clear Spanish transcribed it as Spanish
anyway when this was tested — and `set_translate(false)` means nothing is rendered into
the governing language on purpose. Mixed speech may well survive intact.

Do not act on that reasoning without measuring it. Four cases worth running against
`ggml-base.bin`: Spanish-dominant with English phrases, English-dominant with Spanish
phrases, and a hard switch mid-recording in both orders — the last two built by
concatenating clips from two voices, so each half has native pronunciation rather than
one voice's phonetics applied to the other language. The pair of hard switches is the
informative one: it separates "whichever language starts wins" from "overall content
wins", and only the first of those would mean a user has to think about word order.

- [ ] Measure code-switched transcription across those four cases and record the result
      here. If mixed speech degrades, the fix is not a better shortlist — per-segment
      detection means re-running detection per window and accepting that a segment
      boundary can land mid-sentence

## Requested: drawing on the screen to point things out

Asked for after agentic capture worked: *"me gustaría que magi sea capaz de dibujar en mi
monitor... círculos, o flechas, o incluso escribir en mi pantalla para indicarme dónde dar
click o qué hacer"*.

**Feasible, and the drawing is the easy half.** A transparent, undecorated, always-on-top
window with cursor events ignored is a shape Tauri already builds — the panel is three of
those four things — and `set_ignore_cursor_events(true)` is what lets clicks pass through to
the application underneath. Magi renders in a webview, so an arrow, a circle or a label is
SVG. Nothing new is needed permission-wise either: pointing at the screen requires no
Accessibility grant, because nothing is being clicked.

**The hard half is knowing where to draw.** Coordinates have to come from the model, and the
model saw a downscaled screenshot, so every mark depends on inverting that transform. Two
things make this more tractable here than it usually is:

- Magi *asks* ScreenCaptureKit for an exact output size rather than resizing afterwards, so
  the mapping from image pixels back to screen points is exact and known, not inferred.
  Anthropic's own vision-coordinates guidance names failing to account for the resize as the
  common cause of coordinate misalignment; that failure mode is closed by construction.
- The capture already records which display and which window it photographed, so a mark can
  be placed on the right monitor on a desk with three of them.

What is genuinely unknown is **whether a general vision model can point accurately enough to
be useful.** Models built for computer use are trained to; a model asked "where is the Save
button" often answers with coordinates that are close enough to describe and too far off to
draw on. That is a measurement to make before building the overlay, not an assumption to
build on — an arrow pointing confidently at the wrong button is worse than a sentence saying
"the Save button, top right".

Note also what this is: **M9's coordinate problem without M9's risk.** Computer use is this
plus synthesising input, and the dangerous part is the synthesis. Advising where to click
while the person clicks is strictly safer than clicking for them, and it may well be the more
useful feature. Worth considering ahead of M9 rather than as part of it.

- [ ] Measure first: give a model a screenshot with a known target and ask for its
      coordinates, across several models and window sizes. Record the error in pixels. If
      the error exceeds the size of a button, the overlay is not worth building yet and the
      honest feature is a sentence rather than an arrow
- [ ] An overlay window: transparent, undecorated, always-on-top, `skipTaskbar`, and
      cursor-events-ignored so it never intercepts a click. One per display, since a mark
      belongs to the screen it describes
- [ ] A neutral shape vocabulary — circle, arrow, box, label — with coordinates in the
      captured image's own pixel space, converted to screen points by Magi rather than by
      the model. The model should never be asked to reason about Retina scaling or monitor
      layout, both of which it cannot see
- [ ] A `point_at` tool for the agentic tier, offered only where the measurement above says
      the model can hit what it aims at
- [ ] Marks expire. An arrow left on screen after the answer is stale advice, and stale
      advice about where to click is worse than none — dismiss on the next question, on
      Escape, and after a timeout

## Decided: closing the panel keeps the thread

Reversal of a design-doc decision, at the maintainer's request: *"creo que el hilo no se
debería descartar a menos que se haga click en clear"*.

The doc says it twice — "dismissing it ends the thread", and "no conversation persistence in
v1... privacy-preserving default". Implemented that way for one commit and then reversed,
because the cost is paid more often than the benefit: Escape and clicking away are easy to
trigger by accident, so discarding there loses a conversation to a mistaken keypress, while
the privacy it buys only matters when somebody else is at the machine. **Clear** is
unambiguous, and nobody presses it by accident.

What this changes, and what still holds:

- Closing the panel — by Escape, by clicking away, by the hotkey or from the tray menu — hides
  it and keeps everything. All four now behave alike; two of them did not, and the difference
  was invisible.
- **Clear** discards both halves: the thread the panel shows and the screenshot the backend
  kept for the next question. A Clear that left an image behind would be invisible state the
  user believed they had thrown away.
- Nothing is written to disk. The thread and the capture live in memory and go on quit, so
  "no conversation persistence" is still true of storage — it is no longer true of dismissal.

- [x] Correct section 4 and decision 2 of `docs/superpowers/specs/2026-08-06-magi-design.md`,
      which described a dismissal that ends the thread
- [x] Expire the remembered screenshot five minutes after the panel is closed. The
      *conversation* does not expire — the text is small and is the part worth continuing,
      while the image is megabytes and is a photograph of what somebody was doing. A timestamp
      **and** a timer: a timer alone would fire five minutes after a close that was followed by
      a reopen and a second close thirty seconds ago, and a timestamp alone would leave the
      memory held until somebody happened to ask for it — which is the case being guarded
      against

## Requested: memory beyond one turn

Asked for after noticing Magi cannot see what it looked at a turn ago: *"en vez de tenerlo
todo en memoria, podríamos agregar un sistema de memoria? como un RAG o engram o sqlite?"*

**First, what is actually broken.** The panel resends history as `{role, content}` only, so
screenshots exist just inside the turn that took them. That was never decided — `Turn` in
`conversation.svelte.ts` has no field for an image — and it has two consequences worth
separating. The design doc's cost argument, that an image attached once is paid for on every
later turn, **does not describe this implementation**: images are not resent, so the
quadratic growth it warns about is not happening. And a follow-up question about a screen
Magi just read cannot see it, which is the part that is genuinely wrong.

**Why storage is the wrong first tool.** SQLite solves persistence, not relevance: the whole
conversation is already in memory, and the limit is what fits in a request rather than what is
kept. RAG solves relevance but needs embeddings — either another local model on the scale of
whisper, or a remote API, which sends the text of someone's conversations off the machine and
contradicts the reason the capture log is memory-only. Both also miss the thing being
forgotten: it is an image, and there is no useful way to retrieve a screenshot by similarity.
What would be stored is a text description, which is what the model's own answer already is.

Cheapest first, and each step is useful without the next:

- [x] Carry the most recent capture into the following turn. Held in `AppState` rather than in the panel, which never receives the image — sending megabytes out to the webview so it could send them back is the obvious wrong shape. Introduced by a sentence rather than attached bare, so the model treats it as context from earlier and can notice if it looks stale. The newest replaces the previous one: an older screenshot is not merely less useful, it is misleading, because the screen moved on and the model cannot tell
- [ ] Summarise instead of truncating when the budget bites. Ask the configured model to
      condense the oldest exchanges into a paragraph and send that in their place. Uses the
      model already there; no new infrastructure. `llm::history::fit` is where the decision
      to drop currently lives, and is where the choice to summarise belongs
- [ ] Correct the design doc's cost argument in section 5, which describes resent images that
      are not resent. Either the behaviour or the doc is wrong, and right now it is the doc
- [ ] Only then, persistence across sessions — which the design doc already places in v2 and
      calls an opt-in, because a stored conversation is a record of what someone was doing all
      day and their questions about it. SQLite for storage when it happens; embeddings only if
      retrieval proves necessary after summarisation, and local ones, or the privacy promise
      goes with them

## Deferred: asking the endpoint how big its window is

Outside the summary table, same as the sections around it.

`context_tokens` is per provider and typed by hand. That is honest and it is enough for the
common case — one local model, or one hosted family — but it is wrong in two places. An
endpoint serving many models has one number for all of them, so it has to be the smallest to
be safe: OpenRouter's catalogue runs from 4k to over a million, and pinning it at 4k penalises
every model on the list. And a number typed by hand is a number that goes stale silently.

Some endpoints will say. **None of them is the OpenAI listing route Magi already calls** —
`GET /v1/models` returns ids and nothing else, which is why `discovery.rs` reads only
`data[].id`. Each source below is a separate request in a separate shape:

- **OpenRouter** — `GET /api/v1/models` carries `context_length` per model. The cleanest of
  the three, and the one where per-model matters most.
- **LM Studio** — `GET /api/v0/models` carries `max_context_length` and
  `loaded_context_length`. Its own namespace, not `/v1`.
- **Ollama** — `POST /api/show`, and **the trap is here**. The response has both
  `model_info["<arch>.context_length"]`, which is what the architecture was *trained* for, and
  a `parameters` string carrying `num_ctx`, which is what the server actually *loaded*. They
  routinely disagree by more than an order of magnitude, in the dangerous direction. Ollama's
  own documentation shows it: the same example response reads
  `"parameters": "temperature 0.7\nnum_ctx 2048"` beside `"gemma4.context_length": 131072`.
  Reading the architecture figure and believing it would tell Magi 131072 about a model being
  served at 2048 — a request 64× too large, built on a number Magi went and fetched, which is
  worse than the conservative constant it replaced. `num_ctx` is the field that governs, and
  it is prose inside a string rather than a typed value.
- **Anthropic** — has no listing route and no per-model metadata. Its windows are documented
  and not discoverable, and hardcoding them would put model facts in Magi that go stale
  between releases: the 1M-context variants already make one number per vendor wrong.

So this is not "parse one more field". It is three vendor-specific requests, one of which
reports two contradictory numbers and needs the less obvious one.

- [ ] Move `context_tokens` from per-provider to per-model, cached in `capabilities.json`
      rather than written to `config.toml` — it is already keyed provider → model, already
      versioned, and already discarded when a provider is saved, which is the invalidation this
      needs. A hand-set value must still win over a discovered one
- [ ] Read `context_length` from OpenRouter's `/api/v1/models` during discovery
- [ ] Read `max_context_length` from LM Studio's `/api/v0/models`
- [ ] Read Ollama's **`num_ctx`** from `POST /api/show`, parsed out of the `parameters` string,
      and **never** the architecture's `context_length` — with a test pinning both against a
      captured response so the wrong field cannot be substituted later by someone reading
      `model_info` and finding a plausible key
- [ ] Show a discovered window as discovered in Settings, distinct from one that was typed. A
      number Magi guessed and a number the user asserted must not look the same, because only
      one of them is worth trusting when an answer comes back truncated

## Deferred: the text estimator is wrong in the unsafe direction for CJK

Outside the summary table, same as the sections around it.

Found while making the history budget per-provider, and deliberately not fixed in the same
change: the budget now has a number to fit, which makes the accuracy of the *measurement*
matter in a way it did not when the budget was an arbitrary constant.

`history::estimate_text` divides characters by four. The doc comment on `CHARS_PER_TOKEN`
already says it is "less accurate for Spanish and worse for CJK", and claims the rounding-up
plus a four-token per-message overhead compensate. **They do not, and cannot.** Those add a
constant; the error is multiplicative. A per-message `+4` cannot offset a per-character
factor, so the longer the message the further off it gets — and it errs by *under*-counting,
which is the direction the module elsewhere goes out of its way to avoid.

This is not hypothetical for Magi. M4 ships speech-to-text in ninety-nine languages and the
changelog names Japanese specifically, so a Japanese thread is an ordinary use of the app,
not an edge case. A Japanese, Chinese or Korean conversation is measured as far cheaper than
it is, and `fit` therefore keeps more of it than the budget intends — the provider's
context-length error, at exactly the point truncation was supposed to prevent one.

What is *not* known is the size of the factor. It should be measured against real tokenisers
rather than asserted: CJK text is much denser in tokens per character than English under BPE,
but how much depends on the tokeniser, and Magi does not have the model's. Recording an
unverified multiplier here would repeat the mistake of the comment being corrected.

Deliberately separate from the per-provider budget work, because changing the estimator
changes truncation for **every** user, language and provider, which is a far wider blast
radius than adding an optional per-provider setting.

- [ ] Measure it before changing it: take a few representative strings — English, Spanish,
      Japanese, Chinese, Korean, and mixed — and compare `estimate_text` against a real
      tokeniser's count. Write the measured ratios into the skill or this file. The vision
      probe's lesson applies: the bug there survived ten passing tests and died when someone
      looked at the actual artefact
- [ ] Make the estimate script-aware — most cheaply by counting characters outside Latin-1 at
      a different rate rather than by embedding a tokeniser, which would tie Magi to one
      vendor's and defeat being model-agnostic
- [ ] Correct the `CHARS_PER_TOKEN` doc comment either way. It currently states a compensation
      that does not hold, which is worse than stating the limitation plainly

## Deferred: what a conversation costs

Requested after watching agentic capture work: *"quiero saber que tan costoso es mantener
una conversación con el agente"*. A fair question that Magi currently cannot answer, and the
design makes it a sharper one than usual — history is resent on every turn, so an image
attached once is paid for again on every later turn, and the cost of a thread grows
faster than its length. `Settings › Screen` already shows what each capture cost in visual
tokens, which is the expensive half and only half.

Both families report usage, and one of them needs asking:

- **Anthropic** puts it in the `message_delta` event of the stream, and `message_start`
  carries the input count.
- **OpenAI-compatible** streams **no usage at all by default** — it arrives only if the
  request sets `stream_options: {"include_usage": true}`, and then in a final chunk whose
  `choices` array is empty. A parser that assumes every chunk has a choice drops it.

- [ ] Read usage from both providers' streams and carry it as a neutral type — input,
      output, and cached input where reported, since a cached prompt costs a fraction of a
      fresh one and a total that ignores the distinction overstates a long thread
- [ ] Send `stream_options: {"include_usage": true}` for the OpenAI family, and treat a
      final chunk with an empty `choices` array as usage rather than as a malformed frame
- [ ] Show the running cost of the open conversation in the panel — tokens for the thread
      so far, not for the last turn, because the resend is the part that surprises people
- [ ] A usage log in Settings beside the capture log: per turn, what went up, what came
      back, and what it cost. In memory only, for the same reason the capture log is —
      a persisted record of every question asked is a diary nobody consented to
- [ ] Money, not just tokens, where the price is known. A per-model price in the provider
      config would let Settings show a number people actually reason about; leave it
      absent rather than guessed, since a wrong price is worse than none

## Deferred: lowering the macOS floor back below 14

Outside the summary table, same as the sections below it.

**Decision — ScreenCaptureKit, and macOS 14 as the minimum.** The alternative was
`CGWindowListCreateImage`, which covers macOS 11 and still runs today, but which returns a
picture of an empty desktop rather than an error when Screen Recording permission is absent.
A permission check before every capture closes that hole, so the choice was really about
longevity: Apple marks that call `obsoleted=15.0` with "Please use ScreenCaptureKit
instead", and building capture on it means rebuilding it later. ScreenCaptureKit itself
starts at macOS 12.3, but its one-shot `SCScreenshotManager.captureImageWithFilter` — the
only path that is a screenshot rather than a video stream stopped after one frame — starts
at 14.0.

Lowering the floor later only widens support; it can never strand someone who already runs
Magi, which is why taking the simple path first is safe.

**The maintainer has since said they do not need to support older macOS versions**, so the
task below is recorded rather than planned. Do not spend effort on it without someone asking
for it — it is a fair amount of machinery for two macOS versions nobody has asked about.

- [ ] Capture on macOS 12.3 through 13 via `SCStream`, to lower the minimum from 14. Needs
      an `SCStreamOutput` delegate implemented with `objc2`'s `define_class!`, a stream
      started and stopped around a single frame, and a `CMSampleBuffer` converted to pixels
      — considerably more machinery than the one-shot call, for two macOS versions

## Deferred: native audio input

Outside the summary table on purpose: `tools/task_counts.py` only counts sections whose
heading starts with a milestone number, so nothing here inflates a denominator for work
that is not scheduled. Ticking a box in this section will not move the totals — move the
task into a milestone when it is scheduled, and the count follows.

**Design decision — local STT is the intended path, not a fallback.** Whether Magi could detect native audio support and route voice input directly to a capable model was considered and deliberately deferred. Local whisper.cpp transcription is the *design* for three reasons: the design doc and Settings copy commit to local-only processing ("Transcription happens on this Mac. Nothing you say is sent anywhere"); `voice.rs` puts the transcript in the panel input so a mis-transcription can be corrected before it reaches the model — native audio input removes that review step; and most target providers (Ollama, LM Studio, local runtimes) accept no audio at all, so a primary path that depends on native support inverts the model-agnostic goal. The useful extension is detection and opt-in bypass for providers that genuinely support it, not a default replacement.

- [ ] Add a `hears` field to `Capabilities` in `src-tauri/src/llm/capability.rs`, recorded by the pre-flight probe and displayed in Settings alongside `vision`, `tools`, and `structured_output`. The field must not affect tier assignment — exact precedent is `structured_output`, documented "Recorded but not used for tier assignment; see [`Tier`]. It is shown in Settings because it explains capabilities that arrive in later milestones."
- [ ] Commit a small pre-recorded speech asset to `assets/probe/` — a single spoken word, ~16 kHz mono WAV, ≤ 50 KB — for use by the audio probe. A synthetic tone is not sufficient: a provider that accepts an audio payload and ignores it must fail the probe, so only a probe that sends recognizable speech and checks the transcript can distinguish "accepting" from "hearing". The vision probe applies the same logic: it generates a digit image locally and asks the model to read back a specific digit.
- [ ] Implement the audio probe: send the committed asset to the provider and ask what word was spoken; set `hears: true` only if the response names the word. Follow the architecture of the vision probe in `src-tauri/src/llm/probe.rs`.
- [ ] Add an opt-in "Send audio directly to model" toggle in Settings, off by default, visible only when the active model's `hears` field is `true`. When on, voice input bypasses whisper.cpp and sends the raw audio to the provider. The setting's description must state explicitly what this trades: the local-only privacy promise (audio leaves the device) and the transcript review step (a mis-transcription becomes a confident wrong question). Off by default and honest about the trade-off are not negotiable — local STT is the design; direct audio is the exception.
