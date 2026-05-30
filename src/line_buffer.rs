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
/// buffer (interactive prompts have no trailing `\n`). Consumed by
/// `Pipeline::tick`, which the output thread polls on a `poll(2)` timeout of
/// the same duration (`runtime::POLL_TIMEOUT_MS`).
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
    // the live pipeline path uses `feed_data_run` (chunk-level Data runs) or
    // `feed_with_overflow` (sequence scratch). Kept as part of the type's
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

    /// Feed a run of Data bytes (no ESC — caller guarantees it is a Ground run
    /// up to the next ESC) in bulk. Returns one `(line, trailing_newline)` per
    /// emitted line: `line` is the rule-input with any terminating `\n`
    /// stripped (matching the former per-byte `feed_byte_with_overflow`
    /// contract), and `trailing_newline` tells the caller to re-emit a raw
    /// `\n` after applying rules. A single line exceeding `MAX_BUFFER_BYTES`
    /// is flushed as `(blob, false)` with a warning. The partial tail stays
    /// buffered across calls. See spec §5 C-2 (`\n`-strip contract) and I-3
    /// (per-line overflow framing).
    ///
    /// Invariant relied on: `inner.len() <= MAX_BUFFER_BYTES` on entry and after
    /// every emit (overflow/newline emits reset to empty; a buffered tail is
    /// strictly shorter than the distance to the cap), so `until_overflow >= 1`.
    pub(crate) fn feed_data_run(&mut self, run: &[u8]) -> Vec<(Vec<u8>, bool)> {
        if run.is_empty() {
            return Vec::new();
        }
        self.last_write = Instant::now();
        let mut emitted = Vec::new();
        let mut i = 0;
        while i < run.len() {
            // Bytes addable before the (cap+1)th byte triggers an overflow flush.
            let until_overflow = MAX_BUFFER_BYTES + 1 - self.inner.len();
            match memchr_newline(&run[i..]) {
                Some(r) if r < until_overflow => {
                    // A `\n` completes a line at or before the overflow point.
                    // `r + 1 == until_overflow` is the simultaneous overflow+newline
                    // edge: the per-byte path warns, strips the `\n`, re-emits it —
                    // same emitted value as a normal newline, plus the warning.
                    self.inner.extend_from_slice(&run[i..i + r]);
                    if r + 1 == until_overflow {
                        let cap = MAX_BUFFER_BYTES;
                        crate::log::warn_msg!("line buffer overflowed; cap={cap}");
                    }
                    let line = std::mem::take(&mut self.inner);
                    self.inner = Vec::with_capacity(4096);
                    emitted.push((line, true));
                    i += r + 1;
                }
                _ => {
                    if until_overflow <= run.len() - i {
                        // Overflow: the `until_overflow`-th added byte is non-`\n`
                        // (an earlier `\n` would have matched the arm above), so the
                        // blob has no trailing `\n` to strip -> (blob, false).
                        self.inner.extend_from_slice(&run[i..i + until_overflow]);
                        let cap = MAX_BUFFER_BYTES;
                        crate::log::warn_msg!("line buffer overflowed; cap={cap}");
                        let blob = std::mem::take(&mut self.inner);
                        self.inner = Vec::with_capacity(4096);
                        emitted.push((blob, false));
                        i += until_overflow;
                    } else {
                        // Run ends with no `\n` and no overflow: buffer the tail.
                        self.inner.extend_from_slice(&run[i..]);
                        i = run.len();
                    }
                }
            }
        }
        emitted
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

    #[test]
    fn feed_data_run_across_calls_preserves_partial() {
        // A line split across two runs (as a real PTY would deliver) must emit
        // once, on the run carrying the newline, with the joined content.
        let mut buf = LineBuffer::new();
        assert!(buf.feed_data_run(b"abc").is_empty(), "no newline yet -> nothing emitted");
        let lines = buf.feed_data_run(b"def\n");
        assert_eq!(lines, vec![(b"abcdef".to_vec(), true)]);
    }

    #[test]
    fn feed_data_run_emits_expected_static_cases() {
        let mut b = LineBuffer::new();
        assert_eq!(
            b.feed_data_run(b"a\nb\nc\n"),
            vec![(b"a".to_vec(), true), (b"b".to_vec(), true), (b"c".to_vec(), true)]
        );
        assert!(b.drain().is_empty());

        let mut b = LineBuffer::new();
        assert_eq!(b.feed_data_run(b"line1\npartial2"), vec![(b"line1".to_vec(), true)]);
        assert_eq!(b.drain(), b"partial2");

        let mut b = LineBuffer::new();
        let blob = b.feed_data_run(&vec![b'x'; MAX_BUFFER_BYTES + 1]);
        assert_eq!(blob.len(), 1);
        assert!(!blob[0].1, "overflow blob has no trailing newline");
        assert_eq!(blob[0].0.len(), MAX_BUFFER_BYTES + 1);

        // simultaneous overflow + newline -> stripped content + trailing newline.
        let mut b = LineBuffer::new();
        let mut run = vec![b'x'; MAX_BUFFER_BYTES];
        run.push(b'\n');
        let out = b.feed_data_run(&run);
        assert_eq!(out, vec![(vec![b'x'; MAX_BUFFER_BYTES], true)]);
    }
}
