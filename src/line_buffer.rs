//! UTF-8-safe accumulator with timeout flush and a hard cap.
//!
//! Lines are emitted on `\n`, on a 50ms idle timeout, or when the buffer
//! reaches `MAX_BUFFER_BYTES` — in the last case the partial line is flushed
//! *without* rule application (spec §6.1, "Memory exhaustion").

use std::time::{Duration, Instant};

use crate::error::Error;

/// Hard cap on a single accumulated line. Above this, we flush and reset.
pub(crate) const MAX_BUFFER_BYTES: usize = 64 * 1024;

/// Idle timeout. If no newline arrives within this window, flush the partial
/// buffer (interactive prompts have no trailing `\n`).
// reason: consumed by `Pipeline::tick`, which is the spec'd idle-flush hook
// but not yet polled from the v0.1 runtime (see `Pipeline::tick` doc-comment
// and spec §3.4). The constant is exercised by tests and reserved for the
// next runtime iteration.
#[allow(dead_code)]
pub(crate) const FLUSH_TIMEOUT: Duration = Duration::from_millis(50);

/// Accumulator that emits complete lines.
pub(crate) struct LineBuffer {
    inner: Vec<u8>,
    last_write: Instant,
}

impl LineBuffer {
    pub(crate) fn new() -> Self {
        LineBuffer { inner: Vec::with_capacity(4096), last_write: Instant::now() }
    }

    /// Feed bytes; return any complete lines (each ending in `\n`).
    ///
    /// If feeding causes the buffer to exceed `MAX_BUFFER_BYTES`, the entire
    /// accumulated buffer (including the new chunk) is flushed as a single
    /// "line" without regex application. The caller is expected to log the
    /// overflow.
    // reason: thin overflow-discarding wrapper around `feed_with_overflow`;
    // the live pipeline path always uses `feed_with_overflow` so it can
    // surface the warning via tracing. Kept as part of the type's
    // documented surface and exercised by unit tests.
    #[allow(dead_code)]
    pub(crate) fn feed(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        let (lines, _) = self.feed_with_overflow(chunk);
        lines
    }

    /// Variant of `feed` that also reports overflow events to the caller.
    pub(crate) fn feed_with_overflow(&mut self, chunk: &[u8]) -> (Vec<Vec<u8>>, Option<Error>) {
        if chunk.is_empty() {
            return (Vec::new(), None);
        }
        self.last_write = Instant::now();
        self.inner.extend_from_slice(chunk);

        let mut lines = Vec::new();
        let mut overflow = None;

        if self.inner.len() > MAX_BUFFER_BYTES {
            // SAFETY/INVARIANT: The overflow flush may emit bytes that end mid-codepoint
            // (e.g., a 0xC3 lead byte whose continuation arrives in the next feed). This
            // is safe because downstream rule application uses regex::bytes::Regex on raw
            // byte slices, not str, so partial UTF-8 sequences cannot cause UB or panics.
            // The next feed starts from an empty buffer; the dropped tail will resync.
            // Splitting on '\n' below is a separate concern and IS always at a UTF-8
            // codepoint boundary because 0x0A never appears as a continuation byte
            // (continuations are 0x80-0xBF).
            overflow = Some(Error::BufferOverflow { cap: MAX_BUFFER_BYTES });
            lines.push(std::mem::take(&mut self.inner));
            self.inner = Vec::with_capacity(4096);
            return (lines, overflow);
        }

        while let Some(pos) = memchr_newline(&self.inner) {
            let line: Vec<u8> = self.inner.drain(..=pos).collect();
            lines.push(line);
        }

        (lines, overflow)
    }

    /// If the buffer has been idle since `cutoff`, drain and return it.
    // reason: consumed by `Pipeline::tick`, the spec'd idle-flush hook not
    // yet polled from the v0.1 runtime. See the `FLUSH_TIMEOUT` note above.
    #[allow(dead_code)]
    pub(crate) fn flush_if_stale(&mut self, cutoff: Instant) -> Option<Vec<u8>> {
        if self.inner.is_empty() {
            return None;
        }
        if self.last_write <= cutoff {
            Some(std::mem::take(&mut self.inner))
        } else {
            None
        }
    }

    /// Return any remaining bytes (used at shutdown).
    pub(crate) fn drain(&mut self) -> Vec<u8> {
        let out = std::mem::take(&mut self.inner);
        self.inner = Vec::with_capacity(4096);
        out
    }

    #[cfg(test)]
    pub(crate) fn set_last_write_for_test(&mut self, t: Instant) {
        self.last_write = t;
    }
}

fn memchr_newline(haystack: &[u8]) -> Option<usize> {
    haystack.iter().position(|&b| b == b'\n')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_feed_yields_nothing() {
        let mut buf = LineBuffer::new();
        let lines = buf.feed(b"");
        assert!(lines.is_empty());
    }

    #[test]
    fn single_line_with_newline_is_emitted() {
        let mut buf = LineBuffer::new();
        let lines = buf.feed(b"hello world\n");
        assert_eq!(lines, vec![b"hello world\n".to_vec()]);
    }

    #[test]
    fn two_lines_in_one_chunk_are_split() {
        let mut buf = LineBuffer::new();
        let lines = buf.feed(b"a\nb\n");
        assert_eq!(lines, vec![b"a\n".to_vec(), b"b\n".to_vec()]);
    }

    #[test]
    fn partial_line_is_held() {
        let mut buf = LineBuffer::new();
        let lines = buf.feed(b"part");
        assert!(lines.is_empty());
        let lines = buf.feed(b"ial\n");
        assert_eq!(lines, vec![b"partial\n".to_vec()]);
    }

    #[test]
    fn utf8_multibyte_split_across_chunks_is_safe() {
        let mut buf = LineBuffer::new();
        // "üç" = 0xC3 0xBC 0xC3 0xA7 ; split between bytes 1 and 2.
        let _ = buf.feed(&[0xC3]);
        let lines = buf.feed(&[0xBC, 0xC3, 0xA7, b'\n']);
        assert_eq!(lines, vec![vec![0xC3, 0xBC, 0xC3, 0xA7, b'\n']]);
    }

    #[test]
    fn timeout_flush_releases_partial() {
        let mut buf = LineBuffer::new();
        let _ = buf.feed(b"prompt> ");
        // Simulate the buffer having been idle for longer than FLUSH_TIMEOUT.
        let past = Instant::now().checked_sub(Duration::from_millis(100)).unwrap();
        buf.set_last_write_for_test(past);
        // Production callsite uses `now - FLUSH_TIMEOUT`.
        let cutoff = Instant::now().checked_sub(FLUSH_TIMEOUT).unwrap();
        let flushed = buf.flush_if_stale(cutoff);
        assert_eq!(flushed, Some(b"prompt> ".to_vec()));
    }

    #[test]
    fn timeout_flush_holds_recent_partial() {
        let mut buf = LineBuffer::new();
        let _ = buf.feed(b"prompt> ");
        // last_write is set by feed; with a strict-past cutoff the buffer should NOT flush.
        let cutoff = Instant::now().checked_sub(FLUSH_TIMEOUT).unwrap();
        let flushed = buf.flush_if_stale(cutoff);
        assert!(flushed.is_none(), "fresh buffer must not be flushed by stale cutoff");
    }

    #[test]
    fn overflow_flushes_as_is_and_returns_warning() {
        let mut buf = LineBuffer::new();
        let huge = vec![b'x'; MAX_BUFFER_BYTES + 100];
        let (lines, overflow) = buf.feed_with_overflow(&huge);
        assert!(matches!(overflow, Some(Error::BufferOverflow { .. })));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), MAX_BUFFER_BYTES + 100);
    }

    #[test]
    fn drain_partial_returns_remaining() {
        let mut buf = LineBuffer::new();
        let _ = buf.feed(b"hanging");
        let remaining = buf.drain();
        assert_eq!(remaining, b"hanging");
    }

    #[test]
    fn no_overflow_at_exact_cap() {
        let mut buf = LineBuffer::new();
        let payload = vec![b'x'; MAX_BUFFER_BYTES];
        let (lines, overflow) = buf.feed_with_overflow(&payload);
        assert!(overflow.is_none(), "len == cap must not overflow");
        assert!(lines.is_empty(), "no newline, no emit");
    }

    #[test]
    fn overflow_at_cap_plus_one() {
        let mut buf = LineBuffer::new();
        let payload = vec![b'x'; MAX_BUFFER_BYTES + 1];
        let (lines, overflow) = buf.feed_with_overflow(&payload);
        assert!(matches!(overflow, Some(Error::BufferOverflow { .. })));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), MAX_BUFFER_BYTES + 1);
    }

    #[test]
    fn successive_feeds_accumulate_and_overflow_together() {
        let mut buf = LineBuffer::new();
        let (lines1, ovf1) = buf.feed_with_overflow(&vec![b'a'; 32 * 1024]);
        assert!(ovf1.is_none());
        assert!(lines1.is_empty());

        let (lines2, ovf2) = buf.feed_with_overflow(&vec![b'b'; 40 * 1024]);
        assert!(matches!(ovf2, Some(Error::BufferOverflow { .. })));
        assert_eq!(lines2.len(), 1);
        assert_eq!(lines2[0].len(), 72 * 1024);
    }

    #[test]
    fn feed_after_drain_recovers_state() {
        let mut buf = LineBuffer::new();
        let _ = buf.feed(b"orphan");
        let _ = buf.drain();
        let lines = buf.feed(b"fresh line\n");
        assert_eq!(lines, vec![b"fresh line\n".to_vec()]);
    }

    #[test]
    fn utf8_three_byte_split_across_chunks_is_safe() {
        let mut buf = LineBuffer::new();
        // '日' = 0xE6 0x97 0xA5; split between first and second byte.
        let _ = buf.feed(&[0xE6]);
        let lines = buf.feed(&[0x97, 0xA5, b'\n']);
        assert_eq!(lines, vec![vec![0xE6, 0x97, 0xA5, b'\n']]);
    }

    #[test]
    fn utf8_four_byte_split_across_three_chunks_is_safe() {
        let mut buf = LineBuffer::new();
        // '🦀' = 0xF0 0x9F 0xA6 0x80 (crab emoji); split across three feeds.
        let _ = buf.feed(&[0xF0]);
        let _ = buf.feed(&[0x9F, 0xA6]);
        let lines = buf.feed(&[0x80, b'\n']);
        assert_eq!(lines, vec![vec![0xF0, 0x9F, 0xA6, 0x80, b'\n']]);
    }
}
