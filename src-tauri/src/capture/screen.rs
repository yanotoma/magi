//! Reading the screen on macOS, through ScreenCaptureKit.
//!
//! macOS only, and gated in `Cargo.toml` as well as here, so a Linux CI runner compiles the
//! trait and the fake without needing a display server or an Apple framework.
//!
//! ## Why ScreenCaptureKit
//!
//! The alternative, `CGWindowListCreateImage`, is what every current Rust capture crate
//! uses and it still runs — measured on macOS 15.5, where it is marked `obsoleted=15.0`.
//! Two reasons not to build on it anyway: Apple's own message is "Please use
//! ScreenCaptureKit instead", and without permission it returns a **picture of an empty
//! desktop rather than an error**, which reaches a model as a screen it never saw. The
//! second is closed by checking permission first, so the honest reason is the first one.
//!
//! ## The size is asked for, not corrected afterwards
//!
//! The trap this module would otherwise walk into is that `SCDisplay::width` is in *points*
//! while a capture is in *pixels* — on a Retina display those differ by a factor of two,
//! and code that sizes a buffer from the former gets a quarter of what it needs.
//!
//! It does not arise here, because [`SCStreamConfiguration`] takes the output size in pixels
//! and ScreenCaptureKit renders to it. So the target comes from
//! [`downscale::target_size`] and the physical resolution never enters the arithmetic:
//! asking for 1372×891 gets 1372×891, whether the display is Retina or not. That also means
//! no resize step and no double-resize blur — the compositor scales once, on the GPU.
//!
//! Deriving the target from the display's *points* is not a shortcut either. What the token
//! budget bounds is the pixel count of the image sent, and the aspect ratio is the same in
//! either unit, so points are simply the cheaper way to ask the same question.

use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::AllocAnyThread;
use objc2_foundation::{NSArray, NSError};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCScreenshotManager, SCShareableContent, SCStreamConfiguration,
    SCStreamErrorCode, SCWindow,
};

use crate::capture::downscale;
use crate::capture::encode::{to_png, CaptureError};
use crate::capture::source::{Capture, DisplayInfo, ScreenCapture, Subject, WindowInfo};

/// How long to wait for ScreenCaptureKit to answer.
///
/// A bound rather than an unbounded wait, because the failure this guards is not an error
/// but a silence: if a completion handler never fires, an unbounded `recv` holds a
/// `spawn_blocking` worker for the lifetime of the process. Generous — a first call has to
/// bring up the capture machinery — and still far below anyone's patience.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

/// `kCVPixelFormatType_32BGRA`.
///
/// Set explicitly rather than left to the default, so the byte order this module unpacks is
/// the byte order it asked for. **BGRA, not RGBA** — the channels are swapped on the way
/// out. Getting that wrong does not fail: it produces an image with red and blue exchanged,
/// which a model will describe fluently and wrongly.
const PIXEL_FORMAT_BGRA: u32 = u32::from_be_bytes(*b"BGRA");

/// Reads the screen with ScreenCaptureKit.
///
/// Holds no state. Everything is fetched per call, because displays get unplugged and
/// windows close, and a cached list is a list of things that may no longer exist.
pub struct ScreenCaptureKit;

impl ScreenCaptureKit {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ScreenCaptureKit {
    fn default() -> Self {
        Self::new()
    }
}

/// Everything ScreenCaptureKit is willing to show us.
///
/// Bridges one async call to a blocking one. Safe here specifically because the completion
/// handler runs on a queue of the framework's choosing rather than on the caller's thread —
/// blocking a `spawn_blocking` worker cannot starve the handler that would release it. It
/// would deadlock on the main thread, which is why every caller goes through
/// `spawn_blocking`.
fn shareable_content() -> Result<Retained<SCShareableContent>, CaptureError> {
    // Refuse before asking. Without permission, ScreenCaptureKit reports
    // `SCStreamErrorCode::UserDeclined` and does the right thing — but on a first run it
    // also has nothing to report until the user has been sent to System Settings, and this
    // way the error names the fix rather than the symptom.
    if !crate::permissions::screen_recording().is_usable() {
        return Err(CaptureError::PermissionDenied);
    }

    let (sender, receiver) = mpsc::channel();

    // `*mut` in, owned out. The handler is called on a framework queue, so the values have
    // to be retained before they cross the channel rather than borrowed across it.
    let handler = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            let outcome = if content.is_null() {
                Err(describe(error))
            } else {
                // Safe: non-null, and ScreenCaptureKit hands us a +0 reference that
                // `retain` makes ours for as long as we hold it.
                Ok(unsafe { Retained::retain(content) })
            };
            let _ = sender.send(outcome);
        },
    );

    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&handler) };

    match receiver.recv_timeout(REPLY_TIMEOUT) {
        Ok(Ok(Some(content))) => Ok(content),
        Ok(Ok(None)) => Err(CaptureError::Platform(
            "ScreenCaptureKit returned no content".to_string(),
        )),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(CaptureError::Platform(
            "ScreenCaptureKit did not answer".to_string(),
        )),
    }
}

/// Turns an `NSError` from ScreenCaptureKit into something with a fix in it.
///
/// The one code worth naming is `UserDeclined`. Everything else is a platform detail the
/// user cannot act on, so it is passed through as text rather than mapped to a variant that
/// would imply Magi knows what to do about it.
fn describe(error: *mut NSError) -> CaptureError {
    if error.is_null() {
        return CaptureError::Platform("ScreenCaptureKit failed without saying why".to_string());
    }

    // Safe: checked non-null just above, and only read.
    let error = unsafe { &*error };

    if error.code() == SCStreamErrorCode::UserDeclined.0 {
        return CaptureError::PermissionDenied;
    }

    CaptureError::Platform(error.localizedDescription().to_string())
}

/// Captures with a filter already built, at `width`×`height` pixels.
fn capture(
    filter: &SCContentFilter,
    width: u32,
    height: u32,
    subject: Subject,
) -> Result<Capture, CaptureError> {
    if width == 0 || height == 0 {
        return Err(CaptureError::Vanished {
            kind: "display or window",
        });
    }

    let configuration = unsafe {
        let configuration = SCStreamConfiguration::new();
        // Pixels, and the reason the points-versus-pixels trap does not arise here.
        configuration.setWidth(width as usize);
        configuration.setHeight(height as usize);
        configuration.setPixelFormat(PIXEL_FORMAT_BGRA);
        // The aspect ratio asked for is the source's own, so nothing is letterboxed; this
        // only guards against a rounding difference producing a stretched image.
        configuration.setScalesToFit(true);
        // No cursor. It is not part of what the user is asking about, and a model that sees
        // one tends to describe where it is as though that mattered.
        configuration.setShowsCursor(false);
        configuration
    };

    let (sender, receiver) = mpsc::channel();
    let handler = RcBlock::new(
        move |image: *mut objc2_core_graphics::CGImage, error: *mut NSError| {
            let outcome = if image.is_null() {
                Err(describe(error))
            } else {
                // Safe: non-null. `CGImage` is a CoreFoundation type, so this is a retain
                // rather than an Objective-C ownership transfer.
                Ok(unsafe {
                    objc2_core_foundation::CFRetained::retain(std::ptr::NonNull::new_unchecked(
                        image,
                    ))
                })
            };
            let _ = sender.send(outcome);
        },
    );

    unsafe {
        SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
            filter,
            &configuration,
            Some(&handler),
        )
    };

    let image = match receiver.recv_timeout(REPLY_TIMEOUT) {
        Ok(Ok(image)) => image,
        Ok(Err(error)) => return Err(error),
        Err(_) => {
            return Err(CaptureError::Platform(
                "the screenshot never arrived".to_string(),
            ))
        }
    };

    let pixels = to_rgba(&image)?;
    let width = objc2_core_graphics::CGImage::width(Some(&image)) as u32;
    let height = objc2_core_graphics::CGImage::height(Some(&image)) as u32;

    Ok(Capture {
        png: to_png(&pixels, width, height)?,
        width,
        height,
        subject,
    })
}

/// Copies a `CGImage` into tightly packed RGBA.
///
/// Two conversions, both of which are silent when wrong:
///
/// **Row stride.** `bytes_per_row` is generally wider than `width * 4` — the compositor
/// aligns rows — so copying the buffer whole shifts every row by the padding of the ones
/// before it. The result looks like a sheared photograph of a screen, not like a bug.
///
/// **Channel order.** ScreenCaptureKit returns BGRA for standard dynamic range, as its own
/// documentation states. Skipping the swap produces an image with red and blue exchanged,
/// which a model will describe fluently and wrongly.
fn to_rgba(image: &objc2_core_graphics::CGImage) -> Result<Vec<u8>, CaptureError> {
    use objc2_core_graphics::{CGDataProvider, CGImage};

    let width = CGImage::width(Some(image));
    let height = CGImage::height(Some(image));
    let stride = CGImage::bytes_per_row(Some(image));
    let bits_per_pixel = CGImage::bits_per_pixel(Some(image));

    if bits_per_pixel != 32 {
        // Reachable if the dynamic range is ever changed: HDR comes back as RGhA at a
        // different depth, and unpacking it as if it were 32-bit BGRA would produce noise.
        return Err(CaptureError::Platform(format!(
            "expected 32 bits per pixel, got {bits_per_pixel}"
        )));
    }

    let provider = CGImage::data_provider(Some(image))
        .ok_or_else(|| CaptureError::Platform("the screenshot had no pixel data".to_string()))?;
    let data = CGDataProvider::data(Some(&provider))
        .ok_or_else(|| CaptureError::Platform("the pixel data could not be copied".to_string()))?;

    // Safe: the pointer is valid while `data` is alive, and `data` outlives this slice.
    let bytes = unsafe { std::slice::from_raw_parts(data.byte_ptr(), data.length() as usize) };

    let needed = stride
        .checked_mul(height)
        .ok_or_else(|| CaptureError::Platform("implausible image dimensions".to_string()))?;
    if bytes.len() < needed {
        return Err(CaptureError::MalformedPixels {
            width: width as u32,
            height: height as u32,
            expected: needed,
            actual: bytes.len(),
        });
    }

    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in 0..height {
        let start = row * stride;
        for pixel in bytes[start..start + width * 4].chunks_exact(4) {
            // BGRA in, RGBA out.
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }

    Ok(rgba)
}

/// The scale factor macOS reports for a display.
///
/// Reported rather than used: nothing here sizes a capture from it, because the capture size
/// is asked for outright. It is in [`DisplayInfo`] so Settings can say what kind of display
/// it is, and so a future caller cannot be misled into thinking points are pixels.
///
/// Falls back to `1.0`, which is the honest answer when macOS declines to say — a guess of
/// `2.0` would be wrong on every external monitor.
fn scale_of(display_id: u32) -> f32 {
    use objc2_core_graphics::{CGDisplayBounds, CGDisplayCopyDisplayMode, CGDisplayMode};

    // Already a `CFRetained` — the safe wrapper takes ownership of the +1 reference for us.
    let Some(mode) = CGDisplayCopyDisplayMode(display_id) else {
        return 1.0;
    };

    let pixels = CGDisplayMode::pixel_width(Some(&mode)) as f32;
    let points = CGDisplayBounds(display_id).size.width as f32;

    if points > 0.0 && pixels > 0.0 {
        pixels / points
    } else {
        1.0
    }
}

/// Reads an `SCDisplay` into the neutral shape the trait speaks in.
fn describe_display(display: &SCDisplay, primary: u32) -> DisplayInfo {
    // Safe: plain accessors on a live object, none of which take arguments that could be
    // wrong.
    let (id, width, height) = unsafe {
        (
            display.displayID(),
            display.width() as u32,
            display.height() as u32,
        )
    };

    DisplayInfo {
        id,
        // ScreenCaptureKit exposes no display name, and `NSScreen.localizedName` is
        // main-thread only — calling it from here would be unsound. A number the user can
        // match to the order in System Settings beats a name obtained unsoundly.
        label: if id == primary {
            "Main display".to_string()
        } else {
            format!("Display {id}")
        },
        is_primary: id == primary,
        width,
        height,
        scale: scale_of(id),
    }
}

/// The display the menu bar is on.
fn main_display_id() -> u32 {
    objc2_core_graphics::CGMainDisplayID()
}

impl ScreenCapture for ScreenCaptureKit {
    fn displays(&self) -> Result<Vec<DisplayInfo>, CaptureError> {
        let content = shareable_content()?;
        let primary = main_display_id();

        // Safe: reads a retained array.
        let displays = unsafe { content.displays() };
        Ok(displays
            .iter()
            .map(|display| describe_display(&display, primary))
            .collect())
    }

    fn windows(&self) -> Result<Vec<WindowInfo>, CaptureError> {
        let content = shareable_content()?;

        // Safe: reads a retained array and plain accessors on its elements.
        let windows = unsafe { content.windows() };
        Ok(windows
            .iter()
            .filter(|window| unsafe { window.isOnScreen() })
            .map(|window| unsafe {
                WindowInfo {
                    id: window.windowID(),
                    // Empty is not necessarily a bug: macOS withholds other applications'
                    // titles when Screen Recording is absent. `WindowInfo::title` says so.
                    title: window
                        .title()
                        .map(|title| title.to_string())
                        .unwrap_or_default(),
                    app: window
                        .owningApplication()
                        .map(|app| app.applicationName().to_string())
                        .unwrap_or_default(),
                    // ScreenCaptureKit does not expose focus. The list arrives in front-to-back
                    // order, so the first on-screen window is the frontmost one — which is what
                    // the trait's contract promises and all any caller needs.
                    is_focused: false,
                }
            })
            .enumerate()
            .map(|(index, window)| WindowInfo {
                is_focused: index == 0,
                ..window
            })
            .collect())
    }

    fn capture_display(&self, id: u32) -> Result<Capture, CaptureError> {
        let content = shareable_content()?;
        let primary = main_display_id();

        let displays = unsafe { content.displays() };
        let display = displays
            .iter()
            .find(|display| unsafe { display.displayID() } == id)
            .ok_or(CaptureError::Vanished { kind: "display" })?;

        let info = describe_display(&display, primary);
        let (width, height) = downscale::target_size(info.width, info.height);

        // Magi's own panel is excluded from nothing here: it is hidden while a capture runs,
        // and an empty exclusion list is what asks for "the whole display".
        let nothing_excluded = NSArray::<SCWindow>::new();
        let filter = unsafe {
            SCContentFilter::initWithDisplay_excludingWindows(
                SCContentFilter::alloc(),
                &display,
                &nothing_excluded,
            )
        };

        capture(
            &filter,
            width,
            height,
            Subject::Display {
                id: info.id,
                label: info.label,
            },
        )
    }

    fn capture_window(&self, id: u32) -> Result<Capture, CaptureError> {
        let content = shareable_content()?;

        let windows = unsafe { content.windows() };
        let window = windows
            .iter()
            .find(|window| unsafe { window.windowID() } == id)
            .ok_or(CaptureError::Vanished { kind: "window" })?;

        // Points, and the aspect ratio is what the budget is computed from — see the module
        // docs for why the physical size never enters this.
        let frame = unsafe { window.frame() };
        let (width, height) = downscale::target_size(
            frame.size.width.max(0.0) as u32,
            frame.size.height.max(0.0) as u32,
        );

        let subject = unsafe {
            Subject::Window {
                id,
                title: window
                    .title()
                    .map(|title| title.to_string())
                    .unwrap_or_default(),
                app: window
                    .owningApplication()
                    .map(|app| app.applicationName().to_string())
                    .unwrap_or_default(),
            }
        };

        let filter = unsafe {
            SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &window)
        };

        capture(&filter, width, height, subject)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Nothing here captures anything. A test that did would need a display and a granted
    // permission, which the project's rules forbid and CI cannot provide. What is testable
    // is the arithmetic and the conversions, which is where the silent bugs live.

    #[test]
    fn the_pixel_format_constant_is_the_four_character_code() {
        // `kCVPixelFormatType_32BGRA` is the OSType for 'BGRA'. Written as a byte string so
        // it reads as the code rather than as a magic number, which means asserting that
        // the arithmetic produces what Apple documents.
        assert_eq!(PIXEL_FORMAT_BGRA, 0x4247_5241);
    }

    #[test]
    fn a_capture_is_asked_for_at_a_size_within_budget() {
        // The property that makes the points-versus-pixels trap irrelevant: whatever the
        // display reports, the size asked for is the one the token budget allows.
        for (width, height) in [(1512, 982), (3840, 2160), (3440, 1440), (1280, 800)] {
            let (asked_width, asked_height) = downscale::target_size(width, height);
            assert!(
                downscale::visual_tokens(asked_width, asked_height) <= downscale::MAX_TOKENS,
                "{width}×{height} → {asked_width}×{asked_height}"
            );
        }
    }

    #[test]
    fn the_timeout_is_long_enough_to_be_a_backstop_not_a_limit() {
        // A bound on silence, not on slowness. If this were tight it would turn a slow
        // first capture into an error the user cannot act on.
        assert!(REPLY_TIMEOUT >= Duration::from_secs(5));
    }

    #[test]
    fn a_null_error_still_produces_something_sayable() {
        // ScreenCaptureKit can fail with neither an image nor an error. Passing that through
        // as an empty message would show the user a blank alert.
        let error = describe(std::ptr::null_mut());
        assert!(matches!(error, CaptureError::Platform(_)));
        assert!(!error.to_string().is_empty());
    }
}
