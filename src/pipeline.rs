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

/// Single styled byte range produced by [`apply_rules_spans`].
///
/// `start..end` are byte offsets into the input line (half-open; `end`
/// exclusive). Style is the resolved style for the run — capture-group
/// merging (inner overrides outer) is already applied upstream.
///
/// Non-overlapping invariant: in the returned `Vec<StyleSpan>` from
/// [`apply_rules_spans`], no two spans overlap. Sorted by `start` ASC.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct StyleSpan {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) style: Style,
}

/// Apply the compiled rule set to a single line. Writes the original bytes,
/// with SGR wrappers inserted around accepted matches.
///
/// **Acceptance contract (v0.5.6):**
/// Rules iterate in `(Reverse(priority), rule_index)` order — highest
/// priority first, ties broken by pattern-definition (lower index) order.
/// Each match is accepted unless its span overlaps an already-accepted span
/// (bidirectional check: [`overlaps_accepted`] rejects both nested and
/// enveloping overlaps). This lets profile envelope rules (priority 200)
/// claim envelope spans before interior built-ins (priority 0) can take
/// substrings within them. See [`Compiled::priorities`] tier convention.
///
/// Overlap detection uses `accepted_spans` (sorted by start) + binary search:
/// O(log N) per candidate match, was O(runs²) in v0.3.4.
///
/// Selective dispatch per rule: if `uses_capture_styling[i]` is set, the rule
/// goes through `captures_iter` + `emit_capture_runs` (one match expands to
/// 1..=N styled runs, inner groups overriding outer); otherwise `find_iter`
/// pushes a single run per match.
pub(crate) fn apply_rules<W: Write>(
    line: &[u8],
    compiled_handle: &ArcSwap<Compiled>,
    scratch: &mut PipelineScratch,
    out: &mut W,
) -> std::io::Result<()> {
    let snapshot: Arc<Compiled> = compiled_handle.load_full();
    let runs = select_runs(line, snapshot.as_ref(), scratch);

    let mut cursor = 0usize;
    for &(start, end, style) in runs {
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

/// Collect matched, overlap-rejected, priority-sorted style runs for one
/// line into `scratch.runs`. Steps 1-4 of [`apply_rules`]
/// (pre-filter → priority sort → captures/find iter → overlap reject →
/// final start-sort).
///
/// Returns a borrow into `scratch.runs` valid until the next mutation of
/// `scratch`. The borrow holds `&mut` exclusivity over `scratch`; the
/// caller cannot touch `scratch` until the borrow is dropped. For
/// dual-emit (byte + span) on the same line, materialize spans
/// immediately (e.g., `.iter().copied().collect()`) so the borrow ends.
pub(crate) fn select_runs<'a>(
    line: &[u8],
    compiled: &Compiled,
    scratch: &'a mut PipelineScratch,
) -> &'a [(usize, usize, Style)] {
    scratch.accepted_spans.clear();
    scratch.runs.clear();
    scratch.event_scratch.clear();
    scratch.active_scratch.clear();
    scratch.set_match_scratch.clear();

    // Pre-filter: ask RegexSet which rule indices CAN hit; skip the rest.
    // `SetMatches::iter()` yields indices in pattern-definition order
    // (regex 1.12 stable contract). NO HashSet/BTreeSet here — the
    // priority sort step below depends on a deterministic input ordering
    // for its stable tie-break (v0.5.5 pattern-order behavior is preserved
    // when all priorities are equal, e.g. all built-ins at 0).
    scratch.set_match_scratch.extend(compiled.set.matches(line).iter());

    // v0.5.6 §4.3 — priority sort: iterate (priority DESC, rule_index ASC).
    // MUST be `sort_by` (stable) — preserves pattern-definition order
    // tie-break for priority 0 == priority 0 case. `sort_unstable_by`
    // would silently regress `apply_rules_preserves_pattern_definition_order_*`.
    {
        use std::cmp::Reverse;
        let priorities = &compiled.priorities;
        scratch.set_match_scratch.sort_by(|&a, &b| {
            Reverse(priorities[a]).cmp(&Reverse(priorities[b])).then_with(|| a.cmp(&b))
        });
    }

    for &i in &scratch.set_match_scratch {
        let re = &compiled.individuals[i];
        if compiled.uses_capture_styling[i] {
            for caps in re.captures_iter(line) {
                let m = caps.get(0).expect("capture 0 is always the full match");
                let (start, end) = (m.start(), m.end());
                if overlaps_accepted(&scratch.accepted_spans, start, end) {
                    continue;
                }
                emit_capture_runs(
                    &caps,
                    start,
                    end,
                    compiled.styles[i],
                    &compiled.group_styles[i],
                    &mut scratch.event_scratch,
                    &mut scratch.active_scratch,
                    &mut scratch.runs,
                );
                insert_accepted(&mut scratch.accepted_spans, start, end);
            }
        } else {
            for m in re.find_iter(line) {
                let (start, end) = (m.start(), m.end());
                if overlaps_accepted(&scratch.accepted_spans, start, end) {
                    continue;
                }
                scratch.runs.push((start, end, compiled.styles[i]));
                insert_accepted(&mut scratch.accepted_spans, start, end);
            }
        }
    }

    // Cross-rule interleaving: a later rule's match can land before an
    // earlier rule's match in start order (e.g. rule N matches at byte 5
    // after rule 1 matched at byte 30). Sort once at the end to keep the
    // emit loop monotonic. Within a single rule the iteration order is
    // already start-ascending, but the merge across rules is not.
    scratch.runs.sort_by_key(|&(s, _, _)| s);

    &scratch.runs
}

/// Span-emitting variant for the Config TUI preview. Same matching +
/// overlap + priority semantics as [`apply_rules`]; returns owned spans
/// instead of writing SGR bytes.
///
/// **Capture-group styling:** rules with `uses_capture_styling[i] = true`
/// produce 1..=N spans per match per v0.3.5 inner-overrides-outer
/// semantics — identical to runtime path via shared [`select_runs`].
///
/// Returns `Vec<StyleSpan>` with byte offsets into `line` (sorted by
/// `start` ASC; non-overlapping). UTF-8 multi-byte boundary safety:
/// when `line` was derived from `&str` and patterns are Unicode-aware
/// (regex 1.12 default), match boundaries are char boundaries.
///
/// Snapshot Arc drop: dropped at function return; spans own `Style` by
/// Copy (verified in `src/style.rs:422`). No dangling reference.
pub(crate) fn apply_rules_spans(
    line: &[u8],
    compiled_handle: &ArcSwap<Compiled>,
    scratch: &mut PipelineScratch,
) -> Vec<StyleSpan> {
    let snapshot: Arc<Compiled> = compiled_handle.load_full();
    let runs = select_runs(line, snapshot.as_ref(), scratch);
    runs.iter().map(|&(start, end, style)| StyleSpan { start, end, style }).collect()
}

/// Named-span variant of [`apply_rules_spans`] for the corpus harness.
/// Applies the full production pipeline (priority sort + overlap suppression)
/// and returns `Vec<(rule_name, start, end)>` in start-ascending order.
///
/// `Compiled::names[i]` is parallel to `Compiled::individuals[i]`; each
/// accepted run is tagged with the originating rule name. Capture-group
/// styling rules (`uses_capture_styling[i] = true`) produce 1..=N sub-spans
/// per match — all are tagged with the same rule name (the full match owner).
///
/// Spec §5.3: corpus harness measurement primitive. Not part of the
/// production byte-emit path — used only by `crate::rules::testing_pipeline_spans`.
#[doc(hidden)]
pub(crate) fn select_runs_named(
    line: &[u8],
    compiled: &Compiled,
    scratch: &mut PipelineScratch,
) -> Vec<(String, usize, usize)> {
    scratch.accepted_spans.clear();
    scratch.runs.clear();
    scratch.event_scratch.clear();
    scratch.active_scratch.clear();
    scratch.set_match_scratch.clear();

    scratch.set_match_scratch.extend(compiled.set.matches(line).iter());

    {
        use std::cmp::Reverse;
        let priorities = &compiled.priorities;
        scratch.set_match_scratch.sort_by(|&a, &b| {
            Reverse(priorities[a]).cmp(&Reverse(priorities[b])).then_with(|| a.cmp(&b))
        });
    }

    // name_runs tracks (name_idx, start, end) parallel to scratch.runs so
    // we can re-sort by start and emit names in final order. A `usize` index
    // into `compiled.names` avoids cloning until the final collect.
    let mut name_runs: Vec<(usize, usize, usize)> = Vec::new();

    for &i in &scratch.set_match_scratch {
        let re = &compiled.individuals[i];
        if compiled.uses_capture_styling[i] {
            for caps in re.captures_iter(line) {
                let m = caps.get(0).expect("capture 0 is always the full match");
                let (start, end) = (m.start(), m.end());
                if overlaps_accepted(&scratch.accepted_spans, start, end) {
                    continue;
                }
                // Each sub-span from emit_capture_runs is tagged with rule i's
                // name; record the current runs length before emission so we can
                // tag the newly appended entries.
                let before = scratch.runs.len();
                emit_capture_runs(
                    &caps,
                    start,
                    end,
                    compiled.styles[i],
                    &compiled.group_styles[i],
                    &mut scratch.event_scratch,
                    &mut scratch.active_scratch,
                    &mut scratch.runs,
                );
                for run in &scratch.runs[before..] {
                    name_runs.push((i, run.0, run.1));
                }
                insert_accepted(&mut scratch.accepted_spans, start, end);
            }
        } else {
            for m in re.find_iter(line) {
                let (start, end) = (m.start(), m.end());
                if overlaps_accepted(&scratch.accepted_spans, start, end) {
                    continue;
                }
                scratch.runs.push((start, end, compiled.styles[i]));
                name_runs.push((i, start, end));
                insert_accepted(&mut scratch.accepted_spans, start, end);
            }
        }
    }

    // Sort by start to match the production apply_rules emit order.
    name_runs.sort_by_key(|&(_, s, _)| s);

    name_runs.into_iter().map(|(i, start, end)| (compiled.names[i].clone(), start, end)).collect()
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
/// `(start, end, Style)` runs using a boundary-event sweep. No per-byte
/// allocation; all transient state lives in caller-owned scratch Vecs
/// that are `.clear()`-reused across matches in the same line.
///
/// Inner (higher-index) groups override outer groups when nested. See
/// spec §1.1 for the algorithm walkthrough.
#[allow(clippy::too_many_arguments)] // reason: scratch Vecs are caller-owned to avoid per-match allocation; bundling them into a struct would obscure the reuse pattern.
pub(crate) fn emit_capture_runs(
    caps: &regex::bytes::Captures<'_>,
    match_start: usize,
    match_end: usize,
    default_style: Style,
    group_styles: &[Option<Style>],
    event_scratch: &mut Vec<(usize, OpenClose, u32)>,
    active_scratch: &mut Vec<u32>,
    out: &mut Vec<(usize, usize, Style)>,
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
            let style: Style = if active_g == 0 {
                default_style
            } else {
                group_styles[(active_g - 1) as usize]
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

/// Per-line scratch surface for `apply_rules`. Owned by `Pipeline` so the
/// five Vecs allocate at most once per process lifetime; each `apply_rules`
/// call `.clear()`-reuses them. Capacity grows monotonically to the
/// worst-case line's allocation (memory pressure bounded by max-line scratch
/// surface, not unbounded leak).
///
/// `runs` carries `Style` by value (Style is Copy + ~16 byte); this
/// eliminates the per-call `Arc<Compiled>` snapshot borrow that v0.3.5's
/// `Vec<(usize, usize, &'r Style)>` shape forced.
///
/// `set_match_scratch` collects `RegexSet::matches(line).iter()` output;
/// indices are in pattern-definition order (regex 1.12 stable contract).
#[derive(Default)]
pub(crate) struct PipelineScratch {
    pub(crate) accepted_spans: Vec<(usize, usize)>,
    pub(crate) runs: Vec<(usize, usize, Style)>,
    pub(crate) event_scratch: Vec<(usize, OpenClose, u32)>,
    pub(crate) active_scratch: Vec<u32>,
    pub(crate) set_match_scratch: Vec<usize>,
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
    /// Per-line scratch for `apply_rules`. See `PipelineScratch` doc-comment.
    scratch: PipelineScratch,
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
            scratch: PipelineScratch::default(),
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
            apply_rules(line, &self.rules, &mut self.scratch, out)?;
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
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"connect to 192.168.1.1 now\n", &rules, &mut scratch, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("\x1b["), "expected SGR introducer in: {s:?}");
        assert!(s.contains("192.168.1.1"));
        assert!(s.contains("\x1b[0m"), "expected SGR reset");
    }

    #[test]
    fn no_match_passes_through_unchanged() {
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"plain text line\n", &rules, &mut scratch, &mut out).unwrap();
        assert_eq!(out, b"plain text line\n");
    }

    #[test]
    fn apply_rules_hot_path_behavior_matches_v0_3_4_for_non_captures_rules() {
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"connect to 192.168.1.1 now\n", &rules, &mut scratch, &mut out).unwrap();
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
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"drwxr-xr-x file.txt\n", &rules, &mut scratch, &mut out).unwrap();
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
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"event at 2026-05-24T10:30:45.123Z fired\n", &rules, &mut scratch, &mut out)
            .unwrap();
        let s = String::from_utf8(out).unwrap();
        let intro_count = s.matches("\x1b[").count() - s.matches("\x1b[0m").count();
        assert!(intro_count >= 5, "expected >= 5 SGRs (date/sep/time/ms/tz); got: {s:?}");
    }

    #[test]
    fn syslog_timestamp_substring_survives_colorization() {
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"May 24 10:30:45 host service: msg\n", &rules, &mut scratch, &mut out)
            .unwrap();
        let s = String::from_utf8(out).unwrap();
        // Syslog branch has no captures -> match wrapped with the rule's default style.
        // Other rules (e.g., log_level on "msg") may add additional SGRs; this test
        // only asserts the timestamp substring survives colorization intact.
        assert!(s.contains("May 24 10:30:45"), "syslog timestamp must survive in output: {s:?}");
    }

    #[test]
    fn http_url_match_renders_three_sgrs_with_underline_on_path() {
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"docs at https://example.com/path now\n", &rules, &mut scratch, &mut out)
            .unwrap();
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
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"clone git@github.com:user/repo.git\n", &rules, &mut scratch, &mut out)
            .unwrap();
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
            names: vec!["test_capture".to_owned()],
            styles: vec![default],
            group_styles: vec![vec![Some(red), Some(blue)]],
            uses_capture_styling: vec![true],
            respect_existing_colors: false,
            priorities: vec![0],
        };
        let rules = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"X 12-34 Y\n", &rules, &mut scratch, &mut out).unwrap();
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
        let mut runs: Vec<(usize, usize, Style)> = Vec::new();
        emit_capture_runs(
            &caps,
            4,
            7,
            default,
            &group_styles,
            &mut event_scratch,
            &mut active_scratch,
            &mut runs,
        );
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].0, 4);
        assert_eq!(runs[0].1, 7);
        assert_eq!(runs[0].2, default);
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
        let mut runs: Vec<(usize, usize, Style)> = Vec::new();
        emit_capture_runs(&caps, 4, 7, default, &group_styles, &mut es, &mut as_, &mut runs);
        // Group 1 covers [4,7) — exactly the match. Single run = group style.
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].0, 4);
        assert_eq!(runs[0].1, 7);
        assert_eq!(runs[0].2, group_styles[0].unwrap());
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
        let mut runs: Vec<(usize, usize, Style)> = Vec::new();
        emit_capture_runs(&caps, 0, 2, default, &group_styles, &mut es, &mut as_, &mut runs);
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
        let mut runs: Vec<(usize, usize, Style)> = Vec::new();
        emit_capture_runs(&caps, 0, 3, default, &group_styles, &mut es, &mut as_, &mut runs);
        assert_eq!(runs.len(), 3);
        // a → outer (red)
        assert_eq!(runs[0].0, 0);
        assert_eq!(runs[0].1, 1);
        assert_eq!(runs[0].2, group_styles[0].unwrap());
        // b → inner (blue)
        assert_eq!(runs[1].0, 1);
        assert_eq!(runs[1].1, 2);
        assert_eq!(runs[1].2, group_styles[1].unwrap());
        // c → outer (red) again
        assert_eq!(runs[2].0, 2);
        assert_eq!(runs[2].1, 3);
        assert_eq!(runs[2].2, group_styles[0].unwrap());
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
        let mut runs: Vec<(usize, usize, Style)> = Vec::new();
        emit_capture_runs(&caps, 0, 1, default, &group_styles, &mut es, &mut as_, &mut runs);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].2, default);
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
        let mut runs: Vec<(usize, usize, Style)> = Vec::new();
        emit_capture_runs(&caps, 0, 3, default, &group_styles, &mut es, &mut as_, &mut runs);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].0, 0);
        assert_eq!(runs[0].1, 2);
        assert_eq!(runs[0].2, default);
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
        let mut runs: Vec<(usize, usize, Style)> = Vec::new();
        emit_capture_runs(&caps, 0, 3, default, &group_styles, &mut es, &mut as_, &mut runs);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].0, 0);
        assert_eq!(runs[0].1, 1);
        assert_eq!(runs[1].0, 1);
        assert_eq!(runs[1].1, 3);
        assert_eq!(runs[1].2, default);
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
        let mut runs: Vec<(usize, usize, Style)> = Vec::new();
        for caps in re.captures_iter(line) {
            let m = caps.get(0).unwrap();
            emit_capture_runs(
                &caps,
                m.start(),
                m.end(),
                default,
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

    /// Verifies that the caller-owned `PipelineScratch` Vecs retain their capacity
    /// across `apply_rules` calls (the `.clear()` reuse contract). NOT a zero-
    /// allocation invariant overall — `regex::bytes::RegexSet::matches` itself
    /// allocates a small bitset (~`pattern_count` word) per call internally; that
    /// is a fixed upstream cost outside `PipelineScratch`'s surface.
    #[test]
    fn pipeline_scratch_capacity_preserved_across_apply_rules_calls() {
        // Inline pattern mirrors existing apply_rules unit tests; no shared helper.
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out: Vec<u8> = Vec::new();

        // First call: populate.
        apply_rules(b"connect 192.168.1.1 now\n", &rules, &mut scratch, &mut out).unwrap();
        let cap_after_first = (
            scratch.accepted_spans.capacity(),
            scratch.runs.capacity(),
            scratch.event_scratch.capacity(),
            scratch.active_scratch.capacity(),
            scratch.set_match_scratch.capacity(),
        );
        out.clear();

        // Second call with a smaller line: capacities must NOT shrink (Vec::clear
        // preserves capacity). PipelineScratch surface stays allocation-free.
        apply_rules(b"\n", &rules, &mut scratch, &mut out).unwrap();
        let cap_after_second = (
            scratch.accepted_spans.capacity(),
            scratch.runs.capacity(),
            scratch.event_scratch.capacity(),
            scratch.active_scratch.capacity(),
            scratch.set_match_scratch.capacity(),
        );

        assert_eq!(
            cap_after_first, cap_after_second,
            "PipelineScratch capacities must be preserved across apply_rules calls"
        );
    }

    #[test]
    fn apply_rules_no_set_hits_emits_line_byte_identical() {
        // Line content matches NO built-in pattern: pure ASCII narrative with
        // no IPs, no timestamps, no log levels, no permissions, no URLs/Git URLs.
        let line = b"the quick brown fox jumps over the lazy dog\n";
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out: Vec<u8> = Vec::new();
        apply_rules(line, &rules, &mut scratch, &mut out).unwrap();
        assert_eq!(out, line, "no SGR injected when no rule hits");
        assert!(scratch.set_match_scratch.is_empty(), "pre-filter found zero hits");
    }

    #[test]
    fn apply_rules_priority_higher_wins_envelope_over_interior() {
        // v0.5.6 §2.1.B6 / §4.3 — priority DESC sort lets a higher-index,
        // higher-priority envelope rule claim the span before a lower-index,
        // lower-priority interior rule. Pattern order is intentionally swapped
        // (interior at index 0, envelope at index 1) so the sort step is
        // load-bearing: without it, iteration order = [0, 1] → interior
        // accepts first, envelope rejected by overlap. With it, the priority
        // DESC sort reorders to [1, 0] → envelope accepts, interior rejected.
        use crate::style::{Color, Style};
        use regex::bytes::{Regex, RegexSet};
        let interior_pat = r"\d+";
        let envelope_pat = r"a\d+b";
        let compiled = Compiled {
            set: RegexSet::new([interior_pat, envelope_pat]).unwrap(),
            individuals: vec![Regex::new(interior_pat).unwrap(), Regex::new(envelope_pat).unwrap()],
            names: vec!["interior".to_owned(), "envelope".to_owned()],
            styles: vec![
                Style { fg: Some(Color::Green), ..Style::DEFAULT }, // rule 0 = interior
                Style { fg: Some(Color::Red), ..Style::DEFAULT },   // rule 1 = envelope
            ],
            group_styles: vec![vec![], vec![]],
            uses_capture_styling: vec![false, false],
            respect_existing_colors: true,
            priorities: vec![0, 200], // interior 0, envelope 200
        };
        let handle = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"a123b\n", &handle, &mut scratch, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        // Envelope (rule 1, Red SGR 31) accepts a123b; interior overlap → REJECT.
        assert!(s.contains("\x1b[31m"), "expected Red SGR (rule 1, envelope, priority 200): {s:?}");
        assert!(!s.contains("\x1b[32m"), "Green SGR (rule 0, interior) should not appear: {s:?}");
    }

    #[test]
    fn apply_rules_preserves_pattern_definition_order_for_overlapping_matches() {
        // Two synthetic rules where rule 0 and rule 1 both match overlapping
        // substrings on the same line. First-match-wins must give rule 0 the
        // span; rule 1's match must be dropped by accepted_spans overlap
        // detection. If RegexSet iteration ever switched away from pattern
        // order (e.g. via HashSet), rule 1 could pre-empt rule 0 silently.
        use crate::style::{Color, Style};
        use regex::bytes::{Regex, RegexSet};
        let red = Style { fg: Some(Color::Red), ..Style::DEFAULT };
        let blue = Style { fg: Some(Color::Blue), ..Style::DEFAULT };
        let compiled = Compiled {
            set: RegexSet::new([r"\d{3,5}", r"\d{2}"]).unwrap(),
            individuals: vec![Regex::new(r"\d{3,5}").unwrap(), Regex::new(r"\d{2}").unwrap()],
            names: vec!["long_digits".to_owned(), "short_digits".to_owned()],
            styles: vec![red, blue],
            group_styles: vec![vec![], vec![]],
            uses_capture_styling: vec![false, false],
            respect_existing_colors: false,
            priorities: vec![0, 0],
        };
        let rules = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out: Vec<u8> = Vec::new();
        apply_rules(b"value 12345\n", &rules, &mut scratch, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        // Rule 0 (red, SGR 31) must wrap "12345"; rule 1 (blue, SGR 34) must
        // be suppressed by the overlap check — no SGR 34 in output.
        assert!(s.contains("\x1b[31m"), "rule 0 (red) must fire on '12345': {s:?}");
        assert!(!s.contains("\x1b[34m"), "rule 1 (blue) must be suppressed by overlap: {s:?}");
    }

    #[test]
    fn apply_rules_priority_sort_is_stable_under_equal_priorities() {
        // K=3 priority-0 rules; pin that sort preserves rule_index ASC tie-break.
        // All three patterns are non-overlapping so all should fire — the test
        // primarily guards stability: the sort_by call must use a stable sort,
        // not sort_unstable_by, so byte-identical v0.5.5 behavior is preserved
        // for the priority-0 == priority-0 case.
        use regex::bytes::{Regex, RegexSet};
        let pats = [r"foo", r"bar", r"baz"];
        let compiled = Compiled {
            set: RegexSet::new(pats).unwrap(),
            individuals: pats.iter().map(|p| Regex::new(p).unwrap()).collect(),
            names: pats.iter().map(|p| (*p).to_owned()).collect(),
            styles: vec![Style::DEFAULT; 3],
            group_styles: vec![vec![]; 3],
            uses_capture_styling: vec![false; 3],
            respect_existing_colors: true,
            priorities: vec![0, 0, 0],
        };
        let handle = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"baz bar foo\n", &handle, &mut scratch, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("foo") && s.contains("bar") && s.contains("baz"));
    }

    #[test]
    fn apply_rules_priority_equal_falls_back_to_rule_index_order() {
        // Two overlapping priority-0 rules; lower-index wins (v0.5.5 invariant).
        use crate::style::{Color, Style};
        use regex::bytes::{Regex, RegexSet};
        let compiled = Compiled {
            set: RegexSet::new([r"\d{3,5}", r"\d{2}"]).unwrap(),
            individuals: vec![Regex::new(r"\d{3,5}").unwrap(), Regex::new(r"\d{2}").unwrap()],
            names: vec!["long_match".to_owned(), "short_match".to_owned()],
            styles: vec![
                Style { fg: Some(Color::Red), ..Style::DEFAULT }, // rule 0 — longer match
                Style { fg: Some(Color::Green), ..Style::DEFAULT }, // rule 1 — shorter
            ],
            group_styles: vec![vec![], vec![]],
            uses_capture_styling: vec![false, false],
            respect_existing_colors: true,
            priorities: vec![0, 0],
        };
        let handle = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"12345\n", &handle, &mut scratch, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains("\x1b[31m"),
            "rule 0 (Red, lower index, longer match) wins overlap: {s:?}"
        );
    }

    #[test]
    fn apply_rules_priority_negative_yields_to_default() {
        // Rule 0 (Red, neg priority) matches outer "abc"; rule 1 (Green, prio 0)
        // matches interior "a". Sort: Reverse(0) > Reverse(-100) → rule 1 first.
        // Rule 1 accepts "a"; rule 0 then overlap-rejected on "abc".
        use crate::style::{Color, Style};
        use regex::bytes::{Regex, RegexSet};
        let compiled = Compiled {
            set: RegexSet::new([r"abc", r"a"]).unwrap(),
            individuals: vec![Regex::new(r"abc").unwrap(), Regex::new(r"a").unwrap()],
            names: vec!["neg_priority_outer".to_owned(), "default_priority_inner".to_owned()],
            styles: vec![
                Style { fg: Some(Color::Red), ..Style::DEFAULT },
                Style { fg: Some(Color::Green), ..Style::DEFAULT },
            ],
            group_styles: vec![vec![], vec![]],
            uses_capture_styling: vec![false, false],
            respect_existing_colors: true,
            priorities: vec![-100, 0],
        };
        let handle = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"abc\n", &handle, &mut scratch, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains("\x1b[32m"),
            "rule 1 (Green, default priority) wins envelope acceptance: {s:?}"
        );
        assert!(!s.contains("\x1b[31m"), "rule 0 (Red, neg priority) yields: {s:?}");
    }

    #[test]
    fn apply_rules_priority_extreme_values_do_not_overflow() {
        // i32::MAX and i32::MIN — Reverse wrapper must not overflow during cmp.
        // Two non-overlapping patterns; sort iterates rule 0 (i32::MAX) first,
        // then rule 1 (i32::MIN). No panic = pass; observable color order
        // doesn't matter, only that the sort cmp doesn't UB-overflow.
        use regex::bytes::{Regex, RegexSet};
        let compiled = Compiled {
            set: RegexSet::new([r"foo", r"bar"]).unwrap(),
            individuals: vec![Regex::new(r"foo").unwrap(), Regex::new(r"bar").unwrap()],
            names: vec!["extreme_max".to_owned(), "extreme_min".to_owned()],
            styles: vec![Style::DEFAULT; 2],
            group_styles: vec![vec![]; 2],
            uses_capture_styling: vec![false; 2],
            respect_existing_colors: true,
            priorities: vec![i32::MAX, i32::MIN],
        };
        let handle = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        // Should not panic; sort should not overflow.
        apply_rules(b"foo bar\n", &handle, &mut scratch, &mut out).unwrap();
    }

    // ---------------------------------------------------------------
    // v0.6 Group 1 — select_runs / apply_rules_spans coverage.
    // ---------------------------------------------------------------

    #[test]
    fn select_runs_extract_preserves_apply_rules_byte_output() {
        // Golden parity: select_runs must NOT change apply_rules byte output.
        // We compile a real Compiled (load_builtins), feed sample lines through
        // apply_rules (which now uses select_runs internally), and compare bytes
        // against the v0.5.7 fixture captures (regenerated here via in-test
        // baseline computation since this is a refactor — the test compares
        // apply_rules to itself across two calls, ensuring determinism).
        let compiled = Arc::new(Compiled::load_builtins().expect("load builtins"));
        let handle = ArcSwap::from(compiled);
        let mut scratch_a = PipelineScratch::default();
        let mut scratch_b = PipelineScratch::default();
        let samples = [
            b"INFO 192.168.1.1 connection from 10.0.0.5".as_slice(),
            b"ERROR uuid=550e8400-e29b-41d4-a716-446655440000".as_slice(),
            b"GET /api/v1/users HTTP/1.1".as_slice(),
            b"plain text with no matches".as_slice(),
            b"".as_slice(),
            b"hex address: deadbeef cafe in body".as_slice(),
            b"timestamp 2026-05-28T12:00:00Z".as_slice(),
            b"multiple matches 1.2.3.4 and 5.6.7.8".as_slice(),
        ];
        for sample in samples {
            let mut out_a = Vec::new();
            apply_rules(sample, &handle, &mut scratch_a, &mut out_a).unwrap();
            let mut out_b = Vec::new();
            apply_rules(sample, &handle, &mut scratch_b, &mut out_b).unwrap();
            assert_eq!(
                out_a,
                out_b,
                "apply_rules deterministic on sample: {:?}",
                std::str::from_utf8(sample).unwrap_or("<non-utf8>")
            );
            assert!(
                !out_a.is_empty() || sample.is_empty(),
                "non-empty input must produce non-empty output"
            );
        }
    }

    #[test]
    fn apply_rules_spans_returns_empty_vec_for_no_match() {
        let compiled = Arc::new(Compiled::load_builtins().expect("load builtins"));
        let handle = ArcSwap::from(compiled);
        let mut scratch = PipelineScratch::default();
        let spans = apply_rules_spans(b"plain text no matches", &handle, &mut scratch);
        assert!(spans.is_empty(), "no-match line yields empty Vec<StyleSpan>");
    }

    #[test]
    fn apply_rules_spans_returns_single_span_for_single_match() {
        let compiled = Arc::new(Compiled::load_builtins().expect("load builtins"));
        let handle = ArcSwap::from(compiled);
        let mut scratch = PipelineScratch::default();
        let spans = apply_rules_spans(b"see 192.168.1.1 here", &handle, &mut scratch);
        assert_eq!(spans.len(), 1, "exactly one ipv4 span");
        assert_eq!(spans[0].start, 4, "start byte of '192.168.1.1'");
        assert_eq!(spans[0].end, 15, "end byte of '192.168.1.1' exclusive");
    }

    #[test]
    fn apply_rules_spans_respects_overlap_rejection() {
        let compiled = Arc::new(Compiled::load_builtins().expect("load builtins"));
        let handle = ArcSwap::from(compiled);
        let mut scratch = PipelineScratch::default();
        let spans = apply_rules_spans(b"https://192.168.1.1/api", &handle, &mut scratch);
        let url_span = spans.iter().find(|s| s.end - s.start > 10).expect("url span");
        let interior_overlapping = spans
            .iter()
            .filter(|s| {
                s.start >= url_span.start && s.end <= url_span.end && s.start != url_span.start
            })
            .count();
        assert_eq!(interior_overlapping, 0, "no interior span inside accepted url span");
    }

    #[test]
    fn apply_rules_spans_returns_capture_subspans_when_uses_capture_styling() {
        use regex::bytes::{Regex, RegexSet};
        use std::sync::Arc;
        let red = Style { fg: Some(crate::style::Color::Red), ..Default::default() };
        let blue = Style { fg: Some(crate::style::Color::Blue), ..Default::default() };
        let re = Regex::new(r"(\d+)-(\d+)").unwrap();
        let compiled = Arc::new(Compiled {
            set: RegexSet::new([r"(\d+)-(\d+)"]).unwrap(),
            individuals: vec![re],
            names: vec!["capture_range".to_owned()],
            styles: vec![Style::default()],
            group_styles: vec![vec![Some(red), Some(blue)]],
            uses_capture_styling: vec![true],
            respect_existing_colors: false,
            priorities: vec![0],
        });
        let handle = ArcSwap::from(compiled);
        let mut scratch = PipelineScratch::default();
        let spans = apply_rules_spans(b"see 42-100 here", &handle, &mut scratch);
        // Pattern `(\d+)-(\d+)` over `42-100`: g1=42 (bytes 4..6), unstyled `-`
        // gap (byte 6..7), g2=100 (bytes 7..10). `emit_capture_runs` emits a
        // default-styled run for the inter-group gap — see existing test
        // `emit_capture_runs_capture_with_gap_before_emits_default_then_group`.
        assert_eq!(spans.len(), 3, "g1 + default-gap + g2 sub-spans");
        assert_eq!(spans[0].style.fg, Some(crate::style::Color::Red), "first group red");
        assert_eq!(spans[0].start, 4);
        assert_eq!(spans[0].end, 6);
        assert_eq!(spans[1].style.fg, None, "gap span carries default style");
        assert_eq!(spans[1].start, 6);
        assert_eq!(spans[1].end, 7);
        assert_eq!(spans[2].style.fg, Some(crate::style::Color::Blue), "second group blue");
        assert_eq!(spans[2].start, 7);
        assert_eq!(spans[2].end, 10);
    }

    #[test]
    fn apply_rules_spans_byte_offsets_point_into_line() {
        let line = b"192.168.1.1 then 10.0.0.5";
        let compiled = Arc::new(Compiled::load_builtins().expect("load builtins"));
        let handle = ArcSwap::from(compiled);
        let mut scratch = PipelineScratch::default();
        let spans = apply_rules_spans(line, &handle, &mut scratch);
        for s in &spans {
            assert!(s.start < s.end, "start < end");
            assert!(s.end <= line.len(), "end ≤ line.len()");
            let _ = &line[s.start..s.end];
        }
    }

    #[test]
    fn apply_rules_spans_utf8_multibyte_span_boundary_is_char_boundary() {
        let line_str = "duration: 5μs elapsed";
        let line = line_str.as_bytes();
        let compiled = Arc::new(Compiled::load_builtins().expect("load builtins"));
        let handle = ArcSwap::from(compiled);
        let mut scratch = PipelineScratch::default();
        let spans = apply_rules_spans(line, &handle, &mut scratch);
        let duration_span = spans
            .iter()
            .find(|s| std::str::from_utf8(&line[s.start..s.end]).is_ok_and(|t| t.contains("μs")));
        if let Some(s) = duration_span {
            assert!(line_str.is_char_boundary(s.start), "start on char boundary");
            assert!(line_str.is_char_boundary(s.end), "end on char boundary");
        }
    }

    #[test]
    fn apply_rules_spans_byte_emit_parity() {
        let compiled = Arc::new(Compiled::load_builtins().expect("load builtins"));
        let handle = ArcSwap::from(compiled);
        let mut scratch_a = PipelineScratch::default();
        let mut scratch_b = PipelineScratch::default();
        let line = b"see 192.168.1.1 and uuid=550e8400-e29b-41d4-a716-446655440000";
        let mut out_byte = Vec::new();
        apply_rules(line, &handle, &mut scratch_a, &mut out_byte).unwrap();
        let spans = apply_rules_spans(line, &handle, &mut scratch_b);
        let mut out_span = Vec::<u8>::new();
        let mut cursor = 0usize;
        for s in &spans {
            out_span.extend_from_slice(&line[cursor..s.start]);
            let sgr = s.style.to_sgr();
            if !sgr.is_empty() {
                out_span.extend_from_slice(sgr.as_bytes());
            }
            out_span.extend_from_slice(&line[s.start..s.end]);
            out_span.extend_from_slice(Style::reset_sgr().as_bytes());
            cursor = s.end;
        }
        out_span.extend_from_slice(&line[cursor..]);
        assert_eq!(out_byte, out_span, "byte-emit and span-emit reconstructions identical");
    }

    #[test]
    fn apply_rules_spans_priority_extreme_values_do_not_overflow() {
        use regex::bytes::{Regex, RegexSet};
        let compiled = Arc::new(Compiled {
            set: RegexSet::new([r"a", r"b"]).unwrap(),
            individuals: vec![Regex::new(r"a").unwrap(), Regex::new(r"b").unwrap()],
            names: vec!["extreme_max_spans".to_owned(), "extreme_min_spans".to_owned()],
            styles: vec![Style::default(), Style::default()],
            group_styles: vec![vec![], vec![]],
            uses_capture_styling: vec![false, false],
            respect_existing_colors: false,
            priorities: vec![i32::MAX, i32::MIN],
        });
        let handle = ArcSwap::from(compiled);
        let mut scratch = PipelineScratch::default();
        let _ = apply_rules_spans(b"ab", &handle, &mut scratch);
    }

    #[test]
    fn apply_rules_spans_sorted_by_start_ascending_invariant() {
        let compiled = Arc::new(Compiled::load_builtins().expect("load builtins"));
        let handle = ArcSwap::from(compiled);
        let mut scratch = PipelineScratch::default();
        let spans =
            apply_rules_spans(b"1.2.3.4 then 5.6.7.8 then 9.10.11.12", &handle, &mut scratch);
        for w in spans.windows(2) {
            assert!(w[0].start <= w[1].start, "spans sorted by start ASC");
            assert!(w[0].end <= w[1].start, "non-overlapping");
        }
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
        let compiled = Compiled::load_with_theme(
            None,
            None,
            None,
            None,
            None,
            crate::terminfo::ColorDepth::Truecolor,
        )
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
        let compiled = Compiled::load_with_theme(
            None,
            None,
            None,
            None,
            None,
            crate::terminfo::ColorDepth::Truecolor,
        )
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
        let compiled = Compiled::load_with_theme(
            None,
            None,
            None,
            None,
            None,
            crate::terminfo::ColorDepth::Truecolor,
        )
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
        let compiled = Compiled::load_with_theme(
            None,
            None,
            None,
            None,
            None,
            crate::terminfo::ColorDepth::Truecolor,
        )
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
            None,
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
