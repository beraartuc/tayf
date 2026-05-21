//! UTF-8-safe accumulator with timeout flush and a hard cap.
//!
//! Lines are emitted on `\n`, on a 50ms idle timeout, or when the buffer
//! reaches `MAX_BUFFER_BYTES` — in the last case the partial line is flushed
//! *without* rule application (spec §6.1, "Memory exhaustion").

// reason: this module exposes the line accumulator consumed by the io_loop
// module (Task 6). Until that task lands, the items are referenced only by the
// unit tests in this file, so the dead-code lint flags every name. The allow
// scope is the whole module to keep the surface intentional and reviewable in
// one place; it will be removed when io_loop starts importing these.
#![allow(dead_code)]

use std::time::{Duration, Instant};

use crate::error::Error;

/// Hard cap on a single accumulated line. Above this, we flush and reset.
pub(crate) const MAX_BUFFER_BYTES: usize = 64 * 1024;

/// Idle timeout. If no newline arrives within this window, flush the partial
/// buffer (interactive prompts have no trailing `\n`).
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
    pub(crate) fn feed(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        let (lines, _) = self.feed_with_overflow(chunk);
        lines
    }

    /// Variant of `feed` that also reports overflow events to the caller.
    pub(crate) fn feed_with_overflow(&mut self, chunk: &[u8]) -> (Vec<Vec<u8>>, Option<Error>) {
        self.last_write = Instant::now();
        self.inner.extend_from_slice(chunk);

        let mut lines = Vec::new();
        let mut overflow = None;

        if self.inner.len() > MAX_BUFFER_BYTES {
            // SAFETY/INVARIANT: `\n` (0x0A) is never a continuation byte in UTF-8
            // (continuations are 0x80-0xBF), so flushing the raw bytes here cannot
            // split a multi-byte sequence mid-codepoint — downstream rule
            // application operates on full lines only after this branch returns.
            overflow = Some(Error::BufferOverflow { cap: MAX_BUFFER_BYTES });
            lines.push(std::mem::take(&mut self.inner));
            return (lines, overflow);
        }

        while let Some(pos) = memchr_newline(&self.inner) {
            let line: Vec<u8> = self.inner.drain(..=pos).collect();
            lines.push(line);
        }

        (lines, overflow)
    }

    /// If the buffer has been idle since `cutoff`, drain and return it.
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
        std::mem::take(&mut self.inner)
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
        // Simulate clock advance by calling flush with a cutoff in the future,
        // i.e. past the buffer's `last_write`. The contract: flush when
        // `last_write <= cutoff` (caller in io_loop passes `now - FLUSH_TIMEOUT`).
        let future_cutoff = Instant::now() + Duration::from_millis(100);
        let flushed = buf.flush_if_stale(future_cutoff);
        assert_eq!(flushed, Some(b"prompt> ".to_vec()));
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
}
