//! The real microphone, via `cpal`.
//!
//! The only part of `audio` that touches hardware, and deliberately the thinnest:
//! every decision about format, folding, capping and resampling is in
//! [`crate::audio::format`], which is testable with nothing installed. What is left
//! here is opening a device and copying samples out of a callback.
//!
//! Three things in this file exist because of documented cpal 0.18 behaviour rather
//! than preference. See `.claude/skills/audio-stt/SKILL.md`.

use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, FromSample, SampleFormat, SizedSample};

use crate::audio::format::{self, Capacity, Recording};
use crate::audio::source::{AudioError, AudioSource, Captured, Ending};

/// What the capture thread and the caller share.
struct Shared {
    recording: Recording,
    /// Set by cpal's error callback. The device going away mid-sentence should not
    /// lose the sentence, so this is a flag read on `stop` rather than a failure.
    disconnected: bool,
    /// Set when the length cap is reached, so `stop` can say why it ended.
    capped: bool,
}

pub struct Microphone {
    /// Held so the stream stays alive; dropping it is what stops capture.
    ///
    /// Stored directly rather than owned by a dedicated thread. The widely repeated
    /// advice is that `cpal::Stream` is `!Send` and must live on its own thread with
    /// a command channel — that stopped being true in cpal 0.17, which made streams
    /// `Send + Sync` on every platform. Following the old advice would have cost
    /// fifty lines of threading for a problem that no longer exists.
    stream: Mutex<Option<cpal::Stream>>,
    shared: Arc<Mutex<Shared>>,
}

impl Default for Microphone {
    fn default() -> Self {
        Self::new()
    }
}

impl Microphone {
    pub fn new() -> Self {
        Self {
            stream: Mutex::new(None),
            shared: Arc::new(Mutex::new(Shared {
                recording: Recording::new(),
                disconnected: false,
                capped: false,
            })),
        }
    }

    /// Lists the input devices, for Settings.
    ///
    /// Errors on individual devices are skipped rather than propagated: one
    /// misbehaving virtual device should not empty the list.
    pub fn input_devices() -> Result<Vec<String>, AudioError> {
        let host = cpal::default_host();
        let devices = host.input_devices().map_err(classify)?;

        // `description()`, not `name()` — 0.18 replaced the latter with a struct
        // carrying the manufacturer, driver and interface type as well.
        Ok(devices
            .filter_map(|device| device.description().ok())
            .map(|description| description.name().to_string())
            .collect())
    }
}

/// Picks a config for the default input device.
///
/// Both the rate and the format are taken explicitly rather than from
/// `default_input_config()`, because cpal 0.18 changed what that returns: the default
/// is no longer 44.1 kHz, and `I32`/`I24` now outrank `I16` in the selection order.
/// Code that assumed either still compiles.
///
/// The rate closest to 16 kHz is preferred, since every sample above it is thrown
/// away by resampling. A device that can give 16 kHz directly skips resampling
/// entirely.
fn choose_config(device: &cpal::Device) -> Result<cpal::SupportedStreamConfig, AudioError> {
    let ranges: Vec<_> = device
        .supported_input_configs()
        .map_err(classify)?
        .collect();

    if ranges.is_empty() {
        return Err(AudioError::Unavailable(
            "the device reports no supported input configurations".into(),
        ));
    }

    // Prefer a range that spans the target rate; otherwise the one whose maximum is
    // closest to it.
    let best = ranges
        .iter()
        .min_by_key(|range| {
            let min = range.min_sample_rate();
            let max = range.max_sample_rate();
            if (min..=max).contains(&format::TARGET_RATE) {
                0u32
            } else if max < format::TARGET_RATE {
                format::TARGET_RATE - max
            } else {
                min - format::TARGET_RATE
            }
        })
        .ok_or(AudioError::NoDevice)?;

    // `SampleRate` is a plain `u32` in 0.18; it was a tuple struct in 0.17, so the
    // familiar `SampleRate(x)` no longer compiles.
    let min = best.min_sample_rate();
    let max = best.max_sample_rate();
    // `Copy` in 0.18, so no clone is needed to consume it.
    Ok((*best).with_sample_rate(format::TARGET_RATE.clamp(min, max)))
}

/// Builds a typed input stream for one sample format.
///
/// Generic because cpal hands the callback the device's own type, and the format is
/// only known at runtime — so `start` matches on it and calls this once per arm.
fn build_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    shared: Arc<Mutex<Shared>>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels.max(1) as usize;
    let errors = Arc::clone(&shared);

    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            // `try_lock`, never `lock`. This is an OS realtime thread with a
            // deadline; blocking here can stall the whole audio system, which is
            // worse than losing a buffer. The only other holder is `stop`, which
            // happens once, so a miss is close to impossible — and counted rather
            // than ignored when it does happen.
            let Ok(mut shared) = shared.try_lock() else {
                return;
            };

            if shared.capped {
                return;
            }

            // Converted and folded to mono inline. `format::to_mono` allocates, and
            // allocation on this thread is the thing that causes dropouts.
            let outcome = shared
                .recording
                .extend_mono(data.chunks_exact(channels).map(|frame| {
                    frame.iter().map(|&s| f32::from_sample_(s)).sum::<f32>() / channels as f32
                }));

            if outcome == Capacity::Full {
                shared.capped = true;
            }
        },
        move |error| {
            // A device error mid-recording. Flagged rather than logged and forgotten,
            // so `stop` can report why the recording ended and still return what it
            // captured — unplugging a USB microphone mid-sentence should not lose the
            // sentence.
            // `Xrun` is a dropout rather than a disconnection — the stream is still
            // alive and the recording is still usable, so it is logged and not
            // allowed to end the capture.
            if error.kind() == ErrorKind::Xrun {
                tracing::warn!(%error, "the audio system dropped samples");
                return;
            }

            tracing::warn!(%error, kind = ?error.kind(), "the input stream failed");
            if let Ok(mut shared) = errors.lock() {
                shared.disconnected = true;
            }
        },
        None,
    )
}

impl AudioSource for Microphone {
    fn start(&self) -> Result<(), AudioError> {
        let mut slot = self
            .stream
            .lock()
            .map_err(|_| AudioError::Unavailable("the stream lock was poisoned".into()))?;

        // A repeated start is not an error: an OS delivers key-repeat for a held
        // hotkey, and refusing the second one would turn that into a visible failure.
        if slot.is_some() {
            return Ok(());
        }

        let host = cpal::default_host();
        let device = host.default_input_device().ok_or(AudioError::NoDevice)?;
        let supported = choose_config(&device)?;
        let sample_format = supported.sample_format();
        let config = supported.config();

        {
            let mut shared = self
                .shared
                .lock()
                .map_err(|_| AudioError::Unavailable("the buffer lock was poisoned".into()))?;
            shared.recording = Recording::at_rate(config.sample_rate);
            shared.disconnected = false;
            shared.capped = false;
        }

        let shared = Arc::clone(&self.shared);
        let stream = match sample_format {
            SampleFormat::F32 => build_stream::<f32>(&device, config, shared),
            SampleFormat::I16 => build_stream::<i16>(&device, config, shared),
            SampleFormat::I32 => build_stream::<i32>(&device, config, shared),
            SampleFormat::I8 => build_stream::<i8>(&device, config, shared),
            SampleFormat::U8 => build_stream::<u8>(&device, config, shared),
            SampleFormat::U16 => build_stream::<u16>(&device, config, shared),
            other => {
                return Err(AudioError::Unavailable(format!(
                    "the microphone offers only {other:?} samples, which Magi cannot read"
                )))
            }
        }
        .map_err(classify)?;

        // Required. cpal 0.18 no longer auto-starts streams on CoreAudio, and without
        // this `build_input_stream` succeeds, the callback never fires, and the
        // recording is perfect silence with no error anywhere to explain it.
        stream.play().map_err(classify)?;

        tracing::info!(
            rate = config.sample_rate,
            channels = config.channels,
            format = ?sample_format,
            "recording started"
        );

        *slot = Some(stream);
        Ok(())
    }

    fn stop(&self) -> Result<Captured, AudioError> {
        let mut slot = self
            .stream
            .lock()
            .map_err(|_| AudioError::Unavailable("the stream lock was poisoned".into()))?;

        // Nothing was recording. An error rather than empty samples, which would
        // reach the transcriber as "the user said nothing" — a different claim.
        if slot.take().is_none() {
            return Err(AudioError::Empty);
        }
        // Dropping the stream stops the callback, so nothing can be appended after
        // this point and the buffer below is final.

        let (samples, rate, ending, dropped) = {
            let mut shared = self
                .shared
                .lock()
                .map_err(|_| AudioError::Unavailable("the buffer lock was poisoned".into()))?;

            let rate = shared.recording.rate();
            let dropped = shared.recording.dropped();
            // Disconnection is reported ahead of the cap: if both happened, the device
            // going away is the more useful thing to tell the user.
            let ending = if shared.disconnected {
                Ending::Disconnected
            } else if shared.capped {
                Ending::CapReached
            } else {
                Ending::Released
            };

            (shared.recording.take(), rate, ending, dropped)
        };

        if dropped > 0 {
            tracing::warn!(
                dropped,
                "the audio callback discarded buffers rather than blocking a realtime thread"
            );
        }

        // Resampled here rather than in the callback. It is cheap, but not
        // realtime-cheap, and doing it once at the end costs nothing.
        let samples = format::resample(&samples, rate);

        tracing::info!(
            seconds = samples.len() as f32 / format::TARGET_RATE as f32,
            ?ending,
            "recording stopped"
        );

        Ok(Captured { samples, ending })
    }

    fn is_recording(&self) -> bool {
        self.stream
            .lock()
            .map(|slot| slot.is_some())
            .unwrap_or(false)
    }
}

/// Turns a cpal failure into something a user can act on.
///
/// cpal 0.18 consolidated every failure into one `Error` carrying an [`ErrorKind`],
/// and that is a real improvement here: `PermissionDenied` is a first-class variant.
/// The alternative — which is what this function was first written to do — was
/// sniffing for the word "permission" inside a backend-specific message, a fragile
/// match against text that can change in any release.
///
/// The permission case is the one that matters most. Reported as broken hardware, the
/// user checks their microphone; the fix is a System Settings pane they would never
/// think to look in from a message about a device.
fn classify(error: cpal::Error) -> AudioError {
    match error.kind() {
        ErrorKind::PermissionDenied => AudioError::PermissionDenied,

        // The device is gone or was swapped out from under us. Distinct from "no
        // device" because something was working a moment ago.
        ErrorKind::DeviceChanged | ErrorKind::StreamInvalidated => AudioError::Disconnected,

        ErrorKind::DeviceNotAvailable | ErrorKind::HostUnavailable => AudioError::NoDevice,

        ErrorKind::DeviceBusy => {
            AudioError::Unavailable("the microphone is in use by another application".into())
        }

        ErrorKind::UnsupportedConfig | ErrorKind::UnsupportedOperation => AudioError::Unavailable(
            "the microphone rejected every configuration Magi offered".into(),
        ),

        // Not a permission dialog: macOS declined to give the process realtime audio
        // priority. Recording still works, with a higher chance of dropouts.
        ErrorKind::RealtimeDenied => AudioError::Unavailable(
            "the system would not grant audio priority; recording may drop samples".into(),
        ),

        other => AudioError::Unavailable(format!("{error} ({other:?})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Nothing here opens a device. CI has no microphone, and the parts of this module
    // worth testing are the classification and the config choice — the rest is a
    // `build_input_stream` call whose behaviour belongs to cpal.

    #[test]
    fn a_denied_permission_is_classified_from_the_kind_not_from_a_message() {
        // cpal 0.18 gives this as a typed variant. The first version of `classify`
        // searched backend-specific text for the word "permission", which is a match
        // against prose that can change in any release.
        let classified = classify(cpal::Error::new(ErrorKind::PermissionDenied));
        assert!(matches!(classified, AudioError::PermissionDenied));
        assert!(
            classified.to_string().contains("System Settings"),
            "the message has to say where the fix is"
        );
    }

    #[test]
    fn a_device_that_went_away_is_a_disconnection_not_a_missing_device() {
        // Something was working a moment ago, which is a different thing to tell the
        // user than "there is no microphone".
        for kind in [ErrorKind::DeviceChanged, ErrorKind::StreamInvalidated] {
            assert!(matches!(
                classify(cpal::Error::new(kind)),
                AudioError::Disconnected
            ));
        }

        for kind in [ErrorKind::DeviceNotAvailable, ErrorKind::HostUnavailable] {
            assert!(matches!(
                classify(cpal::Error::new(kind)),
                AudioError::NoDevice
            ));
        }
    }

    #[test]
    fn a_busy_device_says_another_application_has_it() {
        let message = classify(cpal::Error::new(ErrorKind::DeviceBusy)).to_string();
        assert!(
            message.contains("another application"),
            "the user needs to know it is not broken: {message}"
        );
    }

    #[test]
    fn an_unsupported_config_says_what_was_rejected() {
        let message = classify(cpal::Error::new(ErrorKind::UnsupportedConfig)).to_string();
        assert!(message.contains("configuration"), "got: {message}");
    }

    #[test]
    fn an_unknown_kind_still_carries_the_original_error() {
        // A variant added in a future cpal release must not become an empty message.
        let message = classify(cpal::Error::new(ErrorKind::Other)).to_string();
        assert!(!message.is_empty());
        assert!(
            message.contains("Other"),
            "the kind must survive: {message}"
        );
    }

    #[test]
    fn the_microphone_starts_out_not_recording() {
        // Constructing one opens nothing, which is what makes this safe to assert in
        // CI.
        let microphone = Microphone::new();
        assert!(!microphone.is_recording());
    }

    #[test]
    fn stopping_a_microphone_that_never_started_is_an_error() {
        let microphone = Microphone::new();
        assert!(matches!(microphone.stop(), Err(AudioError::Empty)));
    }

    #[test]
    fn the_real_microphone_satisfies_the_trait() {
        // `session` holds a `Box<dyn AudioSource>`, so this has to be object-safe and
        // `Send + Sync`. Both are compile-time facts; the test writes them down.
        fn assert_source<T: AudioSource + 'static>(_: T) {}
        assert_source(Microphone::new());

        let boxed: Box<dyn AudioSource> = Box::new(Microphone::new());
        assert!(!boxed.is_recording());
    }
}
