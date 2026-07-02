mod adapters;
mod adequacy;
mod cache;
mod redundancy;
mod smells;

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use testament_core::{
    AppConfig, Axis, EvidenceSet, FileReport, MetricOutcome, TestFileIr, axis_average,
    evaluate_gates,
};

pub use adapters::AdapterRegistry;
pub use testament_lang_ruby::RubyAdapter;

pub fn analyze_file(path: &Path, config: &AppConfig) -> io::Result<FileReport> {
    let content = fs::read_to_string(path)?;
    let ir = lower_file_content(path, &content);
    Ok(analyze_ir(ir, config))
}

pub fn analyze_content(path: &Path, content: &str, config: &AppConfig) -> FileReport {
    let ir = AdapterRegistry::builtin().lower(path, content);
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
    let ir = lower_file_content(path, &content);
    Ok(analyze_ir_with_evidence(ir, config, evidence))
}

pub fn analyze_content_with_evidence(
    path: &Path,
    content: &str,
    config: &AppConfig,
    evidence: &EvidenceSet,
) -> FileReport {
    let ir = AdapterRegistry::builtin().lower(path, content);
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

pub fn analyze_paths(paths: &[PathBuf], config: &AppConfig) -> io::Result<Vec<FileReport>> {
    paths
        .iter()
        .map(|path| analyze_file(path, config))
        .collect()
}

pub fn analyze_paths_with_evidence(
    paths: &[PathBuf],
    config: &AppConfig,
    evidence: &EvidenceSet,
) -> io::Result<Vec<FileReport>> {
    if paths.len() <= 1 {
        return paths
            .iter()
            .map(|path| analyze_file_with_cache(path, config, evidence))
            .collect();
    }

    let config = Arc::new(config.clone());
    let evidence = Arc::new(evidence.clone());
    thread::scope(|scope| {
        let handles = paths
            .iter()
            .cloned()
            .map(|path| {
                let config = Arc::clone(&config);
                let evidence = Arc::clone(&evidence);
                scope.spawn(move || analyze_file_with_cache(&path, &config, &evidence))
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| io::Error::other("analysis worker panicked"))?
            })
            .collect()
    })
}

pub fn metric_catalog(config: &AppConfig) -> Vec<MetricOutcome> {
    let path = Path::new("spec/example_spec.rb");
    let content = r#"
RSpec.describe Example do
  it "works" do
    example = described_class.new
    expect(example.value).to eq(1)
  end
end
"#;
    let mut coverage = testament_core::CoverageEvidence::default();
    coverage.files.insert(
        "lib/example.rb".to_owned(),
        testament_core::FileCoverage {
            line_rate: Some(0.8),
            branch_rate: Some(0.6),
            covered_lines: [1, 2, 3, 4].into_iter().collect(),
            executable_lines: [1, 2, 3, 4, 5].into_iter().collect(),
        },
    );
    let evidence = EvidenceSet {
        coverage: Some(coverage),
        mutation: Some(testament_core::MutationEvidence {
            total: 4,
            killed: 3,
            equivalent_marked: 0,
            per_test_kills: [(
                "catalog-case".to_owned(),
                ["m1".to_owned()].into_iter().collect(),
            )]
            .into_iter()
            .collect(),
        }),
        per_test_coverage: Some(testament_core::PerTestCoverageEvidence {
            cases: [(
                "catalog-case".to_owned(),
                [testament_core::CoverageRequirement {
                    path: "lib/example.rb".to_owned(),
                    line: 1,
                }]
                .into_iter()
                .collect(),
            )]
            .into_iter()
            .collect(),
        }),
        ..EvidenceSet::default()
    };

    analyze_content_with_evidence(path, content, config, &evidence).outcomes
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

fn analyze_file_with_cache(
    path: &Path,
    config: &AppConfig,
    evidence: &EvidenceSet,
) -> io::Result<FileReport> {
    analyze_file_with_evidence(path, config, evidence)
}

fn lower_file_content(path: &Path, content: &str) -> TestFileIr {
    let ir = cache::read_ir(path, content)
        .unwrap_or_else(|| AdapterRegistry::builtin().lower(path, content));
    cache::write_ir(path, content, &ir);
    ir
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;
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
            "lib/user.rb".to_owned(),
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
        let mut cases = BTreeMap::new();
        cases.insert(
            "Cart counts one item".to_owned(),
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
            "Cart counts another item".to_owned(),
            [CoverageRequirement {
                path: "lib/cart.rb".to_owned(),
                line: 1,
            }]
            .into_iter()
            .collect::<BTreeSet<_>>(),
        );

        let evidence = EvidenceSet {
            per_test_coverage: Some(PerTestCoverageEvidence { cases }),
            mutation: Some(MutationEvidence {
                total: 2,
                killed: 2,
                equivalent_marked: 0,
                per_test_kills: [
                    (
                        "Cart counts one item".to_owned(),
                        ["m1".to_owned(), "m2".to_owned()].into_iter().collect(),
                    ),
                    (
                        "Cart counts another item".to_owned(),
                        ["m1".to_owned()].into_iter().collect(),
                    ),
                ]
                .into_iter()
                .collect(),
            }),
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

    #[test]
    fn maps_probe_case_names_to_ir_case_ids() {
        let mut cases = BTreeMap::new();
        cases.insert(
            "Cart counts one item".to_owned(),
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
            "Cart counts another item".to_owned(),
            [CoverageRequirement {
                path: "lib/cart.rb".to_owned(),
                line: 1,
            }]
            .into_iter()
            .collect::<BTreeSet<_>>(),
        );

        let evidence = EvidenceSet {
            per_test_coverage: Some(PerTestCoverageEvidence { cases }),
            mutation: Some(MutationEvidence {
                total: 2,
                killed: 2,
                equivalent_marked: 0,
                per_test_kills: [
                    (
                        "Cart counts one item".to_owned(),
                        ["m1".to_owned(), "m2".to_owned()].into_iter().collect(),
                    ),
                    (
                        "Cart counts another item".to_owned(),
                        ["m1".to_owned()].into_iter().collect(),
                    ),
                ]
                .into_iter()
                .collect(),
            }),
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

        assert_eq!(
            report.metric_value("redundancy.coverage_subsumption"),
            Some(0.5)
        );
        assert_eq!(
            report.metric_value("redundancy.mutant_subsumption"),
            Some(0.5)
        );
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.rule_id == "redundancy.mutant_subsumption")
        );
    }

    #[test]
    fn maps_probe_method_names_to_ir_case_ids() {
        let mut cases = BTreeMap::new();
        cases.insert(
            "CartTest#test_counts_one_item".to_owned(),
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
            "CartTest#test_counts_another_item".to_owned(),
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
            Path::new("test/cart_test.rb"),
            r#"
            require "minitest/autorun"

            class CartTest < Minitest::Test
              def test_counts_one_item
                assert_equal 1, cart.count
              end

              def test_counts_another_item
                assert_equal 1, cart.count
              end
            end
            "#,
            &AppConfig::default(),
            &evidence,
        );

        assert_eq!(
            report.metric_value("redundancy.coverage_subsumption"),
            Some(0.5)
        );
    }

    #[test]
    fn metric_catalog_includes_evidence_driven_metrics() {
        let ids = metric_catalog(&AppConfig::default())
            .into_iter()
            .map(|outcome| outcome.id)
            .collect::<BTreeSet<_>>();

        assert!(ids.contains("adequacy.checked_coverage"));
        assert!(ids.contains("adequacy.mutation_score"));
        assert!(ids.contains("redundancy.coverage_subsumption"));
        assert!(ids.contains("redundancy.mutant_subsumption"));
        assert!(ids.contains("redundancy.assertion_overlap"));
    }
}
