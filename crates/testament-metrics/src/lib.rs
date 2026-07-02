mod adequacy;
mod redundancy;
mod smells;

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use testament_adapter_api::{FrameworkAdapter, LanguageAdapter};
use testament_core::{
    AppConfig, Axis, Confidence, EvidenceSet, FileReport, MetricOutcome, TestFileIr, axis_average,
    evaluate_gates,
};

pub use testament_lang_ruby::RubyAdapter;

pub fn analyze_file(path: &Path, config: &AppConfig) -> io::Result<FileReport> {
    let content = fs::read_to_string(path)?;
    Ok(analyze_content(path, &content, config))
}

pub fn analyze_content(path: &Path, content: &str, config: &AppConfig) -> FileReport {
    let ir = lower_ruby(path, content);
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
    let ir = lower_ruby(path, content);
    analyze_ir_with_evidence(ir, config, evidence)
}

fn lower_ruby(path: &Path, content: &str) -> TestFileIr {
    let adapter = RubyAdapter;
    adapter
        .parse(content.as_bytes())
        .and_then(|tree| FrameworkAdapter::lower(&adapter, &tree, path))
        .unwrap_or_else(|_| unresolved_ruby_ir(path))
}

fn unresolved_ruby_ir(path: &Path) -> TestFileIr {
    let mut ir = TestFileIr::new(path, "ruby", "ruby");
    ir.confidence = Confidence::Unresolved;
    ir
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
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};
    use testament_core::{
        CoverageEvidence, CoverageRequirement, EvidenceSet, FileCoverage, MutationEvidence,
        PerTestCoverageEvidence,
    };

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

    #[test]
    fn computes_coverage_and_mutation_metrics_when_evidence_exists() {
        let mut coverage = CoverageEvidence::default();
        coverage.files.insert(
            "spec/user_spec.rb".to_owned(),
            FileCoverage {
                line_rate: Some(0.8),
                branch_rate: Some(0.6),
                covered_lines: [4].into_iter().collect(),
                executable_lines: [3, 4, 5].into_iter().collect(),
            },
        );
        let evidence = EvidenceSet {
            coverage: Some(coverage),
            mutation: Some(MutationEvidence {
                total: 10,
                killed: 7,
                equivalent_marked: 0,
                per_test_kills: BTreeMap::new(),
            }),
            ..EvidenceSet::default()
        };

        let report = analyze_content_with_evidence(
            Path::new("spec/user_spec.rb"),
            r#"
            RSpec.describe User do
              it "works" do
                expect(1).to eq(1)
              end
            end
            "#,
            &AppConfig::default(),
            &evidence,
        );

        assert_eq!(report.metric_value("adequacy.line_coverage"), Some(0.8));
        assert_eq!(report.metric_value("adequacy.branch_coverage"), Some(0.6));
        assert_eq!(report.metric_value("adequacy.mutation_score"), Some(0.7));
    }

    #[test]
    fn computes_per_test_coverage_redundancy() {
        let first_case = testament_core::stable_test_id(
            &PathBuf::from("spec/cart_spec.rb"),
            "Cart",
            "counts one item",
            3,
        );
        let second_case = testament_core::stable_test_id(
            &PathBuf::from("spec/cart_spec.rb"),
            "Cart",
            "counts another item",
            8,
        );
        let mut cases = BTreeMap::new();
        cases.insert(
            first_case,
            [
                CoverageRequirement {
                    path: "lib/cart.rb".to_owned(),
                    line: 1,
                },
                CoverageRequirement {
                    path: "lib/cart.rb".to_owned(),
                    line: 2,
                },
            ]
            .into_iter()
            .collect::<BTreeSet<_>>(),
        );
        cases.insert(
            second_case,
            [CoverageRequirement {
                path: "lib/cart.rb".to_owned(),
                line: 1,
            }]
            .into_iter()
            .collect::<BTreeSet<_>>(),
        );

        let evidence = EvidenceSet {
            per_test_coverage: Some(PerTestCoverageEvidence { cases }),
            ..EvidenceSet::default()
        };
        let report = analyze_content_with_evidence(
            Path::new("spec/cart_spec.rb"),
            r#"
            RSpec.describe Cart do
              it "counts one item" do
                expect(cart.count).to eq(1)
              end

              it "counts another item" do
                expect(cart.count).to eq(1)
              end
            end
            "#,
            &AppConfig::default(),
            &evidence,
        );

        assert!(
            report
                .metric_value("redundancy.coverage_subsumption")
                .unwrap()
                > 0.0
        );
    }
}
