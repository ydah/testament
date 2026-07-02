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

    output
}

pub fn render_json(report: &ProjectReport) -> String {
    let mut output = String::new();
    output.push_str("{\n");
    output.push_str(&format!(
        "  \"passed\": {},\n  \"score\": {:.6},\n  \"files_analyzed\": {},\n  \"cases_analyzed\": {},\n  \"finding_count\": {},\n",
        report.passed,
        report.score(),
        report.files.len(),
        report.case_count(),
        report.finding_count()
    ));

    output.push_str("  \"files\": [\n");
    for (index, file) in report.files.iter().enumerate() {
        output.push_str(&render_file_json(file, 4));
        if index + 1 != report.files.len() {
            output.push(',');
        }
        output.push('\n');
    }
    output.push_str("  ],\n");

    output.push_str("  \"gates\": [\n");
    for (index, gate) in report.gates.iter().enumerate() {
        output.push_str(&render_gate_json(gate, 4));
        if index + 1 != report.gates.len() {
            output.push(',');
        }
        output.push('\n');
    }
    output.push_str("  ]\n");
    output.push_str("}\n");
    output
}

pub fn render_sarif(report: &ProjectReport) -> String {
    let mut output = String::new();
    output.push_str("{\n");
    output.push_str("  \"version\": \"2.1.0\",\n");
    output.push_str("  \"$schema\": \"https://json.schemastore.org/sarif-2.1.0.json\",\n");
    output.push_str("  \"runs\": [\n");
    output.push_str("    {\n");
    output.push_str(
        "      \"tool\": {\"driver\": {\"name\": \"testament\", \"informationUri\": \"https://example.invalid/testament\", \"rules\": [",
    );
    let rules = collect_rules(report);
    for (index, rule) in rules.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push_str(&format!(
            "{{\"id\":\"{}\",\"name\":\"{}\",\"shortDescription\":{{\"text\":\"{}\"}}}}",
            escape(rule),
            escape(rule),
            escape(rule)
        ));
    }
    output.push_str("]}},\n");
    output.push_str("      \"results\": [\n");
    let findings = report
        .files
        .iter()
        .flat_map(|file| file.findings.iter().map(move |finding| (file, finding)))
        .collect::<Vec<_>>();
    for (index, (file, finding)) in findings.iter().enumerate() {
        output.push_str("        {\n");
        output.push_str(&format!(
            "          \"ruleId\": \"{}\",\n",
            escape(&finding.rule_id)
        ));
        output.push_str(&format!(
            "          \"level\": \"{}\",\n",
            sarif_level(finding.severity)
        ));
        output.push_str(&format!(
            "          \"message\": {{\"text\": \"{}\"}},\n",
            escape(&finding.message)
        ));
        output.push_str("          \"locations\": [{\"physicalLocation\": {");
        output.push_str(&format!(
            "\"artifactLocation\": {{\"uri\": \"{}\"}}",
            escape(&file.ir.path_display())
        ));
        if let Some(span) = &finding.span {
            output.push_str(&format!(
                ", \"region\": {{\"startLine\": {}, \"endLine\": {}}}",
                span.start_line, span.end_line
            ));
        }
        output.push_str("}}]\n");
        output.push_str("        }");
        if index + 1 != findings.len() {
            output.push(',');
        }
        output.push('\n');
    }
    output.push_str("      ]\n");
    output.push_str("    }\n");
    output.push_str("  ]\n");
    output.push_str("}\n");
    output
}

pub fn render_junit(report: &ProjectReport) -> String {
    let failures = report
        .gates
        .iter()
        .filter(|gate| matches!(gate.level, GateLevel::Error))
        .count()
        + report
            .files
            .iter()
            .flat_map(|file| file.findings.iter())
            .filter(|finding| matches!(finding.severity, testament_core::Severity::Error))
            .count();
    let tests = report.files.len().max(1);
    let mut output = String::new();
    output.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    output.push_str(&format!(
        "<testsuite name=\"testament\" tests=\"{}\" failures=\"{}\">\n",
        tests, failures
    ));
    if report.files.is_empty() {
        output.push_str("  <testcase classname=\"testament\" name=\"no files\" />\n");
    }
    for file in &report.files {
        output.push_str(&format!(
            "  <testcase classname=\"testament\" name=\"{}\">\n",
            xml_escape(&file.ir.path_display())
        ));
        for finding in file
            .findings
            .iter()
            .filter(|finding| matches!(finding.severity, testament_core::Severity::Error))
        {
            output.push_str(&format!(
                "    <failure message=\"{}\">{}</failure>\n",
                xml_escape(&finding.rule_id),
                xml_escape(&finding.message)
            ));
        }
        output.push_str("  </testcase>\n");
    }
    output.push_str("</testsuite>\n");
    output
}

fn render_file_json(file: &FileReport, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let child = " ".repeat(indent + 2);
    let mut output = String::new();
    output.push_str(&format!("{pad}{{\n"));
    output.push_str(&format!(
        "{child}\"path\": \"{}\",\n",
        escape(&file.ir.path_display())
    ));
    output.push_str(&format!(
        "{child}\"language\": \"{}\",\n",
        escape(&file.ir.language)
    ));
    output.push_str(&format!(
        "{child}\"framework\": \"{}\",\n",
        escape(&file.ir.framework)
    ));
    output.push_str(&format!(
        "{child}\"confidence\": \"{}\",\n",
        file.ir.confidence.as_str()
    ));
    output.push_str(&format!("{child}\"score\": {:.6},\n", file.score));
    output.push_str(&format!(
        "{child}\"case_count\": {},\n",
        file.ir.case_count()
    ));
    output.push_str(&format!(
        "{child}\"assertion_count\": {},\n",
        file.ir.assertion_count()
    ));
    output.push_str(&format!("{child}\"axis_scores\": {{"));
    output.push_str(&axis_scores_json(file));
    output.push_str("},\n");

    output.push_str(&format!("{child}\"metrics\": [\n"));
    for (index, outcome) in file.outcomes.iter().enumerate() {
        output.push_str(&render_metric_json(outcome, indent + 4));
        if index + 1 != file.outcomes.len() {
            output.push(',');
        }
        output.push('\n');
    }
    output.push_str(&format!("{child}],\n"));

    output.push_str(&format!("{child}\"findings\": [\n"));
    for (index, finding) in file.findings.iter().enumerate() {
        output.push_str(&render_finding_json(finding, indent + 4));
        if index + 1 != file.findings.len() {
            output.push(',');
        }
        output.push('\n');
    }
    output.push_str(&format!("{child}]\n"));
    output.push_str(&format!("{pad}}}"));
    output
}

fn render_metric_json(outcome: &MetricOutcome, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let child = " ".repeat(indent + 2);
    let mut output = String::new();
    output.push_str(&format!("{pad}{{\n"));
    output.push_str(&format!("{child}\"id\": \"{}\",\n", escape(&outcome.id)));
    output.push_str(&format!(
        "{child}\"axis\": \"{}\",\n",
        outcome.axis.as_str()
    ));
    if let Some(score) = outcome.score {
        output.push_str(&format!("{child}\"score\": {:.6},\n", score));
    } else {
        output.push_str(&format!("{child}\"score\": null,\n"));
    }
    output.push_str(&format!("{child}\"value\": {:.6},\n", outcome.value));
    output.push_str(&format!(
        "{child}\"unit\": \"{}\",\n",
        escape(&outcome.unit)
    ));
    output.push_str(&format!(
        "{child}\"summary\": \"{}\",\n",
        escape(&outcome.summary)
    ));
    output.push_str(&format!(
        "{child}\"provenance\": {{\"references\": [{}], \"definition\": \"{}\", \"approximation\": \"{}\"}}\n",
        outcome
            .provenance
            .references
            .iter()
            .map(|reference| format!("\"{}\"", escape(reference)))
            .collect::<Vec<_>>()
            .join(", "),
        escape(&outcome.provenance.definition),
        escape(&outcome.provenance.approximation)
    ));
    output.push_str(&format!("{pad}}}"));
    output
}

fn render_finding_json(finding: &Finding, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let child = " ".repeat(indent + 2);
    let mut output = String::new();
    output.push_str(&format!("{pad}{{\n"));
    output.push_str(&format!(
        "{child}\"rule_id\": \"{}\",\n",
        escape(&finding.rule_id)
    ));
    output.push_str(&format!(
        "{child}\"axis\": \"{}\",\n",
        finding.axis.as_str()
    ));
    output.push_str(&format!(
        "{child}\"severity\": \"{}\",\n",
        finding.severity.as_str()
    ));
    output.push_str(&format!(
        "{child}\"message\": \"{}\",\n",
        escape(&finding.message)
    ));
    output.push_str(&format!(
        "{child}\"evidence\": \"{}\"",
        escape(&finding.evidence)
    ));
    if let Some(span) = &finding.span {
        output.push_str(&format!(
            ",\n{child}\"span\": {{\"start_line\": {}, \"end_line\": {}}}",
            span.start_line, span.end_line
        ));
    }
    if let Some(case_id) = &finding.case_id {
        output.push_str(&format!(",\n{child}\"case_id\": \"{}\"", escape(case_id)));
    }
    output.push('\n');
    output.push_str(&format!("{pad}}}"));
    output
}

fn render_gate_json(gate: &GateViolation, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let child = " ".repeat(indent + 2);
    let mut output = String::new();
    output.push_str(&format!("{pad}{{\n"));
    output.push_str(&format!(
        "{child}\"metric_id\": \"{}\",\n",
        escape(&gate.metric_id)
    ));
    output.push_str(&format!("{child}\"path\": \"{}\",\n", escape(&gate.path)));
    output.push_str(&format!("{child}\"level\": \"{}\",\n", gate.level.as_str()));
    output.push_str(&format!(
        "{child}\"direction\": \"{}\",\n",
        gate.direction.as_str()
    ));
    output.push_str(&format!("{child}\"observed\": {:.6},\n", gate.observed));
    output.push_str(&format!("{child}\"threshold\": {:.6},\n", gate.threshold));
    output.push_str(&format!(
        "{child}\"message\": \"{}\"\n",
        escape(&gate.message)
    ));
    output.push_str(&format!("{pad}}}"));
    output
}

fn axis_scores_json(file: &FileReport) -> String {
    file.axis_scores
        .iter()
        .map(|(axis, score)| format!("\"{}\": {:.6}", escape(axis), score))
        .collect::<Vec<_>>()
        .join(", ")
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

fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character => escaped.push(character),
        }
    }
    escaped
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
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

    use testament_core::AppConfig;
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
    }
}
