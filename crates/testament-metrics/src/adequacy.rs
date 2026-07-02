use std::collections::BTreeSet;

use testament_core::{AssertionKind, Axis, MetricOutcome, Provenance, TestFileIr};

pub fn compute(ir: &TestFileIr) -> Vec<MetricOutcome> {
    vec![
        assertion_density(ir),
        assertion_diversity(ir),
        boundary_signal(ir),
    ]
}

fn assertion_density(ir: &TestFileIr) -> MetricOutcome {
    let cases = ir.case_count();
    let assertions = ir.assertion_count();
    let density = if cases == 0 {
        0.0
    } else {
        assertions as f64 / cases as f64
    };
    let score = density.min(1.0);

    MetricOutcome {
        id: "adequacy.assertion_density".to_owned(),
        axis: Axis::Adequacy,
        score: Some(score),
        value: density,
        unit: "assertions_per_case".to_owned(),
        summary: format!("{assertions} assertions across {cases} test cases"),
        findings: Vec::new(),
        provenance: Provenance::new(
            &["A6"],
            "Assertion count per test case is used as a static proxy for oracle strength.",
            "This is a static approximation and does not prove assertion quality.",
        ),
    }
}

fn assertion_diversity(ir: &TestFileIr) -> MetricOutcome {
    let mut kinds = BTreeSet::<AssertionKind>::new();
    for case in ir.cases() {
        for assertion in &case.assertions {
            kinds.insert(assertion.kind);
        }
    }

    let score = if kinds.is_empty() {
        0.0
    } else {
        (kinds.len() as f64 / 5.0).min(1.0)
    };

    MetricOutcome {
        id: "adequacy.assertion_diversity".to_owned(),
        axis: Axis::Adequacy,
        score: Some(score),
        value: kinds.len() as f64,
        unit: "assertion_kinds".to_owned(),
        summary: format!("{} assertion kind(s) observed", kinds.len()),
        findings: Vec::new(),
        provenance: Provenance::new(
            &["A6"],
            "Assertion kind diversity approximates whether tests exercise multiple oracle styles.",
            "The diversity score is heuristic and extends the cited assertion-effectiveness result.",
        ),
    }
}

fn boundary_signal(ir: &TestFileIr) -> MetricOutcome {
    let cases = ir.cases();
    let with_boundary = cases
        .iter()
        .filter(|case| {
            case.literals
                .iter()
                .any(|literal| matches!(literal.kind, testament_core::LiteralKind::Boundary))
        })
        .count();
    let score = if cases.is_empty() {
        0.0
    } else {
        with_boundary as f64 / cases.len() as f64
    };

    MetricOutcome {
        id: "adequacy.boundary_signal".to_owned(),
        axis: Axis::Adequacy,
        score: Some(score),
        value: with_boundary as f64,
        unit: "cases".to_owned(),
        summary: format!("{with_boundary} case(s) contain boundary-value literals"),
        findings: Vec::new(),
        provenance: Provenance::new(
            &["A1"],
            "Boundary-value literals are treated as a static signal for data adequacy.",
            "This does not know the SUT domain limits and should be interpreted as a signal only.",
        ),
    }
}
