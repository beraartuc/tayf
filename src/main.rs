//! tayf CLI entry. Parses args, runs the facade, and maps errors to exit codes.
//!
//! Exit-code policy (BSD `sysexits.h`):
//! - 0 (`EX_OK`) — success, including `--help` and `--version` output.
//! - 64 (`EX_USAGE`) — CLI parse failure (unknown flag, bad value, etc.),
//!   or a malformed `--config` / `~/.config/tayf/config.toml`.
//! - 70 (`EX_SOFTWARE`) — internal programming error (regex compile, buffer
//!   overflow).
//! - 71 (`EX_OSERR`) — operating-system failure (PTY, TTY, signal, shell
//!   discovery).
//! - Otherwise, the child shell's exit status, low byte.

use std::process::ExitCode;

use tayf::{Args, Error, Tayf};

fn main() -> ExitCode {
    match Args::try_parse_from_env() {
        Ok(args) => match Tayf::run(args) {
            Ok(code) => code,
            Err(err) => {
                eprintln!("tayf: {err}");
                ExitCode::from(map_error_to_exit_code(&err))
            }
        },
        Err(e) => {
            // `print` writes to stdout for help/version and to stderr for
            // real errors; the exit-code mapping below mirrors that split.
            let _ = e.print();
            match e.kind() {
                clap::error::ErrorKind::DisplayHelp
                | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                | clap::error::ErrorKind::DisplayVersion => ExitCode::SUCCESS,
                _ => ExitCode::from(64), // EX_USAGE
            }
        }
    }
}

fn map_error_to_exit_code(err: &Error) -> u8 {
    match err {
        Error::ShellDiscovery(_) => 71, // EX_OSERR
        Error::Pty(_) => 71,
        Error::Tty(_) => 71,
        Error::Signal(_) => 71,
        Error::RegexCompile(_) => 70, // EX_SOFTWARE
        Error::BufferOverflow { .. } => 70,
        Error::Config { .. } => 64, // EX_USAGE
    }
}
