//! Reusable name-list filter for Patterns / Themes / Profiles tabs.
//!
//! Filters a `&'a str` iterator by case-insensitive substring match.
//! Returns matching names in the original iteration order.
//!
//! Used by:
//! - [`crate::config_tui::tabs::patterns`] (builtin rule names)
//! - [`crate::config_tui::tabs::themes`] (builtin theme names)
//! - [`crate::config_tui::tabs::profiles`] (embedded profile names)
//!
//! Invariants:
//! - Empty `filter` returns every name in the iterator's order (allocates
//!   a fresh `Vec`; no early-out by reference).
//! - Comparison is case-insensitive via `to_lowercase` on both needle
//!   and each candidate.
//! - Allocates lowercase copies per candidate (acceptable: names are
//!   short and filtering is per-keystroke at human speeds).

/// Filter `names` to those containing `filter` (case-insensitive).
///
/// Empty `filter` returns every name in order. Allocates lowercase copies
/// of each candidate (acceptable: names are short and filtering is
/// per-keystroke at human speeds).
pub(crate) fn filter_names_lowercase<'a, I>(names: I, filter: &str) -> Vec<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    if filter.is_empty() {
        return names.into_iter().collect();
    }
    let needle = filter.to_lowercase();
    names.into_iter().filter(|n| n.to_lowercase().contains(&needle)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_names_lowercase_empty_filter_returns_all() {
        let names = ["alpha", "beta", "gamma"];
        let out = filter_names_lowercase(names.iter().copied(), "");
        assert_eq!(out, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn filter_names_lowercase_substring_match_case_insensitive() {
        let names = ["alpha", "BetaPrime", "gamma"];
        let out = filter_names_lowercase(names.iter().copied(), "PRIME");
        assert_eq!(out, vec!["BetaPrime"]);
    }

    #[test]
    fn filter_names_lowercase_no_matches_returns_empty() {
        let names = ["alpha", "beta"];
        let out = filter_names_lowercase(names.iter().copied(), "xx");
        assert!(out.is_empty());
    }
}
