//! Effective-theme precedence for the live preview.
//!
//! Mirrors the runtime ladder in `src/lib.rs` (CLI > config > profile.theme >
//! bg-detect default) minus the CLI tier — the TUI edits the config, not CLI
//! flags — so the preview and the real `tayf` output cannot drift. Spec §5;
//! memory `feedback_reload_precedence_snapshot`.
//!
//! [`effective_theme`] is a pure function operating only on its arguments.
//! [`resolve_from_snapshot`] performs a best-effort profile load (`.ok()`) to
//! extract the profile's theme override before delegating to `effective_theme`.
//!
//! **Note (M-3):** The runtime's `--no-color` gate on the bg-detect fallback is
//! not mirrored here — the TUI always renders colors.

use crate::bg_detect::BgTheme;

/// Resolve the theme the preview must compile with: explicit config theme,
/// else the active profile's theme override, else the bg-detect default.
pub(crate) fn effective_theme(
    config_theme: Option<&str>,
    profile_theme: Option<&str>,
    bg: BgTheme,
) -> String {
    config_theme.or(profile_theme).map_or_else(|| bg.as_theme_name().to_owned(), str::to_owned)
}

/// Resolve the effective theme for a snapshot: load the named profile (if any)
/// to read its `theme` override, then apply the `config > profile.theme > bg`
/// precedence via [`effective_theme`].
///
/// A profile **load error is intentionally swallowed** (`.ok()`): the caller's
/// subsequent `compile_from_config` re-loads the profile and surfaces any real
/// error through the preview's `compile_error` banner — resolving the theme name
/// is best-effort and must not itself fail the preview.
pub(crate) fn resolve_from_snapshot(
    config_theme: Option<&str>,
    profile: Option<&str>,
    bg: BgTheme,
) -> String {
    let profile_theme: Option<String> = profile
        .and_then(|name| crate::profiles::load(name).ok())
        .and_then(|lp| lp.profile.theme.clone());
    effective_theme(config_theme, profile_theme.as_deref(), bg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_theme_wins() {
        assert_eq!(effective_theme(Some("classic"), Some("light"), BgTheme::Dark), "classic");
    }

    #[test]
    fn profile_theme_when_no_config_theme() {
        assert_eq!(effective_theme(None, Some("light"), BgTheme::Dark), "light");
    }

    #[test]
    fn bg_fallback_when_unset_dark() {
        assert_eq!(effective_theme(None, None, BgTheme::Dark), "dark");
    }

    #[test]
    fn bg_fallback_when_unset_light() {
        assert_eq!(effective_theme(None, None, BgTheme::Light), "light");
    }

    #[test]
    fn resolve_from_snapshot_no_profile_falls_back_to_bg() {
        assert_eq!(resolve_from_snapshot(None, None, BgTheme::Light), "light");
        assert_eq!(resolve_from_snapshot(Some("classic"), None, BgTheme::Dark), "classic");
    }
}
