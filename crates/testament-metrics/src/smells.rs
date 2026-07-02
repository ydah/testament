use std::collections::BTreeSet;

use testament_core::{
    Assertion, Axis, ExternalRefKind, Finding, MetricOutcome, Provenance, RuleConfig, Severity,
    SourceSpan, TagKind, TestCase, TestFileIr,
};

pub fn compute(ir: &TestFileIr, rules: &RuleConfig) -> MetricOutcome {
    let mut findings = Vec::new();
    for case in ir.cases() {
        findings.extend(case_smells(case, rules));
    }
    findings.extend(file_smells(ir));

    let case_count = ir.case_count().max(1) as f64;
    let penalty = findings
        .iter()
        .map(|finding| severity_penalty(finding.severity))
        .sum::<f64>()
        / case_count;
    let score = (1.0 - penalty).clamp(0.0, 1.0);

    MetricOutcome {
        id: "maintainability.smell_score".to_owned(),
        axis: Axis::Maintainability,
        score: Some(score),
        value: score,
        unit: "score".to_owned(),
        summary: format!("{} maintainability finding(s)", findings.len()),
        findings,
        provenance: Provenance::new(
            &["S1", "S2", "S3", "S4", "S5", "S6"],
            "Test smells are detected with IR-level operational rules inspired by tsDetect and test smell literature.",
            "Ruby dynamic behavior is approximated statically; unresolved metaprogramming is not penalized.",
        ),
    }
}

fn case_smells(case: &TestCase, rules: &RuleConfig) -> Vec<Finding> {
    let mut findings = Vec::new();
    detect_unknown_test(case, &mut findings);
    detect_assertion_roulette(case, rules, &mut findings);
    detect_eager_test(case, rules, &mut findings);
    detect_external_ref_smells(case, &mut findings);
    detect_conditional_logic(case, &mut findings);
    detect_ignored_test(case, &mut findings);
    detect_magic_number(case, rules, &mut findings);
    detect_redundant_print(case, &mut findings);
    detect_duplicate_assert(case, &mut findings);
    detect_mock_overuse(case, rules, &mut findings);
    findings
}

fn detect_unknown_test(case: &TestCase, findings: &mut Vec<Finding>) {
    if case.assertions.is_empty()
        && !case.has_tag(TagKind::Skipped)
        && !case.has_tag(TagKind::Pending)
    {
        findings.push(finding(
            "smell.unknown_test",
            Severity::Error,
            case,
            case.span.clone(),
            "test case has no assertion",
            "no assertion was normalized into the IR",
        ));
    }
}

fn detect_assertion_roulette(case: &TestCase, rules: &RuleConfig, findings: &mut Vec<Finding>) {
    let unmessaged = case
        .assertions
        .iter()
        .filter(|assertion| !assertion.has_message)
        .count();
    if case.assertions.len() > rules.assertion_roulette_max && unmessaged > 1 {
        findings.push(finding(
            "smell.assertion_roulette",
            Severity::Warning,
            case,
            case.span.clone(),
            "multiple assertions lack failure messages",
            format!(
                "{} assertion(s), {} without messages",
                case.assertions.len(),
                unmessaged
            ),
        ));
    }
}

fn detect_eager_test(case: &TestCase, rules: &RuleConfig, findings: &mut Vec<Finding>) {
    let unique_calls = case.sut_calls.iter().collect::<BTreeSet<_>>().len();
    if unique_calls > rules.eager_test_max_sut_calls {
        findings.push(finding(
            "smell.eager_test",
            Severity::Warning,
            case,
            case.span.clone(),
            "test case exercises many SUT calls",
            format!("{unique_calls} unique SUT-like calls"),
        ));
    }
}

fn detect_external_ref_smells(case: &TestCase, findings: &mut Vec<Finding>) {
    for external in &case.external_refs {
        match external.kind {
            ExternalRefKind::FileSystem | ExternalRefKind::Network | ExternalRefKind::Database => {
                findings.push(finding(
                    "smell.mystery_guest",
                    Severity::Warning,
                    case,
                    external.span.clone(),
                    "test case reaches external state",
                    external.expression.clone(),
                ));
            }
            ExternalRefKind::Sleep => findings.push(finding(
                "smell.sleepy_test",
                Severity::Warning,
                case,
                external.span.clone(),
                "test case uses fixed waiting",
                external.expression.clone(),
            )),
            ExternalRefKind::Time => findings.push(finding(
                "smell.time_dependency",
                Severity::Warning,
                case,
                external.span.clone(),
                "test case reads wall-clock time directly",
                external.expression.clone(),
            )),
            ExternalRefKind::GlobalState => findings.push(finding(
                "smell.order_dependency",
                Severity::Warning,
                case,
                external.span.clone(),
                "test case touches process-global state",
                external.expression.clone(),
            )),
        }
    }
}

fn detect_conditional_logic(case: &TestCase, findings: &mut Vec<Finding>) {
    for span in &case.control_flow {
        findings.push(finding(
            "smell.conditional_logic",
            Severity::Warning,
            case,
            span.clone(),
            "test body contains control flow",
            "conditional or loop detected",
        ));
    }
}

fn detect_ignored_test(case: &TestCase, findings: &mut Vec<Finding>) {
    for tag in &case.tags {
        if matches!(tag.kind, TagKind::Skipped | TagKind::Pending) {
            findings.push(finding(
                "smell.ignored_test",
                Severity::Info,
                case,
                tag.span.clone(),
                "test case is skipped or pending",
                tag.label.clone(),
            ));
        }
    }
}

fn detect_magic_number(case: &TestCase, rules: &RuleConfig, findings: &mut Vec<Finding>) {
    for assertion in &case.assertions {
        let Some(expected) = &assertion.expected_expr else {
            continue;
        };
        for number in numbers_in(expected) {
            if rules
                .magic_number_allowlist
                .iter()
                .any(|allowed| allowed == &number)
            {
                continue;
            }
            findings.push(finding(
                "smell.magic_number",
                Severity::Info,
                case,
                assertion.span.clone(),
                "assertion expected value contains an unexplained number",
                format!("expected expression `{expected}` includes `{number}`"),
            ));
        }
    }
}

fn detect_redundant_print(case: &TestCase, findings: &mut Vec<Finding>) {
    for span in &case.print_lines {
        findings.push(finding(
            "smell.redundant_print",
            Severity::Info,
            case,
            span.clone(),
            "test body contains debugging output",
            "puts/p/pp detected",
        ));
    }
}

fn detect_duplicate_assert(case: &TestCase, findings: &mut Vec<Finding>) {
    let mut seen = BTreeSet::<String>::new();
    for assertion in &case.assertions {
        let key = assertion_key(assertion);
        if !seen.insert(key.clone()) {
            findings.push(finding(
                "smell.duplicate_assert",
                Severity::Warning,
                case,
                assertion.span.clone(),
                "test case repeats an equivalent assertion",
                key,
            ));
        }
    }
}

fn detect_mock_overuse(case: &TestCase, rules: &RuleConfig, findings: &mut Vec<Finding>) {
    let sut_calls = case.sut_calls.len().max(1) as f64;
    let ratio = case.doubles.len() as f64 / sut_calls;
    if case.doubles.len() >= 3 && ratio > rules.mock_overuse_ratio {
        findings.push(finding(
            "smell.mock_overuse",
            Severity::Info,
            case,
            case.span.clone(),
            "test case uses many doubles relative to SUT calls",
            format!("{} doubles, ratio {:.2}", case.doubles.len(), ratio),
        ));
    }
}

fn file_smells(ir: &TestFileIr) -> Vec<Finding> {
    let eager_fixtures = ir
        .shared_fixtures
        .iter()
        .filter(|fixture| fixture.eager)
        .count();
    if eager_fixtures <= 3 || ir.case_count() == 0 {
        return Vec::new();
    }
    vec![Finding::new(
        "smell.general_fixture",
        Axis::Maintainability,
        Severity::Info,
        "file defines many eager shared fixtures",
        ir.shared_fixtures
            .first()
            .map(|fixture| fixture.span.clone()),
        format!("{eager_fixtures} eager fixtures may be unused by some cases"),
        None,
    )]
}

fn numbers_in(expression: &str) -> Vec<String> {
    expression
        .split(|character: char| {
            !(character.is_ascii_digit() || character == '-' || character == '.')
        })
        .filter(|token| !token.is_empty())
        .filter(|token| token.parse::<f64>().is_ok())
        .map(ToOwned::to_owned)
        .collect()
}

fn assertion_key(assertion: &Assertion) -> String {
    format!(
        "{}|{}|{}",
        assertion.kind.as_str(),
        assertion.subject_expr,
        assertion.expected_expr.clone().unwrap_or_default()
    )
}

fn finding(
    rule_id: &str,
    severity: Severity,
    case: &TestCase,
    span: SourceSpan,
    message: impl Into<String>,
    evidence: impl Into<String>,
) -> Finding {
    Finding::new(
        rule_id,
        Axis::Maintainability,
        severity,
        message,
        Some(span),
        evidence,
        Some(case.id.clone()),
    )
}

fn severity_penalty(severity: Severity) -> f64 {
    match severity {
        Severity::Info => 0.05,
        Severity::Warning => 0.15,
        Severity::Error => 0.35,
    }
}
