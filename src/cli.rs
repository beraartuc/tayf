//! Command-line interface definition (clap derive).
//!
//! Defines the `tayf` binary's argument surface. `--version` routes through
//! `crate::version::version_string` so the SHA-and-rustc banner is shown.
//!
//! Public API:
//! - [`Args`] — top-level parsed CLI arguments (root flags + subcommand).
//! - [`RunArgs`] — PTY-wrapper flag bag, flatten'd at the root of `Args`.
//! - [`Cmd`] — top-level subcommand enum (only `Config` for v0.5.4).
//! - [`ConfigArgs`] / [`ConfigAction`] / [`DumpArgs`] / [`DumpKind`] —
//!   `tayf config …` sub-subcommand surface.
//! - [`Args::try_parse_from_env`] — fallible convenience wrapper invoked
//!   from `main`. Returning `Result` (rather than exiting internally) lets
//!   `main` own the exit-code policy: BSD `EX_USAGE` (64) on parse failure,
//!   0 on `--help` / `--version`.
//!
//! v0.5.4 — `Args` rename is an acknowledged public-API break
//! (see `docs/superpowers/specs/2026-05-26-tayf-v0.5.4-config-tui.md` §4.3):
//! pre-v0.5.4 `args.shell` is now `args.run.shell`. CHANGELOG carries a
//! `### Changed (breaking)` entry. Pre-1.0, no accessor-shim layer.

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
///
/// Subcommands:
/// - (none) — run the PTY wrapper with [`RunArgs`].
/// - `config` — interactive TUI / dump / status (see [`Cmd::Config`]).
#[derive(Debug, Parser)]
#[command(
    name = "tayf",
    author,
    version = version_str(),
    about,
    long_about = None,
)]
#[non_exhaustive]
pub struct Args {
    /// When no subcommand is given, run the PTY wrapper with these flags.
    ///
    /// Flatten'd at the root so backward-compat invocation forms
    /// (`tayf --shell /bin/fish`) work byte-identical to v0.5.3.
    #[command(flatten)]
    pub run: RunArgs,

    /// Optional subcommand. When `None`, [`Self::run`] determines the
    /// PTY wrapper behavior. When `Some`, the subcommand dispatches
    /// to a non-PTY code path (TUI, dump, status).
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

/// PTY-wrapper arguments. v0.5.3 `Args` field set, lifted out so a
/// future subcommand may flatten them in (currently only the no-subcommand
/// path consumes them).
///
/// `#[non_exhaustive]` — see [`Args`].
#[derive(Debug, clap::Args)]
#[non_exhaustive]
// reason: CLI argument structs are a flat collection of independent toggles —
// each bool maps 1:1 to a user-visible `--flag`, so a state machine or enum
// would obscure the surface rather than clarify it.
#[allow(clippy::struct_excessive_bools)]
pub struct RunArgs {
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

    /// Apply a preset color theme before user-config rules. CLI override
    /// of `[general] theme`. Built-in themes: `classic`, `dark`, `light`. Disk
    /// themes loaded from `<config_base>/themes/<name>.toml` are also
    /// accepted (`$XDG_CONFIG_HOME/tayf/themes/` or
    /// `$HOME/.config/tayf/themes/`). Unknown names, built-in name
    /// collisions, and theme validation errors all exit `EX_USAGE` (64).
    #[arg(long, value_name = "NAME")]
    pub theme: Option<String>,

    /// Apply a named profile. Loaded from
    /// `~/.config/tayf/profiles/<NAME>.toml` (disk) or from embedded
    /// sources (`aws`, `k8s`, `docker`, `gcp`, `network` ship in v0.5.3).
    /// Overrides any `[general] profile` value in the user config.
    /// Single profile only — composition deferred to a future release
    /// via a separate flag.
    #[arg(long, value_name = "NAME")]
    pub profile: Option<String>,

    /// Disable all of tayf's colorization, pattern matching, and background
    /// detection — passthrough the shell's output byte-for-byte while still
    /// wrapping the PTY and forwarding signals. Equivalent to `TAYF_DISABLE=1`
    /// (CLI flag wins on precedence).
    #[arg(long, default_value_t = false)]
    pub bypass: bool,

    /// Disable hot config reloading. The file watcher and reload orchestrator
    /// threads are not spawned. `SIGHUP` is still forwarded to the child
    /// process group (a behavior change from v0.2.1 — see CHANGELOG).
    #[arg(long, default_value_t = false)]
    pub no_hot_reload: bool,
}

/// Top-level subcommand dispatch.
#[derive(Debug, clap::Subcommand)]
#[non_exhaustive]
pub enum Cmd {
    /// Interactive TUI for browsing and editing tayf config; also `dump`
    /// and `status` sub-subcommands.
    Config(ConfigArgs),
}

/// `config` subcommand arguments.
#[derive(Debug, clap::Args)]
#[non_exhaustive]
pub struct ConfigArgs {
    /// Sub-subcommand. `None` launches the interactive TUI.
    #[command(subcommand)]
    pub action: Option<ConfigAction>,
}

/// `config` sub-subcommand variants. `None` of [`ConfigArgs::action`]
/// runs the interactive TUI.
#[derive(Debug, clap::Subcommand)]
#[non_exhaustive]
pub enum ConfigAction {
    /// Write the built-in pattern/theme/profile catalog to stdout as TOML.
    Dump(DumpArgs),
    /// Print resolved config state + hot-reload event log tail.
    Status,
}

/// `config dump` flags.
#[derive(Debug, clap::Args)]
pub struct DumpArgs {
    /// Restrict dump to one section (default: all of patterns / themes / profiles).
    #[arg(long, value_enum)]
    pub kind: Option<DumpKind>,
}

/// `config dump --kind` choices.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum DumpKind {
    /// Built-in pattern catalog only.
    Patterns,
    /// Built-in theme catalog only.
    Themes,
    /// Embedded profile catalog only.
    Profiles,
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
        assert!(args.run.shell.is_none());
        assert!(!args.run.login);
        assert!(!args.run.no_color);
        assert!(args.cmd.is_none(), "no subcommand → cmd = None");
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
            "--bypass",
            "--no-hot-reload",
        ])
        .unwrap();
        assert_eq!(args.run.shell.as_deref(), Some(std::path::Path::new("/bin/fish")));
        assert!(args.run.login);
        assert!(args.run.no_color);
        assert_eq!(args.run.config.as_deref(), Some(std::path::Path::new("/tmp/cfg.toml")));
        assert_eq!(args.run.theme.as_deref(), Some("dark"));
        assert!(args.run.bypass);
        assert!(args.run.no_hot_reload);
    }

    #[test]
    fn parses_config_flag() {
        let args = Args::try_parse_from(["tayf", "--config", "/tmp/cfg.toml"]).unwrap();
        assert_eq!(args.run.config.as_deref(), Some(std::path::Path::new("/tmp/cfg.toml")));
    }

    #[test]
    fn config_defaults_to_none() {
        let args = Args::try_parse_from(["tayf"]).unwrap();
        assert!(args.run.config.is_none());
    }

    #[test]
    fn parses_theme_flag() {
        let args = Args::try_parse_from(["tayf", "--theme", "light"]).unwrap();
        assert_eq!(args.run.theme.as_deref(), Some("light"));
    }

    #[test]
    fn theme_defaults_to_none() {
        let args = Args::try_parse_from(["tayf"]).unwrap();
        assert!(args.run.theme.is_none());
    }

    #[test]
    fn parses_bypass_flag() {
        let args = Args::try_parse_from(["tayf", "--bypass"]).unwrap();
        assert!(args.run.bypass);
        assert!(!args.run.no_hot_reload);
    }

    #[test]
    fn parses_no_hot_reload_flag() {
        let args = Args::try_parse_from(["tayf", "--no-hot-reload"]).unwrap();
        assert!(args.run.no_hot_reload);
        assert!(!args.run.bypass);
    }

    #[test]
    fn bypass_and_no_hot_reload_default_to_false() {
        let args = Args::try_parse_from(["tayf"]).unwrap();
        assert!(!args.run.bypass);
        assert!(!args.run.no_hot_reload);
    }

    #[test]
    fn parses_combined_bypass_and_no_hot_reload() {
        let args = Args::try_parse_from(["tayf", "--bypass", "--no-hot-reload"]).unwrap();
        assert!(args.run.bypass);
        assert!(args.run.no_hot_reload);
    }

    #[test]
    fn cli_profile_arg_parses_as_option_string() {
        use clap::Parser;

        // Sub-assertion 1: --profile foo → Some("foo")
        let a = Args::parse_from(["tayf", "--profile", "foo"]);
        assert_eq!(a.run.profile.as_deref(), Some("foo"));

        // Sub-assertion 2: omit --profile → None
        let a = Args::parse_from(["tayf"]);
        assert_eq!(a.run.profile, None);

        // Sub-assertion 3: duplicate --profile → clap error with byte-pinned
        // wording (clap's standard duplicate-flag message). Because the
        // arg carries `value_name = "NAME"`, clap interpolates the value
        // placeholder into the diagnostic: `'--profile <NAME>'`.
        let res = Args::try_parse_from(["tayf", "--profile", "foo", "--profile", "bar"]);
        let err = res.expect_err("duplicate --profile must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("the argument '--profile <NAME>' cannot be used multiple times"),
            "expected clap's duplicate-flag wording; got: {msg}"
        );
    }

    // v0.5.4 — Args::run.* field migration + Cmd subcommand parsing.

    #[test]
    fn args_field_path_migrated_to_run_subfield() {
        // v0.5.4 — Args::shell etc. moved to Args::run.shell (RunArgs
        // flatten). This test pins the new shape and would fail-to-compile
        // on the pre-v0.5.4 Args.
        let args = Args::try_parse_from(["tayf", "--shell", "/bin/fish"]).unwrap();
        assert_eq!(args.run.shell.as_deref(), Some(std::path::Path::new("/bin/fish")));
        assert!(args.cmd.is_none(), "no subcommand → cmd = None");
    }

    #[test]
    fn parses_config_subcommand_no_action() {
        let args = Args::try_parse_from(["tayf", "config"]).unwrap();
        assert!(matches!(args.cmd, Some(Cmd::Config(ConfigArgs { action: None }))));
    }

    #[test]
    fn parses_config_dump_with_kind_patterns() {
        let args = Args::try_parse_from(["tayf", "config", "dump", "--kind", "patterns"]).unwrap();
        match args.cmd {
            Some(Cmd::Config(ConfigArgs { action: Some(ConfigAction::Dump(d)) })) => {
                assert!(matches!(d.kind, Some(DumpKind::Patterns)));
            }
            other => panic!("expected Config Dump Patterns; got {other:?}"),
        }
    }

    #[test]
    fn parses_config_status() {
        let args = Args::try_parse_from(["tayf", "config", "status"]).unwrap();
        assert!(matches!(
            args.cmd,
            Some(Cmd::Config(ConfigArgs { action: Some(ConfigAction::Status) }))
        ));
    }

    #[test]
    fn root_flags_pass_through_when_subcommand_present() {
        // §4.3 invariant: `tayf --theme dark config` carries --theme into args.run
        // so the TUI can highlight the active theme.
        let args = Args::try_parse_from(["tayf", "--theme", "dark", "config"]).unwrap();
        assert_eq!(args.run.theme.as_deref(), Some("dark"));
        assert!(matches!(args.cmd, Some(Cmd::Config(_))));
    }
}
