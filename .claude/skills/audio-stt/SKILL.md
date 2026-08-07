---
name: audio-stt
description: Use when writing or reviewing Magi's audio capture or speech-to-text — cpal input streams, resampling, whisper-rs, Metal acceleration, or the first-run model download. cpal 0.18 changed behaviour that older examples still rely on, whisper-rs does not resample for you, and the obvious checksum for the model download is the wrong value.
---

# Audio capture and speech-to-text in Magi

Two crates, one format contract between them, and a handful of things that are quiet when wrong.

Everything below was checked against the current releases rather than recalled: **cpal 0.18.1**, **whisper-rs 0.16.0**.

## The format contract

**Whisper requires 16 kHz, mono, `f32`. `whisper-rs` does not resample.** It converts integers to floats and stereo to mono for you, and stops there — the sample rate is the caller's problem. Hand it 48 kHz audio and it will transcribe confidently and wrongly, because the model interprets the samples as if they were 16 kHz: speech comes out as gibberish at the wrong speed rather than as an error.

A microphone will almost never give you 16 kHz. macOS built-in inputs are typically 48 kHz. So resampling is not an optional refinement, it is part of making the thing work at all.

The chain is: device's native rate and format → `f32` → mono → **resample to 16 kHz** → transcribe.

## cpal 0.18 changed behaviour, and older examples do not know

These are behavioural breaks, which means code written against 0.17 compiles and misbehaves.

**Call `stream.play()` after building.** On CoreAudio, ALSA and JACK, streams no longer start on their own. Miss this and `build_input_stream` succeeds, the callback never fires, and you get a recording of perfect silence with no error anywhere.

**Pin the sample rate.** The default config no longer returns 44.1 kHz. Take the rate from `supported_input_configs()` explicitly rather than assuming what `default_input_config()` will hand back.

**Pin the sample format.** `I32` and `I24` now rank above `I16` in the default selection, so a device that used to give you `I16` may now give you `I32`. Match on `config.sample_format()` and handle each arm; do not assume.

**`BufferSize::Fixed` can be rejected.** Fall back to `BufferSize::Default` rather than failing.

**`SampleRate` is a plain `u32`.** It was a tuple struct, so the familiar `SampleRate(48_000)` does not compile. `StreamConfig` and `SupportedStreamConfigRange` are both `Copy`, so nothing needs cloning.

**Every failure is one `cpal::Error` with an [`ErrorKind`].** The separate `BuildStreamError` and `PlayStreamError` types are gone. This is an improvement worth using rather than working around: `ErrorKind::PermissionDenied` is a first-class variant, so a denied microphone does not have to be detected by searching a backend message for the word "permission" — which is a match against prose that can change in any release. `DeviceChanged`, `StreamInvalidated`, `DeviceBusy`, `RealtimeDenied` and `Xrun` are all similarly typed.

**`Xrun` is a dropout, not a disconnection.** It arrives on the error callback while the stream is still alive and the recording still usable. Treating it as a failure would end a capture that was fine.

**`device.name()` is gone**; it is `device.description()?.name()` now, returning a struct that also carries the manufacturer, driver and interface type.

**`Stream` is `Send + Sync` on every platform**, as of 0.17. The widely repeated advice — that it is `!Send` and must live on a dedicated thread behind a command channel — is obsolete, and following it costs about fifty lines of threading for a problem that no longer exists.

## The audio callback is a realtime thread

`build_input_stream`'s data callback runs on a high-priority thread owned by the OS audio system. It has a deadline: return before the next buffer is due or samples are dropped.

**Never allocate, lock a mutex, log, or do I/O in it.** Every one of those can block for longer than the deadline, and the symptom is not an error — it is clicks and gaps in the recording, which then read as a transcription problem.

Push the samples into a pre-allocated buffer or a channel and do everything else elsewhere. Magi's `AudioSource` exists partly to keep this constraint in one place.

## whisper.cpp prints to stdout unless told not to

`FullParams` defaults to printing progress, timestamps and special tokens to standard output. In a background tray app with no terminal, that is noise going nowhere — and it interleaves with `tracing` output when someone does run with `RUST_LOG` set.

Turn all four off:

```rust
params.set_print_special(false);
params.set_print_progress(false);
params.set_print_realtime(false);
params.set_print_timestamps(false);
```

### The segment API in 0.16 is not the one in the examples

Most documentation — including what a docs search returns — shows `state.full_n_segments()` with `full_get_segment_text(i)`, `full_get_segment_t0(i)`, `full_get_segment_t1(i)`. That shape is older. In 0.16 it is:

```rust
for segment in state.as_iter() {
    let text = segment.to_str_lossy()?;          // Cow<str>
    let start = segment.start_timestamp();        // centiseconds
    let confidence = segment.no_speech_probability();
}
```

Timestamps are **centiseconds**, not milliseconds. Divide by 100.

Prefer `to_str_lossy` over `to_str` when the text goes to the UI: the strict version fails on invalid UTF-8, and losing a whole sentence to one bad byte is the wrong trade against a replacement character.

### Use `no_speech_probability`, not a list of hallucinations

`segment.no_speech_probability()` is whisper.cpp's own per-segment judgement that a passage is not speech. It is a far better silence filter than matching output against known hallucinations — numeric, from the model, and language-independent, where a string list is English-only and needs maintaining as the model changes.

Keep a string list as a backstop for artefacts that arrive with a confident probability, but make this the primary signal. `params.set_no_speech_thold()` sets the threshold whisper.cpp itself uses; its default is `0.6`.

Also worth setting: `set_suppress_blank(true)` and `set_suppress_nst(true)`, which stop the blank and non-speech tokens at the source instead of filtering them afterwards.

## Building

**`cmake` must be on `PATH`.** whisper.cpp is C++, and the build shells out to cmake. `brew install cmake`.

**On Apple Silicon, link Accelerate and libc++** via `.cargo/config.toml`:

```toml
[target.aarch64-apple-darwin]
rustflags = "-lc++ -l framework=Accelerate"
```

**Feature flags** on whisper-rs 0.16: `metal`, `coreml`, `cuda`, `vulkan`, `hipblas`, `openblas`. For Magi on macOS that is `metal`. Also available: `log_backend` and `tracing_backend`, which route whisper.cpp's own log output into `log` or `tracing` instead of stderr — worth taking, since it puts the C++ layer's complaints in the same place as everything else.

This is a compile-time dependency on a C++ toolchain, which is the first one Magi has taken. It affects CI: the Linux test job will need it too, or the audio and STT modules cannot be compiled there — which is why the traits and fakes matter more here than anywhere else.

## The model download: the obvious checksum is the wrong one

Models come from `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-<name>.bin`.

**The `ETag` on that URL is not the SHA-256 of the file.** It looks exactly like one — 64 hex characters — which is what makes it dangerous. Verified by downloading `ggml-base.en.bin` and hashing it:

| Value | |
|---|---|
| Actual SHA-256 of the file | `a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002` |
| `lfs.sha256` from the HF API | `a03779c8…` — **matches** |
| `ETag` from the resolve URL | `ff7d10f8…` — **does not** |

So the checksum to verify against comes from the API:

```
https://huggingface.co/api/models/ggerganov/whisper.cpp?blobs=true
→ siblings[].lfs.sha256
```

Verifying against the ETag would fail every download on every machine, and the failure would look like a corrupt network rather than a wrong constant.

Sizes, measured, for the progress UI and for deciding what to default to:

| Model | Size |
|---|---|
| `ggml-base.en.bin` | 141 MB |
| `ggml-small.en.bin` | 465 MB |
| `ggml-medium.en.bin` | 1.4 GB |

`base.en` is the default. A 1.4 GB download on first launch of a tray app is not a first-run experience.

The download must be **resumable** — an HTTP range request against a partial file — because 465 MB on a hotel connection will fail, and a non-resumable download that fails at 90% starts again from zero.

## Traits and fakes, and why they carry more weight here

CI has no microphone. It also has no reason to compile whisper.cpp for a test of Magi's own logic.

| Trait | Fake |
|---|---|
| `AudioSource` | replays a fixture WAV, so the pipeline is exercised with real sample data |
| `Transcriber` | returns scripted text, including the empty and error cases |

Read `hound` for the fixture WAV: it is small, pure Rust, and does exactly one thing.

The parts worth testing are pure and have nothing to do with either crate:

- resampling — assert the output length and rate, not the audio
- the buffer cap and what happens when it fills
- WAV fixture decoding to the format Whisper needs
- the download's resume arithmetic and checksum comparison

## Microphone permission

macOS microphone access is a TCC permission, with the same failure mode as the others: no error, no log, and silence that looks like a working recording. `NSMicrophoneUsageDescription` in `Info.plist` is required or the process is killed on first access.

See `.claude/skills/macos-permissions/SKILL.md` — including the point that a rebuild changes the binary's signature, so a granted permission does not persist across `cargo build` in development the way it will for a signed release.

## Checklist before committing audio or STT code

- [ ] `stream.play()` is called after `build_input_stream`
- [ ] Sample rate and sample format are pinned from `supported_input_configs`, not assumed
- [ ] Audio is resampled to 16 kHz before it reaches the transcriber
- [ ] The data callback allocates nothing, locks nothing, logs nothing
- [ ] Inference runs on `spawn_blocking` — it is CPU-bound and takes seconds
- [ ] whisper.cpp's four print options are disabled
- [ ] Segment timestamps are treated as centiseconds
- [ ] Recording length is capped, and the cap is a defined behaviour rather than an allocation failure
- [ ] Device disconnect mid-recording is handled through the error callback, not ignored
- [ ] The model checksum comes from the HF API's `lfs.sha256`, never the ETag
- [ ] The download resumes rather than restarting
- [ ] `AudioSource` and `Transcriber` both have fakes, and the tests use them
- [ ] No test requires a microphone, a GPU, or the network
