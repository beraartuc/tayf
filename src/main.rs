//! tayf CLI entry. Parses args, runs the facade, and maps errors to exit codes.

use std::process::ExitCode;

use tayf::{Args, Error, Tayf};

fn main() -> ExitCode {
    let args = Args::parse_from_env();
    match Tayf::run(args) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("tayf: {err}");
            ExitCode::from(map_error_to_exit_code(&err))
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
    }
}
