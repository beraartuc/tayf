//! Machine-enforces that the ACCEPT-DOCUMENTED built-in limitations
//! (C-4 filename, C-9 fqdn, E-1 ipv4 — spec v0.7.1 §5.3, §7) are actually
//! documented in the README with a disable-builtin recipe.

#[test]
fn readme_documents_accept_documented_limitations() {
    let readme = include_str!("../README.md");
    // Slice the "## Known limitations" section (its header through the next
    // top-level "## " heading or EOF) so the markers are asserted WITHIN that
    // section — the rule names and "enabled = false" also appear under
    // Configuration, so a whole-file `contains` would be a false guard.
    let start = readme
        .find("## Known limitations")
        .expect("README must have a '## Known limitations' section");
    let after = &readme[start + "## Known limitations".len()..];
    let section = match after.find("\n## ") {
        Some(end) => &after[..end],
        None => after,
    };
    for marker in ["filename", "fqdn", "ipv4"] {
        assert!(section.contains(marker), "limitations section must name rule: {marker}");
    }
    assert!(
        section.contains("enabled = false"),
        "limitations section must include the disable-builtin recipe (enabled = false)",
    );
}
