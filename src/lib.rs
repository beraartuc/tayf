//! tayf — terminal-agnostic, PTY-based, regex-driven output colorizer.
//!
//! Public entry point: [`Tayf::run`]. See
//! `docs/superpowers/specs/2026-05-21-tayf-v0.1-design.md` for the full design.

// Crate-wide policy: unsafe is permitted only with a SAFETY comment;
// reviewer enforces. See `src/terminfo.rs::winsize` (TIOCGWINSZ ioctl) and
// `src/runtime.rs::borrow_master_fd` (PTY master fd borrow for `poll(2)`)
// for the only v0.1.1 use sites.
#![warn(unsafe_code)]
#![warn(clippy::pedantic, clippy::cargo)]
#![allow(clippy::module_name_repetitions, clippy::missing_errors_doc)]
#![allow(clippy::multiple_crate_versions)] // reason: unavoidable transitive dep duplicates in portable-pty + signal-hook (bitflags 1/2, nix 0.25/0.27)

pub(crate) mod cli;
pub(crate) mod error;
pub(crate) mod style;
pub(crate) mod version;

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

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // reason: exit_code & 0xff is structurally in 0..=255
        let code = (exit_code & 0xff) as u8;
        Ok(ExitCode::from(code))
    }
}

/// Bench-only adapters around `pub(crate)` internals so the `benches/`
/// crate (an external crate from rustc's perspective) can drive the hot
/// path directly.
///
/// Not part of the public API — hidden from rustdoc, no stability
/// guarantees, may change or vanish between minor releases. See
/// `benches/throughput.rs`.
#[doc(hidden)]
pub mod __bench__ {
    use std::io::Write;

    /// Opaque newtype carrying the compiled built-in rule set. Constructed
    /// via [`load_builtin_rules`] and passed back into [`apply_rules`].
    pub struct CompiledRules(crate::rules::Compiled);

    /// Compile the v0.1 built-in rule set (same path the production runtime
    /// uses). See `src/rules.rs::Compiled::load_builtins`.
    ///
    /// # Errors
    /// Returns [`crate::Error::RegexCompile`] if any built-in pattern fails
    /// to compile. In practice this never fires — the patterns are tested.
    pub fn load_builtin_rules() -> crate::Result<CompiledRules> {
        crate::rules::Compiled::load_builtins().map(CompiledRules)
    }

    /// Run the per-line rule scanner against `line`, emitting the SGR-wrapped
    /// output to `out`. Mirrors `src/pipeline.rs::apply_rules`.
    ///
    /// # Errors
    /// Forwards any `std::io::Error` produced by `out`.
    pub fn apply_rules<W: Write>(
        line: &[u8],
        rules: &CompiledRules,
        out: &mut W,
    ) -> std::io::Result<()> {
        crate::pipeline::apply_rules(line, &rules.0, out)
    }
}
