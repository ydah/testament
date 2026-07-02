use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EvidenceSet {
    pub coverage: Option<CoverageEvidence>,
    pub mutation: Option<MutationEvidence>,
    pub per_test_coverage: Option<PerTestCoverageEvidence>,
    pub sources: Vec<EvidenceSource>,
    pub warnings: Vec<String>,
}

impl EvidenceSet {
    pub fn is_empty(&self) -> bool {
        self.coverage.is_none() && self.mutation.is_none() && self.per_test_coverage.is_none()
    }

    pub fn merge(&mut self, other: EvidenceSet) {
        if other.coverage.is_some() {
            self.coverage = other.coverage;
        }
        if other.mutation.is_some() {
            self.mutation = other.mutation;
        }
        if other.per_test_coverage.is_some() {
            self.per_test_coverage = other.per_test_coverage;
        }
        self.sources.extend(other.sources);
        self.warnings.extend(other.warnings);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceSource {
    pub format: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoverageEvidence {
    pub files: BTreeMap<String, FileCoverage>,
}

impl CoverageEvidence {
    pub fn file_for_path(&self, path: &Path) -> Option<&FileCoverage> {
        let normalized = normalize_path(path);
        self.files
            .get(&normalized)
            .or_else(|| self.files.get(normalized.trim_start_matches("./")))
            .or_else(|| {
                self.files
                    .iter()
                    .find(|(candidate, _)| normalized.ends_with(candidate.as_str()))
                    .map(|(_, coverage)| coverage)
            })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileCoverage {
    pub line_rate: Option<f64>,
    pub branch_rate: Option<f64>,
    pub covered_lines: BTreeSet<usize>,
    pub executable_lines: BTreeSet<usize>,
}

impl FileCoverage {
    pub fn line_coverage(&self) -> Option<f64> {
        self.line_rate.or_else(|| {
            if self.executable_lines.is_empty() {
                None
            } else {
                Some(self.covered_lines.len() as f64 / self.executable_lines.len() as f64)
            }
        })
    }

    pub fn branch_coverage(&self) -> Option<f64> {
        self.branch_rate
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PerTestCoverageEvidence {
    pub cases: BTreeMap<String, BTreeSet<CoverageRequirement>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct CoverageRequirement {
    pub path: String,
    pub line: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MutationEvidence {
    pub total: usize,
    pub killed: usize,
    pub equivalent_marked: usize,
    pub per_test_kills: BTreeMap<String, BTreeSet<String>>,
}

impl MutationEvidence {
    pub fn score(&self) -> Option<f64> {
        let denominator = self.total.saturating_sub(self.equivalent_marked);
        if denominator == 0 {
            return None;
        }
        Some(self.killed as f64 / denominator as f64)
    }
}

pub fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

