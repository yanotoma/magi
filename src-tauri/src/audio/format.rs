//! Turning whatever the microphone gave us into what Whisper needs.
//!
//! Whisper requires **16 kHz, mono, `f32`**, and `whisper-rs` does not resample —
//! it converts integers to floats and stereo to mono, then stops. A microphone will
//! not give you 16 kHz; macOS built-in inputs are typically 48 kHz.
//!
//! Getting this wrong is silent, which is why it lives in its own module with its
//! own tests. Hand Whisper 48 kHz audio and it transcribes confidently and wrongly:
//! the samples are read as though they were 16 kHz, so speech comes out as
//! gibberish at the wrong speed, with no error anywhere to connect it to a sample
//! rate.
//!
//! Nothing here touches hardware. Every function takes samples and returns samples,
//! so the whole format contract is testable without a microphone.

/// What Whisper is trained on and the only rate it accepts.
pub const TARGET_RATE: u32 = 16_000;

/// Folds interleaved channels down to one.
///
/// Averaged rather than taking the first channel. On a stereo input the two
/// channels are rarely identical — a laptop's array mic, or one channel carrying
/// mostly room noise — and discarding one throws away signal for no gain.
///
/// A trailing partial frame is dropped. It can only happen if the device handed us
/// a buffer that is not a whole number of frames, which means something upstream is
/// already wrong; averaging across a frame boundary would turn that into quiet
/// distortion rather than a missing sample.
pub fn to_mono(interleaved: &[f32], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }

    let channels = channels as usize;
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Resamples mono audio to [`TARGET_RATE`].
///
/// Linear interpolation, preceded by a box low-pass when downsampling.
///
/// `rubato` is the obvious crate for this and does far more than is needed:
/// arbitrary ratios, several interpolation qualities, streaming and fixed-size
/// modes. What Magi needs is one fixed conversion, of speech, for a model trained
/// on 16 kHz speech and robust to far worse than interpolation error. This is
/// thirty lines that can be tested exactly.
///
/// The low-pass is the part that matters and the part that gets left out. Dropping
/// samples to go from 48 kHz to 16 kHz folds everything above 8 kHz back down into
/// the audible range as aliasing — sibilants become tones, and the model hears
/// speech with noise laid over it. Averaging each group of source samples before
/// interpolating removes most of it, which is enough here.
///
/// If transcription quality turns out to be limited by this rather than by the
/// model, `rubato` is where to go. The decision is recorded in the M4 plan so it is
/// made on evidence rather than re-argued.
pub fn resample(samples: &[f32], from_rate: u32) -> Vec<f32> {
    if samples.is_empty() || from_rate == 0 || from_rate == TARGET_RATE {
        return samples.to_vec();
    }

    let ratio = TARGET_RATE as f64 / from_rate as f64;
    let output_len = ((samples.len() as f64) * ratio).round() as usize;
    if output_len == 0 {
        return Vec::new();
    }

    // Downsampling only. Interpolating upward invents no energy above the original
    // Nyquist limit, so there is nothing to filter out.
    let window = if from_rate > TARGET_RATE {
        (from_rate as f64 / TARGET_RATE as f64).round().max(1.0) as usize
    } else {
        1
    };

    let mut out = Vec::with_capacity(output_len);
    for i in 0..output_len {
        // Where this output sample sits in the input.
        let position = i as f64 / ratio;
        let left = position.floor() as usize;

        let value = if window > 1 {
            // Average the window centred on `position`, which is the low-pass.
            let start = left.saturating_sub(window / 2);
            let end = (start + window).min(samples.len());
            let slice = &samples[start.min(samples.len().saturating_sub(1))..end.max(start + 1)];
            slice.iter().sum::<f32>() / slice.len() as f32
        } else {
            // Linear interpolation between neighbouring samples.
            let right = (left + 1).min(samples.len() - 1);
            let fraction = (position - left as f64) as f32;
            let a = samples[left.min(samples.len() - 1)];
            let b = samples[right];
            a + (b - a) * fraction
        };

        out.push(value);
    }

    out
}

/// The longest recording Magi will keep, in seconds.
///
/// Push-to-talk has no natural end: a stuck key, a hotkey held by an app that took
/// focus, a user who walked away. At 16 kHz mono `f32` the buffer grows 64 kB per
/// second — slow enough to be invisible, fast enough to matter over hours.
///
/// Two minutes is far longer than anyone speaks into an overlay panel and short
/// enough that hitting it is a bounded amount of memory.
pub const MAX_RECORDING_SECONDS: usize = 120;

/// The sample count that corresponds to [`MAX_RECORDING_SECONDS`].
pub const MAX_SAMPLES: usize = MAX_RECORDING_SECONDS * TARGET_RATE as usize;

/// What happened when samples were added to a recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capacity {
    /// Room remains.
    Accepted,
    /// The cap was reached. The recording holds everything up to the limit and
    /// should be stopped and transcribed.
    ///
    /// Deliberately not an error. The user said something, and the last thing to do
    /// with it is discard it because they said too much.
    Full,
}

/// A recording buffer with a ceiling.
///
/// Pre-allocated, because the thing appending to it may be a realtime audio
/// callback: growing a `Vec` there can block for longer than the audio deadline,
/// and the symptom is dropped samples that read as a transcription problem several
/// layers from the cause.
#[derive(Debug)]
pub struct Recording {
    samples: Vec<f32>,
}

impl Default for Recording {
    fn default() -> Self {
        Self::new()
    }
}

impl Recording {
    pub fn new() -> Self {
        Self {
            samples: Vec::with_capacity(MAX_SAMPLES),
        }
    }

    /// Appends what fits, and says whether the cap was reached.
    ///
    /// A partial write is deliberate: filling to exactly the limit keeps the
    /// boundary a property of the buffer rather than of how the device happened to
    /// chunk its callbacks.
    pub fn push(&mut self, samples: &[f32]) -> Capacity {
        let room = MAX_SAMPLES.saturating_sub(self.samples.len());
        if room == 0 {
            return Capacity::Full;
        }

        let take = samples.len().min(room);
        self.samples.extend_from_slice(&samples[..take]);

        if self.samples.len() >= MAX_SAMPLES {
            Capacity::Full
        } else {
            Capacity::Accepted
        }
    }

    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// How long the recording is, in seconds.
    pub fn duration_seconds(&self) -> f32 {
        self.samples.len() as f32 / TARGET_RATE as f32
    }

    /// Takes the samples, leaving the buffer ready to record again.
    pub fn take(&mut self) -> Vec<f32> {
        std::mem::replace(&mut self.samples, Vec::with_capacity(MAX_SAMPLES))
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

/// Whether a recording holds enough audio to be worth transcribing.
///
/// A hotkey tapped rather than held produces a few dozen milliseconds of room tone.
/// Whisper will transcribe that, and what it returns is not silence — it
/// hallucinates, usually with something from its training data like "Thank you." or
/// a subtitle credit. Sending it is worse than sending nothing, because the user
/// gets a confident answer to a question they never asked.
pub fn worth_transcribing(samples: &[f32]) -> bool {
    const MIN_SECONDS: f32 = 0.25;
    samples.len() as f32 / TARGET_RATE as f32 >= MIN_SECONDS
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sine wave, for testing that resampling preserves a tone rather than
    /// aliasing it into something else.
    fn tone(hz: f32, rate: u32, seconds: f32) -> Vec<f32> {
        let count = (rate as f32 * seconds) as usize;
        (0..count)
            .map(|i| {
                let t = i as f32 / rate as f32;
                (t * hz * std::f32::consts::TAU).sin()
            })
            .collect()
    }

    /// Compares floats with a tolerance.
    ///
    /// Needed because averaging is division. `[0.6; 6]` sums to 3.5999999 and
    /// averages to 0.59999996 — the mono tests originally used `assert_eq!` and one
    /// of them passed only because it happened to use 1.0, where `1.0 + 1.0 = 2.0`
    /// and `2.0 / 2 = 1.0` are both exact in binary. Powers of two hide this; any
    /// ordinary decimal exposes it.
    fn close(a: &[f32], b: &[f32]) {
        assert_eq!(a.len(), b.len(), "lengths differ: {a:?} vs {b:?}");
        for (x, y) in a.iter().zip(b) {
            assert!(
                (x - y).abs() < 1e-6,
                "{a:?} and {b:?} differ by more than the float tolerance"
            );
        }
    }

    /// Root mean square, as a stand-in for "how much signal is here".
    fn rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    // ---- mono folding ------------------------------------------------------

    #[test]
    fn mono_input_passes_through() {
        let samples = vec![0.1, 0.2, 0.3];
        assert_eq!(to_mono(&samples, 1), samples);
        // A device reporting zero channels is nonsense; returning the input
        // unchanged is better than dividing by zero.
        assert_eq!(to_mono(&samples, 0), samples);
    }

    #[test]
    fn stereo_is_averaged_not_halved() {
        // Averaged, so a full-scale signal in both channels stays full scale.
        let interleaved = vec![1.0, 1.0, -1.0, -1.0];
        close(&to_mono(&interleaved, 2), &[1.0, -1.0]);
    }

    #[test]
    fn stereo_keeps_signal_that_is_only_in_one_channel() {
        // Taking the first channel instead of averaging would return silence here.
        // A laptop array mic routinely puts most of the voice in one channel.
        let interleaved = vec![0.0, 0.8, 0.0, 0.6];
        let mono = to_mono(&interleaved, 2);
        assert!(rms(&mono) > 0.3, "signal from one channel was discarded");
    }

    #[test]
    fn a_trailing_partial_frame_is_dropped() {
        // Five samples across two channels is two frames and a leftover. Averaging
        // across the boundary would produce quiet distortion instead of a missing
        // sample.
        close(&to_mono(&[1.0, 1.0, 2.0, 2.0, 3.0], 2), &[1.0, 2.0]);
    }

    #[test]
    fn six_channels_fold_to_one() {
        close(&to_mono(&[0.6; 6], 6), &[0.6]);
    }

    // ---- resampling --------------------------------------------------------

    #[test]
    fn audio_already_at_the_target_rate_is_untouched() {
        let samples = tone(440.0, TARGET_RATE, 0.1);
        assert_eq!(resample(&samples, TARGET_RATE), samples);
    }

    #[test]
    fn downsampling_from_48k_produces_a_third_of_the_samples() {
        // The rate every macOS built-in input actually reports.
        let samples = tone(440.0, 48_000, 1.0);
        let out = resample(&samples, 48_000);
        assert_eq!(out.len(), TARGET_RATE as usize);
    }

    #[test]
    fn downsampling_from_44_1k_gets_the_length_right() {
        // Not an integer ratio, which is where an implementation that divides by a
        // whole number goes wrong.
        let samples = tone(440.0, 44_100, 1.0);
        let out = resample(&samples, 44_100);
        let expected = TARGET_RATE as usize;
        assert!(
            out.len().abs_diff(expected) <= 1,
            "expected about {expected} samples, got {}",
            out.len()
        );
    }

    #[test]
    fn upsampling_works_too() {
        // Rare, but 8 kHz inputs exist — some Bluetooth headsets negotiate it.
        let samples = tone(300.0, 8_000, 0.5);
        let out = resample(&samples, 8_000);
        assert_eq!(out.len(), (TARGET_RATE as f64 * 0.5) as usize);
    }

    #[test]
    fn a_speech_range_tone_survives_downsampling() {
        // 1 kHz is inside speech and well under the 8 kHz Nyquist limit of the
        // target rate, so it must come through with its energy intact.
        let samples = tone(1_000.0, 48_000, 0.5);
        let out = resample(&samples, 48_000);

        let before = rms(&samples);
        let after = rms(&out);
        assert!(
            after > before * 0.5,
            "a 1 kHz tone lost most of its energy: {before} -> {after}"
        );
    }

    #[test]
    fn a_tone_above_the_target_nyquist_is_attenuated_rather_than_aliased() {
        // The test the low-pass exists for. 15 kHz cannot be represented at 16 kHz;
        // without filtering it folds down to 1 kHz and the model hears a tone that
        // was never spoken. It must come out quiet, not relocated.
        let samples = tone(15_000.0, 48_000, 0.5);
        let out = resample(&samples, 48_000);

        assert!(
            rms(&out) < rms(&samples) * 0.5,
            "15 kHz was not attenuated: it has aliased into the audible range \
             ({} -> {})",
            rms(&samples),
            rms(&out)
        );
    }

    #[test]
    fn empty_and_degenerate_input_do_not_panic() {
        assert!(resample(&[], 48_000).is_empty());
        assert!(
            resample(&[0.5], 0).len() == 1,
            "a zero rate is passed through"
        );
        // One sample at a huge ratio rounds to zero output samples.
        assert!(resample(&[0.5], 1_000_000).is_empty());
    }

    #[test]
    fn a_single_sample_at_a_sane_rate_does_not_index_out_of_bounds() {
        let out = resample(&[0.5], 48_000);
        assert!(out.len() <= 1);
    }

    // ---- the recording buffer ---------------------------------------------

    #[test]
    fn a_new_recording_is_empty_and_preallocated() {
        let recording = Recording::new();
        assert!(recording.is_empty());
        assert_eq!(recording.duration_seconds(), 0.0);
    }

    #[test]
    fn pushing_accumulates() {
        let mut recording = Recording::new();
        assert_eq!(recording.push(&[0.1; 100]), Capacity::Accepted);
        assert_eq!(recording.push(&[0.2; 50]), Capacity::Accepted);
        assert_eq!(recording.len(), 150);
    }

    #[test]
    fn the_cap_fills_exactly_rather_than_overshooting() {
        // The boundary is a property of the buffer, not of how the device happened
        // to chunk its callbacks.
        let mut recording = Recording::new();
        assert_eq!(
            recording.push(&vec![0.1; MAX_SAMPLES - 10]),
            Capacity::Accepted
        );
        assert_eq!(recording.push(&[0.1; 100]), Capacity::Full);
        assert_eq!(recording.len(), MAX_SAMPLES);
    }

    #[test]
    fn a_full_recording_keeps_what_it_has() {
        // Reaching the cap is not an error and must not discard audio. The user said
        // something; the last thing to do with it is throw it away because they said
        // too much.
        let mut recording = Recording::new();
        recording.push(&vec![0.7; MAX_SAMPLES + 5_000]);

        assert_eq!(recording.len(), MAX_SAMPLES);
        assert!(recording.samples().iter().all(|&s| s == 0.7));
        assert_eq!(recording.push(&[0.1; 10]), Capacity::Full);
        assert_eq!(
            recording.len(),
            MAX_SAMPLES,
            "a full buffer stopped growing"
        );
    }

    #[test]
    fn duration_is_reported_in_seconds_of_target_rate_audio() {
        let mut recording = Recording::new();
        recording.push(&vec![0.0; TARGET_RATE as usize * 3]);
        assert_eq!(recording.duration_seconds(), 3.0);
    }

    #[test]
    fn the_cap_is_the_documented_number_of_seconds() {
        let mut recording = Recording::new();
        recording.push(&vec![0.0; MAX_SAMPLES]);
        assert_eq!(
            recording.duration_seconds(),
            MAX_RECORDING_SECONDS as f32,
            "MAX_SAMPLES and MAX_RECORDING_SECONDS have drifted apart"
        );
    }

    #[test]
    fn taking_leaves_the_buffer_reusable() {
        let mut recording = Recording::new();
        recording.push(&[0.5; 32]);

        let taken = recording.take();
        assert_eq!(taken.len(), 32);
        assert!(recording.is_empty());
        assert_eq!(recording.push(&[0.1; 8]), Capacity::Accepted);
    }

    // ---- the silence guard -------------------------------------------------

    #[test]
    fn a_tapped_hotkey_is_not_worth_transcribing() {
        // Whisper does not return silence for near-silence — it hallucinates,
        // usually something from its training data like "Thank you." Sending that is
        // worse than sending nothing, because the user gets a confident answer to a
        // question they never asked.
        let tap = vec![0.0; (TARGET_RATE as f32 * 0.1) as usize];
        assert!(!worth_transcribing(&tap));
        assert!(!worth_transcribing(&[]));
    }

    #[test]
    fn a_held_hotkey_is_worth_transcribing() {
        let held = vec![0.0; (TARGET_RATE as f32 * 1.5) as usize];
        assert!(worth_transcribing(&held));
    }

    #[test]
    fn the_threshold_is_about_length_not_loudness() {
        // Quiet speech is still speech. Gating on amplitude would drop a whispered
        // question, and deciding what is loud enough is exactly the judgement the
        // model is better at than a threshold here.
        let quiet = vec![0.001; TARGET_RATE as usize];
        assert!(worth_transcribing(&quiet));
    }
}
