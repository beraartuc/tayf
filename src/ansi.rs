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
//! - **`StringPayloadByte`**: byte that is part of an OSC/DCS/PM/APC payload;
//!   goes direct to stdout. The `line_buffer` keeps any pre-existing partial
//!   line across the entire string sequence.
//! - **`SequenceCompleted(kind)`**: terminal event for a CSI/ESC sequence.
//!   `kind` tells `Pipeline` where the accumulated scratch goes.
//!
//! **Do NOT invent new states or transitions without consulting Williams'
//! reference.** This SM is a deliberate subset; adding a new sequence class
//! means walking Williams' table, identifying the relevant states, and
//! folding their transitions in. See spec §3.4 (the locked transition
//! table) before extending.

#![allow(dead_code)]
// reason: Tasks 3-7 wire `step` to emit each event variant; until then,
// the stubbed step only returns `Data`. Remove this allow when Task 7 lands.

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
    /// Implements the CSI path (Ground → Escape → `CsiEntry` → `CsiParam`
    /// → completion) plus `CsiIgnore` recovery. ESC direct finals,
    /// `EscapeIntermediate`, OSC/DCS/PM/APC paths are wired by Tasks 4-7;
    /// until then they return [`StepEvent::Data`] as a placeholder.
    #[allow(clippy::match_same_arms)]
    // reason: the `Ground` arm and the catch-all `_ => Data` placeholder for
    // Tasks 4-7 states coincidentally share a body; collapsing them would
    // hide the intent. The wildcard goes away in Task 4 onward.
    pub(crate) fn step(&mut self, byte: u8) -> StepEvent {
        if byte == 0x1b {
            self.state = SmState::Escape;
            self.accum = 0;
            self.private_mode = false;
            self.sequence_bytes_seen = 1;
            return StepEvent::SequenceByte;
        }

        match self.state {
            SmState::Ground => StepEvent::Data,

            SmState::Escape => {
                self.sequence_bytes_seen = self.sequence_bytes_seen.saturating_add(1);
                // Task 3 only routes `[` into CSI. Tasks 4-6 will add arms for
                // `]` (OSC), `P` (DCS), `X`/`^`/`_` (SOS/PM/APC), `(`/`#`
                // (EscIntermediate), and ESC direct finals. Keeping a match
                // here makes the eventual expansion a one-line addition.
                #[allow(clippy::single_match_else)]
                // reason: see comment above — Tasks 4-6 add more arms.
                match byte {
                    b'[' => {
                        self.state = SmState::CsiEntry;
                        StepEvent::SequenceByte
                    }
                    _ => {
                        self.state = SmState::Ground;
                        self.sequence_bytes_seen = 0;
                        StepEvent::SequenceByte
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
                    b'?' => {
                        self.private_mode = true;
                        self.state = SmState::CsiParam;
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
                    b';' => StepEvent::SequenceByte,
                    0x40..=0x7E => self.finalize_csi(byte),
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

            // Tasks 4-7 will wire these states. Task 3 stub: emit Data.
            _ => StepEvent::Data,
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
}
