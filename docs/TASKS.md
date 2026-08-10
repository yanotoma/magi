# Tasks

Complete breakdown of what is done and what is pending, across every milestone.

**Last updated:** 2026-08-10
**Current phase:** M5 — screen capture & agentic vision, targeting `0.4.0-alpha.1`
**Current version:** `0.2.0-alpha.2` (released — see [VERSIONING.md](VERSIONING.md))
**Overall:** 121 / 162 tasks done (75%)

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
| **M6** | Session machine & panel UX | `0.5.0-beta.1` | 8 | 18 | 🔨 In progress |
| **M7** | Packaging & macOS release | `0.6.0-beta.1` | 0 | 14 | ⬜ |
| — | **v1 total** | `1.0.0` | **121** | **148** | |
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
- [ ] `session.rs` — the state machine (Idle → Listening → Transcribing → Thinking → Capturing → Streaming → Idle)
- [x] Conversation thread held in memory; discarded on dismiss. The discard was missing until M6 began: `reset` existed, was imported by the panel, and had no caller, so dismissing hid a thread that returned on reopening — against a design-doc promise made twice, and a privacy one at that
- [ ] History assembly with a token budget and truncation strategy. Assembly exists; nothing bounds it, so a long thread grows until the model refuses it — and with images resent every turn, sooner than length alone suggests
- [ ] `toggle_session` — the hotkey action that moves the state machine out of Idle. `send_text_turn` and `cancel_turn` already exist and are wired. `dismiss_session` is **dropped**: the panel already composes `cancelTurn()` with hiding the window, and a backend command would duplicate that without adding anything
- [ ] Emit `magi://state` from the session machine. `magi://token` and `magi://error` already exist — as do nine more events added since this was written, which is itself the argument for one state event rather than a growing vocabulary the panel has to infer from
- [x] Cancellation — dismissing mid-stream actually aborts the request. `dismiss()` calls `stop()`, which is `cancelTurn()` plus `cancelStream()`; the backend aborts the task, which drops the receiver so the provider stops on its next send. Both halves are needed — an aborted task cannot emit a completion, so nothing would clear the streaming state and the panel would sit showing Stop
- [ ] Unit tests for every state transition, including error paths

**Svelte**
- [x] `conversation.svelte.ts` — shared rune state. The `.svelte.ts` extension is required: runes in a plain `.ts` file are a compile error
- [x] Panel — thread view with per-turn roles
- [x] Panel — token-by-token streaming render
- [x] Panel — status indicator per state, including a distinct capture indicator: a spinner while the request is away, a pulse while recording and transcribing, "Read <what>" while a screenshot is in play, and an inline alert on failure
- [ ] Tray — drive the icon from the session state. `ShellState` already has five variants
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
- [ ] Panel — click-outside dismisses. Esc already does
- [x] Panel — inline error surfaces per failure class, and notices separately from errors — a recording that hit the two-minute cap is worth saying and is not a failure
- [ ] Code-block syntax highlighting. Markdown itself is done, with a renderer configured so it *cannot* emit HTML rather than emitting it and sanitising after
- [ ] Prompt templates: pre-written user prompts bound to a trigger ("explain this error", "summarise this screen"). Distinct from the system prompt — these are user turns, not instructions, and they belong in the panel UI rather than in the prompt assembler

---

## M7 — Packaging & macOS release

- [ ] Configure `tauri.conf.json` bundle settings and app identifier
- [ ] App icon set, all required sizes
- [ ] `Info.plist` usage descriptions for Microphone and Screen Recording
- [ ] Enable `macOSPrivateApi` for window transparency (documented tradeoff: blocks App Store)
- [ ] Universal binary (aarch64 + x86_64)
- [ ] Code signing with a Developer ID certificate
- [ ] Notarization and stapling
- [ ] DMG with a drag-to-Applications layout
- [ ] Verify how each bundle format handles pre-release identifiers (`-alpha.1`); adopt a monotonic build number if they are dropped
- [ ] GitHub Actions release workflow triggered by `v*` tags, marking pre-release versions as GitHub pre-releases
- [ ] Keep `CHANGELOG.md` current — every user-visible change lands in `Unreleased` in the same PR
- [ ] Auto-update via `tauri-plugin-updater`
- [ ] First-run onboarding: permissions walkthrough, model choice, provider setup
- [ ] Measure and publish idle RAM and CPU (the headline claim needs a number behind it)

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
