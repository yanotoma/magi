//! Getting audio off the microphone and into the shape Whisper needs.
//!
//! A leaf module: it knows nothing about the rest of Magi. `session` calls it; it
//! calls nobody. That is what keeps it testable without a microphone, which CI does
//! not have.
//!
//! The format contract lives here rather than in `stt`, because `whisper-rs` does
//! not resample and a device will not hand you 16 kHz. [`AudioSource`] therefore
//! promises 16 kHz mono `f32` as its output — not "whatever the device gave us" —
//! so the fake and the real implementation produce the same thing and a fixture WAV
//! is a genuine test input rather than a separate code path.

pub mod format;
pub mod microphone;
pub mod source;

pub use microphone::Microphone;
pub use source::{AudioError, AudioSource, Captured, Ending, FakeSource};
