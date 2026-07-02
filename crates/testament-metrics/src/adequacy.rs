use std::collections::BTreeSet;

use testament_core::{
    AssertionKind, Axis, EvidenceSet, FileCoverage, MetricOutcome, Provenance, TestFileIr,
};

pub fn compute(ir: &TestFileIr, evidence: &EvidenceSet) -> Vec<MetricOutcome> {
    let mut outcomes = vec![
        assertion_density(ir),
        assertion_diversity(ir),
        boundary_signal(ir),
    ];

    if let Some(file_coverage) = evidence
        .coverage
        .as_ref()
        .and_then(|coverage| coverage.file_for_path(&ir.path))
    {
        if let Some(line_rate) = file_coverage.line_coverage() {
            outcomes.push(line_coverage(line_rate));
            outcomes.push(checked_coverage(ir, file_coverage, line_rate));
        }
        if let Some(branch_rate) = file_coverage.branch_coverage() {
            outcomes.push(branch_coverage(branch_rate));
        }
    }

    if let Some((mutation, score)) = evidence
        .mutation
        .as_ref()
        .and_then(|mutation| mutation.score().map(|score| (mutation, score)))
    {
        outcomes.push(mutation_score(
            score,
            mutation.killed,
            mutation.total,
            mutation.equivalent_marked,
        ));
    }

    outcomes
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

fn line_coverage(line_rate: f64) -> MetricOutcome {
    MetricOutcome {
        id: "adequacy.line_coverage".to_owned(),
        axis: Axis::Adequacy,
        score: Some(line_rate.clamp(0.0, 1.0)),
        value: line_rate.clamp(0.0, 1.0),
        unit: "ratio".to_owned(),
        summary: format!("{:.1}% line coverage from evidence", line_rate * 100.0),
        findings: Vec::new(),
        provenance: Provenance::new(
            &["A1", "A7"],
            "Line coverage is a structural adequacy criterion from coverage evidence.",
            "Coverage alone is not treated as sufficient evidence of test effectiveness.",
        ),
    }
}

fn branch_coverage(branch_rate: f64) -> MetricOutcome {
    MetricOutcome {
        id: "adequacy.branch_coverage".to_owned(),
        axis: Axis::Adequacy,
        score: Some(branch_rate.clamp(0.0, 1.0)),
        value: branch_rate.clamp(0.0, 1.0),
        unit: "ratio".to_owned(),
        summary: format!("{:.1}% branch coverage from evidence", branch_rate * 100.0),
        findings: Vec::new(),
        provenance: Provenance::new(
            &["A1", "A7"],
            "Branch coverage is ranked above line coverage in structural adequacy criteria.",
            "The value depends on the external coverage tool's branch model.",
        ),
    }
}

fn checked_coverage(ir: &TestFileIr, coverage: &FileCoverage, line_rate: f64) -> MetricOutcome {
    let assertion_lines = ir
        .cases()
        .iter()
        .flat_map(|case| {
            case.assertions
                .iter()
                .map(|assertion| assertion.span.start_line)
        })
        .collect::<BTreeSet<_>>();
    let checked_lines = assertion_lines
        .iter()
        .filter(|line| coverage.covered_lines.contains(line))
        .count();
    let assertion_reach = if assertion_lines.is_empty() {
        0.0
    } else {
        checked_lines as f64 / assertion_lines.len() as f64
    };
    let score = (line_rate * assertion_reach).clamp(0.0, 1.0);

    MetricOutcome {
        id: "adequacy.checked_coverage".to_owned(),
        axis: Axis::Adequacy,
        score: Some(score),
        value: score,
        unit: "ratio".to_owned(),
        summary: format!(
            "{:.1}% checked coverage static approximation",
            score * 100.0
        ),
        findings: Vec::new(),
        provenance: Provenance::new(
            &["A5"],
            "Checked coverage estimates which executed lines are reached by assertions.",
            "This implementation approximates dynamic slicing by intersecting assertion lines with covered lines and scaling by line coverage.",
        ),
    }
}

fn mutation_score(
    score: f64,
    killed: usize,
    total: usize,
    equivalent_marked: usize,
) -> MetricOutcome {
    MetricOutcome {
        id: "adequacy.mutation_score".to_owned(),
        axis: Axis::Adequacy,
        score: Some(score.clamp(0.0, 1.0)),
        value: score.clamp(0.0, 1.0),
        unit: "ratio".to_owned(),
        summary: format!(
            "{killed} killed mutants out of {total} total ({equivalent_marked} equivalent marked)"
        ),
        findings: Vec::new(),
        provenance: Provenance::new(
            &["A2", "A3", "A4"],
            "Mutation score is killed / (total - equivalent_marked).",
            "The tool consumes external mutation reports rather than running a mutation engine.",
        ),
    }
}
