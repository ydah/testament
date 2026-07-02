use std::collections::BTreeSet;

use testament_core::{
    Axis, Finding, MetricOutcome, Provenance, RuleConfig, Severity, SourceSpan, TestCase,
    TestFileIr,
};

pub fn compute(ir: &TestFileIr, rules: &RuleConfig) -> Vec<MetricOutcome> {
    let (candidate_ratio, findings) = redundancy_candidates(ir, rules);
    let structural = structural_similarity_score(ir, &findings);

    vec![
        MetricOutcome {
            id: "redundancy.candidate_ratio".to_owned(),
            axis: Axis::Redundancy,
            score: Some(1.0 - candidate_ratio),
            value: candidate_ratio,
            unit: "ratio".to_owned(),
            summary: format!("{:.1}% of cases are redundancy review candidates", candidate_ratio * 100.0),
            findings,
            provenance: Provenance::new(
                &["R1", "R2", "R3", "R5"],
                "Static redundancy candidates are reported for review rather than automatic deletion.",
                "Without per-test coverage or mutation matrices, this uses structural and assertion overlap proxies.",
            ),
        },
        structural,
    ]
}

fn redundancy_candidates(ir: &TestFileIr, rules: &RuleConfig) -> (f64, Vec<Finding>) {
    let cases = ir.cases();
    if cases.is_empty() {
        return (0.0, Vec::new());
    }

    let mut candidate_ids = BTreeSet::new();
    let mut findings = Vec::new();
    for left_index in 0..cases.len() {
        for right_index in (left_index + 1)..cases.len() {
            let left = cases[left_index];
            let right = cases[right_index];
            let similarity = similarity(left, right);
            if similarity >= rules.structural_similarity_threshold {
                candidate_ids.insert(right.id.clone());
                findings.push(redundancy_finding(
                    "redundancy.structural_similarity",
                    right,
                    right.span.clone(),
                    "test case is structurally similar to another case",
                    format!("representative `{}` similarity {:.3}", left.name, similarity),
                ));
            }

            if assertion_overlap(left, right) {
                candidate_ids.insert(right.id.clone());
                findings.push(redundancy_finding(
                    "redundancy.assertion_overlap",
                    right,
                    right.span.clone(),
                    "test case repeats assertion subjects already covered nearby",
                    format!("representative `{}` shares assertion subject/kind", left.name),
                ));
            }
        }
    }

    (candidate_ids.len() as f64 / cases.len() as f64, findings)
}

fn structural_similarity_score(ir: &TestFileIr, candidate_findings: &[Finding]) -> MetricOutcome {
    let cases = ir.case_count().max(1) as f64;
    let ratio = candidate_findings
        .iter()
        .filter(|finding| finding.rule_id == "redundancy.structural_similarity")
        .count() as f64
        / cases;

    MetricOutcome {
        id: "redundancy.structural_similarity".to_owned(),
        axis: Axis::Redundancy,
        score: Some((1.0 - ratio).clamp(0.0, 1.0)),
        value: ratio,
        unit: "ratio".to_owned(),
        summary: format!("{:.1}% structural-similarity finding ratio", ratio * 100.0),
        findings: Vec::new(),
        provenance: Provenance::new(
            &["R1"],
            "Normalized test bodies are compared with token-set Jaccard similarity.",
            "This is a clone-detection proxy, not coverage- or mutant-based subsumption.",
        ),
    }
}

fn similarity(left: &TestCase, right: &TestCase) -> f64 {
    let left_tokens = tokens(left);
    let right_tokens = tokens(right);
    if left_tokens.is_empty() && right_tokens.is_empty() {
        return 1.0;
    }
    let intersection = left_tokens.intersection(&right_tokens).count() as f64;
    let union = left_tokens.union(&right_tokens).count() as f64;
    if union == 0.0 { 0.0 } else { intersection / union }
}

fn tokens(case: &TestCase) -> BTreeSet<String> {
    case.normalized_body
        .iter()
        .flat_map(|line| line.split_whitespace())
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn assertion_overlap(left: &TestCase, right: &TestCase) -> bool {
    if left.assertions.is_empty() || right.assertions.is_empty() {
        return false;
    }
    let left_keys = left
        .assertions
        .iter()
        .map(|assertion| format!("{}|{}", assertion.kind.as_str(), assertion.subject_expr))
        .collect::<BTreeSet<_>>();
    right
        .assertions
        .iter()
        .any(|assertion| left_keys.contains(&format!("{}|{}", assertion.kind.as_str(), assertion.subject_expr)))
}

fn redundancy_finding(
    rule_id: &str,
    case: &TestCase,
    span: SourceSpan,
    message: impl Into<String>,
    evidence: impl Into<String>,
) -> Finding {
    Finding::new(
        rule_id,
        Axis::Redundancy,
        Severity::Info,
        message,
        Some(span),
        evidence,
        Some(case.id.clone()),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use testament_core::AppConfig;

    use crate::analyze_content;

    #[test]
    fn detects_duplicate_cases_as_redundancy_candidates() {
        let report = analyze_content(
            Path::new("spec/cart_spec.rb"),
            r#"
            RSpec.describe Cart do
              it "counts one item" do
                cart.add(item)
                expect(cart.count).to eq(1)
              end

              it "counts another item" do
                cart.add(item)
                expect(cart.count).to eq(1)
              end
            end
            "#,
            &AppConfig::default(),
        );

        assert!(report.metric_value("redundancy.candidate_ratio").unwrap() > 0.0);
    }
}

