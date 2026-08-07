//! The image the vision probe sends.
//!
//! A white digit on black, drawn as seven-segment bars. The point is a picture
//! whose contents Magi knows for certain, so that asking the model what it shows
//! has a single right answer — which is the only way to tell genuine vision from
//! an endpoint that accepts an image payload and quietly ignores it.
//!
//! Seven segments rather than a font, for three reasons: no font dependency and
//! no font file to ship, byte-identical output on every machine so a failure is
//! never "it rendered differently here", and a shape that stays legible when a
//! provider downscales it.
//!
//! Nothing here is a drawing library. It fills rectangles into a byte buffer,
//! because rectangles are all a seven-segment digit is.

use crate::llm::provider::Image;

/// Side of the generated square, in pixels.
///
/// Large enough that a vision model has something to work with after whatever
/// resizing happens on the way in, small enough that the base64 payload stays a
/// few kilobytes. A probe that costs real tokens would discourage re-testing, and
/// re-testing is the whole point of the *Re-test* button.
const SIZE: usize = 192;

/// Which segments each digit lights, in the order below.
///
/// ```text
///  aaa
/// f   b
/// f   b
///  ggg
/// e   c
/// e   c
///  ddd
/// ```
const SEGMENTS: [[bool; 7]; 10] = [
    //  a      b      c      d      e      f      g
    [true, true, true, true, true, true, false],     // 0
    [false, true, true, false, false, false, false], // 1
    [true, true, false, true, true, false, true],    // 2
    [true, true, true, true, false, false, true],    // 3
    [false, true, true, false, false, true, true],   // 4
    [true, false, true, true, false, true, true],    // 5
    [true, false, true, true, true, true, true],     // 6
    [true, true, true, false, false, false, false],  // 7
    [true, true, true, true, true, true, true],      // 8
    [true, true, true, true, false, true, true],     // 9
];

/// The digit the probe asks about.
///
/// Two constraints pull in opposite directions, and the first attempt satisfied only
/// one of them.
///
/// It must not be an easy blind guess: 1 is what a model names for any narrow mark,
/// and 0 and 8 for any closed one, so a guess that happened to be right would record
/// vision the model does not have.
///
/// It must also *look like a digit*. This was 7, chosen for the first constraint —
/// and 7 is the worst digit in this typeface for the second, because its
/// seven-segment form lights only two segments and comes out as a corner. Rendered
/// and looked at, it read as a bracket rather than a number, which invites "a white
/// shape in the corner" as an answer and a false negative as a result.
///
/// 2 lights five segments and gives the unmistakable zigzag of a digital clock. Not
/// a common blind guess, and unambiguously a numeral.
pub const PROBE_DIGIT: u8 = 2;

struct Canvas {
    pixels: Vec<u8>,
}

impl Canvas {
    /// Black, so a model that receives nothing and hallucinates a description has
    /// no bright field to invent shapes in.
    fn black() -> Self {
        Self {
            pixels: vec![0u8; SIZE * SIZE],
        }
    }

    fn fill(&mut self, x0: usize, y0: usize, width: usize, height: usize) {
        for y in y0..(y0 + height).min(SIZE) {
            for x in x0..(x0 + width).min(SIZE) {
                self.pixels[y * SIZE + x] = 0xFF;
            }
        }
    }
}

/// Draws one seven-segment digit, centred.
///
/// **Segments overlap at the corners, and must.** The first version inset the
/// horizontal bars by one thickness at each end *and* started the vertical bars one
/// thickness below the top — so neither filled the corner square between them, and
/// a `7` came out as a detached horizontal stroke above a detached vertical one. It
/// passed every test in this file, because they checked that something was drawn and
/// that the ten digits differed from each other, and neither of those is legibility.
///
/// So horizontals span the full width and verticals run corner to corner. The
/// overlap costs nothing — a filled pixel filled twice is the same pixel — and it is
/// what makes the glyph one connected shape, which is the property
/// `every_digit_is_a_single_connected_shape` now checks.
fn draw_digit(canvas: &mut Canvas, digit: u8) {
    let lit = SEGMENTS[(digit % 10) as usize];

    // Thick and large relative to the canvas. A thin glyph survives a provider's
    // downscaling badly, and downscaling is out of our hands once the image is sent.
    let thickness = SIZE / 8;
    let width = SIZE * 3 / 5;
    let height = SIZE * 3 / 4;
    let left = (SIZE - width) / 2;
    let top = (SIZE - height) / 2;
    let right = left + width - thickness;
    let middle = top + (height - thickness) / 2;
    let bottom = top + height - thickness;

    // Verticals reach from one horizontal's row through the next, so the corners
    // are covered whichever pair of segments happens to be lit.
    let upper = middle - top + thickness;
    let lower = bottom - middle + thickness;

    if lit[0] {
        canvas.fill(left, top, width, thickness); // a — top
    }
    if lit[1] {
        canvas.fill(right, top, thickness, upper); // b — upper right
    }
    if lit[2] {
        canvas.fill(right, middle, thickness, lower); // c — lower right
    }
    if lit[3] {
        canvas.fill(left, bottom, width, thickness); // d — bottom
    }
    if lit[4] {
        canvas.fill(left, middle, thickness, lower); // e — lower left
    }
    if lit[5] {
        canvas.fill(left, top, thickness, upper); // f — upper left
    }
    if lit[6] {
        canvas.fill(left, middle, width, thickness); // g — middle
    }
}

/// Encodes a greyscale buffer as a PNG.
fn encode(pixels: &[u8]) -> Result<Vec<u8>, png::EncodingError> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, SIZE as u32, SIZE as u32);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(pixels)?;
    }
    Ok(bytes)
}

/// The vision probe's image: [`PROBE_DIGIT`] in white on black.
///
/// Returns `None` if encoding fails, which should not happen for a fixed-size
/// greyscale buffer. The caller treats that as "vision could not be tested" rather
/// than "the model cannot see" — a Magi bug must not be reported as a model
/// limitation.
pub fn probe_image() -> Option<Image> {
    let mut canvas = Canvas::black();
    draw_digit(&mut canvas, PROBE_DIGIT);
    encode(&canvas.pixels).ok().map(Image::png)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produces_a_decodable_png() {
        // The reason `png` is a dependency rather than hand-rolled: this test
        // decodes what was written, so "it encoded" and "it is readable" are
        // separate claims. A malformed image would make every model look blind.
        let image = probe_image().expect("encoding a fixed-size buffer must succeed");
        assert_eq!(image.media_type, "image/png");

        let decoder = png::Decoder::new(image.bytes.as_slice());
        let mut reader = decoder.read_info().expect("the PNG must be readable back");
        let info = reader.info();
        assert_eq!(info.width, SIZE as u32);
        assert_eq!(info.height, SIZE as u32);

        let mut buffer = vec![0; reader.output_buffer_size()];
        let frame = reader.next_frame(&mut buffer).expect("decodable frame");
        assert_eq!(frame.width, SIZE as u32);
    }

    #[test]
    fn the_digit_is_actually_drawn() {
        // Guards the case that would be worst: a valid, entirely black PNG. It
        // would encode, decode, send, and produce "I see a black square" from every
        // model — recorded as no vision, for every provider at once.
        let mut canvas = Canvas::black();
        draw_digit(&mut canvas, PROBE_DIGIT);

        let lit = canvas.pixels.iter().filter(|&&p| p == 0xFF).count();
        assert!(lit > 0, "nothing was drawn");

        let total = SIZE * SIZE;
        assert!(
            lit > total / 100 && lit < total / 2,
            "the digit covers {lit} of {total} pixels, which is not a digit"
        );
    }

    #[test]
    fn each_digit_draws_a_distinct_shape() {
        // If two digits rendered identically, the probe could accept a wrong answer
        // as correct.
        //
        // The comparison is the whole pixel buffer, and it has to be. A per-row
        // pixel count was the first attempt and it reported 2 and 3 as identical —
        // wrongly. In seven segments those two are mirror images: 2 lights the
        // upper-right and lower-left bars, 3 the upper-right and lower-right, so
        // each row holds the same number of lit pixels in both. The glyphs differ
        // only in *where* those pixels are, which is exactly what a count discards.
        let mut seen = std::collections::HashSet::new();
        for digit in 0..10u8 {
            let mut canvas = Canvas::black();
            draw_digit(&mut canvas, digit);
            assert!(
                seen.insert(canvas.pixels),
                "digit {digit} renders identically to an earlier one"
            );
        }
    }

    /// Counts the connected regions of lit pixels, four-connected.
    fn lit_regions(pixels: &[u8]) -> usize {
        let mut seen = vec![false; pixels.len()];
        let mut regions = 0;

        for start in 0..pixels.len() {
            if pixels[start] != 0xFF || seen[start] {
                continue;
            }
            regions += 1;

            let mut stack = vec![start];
            seen[start] = true;
            while let Some(index) = stack.pop() {
                let (x, y) = (index % SIZE, index / SIZE);
                let mut push = |nx: usize, ny: usize, stack: &mut Vec<usize>| {
                    let next = ny * SIZE + nx;
                    if pixels[next] == 0xFF && !seen[next] {
                        seen[next] = true;
                        stack.push(next);
                    }
                };
                if x > 0 {
                    push(x - 1, y, &mut stack);
                }
                if x + 1 < SIZE {
                    push(x + 1, y, &mut stack);
                }
                if y > 0 {
                    push(x, y - 1, &mut stack);
                }
                if y + 1 < SIZE {
                    push(x, y + 1, &mut stack);
                }
            }
        }

        regions
    }

    /// The test that would have caught the broken corners.
    ///
    /// On a real seven-segment display the lit segments of every digit touch, so the
    /// glyph is one piece. The first version of `draw_digit` inset the horizontal
    /// bars and dropped the vertical bars below them, leaving the corner square
    /// unfilled — a `7` rendered as two separate strokes, which reads as a corner or
    /// a bracket rather than a digit. Every other test in this file passed.
    ///
    /// Legibility is not directly testable. Connectivity is, and it is the property
    /// that was actually broken.
    #[test]
    fn every_digit_is_a_single_connected_shape() {
        for digit in 0..10u8 {
            let mut canvas = Canvas::black();
            draw_digit(&mut canvas, digit);
            assert_eq!(
                lit_regions(&canvas.pixels),
                1,
                "digit {digit} renders as separate pieces rather than one glyph"
            );
        }
    }

    #[test]
    fn the_probe_digit_fills_a_useful_share_of_the_frame() {
        // A legible glyph has to be big enough to survive whatever resizing the
        // provider does. The earlier geometry drew a small thin mark in a large black
        // field, which is the shape most likely to be read as "a white shape".
        let mut canvas = Canvas::black();
        draw_digit(&mut canvas, PROBE_DIGIT);

        let lit = canvas.pixels.iter().filter(|&&p| p == 0xFF).count();
        let share = lit as f64 / (SIZE * SIZE) as f64;
        assert!(
            share > 0.06,
            "the digit covers only {:.1}% of the image",
            share * 100.0
        );
    }

    #[test]
    fn mirrored_digits_are_told_apart() {
        // 2 and 3 are the pair that caught the flawed comparison above, and 6 and 9
        // are the other mirrored pair. Named explicitly so a future change to the
        // segment geometry cannot quietly collapse either one.
        let render = |digit: u8| {
            let mut canvas = Canvas::black();
            draw_digit(&mut canvas, digit);
            canvas.pixels
        };

        assert_ne!(render(2), render(3), "2 and 3 are mirrored, not identical");
        assert_ne!(render(6), render(9), "6 and 9 are mirrored, not identical");
    }

    #[test]
    fn the_segment_table_matches_the_digits_it_claims() {
        // Spot-checks against the physical facts of a seven-segment display, which
        // is the kind of table that is easy to get subtly wrong and impossible to
        // notice by reading.
        assert_eq!(SEGMENTS[8], [true; 7], "8 lights every segment");
        assert_eq!(
            SEGMENTS[1],
            [false, true, true, false, false, false, false],
            "1 lights only the two right-hand bars"
        );
        assert!(!SEGMENTS[7][6], "7 has no middle bar");
        assert!(!SEGMENTS[0][6], "0 has no middle bar");
        assert!(SEGMENTS[6][4], "6 lights its lower left");
        assert!(!SEGMENTS[5][1], "5 has no upper right");
    }

    #[test]
    fn the_probe_digit_is_not_an_easy_guess() {
        // 0, 1 and 8 are what a model names when it is guessing at a mark rather
        // than reading one, and a lucky guess would record vision that is not there.
        assert!(!matches!(PROBE_DIGIT, 0 | 1 | 8));
    }

    #[test]
    fn the_probe_digit_looks_like_a_digit_rather_than_a_stroke() {
        // The other half of choosing a digit, and the half the first choice missed.
        // 7 lights two segments in this typeface and renders as a corner, which
        // invites "a white shape" as an answer — a false negative produced by the
        // probe's own image rather than by the model.
        let lit = SEGMENTS[(PROBE_DIGIT % 10) as usize]
            .iter()
            .filter(|&&on| on)
            .count();
        assert!(
            lit >= 4,
            "digit {PROBE_DIGIT} lights only {lit} segments, which reads as strokes \
             rather than a numeral"
        );
    }

    #[test]
    fn the_payload_stays_small() {
        // A probe that costs real money discourages re-testing, and re-testing is
        // what the Settings button is for.
        let image = probe_image().expect("encodable");
        assert!(
            image.bytes.len() < 16 * 1024,
            "probe image is {} bytes; base64 inflates it by a third again",
            image.bytes.len()
        );
    }

    #[test]
    fn generation_is_deterministic() {
        // Two runs must agree, or a failure could never be reproduced from a bug
        // report.
        assert_eq!(probe_image(), probe_image());
    }
}
