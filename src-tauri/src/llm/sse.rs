//! Server-sent events, parsed as a pure function over bytes.
//!
//! No I/O, no async, no knowledge of any provider. That is what lets the tests
//! feed it a recorded body one byte at a time and prove the framing is handled
//! rather than assumed.
//!
//! The defect this module exists to prevent: treating one network chunk as one
//! frame. That holds against OpenAI often enough to survive development and
//! breaks against local backends, which split JSON mid-object — producing a
//! parse error on a perfectly valid answer.

/// One `data:` block, with its optional `event:` name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseFrame {
    /// Present on Anthropic's stream, absent on the OpenAI family's.
    pub event: Option<String>,
    /// Multi-line `data:` fields joined with newlines, per the SSE spec.
    pub data: String,
}

/// Accumulates bytes and emits frames as they complete.
#[derive(Default)]
pub struct SseParser {
    /// Bytes not yet forming a complete line. Holds a partial UTF-8 sequence
    /// across chunk boundaries, so a multi-byte character split by the network
    /// is reassembled instead of mangled.
    buffer: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
}

impl SseParser {
    /// Feeds a chunk and returns whatever frames it completed.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseFrame> {
        self.buffer.extend_from_slice(chunk);

        let mut frames = Vec::new();
        while let Some(newline) = self.buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=newline).collect();
            // Trailing \n, and \r before it on servers that send CRLF.
            let line = &line[..line.len() - 1];
            let line = line.strip_suffix(b"\r").unwrap_or(line);

            if let Some(frame) = self.consume_line(&String::from_utf8_lossy(line)) {
                frames.push(frame);
            }
        }
        frames
    }

    /// Emits a trailing frame if the stream ended without a blank line.
    ///
    /// Several local servers just close the connection after the last frame.
    /// Discarding it would drop the final token of every answer — a defect that
    /// reads as the model truncating rather than the parser being wrong.
    pub fn finish(&mut self) -> Vec<SseFrame> {
        if !self.buffer.is_empty() {
            let rest = std::mem::take(&mut self.buffer);
            self.consume_line(&String::from_utf8_lossy(&rest));
        }
        self.take_frame().into_iter().collect()
    }

    fn consume_line(&mut self, line: &str) -> Option<SseFrame> {
        // A blank line terminates the current frame.
        if line.is_empty() {
            return self.take_frame();
        }

        // Lines beginning with a colon are comments; servers use them as
        // keepalives. Treating them as data emits empty tokens.
        if line.starts_with(':') {
            return None;
        }

        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            // A field with no colon is a field with an empty value, per spec.
            None => (line, ""),
        };

        match field {
            "data" => self.data.push(value.to_string()),
            "event" => self.event = Some(value.to_string()),
            // `id` and `retry` are part of SSE but mean nothing to Magi.
            _ => {}
        }
        None
    }

    fn take_frame(&mut self) -> Option<SseFrame> {
        if self.data.is_empty() {
            // Blank lines with nothing buffered are separators, not empty frames.
            self.event = None;
            return None;
        }
        Some(SseFrame {
            event: self.event.take(),
            data: std::mem::take(&mut self.data).join("\n"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds a body one byte at a time. If the parser survives this it survives
    /// any framing a server can produce.
    fn parse_byte_by_byte(body: &str) -> Vec<SseFrame> {
        let mut parser = SseParser::default();
        let mut frames = Vec::new();
        for byte in body.as_bytes() {
            frames.extend(parser.push(&[*byte]));
        }
        frames.extend(parser.finish());
        frames
    }

    fn parse_whole(body: &str) -> Vec<SseFrame> {
        let mut parser = SseParser::default();
        let mut frames = parser.push(body.as_bytes());
        frames.extend(parser.finish());
        frames
    }

    #[test]
    fn parses_a_single_frame() {
        let body = "data: {\"a\":1}\n\n";
        for frames in [parse_whole(body), parse_byte_by_byte(body)] {
            assert_eq!(frames.len(), 1);
            assert_eq!(frames[0].data, "{\"a\":1}");
        }
    }

    #[test]
    fn chunk_boundaries_do_not_change_the_result() {
        // The defect this whole module exists to prevent: a parser that treats
        // one network chunk as one frame works against OpenAI and fails against
        // Ollama, which splits mid-JSON.
        let body = "data: {\"first\":1}\n\ndata: {\"second\":2}\n\n";
        assert_eq!(parse_whole(body), parse_byte_by_byte(body));
        assert_eq!(parse_whole(body).len(), 2);
    }

    #[test]
    fn several_frames_can_arrive_in_one_chunk() {
        let frames = parse_whole("data: a\n\ndata: b\n\ndata: c\n\n");
        let payloads: Vec<_> = frames.iter().map(|f| f.data.as_str()).collect();
        assert_eq!(payloads, ["a", "b", "c"]);
    }

    #[test]
    fn comment_lines_are_ignored() {
        // Keepalives. A parser that treats them as data emits empty tokens.
        let frames = parse_whole(": keepalive\n\ndata: real\n\n");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, "real");
    }

    #[test]
    fn the_event_field_is_captured() {
        // Anthropic names its events; the OpenAI family does not.
        let frames = parse_whole("event: content_block_delta\ndata: {\"x\":1}\n\n");
        assert_eq!(frames[0].event.as_deref(), Some("content_block_delta"));
    }

    #[test]
    fn multi_line_data_is_joined_with_newlines() {
        let frames = parse_whole("data: line one\ndata: line two\n\n");
        assert_eq!(frames[0].data, "line one\nline two");
    }

    #[test]
    fn carriage_returns_are_tolerated() {
        let frames = parse_whole("data: value\r\n\r\n");
        assert_eq!(frames[0].data, "value");
    }

    #[test]
    fn a_leading_space_after_the_colon_is_optional() {
        assert_eq!(parse_whole("data:no-space\n\n")[0].data, "no-space");
        assert_eq!(parse_whole("data: with-space\n\n")[0].data, "with-space");
    }

    #[test]
    fn a_stream_ending_without_a_blank_line_still_yields_its_last_frame() {
        // Several local servers close the connection after the final frame
        // without the terminating blank line. Dropping it would lose the last
        // token of every answer — a defect that looks like the model being
        // truncated rather than the parser being wrong.
        let frames = parse_whole("data: last\n");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, "last");
    }

    #[test]
    fn the_done_sentinel_is_reported_as_a_frame_not_swallowed() {
        // Deciding what [DONE] means belongs to the provider, not here. The
        // OpenAI family sends it; Anthropic does not.
        let frames = parse_whole("data: [DONE]\n\n");
        assert_eq!(frames[0].data, "[DONE]");
    }

    #[test]
    fn an_empty_stream_yields_nothing() {
        assert!(parse_whole("").is_empty());
    }

    #[test]
    fn blank_lines_alone_do_not_emit_empty_frames() {
        assert!(parse_whole("\n\n\n\n").is_empty());
    }

    #[test]
    fn invalid_utf8_does_not_lose_the_rest_of_the_stream() {
        // A multi-byte character split across chunks must not be mangled, and a
        // genuinely invalid byte must not abort the connection.
        let mut parser = SseParser::default();
        let mut frames = parser.push("data: caf".as_bytes());
        frames.extend(parser.push(&[0xC3])); // first half of é
        frames.extend(parser.push(&[0xA9])); // second half
        frames.extend(parser.push(b"\n\n"));
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, "café");
    }
}
