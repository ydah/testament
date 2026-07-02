use crate::config::{AppConfig, GateLevel};
use crate::metric::FileReport;
use std::collections::BTreeMap;

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

            if let Some(min) = gate.min {
                let observed = metric.score.unwrap_or(metric.value);
                if observed + f64::EPSILON < min {
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
            }

            if let Some(max) = gate.max {
                let observed = metric.value;
                if observed > max + f64::EPSILON {
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
    files
        .iter()
        .filter_map(|file| {
            let path = file.ir.path_display();
            let (baseline_path, baseline) = baseline_for_path(baseline_scores, &path)?;
            if file.score + tolerance >= baseline {
                return None;
            }
            let rename_note = if baseline_path == path {
                String::new()
            } else {
                format!(" (matched prior path `{baseline_path}`)")
            };
            Some(GateViolation {
                metric_id: "project.file_score".to_owned(),
                path,
                level: GateLevel::Error,
                observed: file.score,
                threshold: baseline,
                direction: GateDirection::Ratchet,
                message: format!(
                    "file score regressed from {:.3} to {:.3}{}",
                    baseline, file.score, rename_note
                ),
            })
        })
        .collect()
}

fn baseline_for_path(baseline_scores: &BTreeMap<String, f64>, path: &str) -> Option<(String, f64)> {
    if let Some(score) = baseline_scores.get(path) {
        return Some((path.to_owned(), *score));
    }

    baseline_scores
        .iter()
        .map(|(candidate, score)| (candidate, *score, path_similarity(candidate, path)))
        .filter(|(_, _, similarity)| *similarity >= 0.50)
        .max_by(|left, right| left.2.total_cmp(&right.2))
        .map(|(candidate, score, _)| (candidate.clone(), score))
}

pub fn parse_baseline_scores(input: &str) -> BTreeMap<String, f64> {
    let mut scores = BTreeMap::new();
    let mut current_path = None::<String>;

    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("\"path\"") {
            current_path = json_string_value(trimmed);
        } else if trimmed.starts_with("\"score\"") {
            let parsed = current_path.take().zip(json_number_value(trimmed));
            if let Some((path, score)) = parsed {
                scores.insert(path, score);
            }
        }
    }

    scores
}

fn json_string_value(line: &str) -> Option<String> {
    let value = line.split_once(':')?.1.trim().trim_end_matches(',');
    Some(value.trim_matches('"').to_owned())
}

fn json_number_value(line: &str) -> Option<f64> {
    let value = line.split_once(':')?.1.trim().trim_end_matches(',');
    value.parse::<f64>().ok()
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
    fn tracks_baseline_across_similar_renames() {
        let mut scores = BTreeMap::new();
        scores.insert("spec/models/user_profile_spec.rb".to_owned(), 0.8);

        let matched = baseline_for_path(&scores, "spec/domain/user_profile_spec.rb");

        assert_eq!(
            matched,
            Some(("spec/models/user_profile_spec.rb".to_owned(), 0.8))
        );
    }
}
