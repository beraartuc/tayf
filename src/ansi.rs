//! ANSI byte-stream state machine.
//!
//! Implements a 16-state subset of Paul Williams' VT500 ANSI parser
//! (14 logical states from the canonical reference, plus two peek-ahead
//! states — `DcsEsc` and `SosPmApcEsc` — that resolve the 7-bit ST
//! terminator `\e\\` lookahead)
//! (<https://vt100.net/emu/dec_ansi_parser>) scoped to tayf's classification
//! needs. The SM does not interpret payloads; it classifies each byte by
//! what `Pipeline` should do with it:
//!
//! - **Data**: normal text byte, line-buffered for rule application.
//! - **`SequenceByte`**: byte that is part of a CSI/ESC sequence whose
//!   destination (stdout or `line_buffer`) is decided when the sequence
//!   completes. `Pipeline` accumulates these in a scratch buffer.
//! - **`StringPayloadByte`**: byte that is part of an OSC/DCS/PM/APC payload
//!   (or its introducer or terminator). `Pipeline` routes the byte direct to
//!   stdout. `Pipeline`'s policy on the surrounding line is to flush any
//!   partial pre-string buffer verbatim to stdout and treat the rest of the
//!   line as verbatim too; the SM does not enforce this — it only classifies
//!   bytes.
//! - **`SequenceCompleted(kind)`**: terminal event for a CSI/ESC sequence.
//!   `kind` tells `Pipeline` where the accumulated scratch goes.
//!
//! **Do NOT invent new states or transitions without consulting Williams'
//! reference.** This SM is a deliberate subset; adding a new sequence class
//! means walking Williams' table, identifying the relevant states, and
//! folding their transitions in. See spec §3.4 (the locked transition
//! table) before extending.

/// 16-state Williams VT500 subset (14 canonical + 2 ST peek-ahead).
/// See spec §3.2 for the canonical states and §3.4 for the peek-ahead
/// transitions on `OscEsc` / `DcsEsc` / `SosPmApcEsc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum SmState {
    Ground,
    Escape,
    EscapeIntermediate,
    CsiEntry,
    CsiParam,
    CsiIntermediate,
    CsiIgnore,
    OscString,
    OscEsc,
    DcsEntry,
    DcsParam,
    DcsIntermediate,
    DcsPassthrough,
    DcsEsc,
    /// Combined: `DcsIgnore` and PM/APC string share the same drop-until-ST behavior.
    SosPmApcString,
    SosPmApcEsc,
}

/// Per-byte event surfaced from [`AnsiSm::step`]. See spec §3.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepEvent {
    /// Ground state, normal data byte. Pipeline: feed byte to `line_buffer`.
    /// Invariant: SM scratch (kept by Pipeline) is empty when this fires.
    Data,
    /// Byte is part of an in-progress CSI / ESC / `EscIntermediate` sequence.
    /// Pipeline: push byte onto `sequence_scratch`.
    SequenceByte,
    /// Byte is part of an OSC/DCS-passthrough/PM/APC payload (or its
    /// introducer or terminator). Pipeline: flush `sequence_scratch` to
    /// stdout if non-empty, then write byte to stdout.
    StringPayloadByte,
    /// CSI or ESC sequence just completed (final byte already counted as
    /// the triggering byte for this event). Pipeline: push the final byte
    /// onto `sequence_scratch`, then route the scratch based on `kind`.
    SequenceCompleted(SequenceKind),
}

/// Classification of a completed CSI or ESC sequence; tells `Pipeline`
/// where the accumulated `sequence_scratch` should go. See spec §3.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SequenceKind {
    /// `CSI ?Pm h` that flipped a TUI flag (alt-screen / paste / mouse) ON.
    /// Pipeline: write `sequence_scratch` to stdout; following bytes are
    /// in TUI passthrough mode (write each byte direct to stdout).
    TuiToggleOn,
    /// `CSI ?Pm l` matching a TUI flag turning OFF.
    /// Pipeline: write `sequence_scratch` to stdout; exit TUI passthrough.
    TuiToggleOff,
    /// `CSI ... m` (SGR). Pipeline: drain scratch into `line_buffer`;
    /// set `line_has_sgr = true`.
    Sgr,
    /// CSI ending in a non-`m` final, or terminated `CsiIgnore`.
    /// Pipeline: drain scratch into `line_buffer`.
    OtherCsi,
    /// Single-byte ESC final (e.g. `\e=`, `\eM`, `\e7`, `\ec`).
    /// Pipeline: drain scratch (2 bytes) into `line_buffer`.
    EscFinal,
    /// ESC + intermediate(s) + final (e.g. `\e(B`, `\e#8`).
    /// Pipeline: drain scratch (3+ bytes) into `line_buffer`.
    EscIntermediateFinal,
}

/// Hard cap on bytes accumulated within a single in-progress CSI/ESC
/// sequence before the SM force-resets to Ground. Defense against
/// malicious unterminated input. Spec §7.1.
const SEQUENCE_BYTES_CAP: u16 = 4096;

/// TUI mode bitmask flags. Any non-zero value means alt-screen / bracketed
/// paste / mouse tracking is active; bytes go straight to stdout without
/// passing through `apply_rules`.
mod tui_flags {
    pub const ALT_SCREEN: u32 = 1 << 0;
    pub const BRACKETED_PASTE: u32 = 1 << 1;
    pub const MOUSE: u32 = 1 << 2;
}

/// Map a DEC private mode number (e.g. `1049`) to a TUI flag bit; 0 if not
/// a tracked TUI indicator. See spec §3.4 (private mode classification).
fn flag_for_mode(num: u32) -> u32 {
    match num {
        47 | 1047 | 1049 => tui_flags::ALT_SCREEN,
        2004 => tui_flags::BRACKETED_PASTE,
        1000 | 1002 | 1003 | 1006 => tui_flags::MOUSE,
        _ => 0,
    }
}

/// Per-byte ANSI state machine. See spec §3 for the full transition table.
#[derive(Debug)]
pub(crate) struct AnsiSm {
    state: SmState,
    flags: u32,
    accum: u32,
    private_mode: bool,
    /// Bytes accumulated within the current CSI/ESC sequence; capped at
    /// [`SEQUENCE_BYTES_CAP`] (4 KiB). Tasks 3+ enforce the cap by
    /// resetting to Ground when this field reaches the constant.
    sequence_bytes_seen: u16,
}

impl AnsiSm {
    pub(crate) fn new() -> Self {
        AnsiSm {
            state: SmState::Ground,
            flags: 0,
            accum: 0,
            private_mode: false,
            sequence_bytes_seen: 0,
        }
    }

    /// True iff at least one TUI mode flag is currently set.
    pub(crate) fn tui_mode_active(&self) -> bool {
        self.flags != 0
    }

    /// Advance the machine by one byte; emit the classification event.
    ///
    /// Implements the full Williams VT500 subset: CSI (with `CsiIgnore`
    /// recovery), ESC direct finals, `EscapeIntermediate`, and the
    /// OSC / DCS / PM / APC string sequences. See spec §3.4.
    #[allow(clippy::too_many_lines)]
    // reason: the per-state match is the spec's transition table written as
    // code; splitting state arms into helpers obscures Williams §3.4. The
    // shape stays here verbatim and grows linearly with new states.
    pub(crate) fn step(&mut self, byte: u8) -> StepEvent {
        // ESC normally aborts any in-progress sequence and restarts at Escape.
        // String-payload states are the exception: their per-state arms peek
        // ahead one byte to resolve the 7-bit ST terminator `\e\\`, so we
        // must route 0x1b through the match for them. See spec §3.4.
        if byte == 0x1b
            && !matches!(
                self.state,
                SmState::OscString
                    | SmState::OscEsc
                    | SmState::DcsEntry
                    | SmState::DcsParam
                    | SmState::DcsIntermediate
                    | SmState::DcsPassthrough
                    | SmState::DcsEsc
                    | SmState::SosPmApcString
                    | SmState::SosPmApcEsc
            )
        {
            self.state = SmState::Escape;
            self.accum = 0;
            self.private_mode = false;
            self.sequence_bytes_seen = 1;
            return StepEvent::SequenceByte;
        }

        // Defense-in-depth: refuse to accumulate sequence bytes past
        // SEQUENCE_BYTES_CAP (4 KiB). Catches malicious unterminated CSI/ESC
        // inputs (spec §7.1). On hit, SM resets to Ground; the offending
        // byte is consumed as a Data event so the byte stream stays in sync
        // (it was inside a sequence so visually noise anyway).
        if self.sequence_bytes_seen >= SEQUENCE_BYTES_CAP {
            self.state = SmState::Ground;
            self.accum = 0;
            self.private_mode = false;
            self.sequence_bytes_seen = 0;
            return StepEvent::Data;
        }

        match self.state {
            SmState::Ground => StepEvent::Data,

            SmState::Escape => {
                self.sequence_bytes_seen = self.sequence_bytes_seen.saturating_add(1);
                match byte {
                    b'[' => {
                        self.state = SmState::CsiEntry;
                        StepEvent::SequenceByte
                    }
                    b']' => {
                        // OSC string sequence start. The `]` byte transitions
                        // us into OscString; Pipeline reads StringPayloadByte
                        // and flushes the leading `\e` from scratch + writes `]`
                        // to stdout. Subsequent payload bytes also StringPayloadByte.
                        self.state = SmState::OscString;
                        StepEvent::StringPayloadByte
                    }
                    b'P' => {
                        self.state = SmState::DcsEntry;
                        StepEvent::StringPayloadByte
                    }
                    b'X' | b'^' | b'_' => {
                        self.state = SmState::SosPmApcString;
                        StepEvent::StringPayloadByte
                    }
                    // INTERMEDIATE: \e ( B, \e # 8, etc.
                    0x20..=0x2F => {
                        self.state = SmState::EscapeIntermediate;
                        StepEvent::SequenceByte
                    }
                    // Single-byte ESC final: \e=, \eM, \e7, \e8, \ec, \eD, ...
                    // Williams range 0x30..=0x7E minus introducers handled above.
                    0x30..=0x7E => {
                        self.state = SmState::Ground;
                        self.accum = 0;
                        self.sequence_bytes_seen = 0;
                        StepEvent::SequenceCompleted(SequenceKind::EscFinal)
                    }
                    // C0 control / DEL: treat as inert noise, complete with EscFinal.
                    _ => {
                        self.state = SmState::Ground;
                        self.sequence_bytes_seen = 0;
                        StepEvent::SequenceCompleted(SequenceKind::EscFinal)
                    }
                }
            }

            SmState::EscapeIntermediate => {
                self.sequence_bytes_seen = self.sequence_bytes_seen.saturating_add(1);
                #[allow(clippy::single_match_else)]
                // reason: matching on the 0x20..=0x2F range with a multi-line
                // wildcard reads as Williams' transition table verbatim; the
                // `if let` rewrite hides the range semantics.
                match byte {
                    0x20..=0x2F => StepEvent::SequenceByte, // intermediate chain
                    _ => {
                        self.state = SmState::Ground;
                        self.sequence_bytes_seen = 0;
                        StepEvent::SequenceCompleted(SequenceKind::EscIntermediateFinal)
                    }
                }
            }

            SmState::CsiEntry => {
                self.sequence_bytes_seen = self.sequence_bytes_seen.saturating_add(1);
                match byte {
                    b'0'..=b'9' => {
                        self.accum = u32::from(byte - b'0');
                        self.state = SmState::CsiParam;
                        StepEvent::SequenceByte
                    }
                    b';' => {
                        self.state = SmState::CsiParam;
                        StepEvent::SequenceByte
                    }
                    b'<' | b'=' | b'>' | b'?' => {
                        self.private_mode = true;
                        self.state = SmState::CsiParam;
                        StepEvent::SequenceByte
                    }
                    0x20..=0x2F => {
                        self.state = SmState::CsiIntermediate;
                        StepEvent::SequenceByte
                    }
                    0x40..=0x7E => self.finalize_csi(byte),
                    _ => {
                        self.state = SmState::CsiIgnore;
                        StepEvent::SequenceByte
                    }
                }
            }

            SmState::CsiParam => {
                self.sequence_bytes_seen = self.sequence_bytes_seen.saturating_add(1);
                match byte {
                    b'0'..=b'9' => {
                        self.accum =
                            self.accum.saturating_mul(10).saturating_add(u32::from(byte - b'0'));
                        StepEvent::SequenceByte
                    }
                    b';' => {
                        self.accum = 0;
                        StepEvent::SequenceByte
                    }
                    0x20..=0x2F => {
                        self.state = SmState::CsiIntermediate;
                        StepEvent::SequenceByte
                    }
                    0x40..=0x7E => self.finalize_csi(byte),
                    _ => {
                        self.state = SmState::CsiIgnore;
                        StepEvent::SequenceByte
                    }
                }
            }

            SmState::CsiIntermediate => {
                self.sequence_bytes_seen = self.sequence_bytes_seen.saturating_add(1);
                match byte {
                    0x20..=0x2F => StepEvent::SequenceByte,
                    0x40..=0x7E => {
                        self.state = SmState::Ground;
                        self.private_mode = false;
                        self.accum = 0;
                        self.sequence_bytes_seen = 0;
                        StepEvent::SequenceCompleted(SequenceKind::OtherCsi)
                    }
                    _ => {
                        self.state = SmState::CsiIgnore;
                        StepEvent::SequenceByte
                    }
                }
            }

            SmState::CsiIgnore => {
                self.sequence_bytes_seen = self.sequence_bytes_seen.saturating_add(1);
                match byte {
                    0x40..=0x7E => {
                        self.state = SmState::Ground;
                        self.private_mode = false;
                        self.accum = 0;
                        self.sequence_bytes_seen = 0;
                        StepEvent::SequenceCompleted(SequenceKind::OtherCsi)
                    }
                    _ => StepEvent::SequenceByte,
                }
            }

            SmState::OscString => {
                self.sequence_bytes_seen = self.sequence_bytes_seen.saturating_add(1);
                match byte {
                    0x07 => {
                        // BEL terminator. Emit StringPayloadByte for the BEL,
                        // transition to Ground for next byte.
                        self.state = SmState::Ground;
                        self.sequence_bytes_seen = 0;
                        StepEvent::StringPayloadByte
                    }
                    0x1b => {
                        // Peek-ahead for ST. Don't emit data event yet —
                        // OscEsc state decides on next byte.
                        self.state = SmState::OscEsc;
                        StepEvent::StringPayloadByte
                    }
                    _ => StepEvent::StringPayloadByte,
                }
            }

            SmState::OscEsc => {
                self.sequence_bytes_seen = self.sequence_bytes_seen.saturating_add(1);
                match byte {
                    b'\\' => {
                        // ST completed.
                        self.state = SmState::Ground;
                        self.sequence_bytes_seen = 0;
                        StepEvent::StringPayloadByte
                    }
                    0x1b => {
                        // Double ESC in OSC — abort, restart Escape.
                        self.state = SmState::Escape;
                        self.accum = 0;
                        self.private_mode = false;
                        self.sequence_bytes_seen = 1;
                        StepEvent::SequenceByte
                    }
                    _ => {
                        // OSC aborted; byte starts new sequence from Escape state.
                        // Safe recursion: Escape arm never re-enters OscEsc.
                        self.state = SmState::Escape;
                        self.accum = 0;
                        self.private_mode = false;
                        self.sequence_bytes_seen = 1;
                        self.step(byte)
                    }
                }
            }

            SmState::DcsEntry => {
                self.sequence_bytes_seen = self.sequence_bytes_seen.saturating_add(1);
                match byte {
                    b'0'..=b'9' | b';' => {
                        self.state = SmState::DcsParam;
                        StepEvent::StringPayloadByte
                    }
                    0x20..=0x2F => {
                        self.state = SmState::DcsIntermediate;
                        StepEvent::StringPayloadByte
                    }
                    0x40..=0x7E => {
                        self.state = SmState::DcsPassthrough;
                        StepEvent::StringPayloadByte
                    }
                    _ => {
                        // Invalid: route to SosPmApcString (acts as DcsIgnore).
                        self.state = SmState::SosPmApcString;
                        StepEvent::StringPayloadByte
                    }
                }
            }

            SmState::DcsParam | SmState::DcsIntermediate => {
                self.sequence_bytes_seen = self.sequence_bytes_seen.saturating_add(1);
                match byte {
                    0x40..=0x7E => {
                        self.state = SmState::DcsPassthrough;
                        StepEvent::StringPayloadByte
                    }
                    0x1b => {
                        // ESC mid-DCS-header aborts. Restart Escape.
                        self.state = SmState::Escape;
                        self.sequence_bytes_seen = 1;
                        StepEvent::SequenceByte
                    }
                    _ => StepEvent::StringPayloadByte,
                }
            }

            SmState::DcsPassthrough => {
                self.sequence_bytes_seen = self.sequence_bytes_seen.saturating_add(1);
                match byte {
                    0x1b => {
                        self.state = SmState::DcsEsc;
                        StepEvent::StringPayloadByte
                    }
                    _ => StepEvent::StringPayloadByte,
                }
            }

            SmState::SosPmApcString => {
                self.sequence_bytes_seen = self.sequence_bytes_seen.saturating_add(1);
                match byte {
                    0x1b => {
                        self.state = SmState::SosPmApcEsc;
                        StepEvent::StringPayloadByte
                    }
                    _ => StepEvent::StringPayloadByte,
                }
            }

            // DcsEsc and SosPmApcEsc share identical ST-peek logic: \\ closes
            // the string, lone ESC restarts Escape, anything else aborts and
            // re-routes the byte through Escape.
            SmState::DcsEsc | SmState::SosPmApcEsc => {
                self.sequence_bytes_seen = self.sequence_bytes_seen.saturating_add(1);
                match byte {
                    b'\\' => {
                        self.state = SmState::Ground;
                        self.sequence_bytes_seen = 0;
                        StepEvent::StringPayloadByte
                    }
                    0x1b => {
                        self.state = SmState::Escape;
                        self.sequence_bytes_seen = 1;
                        StepEvent::SequenceByte
                    }
                    _ => {
                        self.state = SmState::Escape;
                        self.accum = 0;
                        self.private_mode = false;
                        self.sequence_bytes_seen = 1;
                        self.step(byte)
                    }
                }
            }
        }
    }

    fn finalize_csi(&mut self, byte: u8) -> StepEvent {
        let kind = match byte {
            b'h' if self.private_mode => {
                let bit = flag_for_mode(self.accum);
                if bit != 0 {
                    self.flags |= bit;
                    SequenceKind::TuiToggleOn
                } else {
                    SequenceKind::OtherCsi
                }
            }
            b'l' if self.private_mode => {
                let bit = flag_for_mode(self.accum);
                if bit != 0 {
                    self.flags &= !bit;
                    SequenceKind::TuiToggleOff
                } else {
                    SequenceKind::OtherCsi
                }
            }
            b'm' => SequenceKind::Sgr,
            _ => SequenceKind::OtherCsi,
        };
        self.state = SmState::Ground;
        self.private_mode = false;
        self.accum = 0;
        self.sequence_bytes_seen = 0;
        StepEvent::SequenceCompleted(kind)
    }
}

#[cfg(test)]
mod ansi_tests {
    use super::*;

    fn step_all(sm: &mut AnsiSm, bytes: &[u8]) -> Vec<StepEvent> {
        bytes.iter().map(|&b| sm.step(b)).collect()
    }

    #[test]
    fn new_sm_starts_in_ground_with_no_flags() {
        let sm = AnsiSm::new();
        assert!(!sm.tui_mode_active());
    }

    #[test]
    fn flag_for_mode_classifies_known_modes() {
        assert_eq!(flag_for_mode(1049), tui_flags::ALT_SCREEN);
        assert_eq!(flag_for_mode(47), tui_flags::ALT_SCREEN);
        assert_eq!(flag_for_mode(1047), tui_flags::ALT_SCREEN);
        assert_eq!(flag_for_mode(2004), tui_flags::BRACKETED_PASTE);
        assert_eq!(flag_for_mode(1000), tui_flags::MOUSE);
        assert_eq!(flag_for_mode(1002), tui_flags::MOUSE);
        assert_eq!(flag_for_mode(1003), tui_flags::MOUSE);
        assert_eq!(flag_for_mode(1006), tui_flags::MOUSE);
        assert_eq!(flag_for_mode(0), 0);
        assert_eq!(flag_for_mode(99999), 0);
    }

    #[test]
    fn ground_data_byte_emits_data_event() {
        let mut sm = AnsiSm::new();
        assert_eq!(sm.step(b'a'), StepEvent::Data);
        assert_eq!(sm.step(b'\n'), StepEvent::Data);
    }

    #[test]
    fn esc_in_ground_emits_sequence_byte_and_transitions() {
        let mut sm = AnsiSm::new();
        assert_eq!(sm.step(0x1b), StepEvent::SequenceByte);
    }

    #[test]
    fn alt_screen_enters_on_1049h() {
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1b[?1049h");
        let last = events.last().expect("events");
        assert!(
            matches!(last, StepEvent::SequenceCompleted(SequenceKind::TuiToggleOn)),
            "expected TuiToggleOn, got {last:?}"
        );
        assert!(sm.tui_mode_active());
    }

    #[test]
    fn alt_screen_exits_on_1049l() {
        let mut sm = AnsiSm::new();
        let _ = step_all(&mut sm, b"\x1b[?1049h");
        assert!(sm.tui_mode_active());
        let events = step_all(&mut sm, b"\x1b[?1049l");
        let last = events.last().expect("events");
        assert!(
            matches!(last, StepEvent::SequenceCompleted(SequenceKind::TuiToggleOff)),
            "expected TuiToggleOff, got {last:?}"
        );
        assert!(!sm.tui_mode_active());
    }

    #[test]
    fn accepts_legacy_alt_screen_variants() {
        for mode in [b"47".as_slice(), b"1047"] {
            let mut sm = AnsiSm::new();
            let mut seq = vec![0x1b, b'[', b'?'];
            seq.extend_from_slice(mode);
            seq.push(b'h');
            let events = step_all(&mut sm, &seq);
            let last = events.last().expect("events");
            assert!(
                matches!(last, StepEvent::SequenceCompleted(SequenceKind::TuiToggleOn)),
                "mode {mode:?}: expected TuiToggleOn, got {last:?}"
            );
            assert!(sm.tui_mode_active(), "mode {mode:?}: should be active");
        }
    }

    #[test]
    fn bracketed_paste_enters_on_2004h() {
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1b[?2004h");
        assert!(matches!(
            events.last(),
            Some(StepEvent::SequenceCompleted(SequenceKind::TuiToggleOn))
        ));
        assert!(sm.tui_mode_active());
    }

    #[test]
    fn mouse_tracking_enters_on_1000h_1002h_1003h_1006h() {
        for mode in [b"1000".as_slice(), b"1002", b"1003", b"1006"] {
            let mut sm = AnsiSm::new();
            let mut seq = vec![0x1b, b'[', b'?'];
            seq.extend_from_slice(mode);
            seq.push(b'h');
            let events = step_all(&mut sm, &seq);
            assert!(matches!(
                events.last(),
                Some(StepEvent::SequenceCompleted(SequenceKind::TuiToggleOn))
            ));
        }
    }

    #[test]
    fn sgr_csi_emits_completed_sgr() {
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1b[31m");
        let last = events.last().expect("events");
        assert!(
            matches!(last, StepEvent::SequenceCompleted(SequenceKind::Sgr)),
            "expected Sgr, got {last:?}"
        );
        assert!(!sm.tui_mode_active());
    }

    #[test]
    fn sgr_with_default_reset_csi_m_alone() {
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1b[m");
        assert!(matches!(events.last(), Some(StepEvent::SequenceCompleted(SequenceKind::Sgr))));
    }

    #[test]
    fn non_sgr_csi_emits_other_csi() {
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1b[2A");
        let last = events.last().expect("events");
        assert!(
            matches!(last, StepEvent::SequenceCompleted(SequenceKind::OtherCsi)),
            "expected OtherCsi, got {last:?}"
        );
    }

    #[test]
    fn csi_with_unknown_private_mode_does_not_set_flag() {
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1b[?9999h");
        let last = events.last().expect("events");
        assert!(
            matches!(last, StepEvent::SequenceCompleted(SequenceKind::OtherCsi)),
            "expected OtherCsi for unknown private mode, got {last:?}"
        );
        assert!(!sm.tui_mode_active());
    }

    #[test]
    fn multi_param_csi_resets_accum_on_semicolon() {
        let mut sm = AnsiSm::new();
        // \e[?1049;1000h — alt-screen first, then mouse. Both private.
        // With accum reset on ';', the LAST param (1000) drives finalize.
        let events = step_all(&mut sm, b"\x1b[?1049;1000h");
        assert!(matches!(
            events.last(),
            Some(StepEvent::SequenceCompleted(SequenceKind::TuiToggleOn))
        ));
        // Mouse flag set (1000) because accum landed on 1000 at FINAL.
        assert!(sm.tui_mode_active());
    }

    #[test]
    fn dec_secondary_da_with_lt_prefix_does_not_set_flag() {
        // \e[>0c — DEC secondary device attributes. '<>=' are PRIVATE_PREFIX
        // per Williams; our flag_for_mode(0) returns 0, so this is OtherCsi.
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1b[>0c");
        assert!(matches!(
            events.last(),
            Some(StepEvent::SequenceCompleted(SequenceKind::OtherCsi))
        ));
        assert!(!sm.tui_mode_active());
    }

    #[test]
    fn csi_with_intermediate_byte_routes_through_csi_intermediate() {
        // \e[ q — DECSCUSR (cursor style). Intermediate ' ' (0x20), final 'q'.
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1b[ q");
        assert!(matches!(
            events.last(),
            Some(StepEvent::SequenceCompleted(SequenceKind::OtherCsi))
        ));
    }

    #[test]
    fn esc_equals_emits_esc_final() {
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1b=");
        assert!(
            matches!(events.last(), Some(StepEvent::SequenceCompleted(SequenceKind::EscFinal))),
            "expected EscFinal for \\e=, got {:?}",
            events.last()
        );
    }

    #[test]
    fn esc_m_ri_emits_esc_final() {
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1bM");
        assert!(matches!(
            events.last(),
            Some(StepEvent::SequenceCompleted(SequenceKind::EscFinal))
        ));
    }

    #[test]
    fn esc_7_decsc_emits_esc_final() {
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1b7");
        assert!(matches!(
            events.last(),
            Some(StepEvent::SequenceCompleted(SequenceKind::EscFinal))
        ));
    }

    #[test]
    fn esc_8_decrc_emits_esc_final() {
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1b8");
        assert!(matches!(
            events.last(),
            Some(StepEvent::SequenceCompleted(SequenceKind::EscFinal))
        ));
    }

    #[test]
    fn esc_c_ris_emits_esc_final() {
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1bc");
        assert!(matches!(
            events.last(),
            Some(StepEvent::SequenceCompleted(SequenceKind::EscFinal))
        ));
    }

    #[test]
    fn esc_paren_b_g0_designate_emits_esc_intermediate_final() {
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1b(B");
        assert!(
            matches!(
                events.last(),
                Some(StepEvent::SequenceCompleted(SequenceKind::EscIntermediateFinal))
            ),
            "expected EscIntermediateFinal for \\e(B, got {:?}",
            events.last()
        );
    }

    #[test]
    fn esc_hash_8_dec_alignment_test() {
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1b#8");
        assert!(matches!(
            events.last(),
            Some(StepEvent::SequenceCompleted(SequenceKind::EscIntermediateFinal))
        ));
    }

    #[test]
    fn esc_intermediate_chain() {
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1b $@");
        assert!(matches!(
            events.last(),
            Some(StepEvent::SequenceCompleted(SequenceKind::EscIntermediateFinal))
        ));
    }

    #[test]
    fn osc_2_title_emits_payload_events_until_bel() {
        let mut sm = AnsiSm::new();
        // \e ] 2 ; t i t l e \a — 10 bytes.
        let events = step_all(&mut sm, b"\x1b]2;title\x07");
        assert_eq!(events.len(), 10);
        // First byte (\e) is SequenceByte; second (]) is StringPayloadByte;
        // all subsequent up to and including \a are StringPayloadByte.
        assert_eq!(events[0], StepEvent::SequenceByte);
        for (i, e) in events.iter().enumerate().skip(1) {
            assert!(
                matches!(e, StepEvent::StringPayloadByte),
                "byte {i}: expected StringPayloadByte, got {e:?}"
            );
        }
    }

    #[test]
    fn osc_8_hyperlink_through_two_sequences() {
        let mut sm = AnsiSm::new();
        let bytes = b"\x1b]8;;https://example.com\x07click\x1b]8;;\x07";
        let events = step_all(&mut sm, bytes);
        // The "click" word between the two OSCs should emit Data events.
        let click_start = bytes.windows(5).position(|w| w == b"click").expect("click substring");
        for i in click_start..click_start + 5 {
            assert_eq!(
                events[i],
                StepEvent::Data,
                "byte {i} ({:?}) expected Data",
                bytes[i] as char
            );
        }
    }

    #[test]
    fn osc_with_st_terminator() {
        let mut sm = AnsiSm::new();
        // \e]0;foo\e\\ — 9 bytes including ST.
        let events = step_all(&mut sm, b"\x1b]0;foo\x1b\\");
        let last = events.last().expect("events");
        assert_eq!(*last, StepEvent::StringPayloadByte);
        let after = sm.step(b'X');
        assert_eq!(after, StepEvent::Data);
    }

    #[test]
    fn osc_with_bel_terminator_ends_payload() {
        let mut sm = AnsiSm::new();
        let _ = step_all(&mut sm, b"\x1b]2;t\x07");
        let after = sm.step(b'a');
        assert_eq!(after, StepEvent::Data);
    }

    #[test]
    fn osc_133_prompt_marker_intact() {
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1b]133;A\x07");
        assert_eq!(events[0], StepEvent::SequenceByte);
        for e in &events[1..] {
            assert!(matches!(e, StepEvent::StringPayloadByte));
        }
    }

    #[test]
    fn osc_lone_esc_followed_by_non_backslash_aborts_string() {
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1b]2;abc\x1b[31m");
        let last = events.last().expect("events");
        assert!(
            matches!(last, StepEvent::SequenceCompleted(SequenceKind::Sgr)),
            "expected new CSI Sgr after OSC abort, got {last:?}"
        );
    }

    #[test]
    fn osc_with_embedded_newline_does_not_emit_data() {
        let mut sm = AnsiSm::new();
        let bytes = b"\x1b]52;c;base64\n=more\x07";
        let events = step_all(&mut sm, bytes);
        let nl_idx = bytes.iter().position(|&b| b == b'\n').expect("\\n");
        assert!(
            matches!(events[nl_idx], StepEvent::StringPayloadByte),
            "embedded \\n inside OSC must be StringPayloadByte, got {:?}",
            events[nl_idx]
        );
    }

    #[test]
    fn dcs_passthrough_until_st() {
        let mut sm = AnsiSm::new();
        // \eP$qm\e\\ — DCS query.
        let events = step_all(&mut sm, b"\x1bP$qm\x1b\\");
        // After ST, next byte should be Data.
        let after = sm.step(b'X');
        assert_eq!(after, StepEvent::Data);
        // First byte (\e) is SequenceByte; second (P) is StringPayloadByte.
        assert_eq!(events[0], StepEvent::SequenceByte);
        assert_eq!(events[1], StepEvent::StringPayloadByte);
    }

    #[test]
    fn dcs_lone_esc_aborts_and_resyncs() {
        let mut sm = AnsiSm::new();
        // \eP$q\e[31m — DCS with embedded lone ESC aborts; \e[31m starts new CSI.
        let events = step_all(&mut sm, b"\x1bP$q\x1b[31m");
        let last = events.last().expect("events");
        assert!(
            matches!(last, StepEvent::SequenceCompleted(SequenceKind::Sgr)),
            "expected new CSI Sgr after DCS abort, got {last:?}"
        );
    }

    #[test]
    fn pm_passthrough() {
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1b^private\x1b\\");
        let after = sm.step(b'X');
        assert_eq!(after, StepEvent::Data);
        assert_eq!(events[1], StepEvent::StringPayloadByte);
    }

    #[test]
    fn apc_passthrough() {
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1b_payload\x1b\\");
        let after = sm.step(b'X');
        assert_eq!(after, StepEvent::Data);
        assert_eq!(events[1], StepEvent::StringPayloadByte);
    }

    #[test]
    fn sos_passthrough() {
        let mut sm = AnsiSm::new();
        // \eX is SOS (Start of String) — same handling as PM/APC in our impl.
        let events = step_all(&mut sm, b"\x1bXpayload\x1b\\");
        let after = sm.step(b'X');
        assert_eq!(after, StepEvent::Data);
        assert_eq!(events[1], StepEvent::StringPayloadByte);
    }

    #[test]
    fn accum_overflow_does_not_corrupt_state() {
        let mut sm = AnsiSm::new();
        // \e[?<40 nines>h
        let mut seq = vec![0x1b, b'[', b'?'];
        seq.extend(std::iter::repeat(b'9').take(40));
        seq.push(b'h');
        let events = step_all(&mut sm, &seq);
        assert!(matches!(
            events.last(),
            Some(StepEvent::SequenceCompleted(SequenceKind::OtherCsi))
        ));
        assert!(!sm.tui_mode_active());
    }

    #[test]
    fn interleaved_esc_resyncs_correctly() {
        let mut sm = AnsiSm::new();
        // \e\e[?1049h — double ESC at start; second \e re-initiates.
        let events = step_all(&mut sm, b"\x1b\x1b[?1049h");
        let last = events.last().expect("events");
        assert!(matches!(last, StepEvent::SequenceCompleted(SequenceKind::TuiToggleOn)));
        assert!(sm.tui_mode_active());
    }

    #[test]
    fn interrupted_csi_at_eof_does_not_corrupt() {
        let mut sm = AnsiSm::new();
        let _ = step_all(&mut sm, b"\x1b[?1049");
        assert!(!sm.tui_mode_active());
        // Next byte: send something that wouldn't be a CSI final.
        let _ = sm.step(b'\n');
        // Just verify no panic / state corruption.
    }

    #[test]
    fn eight_bit_c1_st_not_recognized() {
        // \e]2;foo\x9c — 8-bit C1 ST per Karar 6 treated as data, not ST.
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1b]2;foo\x9c");
        let last = events.last().expect("events");
        assert!(matches!(last, StepEvent::StringPayloadByte));
        // Send BEL to terminate the OSC properly.
        let after_bel = sm.step(0x07);
        assert_eq!(after_bel, StepEvent::StringPayloadByte);
        let after_ground = sm.step(b'X');
        assert_eq!(after_ground, StepEvent::Data);
    }

    #[test]
    fn scratch_cap_exceeded_drops_sequence() {
        let mut sm = AnsiSm::new();
        sm.step(0x1b);
        sm.step(b'[');
        for _ in 0..5000 {
            let _ = sm.step(b'9');
        }
        // After cap exceeded, SM should have dropped back to Ground.
        // Probe with a Data byte.
        let probe = sm.step(b'A');
        assert_eq!(
            probe,
            StepEvent::Data,
            "expected Ground state after cap exceeded, got {probe:?}"
        );
    }

    #[test]
    fn osc_no_terminator_then_eof_keeps_payload_event_stream() {
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"\x1b]2;title");
        // Last byte should emit StringPayloadByte — payload passes through
        // even though sequence is unterminated. Drain is Pipeline's job.
        assert_eq!(events.last(), Some(&StepEvent::StringPayloadByte));
    }
}
