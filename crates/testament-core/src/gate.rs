use crate::config::{AppConfig, GateLevel, GateTarget};
use crate::metric::FileReport;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq)]
pub struct GateViolation {
    pub metric_id: String,
    pub path: String,
    pub level: GateLevel,
    pub observed: f64,
    pub threshold: f64,
    pub direction: GateDirection,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GateDirection {
    Min,
    Max,
    Ratchet,
}

impl GateDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Min => "min",
            Self::Max => "max",
            Self::Ratchet => "ratchet",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GateEvaluation {
    pub passed: bool,
    pub violations: Vec<GateViolation>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BaselineFile {
    pub score: f64,
    pub metric_ids: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RatchetEvaluation {
    pub violations: Vec<GateViolation>,
    pub warnings: Vec<String>,
}

pub fn evaluate_gates(config: &AppConfig, files: &[FileReport]) -> GateEvaluation {
    let mut violations = Vec::new();

    for file in files {
        for gate in &config.gates {
            let metric = file
                .outcomes
                .iter()
                .find(|outcome| outcome.id == gate.metric_id);
            let Some(metric) = metric else {
                if gate.when_evidence_available {
                    continue;
                }
                violations.push(GateViolation {
                    metric_id: gate.metric_id.clone(),
                    path: file.ir.path_display(),
                    level: gate.level,
                    observed: 0.0,
                    threshold: 0.0,
                    direction: GateDirection::Min,
                    message: format!("required metric `{}` was not computed", gate.metric_id),
                });
                continue;
            };

            let observed = match gate.target {
                GateTarget::Score => metric.score.unwrap_or(metric.value),
                GateTarget::Value => metric.value,
            };
            const GATE_TOLERANCE: f64 = 1e-9;

            if let Some(min) = gate.min
                && observed + GATE_TOLERANCE < min
            {
                violations.push(GateViolation {
                    metric_id: gate.metric_id.clone(),
                    path: file.ir.path_display(),
                    level: gate.level,
                    observed,
                    threshold: min,
                    direction: GateDirection::Min,
                    message: format!(
                        "{} is {:.3}, below minimum {:.3}",
                        gate.metric_id, observed, min
                    ),
                });
            }

            if let Some(max) = gate.max
                && observed > max + GATE_TOLERANCE
            {
                violations.push(GateViolation {
                    metric_id: gate.metric_id.clone(),
                    path: file.ir.path_display(),
                    level: gate.level,
                    observed,
                    threshold: max,
                    direction: GateDirection::Max,
                    message: format!(
                        "{} is {:.3}, above maximum {:.3}",
                        gate.metric_id, observed, max
                    ),
                });
            }
        }
    }

    let passed = violations
        .iter()
        .all(|violation| violation.level != GateLevel::Error);

    GateEvaluation { passed, violations }
}

pub fn evaluate_ratchet(
    baseline_scores: &BTreeMap<String, f64>,
    tolerance: f64,
    files: &[FileReport],
) -> Vec<GateViolation> {
    let baseline = baseline_scores
        .iter()
        .map(|(path, score)| {
            (
                path.clone(),
                BaselineFile {
                    score: *score,
                    metric_ids: BTreeSet::new(),
                },
            )
        })
        .collect();
    evaluate_ratchet_with_metrics(&baseline, tolerance, files, true).violations
}

pub fn evaluate_ratchet_with_metrics(
    baseline: &BTreeMap<String, BaselineFile>,
    tolerance: f64,
    files: &[FileReport],
    rename_tracking: bool,
) -> RatchetEvaluation {
    let mut evaluation = RatchetEvaluation {
        violations: Vec::new(),
        warnings: Vec::new(),
    };
    let mut consumed = BTreeSet::new();
    let exact_paths = files
        .iter()
        .map(|file| file.ir.path_display())
        .filter(|path| baseline.contains_key(path))
        .collect::<BTreeSet<_>>();
    for file in files {
        let path = file.ir.path_display();
        let Some((baseline_path, prior)) =
            baseline_entry_for_path(baseline, &path, &consumed, &exact_paths, rename_tracking)
        else {
            continue;
        };
        consumed.insert(baseline_path.clone());
        let current_metrics = file
            .outcomes
            .iter()
            .map(|outcome| outcome.id.clone())
            .collect::<BTreeSet<_>>();
        if !prior.metric_ids.is_empty() && prior.metric_ids != current_metrics {
            evaluation.warnings.push(format!(
                "ratchet skipped for {path}: metric/evidence set differs from baseline"
            ));
            continue;
        }
        if file.score + tolerance >= prior.score {
            continue;
        }
        let rename_note = if baseline_path == path {
            String::new()
        } else {
            format!(" (matched prior path `{baseline_path}`)")
        };
        evaluation.violations.push(GateViolation {
            metric_id: "project.file_score".to_owned(),
            path,
            level: GateLevel::Error,
            observed: file.score,
            threshold: prior.score,
            direction: GateDirection::Ratchet,
            message: format!(
                "file score regressed from {:.3} to {:.3}{}",
                prior.score, file.score, rename_note
            ),
        });
    }
    evaluation
}

fn baseline_entry_for_path(
    baseline: &BTreeMap<String, BaselineFile>,
    path: &str,
    consumed: &BTreeSet<String>,
    exact_paths: &BTreeSet<String>,
    rename_tracking: bool,
) -> Option<(String, BaselineFile)> {
    if let Some(entry) = baseline.get(path) {
        return Some((path.to_owned(), entry.clone()));
    }
    if !rename_tracking {
        return None;
    }
    baseline
        .iter()
        .filter(|(candidate, _)| {
            !consumed.contains(*candidate) && !exact_paths.contains(*candidate)
        })
        .filter(|(candidate, _)| {
            std::path::Path::new(candidate.as_str()).file_name()
                == std::path::Path::new(path).file_name()
        })
        .map(|(candidate, entry)| (candidate, entry, path_similarity(candidate, path)))
        .filter(|(_, _, similarity)| *similarity >= 0.50)
        .max_by(|left, right| left.2.total_cmp(&right.2))
        .map(|(candidate, entry, _)| (candidate.clone(), entry.clone()))
}

#[cfg(test)]
fn baseline_for_path(baseline_scores: &BTreeMap<String, f64>, path: &str) -> Option<(String, f64)> {
    if let Some(score) = baseline_scores.get(path) {
        return Some((path.to_owned(), *score));
    }

    baseline_scores
        .iter()
        .filter(|(candidate, _)| {
            std::path::Path::new(candidate.as_str()).file_name()
                == std::path::Path::new(path).file_name()
        })
        .map(|(candidate, score)| (candidate, *score, path_similarity(candidate, path)))
        .filter(|(_, _, similarity)| *similarity >= 0.50)
        .max_by(|left, right| left.2.total_cmp(&right.2))
        .map(|(candidate, score, _)| (candidate.clone(), score))
}

pub fn parse_baseline_scores(input: &str) -> BTreeMap<String, f64> {
    parse_baseline_files(input)
        .into_iter()
        .map(|(path, file)| (path, file.score))
        .collect()
}

pub fn parse_baseline_files(input: &str) -> BTreeMap<String, BaselineFile> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(input) else {
        return BTreeMap::new();
    };
    value
        .get("files")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|file| {
            Some((
                file.get("path")?.as_str()?.to_owned(),
                BaselineFile {
                    score: file.get("score")?.as_f64()?,
                    metric_ids: file
                        .get("metrics")
                        .and_then(serde_json::Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|metric| metric.get("id")?.as_str().map(ToOwned::to_owned))
                        .collect(),
                },
            ))
        })
        .collect()
}

fn path_similarity(left: &str, right: &str) -> f64 {
    if left == right {
        return 1.0;
    }
    let left_tokens = path_tokens(left);
    let right_tokens = path_tokens(right);
    if left_tokens.is_empty() || right_tokens.is_empty() {
        return 0.0;
    }
    let intersection = left_tokens
        .iter()
        .filter(|token| right_tokens.contains(token))
        .count();
    let union = left_tokens.len() + right_tokens.len() - intersection;
    intersection as f64 / union as f64
}

fn path_tokens(path: &str) -> Vec<String> {
    let mut tokens = path
        .split(['/', '_', '-', '.'])
        .filter(|token| !matches!(*token, "" | "spec" | "test" | "rb"))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    tokens.sort();
    tokens.dedup();
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Axis, Confidence, MetricOutcome, Provenance, TestFileIr};

    fn file_report(path: &str, score: f64) -> FileReport {
        let mut ir = TestFileIr::new(path, "ruby", "rspec");
        ir.confidence = Confidence::Exact;
        FileReport {
            ir,
            outcomes: Vec::new(),
            axis_scores: BTreeMap::new(),
            score,
            findings: Vec::new(),
        }
    }

    #[test]
    fn parses_scores_from_report_json() {
        let scores = parse_baseline_scores(
            r#"
            {
              "files": [
                {
                  "path": "spec/user_spec.rb",
                  "score": 0.900
                }
              ]
            }
            "#,
        );
        assert_eq!(scores.get("spec/user_spec.rb"), Some(&0.9));
    }

    #[test]
    fn parses_minified_baselines_without_key_order_dependencies() {
        let scores = parse_baseline_scores(
            r#"{"gates":[{"path":"wrong.rb"}],"files":[{"score":0.75,"path":"spec/right_spec.rb"}]}"#,
        );

        assert_eq!(
            scores,
            BTreeMap::from([("spec/right_spec.rb".to_owned(), 0.75)])
        );
    }

    #[test]
    fn tracks_baseline_across_similar_renames() {
        let mut scores = BTreeMap::new();
        scores.insert("spec/models/user_profile_spec.rb".to_owned(), 0.8);

        let matched = baseline_for_path(&scores, "spec/domain/user_profile_spec.rb");

        assert_eq!(
            matched,
            Some(("spec/models/user_profile_spec.rb".to_owned(), 0.8))
        );
    }

    #[test]
    fn does_not_match_a_different_test_file_as_a_rename() {
        let scores = BTreeMap::from([("spec/api/v1/users_controller_spec.rb".to_owned(), 0.8)]);

        assert_eq!(
            baseline_for_path(&scores, "spec/api/v1/orders_controller_spec.rb"),
            None
        );
    }

    #[test]
    fn ratchet_skips_files_with_different_metric_sets() {
        let baseline = parse_baseline_files(
            r#"{"files":[{"path":"spec/a_spec.rb","score":0.9,"metrics":[{"id":"adequacy.line_coverage"}]}]}"#,
        );
        let mut ir = TestFileIr::new("spec/a_spec.rb", "ruby", "rspec");
        ir.confidence = Confidence::Exact;
        let file = FileReport {
            ir,
            outcomes: vec![MetricOutcome::scored(
                "adequacy.assertion_density",
                Axis::Adequacy,
                0.5,
                "ratio",
                "test",
                Provenance::new(&[], "test", "test"),
            )],
            axis_scores: BTreeMap::new(),
            score: 0.5,
            findings: Vec::new(),
        };

        let evaluation = evaluate_ratchet_with_metrics(&baseline, 0.0, &[file], true);
        assert!(evaluation.violations.is_empty());
        assert_eq!(evaluation.warnings.len(), 1);
    }

    #[test]
    fn rename_fallback_does_not_consume_an_exact_baseline() {
        let baseline = BTreeMap::from([(
            "spec/models/user_spec.rb".to_owned(),
            BaselineFile {
                score: 0.9,
                metric_ids: BTreeSet::new(),
            },
        )]);
        let files = [
            file_report("spec/domain/user_spec.rb", 0.1),
            file_report("spec/models/user_spec.rb", 0.1),
        ];

        let evaluation = evaluate_ratchet_with_metrics(&baseline, 0.0, &files, true);
        assert_eq!(evaluation.violations.len(), 1);
        assert_eq!(evaluation.violations[0].path, "spec/models/user_spec.rb");
    }

    #[test]
    fn rename_tracking_can_be_disabled() {
        let baseline = BTreeMap::from([(
            "spec/models/user_spec.rb".to_owned(),
            BaselineFile {
                score: 0.9,
                metric_ids: BTreeSet::new(),
            },
        )]);
        let files = [file_report("spec/domain/user_spec.rb", 0.1)];

        let evaluation = evaluate_ratchet_with_metrics(&baseline, 0.0, &files, false);
        assert!(evaluation.violations.is_empty());
    }
}
