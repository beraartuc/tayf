//! ANSI byte-stream state machine.
//!
//! Implements a 14-state subset of Paul Williams' VT500 ANSI parser
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

/// 14-state Williams VT500 subset. See spec §3.2 for the per-state meanings.
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
    /// Internal sequence-byte budget: refuse to accumulate sequences larger
    /// than this many bytes. Defends against malicious unterminated CSI/ESC
    /// inputs. See spec §7.1 (4 KiB cap rationale).
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
    /// Tasks 3-7 fill in the real transitions. Task 2 stubs this to always
    /// emit `Data` so the module compiles and tests can be authored against
    /// a known baseline.
    #[allow(clippy::unused_self)]
    // reason: Tasks 3-7 wire `step` to mutate `self.state`/`self.flags`/etc.;
    // keeping the `&mut self` signature now avoids churning every call site
    // in Pipeline (Task 8) when the real transitions land.
    pub(crate) fn step(&mut self, _byte: u8) -> StepEvent {
        StepEvent::Data
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
    fn step_all_helper_returns_events_in_order() {
        let mut sm = AnsiSm::new();
        let events = step_all(&mut sm, b"abc");
        assert_eq!(events.len(), 3);
    }
}
