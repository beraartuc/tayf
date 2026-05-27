//! 200 ms debouncer for live preview recompile.

use std::time::{Duration, Instant};

const DEBOUNCE_WINDOW: Duration = Duration::from_millis(200);

#[derive(Debug, Default)]
pub(crate) struct Debouncer {
    last_edit: Option<Instant>,
    pending: bool,
}

impl Debouncer {
    // reason: called by v0.6+ inline regex-source editor per spec §9.1;
    // v0.5.4 ships the Debouncer scaffold so `should_recompile` ticks
    // cleanly, but no debounce-triggering edits land until the inline
    // editor (currently a v0.6+ Toast::warn stub on the `e` key) wires.
    #[allow(dead_code)]
    pub(crate) fn mark_edit(&mut self) {
        self.last_edit = Some(Instant::now());
        self.pending = true;
    }

    /// Caller invokes after the main loop's 100 ms tick; returns true exactly
    /// once per quiescent-window expiry. Pending flag clears on consume.
    pub(crate) fn should_recompile(&mut self) -> bool {
        let Some(t) = self.last_edit else {
            return false;
        };
        if self.pending && t.elapsed() >= DEBOUNCE_WINDOW {
            self.pending = false;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn fresh_debouncer_does_not_recompile() {
        let mut d = Debouncer::default();
        assert!(!d.should_recompile());
    }

    #[test]
    fn mark_then_immediate_check_does_not_recompile() {
        let mut d = Debouncer::default();
        d.mark_edit();
        assert!(!d.should_recompile(), "200ms window not elapsed");
    }

    #[test]
    fn mark_then_wait_past_window_recompiles_once() {
        let mut d = Debouncer::default();
        d.mark_edit();
        thread::sleep(Duration::from_millis(250));
        assert!(d.should_recompile());
        assert!(!d.should_recompile(), "consume-once semantic; pending flag clears");
    }

    #[test]
    fn mark_resets_window() {
        let mut d = Debouncer::default();
        d.mark_edit();
        thread::sleep(Duration::from_millis(100));
        d.mark_edit();
        thread::sleep(Duration::from_millis(150));
        assert!(!d.should_recompile(), "second mark restarts window — should NOT fire yet");
        thread::sleep(Duration::from_millis(100));
        assert!(d.should_recompile());
    }
}
