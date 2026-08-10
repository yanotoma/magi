//! Turning raw pixels into the PNG a model is sent.
//!
//! Uses the `png` crate rather than `image`, for the same reason `llm/probe_image.rs`
//! does: one encoder in the binary, and a subtly malformed PNG is the most expensive bug
//! this area can produce — accepted by the HTTP layer, misread by every model, and
//! reported to the user as "this model cannot see".
//!
//! PNG rather than JPEG despite the size difference. Screenshots are mostly flat colour
//! and sharp text, which is the case PNG compresses well and JPEG damages exactly where it
//! matters: the ringing artefacts land on glyph edges, and the whole point is for a model
//! to read the text.

/// Something went wrong reading the screen.
///
/// Kept in this module rather than beside the trait because encoding is the first thing
/// that can fail without a display being involved, and the variants are shared.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// Screen Recording permission has not been granted.
    ///
    /// Its own variant because it is the only failure with a fix the user can act on, and
    /// the action is in System Settings rather than anywhere in Magi.
    #[error("Magi does not have permission to read the screen")]
    PermissionDenied,

    /// The display or window asked for is gone.
    ///
    /// Normal, not exceptional: windows close and displays get unplugged between being
    /// enumerated and being captured.
    #[error("that {kind} is no longer there")]
    Vanished { kind: &'static str },

    /// No display was found at all.
    #[error("no display was found")]
    NoDisplay,

    /// The pixel buffer did not match the dimensions given.
    #[error("expected {expected} bytes of pixel data for {width}×{height}, got {actual}")]
    MalformedPixels {
        width: u32,
        height: u32,
        expected: usize,
        actual: usize,
    },

    /// The PNG encoder failed.
    #[error("could not encode the screenshot: {0}")]
    Encoding(String),

    /// Anything the platform reported that does not map to the above.
    #[error("could not read the screen: {0}")]
    Platform(String),
}

/// Encodes 8-bit RGBA pixels as PNG.
///
/// `pixels` must be exactly `width * height * 4` bytes, row-major, with no padding —
/// stride handling belongs to whoever produced the buffer, because only they know what the
/// platform padded it to.
pub fn to_png(pixels: &[u8], width: u32, height: u32) -> Result<Vec<u8>, CaptureError> {
    // `as usize` on a u32 is lossless on every target Magi builds for, but the product
    // is not: a 65536×65536 buffer overflows u32 and would wrap to a small expected
    // length that a short buffer then matches. usize arithmetic is what makes the check
    // below mean what it says.
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(CaptureError::MalformedPixels {
            width,
            height,
            expected: usize::MAX,
            actual: pixels.len(),
        })?;

    if pixels.len() != expected {
        return Err(CaptureError::MalformedPixels {
            width,
            height,
            expected,
            actual: pixels.len(),
        });
    }

    if width == 0 || height == 0 {
        // The `png` crate writes a header for a zero-sized image without complaint, and
        // every model rejects it with a message about the request rather than the image.
        return Err(CaptureError::MalformedPixels {
            width,
            height,
            expected,
            actual: pixels.len(),
        });
    }

    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);

        let mut writer = encoder
            .write_header()
            .map_err(|error| CaptureError::Encoding(error.to_string()))?;
        writer
            .write_image_data(pixels)
            .map_err(|error| CaptureError::Encoding(error.to_string()))?;
        writer
            .finish()
            .map_err(|error| CaptureError::Encoding(error.to_string()))?;
    }

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A solid-colour buffer of the given size.
    fn solid(width: u32, height: u32) -> Vec<u8> {
        [0x40, 0x80, 0xC0, 0xFF].repeat((width * height) as usize)
    }

    #[test]
    fn a_png_round_trips_to_the_same_pixels() {
        // Encoding is only correct if something else can read it back. Asserting the
        // bytes are non-empty would pass for a truncated file, which is the failure that
        // reaches a model as "this model cannot see".
        let pixels = solid(8, 4);
        let encoded = to_png(&pixels, 8, 4).expect("encodes");

        let decoder = png::Decoder::new(encoded.as_slice());
        let mut reader = decoder.read_info().expect("valid png");
        let mut decoded = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut decoded).expect("one frame");

        assert_eq!(info.width, 8);
        assert_eq!(info.height, 4);
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(&decoded[..info.buffer_size()], &pixels[..]);
    }

    #[test]
    fn the_png_signature_is_present() {
        // The eight bytes every decoder checks first. Cheap, and it catches an encoder
        // that was swapped for one writing a different container.
        let encoded = to_png(&solid(2, 2), 2, 2).expect("encodes");
        assert_eq!(&encoded[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn a_short_buffer_is_rejected_rather_than_encoded() {
        // The bug this prevents: a platform that pads rows to a 16-byte boundary hands
        // back more bytes than width*height*4, and slicing to the expected length without
        // removing the padding shears every row progressively — an image that looks
        // plausibly like a skewed screenshot and reads as gibberish.
        let error = to_png(&solid(8, 4)[..100], 8, 4).expect_err("must reject");
        assert!(matches!(
            error,
            CaptureError::MalformedPixels {
                expected: 128,
                actual: 100,
                ..
            }
        ));
    }

    #[test]
    fn an_overlong_buffer_is_rejected_too() {
        let mut pixels = solid(4, 4);
        pixels.extend_from_slice(&[0; 16]);
        assert!(matches!(
            to_png(&pixels, 4, 4),
            Err(CaptureError::MalformedPixels { .. })
        ));
    }

    #[test]
    fn a_zero_sized_image_is_rejected() {
        // Reachable: a display being reconfigured reports no size. A 0×0 PNG encodes
        // without error and is rejected by the model with a message about the request,
        // which sends the user looking in the wrong place.
        assert!(matches!(
            to_png(&[], 0, 0),
            Err(CaptureError::MalformedPixels { .. })
        ));
        assert!(matches!(
            to_png(&[], 100, 0),
            Err(CaptureError::MalformedPixels { .. })
        ));
    }

    #[test]
    fn dimensions_that_overflow_are_rejected_not_wrapped() {
        // `width * height * 4` in u32 wraps for large dimensions, and a wrapped expected
        // length can match a short buffer — so the length check would pass and the
        // encoder would read past the buffer. usize arithmetic plus checked_mul is what
        // makes this an error instead.
        assert!(matches!(
            to_png(&[0; 16], u32::MAX, u32::MAX),
            Err(CaptureError::MalformedPixels { .. })
        ));
    }

    #[test]
    fn the_permission_error_says_what_to_do_about_it() {
        // The one failure with a user-actionable fix. Its message is shown verbatim, so
        // it has to name permission rather than an API.
        let message = CaptureError::PermissionDenied.to_string();
        assert!(message.contains("permission"), "{message}");
    }
}
