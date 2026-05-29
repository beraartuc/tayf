# Audit Corpus Regression Harness

Per spec v0.7 §5.3 — adversarial input corpora for 7 audit-doc items.

## File format

Each `.txt` file under this directory describes one audit item:

```
# Audit item: <ID>
# Rule under test: <rule_name>
# Measurement mode: PIPELINE | RULE
# Profile context: (none) | aws | docker | k8s | gcp | network
# Audit doc: docs/superpowers/reviews/2026-05-27-builtin-pattern-fp-audit.md §<ref>
# Decision: <KEEP | TIGHTEN | DEMOTE | ACCEPT-DOCUMENTED | TBD>

POS: <input> => <expected_match>
NEG: <input>
```

- `POS:` rows assert the rule's leftmost match span equals `<expected_match>` exactly.
- `NEG:` rows assert the rule produces NO match for `<input>`.
- `PIPELINE` mode uses the production pipeline (priority + overlap + profile). Use this for decisions.
- `RULE` mode uses single-rule regex isolation. Use only for debugging.

## Decision enum (spec §5.4)

- **KEEP** — pattern stays; corpus pins regression. Allowed only if FP rate ≤ 5% of NEG.
- **TIGHTEN** — pattern data tightened; before/after corpus measurement.
- **DEMOTE-to-user-config** — pattern removed from built-ins; recipe moved to user-config README.
- **ACCEPT-DOCUMENTED** — high-FP (>5%) built-in with no clean fix under the regex crate's no-look-around constraint; retained for common-case value and documented in the README "Known limitations" section. Not valid at FP ≤ 5% (use KEEP).

Memory mandate: high-FP (>5%) requires TIGHTEN, DEMOTE, or ACCEPT-DOCUMENTED.
