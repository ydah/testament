use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use serde_json::{Value, json};
use testament_core::{
    Axis, FileReport, Finding, GateLevel, GateViolation, MetricOutcome, ProjectReport,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReportFormat {
    Tty,
    Json,
    Markdown,
    Sarif,
    Junit,
}

impl ReportFormat {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "tty" | "human" => Some(Self::Tty),
            "json" => Some(Self::Json),
            "md" | "markdown" => Some(Self::Markdown),
            "sarif" => Some(Self::Sarif),
            "junit" | "junit-xml" => Some(Self::Junit),
            _ => None,
        }
    }
}

pub fn render(report: &ProjectReport, format: ReportFormat) -> String {
    match format {
        ReportFormat::Tty => render_tty(report),
        ReportFormat::Json => render_json(report),
        ReportFormat::Markdown => render_markdown(report),
        ReportFormat::Sarif => render_sarif(report),
        ReportFormat::Junit => render_junit(report),
    }
}

pub fn render_tty(report: &ProjectReport) -> String {
    let mut output = String::new();
    let status = if report.passed { "PASS" } else { "FAIL" };
    output.push_str(&format!(
        "testament {status} score={:.3} files={} cases={} findings={}\n",
        report.score(),
        report.files.len(),
        report.case_count(),
        report.finding_count()
    ));

    for file in &report.files {
        output.push_str(&format!(
            "\n{} score={:.3} framework={} cases={}\n",
            file.ir.path_display(),
            file.score,
            file.ir.framework,
            file.ir.case_count()
        ));
        for (axis, score) in &file.axis_scores {
            output.push_str(&format!("  {axis}: {:.3}\n", score));
        }
        for finding in &file.findings {
            output.push_str(&format!("  {}\n", format_finding(finding)));
        }
    }

    if !report.gates.is_empty() {
        output.push_str("\nGates\n");
        for violation in &report.gates {
            output.push_str(&format!("  {}\n", format_gate(violation)));
        }
    }

    if !report.warnings.is_empty() {
        output.push_str("\nWarnings\n");
        for warning in &report.warnings {
            output.push_str(&format!("  {warning}\n"));
        }
    }

    output
}

pub fn render_markdown(report: &ProjectReport) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "# Testament Report\n\n- Status: {}\n- Score: {:.3}\n- Files: {}\n- Cases: {}\n- Findings: {}\n\n",
        if report.passed { "PASS" } else { "FAIL" },
        report.score(),
        report.files.len(),
        report.case_count(),
        report.finding_count()
    ));

    output.push_str("| File | Score | Adequacy | Redundancy | Maintainability | Findings |\n");
    output.push_str("|---|---:|---:|---:|---:|---:|\n");
    for file in &report.files {
        output.push_str(&format!(
            "| `{}` | {:.3} | {} | {} | {} | {} |\n",
            file.ir.path_display(),
            file.score,
            axis_score(file, Axis::Adequacy),
            axis_score(file, Axis::Redundancy),
            axis_score(file, Axis::Maintainability),
            file.findings.len()
        ));
    }

    if !report.gates.is_empty() {
        output.push_str("\n## Gates\n\n");
        for violation in &report.gates {
            output.push_str(&format!("- {}\n", format_gate(violation)));
        }
    }
    if !report.warnings.is_empty() {
        output.push_str("\n## Warnings\n\n");
        for warning in &report.warnings {
            output.push_str(&format!("- {warning}\n"));
        }
    }

    output
}

pub fn render_json(report: &ProjectReport) -> String {
    let value = json!({
        "passed": report.passed,
        "score": report.score(),
        "files_analyzed": report.files.len(),
        "cases_analyzed": report.case_count(),
        "finding_count": report.finding_count(),
        "warnings": report.warnings,
        "files": report.files.iter().map(file_json).collect::<Vec<_>>(),
        "gates": report.gates.iter().map(gate_json).collect::<Vec<_>>(),
    });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&value).expect("JSON value is serializable")
    )
}

pub fn render_baseline(report: &ProjectReport) -> String {
    let files = report
        .files
        .iter()
        .map(|file| {
            json!({
                "path": file.ir.path_display(),
                "score": file.score,
                "metrics": file
                    .outcomes
                    .iter()
                    .map(|outcome| json!({ "id": outcome.id }))
                    .collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    let value = json!({ "version": 1, "files": files });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&value).expect("baseline JSON value is serializable")
    )
}

pub fn render_sarif(report: &ProjectReport) -> String {
    let rules = collect_rules(report)
        .into_iter()
        .map(|rule| {
            json!({
                "id": rule,
                "name": rule,
                "shortDescription": { "text": rule },
            })
        })
        .collect::<Vec<_>>();
    let results = report
        .files
        .iter()
        .flat_map(|file| file.findings.iter().map(move |finding| (file, finding)))
        .map(|(file, finding)| {
            let mut physical_location = json!({
                "artifactLocation": { "uri": file.ir.path_display() }
            });
            if let Some(span) = &finding.span {
                physical_location["region"] = json!({
                    "startLine": span.start_line,
                    "endLine": span.end_line,
                });
            }
            json!({
                "ruleId": finding.rule_id,
                "level": sarif_level(finding.severity),
                "message": { "text": finding.message },
                "locations": [{ "physicalLocation": physical_location }],
            })
        })
        .collect::<Vec<_>>();
    let value = json!({
        "version": "2.1.0",
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "runs": [{
            "tool": { "driver": {
                "name": "testament",
                "informationUri": "https://github.com/ydah/testament",
                "rules": rules,
            }},
            "results": results,
        }],
    });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&value).expect("SARIF value is serializable")
    )
}

pub fn render_junit(report: &ProjectReport) -> String {
    let gate_failures = report
        .gates
        .iter()
        .filter(|gate| matches!(gate.level, GateLevel::Error))
        .collect::<Vec<_>>();
    let file_failures = report
        .files
        .iter()
        .filter(|file| {
            file.findings
                .iter()
                .any(|finding| matches!(finding.severity, testament_core::Severity::Error))
        })
        .count();
    let failures = gate_failures.len() + file_failures;
    let tests = (report.files.len() + gate_failures.len()).max(1);
    let mut writer = Writer::new_with_indent(Vec::new(), b' ', 2);
    writer
        .write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))
        .expect("writing XML to memory cannot fail");
    let tests_value = tests.to_string();
    let failures_value = failures.to_string();
    let mut test_suite = BytesStart::new("testsuite");
    test_suite.push_attribute(("name", "testament"));
    test_suite.push_attribute(("tests", tests_value.as_str()));
    test_suite.push_attribute(("failures", failures_value.as_str()));
    writer
        .write_event(Event::Start(test_suite))
        .expect("writing XML to memory cannot fail");
    if report.files.is_empty() {
        write_empty_test_case(&mut writer, "no files");
    }
    for file in &report.files {
        let mut test_case = BytesStart::new("testcase");
        let name = file.ir.path_display();
        test_case.push_attribute(("classname", "testament"));
        test_case.push_attribute(("name", name.as_str()));
        writer
            .write_event(Event::Start(test_case))
            .expect("writing XML to memory cannot fail");
        let error_findings = file
            .findings
            .iter()
            .filter(|finding| matches!(finding.severity, testament_core::Severity::Error))
            .collect::<Vec<_>>();
        if !error_findings.is_empty() {
            let message = format!("{} error finding(s)", error_findings.len());
            let body = error_findings
                .iter()
                .map(|finding| format!("{}: {}", finding.rule_id, finding.message))
                .collect::<Vec<_>>()
                .join("\n");
            write_failure(&mut writer, &message, &body);
        }
        writer
            .write_event(Event::End(BytesEnd::new("testcase")))
            .expect("writing XML to memory cannot fail");
    }
    for gate in gate_failures {
        let mut test_case = BytesStart::new("testcase");
        let name = format!("gate:{}:{}", gate.metric_id, gate.path);
        test_case.push_attribute(("classname", "testament"));
        test_case.push_attribute(("name", name.as_str()));
        writer
            .write_event(Event::Start(test_case))
            .expect("writing XML to memory cannot fail");
        write_failure(&mut writer, &gate.metric_id, &gate.message);
        writer
            .write_event(Event::End(BytesEnd::new("testcase")))
            .expect("writing XML to memory cannot fail");
    }
    writer
        .write_event(Event::End(BytesEnd::new("testsuite")))
        .expect("writing XML to memory cannot fail");
    String::from_utf8(writer.into_inner()).expect("quick-xml emits UTF-8")
}

fn write_empty_test_case(writer: &mut Writer<Vec<u8>>, name: &str) {
    let mut test_case = BytesStart::new("testcase");
    test_case.push_attribute(("classname", "testament"));
    test_case.push_attribute(("name", name));
    writer
        .write_event(Event::Empty(test_case))
        .expect("writing XML to memory cannot fail");
}

fn write_failure(writer: &mut Writer<Vec<u8>>, message: &str, body: &str) {
    let mut failure = BytesStart::new("failure");
    failure.push_attribute(("message", message));
    writer
        .write_event(Event::Start(failure))
        .expect("writing XML to memory cannot fail");
    writer
        .write_event(Event::Text(BytesText::new(body)))
        .expect("writing XML to memory cannot fail");
    writer
        .write_event(Event::End(BytesEnd::new("failure")))
        .expect("writing XML to memory cannot fail");
}

fn file_json(file: &FileReport) -> Value {
    json!({
        "path": file.ir.path_display(),
        "language": file.ir.language,
        "framework": file.ir.framework,
        "confidence": file.ir.confidence.as_str(),
        "score": file.score,
        "case_count": file.ir.case_count(),
        "assertion_count": file.ir.assertion_count(),
        "axis_scores": file.axis_scores,
        "metrics": file.outcomes.iter().map(metric_json).collect::<Vec<_>>(),
        "findings": file.findings.iter().map(finding_json).collect::<Vec<_>>(),
    })
}

fn metric_json(outcome: &MetricOutcome) -> Value {
    json!({
        "id": outcome.id,
        "axis": outcome.axis.as_str(),
        "score": outcome.score,
        "value": outcome.value,
        "unit": outcome.unit,
        "summary": outcome.summary,
        "provenance": {
            "references": outcome.provenance.references,
            "definition": outcome.provenance.definition,
            "approximation": outcome.provenance.approximation,
        },
    })
}

fn finding_json(finding: &Finding) -> Value {
    let mut value = json!({
        "rule_id": finding.rule_id,
        "axis": finding.axis.as_str(),
        "severity": finding.severity.as_str(),
        "message": finding.message,
        "evidence": finding.evidence,
    });
    if let Some(span) = &finding.span {
        value["span"] = json!({
            "start_line": span.start_line,
            "end_line": span.end_line,
        });
    }
    if let Some(case_id) = &finding.case_id {
        value["case_id"] = json!(case_id);
    }
    value
}

fn gate_json(gate: &GateViolation) -> Value {
    json!({
        "metric_id": gate.metric_id,
        "path": gate.path,
        "level": gate.level.as_str(),
        "direction": gate.direction.as_str(),
        "observed": gate.observed,
        "threshold": gate.threshold,
        "message": gate.message,
    })
}

fn axis_score(file: &FileReport, axis: Axis) -> String {
    file.axis_scores
        .get(axis.as_str())
        .map(|score| format!("{score:.3}"))
        .unwrap_or_else(|| "-".to_owned())
}

fn format_finding(finding: &Finding) -> String {
    let location = finding
        .span
        .as_ref()
        .map(|span| format!(":{}", span.start_line))
        .unwrap_or_default();
    format!(
        "[{}] {}{} {} ({})",
        finding.severity.as_str(),
        finding.rule_id,
        location,
        finding.message,
        finding.evidence
    )
}

fn format_gate(gate: &GateViolation) -> String {
    let level = match gate.level {
        GateLevel::Error => "error",
        GateLevel::Warn => "warn",
    };
    format!(
        "[{level}] {} {} observed={:.3} threshold={:.3}: {}",
        gate.path, gate.metric_id, gate.observed, gate.threshold, gate.message
    )
}

fn sarif_level(severity: testament_core::Severity) -> &'static str {
    match severity {
        testament_core::Severity::Info => "note",
        testament_core::Severity::Warning => "warning",
        testament_core::Severity::Error => "error",
    }
}

fn collect_rules(report: &ProjectReport) -> Vec<String> {
    let mut rules = report
        .files
        .iter()
        .flat_map(|file| file.findings.iter().map(|finding| finding.rule_id.clone()))
        .collect::<Vec<_>>();
    rules.sort();
    rules.dedup();
    rules
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use testament_core::{AppConfig, parse_baseline_files};
    use testament_metrics::{analyze_content, evaluate_project};

    use super::*;

    #[test]
    fn renders_json_with_file_scores() {
        let file = analyze_content(
            Path::new("spec/a_spec.rb"),
            r#"
            RSpec.describe A do
              it "works" do
                expect(1).to eq(1)
              end
            end
            "#,
            &AppConfig::default(),
        );
        let report = evaluate_project(vec![file], &AppConfig::default());
        let json = render_json(&report);

        assert!(json.contains("\"path\": \"spec/a_spec.rb\""));
        assert!(json.contains("\"score\":"));
        assert!(json.contains("\"metrics\":"));
    }

    #[test]
    fn renders_sarif_and_junit() {
        let file = analyze_content(
            Path::new("spec/a_spec.rb"),
            r#"
            RSpec.describe A do
              it "does nothing" do
              end
            end
            "#,
            &AppConfig::default(),
        );
        let report = evaluate_project(vec![file], &AppConfig::default());

        let sarif = render_sarif(&report);
        let junit = render_junit(&report);

        assert!(sarif.contains("\"version\": \"2.1.0\""));
        assert!(sarif.contains("smell.unknown_test"));
        assert!(junit.contains("<testsuite"));
        if report
            .gates
            .iter()
            .any(|gate| gate.level == GateLevel::Error)
        {
            assert!(junit.contains("name=\"gate:"));
        }
        assert_eq!(
            junit.matches("<failure ").count(),
            report
                .gates
                .iter()
                .filter(|gate| gate.level == GateLevel::Error)
                .count()
                + report
                    .files
                    .iter()
                    .filter(|file| {
                        file.findings
                            .iter()
                            .any(|finding| finding.severity == testament_core::Severity::Error)
                    })
                    .count()
        );
    }

    #[test]
    fn junit_counts_failed_test_cases_not_individual_findings() {
        let mut file = analyze_content(
            Path::new("spec/a_spec.rb"),
            "RSpec.describe(A) { it(\"works\") { expect(1).to eq(1) } }",
            &AppConfig::default(),
        );
        for rule in ["first", "second"] {
            file.findings.push(Finding::new(
                rule,
                Axis::Maintainability,
                testament_core::Severity::Error,
                "failed",
                None,
                "evidence",
                None,
            ));
        }
        let report = ProjectReport {
            files: vec![file],
            gates: Vec::new(),
            warnings: Vec::new(),
            passed: false,
        };

        let junit = render_junit(&report);

        assert!(junit.contains("tests=\"1\""));
        assert!(junit.contains("failures=\"1\""));
        assert_eq!(junit.matches("<failure ").count(), 1);
    }

    #[test]
    fn json_escapes_all_control_characters() {
        let mut file = analyze_content(
            Path::new("spec/a_spec.rb"),
            "RSpec.describe A do\n  it(\"works\") { expect(1).to eq(1) }\nend",
            &AppConfig::default(),
        );
        file.findings.push(Finding::new(
            "control",
            Axis::Maintainability,
            testament_core::Severity::Info,
            "contains control",
            None,
            "bad\u{0001}value",
            None,
        ));
        let report = evaluate_project(vec![file], &AppConfig::default());
        let json = render_json(&report);

        assert!(serde_json::from_str::<serde_json::Value>(&json).is_ok());
    }

    #[test]
    fn baseline_contains_only_ratchet_data_and_metric_signature() {
        let file = analyze_content(
            Path::new("spec/a_spec.rb"),
            "RSpec.describe(A) { it(\"works\") { expect(1).to eq(1) } }",
            &AppConfig::default(),
        );
        let report = evaluate_project(vec![file], &AppConfig::default());
        let baseline = render_baseline(&report);
        let value: serde_json::Value = serde_json::from_str(&baseline).unwrap();

        assert!(value.get("gates").is_none());
        assert!(
            !parse_baseline_files(&baseline)["spec/a_spec.rb"]
                .metric_ids
                .is_empty()
        );
    }

    #[test]
    fn junit_is_well_formed_for_special_characters() {
        let mut file = analyze_content(
            Path::new("spec/a&b_spec.rb"),
            "RSpec.describe A do\n  it \"empty\" do\n  end\nend",
            &AppConfig::default(),
        );
        file.findings[0].message = "bad <value> & output".to_owned();
        let report = evaluate_project(vec![file], &AppConfig::default());
        let junit = render_junit(&report);
        let mut reader = quick_xml::Reader::from_str(&junit);

        loop {
            if reader.read_event().unwrap() == Event::Eof {
                break;
            }
        }
    }

    #[test]
    fn json_converts_non_finite_numbers_to_null() {
        let mut file = analyze_content(
            Path::new("spec/a_spec.rb"),
            "RSpec.describe(A) { it(\"works\") { expect(1).to eq(1) } }",
            &AppConfig::default(),
        );
        file.score = f64::NAN;
        file.outcomes[0].value = f64::INFINITY;
        let report = evaluate_project(vec![file], &AppConfig::default());
        let json = render_json(&report);
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(value["score"].is_null());
        assert!(value["files"][0]["metrics"][0]["value"].is_null());
    }
}
