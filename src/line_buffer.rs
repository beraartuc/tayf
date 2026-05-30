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
    // the live pipeline path always uses `feed_with_overflow` so it can
    // surface the warning via `crate::log::warn_msg!`. Kept as part of the
    // type's documented surface and exercised by unit tests.
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

    /// Feed a single byte. Returns `Some(line)` when the byte completes a
    /// line (newline seen) or when adding the byte would exceed the buffer
    /// cap (in which case the accumulated bytes are returned as the line;
    /// a warning is logged internally).
    ///
    /// Mirrors [`Self::feed_with_overflow`] semantics one byte at a time,
    /// except the trailing `\n` is stripped from the returned line for
    /// consumers that route bytes through `AnsiSm::step` (spec §6.1). Used
    /// by the per-byte pipeline path in `pipeline::Pipeline::feed`.
    pub(crate) fn feed_byte_with_overflow(&mut self, byte: u8) -> Option<Vec<u8>> {
        // Per-byte hot path. The general `feed_with_overflow` rescans the whole
        // accumulated buffer for newlines on every call (O(L²) across a line);
        // here we exploit the invariant that `inner` never holds a `\n` between
        // calls (every completed line drains it, and the multi-byte path drains
        // all newlines before returning). So after pushing one byte, the only
        // possible newline is that byte. Push + a single comparison = O(1).
        self.last_write = Instant::now();
        self.inner.push(byte);

        let overflow = self.inner.len() > MAX_BUFFER_BYTES;
        if overflow {
            let cap = MAX_BUFFER_BYTES;
            crate::log::warn_msg!("line buffer overflowed; cap={cap}");
        } else if byte != b'\n' {
            return None;
        }

        // Either the cap was exceeded, or this byte completed a line: flush
        // `inner` as the line and reset. Strip a trailing `\n` so byte consumers
        // get a clean payload (matches the prior contract; overflow flushes that
        // do not end in `\n` are returned verbatim).
        let mut line = std::mem::take(&mut self.inner);
        self.inner = Vec::with_capacity(4096);
        // Fast-path invariant guard. The O(1) path is only correct because the
        // buffer never holds an *interior* `\n` — every newline flushes
        // immediately, so the only `\n` a flushed line can contain is its final
        // byte. Checked once per flush (O(line), i.e. O(total bytes) overall —
        // NOT the per-byte O(L²) scan this method just removed) and compiled
        // out in release. If a future change (e.g. the H4 chunk-level rewrite)
        // ever lets a `\n` accumulate mid-buffer, this fires loudly instead of
        // silently returning a wrong line boundary.
        debug_assert!(
            !line.iter().take(line.len().saturating_sub(1)).any(|&b| b == b'\n'),
            "line_buffer per-byte fast path: interior newline in flushed line",
        );
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        Some(line)
    }

    /// Feed a run of Data bytes (no ESC — caller guarantees it is a Ground run
    /// up to the next ESC) in bulk. Returns one `(line, trailing_newline)` per
    /// emitted line: `line` is the rule-input with any terminating `\n`
    /// stripped (matching the former per-byte `feed_byte_with_overflow`
    /// contract), and `trailing_newline` tells the caller to re-emit a raw
    /// `\n` after applying rules. A single line exceeding `MAX_BUFFER_BYTES`
    /// is flushed as `(blob, false)` with a warning — byte-identical to feeding
    /// each byte through `feed_byte_with_overflow`. The partial tail stays
    /// buffered across calls. See spec §5 C-2 (`\n`-strip contract) and I-3
    /// (per-line overflow framing).
    ///
    /// Invariant relied on: `inner.len() <= MAX_BUFFER_BYTES` on entry and after
    /// every emit (overflow/newline emits reset to empty; a buffered tail is
    /// strictly shorter than the distance to the cap), so `until_overflow >= 1`.
    // reason: called by the H4a chunk-level feed path (Task 3); not yet wired
    // to pipeline.rs at the Task-2 stage of the plan.
    #[allow(dead_code)]
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
    fn feed_byte_returns_line_on_newline() {
        let mut buf = LineBuffer::new();
        for &b in b"hello" {
            assert!(buf.feed_byte_with_overflow(b).is_none());
        }
        let line = buf.feed_byte_with_overflow(b'\n').expect("line on newline");
        // Mirror feed_with_overflow's convention: newline NOT included in line.
        assert_eq!(line, b"hello");
    }

    #[test]
    fn feed_byte_no_newline_no_line() {
        let mut buf = LineBuffer::new();
        for &b in b"partial" {
            assert!(buf.feed_byte_with_overflow(b).is_none());
        }
    }

    #[test]
    fn feed_byte_two_lines() {
        let mut buf = LineBuffer::new();
        // First line.
        for &b in b"first" {
            assert!(buf.feed_byte_with_overflow(b).is_none());
        }
        let line1 = buf.feed_byte_with_overflow(b'\n').expect("line1");
        assert_eq!(line1, b"first");
        // Second line.
        for &b in b"second" {
            assert!(buf.feed_byte_with_overflow(b).is_none());
        }
        let line2 = buf.feed_byte_with_overflow(b'\n').expect("line2");
        assert_eq!(line2, b"second");
    }

    #[test]
    fn feed_byte_overflow_flushes_without_newline_strip() {
        // Per-byte path: feeding MAX_BUFFER_BYTES+1 non-newline bytes must
        // flush the accumulated buffer (no trailing newline to strip).
        let mut buf = LineBuffer::new();
        let mut flushed: Option<Vec<u8>> = None;
        for _ in 0..=MAX_BUFFER_BYTES {
            if let Some(line) = buf.feed_byte_with_overflow(b'x') {
                flushed = Some(line);
                break;
            }
        }
        let line = flushed.expect("overflow must flush a line via the per-byte path");
        assert_eq!(
            line.len(),
            MAX_BUFFER_BYTES + 1,
            "overflow flush returns all accumulated bytes"
        );
        assert!(line.iter().all(|&b| b == b'x'), "no newline present, nothing stripped");
    }

    #[test]
    fn feed_byte_resumes_after_newline_flush() {
        // After a newline flush, the buffer must be empty and accept a fresh line.
        let mut buf = LineBuffer::new();
        for &b in b"first" {
            assert!(buf.feed_byte_with_overflow(b).is_none());
        }
        assert_eq!(buf.feed_byte_with_overflow(b'\n'), Some(b"first".to_vec()));
        for &b in b"second" {
            assert!(buf.feed_byte_with_overflow(b).is_none());
        }
        assert_eq!(buf.feed_byte_with_overflow(b'\n'), Some(b"second".to_vec()));
    }

    /// Oracle: `feed_data_run` over a slice must emit exactly the `(line,
    /// had_newline)` sequence that feeding the same bytes one-at-a-time through
    /// `feed_byte_with_overflow` (+ the Data arm's `byte == b'\n'` flag) would,
    /// and leave an identical residual partial. This is the byte-identity proof
    /// for the H4a chunk path (spec §5 C-2/I-3).
    fn assert_data_run_matches_per_byte(run: &[u8]) {
        let mut bulk = LineBuffer::new();
        let bulk_lines = bulk.feed_data_run(run);

        let mut per_byte = LineBuffer::new();
        let mut pb_lines: Vec<(Vec<u8>, bool)> = Vec::new();
        for &b in run {
            if let Some(line) = per_byte.feed_byte_with_overflow(b) {
                pb_lines.push((line, b == b'\n'));
            }
        }
        assert_eq!(bulk_lines, pb_lines, "emitted (line,newline) differ for run len {}", run.len());
        assert_eq!(
            bulk.drain(),
            per_byte.drain(),
            "residual partial differs for run len {}",
            run.len()
        );
    }

    #[test]
    fn feed_data_run_matches_per_byte_oracle() {
        assert_data_run_matches_per_byte(b"");
        assert_data_run_matches_per_byte(b"hello world\n");
        assert_data_run_matches_per_byte(b"a\nb\nc\n");
        assert_data_run_matches_per_byte(b"partial without newline");
        assert_data_run_matches_per_byte(b"line1\npartial2");
        assert_data_run_matches_per_byte(b"\n\n\n");
        assert_data_run_matches_per_byte(b"trailing\n\n");
        let max = MAX_BUFFER_BYTES;
        assert_data_run_matches_per_byte(&vec![b'x'; max]); // exactly cap, no newline -> partial
        assert_data_run_matches_per_byte(&vec![b'x'; max + 1]); // cap+1 -> one overflow blob
        assert_data_run_matches_per_byte(&vec![b'x'; 2 * max + 5]); // multiple overflow blobs
        let mut nl_at_overflow = vec![b'x'; max]; // simultaneous overflow + newline
        nl_at_overflow.push(b'\n');
        assert_data_run_matches_per_byte(&nl_at_overflow);
        let mut over_then_nl = vec![b'x'; max + 1];
        over_then_nl.push(b'\n');
        assert_data_run_matches_per_byte(&over_then_nl);
        let mut nl_then_tail = vec![b'x'; max - 1]; // newline as the max-th byte, then more
        nl_then_tail.push(b'\n');
        nl_then_tail.extend_from_slice(b"tail\nmore");
        assert_data_run_matches_per_byte(&nl_then_tail);
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
}
