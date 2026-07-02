use std::collections::{BTreeMap, BTreeSet};

use testament_core::{
    Axis, CoverageRequirement, EvidenceSet, Finding, MetricOutcome, Provenance, RuleConfig,
    Severity, SourceSpan, TestCase, TestFileIr,
};

pub fn compute(ir: &TestFileIr, rules: &RuleConfig, evidence: &EvidenceSet) -> Vec<MetricOutcome> {
    let (candidate_ratio, findings) = redundancy_candidates(ir, rules, evidence);
    let structural = structural_similarity_score(ir, &findings);
    let assertion_overlap = assertion_overlap_score(ir, &findings);
    let mut outcomes = vec![
        MetricOutcome {
            id: "redundancy.candidate_ratio".to_owned(),
            axis: Axis::Redundancy,
            score: Some(1.0 - candidate_ratio),
            value: candidate_ratio,
            unit: "ratio".to_owned(),
            summary: format!(
                "{:.1}% of cases are redundancy review candidates",
                candidate_ratio * 100.0
            ),
            findings,
            provenance: Provenance::new(
                &["R1", "R2", "R3", "R5"],
                "Redundancy candidates are reported for review rather than automatic deletion.",
                "Uses mutation/per-test coverage when available, otherwise static structural proxies.",
            ),
        },
        structural,
        assertion_overlap,
    ];

    if let Some(per_test) = evidence.per_test_coverage.as_ref() {
        outcomes.push(coverage_subsumption_score(ir, &per_test.cases));
    }
    if let Some(mutation) = evidence
        .mutation
        .as_ref()
        .filter(|mutation| !mutation.per_test_kills.is_empty())
    {
        outcomes.push(mutant_subsumption_score(ir, &mutation.per_test_kills));
    }
    outcomes
}

fn redundancy_candidates(
    ir: &TestFileIr,
    rules: &RuleConfig,
    evidence: &EvidenceSet,
) -> (f64, Vec<Finding>) {
    let cases = ir.cases();
    if cases.is_empty() {
        return (0.0, Vec::new());
    }

    let mut candidate_ids = BTreeSet::new();
    let mut findings = Vec::new();

    if let Some(mutation) = evidence.mutation.as_ref() {
        add_set_subsumption_findings(
            "redundancy.mutant_subsumption",
            cases.as_slice(),
            &mutation.per_test_kills,
            &mut candidate_ids,
            &mut findings,
            "test case kills no unique mutants compared with another case",
        );
    }

    if let Some(per_test) = evidence.per_test_coverage.as_ref() {
        let mapped = per_test
            .cases
            .iter()
            .map(|(case_id, lines)| {
                (
                    case_id.clone(),
                    lines
                        .iter()
                        .map(requirement_key)
                        .collect::<BTreeSet<String>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        add_set_subsumption_findings(
            "redundancy.coverage_subsumption",
            cases.as_slice(),
            &mapped,
            &mut candidate_ids,
            &mut findings,
            "test case covers no unique requirements compared with another case",
        );
    }

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
                    format!(
                        "representative `{}` similarity {:.3}",
                        left.name, similarity
                    ),
                ));
            }

            if assertion_overlap(left, right) {
                candidate_ids.insert(right.id.clone());
                findings.push(redundancy_finding(
                    "redundancy.assertion_overlap",
                    right,
                    right.span.clone(),
                    "test case repeats assertion subjects already covered nearby",
                    format!(
                        "representative `{}` shares assertion subject/kind",
                        left.name
                    ),
                ));
            }
        }
    }

    (candidate_ids.len() as f64 / cases.len() as f64, findings)
}

fn add_set_subsumption_findings(
    rule_id: &str,
    cases: &[&TestCase],
    sets: &BTreeMap<String, BTreeSet<String>>,
    candidate_ids: &mut BTreeSet<String>,
    findings: &mut Vec<Finding>,
    message: &str,
) {
    for left in cases {
        let Some(left_set) = sets.get(&left.id) else {
            continue;
        };
        for right in cases {
            if left.id == right.id {
                continue;
            }
            let Some(right_set) = sets.get(&right.id) else {
                continue;
            };
            if right_set.is_empty() || !right_set.is_subset(left_set) {
                continue;
            }
            if candidate_ids.insert(right.id.clone()) {
                findings.push(redundancy_finding(
                    rule_id,
                    right,
                    right.span.clone(),
                    message,
                    format!(
                        "representative `{}` subsumes {} requirement(s)",
                        left.name,
                        right_set.len()
                    ),
                ));
            }
        }
    }
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

fn assertion_overlap_score(ir: &TestFileIr, candidate_findings: &[Finding]) -> MetricOutcome {
    let cases = ir.case_count().max(1) as f64;
    let ratio = candidate_findings
        .iter()
        .filter(|finding| finding.rule_id == "redundancy.assertion_overlap")
        .count() as f64
        / cases;

    MetricOutcome {
        id: "redundancy.assertion_overlap".to_owned(),
        axis: Axis::Redundancy,
        score: Some((1.0 - ratio).clamp(0.0, 1.0)),
        value: ratio,
        unit: "ratio".to_owned(),
        summary: format!("{:.1}% assertion-overlap finding ratio", ratio * 100.0),
        findings: Vec::new(),
        provenance: Provenance::new(
            &["R3"],
            "Assertion overlap reports repeated assertion subject/kind pairs as review candidates.",
            "This is a static review signal and does not imply tests should be deleted automatically.",
        ),
    }
}

fn coverage_subsumption_score(
    ir: &TestFileIr,
    cases: &BTreeMap<String, BTreeSet<CoverageRequirement>>,
) -> MetricOutcome {
    let represented = greedy_representatives(
        &cases
            .iter()
            .map(|(case_id, values)| {
                (
                    case_id.clone(),
                    values.iter().map(requirement_key).collect::<BTreeSet<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>(),
    );
    representative_metric(
        "redundancy.coverage_subsumption",
        ir,
        cases.len(),
        represented.len(),
        &["R1", "R2", "R5"],
        "Greedy set cover selects representative tests for per-test coverage requirements.",
    )
}

fn mutant_subsumption_score(
    ir: &TestFileIr,
    cases: &BTreeMap<String, BTreeSet<String>>,
) -> MetricOutcome {
    let represented = greedy_representatives(cases);
    representative_metric(
        "redundancy.mutant_subsumption",
        ir,
        cases.len(),
        represented.len(),
        &["R4"],
        "Per-test mutant kill sets select representative tests with mutant-based redundancy.",
    )
}

fn representative_metric(
    id: &str,
    ir: &TestFileIr,
    known_cases: usize,
    representatives: usize,
    references: &[&str],
    definition: &str,
) -> MetricOutcome {
    let total_cases = ir.case_count().max(known_cases).max(1);
    let redundant = total_cases.saturating_sub(representatives);
    let ratio = redundant as f64 / total_cases as f64;
    MetricOutcome {
        id: id.to_owned(),
        axis: Axis::Redundancy,
        score: Some((1.0 - ratio).clamp(0.0, 1.0)),
        value: ratio,
        unit: "ratio".to_owned(),
        summary: format!(
            "{representatives} representative case(s), {redundant} redundancy candidate(s)"
        ),
        findings: Vec::new(),
        provenance: Provenance::new(
            references,
            definition,
            "This is a review candidate workflow; no automatic deletion is performed.",
        ),
    }
}

fn greedy_representatives(sets: &BTreeMap<String, BTreeSet<String>>) -> BTreeSet<String> {
    let mut uncovered = sets
        .values()
        .flat_map(|requirements| requirements.iter().cloned())
        .collect::<BTreeSet<_>>();
    let mut representatives = BTreeSet::new();

    while !uncovered.is_empty() {
        let best = sets
            .iter()
            .filter(|(case_id, _)| !representatives.contains(*case_id))
            .max_by_key(|(_, requirements)| requirements.intersection(&uncovered).count());
        let Some((case_id, requirements)) = best else {
            break;
        };
        let covered_now = requirements
            .intersection(&uncovered)
            .cloned()
            .collect::<Vec<_>>();
        if covered_now.is_empty() {
            break;
        }
        representatives.insert(case_id.clone());
        for requirement in covered_now {
            uncovered.remove(&requirement);
        }
    }

    representatives
}

fn requirement_key(requirement: &CoverageRequirement) -> String {
    format!("{}:{}", requirement.path, requirement.line)
}

fn similarity(left: &TestCase, right: &TestCase) -> f64 {
    let left_tokens = tokens(left);
    let right_tokens = tokens(right);
    if left_tokens.is_empty() && right_tokens.is_empty() {
        return 1.0;
    }
    let intersection = left_tokens.intersection(&right_tokens).count() as f64;
    let union = left_tokens.union(&right_tokens).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
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
    right.assertions.iter().any(|assertion| {
        left_keys.contains(&format!(
            "{}|{}",
            assertion.kind.as_str(),
            assertion.subject_expr
        ))
    })
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
