//! Audit-doc FP/FN regression harness — spec v0.7 §5.3.

use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
#[allow(dead_code)]
// reason: variants will be populated by corpus files in Task 18+. Both
// variants are part of the public corpus grammar even if no current
// corpus uses RULE mode.
enum MeasurementMode {
    Pipeline,
    Rule,
}

#[derive(Debug)]
#[allow(dead_code)]
// reason: fields read by the future corpus tests; `item_id` surfaces in
// drift messages, `profile_context` threads to pipeline_spans.
struct AuditCase {
    item_id: String,
    rule_name: String,
    measurement_mode: MeasurementMode,
    profile_context: Option<String>,
    positives: Vec<(String, String)>,
    negatives: Vec<String>,
}

#[allow(dead_code)]
// reason: same as struct fields — corpus tests in Task 18+ call this.
fn corpus_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/audit_corpus").join(name)
}

#[allow(dead_code)]
// reason: corpus tests call this; not yet wired up in this Task.
fn parse_corpus_file(path: &PathBuf) -> AuditCase {
    let content = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read corpus {}: {e}", path.display()));
    parse_corpus_string(&content)
}

fn parse_corpus_string(s: &str) -> AuditCase {
    let mut item_id = None;
    let mut rule_name = None;
    let mut mode = None;
    let mut profile = None;
    let mut positives = Vec::new();
    let mut negatives = Vec::new();

    for (lineno, line) in s.lines().enumerate() {
        let lineno = lineno + 1;
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('#') {
            let kv = rest.trim();
            if let Some(v) = kv.strip_prefix("Audit item:") {
                item_id = Some(v.trim().to_owned());
            } else if let Some(v) = kv.strip_prefix("Rule under test:") {
                rule_name = Some(v.trim().to_owned());
            } else if let Some(v) = kv.strip_prefix("Measurement mode:") {
                mode = Some(match v.trim() {
                    "PIPELINE" => MeasurementMode::Pipeline,
                    "RULE" => MeasurementMode::Rule,
                    other => panic!("line {lineno}: unknown measurement mode {other:?}"),
                });
            } else if let Some(v) = kv.strip_prefix("Profile context:") {
                let t = v.trim();
                profile = if t == "(none)" { None } else { Some(t.to_owned()) };
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("POS:") {
            let (input, expected) = rest
                .split_once(" => ")
                .unwrap_or_else(|| panic!("line {lineno}: POS missing ' => ' separator"));
            positives.push((input.trim().to_owned(), expected.trim().to_owned()));
        } else if let Some(rest) = trimmed.strip_prefix("NEG:") {
            negatives.push(rest.trim().to_owned());
        } else {
            panic!("line {lineno}: unrecognized line {trimmed:?}");
        }
    }

    let item_id = item_id.expect("corpus missing '# Audit item:' header");
    let rule_name = rule_name.expect("corpus missing '# Rule under test:' header");
    let mode = mode.expect("corpus missing '# Measurement mode:' header");

    assert!(
        !positives.is_empty() && !negatives.is_empty(),
        "corpus '{item_id}': must have at least one POS and one NEG (no signal otherwise)",
    );

    AuditCase {
        item_id,
        rule_name,
        measurement_mode: mode,
        profile_context: profile,
        positives,
        negatives,
    }
}

#[allow(dead_code)]
// reason: corpus tests (Task 18+) use this; the integration suite for
// Task 17 only exercises parser self-tests.
fn measure(case: &AuditCase) -> (usize, usize, usize, usize) {
    let lookup = |input: &str| -> Option<String> {
        match case.measurement_mode {
            MeasurementMode::Rule => tayf::__test_api::match_named_rule(&case.rule_name, input),
            MeasurementMode::Pipeline => {
                tayf::__test_api::pipeline_spans(input, case.profile_context.as_deref())
                    .into_iter()
                    .find(|(name, _)| name == &case.rule_name)
                    .map(|(_, span)| span)
            }
        }
    };
    let mut fp = 0;
    let mut fn_ = 0;
    for (input, expected) in &case.positives {
        if lookup(input).as_deref() != Some(expected.as_str()) {
            fn_ += 1;
        }
    }
    for input in &case.negatives {
        if lookup(input).is_some() {
            fp += 1;
        }
    }
    (fp, fn_, case.positives.len(), case.negatives.len())
}

#[allow(dead_code)]
// reason: corpus tests (Task 18+) use this karar-mandate enforcement
// helper.
fn check_karar_mandate(fp: usize, nneg: usize, decision: &str, item: &str) {
    if nneg == 0 {
        return;
    }
    let fp_rate = fp as f64 / nneg as f64;
    if fp_rate > 0.05 {
        let fp_pct = fp_rate * 100.0;
        assert_ne!(
            decision, "KALSIN",
            "{item}: FP rate {fp_pct:.1}% > 5% threshold; karar must be TIGHTEN or DEMOTE",
        );
    }
}

#[test]
fn corpus_parser_panics_on_malformed_pos_line() {
    let bad = "# Audit item: fake\n# Rule under test: filename\n# Measurement mode: PIPELINE\nPOS: input without arrow\nNEG: x\n";
    let r = std::panic::catch_unwind(|| parse_corpus_string(bad));
    assert!(r.is_err(), "malformed POS line must fail-fast");
}

#[test]
fn corpus_parser_rejects_empty_case() {
    let empty = "# Audit item: fake\n# Rule under test: filename\n# Measurement mode: PIPELINE\n";
    let r = std::panic::catch_unwind(|| parse_corpus_string(empty));
    assert!(r.is_err(), "empty corpus must fail-fast");
}
