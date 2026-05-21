//! tayf — terminal-agnostic, PTY-based, regex-driven output colorizer.
//!
//! Public entry point: [`Tayf::run`]. See
//! `docs/superpowers/specs/2026-05-21-tayf-v0.1-design.md` for the full design.

// Crate-wide policy: unsafe is permitted only with a SAFETY comment;
// reviewer enforces. See src/pty.rs::current_term_size for the sole use
// in v0.1 (TIOCGWINSZ ioctl).
#![warn(unsafe_code)]
#![warn(clippy::pedantic, clippy::cargo)]
#![allow(clippy::module_name_repetitions, clippy::missing_errors_doc)]
#![allow(clippy::multiple_crate_versions)] // reason: unavoidable transitive dep duplicates in portable-pty + signal-hook (bitflags 1/2, nix 0.25/0.27)

pub mod cli;
pub mod error;
pub mod style;
pub mod version;

pub(crate) mod line_buffer;
pub(crate) mod logging;
pub(crate) mod pipeline;
pub(crate) mod pty;
pub(crate) mod rules;
pub(crate) mod runtime;
pub(crate) mod shell;
pub(crate) mod signals;
pub(crate) mod terminfo;
pub(crate) mod tty_guard;

pub use cli::Args;
pub use error::{Error, Result};

use std::process::ExitCode;

/// Top-level facade. Wires logging, shell discovery, TTY guard, PTY spawn,
/// signal handling, and the I/O loop into a single entry point.
pub struct Tayf;

impl Tayf {
    /// Run tayf with the given arguments. Returns the exit code that should
    /// be propagated to the OS.
    ///
    /// # Errors
    /// Returns any [`Error`] encountered during setup or execution. Failures
    /// after the TTY guard is engaged are still surfaced; the guard's Drop
    /// restores the terminal.
    #[allow(clippy::needless_pass_by_value)]
    // reason: `Args` is the parsed CLI surface and this is the process entry
    // point; taking ownership is the conventional shape even though we
    // currently only read individual fields.
    pub fn run(args: Args) -> Result<ExitCode> {
        logging::init();

        let apply_colors = !args.no_color && terminfo::stdout_is_tty();

        let spec = shell::discover(args.shell.as_deref(), args.login)?;

        let guard = tty_guard::TtyGuard::engage()?;

        let session = pty::PtySession::spawn(&spec)?;
        let (reader, writer, resizer, child) = session.into_parts()?;
        let child_pid = child.pid();

        let _signal_guard = signals::spawn_handler(resizer, child_pid)?;

        let rules = rules::Compiled::load_builtins()?;
        let exit_code = runtime::run(reader, writer, child, rules, apply_colors)?;

        drop(guard); // explicit; Drop would handle it but make ordering clear

        let code: u8 = u8::try_from(exit_code & 0xff).unwrap_or(1);
        Ok(ExitCode::from(code))
    }
}
