//! Cross-thread shutdown coordination for the PTY runtime.
//!
//! tayf has no single owner of the child's lifetime: the signal thread
//! observes terminal hangups and termination signals, the output thread
//! observes PTY EOF/EIO, and the input thread observes stdin EOF/EIO. Any of
//! those events means the session must end, even when the child shell ignores
//! `SIGTERM`/`SIGHUP` (interactive shells sometimes do).
//!
//! Each observing thread sends a [`ShutdownReason`] over this channel. The
//! main thread owns the [`ChildHandle`](crate::pty::ChildHandle), so it is the
//! only place that can escalate with `killpg(SIGKILL)` and reap the child.
//! Keeping the decision in one place avoids competing kill/wait calls from
//! signal and I/O threads.
//!
//! The channel is unbounded and best-effort: a send failure means the main
//! thread has already exited, in which case the sender's observation is moot.

use std::sync::mpsc::{self, Receiver, Sender};

/// Why the runtime should begin shutting down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShutdownReason {
    /// A termination signal was delivered to tayf. The signal thread has
    /// already forwarded it to the child process group; the main thread only
    /// needs to reap the child (and escalate if it ignores the signal).
    Signal(i32),
    /// stdin returned EOF/EIO or poll reported a hangup, e.g. because the
    /// terminal emulator window was closed.
    StdinClosed,
    /// The PTY master returned EOF/EIO: the child side closed its terminal
    /// (normal exit, or it dropped all slave fds while staying alive).
    PtyClosed,
}

/// Create the shutdown channel used by the signal and I/O threads.
pub(crate) fn channel() -> (Sender<ShutdownReason>, Receiver<ShutdownReason>) {
    mpsc::channel()
}
