use std::collections::BTreeSet;
use std::path::Path;

use testament_core::{
    AssertionKind, Axis, CoverageEvidence, EvidenceSet, FileCoverage, MetricOutcome, Provenance,
    TestFileIr, TraceEvidence, normalize_path, resolve_test_case_id,
};

pub fn compute(ir: &TestFileIr, evidence: &EvidenceSet) -> Vec<MetricOutcome> {
    let mut outcomes = vec![
        assertion_density(ir),
        assertion_diversity(ir),
        boundary_signal(ir),
    ];

    if let Some((file_coverage, coverage_path)) = evidence
        .coverage
        .as_ref()
        .and_then(|coverage| sut_coverage_for_ir(ir, coverage))
    {
        if let Some(line_rate) = file_coverage.line_coverage() {
            outcomes.push(line_coverage(line_rate, &coverage_path));
            outcomes.push(checked_coverage(
                ir,
                file_coverage,
                line_rate,
                &coverage_path,
                evidence.trace.as_ref(),
            ));
        }
        if let Some(branch_rate) = file_coverage.branch_coverage() {
            outcomes.push(branch_coverage(branch_rate, &coverage_path));
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
            mutation.score_override.is_some(),
        ));
    }

    outcomes
}

fn sut_coverage_for_ir<'a>(
    ir: &TestFileIr,
    coverage: &'a CoverageEvidence,
) -> Option<(&'a FileCoverage, String)> {
    for hint in &ir.subject_hints {
        if let Some(file) = coverage.file_for_path(&hint.path) {
            return Some((file, hint.path.to_string_lossy().into_owned()));
        }
    }

    coverage
        .file_for_path(&ir.path)
        .map(|file| (file, ir.path_display()))
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
    // Five distinct oracle styles are treated as broad coverage; the IR has
    // additional framework-specific categories that are not expected in every suite.
    const SATURATING_ASSERTION_KIND_COUNT: f64 = 5.0;
    let mut kinds = BTreeSet::<AssertionKind>::new();
    for case in ir.cases() {
        for assertion in &case.assertions {
            kinds.insert(assertion.kind);
        }
    }

    let score = if kinds.is_empty() {
        0.0
    } else {
        (kinds.len() as f64 / SATURATING_ASSERTION_KIND_COUNT).min(1.0)
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

fn line_coverage(line_rate: f64, coverage_path: &str) -> MetricOutcome {
    MetricOutcome {
        id: "adequacy.line_coverage".to_owned(),
        axis: Axis::Adequacy,
        score: Some(line_rate.clamp(0.0, 1.0)),
        value: line_rate.clamp(0.0, 1.0),
        unit: "ratio".to_owned(),
        summary: format!(
            "{:.1}% line coverage from evidence for {}",
            line_rate * 100.0,
            coverage_path
        ),
        findings: Vec::new(),
        provenance: Provenance::new(
            &["A1", "A7"],
            "Line coverage is a structural adequacy criterion from coverage evidence.",
            "Coverage alone is not treated as sufficient evidence of test effectiveness.",
        ),
    }
}

fn branch_coverage(branch_rate: f64, coverage_path: &str) -> MetricOutcome {
    MetricOutcome {
        id: "adequacy.branch_coverage".to_owned(),
        axis: Axis::Adequacy,
        score: Some(branch_rate.clamp(0.0, 1.0)),
        value: branch_rate.clamp(0.0, 1.0),
        unit: "ratio".to_owned(),
        summary: format!(
            "{:.1}% branch coverage from evidence for {}",
            branch_rate * 100.0,
            coverage_path
        ),
        findings: Vec::new(),
        provenance: Provenance::new(
            &["A1", "A7"],
            "Branch coverage is ranked above line coverage in structural adequacy criteria.",
            "The value depends on the external coverage tool's branch model.",
        ),
    }
}

fn checked_coverage(
    ir: &TestFileIr,
    coverage: &FileCoverage,
    line_rate: f64,
    coverage_path: &str,
    trace: Option<&TraceEvidence>,
) -> MetricOutcome {
    if let Some(outcome) = dynamic_checked_coverage(ir, coverage, coverage_path, trace) {
        return outcome;
    }

    let cases = ir.cases();
    let asserted_cases = cases
        .iter()
        .filter(|case| !case.assertions.is_empty())
        .count();
    let assertion_reach = if cases.is_empty() {
        0.0
    } else {
        asserted_cases as f64 / cases.len() as f64
    };
    let executable_reach = coverage.line_coverage().unwrap_or(line_rate);
    let score = (executable_reach * assertion_reach).clamp(0.0, 1.0);

    MetricOutcome {
        id: "adequacy.checked_coverage_static".to_owned(),
        axis: Axis::Adequacy,
        score: Some(score),
        value: score,
        unit: "ratio".to_owned(),
        summary: format!(
            "{:.1}% checked coverage static approximation for {}",
            score * 100.0,
            coverage_path
        ),
        findings: Vec::new(),
        provenance: Provenance::new(
            &["A5"],
            "Static checked coverage approximates which covered lines may be reached by assertions.",
            "This separate static metric scales SUT line coverage by the ratio of test cases with normalized assertions; it is not used as dynamic trace evidence.",
        ),
    }
}

fn dynamic_checked_coverage(
    ir: &TestFileIr,
    coverage: &FileCoverage,
    coverage_path: &str,
    trace: Option<&TraceEvidence>,
) -> Option<MetricOutcome> {
    let trace = trace?;
    let trace_lines = dynamic_lines_for_ir(trace, ir, coverage_path);
    if trace_lines.checked.is_empty() {
        return None;
    }

    let denominator = checked_denominator(&trace_lines.executed, coverage);
    if denominator.is_empty() {
        return None;
    }

    let checked = trace_lines.checked.intersection(&denominator).count();
    let score = (checked as f64 / denominator.len() as f64).clamp(0.0, 1.0);

    Some(MetricOutcome {
        id: "adequacy.checked_coverage".to_owned(),
        axis: Axis::Adequacy,
        score: Some(score),
        value: score,
        unit: "ratio".to_owned(),
        summary: format!(
            "{:.1}% checked coverage from dynamic trace for {} ({} checked line(s) / {} executed line(s))",
            score * 100.0,
            coverage_path,
            checked,
            denominator.len()
        ),
        findings: Vec::new(),
        provenance: Provenance::new(
            &["A5"],
            "Checked coverage estimates which executed SUT lines are reached by assertions.",
            "Uses dynamic assertion-dependency trace evidence; the static approximation is reported under adequacy.checked_coverage_static.",
        ),
    })
}

#[derive(Default)]
struct DynamicTraceLines {
    executed: BTreeSet<usize>,
    checked: BTreeSet<usize>,
}

fn dynamic_lines_for_ir(
    trace: &TraceEvidence,
    ir: &TestFileIr,
    coverage_path: &str,
) -> DynamicTraceLines {
    let cases = ir.cases();
    let mut lines = DynamicTraceLines::default();
    for (case_key, case_trace) in &trace.cases {
        if resolve_test_case_id(cases.as_slice(), case_key).is_none() {
            continue;
        }
        lines.executed.extend(
            case_trace
                .executed_lines
                .iter()
                .filter(|requirement| same_path(&requirement.path, coverage_path))
                .map(|requirement| requirement.line),
        );
        lines.checked.extend(
            case_trace
                .checked_lines
                .iter()
                .filter(|requirement| same_path(&requirement.path, coverage_path))
                .map(|requirement| requirement.line),
        );
    }
    lines
}

fn checked_denominator(
    executed_lines: &BTreeSet<usize>,
    coverage: &FileCoverage,
) -> BTreeSet<usize> {
    if !executed_lines.is_empty() {
        return restrict_to_executable_lines(executed_lines, coverage);
    }
    if !coverage.covered_lines.is_empty() {
        return coverage.covered_lines.clone();
    }
    coverage.executable_lines.clone()
}

fn restrict_to_executable_lines(
    lines: &BTreeSet<usize>,
    coverage: &FileCoverage,
) -> BTreeSet<usize> {
    if coverage.executable_lines.is_empty() {
        return lines.clone();
    }
    lines
        .intersection(&coverage.executable_lines)
        .copied()
        .collect()
}

fn same_path(left: &str, right: &str) -> bool {
    let left = normalize_path(Path::new(left));
    let right = normalize_path(Path::new(right));
    left == right
        || Path::new(&left).ends_with(Path::new(&right))
        || Path::new(&right).ends_with(Path::new(&left))
}

fn mutation_score(
    score: f64,
    killed: usize,
    total: usize,
    equivalent_marked: usize,
    from_summary: bool,
) -> MetricOutcome {
    MetricOutcome {
        id: "adequacy.mutation_score".to_owned(),
        axis: Axis::Adequacy,
        score: Some(score.clamp(0.0, 1.0)),
        value: score.clamp(0.0, 1.0),
        unit: "ratio".to_owned(),
        summary: if from_summary {
            format!("mutation score {score:.3} from report summary")
        } else {
            format!(
                "{killed} killed mutants out of {total} total ({equivalent_marked} equivalent marked)"
            )
        },
        findings: Vec::new(),
        provenance: Provenance::new(
            &["A2", "A3", "A4"],
            "Mutation score is killed / (total - equivalent_marked).",
            "The tool consumes external mutation reports rather than running a mutation engine.",
        ),
    }
}

#[cfg(test)]
mod path_tests {
    use super::same_path;

    #[test]
    fn suffix_matching_respects_path_components() {
        assert!(same_path("/work/lib/cart.rb", "lib/cart.rb"));
        assert!(!same_path("lib/shopping_cart.rb", "cart.rb"));
    }
}
