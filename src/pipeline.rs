//! Output processing pipeline: ANSI-aware byte routing + line buffering +
//! rule application.
//!
//! See spec §3 (ANSI state machine) and §5 (three-path feed mimari).
//!
//! Pipeline owns an [`crate::ansi::AnsiSm`] and routes each byte by the
//! classification event surfaced from the SM:
//!
//! 1. **TUI passthrough** — while any TUI flag is set (alt-screen, bracketed
//!    paste, mouse), bytes go verbatim to stdout. No rule application.
//! 2. **Sequence accumulation** — CSI/ESC bytes accumulate in
//!    `sequence_scratch`; on completion their destination depends on the
//!    sequence kind:
//!      - TUI toggle on/off → stdout (terminal needs the trigger).
//!      - SGR (CSI `m`) → `line_buffer` (sets `line_has_sgr`).
//!      - Other CSI / ESC final / ESC intermediate final → `line_buffer`.
//! 3. **OSC/DCS/PM/APC payload** — payload bytes go direct to stdout
//!    (flushing any pending scratch first); never line-buffered.
//!
//! At line boundary the `respect_existing_colors` flag on the rule snapshot
//! gates rule application: if the line carried any SGR, rules are skipped
//! and the original bytes pass verbatim.

use std::io::Write;
use std::sync::Arc;
use std::time::Instant;

use arc_swap::ArcSwap;

use crate::error::Error;
use crate::line_buffer::{LineBuffer, FLUSH_TIMEOUT};
use crate::rules::Compiled;
use crate::style::Style;

/// Apply the compiled rule set to a single line. Writes the original bytes,
/// with SGR wrappers inserted around the first non-overlapping match of each
/// rule (in rule definition order).
///
/// v0.1 strategy: "first match wins" — overlapping matches from later rules
/// are dropped. Conflict resolution as configurable priority lands in v0.5.
pub(crate) fn apply_rules<W: Write>(
    line: &[u8],
    compiled_handle: &ArcSwap<Compiled>,
    out: &mut W,
) -> std::io::Result<()> {
    // Snapshot the rule set for the duration of this line. Reloads landing
    // mid-line take effect on the NEXT line, never split the current one.
    // The `Arc` clone behind `load_full` is a single AcqRel atomic — cheap.
    let snapshot: Arc<Compiled> = compiled_handle.load_full();
    let compiled: &Compiled = snapshot.as_ref();

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

/// Output pipeline. Owns the ANSI state machine, line buffer, sequence
/// scratch (for accumulating CSI/ESC sequence bytes whose destination is
/// decided at completion), and an `ArcSwap` handle to the rule set.
///
/// See spec §5 for the three-path feed mimari. `apply_rules` snapshots the
/// `ArcSwap` once per line so reloads landing mid-call never split a line.
pub(crate) struct Pipeline {
    sm: crate::ansi::AnsiSm,
    buffer: LineBuffer,
    /// Accumulates CSI/ESC sequence bytes; routed to stdout (TUI toggle)
    /// or `line_buffer` (SGR / other CSI / ESC final) at sequence completion.
    sequence_scratch: Vec<u8>,
    rules: Arc<ArcSwap<Compiled>>,
    /// Set when the current line contained at least one completed SGR
    /// (CSI `m`). Reset on every newline. Drives the
    /// `respect_existing_colors` skip behavior.
    line_has_sgr: bool,
    /// Set when an OSC/DCS/PM/APC string payload appeared mid-line. Forces
    /// the rest of the line to pass verbatim (no rule application). See
    /// the `StringPayloadByte` arm in `feed` for the buffer-drain rationale.
    line_has_string_payload: bool,
}

impl Pipeline {
    pub(crate) fn new(rules: Arc<ArcSwap<Compiled>>) -> Self {
        Pipeline {
            sm: crate::ansi::AnsiSm::new(),
            buffer: LineBuffer::new(),
            sequence_scratch: Vec::with_capacity(64),
            rules,
            line_has_sgr: false,
            line_has_string_payload: false,
        }
    }

    /// Feed a chunk from the PTY master into the pipeline. See spec §5
    /// for the three-path mimari (TUI passthrough / scratch accumulation /
    /// OSC payload direct).
    pub(crate) fn feed<W: Write>(&mut self, chunk: &[u8], out: &mut W) -> std::io::Result<()> {
        for &byte in chunk {
            if self.sm.tui_mode_active() {
                // Path 1: TUI mode active — verbatim passthrough.
                out.write_all(&[byte])?;
                let _ = self.sm.step(byte);
                continue;
            }
            let event = self.sm.step(byte);
            match event {
                crate::ansi::StepEvent::Data => {
                    debug_assert!(self.sequence_scratch.is_empty());
                    if let Some(line) = self.buffer.feed_byte_with_overflow(byte) {
                        self.apply_or_passthrough(&line, out)?;
                        // `feed_byte_with_overflow` strips the trailing `\n`
                        // from newline-terminated lines (see line_buffer.rs);
                        // restore it here so byte-for-byte fidelity holds.
                        // The slice-API path (used for scratch drains below)
                        // keeps `\n` in the line, so it does not need this.
                        if byte == b'\n' {
                            out.write_all(b"\n")?;
                        }
                    }
                }
                crate::ansi::StepEvent::SequenceByte => {
                    self.sequence_scratch.push(byte);
                }
                crate::ansi::StepEvent::StringPayloadByte => {
                    // Path 3: OSC/DCS-passthrough/PM/APC payload byte. To
                    // preserve byte ordering with any pre-OSC content sitting
                    // in the line buffer, drain the buffer's partial line to
                    // stdout *verbatim* first; then flush any pending scratch
                    // (introducer bytes) and write the payload byte direct.
                    //
                    // Decision: a line that contains OSC/DCS/PM/APC cannot be
                    // rule-applied (the pre-OSC portion is already on the
                    // wire). Mark `line_has_string_payload` so the post-OSC
                    // remainder also passes verbatim at `\n`. This keeps
                    // hyperlinks (`\e]8;;URL\aLABEL\e]8;;\a`) byte-intact
                    // and avoids regex inside URL payloads. Spec §4.1.
                    let partial = self.buffer.drain();
                    if !partial.is_empty() {
                        out.write_all(&partial)?;
                    }
                    if !self.sequence_scratch.is_empty() {
                        out.write_all(&self.sequence_scratch)?;
                        self.sequence_scratch.clear();
                    }
                    out.write_all(&[byte])?;
                    self.line_has_string_payload = true;
                }
                crate::ansi::StepEvent::SequenceCompleted(kind) => {
                    self.sequence_scratch.push(byte);
                    self.dispatch_completed_sequence(kind, out)?;
                }
            }
        }
        Ok(())
    }

    /// Route a completed CSI/ESC sequence to its destination per
    /// [`crate::ansi::SequenceKind`]. Consumes `sequence_scratch`.
    fn dispatch_completed_sequence<W: Write>(
        &mut self,
        kind: crate::ansi::SequenceKind,
        out: &mut W,
    ) -> std::io::Result<()> {
        use crate::ansi::SequenceKind;
        match kind {
            SequenceKind::TuiToggleOn | SequenceKind::TuiToggleOff => {
                // Trigger sequence goes verbatim to stdout — terminal needs it.
                out.write_all(&self.sequence_scratch)?;
            }
            SequenceKind::Sgr => {
                let drained = std::mem::take(&mut self.sequence_scratch);
                let (lines, overflow) = self.buffer.feed_with_overflow(&drained);
                if let Some(Error::BufferOverflow { cap }) = overflow {
                    crate::log::warn_msg!("line buffer overflowed; cap={cap}");
                }
                for line in lines {
                    self.apply_or_passthrough(&line, out)?;
                }
                self.line_has_sgr = true;
            }
            SequenceKind::OtherCsi
            | SequenceKind::EscFinal
            | SequenceKind::EscIntermediateFinal => {
                let drained = std::mem::take(&mut self.sequence_scratch);
                let (lines, overflow) = self.buffer.feed_with_overflow(&drained);
                if let Some(Error::BufferOverflow { cap }) = overflow {
                    crate::log::warn_msg!("line buffer overflowed; cap={cap}");
                }
                for line in lines {
                    self.apply_or_passthrough(&line, out)?;
                }
            }
        }
        self.sequence_scratch.clear();
        Ok(())
    }

    /// Apply rules to `line`, OR pass it through verbatim. Rules are skipped
    /// when either:
    /// - The rule snapshot has `respect_existing_colors = true` and the line
    ///   carried any SGR (spec §4.4, Karar 11); or
    /// - The line had an OSC/DCS/PM/APC string payload (the pre-string
    ///   portion was already drained verbatim — re-applying rules to the
    ///   post-string remainder alone would split styling across the line).
    ///
    /// Resets both line flags after handling.
    fn apply_or_passthrough<W: Write>(&mut self, line: &[u8], out: &mut W) -> std::io::Result<()> {
        // Karar 11: snapshot Compiled at line boundary.
        let compiled = self.rules.load_full();
        let skip_rules =
            self.line_has_string_payload || (compiled.respect_existing_colors && self.line_has_sgr);
        if skip_rules {
            out.write_all(line)?;
        } else {
            apply_rules(line, &self.rules, out)?;
        }
        self.line_has_sgr = false;
        self.line_has_string_payload = false;
        Ok(())
    }

    /// Drain any in-flight `sequence_scratch` into the line buffer + emit
    /// completed lines. Called by `tick` (on idle) and `drain` (on shutdown)
    /// to ensure unterminated CSI/ESC bytes do not get stuck forever.
    fn flush_partial<W: Write>(&mut self, out: &mut W) -> std::io::Result<()> {
        if !self.sequence_scratch.is_empty() {
            let drained = std::mem::take(&mut self.sequence_scratch);
            let (lines, _) = self.buffer.feed_with_overflow(&drained);
            for line in lines {
                self.apply_or_passthrough(&line, out)?;
            }
        }
        Ok(())
    }

    /// Flush any pending partial line if it has been idle long enough.
    ///
    /// Called from the poll-driven output thread on every 50ms timeout
    /// (see `runtime::spawn_output_thread`). In TUI passthrough mode the
    /// pipeline holds no buffered content, so this is a no-op.
    pub(crate) fn tick<W: Write>(&mut self, out: &mut W) -> std::io::Result<()> {
        if self.sm.tui_mode_active() {
            return Ok(());
        }
        // `checked_sub` may return None very early in the process lifetime
        // (Instant::now() < FLUSH_TIMEOUT since boot); in that case there is
        // nothing to flush yet anyway.
        let Some(cutoff) = Instant::now().checked_sub(FLUSH_TIMEOUT) else {
            return Ok(());
        };
        self.flush_partial(out)?;
        if let Some(partial) = self.buffer.flush_if_stale(cutoff) {
            self.apply_or_passthrough(&partial, out)?;
        }
        Ok(())
    }

    /// Drain remaining bytes at shutdown.
    pub(crate) fn drain<W: Write>(&mut self, out: &mut W) -> std::io::Result<()> {
        self.flush_partial(out)?;
        let remaining = self.buffer.drain();
        if !remaining.is_empty() {
            self.apply_or_passthrough(&remaining, out)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod rule_tests {
    use super::*;
    use arc_swap::ArcSwap;

    #[test]
    fn ipv4_in_line_gets_sgr_wrapping() {
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut out = Vec::new();
        apply_rules(b"connect to 192.168.1.1 now\n", &rules, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("\x1b["), "expected SGR introducer in: {s:?}");
        assert!(s.contains("192.168.1.1"));
        assert!(s.contains("\x1b[0m"), "expected SGR reset");
    }

    #[test]
    fn no_match_passes_through_unchanged() {
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut out = Vec::new();
        apply_rules(b"plain text line\n", &rules, &mut out).unwrap();
        assert_eq!(out, b"plain text line\n");
    }
}

#[cfg(test)]
mod pipeline_tests {
    use super::*;
    use arc_swap::ArcSwap;
    use std::sync::Arc;

    #[test]
    fn alt_screen_toggle_forwarded_and_content_bypasses_rules() {
        let compiled = Compiled::load_builtins().unwrap();
        let rules = Arc::new(ArcSwap::from_pointee(compiled));
        let mut pipe = Pipeline::new(rules);
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
        let rules = Arc::new(ArcSwap::from_pointee(compiled));
        let mut pipe = Pipeline::new(rules);
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
        let rules = Arc::new(ArcSwap::from_pointee(compiled));
        let mut pipe = Pipeline::new(rules);
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
        let rules = Arc::new(ArcSwap::from_pointee(compiled));
        let mut pipe = Pipeline::new(rules);
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
        let rules = Arc::new(ArcSwap::from_pointee(compiled));
        let mut pipe = Pipeline::new(rules);
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

    #[test]
    fn alt_screen_entry_sequence_not_regexed() {
        // C1 regression guard from spec §5.1. \e[?1049h must reach stdout
        // byte-for-byte, NOT through apply_rules.
        let compiled =
            Compiled::load_with_theme(None, None, None, crate::terminfo::ColorDepth::Truecolor)
                .unwrap();
        let handle = std::sync::Arc::new(arc_swap::ArcSwap::from_pointee(compiled));
        let mut pipeline = Pipeline::new(handle);
        let mut out = Vec::new();
        pipeline.feed(b"\x1b[?1049h", &mut out).unwrap();
        assert_eq!(
            out,
            b"\x1b[?1049h",
            "trigger sequence must reach stdout verbatim; got {:?}",
            String::from_utf8_lossy(&out)
        );
    }

    #[test]
    fn three_path_preserves_byte_ordering_with_osc() {
        let compiled =
            Compiled::load_with_theme(None, None, None, crate::terminfo::ColorDepth::Truecolor)
                .unwrap();
        let handle = std::sync::Arc::new(arc_swap::ArcSwap::from_pointee(compiled));
        let mut pipeline = Pipeline::new(handle);
        let mut out = Vec::new();
        pipeline.feed(b"error \x1b]8;;https://x\x07click\x1b]8;;\x07 tail\n", &mut out).unwrap();
        assert!(
            out.windows(b"\x1b]8;;https://x\x07click\x1b]8;;\x07".len())
                .any(|w| w == b"\x1b]8;;https://x\x07click\x1b]8;;\x07"),
            "OSC sequence must appear in output; got: {:?}",
            String::from_utf8_lossy(&out)
        );
    }

    #[test]
    fn sgr_in_line_with_respect_true_skips_rules() {
        // \e[31mERROR\e[0m 192.168.1.1 — respect_existing_colors=true → verbatim.
        use crate::config::{Config, GeneralSection, UserRule};
        let cfg = Config {
            general: GeneralSection { respect_existing_colors: true, ..GeneralSection::default() },
            rules: Vec::<UserRule>::new(),
        };
        let compiled = Compiled::load_with_theme(
            Some(&cfg),
            Some("/x"),
            None,
            crate::terminfo::ColorDepth::Truecolor,
        )
        .unwrap();
        let handle = std::sync::Arc::new(arc_swap::ArcSwap::from_pointee(compiled));
        let mut pipeline = Pipeline::new(handle);
        let mut out = Vec::new();
        pipeline.feed(b"\x1b[31mERROR\x1b[0m 192.168.1.1\n", &mut out).unwrap();
        // Output should match input byte-for-byte (no tayf SGR injection).
        assert_eq!(out, b"\x1b[31mERROR\x1b[0m 192.168.1.1\n");
    }
}
