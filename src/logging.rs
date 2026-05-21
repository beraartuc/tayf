//! Diagnostic logging via tracing-subscriber.
//!
//! Off by default. Writes only to stderr to avoid contaminating user output.
//! Enabled by setting `TAYF_LOG`, e.g. `TAYF_LOG=debug tayf`.

/// Initialize the global subscriber. Safe to call exactly once at program start.
// reason: wired up by the CLI entry point in Task 11; tests already exercise it.
#[allow(dead_code)]
pub(crate) fn init() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_env("TAYF_LOG").unwrap_or_else(|_| EnvFilter::new("off"));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_level(true)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_is_idempotent() {
        init();
        init();
    }
}
