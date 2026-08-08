//! Reading the screen.
//!
//! A leaf, in the sense `CLAUDE.md` means it: nothing here knows that a session, a
//! provider or a config exists. `session.rs` decides *when* to look; this module answers
//! *what is on screen* and, in [`deixis`], *whether the words asked for it*.
//!
//! The split matters for the no-hardware-in-CI rule. [`deixis`] is pure text and runs
//! anywhere. The parts that touch a display sit behind a trait with a fake, so the tests
//! that matter — which display, what size, whether the words pointed at it — never need a
//! screen to run on.

pub mod deixis;
pub mod downscale;
pub mod encode;
pub mod source;

pub use deixis::{asks_about_the_screen, Deixis};
pub use downscale::{target_size, visual_tokens};
pub use encode::CaptureError;
pub use source::{Capture, DisplayInfo, FakeCapture, ScreenCapture, Subject, WindowInfo};
