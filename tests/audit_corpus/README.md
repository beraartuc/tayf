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
# Decision: <KALSIN | TIGHTEN | DEMOTE | TBD>

POS: <input> => <expected_match>
NEG: <input>
```

- `POS:` rows assert the rule's leftmost match span equals `<expected_match>` exactly.
- `NEG:` rows assert the rule produces NO match for `<input>`.
- `PIPELINE` mode uses the production pipeline (priority + overlap + profile). Use this for karar decisions.
- `RULE` mode uses single-rule regex isolation. Use only for debugging.

## Karar enum (spec §5.4)

- **KALSIN** — pattern stays; corpus pins regression. Allowed only if FP rate ≤ 5% of NEG.
- **TIGHTEN** — pattern data tightened; before/after corpus measurement.
- **DEMOTE-to-user-config** — pattern removed from built-ins; recipe moved to user-config README.

Memory mandate: high-FP (>5%) → TIGHTEN or DEMOTE zorunlu.
