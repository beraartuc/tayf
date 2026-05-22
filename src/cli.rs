//! Command-line interface definition (clap derive).
//!
//! Defines the `tayf` binary's argument surface. `--version` routes through
//! `crate::version::version_string` so the SHA-and-rustc banner is shown.
//!
//! Public API:
//! - [`Args`] — parsed CLI arguments.
//! - [`Args::try_parse_from_env`] — fallible convenience wrapper invoked from
//!   `main`. Returning `Result` (rather than exiting internally) lets `main`
//!   own the exit-code policy: BSD `EX_USAGE` (64) on parse failure, 0 on
//!   `--help` / `--version`.

use std::path::PathBuf;
use std::sync::OnceLock;

use clap::Parser;

/// Returns the version banner as a `&'static str`.
///
/// clap's derive attribute requires a `&'static str` for `version`, but
/// [`crate::version::version_string`] returns a `String` because the banner is
/// composed at runtime from build-time metadata. Caching in a [`OnceLock`]
/// lets us promote the owned `String` to a `&'static str` exactly once.
fn version_str() -> &'static str {
    static V: OnceLock<String> = OnceLock::new();
    V.get_or_init(crate::version::version_string).as_str()
}

/// Terminal-agnostic, PTY-based, regex-driven output colorizer.
#[derive(Debug, Parser)]
#[command(
    name = "tayf",
    author,
    version = version_str(),
    about,
    long_about = None,
)]
pub struct Args {
    /// Override the shell to launch. Defaults to $SHELL, then /etc/passwd, then /bin/sh.
    #[arg(long, value_name = "PATH")]
    pub shell: Option<PathBuf>,

    /// Spawn the shell as a login shell (e.g. -zsh as `argv[0]`).
    #[arg(long, default_value_t = false)]
    pub login: bool,

    /// Disable colorization. Useful for debugging or when stdout is a TTY but
    /// you want raw passthrough.
    #[arg(long, default_value_t = false)]
    pub no_color: bool,

    /// Path to a TOML config file. Defaults to `$XDG_CONFIG_HOME/tayf/config.toml`,
    /// then `~/.config/tayf/config.toml`. When absent, tayf uses only the
    /// built-in rule set (v0.1 behavior).
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Apply a preset color theme before user-config rules. CLI override of
    /// `[general] theme`. Available themes are `dark` and `light`; unknown
    /// names exit with `EX_USAGE` (64) and a list of known themes.
    #[arg(long, value_name = "NAME")]
    pub theme: Option<String>,
}

impl Args {
    /// Try to parse arguments from the process's environment
    /// (`std::env::args_os`).
    ///
    /// Returns the [`clap::Error`] unchanged so the caller can decide how to
    /// surface it. `main` uses the error's [`clap::error::ErrorKind`] to map
    /// `--help` / `--version` to a success exit and every other parse failure
    /// to BSD `EX_USAGE` (64); clap's own `parse` would exit with code 2,
    /// which contradicts the v0.1 spec.
    ///
    /// # Errors
    /// Propagates any [`clap::Error`] raised by clap's parser, including the
    /// non-failure `DisplayHelp` / `DisplayVersion` kinds.
    pub fn try_parse_from_env() -> std::result::Result<Self, clap::Error> {
        <Self as Parser>::try_parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal() {
        let args = Args::try_parse_from(["tayf"]).unwrap();
        assert!(args.shell.is_none());
        assert!(!args.login);
        assert!(!args.no_color);
    }

    #[test]
    fn parses_all_flags() {
        let args = Args::try_parse_from([
            "tayf",
            "--shell",
            "/bin/fish",
            "--login",
            "--no-color",
            "--config",
            "/tmp/cfg.toml",
            "--theme",
            "dark",
        ])
        .unwrap();
        assert_eq!(args.shell.as_deref(), Some(std::path::Path::new("/bin/fish")));
        assert!(args.login);
        assert!(args.no_color);
        assert_eq!(args.config.as_deref(), Some(std::path::Path::new("/tmp/cfg.toml")));
        assert_eq!(args.theme.as_deref(), Some("dark"));
    }

    #[test]
    fn parses_config_flag() {
        let args = Args::try_parse_from(["tayf", "--config", "/tmp/cfg.toml"]).unwrap();
        assert_eq!(args.config.as_deref(), Some(std::path::Path::new("/tmp/cfg.toml")));
    }

    #[test]
    fn config_defaults_to_none() {
        let args = Args::try_parse_from(["tayf"]).unwrap();
        assert!(args.config.is_none());
    }

    #[test]
    fn parses_theme_flag() {
        let args = Args::try_parse_from(["tayf", "--theme", "light"]).unwrap();
        assert_eq!(args.theme.as_deref(), Some("light"));
    }

    #[test]
    fn theme_defaults_to_none() {
        let args = Args::try_parse_from(["tayf"]).unwrap();
        assert!(args.theme.is_none());
    }
}
