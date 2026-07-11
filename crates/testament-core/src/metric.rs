use crate::ir::{Axis, Severity, SourceSpan, TestFileIr};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Provenance {
    pub references: Vec<String>,
    pub definition: String,
    pub approximation: String,
}

impl Provenance {
    pub fn new(
        references: &[&str],
        definition: impl Into<String>,
        approximation: impl Into<String>,
    ) -> Self {
        Self {
            references: references
                .iter()
                .map(|reference| (*reference).to_owned())
                .collect(),
            definition: definition.into(),
            approximation: approximation.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetricOutcome {
    pub id: String,
    pub axis: Axis,
    pub score: Option<f64>,
    pub value: f64,
    pub unit: String,
    pub summary: String,
    pub findings: Vec<Finding>,
    pub provenance: Provenance,
}

impl MetricOutcome {
    pub fn scored(
        id: impl Into<String>,
        axis: Axis,
        score: f64,
        unit: impl Into<String>,
        summary: impl Into<String>,
        provenance: Provenance,
    ) -> Self {
        Self {
            id: id.into(),
            axis,
            score: Some(score.clamp(0.0, 1.0)),
            value: score.clamp(0.0, 1.0),
            unit: unit.into(),
            summary: summary.into(),
            findings: Vec::new(),
            provenance,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub rule_id: String,
    pub axis: Axis,
    pub severity: Severity,
    pub message: String,
    pub span: Option<SourceSpan>,
    pub evidence: String,
    pub case_id: Option<String>,
}

impl Finding {
    pub fn new(
        rule_id: impl Into<String>,
        axis: Axis,
        severity: Severity,
        message: impl Into<String>,
        span: Option<SourceSpan>,
        evidence: impl Into<String>,
        case_id: Option<String>,
    ) -> Self {
        Self {
            rule_id: rule_id.into(),
            axis,
            severity,
            message: message.into(),
            span,
            evidence: evidence.into(),
            case_id,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileReport {
    pub ir: TestFileIr,
    pub outcomes: Vec<MetricOutcome>,
    pub axis_scores: BTreeMap<String, f64>,
    pub score: f64,
    pub findings: Vec<Finding>,
}

impl FileReport {
    pub fn metric_score(&self, metric_id: &str) -> Option<f64> {
        self.outcomes
            .iter()
            .find(|outcome| outcome.id == metric_id)
            .and_then(|outcome| outcome.score)
    }

    pub fn metric_value(&self, metric_id: &str) -> Option<f64> {
        self.outcomes
            .iter()
            .find(|outcome| outcome.id == metric_id)
            .map(|outcome| outcome.value)
    }

    pub fn candidate_ratio(&self) -> f64 {
        self.metric_score("redundancy.candidate_ratio")
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectReport {
    pub files: Vec<FileReport>,
    pub gates: Vec<crate::gate::GateViolation>,
    pub warnings: Vec<String>,
    pub passed: bool,
}

impl ProjectReport {
    pub fn empty() -> Self {
        Self {
            files: Vec::new(),
            gates: Vec::new(),
            warnings: Vec::new(),
            passed: true,
        }
    }

    pub fn score(&self) -> f64 {
        if self.files.is_empty() {
            return 1.0;
        }
        self.files.iter().map(|file| file.score).sum::<f64>() / self.files.len() as f64
    }

    pub fn case_count(&self) -> usize {
        self.files.iter().map(|file| file.ir.case_count()).sum()
    }

    pub fn finding_count(&self) -> usize {
        self.files.iter().map(|file| file.findings.len()).sum()
    }
}

pub fn axis_average(outcomes: &[MetricOutcome], axis: Axis) -> Option<f64> {
    let scores: Vec<f64> = outcomes
        .iter()
        .filter(|outcome| outcome.axis == axis)
        .filter_map(|outcome| outcome.score)
        .collect();
    if scores.is_empty() {
        None
    } else {
        Some(scores.iter().sum::<f64>() / scores.len() as f64)
    }
}
