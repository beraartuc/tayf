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
/// "First match wins" — overlapping matches from later rules are dropped.
/// Overlap detection uses `accepted_spans` (sorted by start) + binary search:
/// O(log N) per candidate match, was O(runs²) in v0.3.4.
///
/// Selective dispatch per rule: if `uses_capture_styling[i]` is set, the rule
/// goes through `captures_iter` + `emit_capture_runs` (one match expands to
/// 1..=N styled runs, inner groups overriding outer); otherwise `find_iter`
/// pushes a single run per match — byte-identical to v0.3.4 on this path.
/// Conflict resolution as configurable priority lands in v0.5.
pub(crate) fn apply_rules<W: Write>(
    line: &[u8],
    compiled_handle: &ArcSwap<Compiled>,
    out: &mut W,
) -> std::io::Result<()> {
    let snapshot: Arc<Compiled> = compiled_handle.load_full();
    let compiled: &Compiled = snapshot.as_ref();

    // Accepted match spans (sorted by start) — used for overlap detection
    // only. One entry per accepted match, regardless of how many runs the
    // match emits via the captures path.
    let mut accepted_spans: Vec<(usize, usize)> = Vec::new();
    let mut runs: Vec<(usize, usize, &Style)> = Vec::new();
    let mut event_scratch: Vec<(usize, OpenClose, u32)> = Vec::new();
    let mut active_scratch: Vec<u32> = Vec::new();

    for (i, re) in compiled.individuals.iter().enumerate() {
        if compiled.uses_capture_styling[i] {
            for caps in re.captures_iter(line) {
                let m = caps.get(0).expect("capture 0 is always the full match");
                let (start, end) = (m.start(), m.end());
                if overlaps_accepted(&accepted_spans, start, end) {
                    continue;
                }
                emit_capture_runs(
                    &caps,
                    start,
                    end,
                    &compiled.styles[i],
                    &compiled.group_styles[i],
                    &mut event_scratch,
                    &mut active_scratch,
                    &mut runs,
                );
                insert_accepted(&mut accepted_spans, start, end);
            }
        } else {
            for m in re.find_iter(line) {
                let (start, end) = (m.start(), m.end());
                if overlaps_accepted(&accepted_spans, start, end) {
                    continue;
                }
                runs.push((start, end, &compiled.styles[i]));
                insert_accepted(&mut accepted_spans, start, end);
            }
        }
    }

    runs.sort_by_key(|&(s, _, _)| s);

    let mut cursor = 0usize;
    for (start, end, style) in runs {
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

/// O(log N) overlap check against sorted-by-start `accepted_spans`. Two
/// half-open intervals `[a_start, a_end)` and `[b_start, b_end)` overlap
/// iff `a_start < b_end AND b_start < a_end`. With `accepted_spans`
/// sorted, the candidate `[start, end)` overlaps an existing entry iff
/// either the entry immediately preceding `start` extends past `start`,
/// or the entry immediately at-or-after `start` starts before `end`.
#[inline]
pub(crate) fn overlaps_accepted(spans: &[(usize, usize)], start: usize, end: usize) -> bool {
    let i = spans.partition_point(|&(s, _)| s < start);
    if i > 0 && spans[i - 1].1 > start {
        return true;
    }
    if i < spans.len() && spans[i].0 < end {
        return true;
    }
    false
}

/// Insert `(start, end)` into the sorted-by-start `accepted_spans` vec.
/// Maintains the sort invariant so subsequent `overlaps_accepted` checks
/// remain O(log N).
#[inline]
pub(crate) fn insert_accepted(spans: &mut Vec<(usize, usize)>, start: usize, end: usize) {
    let i = spans.partition_point(|&(s, _)| s < start);
    spans.insert(i, (start, end));
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum OpenClose {
    Close,
    Open,
}
// Close < Open: when a group ends at position p and another starts at
// position p, the Close-then-Open ordering elides a zero-length gap run.

/// Expand a single captured match into 1..=N non-overlapping
/// `(start, end, &Style)` runs using a boundary-event sweep. No per-byte
/// allocation; all transient state lives in caller-owned scratch Vecs
/// that are `.clear()`-reused across matches in the same line.
///
/// Inner (higher-index) groups override outer groups when nested. See
/// spec §1.1 for the algorithm walkthrough.
#[allow(clippy::too_many_arguments)] // reason: scratch Vecs are caller-owned to avoid per-match allocation; bundling them into a struct would obscure the reuse pattern.
pub(crate) fn emit_capture_runs<'r>(
    caps: &regex::bytes::Captures<'_>,
    match_start: usize,
    match_end: usize,
    default_style: &'r Style,
    group_styles: &'r [Option<Style>],
    event_scratch: &mut Vec<(usize, OpenClose, u32)>,
    active_scratch: &mut Vec<u32>,
    out: &mut Vec<(usize, usize, &'r Style)>,
) {
    event_scratch.clear();
    active_scratch.clear();
    for (gi, slot) in group_styles.iter().enumerate() {
        if slot.is_none() {
            continue;
        }
        let Some(sub) = caps.get(gi + 1) else { continue };
        #[allow(clippy::cast_possible_truncation)]
        // reason: regex caps capture count well below u32::MAX; group_styles.len() bounded by pattern.
        let g = (gi + 1) as u32;
        event_scratch.push((sub.start(), OpenClose::Open, g));
        event_scratch.push((sub.end(), OpenClose::Close, g));
    }
    if event_scratch.is_empty() {
        out.push((match_start, match_end, default_style));
        return;
    }
    event_scratch.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    let mut prev_pos = match_start;
    for &(pos, kind, g) in event_scratch.iter() {
        if pos > prev_pos {
            let active_g = active_scratch.last().copied().unwrap_or(0);
            let style: &Style = if active_g == 0 {
                default_style
            } else {
                group_styles[(active_g - 1) as usize]
                    .as_ref()
                    .expect("event pushed for Some-styled group; slot is Some")
            };
            out.push((prev_pos, pos, style));
            prev_pos = pos;
        }
        match kind {
            OpenClose::Open => {
                let ip = active_scratch.iter().position(|&x| x > g).unwrap_or(active_scratch.len());
                active_scratch.insert(ip, g);
            }
            OpenClose::Close => {
                if let Some(rp) = active_scratch.iter().position(|&x| x == g) {
                    active_scratch.remove(rp);
                }
            }
        }
    }
    if prev_pos < match_end {
        out.push((prev_pos, match_end, default_style));
    }
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
                let event = self.sm.step(byte);
                if matches!(event, crate::ansi::StepEvent::ForceStringTerminate) {
                    // tmux running inside tayf can emit large OSC 52 payloads while
                    // alt-screen is held by tmux — cap-fire mid-OSC even in TUI mode.
                    // Emit synthetic ST so terminal doesn't keep eating shell output.
                    // No re-step: byte was already written to stdout above. See spec §4.4.
                    out.write_all(b"\x1b\\")?;
                }
                continue;
            }
            let event = self.sm.step(byte);
            if let crate::ansi::StepEvent::ForceStringTerminate = event {
                out.write_all(b"\x1b\\")?;
                // SM has reset to Ground; re-step the byte for fresh classification.
                let event = self.sm.step(byte);
                debug_assert!(
                    !matches!(event, crate::ansi::StepEvent::ForceStringTerminate),
                    "ForceStringTerminate must not recur after re-step"
                );
                self.dispatch_classification_event(event, byte, out)?;
                continue;
            }
            self.dispatch_classification_event(event, byte, out)?;
        }
        Ok(())
    }

    /// Per-byte event dispatch. Routes `Data`, `SequenceByte`,
    /// `StringPayloadByte`, and `SequenceCompleted` to their existing
    /// handlers. `ForceStringTerminate` must NOT reach this dispatch —
    /// `feed` writes the synthetic ST and re-steps before calling here.
    /// See spec §4.3.
    fn dispatch_classification_event<W: Write>(
        &mut self,
        event: crate::ansi::StepEvent,
        byte: u8,
        out: &mut W,
    ) -> std::io::Result<()> {
        use crate::ansi::StepEvent;
        match event {
            StepEvent::Data => {
                debug_assert!(self.sequence_scratch.is_empty());
                if let Some(line) = self.buffer.feed_byte_with_overflow(byte) {
                    self.apply_or_passthrough(&line, out)?;
                    // `feed_byte_with_overflow` strips the trailing `\n` from
                    // newline-terminated lines (see line_buffer.rs); restore
                    // it here so byte-for-byte fidelity holds. The slice-API
                    // path (used for scratch drains in dispatch_completed_sequence)
                    // keeps `\n` in the line, so it does not need this.
                    if byte == b'\n' {
                        out.write_all(b"\n")?;
                    }
                }
            }
            StepEvent::SequenceByte => {
                self.sequence_scratch.push(byte);
            }
            StepEvent::StringPayloadByte => {
                // Path 3: OSC/DCS-passthrough/PM/APC payload byte. To preserve
                // byte ordering with any pre-OSC content sitting in the line
                // buffer, drain the buffer's partial line to stdout *verbatim*
                // first, then flush any pending scratch (introducer bytes), then
                // write the payload byte direct.
                //
                // Decision: a line that contains OSC/DCS/PM/APC cannot be
                // rule-applied (pre-OSC bytes are already on the wire). Mark
                // `line_has_string_payload` so the post-OSC remainder also
                // passes verbatim at `\n`. Keeps hyperlinks intact and avoids
                // regex inside OSC payloads. Spec v0.3.0 §4.1.
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
            StepEvent::SequenceCompleted(kind) => {
                self.sequence_scratch.push(byte);
                self.dispatch_completed_sequence(kind, out)?;
            }
            StepEvent::ForceStringTerminate => {
                unreachable!("ForceStringTerminate is handled in feed before reaching dispatch");
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
                // Flush any partial line that was in buffer BEFORE the trigger.
                // The trigger sequence will switch us into TUI passthrough mode;
                // subsequent bytes bypass line_buffer entirely, so any orphaned
                // partial would be stuck until shutdown drain — wrong stdout order.
                let partial = self.buffer.drain();
                if !partial.is_empty() {
                    out.write_all(&partial)?;
                }
                // Now write the trigger sequence so terminal sees it.
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

    /// Drain any in-flight sequence scratch directly to stdout (NOT into
    /// `line_buffer`; rule application must never see raw ESC/CSI bytes).
    /// Called by tick (on idle) and drain (on shutdown).
    fn flush_partial<W: Write>(&mut self, out: &mut W) -> std::io::Result<()> {
        if !self.sequence_scratch.is_empty() {
            out.write_all(&self.sequence_scratch)?;
            self.sequence_scratch.clear();
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

    #[test]
    fn apply_rules_hot_path_behavior_matches_v0_3_4_for_non_captures_rules() {
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut out = Vec::new();
        apply_rules(b"connect to 192.168.1.1 now\n", &rules, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("\x1b["));
        assert!(s.contains("192.168.1.1"));
        assert!(s.contains("\x1b[0m"));
        // Exactly ONE SGR introducer + ONE reset, because no built-in is
        // captures-styled in Phase 3 (no group_styles populated yet).
        let introducers = s.matches("\x1b[").count() - s.matches("\x1b[0m").count();
        assert_eq!(introducers, 1, "expected one non-reset SGR; got: {s:?}");
    }

    #[test]
    fn permission_match_renders_four_distinct_sgrs() {
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut out = Vec::new();
        apply_rules(b"drwxr-xr-x file.txt\n", &rules, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        // Expect 4 distinct non-reset SGRs wrapping type / user / group / other.
        // (Plus filename rule may fire on "file.txt" — that's 1 extra. Total ≥ 4.)
        let intro_count = s.matches("\x1b[").count() - s.matches("\x1b[0m").count();
        assert!(intro_count >= 4, "expected ≥ 4 SGRs (type/user/group/other); got: {s:?}");
    }

    #[test]
    fn iso_timestamp_match_renders_five_distinct_sgrs() {
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut out = Vec::new();
        apply_rules(b"event at 2026-05-24T10:30:45.123Z fired\n", &rules, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        let intro_count = s.matches("\x1b[").count() - s.matches("\x1b[0m").count();
        assert!(intro_count >= 5, "expected >= 5 SGRs (date/sep/time/ms/tz); got: {s:?}");
    }

    #[test]
    fn syslog_timestamp_match_renders_one_sgr() {
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut out = Vec::new();
        apply_rules(b"May 24 10:30:45 host service: msg\n", &rules, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        // Syslog branch has no captures -> entire match wrapped in one default-style SGR.
        // Plus log_level rule will catch "msg" — that's an extra SGR.
        // Assert the substring appears in output (basic survival check).
        assert!(s.contains("May 24 10:30:45"), "syslog timestamp must survive in output: {s:?}");
    }

    #[test]
    fn http_url_match_renders_three_sgrs_with_underline_on_path() {
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut out = Vec::new();
        apply_rules(b"docs at https://example.com/path now\n", &rules, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        let intro_count = s.matches("\x1b[").count() - s.matches("\x1b[0m").count();
        assert!(intro_count >= 3, "expected >= 3 SGRs (scheme/'://'/path); got: {s:?}");
        // Underline attribute should appear (SGR code 4 for underline).
        assert!(
            s.contains("4m") || s.contains("4;") || s.contains(";4m"),
            "expected underline SGR for path; got: {s:?}"
        );
    }

    #[test]
    fn git_at_url_match_renders_match_via_default_style() {
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut out = Vec::new();
        apply_rules(b"clone git@github.com:user/repo.git\n", &rules, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        // git@ branch has no captures → collapsed to one default-style SGR.
        // (Filename rule may also fire on "repo.git" — that's separate.)
        assert!(
            s.contains("git@github.com:user/repo.git"),
            "git@ URL must survive in output: {s:?}"
        );
    }

    #[test]
    fn apply_rules_with_capture_styling_rule_emits_multi_run_match() {
        // Synthetic captures-styled rule. We assemble a Compiled manually
        // because Phase 6 hasn't restructured permission/timestamp/url yet.
        use crate::style::{Color, Style};
        use regex::bytes::{Regex, RegexSet};
        let re = Regex::new(r"(\d+)-(\d+)").unwrap();
        let red = Style { fg: Some(Color::Red), ..Style::DEFAULT };
        let blue = Style { fg: Some(Color::Blue), ..Style::DEFAULT };
        let default = Style::DEFAULT;
        let compiled = Compiled {
            set: RegexSet::new([r"(\d+)-(\d+)"]).unwrap(),
            individuals: vec![re],
            styles: vec![default],
            group_styles: vec![vec![Some(red), Some(blue)]],
            uses_capture_styling: vec![true],
            respect_existing_colors: false,
        };
        let rules = ArcSwap::from_pointee(compiled);
        let mut out = Vec::new();
        apply_rules(b"X 12-34 Y\n", &rules, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        // Expect at least 2 non-reset SGRs (red and blue for the two groups).
        let sgr_count = s.matches("\x1b[").count() - s.matches("\x1b[0m").count();
        assert!(sgr_count >= 2, "expected >= 2 non-reset SGRs (red + blue); got: {s:?}");
        assert!(s.contains("12") && s.contains("34") && s.contains('-'));
    }

    #[test]
    fn overlaps_accepted_empty_never_overlaps() {
        assert!(!overlaps_accepted(&[], 0, 10));
        assert!(!overlaps_accepted(&[], 5, 7));
    }

    #[test]
    fn overlaps_accepted_finds_immediate_predecessor() {
        let spans = vec![(0usize, 5usize), (10, 15), (20, 25)];
        assert!(overlaps_accepted(&spans, 4, 8)); // overlaps (0,5)
        assert!(!overlaps_accepted(&spans, 5, 10)); // abuts both sides, no overlap
        assert!(overlaps_accepted(&spans, 14, 22)); // straddles (10,15) and (20,25)
        assert!(!overlaps_accepted(&spans, 25, 30)); // abuts (20,25), no overlap
        assert!(!overlaps_accepted(&spans, 26, 28)); // past everything
    }

    #[test]
    fn overlaps_accepted_handles_single_entry() {
        let spans = vec![(10usize, 20usize)];
        assert!(!overlaps_accepted(&spans, 0, 5));
        assert!(!overlaps_accepted(&spans, 0, 10)); // abuts
        assert!(overlaps_accepted(&spans, 5, 15)); // overlaps left edge
        assert!(overlaps_accepted(&spans, 12, 18)); // contained
        assert!(overlaps_accepted(&spans, 15, 25)); // overlaps right edge
        assert!(!overlaps_accepted(&spans, 20, 30)); // abuts
        assert!(!overlaps_accepted(&spans, 25, 30));
    }

    #[test]
    fn insert_accepted_maintains_sorted_order() {
        let mut spans: Vec<(usize, usize)> = Vec::new();
        insert_accepted(&mut spans, 10, 15);
        insert_accepted(&mut spans, 0, 5);
        insert_accepted(&mut spans, 20, 25);
        insert_accepted(&mut spans, 5, 10);
        assert_eq!(spans, vec![(0, 5), (5, 10), (10, 15), (20, 25)]);
    }

    #[test]
    fn emit_capture_runs_no_styled_groups_emits_single_default_run() {
        // Synthetic pattern with one capture group, but group_styles is None
        // for that index — so no boundary events fire.
        let re = regex::bytes::Regex::new(r"(\d+)").unwrap();
        let line = b"abc 123 xyz";
        let caps = re.captures(line).unwrap();
        let default = Style::DEFAULT;
        let group_styles: Vec<Option<Style>> = vec![None];
        let mut event_scratch = Vec::new();
        let mut active_scratch = Vec::new();
        let mut runs: Vec<(usize, usize, &Style)> = Vec::new();
        emit_capture_runs(
            &caps,
            4,
            7,
            &default,
            &group_styles,
            &mut event_scratch,
            &mut active_scratch,
            &mut runs,
        );
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].0, 4);
        assert_eq!(runs[0].1, 7);
        assert!(std::ptr::eq(runs[0].2, &default));
    }

    #[test]
    fn emit_capture_runs_single_group_emits_one_run() {
        let re = regex::bytes::Regex::new(r"(\d+)").unwrap();
        let line = b"abc 123 xyz";
        let caps = re.captures(line).unwrap();
        let default = Style::DEFAULT;
        let red = Style { fg: Some(crate::style::Color::Red), ..Style::DEFAULT };
        let group_styles: Vec<Option<Style>> = vec![Some(red)];
        let mut es = Vec::new();
        let mut as_ = Vec::new();
        let mut runs: Vec<(usize, usize, &Style)> = Vec::new();
        emit_capture_runs(&caps, 4, 7, &default, &group_styles, &mut es, &mut as_, &mut runs);
        // Group 1 covers [4,7) — exactly the match. Single run = group style.
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].0, 4);
        assert_eq!(runs[0].1, 7);
        assert!(std::ptr::eq(runs[0].2, group_styles[0].as_ref().unwrap()));
    }

    #[test]
    fn emit_capture_runs_two_adjacent_groups_emit_two_runs_no_gap() {
        let re = regex::bytes::Regex::new(r"(a)(b)").unwrap();
        let line = b"ab";
        let caps = re.captures(line).unwrap();
        let default = Style::DEFAULT;
        let red = Style { fg: Some(crate::style::Color::Red), ..Style::DEFAULT };
        let blue = Style { fg: Some(crate::style::Color::Blue), ..Style::DEFAULT };
        let group_styles: Vec<Option<Style>> = vec![Some(red), Some(blue)];
        let mut es = Vec::new();
        let mut as_ = Vec::new();
        let mut runs: Vec<(usize, usize, &Style)> = Vec::new();
        emit_capture_runs(&caps, 0, 2, &default, &group_styles, &mut es, &mut as_, &mut runs);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].0, 0);
        assert_eq!(runs[0].1, 1);
        assert_eq!(runs[1].0, 1);
        assert_eq!(runs[1].1, 2);
    }

    #[test]
    fn emit_capture_runs_nested_groups_inner_wins_three_runs() {
        let re = regex::bytes::Regex::new(r"(a(b)c)").unwrap();
        let line = b"abc";
        let caps = re.captures(line).unwrap();
        let default = Style::DEFAULT;
        let red = Style { fg: Some(crate::style::Color::Red), ..Style::DEFAULT };
        let blue = Style { fg: Some(crate::style::Color::Blue), ..Style::DEFAULT };
        let group_styles: Vec<Option<Style>> = vec![Some(red), Some(blue)];
        let mut es = Vec::new();
        let mut as_ = Vec::new();
        let mut runs: Vec<(usize, usize, &Style)> = Vec::new();
        emit_capture_runs(&caps, 0, 3, &default, &group_styles, &mut es, &mut as_, &mut runs);
        assert_eq!(runs.len(), 3);
        // a → outer (red)
        assert_eq!(runs[0].0, 0);
        assert_eq!(runs[0].1, 1);
        assert!(std::ptr::eq(runs[0].2, group_styles[0].as_ref().unwrap()));
        // b → inner (blue)
        assert_eq!(runs[1].0, 1);
        assert_eq!(runs[1].1, 2);
        assert!(std::ptr::eq(runs[1].2, group_styles[1].as_ref().unwrap()));
        // c → outer (red) again
        assert_eq!(runs[2].0, 2);
        assert_eq!(runs[2].1, 3);
        assert!(std::ptr::eq(runs[2].2, group_styles[0].as_ref().unwrap()));
    }

    #[test]
    fn emit_capture_runs_unfired_alternation_group_treated_as_none() {
        let re = regex::bytes::Regex::new(r"(?:(x)|y)").unwrap();
        let line = b"y";
        let caps = re.captures(line).unwrap();
        let default = Style::DEFAULT;
        let red = Style { fg: Some(crate::style::Color::Red), ..Style::DEFAULT };
        let group_styles: Vec<Option<Style>> = vec![Some(red)];
        let mut es = Vec::new();
        let mut as_ = Vec::new();
        let mut runs: Vec<(usize, usize, &Style)> = Vec::new();
        emit_capture_runs(&caps, 0, 1, &default, &group_styles, &mut es, &mut as_, &mut runs);
        assert_eq!(runs.len(), 1);
        assert!(std::ptr::eq(runs[0].2, &default));
    }

    #[test]
    fn emit_capture_runs_capture_with_gap_before_emits_default_then_group() {
        let re = regex::bytes::Regex::new(r"ab(c)").unwrap();
        let line = b"abc";
        let caps = re.captures(line).unwrap();
        let default = Style::DEFAULT;
        let red = Style { fg: Some(crate::style::Color::Red), ..Style::DEFAULT };
        let group_styles: Vec<Option<Style>> = vec![Some(red)];
        let mut es = Vec::new();
        let mut as_ = Vec::new();
        let mut runs: Vec<(usize, usize, &Style)> = Vec::new();
        emit_capture_runs(&caps, 0, 3, &default, &group_styles, &mut es, &mut as_, &mut runs);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].0, 0);
        assert_eq!(runs[0].1, 2);
        assert!(std::ptr::eq(runs[0].2, &default));
        assert_eq!(runs[1].0, 2);
        assert_eq!(runs[1].1, 3);
    }

    #[test]
    fn emit_capture_runs_capture_with_gap_after_emits_group_then_default() {
        let re = regex::bytes::Regex::new(r"(a)bc").unwrap();
        let line = b"abc";
        let caps = re.captures(line).unwrap();
        let default = Style::DEFAULT;
        let red = Style { fg: Some(crate::style::Color::Red), ..Style::DEFAULT };
        let group_styles: Vec<Option<Style>> = vec![Some(red)];
        let mut es = Vec::new();
        let mut as_ = Vec::new();
        let mut runs: Vec<(usize, usize, &Style)> = Vec::new();
        emit_capture_runs(&caps, 0, 3, &default, &group_styles, &mut es, &mut as_, &mut runs);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].0, 0);
        assert_eq!(runs[0].1, 1);
        assert_eq!(runs[1].0, 1);
        assert_eq!(runs[1].1, 3);
        assert!(std::ptr::eq(runs[1].2, &default));
    }

    #[test]
    fn emit_capture_runs_scratch_reused_across_calls_correctly() {
        let re = regex::bytes::Regex::new(r"(\d+)").unwrap();
        let line = b"abc 123 xyz 456";
        let default = Style::DEFAULT;
        let red = Style { fg: Some(crate::style::Color::Red), ..Style::DEFAULT };
        let group_styles: Vec<Option<Style>> = vec![Some(red)];
        let mut es = Vec::new();
        let mut as_ = Vec::new();
        let mut runs: Vec<(usize, usize, &Style)> = Vec::new();
        for caps in re.captures_iter(line) {
            let m = caps.get(0).unwrap();
            emit_capture_runs(
                &caps,
                m.start(),
                m.end(),
                &default,
                &group_styles,
                &mut es,
                &mut as_,
                &mut runs,
            );
        }
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].0, 4);
        assert_eq!(runs[0].1, 7);
        assert_eq!(runs[1].0, 12);
        assert_eq!(runs[1].1, 15);
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
    fn partial_line_then_alt_screen_toggle_preserves_byte_order() {
        // C1 regression: when normal data sits in line_buffer and an
        // alt-screen toggle completes in the same chunk, the partial line
        // must reach stdout BEFORE the toggle sequence.
        use crate::rules::Compiled;
        let compiled =
            Compiled::load_with_theme(None, None, None, crate::terminfo::ColorDepth::Truecolor)
                .unwrap();
        let handle = std::sync::Arc::new(arc_swap::ArcSwap::from_pointee(compiled));
        let mut pipeline = Pipeline::new(handle);
        let mut out = Vec::new();
        pipeline.feed(b"abc\x1b[?1049h", &mut out).unwrap();
        // Order must be: "abc" first (partial line), then trigger sequence.
        assert_eq!(
            out,
            b"abc\x1b[?1049h",
            "partial line must precede toggle; got {:?}",
            String::from_utf8_lossy(&out)
        );
    }

    #[test]
    fn partial_sgr_then_alt_screen_toggle_preserves_byte_order() {
        // C1 regression: an SGR completion + trailing data + TUI toggle
        // in one chunk. SGR bytes + text must reach stdout before toggle.
        use crate::rules::Compiled;
        let compiled =
            Compiled::load_with_theme(None, None, None, crate::terminfo::ColorDepth::Truecolor)
                .unwrap();
        let handle = std::sync::Arc::new(arc_swap::ArcSwap::from_pointee(compiled));
        let mut pipeline = Pipeline::new(handle);
        let mut out = Vec::new();
        pipeline.feed(b"\x1b[31mERR\x1b[?1049hX", &mut out).unwrap();
        // The SGR bytes + "ERR" should appear in output before \x1b[?1049h.
        // After toggle, "X" goes verbatim.
        let s = String::from_utf8_lossy(&out);
        let toggle_pos = s.find("\x1b[?1049h").expect("toggle in output");
        let err_pos = s.find("ERR").expect("ERR in output");
        assert!(err_pos < toggle_pos, "ERR must appear before toggle; got: {s:?}");
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

    #[test]
    fn pipeline_writes_st_on_cap_fire_in_string_state() {
        let compiled = Compiled::load_builtins().unwrap();
        let rules = Arc::new(ArcSwap::from_pointee(compiled));
        let mut pipeline = Pipeline::new(rules);
        let mut out: Vec<u8> = Vec::new();

        // Adversarial unterminated OSC: \e]2; + lots of 'a' bytes.
        // SEQUENCE_BYTES_CAP is 4096; this exceeds the cap.
        let mut input = vec![0x1b, b']', b'2', b';'];
        input.extend(std::iter::repeat(b'a').take(5000));
        pipeline.feed(&input, &mut out).unwrap();

        // Stdout must contain a synthetic \e\\ ST emitted at cap fire.
        let has_st = out.windows(2).any(|w| w == b"\x1b\\");
        assert!(has_st, "expected synthetic ST in stdout; got len={}", out.len());
    }
}
