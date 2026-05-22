//! Config-file watcher abstraction over `notify`.
//!
//! Spawns a debounce thread that coalesces a burst of `notify`
//! events into a single [`crate::reload::ReloadRequest::FileChanged`]
//! per 200 ms quiescent window. The watcher itself runs on its own
//! thread (`tayf-watch`); the debounce loop runs on `tayf-debounce`.
//!
//! See `docs/superpowers/specs/2026-05-22-tayf-v0.2.1-hot-reload.md` §3.1.

use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use notify::{RecommendedWatcher, Watcher};

use crate::error::{Error, Result};
use crate::reload::ReloadRequest;

/// Manual debounce quiescence window. 200 ms covers a typical
/// editor save sequence (vim swap → write → rename ≈ 100–150 ms)
/// without making interactive edits feel laggy.
const DEBOUNCE_WINDOW: Duration = Duration::from_millis(200);

/// Owns the watcher and the debounce thread. Drop closes both:
/// the watcher's OS resource (`Drop` on `RecommendedWatcher`) and
/// the debounce thread (channel close → loop exits → join).
pub(crate) struct ConfigWatcher {
    debounce_handle: Option<JoinHandle<()>>,
    // Held as Option so Drop can release the OS watcher BEFORE
    // joining the debounce thread. Without that ordering the
    // debounce thread sits on its `recv_timeout` (60s long-idle
    // branch) and `join()` blocks for the full duration. Wrapping in
    // Option lets us `take()` out of `&mut self`.
    watcher: Option<RecommendedWatcher>,
}

impl ConfigWatcher {
    /// Begin watching `path`. Bursts of events are debounced into a
    /// single `ReloadRequest::FileChanged` per quiescent window.
    ///
    /// # Errors
    /// Returns [`Error::Watch`] if notify cannot register the path
    /// (e.g., inotify watch limit exhausted, path does not exist),
    /// or if the debounce thread cannot be spawned.
    pub(crate) fn spawn(path: &Path, tx: Sender<ReloadRequest>) -> Result<Self> {
        Self::spawn_with_window(path, tx, DEBOUNCE_WINDOW)
    }

    /// Same as [`spawn`] but with a configurable debounce window.
    /// Test-only — production always uses [`DEBOUNCE_WINDOW`].
    pub(crate) fn spawn_with_window(
        path: &Path,
        tx: Sender<ReloadRequest>,
        window: Duration,
    ) -> Result<Self> {
        let (raw_tx, raw_rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |res| {
            // Best-effort: if the debounce thread already exited
            // (ConfigWatcher dropped), the send fails silently.
            let _ = raw_tx.send(res);
        })
        .map_err(Error::Watch)?;

        watcher.watch(path, notify::RecursiveMode::NonRecursive).map_err(Error::Watch)?;

        let debounce_handle = thread::Builder::new()
            .name("tayf-debounce".into())
            .spawn(move || debounce_loop(raw_rx, tx, window))
            .map_err(|e| Error::Watch(notify::Error::io(e)))?;

        Ok(ConfigWatcher { debounce_handle: Some(debounce_handle), watcher: Some(watcher) })
    }
}

impl Drop for ConfigWatcher {
    fn drop(&mut self) {
        // Drop order matters: release the OS watcher FIRST so its
        // internal raw_tx (held inside the callback closure) is
        // dropped. That closes the raw event channel; the debounce
        // thread's `recv_timeout` immediately returns `Disconnected`
        // and the loop exits. Then join. If we joined first, the
        // thread would sit on its 60s long-idle timeout while the
        // watcher is still live (no Disconnected signal yet), and
        // `join` would block for the full duration.
        self.watcher.take();
        if let Some(h) = self.debounce_handle.take() {
            let _ = h.join();
        }
    }
}

/// The debounce loop: collect raw notify events, emit one
/// `ReloadRequest::FileChanged` per quiescent window.
#[allow(clippy::needless_pass_by_value)]
// reason: this function is the body of the `tayf-debounce` thread.
// `raw_rx` and `tx` MUST be owned by the thread (they cannot be
// borrowed across `thread::spawn`). Clippy cannot see that the
// thread closure is what consumes them.
fn debounce_loop(
    raw_rx: mpsc::Receiver<notify::Result<notify::Event>>,
    tx: Sender<ReloadRequest>,
    window: Duration,
) {
    use std::sync::mpsc::RecvTimeoutError;

    let mut pending = false;
    loop {
        let timeout = if pending {
            window
        } else {
            // Long idle — bounded so that we eventually notice
            // a Disconnected if the watcher dropped without sending
            // any event since spawn.
            Duration::from_secs(60)
        };
        match raw_rx.recv_timeout(timeout) {
            Ok(event_or_err) => {
                // notify delivers `Result<Event>` to the callback.
                // Errors (events_dropped, must_rescan, path_not_found)
                // should still trigger a reload attempt — the watcher
                // typically needs reattention — but the operator
                // deserves a diagnostic when they happen.
                if let Err(e) = event_or_err {
                    crate::log::warn_msg!("watch error: {e}");
                }
                pending = true;
            }
            Err(RecvTimeoutError::Timeout) => {
                if pending {
                    // Quiescent — emit one event.
                    if tx.send(ReloadRequest::FileChanged).is_err() {
                        // Orchestrator dropped; nothing more to do.
                        break;
                    }
                    pending = false;
                }
                // Else: long idle, keep waiting.
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    /// Test-window choice: 150ms covers macOS `FSEvents` baseline latency
    /// (50–100ms) plus headroom. Production uses 200ms (`DEBOUNCE_WINDOW`).
    const TEST_DEBOUNCE_WINDOW: Duration = Duration::from_millis(150);

    /// Notify backends need a small warmup after `Watcher::watch` before
    /// they reliably deliver events for changes that happen on the same
    /// path. 100ms covers both macOS `FSEvents` (~50ms baseline) and slow
    /// inotify event-queue flushing on loaded CI runners.
    fn warmup_watcher() {
        std::thread::sleep(Duration::from_millis(100));
    }

    #[test]
    fn single_edit_emits_one_event_after_quiescence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "initial\n").unwrap();

        let (tx, rx) = mpsc::channel::<ReloadRequest>();
        let _watcher = ConfigWatcher::spawn_with_window(&path, tx, TEST_DEBOUNCE_WINDOW).unwrap();
        warmup_watcher();

        // Single write.
        fs::write(&path, "second\n").unwrap();

        let got = rx.recv_timeout(Duration::from_secs(2)).expect("event");
        assert_eq!(got, ReloadRequest::FileChanged);

        // At most one further event within a wide 400ms recheck window.
        let mut extras = 0;
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(400) {
            match rx.try_recv() {
                Ok(_) => extras += 1,
                Err(mpsc::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(mpsc::TryRecvError::Disconnected) => break,
            }
        }
        assert!(extras <= 1, "single write produced too many debounced events: {extras} extra");
    }

    #[test]
    fn burst_of_edits_coalesces_to_bounded_event_count() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "initial\n").unwrap();

        let (tx, rx) = mpsc::channel::<ReloadRequest>();
        let _watcher = ConfigWatcher::spawn_with_window(&path, tx, TEST_DEBOUNCE_WINDOW).unwrap();
        warmup_watcher();

        for i in 0..5 {
            fs::write(&path, format!("edit-{i}\n")).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }

        // 1 or 2 events total over a 1s window. > 2 is a real bug.
        let mut total = 0;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(1) {
            match rx.try_recv() {
                Ok(_) => total += 1,
                Err(mpsc::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(40));
                }
                Err(mpsc::TryRecvError::Disconnected) => break,
            }
        }
        assert!(
            (1..=2).contains(&total),
            "burst of 5 writes must coalesce to 1 or 2 events; got {total}"
        );
    }

    #[test]
    fn drop_stops_debounce_thread() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "initial\n").unwrap();

        let (tx, rx) = mpsc::channel::<ReloadRequest>();
        {
            let _watcher =
                ConfigWatcher::spawn_with_window(&path, tx, TEST_DEBOUNCE_WINDOW).unwrap();
            warmup_watcher();
        }
        // The orchestrator-side rx should now error (no more senders).
        assert!(rx.recv_timeout(Duration::from_millis(500)).is_err());
    }
}
