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
            let metric = file.outcomes.iter().find(|outcome| outcome.id == gate.metric_id);
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
            let baseline = baseline_scores.get(&path)?;
            if file.score + tolerance >= *baseline {
                return None;
            }
            Some(GateViolation {
                metric_id: "project.file_score".to_owned(),
                path,
                level: GateLevel::Error,
                observed: file.score,
                threshold: *baseline,
                direction: GateDirection::Ratchet,
                message: format!(
                    "file score regressed from {:.3} to {:.3}",
                    baseline, file.score
                ),
            })
        })
        .collect()
}

pub fn parse_baseline_scores(input: &str) -> BTreeMap<String, f64> {
    let mut scores = BTreeMap::new();
    let mut current_path = None::<String>;

    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("\"path\"") {
            current_path = json_string_value(trimmed);
        } else if trimmed.starts_with("\"score\"") {
            if let Some(path) = current_path.take() {
                if let Some(score) = json_number_value(trimmed) {
                    scores.insert(path, score);
                }
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
}

