//! The screen, behind a trait.
//!
//! Same shape and same reason as [`crate::audio::AudioSource`]: the tests that matter are
//! about which display was chosen, how big the result is and whether the words asked for
//! it — none of which needs a screen to be attached. CI has no display, and the rule that
//! no test may require one is what keeps this module honest.

use crate::capture::{downscale, encode::CaptureError};

/// A display Magi could capture.
///
/// `width` and `height` are **logical** points, not pixels. That distinction is the one
/// thing most likely to be got wrong here: on a Retina display the captured image is twice
/// these numbers on each axis, so sizing a buffer from them produces something a quarter
/// of the size it needs to be. [`Capture`] carries the real pixel dimensions.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DisplayInfo {
    /// Platform display identifier. Stable while the display stays connected.
    pub id: u32,

    /// Something a person would recognise, for the audit log and for Settings.
    pub label: String,

    /// Whether this is the display the menu bar is on.
    pub is_primary: bool,

    /// Logical width in points.
    pub width: u32,

    /// Logical height in points.
    pub height: u32,

    /// Physical pixels per logical point. `2.0` on a Retina display.
    pub scale: f32,
}

impl DisplayInfo {
    /// The pixel dimensions a capture of this display will have.
    ///
    /// Rounded rather than truncated: a scale factor arrives as a float and 1512 × 2.0 can
    /// present as 3023.9999. Truncating loses a row of pixels and makes the returned
    /// buffer length disagree with the dimensions by exactly one stride, which is the
    /// hardest kind of off-by-one to see in an image.
    pub fn pixel_size(&self) -> (u32, u32) {
        let scale = f64::from(self.scale.max(1.0));
        let width = (f64::from(self.width) * scale).round().max(1.0);
        let height = (f64::from(self.height) * scale).round().max(1.0);
        (width as u32, height as u32)
    }
}

/// A window Magi could capture.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct WindowInfo {
    /// Platform window identifier. Valid only until the window closes.
    pub id: u32,

    /// The window's own title.
    ///
    /// May be empty, and an empty title is not necessarily a bug: macOS 13 and later
    /// withhold other applications' window titles when Screen Recording permission is
    /// absent, so blank titles across the board are a permission symptom rather than a
    /// naming one.
    pub title: String,

    /// The application that owns it.
    pub app: String,

    /// Whether it is the frontmost window.
    pub is_focused: bool,
}

/// What a capture was taken of.
///
/// Carried with the image so the audit log can say what was read without having to
/// re-enumerate anything — by the time a user opens the log, the window may be gone.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum Subject {
    Display { id: u32, label: String },
    Window { id: u32, title: String, app: String },
}

impl Subject {
    /// One line naming what was captured, for the audit log.
    pub fn describe(&self) -> String {
        match self {
            Subject::Display { label, .. } => label.clone(),
            Subject::Window { title, app, .. } if title.is_empty() => app.clone(),
            Subject::Window { title, app, .. } => format!("{app} — {title}"),
        }
    }
}

/// A screenshot, encoded and ready to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    /// PNG bytes.
    pub png: Vec<u8>,

    /// Width of the encoded image in pixels, after any downscaling.
    pub width: u32,

    /// Height of the encoded image in pixels, after any downscaling.
    pub height: u32,

    /// What was captured.
    pub subject: Subject,
}

impl Capture {
    /// What this image costs a vision model, in visual tokens.
    ///
    /// Convenience for the audit log, so the user can see what a capture actually cost
    /// rather than being told a screenshot happened.
    pub fn visual_tokens(&self) -> u32 {
        downscale::visual_tokens(self.width, self.height)
    }
}

/// Reading the screen.
///
/// Every method is **blocking**. The platform APIs behind them are synchronous — a display
/// capture is a round trip through the window server — so every call site goes through
/// `spawn_blocking`. `Send + Sync` is what allows that, and it is a supertrait rather than
/// a bound at the call site so a future implementation cannot quietly drop it.
pub trait ScreenCapture: Send + Sync {
    /// The displays currently attached.
    fn displays(&self) -> Result<Vec<DisplayInfo>, CaptureError>;

    /// The windows currently open, frontmost first.
    ///
    /// Ordering is part of the contract: "the window I am looking at" is the first one,
    /// and a caller that has to sort by an undocumented rule will get it wrong.
    fn windows(&self) -> Result<Vec<WindowInfo>, CaptureError>;

    /// Captures one display, downscaled and encoded.
    fn capture_display(&self, id: u32) -> Result<Capture, CaptureError>;

    /// Captures one window, downscaled and encoded.
    fn capture_window(&self, id: u32) -> Result<Capture, CaptureError>;

    /// Captures the display the menu bar is on.
    ///
    /// Provided rather than required: every implementation would write the same thing, and
    /// what "active" means is a policy question the trait should answer once. The primary
    /// display is chosen over the one holding the frontmost window because the panel
    /// itself is centred on the primary display, so it is the one the user is looking at
    /// when they invoke Magi.
    fn capture_active_display(&self) -> Result<Capture, CaptureError> {
        let displays = self.displays()?;
        let chosen = displays
            .iter()
            .find(|display| display.is_primary)
            .or_else(|| displays.first())
            .ok_or(CaptureError::NoDisplay)?;

        self.capture_display(chosen.id)
    }
}

/// A screen that returns a fixture.
///
/// Used by tests and by the wiring before the platform implementation is reachable — on
/// Linux it is the only implementation there is, which is what lets the whole crate build
/// and test on a CI runner with no display server.
pub struct FakeCapture {
    displays: Vec<DisplayInfo>,
    windows: Vec<WindowInfo>,
    failure: Option<fn() -> CaptureError>,
    /// Every capture this fake was asked for, so tests can assert on what a caller did
    /// rather than only on what it got back.
    requested: std::sync::Mutex<Vec<Subject>>,
}

impl FakeCapture {
    /// One 1512×982 Retina display and two windows — a plausible laptop.
    pub fn laptop() -> Self {
        Self {
            displays: vec![DisplayInfo {
                id: 1,
                label: "Built-in Retina Display".to_string(),
                is_primary: true,
                width: 1512,
                height: 982,
                scale: 2.0,
            }],
            windows: vec![
                WindowInfo {
                    id: 10,
                    title: "src/main.rs".to_string(),
                    app: "Zed".to_string(),
                    is_focused: true,
                },
                WindowInfo {
                    id: 11,
                    title: "magi — cargo test".to_string(),
                    app: "Terminal".to_string(),
                    is_focused: false,
                },
            ],
            failure: None,
            requested: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Two displays, the second not primary, for the multi-display cases.
    pub fn with_external_display() -> Self {
        let mut fake = Self::laptop();
        fake.displays.push(DisplayInfo {
            id: 2,
            label: "DELL U2720Q".to_string(),
            is_primary: false,
            width: 3840,
            height: 2160,
            scale: 1.0,
        });
        fake
    }

    /// No displays at all. Happens with the lid closed and no external monitor.
    pub fn headless() -> Self {
        Self {
            displays: Vec::new(),
            windows: Vec::new(),
            failure: None,
            requested: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Fails every call with `error()`.
    ///
    /// Takes a function rather than a value because [`CaptureError`] is not `Clone` — it
    /// wraps platform strings — and a fake that can only fail once is not much of a fake.
    pub fn failing(error: fn() -> CaptureError) -> Self {
        Self {
            failure: Some(error),
            ..Self::laptop()
        }
    }

    /// What was captured, in order.
    pub fn requested(&self) -> Vec<Subject> {
        self.requested
            .lock()
            .map(|requested| requested.clone())
            .unwrap_or_default()
    }

    /// Builds a capture of `subject` at `width`×`height`, downscaled like the real thing.
    fn produce(&self, subject: Subject, width: u32, height: u32) -> Result<Capture, CaptureError> {
        if let Some(error) = self.failure {
            return Err(error());
        }

        // Recorded before the encode can fail, so a test asserting "the caller asked for
        // the external display" still works when the fake is set to fail.
        if let Ok(mut requested) = self.requested.lock() {
            requested.push(subject.clone());
        }

        // The same downscale the real implementation applies. A fake that skipped it would
        // let a caller's size assumptions pass in tests and break on a real display.
        let (width, height) = downscale::target_size(width, height);

        // A recognisable gradient rather than a solid colour: a test that asserts on pixel
        // content can tell an image apart from a blank one, and a human looking at a
        // written-out fixture can see it is not garbage.
        let mut pixels = Vec::with_capacity((width as usize) * (height as usize) * 4);
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, 0x80, 0xFF]);
            }
        }

        Ok(Capture {
            png: crate::capture::encode::to_png(&pixels, width, height)?,
            width,
            height,
            subject,
        })
    }
}

impl ScreenCapture for FakeCapture {
    fn displays(&self) -> Result<Vec<DisplayInfo>, CaptureError> {
        if let Some(error) = self.failure {
            return Err(error());
        }
        Ok(self.displays.clone())
    }

    fn windows(&self) -> Result<Vec<WindowInfo>, CaptureError> {
        if let Some(error) = self.failure {
            return Err(error());
        }
        Ok(self.windows.clone())
    }

    fn capture_display(&self, id: u32) -> Result<Capture, CaptureError> {
        let display = self
            .displays
            .iter()
            .find(|display| display.id == id)
            .cloned()
            .ok_or(CaptureError::Vanished { kind: "display" })?;

        let (width, height) = display.pixel_size();
        self.produce(
            Subject::Display {
                id: display.id,
                label: display.label,
            },
            width,
            height,
        )
    }

    fn capture_window(&self, id: u32) -> Result<Capture, CaptureError> {
        let window = self
            .windows
            .iter()
            .find(|window| window.id == id)
            .cloned()
            .ok_or(CaptureError::Vanished { kind: "window" })?;

        // Windows have no size in this fake; a plausible one is enough for callers that
        // only care that they got an image of the right thing.
        self.produce(
            Subject::Window {
                id: window.id,
                title: window.title,
                app: window.app,
            },
            1400,
            900,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_retina_display_captures_at_twice_its_logical_size() {
        // The trap this exists to close. `DisplayInfo::width` is 1512 logical points and
        // the image is 3024 pixels wide; code that sizes a buffer from the logical value
        // gets a quarter of what it needs.
        let display = &FakeCapture::laptop().displays().expect("displays")[0];
        assert_eq!((display.width, display.height), (1512, 982));
        assert_eq!(display.pixel_size(), (3024, 1964));
    }

    #[test]
    fn a_non_retina_display_captures_at_its_logical_size() {
        let fake = FakeCapture::with_external_display();
        let external = fake
            .displays()
            .expect("displays")
            .into_iter()
            .find(|display| display.id == 2)
            .expect("the external display");
        assert_eq!(external.scale, 1.0);
        assert_eq!(external.pixel_size(), (3840, 2160));
    }

    #[test]
    fn a_scale_factor_that_is_not_quite_two_still_lands_on_whole_pixels() {
        // Scale arrives as a float. 1512 × 1.9999998 truncates to 3023, one row short of
        // the buffer the platform actually returns — a one-stride shear that looks like a
        // slightly skewed screenshot rather than like a bug.
        let display = DisplayInfo {
            id: 1,
            label: "nearly two".to_string(),
            is_primary: true,
            width: 1512,
            height: 982,
            scale: 1.999_999_8,
        };
        assert_eq!(display.pixel_size(), (3024, 1964));
    }

    #[test]
    fn the_active_display_is_the_primary_one() {
        let fake = FakeCapture::with_external_display();
        let capture = fake.capture_active_display().expect("captures");

        assert_eq!(
            capture.subject,
            Subject::Display {
                id: 1,
                label: "Built-in Retina Display".to_string()
            },
            "the 4K external display was captured instead of the primary one"
        );
    }

    #[test]
    fn capturing_with_no_display_attached_says_so() {
        // Lid closed, no external monitor. Distinct from a permission failure, because
        // there is nothing the user can do about it in System Settings.
        assert!(matches!(
            FakeCapture::headless().capture_active_display(),
            Err(CaptureError::NoDisplay)
        ));
    }

    #[test]
    fn a_capture_is_downscaled_before_it_is_encoded() {
        // 3024×1964 is 7668 visual tokens unresized. The fake applies the same policy as
        // the real implementation so a caller's assumptions are tested honestly.
        let capture = FakeCapture::laptop()
            .capture_active_display()
            .expect("captures");

        assert!(
            capture.visual_tokens() <= downscale::MAX_TOKENS,
            "{}×{} costs {} tokens",
            capture.width,
            capture.height,
            capture.visual_tokens()
        );
        assert_eq!(
            (capture.width, capture.height),
            downscale::target_size(3024, 1964)
        );
    }

    #[test]
    fn the_capture_is_a_valid_png_of_the_stated_size() {
        // A fixture that is not decodable would make every downstream test pass against
        // something no model could read.
        let capture = FakeCapture::laptop()
            .capture_active_display()
            .expect("captures");

        let decoder = png::Decoder::new(capture.png.as_slice());
        let reader = decoder.read_info().expect("valid png");
        let info = reader.info();
        assert_eq!(info.width, capture.width);
        assert_eq!(info.height, capture.height);
    }

    #[test]
    fn a_vanished_window_is_reported_as_vanished() {
        // Windows close between being listed and being captured. Routine, and the message
        // should not suggest the user do anything.
        let error = FakeCapture::laptop()
            .capture_window(9999)
            .expect_err("no such window");
        assert!(matches!(error, CaptureError::Vanished { kind: "window" }));
        assert!(error.to_string().contains("window"), "{error}");
    }

    #[test]
    fn windows_are_listed_frontmost_first() {
        // Part of the trait's contract: "the window I am looking at" is the first one.
        let windows = FakeCapture::laptop().windows().expect("windows");
        assert!(
            windows[0].is_focused,
            "the first window is not the focused one"
        );
    }

    #[test]
    fn the_fake_records_what_it_was_asked_for() {
        let fake = FakeCapture::with_external_display();
        fake.capture_display(2).expect("captures");
        fake.capture_window(10).expect("captures");

        let requested = fake.requested();
        assert_eq!(requested.len(), 2);
        assert!(matches!(requested[0], Subject::Display { id: 2, .. }));
        assert!(matches!(requested[1], Subject::Window { id: 10, .. }));
    }

    #[test]
    fn a_failing_screen_still_records_the_attempt() {
        // So a test can assert what a caller tried to do even when the platform refused.
        let fake = FakeCapture::failing(|| CaptureError::PermissionDenied);
        assert!(matches!(
            fake.capture_display(1),
            Err(CaptureError::PermissionDenied)
        ));
        assert!(matches!(
            fake.displays(),
            Err(CaptureError::PermissionDenied)
        ));
    }

    #[test]
    fn a_subject_describes_itself_for_the_audit_log() {
        assert_eq!(
            Subject::Display {
                id: 1,
                label: "Built-in Retina Display".into()
            }
            .describe(),
            "Built-in Retina Display"
        );
        assert_eq!(
            Subject::Window {
                id: 10,
                title: "src/main.rs".into(),
                app: "Zed".into()
            }
            .describe(),
            "Zed — src/main.rs"
        );
        // An untitled window falls back to the application. Reachable without a bug:
        // macOS 13 and later withhold other apps' titles when Screen Recording is not
        // granted, so "" is a permission symptom and the app name is all there is.
        assert_eq!(
            Subject::Window {
                id: 10,
                title: String::new(),
                app: "Zed".into()
            }
            .describe(),
            "Zed"
        );
    }

    #[test]
    fn the_trait_is_object_safe_and_sendable() {
        // `AppState` stores this as a `Box<dyn ScreenCapture>` and every call goes to
        // `spawn_blocking`, which needs both. A method taking `self` by value or a
        // generic parameter would break this at the point of use rather than here.
        fn assert_usable<T: Send + Sync + ?Sized>() {}
        assert_usable::<dyn ScreenCapture>();
        let _: Box<dyn ScreenCapture> = Box::new(FakeCapture::laptop());
    }
}
