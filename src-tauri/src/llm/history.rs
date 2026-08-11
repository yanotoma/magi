//! Keeping a conversation inside a budget.
//!
//! Nothing bounded the history until now: it grew until the model refused it, and with a
//! screenshot resent on every turn it got there faster than length alone suggests. A thread
//! of six questions with one image in it has paid for that image six times.
//!
//! ## The rule that makes this delicate
//!
//! Both APIs reject a request in which a tool call has no matching result — the
//! `llm-providers` skill records it as "a missing result for an emitted call is rejected".
//! So the unit that may be dropped is **not a message**. Dropping a `ToolResult` orphans the
//! call above it; dropping the assistant turn that made the call orphans the result below.
//! Either produces an error rather than a shorter conversation, which is the worst kind of
//! saving.
//!
//! [`fit`] therefore drops whole *exchanges*: a question and everything that answered it,
//! tool calls and their results included.
//!
//! ## Estimated text, exact images
//!
//! Text is estimated, and there is no honest way around it — the tokeniser belongs to the
//! model and differs between them. Images are not estimated: Magi encoded them, so their
//! dimensions are in the PNG header and their cost follows from the same arithmetic
//! `capture::downscale` uses. Since images dominate a thread's cost, the half that can be
//! exact is the half that matters.

use crate::capture::downscale;
use crate::llm::provider::{Image, Message};

/// Characters per token, for estimating text.
///
/// Four is the usual rule of thumb across these tokenisers for English, and less accurate for
/// Spanish and worse for CJK — which is why [`estimate_tokens`] rounds up and adds a
/// per-message overhead. This is a bound to stay under, not a figure to report.
const CHARS_PER_TOKEN: u32 = 4;

/// Added per message for the role and framing every family wraps around one.
const MESSAGE_OVERHEAD: u32 = 4;

/// Roughly what a message costs.
///
/// Deliberately an over-estimate. Being wrong in the direction of truncating slightly early
/// costs a turn of context; being wrong the other way costs the whole request, and the error
/// arrives as a provider rejection the user cannot act on.
pub fn estimate_tokens(message: &Message) -> u32 {
    let text = message.text().chars().count() as u32;
    let mut total = MESSAGE_OVERHEAD + text.div_ceil(CHARS_PER_TOKEN);

    for image in images_of(message) {
        total = total.saturating_add(image_tokens(image));
    }

    total
}

/// The images attached to a message, whichever kind it is.
fn images_of(message: &Message) -> &[Image] {
    match message {
        Message::User { images, .. } | Message::ToolResult { images, .. } => images,
        // A tool call's arguments are counted as part of the text estimate above; they are
        // short by construction, since the only tool takes a target and a sentence.
        Message::Assistant { .. } => &[],
    }
}

/// What an image costs, from its own header.
///
/// Exact rather than estimated, because Magi encoded the PNG and the dimensions are eight
/// bytes at a fixed offset. Falls back to the full budget when the header cannot be read: an
/// image whose size is unknown is treated as the most expensive thing it could be, so an
/// unreadable header truncates more rather than overflowing the request.
fn image_tokens(image: &Image) -> u32 {
    match png_dimensions(&image.bytes) {
        Some((width, height)) => downscale::visual_tokens(width, height),
        None => {
            tracing::warn!(
                bytes = image.bytes.len(),
                "could not read a PNG header; assuming the worst for the budget"
            );
            downscale::MAX_TOKENS
        }
    }
}

/// Width and height from a PNG's `IHDR` chunk.
///
/// The signature is eight bytes, then a four-byte length, then `IHDR`, then the two
/// dimensions as big-endian `u32`s — so they sit at offsets 16 and 20 of every valid PNG.
/// Checked rather than assumed: the signature has to match, because reading four bytes at a
/// fixed offset of something that is not a PNG produces a plausible number.
fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }

    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);

    // A zero dimension is not a size, and it would make the cost zero — the one answer that
    // would let an image slip past the budget entirely.
    (width > 0 && height > 0).then_some((width, height))
}

/// Trims `messages` to fit `budget`, dropping the oldest exchanges first.
///
/// **The newest exchange always survives**, even when it alone exceeds the budget. There is
/// no useful request without the question being asked, and sending an over-budget one at
/// least produces the provider's own error rather than silently asking nothing.
///
/// An exchange begins at a [`Message::User`] and runs to just before the next one, so an
/// assistant turn and the tool results answering it are never separated — see the module
/// docs for why that is a correctness property rather than a preference.
pub fn fit(messages: Vec<Message>, budget: u32) -> Vec<Message> {
    let exchanges = split_into_exchanges(messages);
    if exchanges.is_empty() {
        return Vec::new();
    }

    // From the newest backwards, keeping what fits. The newest is taken unconditionally.
    let mut kept: Vec<Vec<Message>> = Vec::new();
    let mut spent = 0_u32;

    for (index, exchange) in exchanges.iter().enumerate().rev() {
        let cost: u32 = exchange.iter().map(estimate_tokens).sum();
        let newest = index == exchanges.len() - 1;

        if !newest && spent.saturating_add(cost) > budget {
            // Stop rather than skip. Keeping an older exchange after dropping a newer one
            // would hand the model a conversation with a hole in the middle, which reads as
            // the user having changed the subject and then changed back.
            break;
        }

        spent = spent.saturating_add(cost);
        kept.push(exchange.clone());
    }

    kept.reverse();
    kept.into_iter().flatten().collect()
}

/// Groups messages so that a call and its result stay together.
///
/// A leading run that does not begin with a [`Message::User`] becomes its own group rather
/// than being discarded. That should not happen, and if it does the messages are still
/// well-formed relative to each other, so keeping them is safer than dropping something the
/// caller believed it had sent.
fn split_into_exchanges(messages: Vec<Message>) -> Vec<Vec<Message>> {
    let mut exchanges: Vec<Vec<Message>> = Vec::new();

    for message in messages {
        let starts_one = matches!(message, Message::User { .. });
        if starts_one || exchanges.is_empty() {
            exchanges.push(vec![message]);
        } else {
            // `last_mut` cannot be `None` here: the branch above pushes when empty.
            if let Some(current) = exchanges.last_mut() {
                current.push(message);
            }
        }
    }

    exchanges
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::provider::ToolCall;

    /// A PNG header for the given size, with no pixel data.
    ///
    /// Enough for the budget, which only reads the first 24 bytes. Using a real encoder here
    /// would test the encoder rather than this.
    fn png_of(width: u32, height: u32) -> Image {
        let mut bytes = Vec::from(*b"\x89PNG\r\n\x1a\n");
        bytes.extend_from_slice(&13_u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        Image {
            media_type: "image/png",
            bytes,
        }
    }

    fn a_call() -> ToolCall {
        ToolCall {
            id: "call_1".to_string(),
            name: "capture_screen".to_string(),
            arguments: serde_json::json!({ "target": "focused_window", "reason": "read it" }),
        }
    }

    /// A question, an assistant turn that called a tool, the result, and an answer.
    fn an_exchange_with_a_capture(question: &str) -> Vec<Message> {
        vec![
            Message::user(question),
            Message::Assistant {
                text: String::new(),
                calls: vec![a_call()],
            },
            Message::ToolResult {
                call_id: "call_1".to_string(),
                text: "Screenshot of Main display.".to_string(),
                images: vec![png_of(1372, 882)],
            },
            Message::assistant("It says the build failed."),
        ]
    }

    #[test]
    fn a_conversation_that_fits_is_untouched() {
        let messages = vec![Message::user("hi"), Message::assistant("hello")];
        assert_eq!(fit(messages.clone(), 10_000), messages);
    }

    #[test]
    fn the_oldest_exchange_goes_first() {
        let mut messages = vec![Message::user("first"), Message::assistant("one")];
        messages.extend([Message::user("second"), Message::assistant("two")]);
        messages.extend([Message::user("third"), Message::assistant("three")]);

        // Room for two exchanges, not three.
        let per_exchange: u32 = messages[..2].iter().map(estimate_tokens).sum();
        let kept = fit(messages, per_exchange * 2 + 1);

        let texts: Vec<&str> = kept.iter().map(|message| message.text()).collect();
        assert_eq!(texts, ["second", "two", "third", "three"]);
    }

    #[test]
    fn a_tool_call_is_never_separated_from_its_result() {
        // The correctness property. Both APIs reject a call with no matching result, so a
        // truncation that split one would not shorten the conversation — it would break the
        // request. Budgeted so only the newest exchange fits.
        let mut messages = an_exchange_with_a_capture("old question");
        messages.extend(an_exchange_with_a_capture("new question"));

        let newest: u32 = messages[4..].iter().map(estimate_tokens).sum();
        let kept = fit(messages, newest);

        let calls = kept
            .iter()
            .filter(
                |message| matches!(message, Message::Assistant { calls, .. } if !calls.is_empty()),
            )
            .count();
        let results = kept
            .iter()
            .filter(|message| matches!(message, Message::ToolResult { .. }))
            .count();

        assert_eq!(
            calls, results,
            "a call was left without its result: {kept:?}"
        );
        assert_eq!(kept.len(), 4, "a whole exchange should have survived");
        assert_eq!(kept[0].text(), "new question");
    }

    #[test]
    fn the_newest_exchange_survives_even_when_it_alone_is_too_big() {
        // There is no useful request without the question being asked. An over-budget one at
        // least produces the provider's own error, which names a limit; sending nothing
        // produces an answer to a question nobody asked.
        let messages = an_exchange_with_a_capture("what is this");
        let kept = fit(messages.clone(), 1);
        assert_eq!(kept, messages);
    }

    #[test]
    fn nothing_in_means_nothing_out() {
        assert!(fit(Vec::new(), 1000).is_empty());
    }

    #[test]
    fn a_hole_is_never_left_in_the_middle() {
        // Stopping rather than skipping. A conversation missing a turn from its middle reads
        // as the user having changed the subject and then changed back, and a model will
        // answer the question it appears to have been asked.
        let mut messages = vec![Message::user("first"), Message::assistant("one")];
        // An expensive middle exchange that will not fit.
        messages.extend([
            Message::user("second"),
            Message::assistant("x".repeat(4_000)),
        ]);
        messages.extend([Message::user("third"), Message::assistant("three")]);

        let kept = fit(messages, 200);
        let texts: Vec<&str> = kept.iter().map(|message| message.text()).collect();
        assert_eq!(
            texts,
            ["third", "three"],
            "the first exchange was kept across a dropped middle one"
        );
    }

    #[test]
    fn an_image_costs_what_the_capture_module_says_it_does() {
        // Exact, not estimated: Magi encoded the PNG, so the size is in its header and the
        // cost follows from the same arithmetic the downscaler uses. Images dominate a
        // thread's cost, so the half that can be exact is the half that matters.
        let message = Message::user_seeing("look", vec![png_of(1372, 882)]);
        let expected = downscale::visual_tokens(1372, 882);

        let estimated = estimate_tokens(&message);
        assert!(
            estimated >= expected,
            "{estimated} does not cover the image's {expected}"
        );
        assert!(
            estimated < expected + 20,
            "{estimated} is far above the image's {expected} plus a short text"
        );
    }

    #[test]
    fn an_unreadable_header_is_assumed_to_be_expensive() {
        // The safe direction. An image of unknown size treated as free is how one slips past
        // the budget; treated as the most it could be, it only truncates more.
        let broken = Image {
            media_type: "image/png",
            bytes: vec![0; 24],
        };
        assert!(estimate_tokens(&Message::user_seeing("x", vec![broken])) >= downscale::MAX_TOKENS);
    }

    #[test]
    fn a_zero_sized_image_is_not_free() {
        // The one dimension that would make the arithmetic return zero, which is the only
        // answer that lets an image through unbudgeted.
        let zero = png_of(0, 0);
        assert!(estimate_tokens(&Message::user_seeing("x", vec![zero])) >= downscale::MAX_TOKENS);
    }

    #[test]
    fn something_that_is_not_a_png_is_not_measured_as_one() {
        // Reading four bytes at a fixed offset of arbitrary data produces a plausible
        // number, which is why the signature is checked rather than assumed.
        assert_eq!(png_dimensions(b"not a png at all, but long enough"), None);
    }

    #[test]
    fn text_is_over_estimated_rather_than_under() {
        // Wrong in the direction of truncating early costs a turn of context. Wrong the
        // other way costs the request, and arrives as a provider rejection the user cannot
        // act on.
        let message = Message::user("a".repeat(400));
        assert!(estimate_tokens(&message) >= 100);
    }

    #[test]
    fn a_leading_orphan_is_kept_rather_than_dropped() {
        // Should not happen, and if it does the messages are still well-formed relative to
        // each other — so keeping them is safer than discarding something the caller
        // believed it had sent.
        let messages = vec![Message::assistant("orphaned"), Message::user("question")];
        assert_eq!(fit(messages.clone(), 10_000), messages);
    }
}
