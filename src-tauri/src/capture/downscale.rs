//! How big a screenshot should be before it is encoded.
//!
//! Pure arithmetic over dimensions. No image data, no provider, no network — which is what
//! lets the interesting cases be tested without a display, and lets the reference vector
//! Anthropic publishes be asserted directly.
//!
//! ## Why this matters more than it looks
//!
//! The design doc's cost argument is that conversation history is resent on every request,
//! so an image attached once is paid for on every later turn — quadratic in thread length.
//! That argument is right, but its arithmetic needs updating and the conclusion shifts.
//!
//! Anthropic's documented cost is **one visual token per 28×28 pixel patch**, and the
//! standard resolution tier caps an image at **1568 tokens and a 1568-pixel long edge**. An
//! image over that is resized *server-side* before it is charged. So a 3024×1964 Retina
//! capture and a 1568-token thumbnail cost the **same tokens**: the extra pixels are
//! discarded after being uploaded.
//!
//! What the extra pixels do cost is bytes. A full Retina PNG is megabytes, base64 inflates
//! it by a third, and the design's own point — history is resent every turn — applies to
//! those bytes too. Downscaling before encoding is therefore about upload size and latency
//! first, and about tokens only when the budget is deliberately set below the cap.
//!
//! A second reason to do the resize here rather than let the server do it: resizing twice
//! blurs. Matching the documented algorithm means Magi hands over an image the server will
//! pass through unchanged.
//!
//! ## Why Anthropic's numbers, for every provider
//!
//! The OpenAI family tiles differently and Ollama depends on the model. Rather than a
//! per-provider table that would need maintaining against five vendors, this targets the
//! tightest documented budget of the providers Magi supports. An image small enough for
//! Anthropic's standard tier is small enough everywhere else; a per-provider optimum is a
//! refinement worth having only once there is evidence that it matters.

/// The longest edge, in pixels, that Anthropic's standard resolution tier accepts without
/// resizing.
///
/// Their high-resolution tier allows 2576 with a 4784-token budget. Not used: the point of
/// this module is to make screenshots cheap, and a tier that costs three times as much for
/// detail nobody asked for is the wrong default for a screenshot of a text editor.
pub const MAX_EDGE: u32 = 1568;

/// The visual-token budget for one image, on the same tier.
///
/// Per Anthropic's own documentation this, not [`MAX_EDGE`], is the binding constraint for
/// most images — the edge limit only bites on elongated ones like panoramas. Which is to
/// say: an ultrawide monitor is the case where the edge limit does the work.
pub const MAX_TOKENS: u32 = 1568;

/// The side of one visual token's patch, in pixels.
const PATCH: u32 = 28;

/// What an image of these dimensions costs, in visual tokens.
///
/// `ceil(width / 28) * ceil(height / 28)`. A partial patch is a whole token, which is why
/// this rounds up on each axis independently rather than dividing the area.
pub fn visual_tokens(width: u32, height: u32) -> u32 {
    width.div_ceil(PATCH) * height.div_ceil(PATCH)
}

/// Whether an image of these dimensions is accepted unchanged.
fn fits(width: u32, height: u32) -> bool {
    width.div_ceil(PATCH) * PATCH <= MAX_EDGE
        && height.div_ceil(PATCH) * PATCH <= MAX_EDGE
        && visual_tokens(width, height) <= MAX_TOKENS
}

/// The dimensions to resize to before encoding.
///
/// Returns the input unchanged when it already fits, so this **never upscales** — a
/// 320×240 window stays 320×240 rather than being blown up to spend a budget it does not
/// need.
///
/// The algorithm is Anthropic's, reproduced rather than approximated: binary search along
/// the long edge for the largest aspect-preserving size that satisfies both limits, with
/// the short edge rounding **half to even**. That last detail is not decoration — the live
/// API rounds ties to even, and `f64::round` rounds them away from zero, so the two
/// disagree on exact `.5` ties and an image sized by the wrong rule gets resized again on
/// arrival. `docs/…/vision-coordinates` publishes 1075×1520 → 924×1307 as a reference
/// vector; the tests assert it.
pub fn target_size(width: u32, height: u32) -> (u32, u32) {
    // A zero-sized capture is not something to divide by. Returning it unchanged lets the
    // caller fail on the real problem — an empty image — rather than on a panic here.
    if width == 0 || height == 0 {
        return (width, height);
    }

    if fits(width, height) {
        return (width, height);
    }

    // Solve for landscape only, and transpose. Halves the reasoning and the tests.
    if height > width {
        let (transposed_width, transposed_height) = target_size(height, width);
        return (transposed_height, transposed_width);
    }

    let aspect = f64::from(width) / f64::from(height);
    let short_edge = |long_edge: u32| -> u32 {
        let short = (f64::from(long_edge) / aspect).round_ties_even();
        // `as u32` saturates at zero for negatives and cannot overflow from a value
        // bounded by `width`, so the only case to guard is a sub-pixel short edge on an
        // extreme aspect ratio.
        (short as u32).max(1)
    };

    // `lo` always fits — a 1×1 image costs one token. `hi` never fits, because the
    // early return above already rejected `width`. The invariant is what makes the loop
    // terminate with `lo` as the largest fitting long edge.
    let mut lo = 1;
    let mut hi = width;
    while lo + 1 < hi {
        let mid = lo + (hi - lo) / 2;
        if fits(mid, short_edge(mid)) {
            lo = mid;
        } else {
            hi = mid;
        }
    }

    (lo, short_edge(lo))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_published_reference_vector_matches() {
        // Anthropic documents this exact pair under "How Claude resizes and pads images".
        // If our arithmetic ever drifts from theirs, this is the test that says so — and
        // drift means every capture gets resized twice and arrives blurrier than it left.
        assert_eq!(target_size(1075, 1520), (924, 1307));
    }

    #[test]
    fn a_retina_capture_is_brought_under_budget() {
        // The case that actually happens: a 1512×982 "logical" MacBook display captures at
        // 3024×1964 physical pixels. Unresized that is 108 × 71 = 7668 visual tokens, so
        // nearly five times the budget — and every byte of it would be re-uploaded on
        // every later turn in the thread.
        assert_eq!(visual_tokens(3024, 1964), 7668);

        let (width, height) = target_size(3024, 1964);
        assert!(
            visual_tokens(width, height) <= MAX_TOKENS,
            "{width}×{height} costs {} tokens",
            visual_tokens(width, height)
        );
        assert!(width <= MAX_EDGE && height <= MAX_EDGE);
    }

    #[test]
    fn the_design_docs_example_was_already_over_budget() {
        // The design doc estimates "roughly 1,100 vision tokens" for a 1512×982
        // screenshot. Under Anthropic's documented formula it is 54 × 36 = 1944, over the
        // 1568 cap, so it would be resized server-side. Recorded here because the
        // conclusion the doc draws from it — that images dominate cost — survives, while
        // the number does not.
        assert_eq!(visual_tokens(1512, 982), 1944);
        assert!(!fits(1512, 982));
    }

    #[test]
    fn something_already_small_is_left_alone() {
        // Never upscale. A small window or a dialog should not be inflated to spend a
        // budget it has no detail to fill.
        assert_eq!(target_size(320, 240), (320, 240));
        assert_eq!(target_size(1, 1), (1, 1));
        assert_eq!(target_size(1024, 768), (1024, 768));
    }

    #[test]
    fn the_token_limit_binds_long_before_the_edge_limit() {
        // Worth pinning, because "1568" reads like a pixel budget and is not one. At 16:10
        // the token cap is reached around 1372×882 — well under 1568 on both edges — so an
        // image can be comfortably inside the edge limit and still be resized. Anthropic
        // says as much: the token limit is the primary constraint and the edge limit only
        // bites on elongated images.
        assert!(fits(1372, 882));
        assert_eq!(visual_tokens(1372, 882), MAX_TOKENS);

        assert!(
            !fits(1400, 900),
            "under 1568px on both edges, yet over budget"
        );
        assert_eq!(visual_tokens(1400, 900), 1650);

        // A square at the edge limit is more than twice the budget.
        assert!(!fits(MAX_EDGE, MAX_EDGE));
        assert_eq!(visual_tokens(MAX_EDGE, MAX_EDGE), 3136);
    }

    #[test]
    fn the_result_always_fits_both_limits() {
        // Every shape that a real display or window can plausibly be, including the
        // extremes: an ultrawide where the edge limit binds, and a tall sidebar where it
        // binds after transposing.
        let shapes = [
            (3024, 1964), // 14" MacBook Pro, Retina
            (5120, 2880), // 5K iMac
            (7680, 2160), // dual 4K side by side
            (3440, 1440), // ultrawide
            (1964, 3024), // portrait rotation
            (400, 3000),  // a tall narrow sidebar
            (3000, 400),  // a wide short strip
            (2, 4000),    // degenerate, but must not panic or return zero
        ];

        for (width, height) in shapes {
            let (new_width, new_height) = target_size(width, height);
            assert!(
                new_width >= 1 && new_height >= 1,
                "{width}×{height} collapsed to {new_width}×{new_height}"
            );
            assert!(
                visual_tokens(new_width, new_height) <= MAX_TOKENS,
                "{width}×{height} → {new_width}×{new_height} costs {} tokens",
                visual_tokens(new_width, new_height)
            );
            assert!(
                new_width.div_ceil(PATCH) * PATCH <= MAX_EDGE
                    && new_height.div_ceil(PATCH) * PATCH <= MAX_EDGE,
                "{width}×{height} → {new_width}×{new_height} exceeds the edge limit"
            );
        }
    }

    #[test]
    fn the_aspect_ratio_survives() {
        // A distorted screenshot is a wrong answer waiting to happen: the model is being
        // asked to read what is on screen, and stretched text and misplaced coordinates
        // are exactly what it would get wrong.
        for (width, height) in [(3024, 1964), (3440, 1440), (5120, 2880)] {
            let (new_width, new_height) = target_size(width, height);
            let before = f64::from(width) / f64::from(height);
            let after = f64::from(new_width) / f64::from(new_height);
            assert!(
                (before - after).abs() / before < 0.01,
                "{width}×{height} → {new_width}×{new_height}: {before} vs {after}"
            );
        }
    }

    #[test]
    fn transposing_transposes_the_result() {
        // Landscape is solved and portrait is derived from it, so the two must agree.
        let (width, height) = target_size(3024, 1964);
        assert_eq!(target_size(1964, 3024), (height, width));
    }

    #[test]
    fn the_result_is_the_largest_that_fits() {
        // The binary search is only correct if it stops at the boundary rather than near
        // it. One pixel wider on the long edge must not fit.
        let (width, height) = target_size(3024, 1964);
        assert!(fits(width, height));

        let aspect = 3024.0 / 1964.0;
        let wider = width + 1;
        let taller = ((f64::from(wider) / aspect).round_ties_even() as u32).max(1);
        assert!(
            !fits(wider, taller),
            "{wider}×{taller} also fits, so {width}×{height} was not the largest"
        );
    }

    #[test]
    fn a_zero_dimension_does_not_panic() {
        // Reachable: a display that reports nothing while it is being reconfigured, or a
        // window that closed between enumeration and capture.
        assert_eq!(target_size(0, 1080), (0, 1080));
        assert_eq!(target_size(1920, 0), (1920, 0));
        assert_eq!(target_size(0, 0), (0, 0));
        assert_eq!(visual_tokens(0, 0), 0);
    }
}
