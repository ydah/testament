mod adequacy;
mod redundancy;
mod smells;

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use testament_core::{
    axis_average, evaluate_gates, AppConfig, Axis, EvidenceSet, FileReport, MetricOutcome,
    TestFileIr,
};

pub use testament_lang_ruby::RubyAdapter;

pub fn analyze_file(path: &Path, config: &AppConfig) -> io::Result<FileReport> {
    let content = fs::read_to_string(path)?;
    Ok(analyze_content(path, &content, config))
}

pub fn analyze_content(path: &Path, content: &str, config: &AppConfig) -> FileReport {
    let ir = RubyAdapter::lower(path, content);
    analyze_ir(ir, config)
}

pub fn analyze_ir(ir: TestFileIr, config: &AppConfig) -> FileReport {
    analyze_ir_with_evidence(ir, config, &EvidenceSet::default())
}

pub fn analyze_file_with_evidence(
    path: &Path,
    config: &AppConfig,
    evidence: &EvidenceSet,
) -> io::Result<FileReport> {
    let content = fs::read_to_string(path)?;
    Ok(analyze_content_with_evidence(
        path, &content, config, evidence,
    ))
}

pub fn analyze_content_with_evidence(
    path: &Path,
    content: &str,
    config: &AppConfig,
    evidence: &EvidenceSet,
) -> FileReport {
    let ir = RubyAdapter::lower(path, content);
    analyze_ir_with_evidence(ir, config, evidence)
}

pub fn analyze_ir_with_evidence(
    ir: TestFileIr,
    config: &AppConfig,
    evidence: &EvidenceSet,
) -> FileReport {
    let mut outcomes = Vec::new();
    outcomes.extend(adequacy::compute(&ir, evidence));
    outcomes.push(smells::compute(&ir, &config.rules));
    outcomes.extend(redundancy::compute(&ir, &config.rules, evidence));

    let findings = outcomes
        .iter()
        .flat_map(|outcome| outcome.findings.iter().cloned())
        .collect::<Vec<_>>();
    let axis_scores = compute_axis_scores(&outcomes);
    let score = aggregate_score(&axis_scores);

    FileReport {
        ir,
        outcomes,
        axis_scores,
        score,
        findings,
    }
}

pub fn analyze_paths(
    paths: &[std::path::PathBuf],
    config: &AppConfig,
) -> io::Result<Vec<FileReport>> {
    paths
        .iter()
        .map(|path| analyze_file(path, config))
        .collect()
}

pub fn analyze_paths_with_evidence(
    paths: &[std::path::PathBuf],
    config: &AppConfig,
    evidence: &EvidenceSet,
) -> io::Result<Vec<FileReport>> {
    paths
        .iter()
        .map(|path| analyze_file_with_evidence(path, config, evidence))
        .collect()
}

pub fn evaluate_project(
    files: Vec<FileReport>,
    config: &AppConfig,
) -> testament_core::ProjectReport {
    let gate_eval = evaluate_gates(config, &files);
    testament_core::ProjectReport {
        files,
        passed: gate_eval.passed,
        gates: gate_eval.violations,
    }
}

fn compute_axis_scores(outcomes: &[MetricOutcome]) -> BTreeMap<String, f64> {
    let mut axis_scores = BTreeMap::new();
    for axis in [Axis::Adequacy, Axis::Redundancy, Axis::Maintainability] {
        if let Some(score) = axis_average(outcomes, axis) {
            axis_scores.insert(axis.as_str().to_owned(), score);
        }
    }
    axis_scores
}

fn aggregate_score(axis_scores: &BTreeMap<String, f64>) -> f64 {
    let adequacy = axis_scores.get("adequacy").copied().unwrap_or(1.0);
    let redundancy = axis_scores.get("redundancy").copied().unwrap_or(1.0);
    let maintainability = axis_scores.get("maintainability").copied().unwrap_or(1.0);
    ((adequacy * 0.40) + (redundancy * 0.20) + (maintainability * 0.40)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn analyzes_rspec_file_into_scored_report() {
        let report = analyze_content(
            Path::new("spec/user_spec.rb"),
            r#"
            RSpec.describe User do
              it "creates active users" do
                user = described_class.create!(name: "Y", age: 18)
                expect(user.active?).to eq(true)
              end
            end
            "#,
            &AppConfig::default(),
        );

        assert_eq!(report.ir.framework, "rspec");
        assert_eq!(report.ir.case_count(), 1);
        assert!(report.metric_score("adequacy.assertion_density").is_some());
        assert!(report.score > 0.0);
    }
}
