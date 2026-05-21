//! Output processing pipeline: TUI mode detection + line buffering +
//! rule application.
//!
//! See spec §3.5 (TUI mode state machine) and §3.6 (rules engine).
//!
//! The state machine never consumes bytes; it only flips internal flag bits
//! tracking which TUI mode (alt-screen, bracketed paste, mouse tracking) is
//! active. When any flag is set, bytes go straight to the writer without
//! line buffering or rule application.

use std::io::Write;
use std::time::Instant;

use crate::error::Error;
use crate::line_buffer::{LineBuffer, FLUSH_TIMEOUT};
use crate::rules::Compiled;
use crate::style::Style;

/// TUI mode bitmask flags. Any non-zero value means the pipeline is in
/// passthrough mode; bytes go straight to stdout without rule application.
mod tui_flags {
    pub const ALT_SCREEN: u32 = 1 << 0;
    pub const BRACKETED_PASTE: u32 = 1 << 1;
    pub const MOUSE: u32 = 1 << 2;
}

/// Map a DEC private mode number to a flag bit, or 0 if it's not a tracked
/// TUI indicator (see spec §3.5).
fn flag_for_mode(num: u32) -> u32 {
    match num {
        47 | 1047 | 1049 => tui_flags::ALT_SCREEN,
        2004 => tui_flags::BRACKETED_PASTE,
        1000 | 1002 | 1003 | 1006 => tui_flags::MOUSE,
        _ => 0,
    }
}

/// 5-state TUI mode parser. Detects DEC private mode set/reset
/// (CSI ? Pm h/l) and toggles bitmask flags. See spec §3.5.
#[derive(Debug)]
pub(crate) struct TuiModeSm {
    state: SmState,
    accum: u32,
    flags: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SmState {
    Ground,
    EscSeen,
    Csi,
    Question,
    Digits,
}

impl TuiModeSm {
    pub(crate) fn new() -> Self {
        TuiModeSm { state: SmState::Ground, accum: 0, flags: 0 }
    }

    /// True iff any TUI mode is currently active.
    pub(crate) fn passthrough(&self) -> bool {
        self.flags != 0
    }

    /// Advance the machine by one byte. Returns the new passthrough state.
    pub(crate) fn step(&mut self, byte: u8) -> bool {
        use SmState::{Csi, Digits, EscSeen, Ground, Question};
        // ESC always restarts a fresh escape sequence regardless of current
        // state — a lone or interrupting `\x1b` resyncs the parser.
        if byte == 0x1b {
            self.state = EscSeen;
            return self.passthrough();
        }
        self.state = match (self.state, byte) {
            (EscSeen, b'[') => Csi,
            (Csi, b'?') => {
                self.accum = 0;
                Question
            }
            (Question | Digits, b) if b.is_ascii_digit() => {
                let digit = u32::from(b - b'0');
                self.accum = if self.state == Question {
                    digit
                } else {
                    self.accum.saturating_mul(10).saturating_add(digit)
                };
                Digits
            }
            (Digits, b'h') => {
                let bit = flag_for_mode(self.accum);
                if bit != 0 {
                    self.flags |= bit;
                }
                Ground
            }
            (Digits, b'l') => {
                let bit = flag_for_mode(self.accum);
                if bit != 0 {
                    self.flags &= !bit;
                }
                Ground
            }
            _ => Ground,
        };
        self.passthrough()
    }
}

/// Apply the compiled rule set to a single line. Writes the original bytes,
/// with SGR wrappers inserted around the first non-overlapping match of each
/// rule (in rule definition order).
///
/// v0.1 strategy: "first match wins" — overlapping matches from later rules
/// are dropped. Conflict resolution as configurable priority lands in v0.5.
pub(crate) fn apply_rules<W: Write>(
    line: &[u8],
    compiled: &Compiled,
    out: &mut W,
) -> std::io::Result<()> {
    // Collect (start, end, style) spans without overlapping.
    let mut spans: Vec<(usize, usize, &Style)> = Vec::new();

    for (i, re) in compiled.individuals.iter().enumerate() {
        for m in re.find_iter(line) {
            let (start, end) = (m.start(), m.end());
            // Reject if it overlaps any existing accepted span.
            if spans.iter().any(|&(s, e, _)| !(end <= s || start >= e)) {
                continue;
            }
            spans.push((start, end, &compiled.styles[i]));
        }
    }

    // Sort spans by start position.
    spans.sort_by_key(|&(s, _, _)| s);

    let mut cursor = 0usize;
    for (start, end, style) in spans {
        out.write_all(&line[cursor..start])?;
        let sgr = style.to_sgr();
        if !sgr.is_empty() {
            out.write_all(sgr.as_bytes())?;
        }
        out.write_all(&line[start..end])?;
        out.write_all(Style::reset_sgr().as_bytes())?;
        cursor = end;
    }
    out.write_all(&line[cursor..])?;
    Ok(())
}

/// Output pipeline. Owns the TUI-mode SM, line buffer, and rule set.
pub(crate) struct Pipeline {
    sm: TuiModeSm,
    buffer: LineBuffer,
    rules: Compiled,
}

impl Pipeline {
    pub(crate) fn new(rules: Compiled) -> Self {
        Pipeline { sm: TuiModeSm::new(), buffer: LineBuffer::new(), rules }
    }

    /// Feed a chunk from the PTY master into the pipeline; emit processed
    /// output to `out`. May produce zero, one, or many writes per call.
    pub(crate) fn feed<W: Write>(&mut self, chunk: &[u8], out: &mut W) -> std::io::Result<()> {
        let mut cursor = 0;
        while cursor < chunk.len() {
            let pass_before = self.sm.passthrough();
            let mut i = cursor;
            while i < chunk.len() {
                self.sm.step(chunk[i]);
                i += 1;
                if self.sm.passthrough() != pass_before {
                    break;
                }
            }
            let segment = &chunk[cursor..i];
            cursor = i;

            let pass_after = self.sm.passthrough();
            let became_passthrough = !pass_before && pass_after;
            if pass_before || became_passthrough {
                // Either we WERE in passthrough for the whole segment, OR we
                // transitioned INTO passthrough mid-segment (segment ends with
                // the trigger sequence). Either way, those bytes are terminal
                // control sequences and must reach the terminal immediately —
                // not via LineBuffer.
                out.write_all(segment)?;
            } else {
                let (lines, overflow) = self.buffer.feed_with_overflow(segment);
                if let Some(Error::BufferOverflow { cap }) = overflow {
                    tracing::warn!(cap, "line buffer overflowed; flushing as-is");
                }
                for line in lines {
                    apply_rules(&line, &self.rules, out)?;
                }
            }
        }
        Ok(())
    }

    /// Flush any pending partial line if it has been idle long enough.
    // reason: spec'd idle-flush hook (§3.4). The v0.1 runtime is a pure
    // blocking-read loop and does not yet poll `tick`; promoting the loop
    // to a `poll(2)`-driven timer in v0.2 will wire it in. Exercised by
    // tests via direct calls.
    #[allow(dead_code)]
    pub(crate) fn tick<W: Write>(&mut self, out: &mut W) -> std::io::Result<()> {
        if self.sm.passthrough() {
            return Ok(());
        }
        // `checked_sub` may return None very early in the process lifetime
        // (Instant::now() < FLUSH_TIMEOUT since boot); in that case there is
        // nothing to flush yet anyway.
        let Some(cutoff) = Instant::now().checked_sub(FLUSH_TIMEOUT) else {
            return Ok(());
        };
        if let Some(partial) = self.buffer.flush_if_stale(cutoff) {
            apply_rules(&partial, &self.rules, out)?;
        }
        Ok(())
    }

    /// Drain remaining bytes at shutdown.
    pub(crate) fn drain<W: Write>(&mut self, out: &mut W) -> std::io::Result<()> {
        let remaining = self.buffer.drain();
        if !remaining.is_empty() {
            apply_rules(&remaining, &self.rules, out)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tui_mode_tests {
    use super::*;

    #[test]
    fn alt_screen_modern_enters_on_1049h() {
        let mut sm = TuiModeSm::new();
        for &b in b"\x1b[?1049h" {
            sm.step(b);
        }
        assert!(sm.passthrough());
    }

    #[test]
    fn alt_screen_exits_on_1049l() {
        let mut sm = TuiModeSm::new();
        for &b in b"\x1b[?1049h\x1b[?1049l" {
            sm.step(b);
        }
        assert!(!sm.passthrough());
    }

    #[test]
    fn accepts_legacy_alt_screen_variants() {
        for seq in [b"\x1b[?47h".as_slice(), b"\x1b[?1047h".as_slice()] {
            let mut sm = TuiModeSm::new();
            for &b in seq {
                sm.step(b);
            }
            assert!(sm.passthrough(), "expected passthrough after {seq:?}");
        }
    }

    #[test]
    fn bracketed_paste_enters_on_2004h() {
        let mut sm = TuiModeSm::new();
        for &b in b"\x1b[?2004h" {
            sm.step(b);
        }
        assert!(sm.passthrough());
    }

    #[test]
    fn mouse_tracking_enters_on_1000h_1002h_1003h_1006h() {
        for code in [b"1000", b"1002", b"1003", b"1006"] {
            let mut sm = TuiModeSm::new();
            let mut seq = b"\x1b[?".to_vec();
            seq.extend_from_slice(code);
            seq.push(b'h');
            for &b in &seq {
                sm.step(b);
            }
            assert!(sm.passthrough(), "expected passthrough after {code:?}h");
        }
    }

    #[test]
    fn multiple_modes_active_simultaneously() {
        let mut sm = TuiModeSm::new();
        for &b in b"\x1b[?2004h\x1b[?1000h" {
            sm.step(b);
        }
        assert!(sm.passthrough());
        for &b in b"\x1b[?2004l" {
            sm.step(b);
        }
        assert!(sm.passthrough(), "mouse mode still on");
        for &b in b"\x1b[?1000l" {
            sm.step(b);
        }
        assert!(!sm.passthrough(), "all modes cleared");
    }

    #[test]
    fn split_across_chunks_still_triggers() {
        let mut sm = TuiModeSm::new();
        for &b in b"\x1b[?104" {
            sm.step(b);
        }
        assert!(!sm.passthrough());
        for &b in b"9h" {
            sm.step(b);
        }
        assert!(sm.passthrough());
    }

    #[test]
    fn other_csi_does_not_trigger() {
        let mut sm = TuiModeSm::new();
        for &b in b"\x1b[31m" {
            sm.step(b);
        }
        assert!(!sm.passthrough());
    }

    #[test]
    fn unknown_dec_private_mode_does_not_trigger() {
        let mut sm = TuiModeSm::new();
        for &b in b"\x1b[?25h" {
            sm.step(b);
        }
        assert!(!sm.passthrough());
    }

    #[test]
    fn lone_esc_does_not_corrupt_state() {
        let mut sm = TuiModeSm::new();
        for &b in b"\x1bA" {
            sm.step(b);
        }
        assert!(!sm.passthrough());
        for &b in b"\x1b[?1049h" {
            sm.step(b);
        }
        assert!(sm.passthrough());
    }
}

#[cfg(test)]
mod rule_tests {
    use super::*;

    #[test]
    fn ipv4_in_line_gets_sgr_wrapping() {
        let compiled = Compiled::load_builtins().unwrap();
        let mut out = Vec::new();
        apply_rules(b"connect to 192.168.1.1 now\n", &compiled, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("\x1b["), "expected SGR introducer in: {s:?}");
        assert!(s.contains("192.168.1.1"));
        assert!(s.contains("\x1b[0m"), "expected SGR reset");
    }

    #[test]
    fn no_match_passes_through_unchanged() {
        let compiled = Compiled::load_builtins().unwrap();
        let mut out = Vec::new();
        apply_rules(b"plain text line\n", &compiled, &mut out).unwrap();
        assert_eq!(out, b"plain text line\n");
    }
}

#[cfg(test)]
mod pipeline_tests {
    use super::*;

    #[test]
    fn alt_screen_toggle_forwarded_and_content_bypasses_rules() {
        let compiled = Compiled::load_builtins().unwrap();
        let mut pipe = Pipeline::new(compiled);
        let mut out = Vec::new();
        pipe.feed(b"\x1b[?1049h", &mut out).unwrap();
        pipe.feed(b"192.168.1.1\n", &mut out).unwrap();

        // Toggle bytes must be forwarded to the terminal.
        let toggle_pos = out.windows(8).position(|w| w == b"\x1b[?1049h");
        let content_pos = out.windows(11).position(|w| w == b"192.168.1.1");
        assert!(toggle_pos.is_some(), "alt-screen toggle missing from out: {out:?}");
        assert!(content_pos.is_some(), "content missing from out: {out:?}");
        assert!(toggle_pos < content_pos, "toggle must precede content");

        // No SGR introducer for the IPv4 rule should appear (passthrough mode).
        // The only \x1b[ in the output should be the toggle itself.
        let esc_count = out.windows(2).filter(|w| w == b"\x1b[").count();
        assert_eq!(esc_count, 1, "exactly one \\x1b[ expected (the toggle): {out:?}");
    }

    #[test]
    fn bracketed_paste_toggle_forwarded_and_bypasses_rules() {
        let compiled = Compiled::load_builtins().unwrap();
        let mut pipe = Pipeline::new(compiled);
        let mut out = Vec::new();
        pipe.feed(b"\x1b[?2004h", &mut out).unwrap();
        pipe.feed(b"claude.md\n", &mut out).unwrap();

        let toggle_pos = out.windows(8).position(|w| w == b"\x1b[?2004h");
        let content_pos = out.windows(9).position(|w| w == b"claude.md");
        assert!(toggle_pos.is_some());
        assert!(content_pos.is_some());
        assert!(toggle_pos < content_pos);

        let esc_count = out.windows(2).filter(|w| w == b"\x1b[").count();
        assert_eq!(esc_count, 1, "exactly the toggle: {out:?}");
    }

    #[test]
    fn mouse_toggle_forwarded_and_bypasses_rules() {
        let compiled = Compiled::load_builtins().unwrap();
        let mut pipe = Pipeline::new(compiled);
        let mut out = Vec::new();
        pipe.feed(b"\x1b[?1000h", &mut out).unwrap();
        pipe.feed(b"server 10.0.0.1 ready\n", &mut out).unwrap();

        let toggle_pos = out.windows(8).position(|w| w == b"\x1b[?1000h");
        let content_pos = out.windows(8).position(|w| w == b"10.0.0.1");
        assert!(toggle_pos.is_some());
        assert!(content_pos.is_some());
        assert!(toggle_pos < content_pos);

        let esc_count = out.windows(2).filter(|w| w == b"\x1b[").count();
        assert_eq!(esc_count, 1, "exactly the toggle: {out:?}");
    }

    #[test]
    fn toggle_and_content_in_single_chunk() {
        let compiled = Compiled::load_builtins().unwrap();
        let mut pipe = Pipeline::new(compiled);
        let mut out = Vec::new();
        // One chunk containing both the alt-screen enter and the content.
        pipe.feed(b"\x1b[?1049hfile.md content\n", &mut out).unwrap();

        let toggle_pos = out.windows(8).position(|w| w == b"\x1b[?1049h");
        let content_pos = out.windows(7).position(|w| w == b"file.md");
        assert!(toggle_pos.is_some(), "toggle must be forwarded: {out:?}");
        assert!(content_pos.is_some(), "content must be forwarded: {out:?}");
        assert!(toggle_pos < content_pos);

        // No filename-rule SGR — we're in passthrough.
        let esc_count = out.windows(2).filter(|w| w == b"\x1b[").count();
        assert_eq!(esc_count, 1);
    }

    #[test]
    fn exit_passthrough_returns_to_rule_application() {
        let compiled = Compiled::load_builtins().unwrap();
        let mut pipe = Pipeline::new(compiled);
        let mut out = Vec::new();
        pipe.feed(b"\x1b[?1049hinside vim\n\x1b[?1049lback to shell file.md\n", &mut out).unwrap();

        // Both toggles forwarded.
        assert!(out.windows(8).any(|w| w == b"\x1b[?1049h"));
        assert!(out.windows(8).any(|w| w == b"\x1b[?1049l"));
        // After exiting passthrough, "file.md" must be colorized — at least one SGR
        // wrapping it should appear after the exit toggle.
        let exit_pos = out.windows(8).position(|w| w == b"\x1b[?1049l").unwrap();
        let post_exit = &out[exit_pos + 8..];
        // Look for an SGR introducer in the post-exit slice.
        assert!(
            post_exit.windows(2).any(|w| w == b"\x1b["),
            "post-exit content should be rule-wrapped: {post_exit:?}"
        );
    }
}
