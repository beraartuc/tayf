//! tayf CLI entry. Parses args, runs the facade, and maps errors to exit codes.
//!
//! Exit-code policy (BSD `sysexits.h`):
//! - 0 (`EX_OK`) — success, including `--help` and `--version` output.
//! - 64 (`EX_USAGE`) — CLI parse failure (unknown flag, bad value, etc.),
//!   or a malformed `--config` / `~/.config/tayf/config.toml`.
//! - 70 (`EX_SOFTWARE`) — internal programming error (regex compile, buffer
//!   overflow surfaced as Result, embedded profile RegexCompile — a tayf
//!   library bug because the shipped `assets/profiles/*.toml` patterns
//!   failed to compile).
//! - 71 (`EX_OSERR`) — operating-system failure (PTY, TTY, signal, shell
//!   discovery).
//! - Otherwise, the child shell's exit status, low byte.

use std::process::ExitCode;

use tayf::{Args, Cmd, ConfigAction, Error, Tayf};

fn main() -> ExitCode {
    match Args::try_parse_from_env() {
        Ok(args) => match args.cmd {
            None => match Tayf::run(args.run) {
                Ok(code) => code,
                Err(err) => {
                    eprintln!("tayf: {err}");
                    ExitCode::from(map_error_to_exit_code(&err))
                }
            },
            // Subcommand dispatch — non-PTY code paths.
            // v0.5.4 — stubs in `tayf::config_tui` are filled by
            // Phase B (dump/status) and Phase C (run) tasks.
            Some(Cmd::Config(cfg)) => match cfg.action {
                None => tayf::config_tui::run(args.run),
                Some(ConfigAction::Dump(d)) => tayf::config_tui::dump(d.kind),
                Some(ConfigAction::Status) => tayf::config_tui::status(args.run),
                // reason: ConfigAction is #[non_exhaustive] for additive
                // forward compat (future `tayf config new-profile` etc.).
                // Compile-time exhaustiveness requires this catch-all in
                // downstream consumers — including this binary.
                Some(_) => {
                    eprintln!("tayf config: unknown sub-subcommand");
                    ExitCode::from(64) // EX_USAGE
                }
            },
            Some(Cmd::Init(init_args)) => tayf::init::run(init_args),
            // Same forward-compat catch-all for Cmd::* additions.
            Some(_) => {
                eprintln!("tayf: unknown subcommand");
                ExitCode::from(64)
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
        Error::Config { .. } => 64,          // EX_USAGE
        Error::Theme { .. } => 64,           // EX_USAGE — unknown --theme value
        Error::ThemeValidation { .. } => 64, // EX_USAGE — disk/preset theme validation
        // Embedded profile RegexCompile is a tayf library bug — pattern
        // shipped in `assets/profiles/*.toml` failed to compile. Maps to
        // EX_SOFTWARE 70 (internal programming error). Discriminator:
        // source_path begins with `<embedded:profile/` (v0.5.2 LoadedProfile
        // path_label convention). Note: only RegexCompile splits. Other
        // ProfileErrorKind variants (ParseError, NotFound, etc.) on
        // embedded paths are practically unreachable (we ship the files)
        // and remain EX_USAGE for the failsafe path.
        Error::Profile {
            kind: tayf::ProfileErrorKind::RegexCompile { .. }, source_path, ..
        } if source_path.starts_with("<embedded:profile/") => 70, // EX_SOFTWARE
        Error::Profile { .. } => 64, // EX_USAGE — disk profile / unknown name / etc.
        Error::ProfileValidation { .. } => 64, // EX_USAGE — profile body validation
        // reason: `Error` is `#[non_exhaustive]` for forward compat; future
        // variants default to EX_SOFTWARE until explicitly mapped. New
        // variants SHOULD be added above this arm with the appropriate code.
        _ => 70,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_validation_maps_to_ex_usage() {
        let err = Error::ThemeValidation {
            theme: "x".into(),
            source_path: "<x>".into(),
            errors: vec![tayf::ThemeRuleError {
                rule_name: "a".into(),
                kind: tayf::ThemeRuleErrorKind::UnknownName,
            }],
        };
        assert_eq!(
            map_error_to_exit_code(&err),
            64,
            "ThemeValidation is a user-input error; must map to EX_USAGE",
        );
    }

    #[test]
    fn profile_maps_to_ex_usage() {
        let err = Error::Profile {
            name: "test".into(),
            source_path: "<test>".into(),
            kind: tayf::ProfileErrorKind::NotFound { searched: Vec::new() },
        };
        assert_eq!(
            map_error_to_exit_code(&err),
            64,
            "Profile is a user-input error; must map to EX_USAGE",
        );
    }

    #[test]
    fn profile_validation_maps_to_ex_usage() {
        let err = Error::ProfileValidation {
            profile: "test".into(),
            source_path: "<test>".into(),
            errors: vec![tayf::ProfileRuleError {
                rule_name: "foo".into(),
                kind: tayf::ProfileRuleErrorKind::RuleNameInvalid { name: "foo bar".into() },
            }],
        };
        assert_eq!(
            map_error_to_exit_code(&err),
            64,
            "ProfileValidation is a user-input error; must map to EX_USAGE",
        );
    }

    // v0.5.3 — Profile/RegexCompile exit-code split (§7.4).

    #[test]
    fn embedded_profile_regex_compile_maps_to_ex_software() {
        let err = Error::Profile {
            name: "aws".to_owned(),
            source_path: "<embedded:profile/aws>".to_owned(),
            kind: tayf::ProfileErrorKind::RegexCompile {
                rule_name: "synthetic".to_owned(),
                pattern: "(".to_owned(),
                message: "unbalanced (".to_owned(),
            },
        };
        assert_eq!(
            map_error_to_exit_code(&err),
            70,
            "Embedded profile RegexCompile is a tayf library bug; must map to EX_SOFTWARE 70",
        );
    }

    #[test]
    fn disk_profile_regex_compile_maps_to_ex_usage() {
        let err = Error::Profile {
            name: "myaws".to_owned(),
            source_path: "/home/user/.config/tayf/profiles/myaws.toml".to_owned(),
            kind: tayf::ProfileErrorKind::RegexCompile {
                rule_name: "synthetic".to_owned(),
                pattern: "(".to_owned(),
                message: "unbalanced (".to_owned(),
            },
        };
        assert_eq!(
            map_error_to_exit_code(&err),
            64,
            "Disk profile RegexCompile is a user TOML error; must map to EX_USAGE 64",
        );
    }

    #[test]
    fn embedded_profile_parse_error_still_maps_to_ex_usage() {
        // Split is RegexCompile-specific. Other ProfileErrorKind variants
        // (ParseError, etc.) on embedded paths remain EX_USAGE failsafe.
        let err = Error::Profile {
            name: "aws".to_owned(),
            source_path: "<embedded:profile/aws>".to_owned(),
            kind: tayf::ProfileErrorKind::ParseError { message: "synthetic: bad TOML".to_owned() },
        };
        assert_eq!(map_error_to_exit_code(&err), 64);
    }
}
